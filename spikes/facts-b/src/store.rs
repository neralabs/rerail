//! Process-side fact store: per-process string interning, per-content exported facts as integer
//! columns, and worktree memberships (path -> content hash).
use crate::facts::{self, Exported};
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

pub type Sym = u32;
pub type Cid = u32; // content id (index into Store::contents)

#[derive(Default)]
pub struct Interner {
    map: FxHashMap<Box<str>, Sym>,
    strs: Vec<Box<str>>,
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> Sym {
        if let Some(&i) = self.map.get(s) {
            return i;
        }
        let i = self.strs.len() as Sym;
        self.strs.push(s.into());
        self.map.insert(s.into(), i);
        i
    }
    pub fn get(&self, s: &str) -> Option<Sym> {
        self.map.get(s).copied()
    }
    pub fn str(&self, i: Sym) -> &str {
        &self.strs[i as usize]
    }
    pub fn len(&self) -> usize {
        self.strs.len()
    }
    pub fn bytes(&self) -> usize {
        self.strs.iter().map(|s| s.len() + 16).sum::<usize>() + self.map.capacity() * 24
    }
}

pub const NONE: Sym = u32::MAX;

#[derive(Clone, Copy, Debug, Default)]
pub struct Imp {
    pub module: Sym,
    /// NONE for `import M`.
    pub name: Sym,
    pub local: Sym,
    pub top: bool,
    pub module_scope: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Content {
    pub hash: [u8; 32],
    /// (name, kind, qualname, line)
    pub defs: Vec<(Sym, Sym, Sym, u32)>,
    pub refs: Vec<Sym>,
    /// Source order kept: a later binding of the same local name wins.
    pub imports: Vec<Imp>,
    pub calls: Vec<(Sym, Sym)>,
    pub decorators: Vec<(Sym, Sym)>,
    pub dunder_all: Option<bool>,
    pub dunder_all_names: Vec<Sym>,
    pub seeds: Vec<(Sym, Sym)>,
    pub has_error: bool,
}

impl Content {
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Content>()
            + self.defs.len() * 16
            + self.refs.len() * 4
            + self.imports.len() * 16
            + self.calls.len() * 8
            + self.decorators.len() * 8
            + self.dunder_all_names.len() * 4
            + self.seeds.len() * 8
    }
}

#[derive(Default)]
pub struct Store {
    pub syms: Interner,
    pub contents: Vec<Content>,
    pub by_hash: FxHashMap<[u8; 32], Cid>,
}

impl Store {
    pub fn add(&mut self, hash: [u8; 32], e: &Exported) -> Cid {
        if let Some(&c) = self.by_hash.get(&hash) {
            return c;
        }
        let s = &mut self.syms;
        let c = Content {
            hash,
            defs: e.defs.iter().map(|d| (s.intern(&d.name), s.intern(&d.kind), s.intern(&d.qualname), d.line)).collect(),
            refs: e.refs.iter().map(|r| s.intern(r)).collect(),
            imports: e
                .imports
                .iter()
                .map(|i| Imp {
                    module: s.intern(&i.module),
                    name: i.name.as_deref().map(|n| s.intern(n)).unwrap_or(NONE),
                    local: s.intern(&i.local),
                    top: i.top,
                    module_scope: i.module_scope,
                })
                .collect(),
            calls: e.calls.iter().map(|(a, b)| (s.intern(a), s.intern(b))).collect(),
            decorators: e.decorators.iter().map(|(a, b)| (s.intern(a), s.intern(b))).collect(),
            dunder_all: e.dunder_all,
            dunder_all_names: e.dunder_all_names.iter().map(|n| s.intern(n)).collect(),
            seeds: e.seeds.iter().map(|(a, b)| (s.intern(a), s.intern(b))).collect(),
            has_error: e.has_error,
        };
        let id = self.contents.len() as Cid;
        self.contents.push(c);
        self.by_hash.insert(hash, id);
        id
    }

    pub fn load(&mut self, store_dir: &Path, hash: &[u8; 32]) -> Cid {
        if let Some(&c) = self.by_hash.get(hash) {
            return c;
        }
        let p = facts::store_path(store_dir, &facts::hex(hash));
        let bytes = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let (h, e) = facts::decode(&bytes);
        assert_eq!(&h, hash, "{} header hash", p.display());
        self.add(h, &e)
    }
}

pub fn parse_hex(s: &str) -> [u8; 32] {
    let mut h = [0u8; 32];
    for i in 0..32 {
        h[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    h
}

/// A worktree membership: sorted (path, hash).
pub type Membership = Vec<(String, [u8; 32])>;

pub fn read_membership(p: &Path) -> Membership {
    let s = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    s.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (path, h) = l.split_once('\t').unwrap();
            (path.to_string(), parse_hex(h))
        })
        .collect()
}

pub fn write_membership(p: &Path, m: &Membership) {
    let mut s = String::new();
    for (path, h) in m {
        s.push_str(path);
        s.push('\t');
        s.push_str(&facts::hex(h));
        s.push('\n');
    }
    std::fs::write(p, s).unwrap();
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap())
}

pub fn bench_root() -> PathBuf {
    home().join("rerail-bench/facts")
}

pub fn store_dir() -> PathBuf {
    bench_root().join("store")
}

pub fn spike_dir() -> PathBuf {
    bench_root().join("spike-b")
}

/// Walk a worktree: every regular `.py` file (no symlinks), skipping `.git`.
pub fn walk_py(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let ft = e.file_type().unwrap();
            let p = e.path();
            if ft.is_dir() {
                if e.file_name() != ".git" {
                    stack.push(p);
                }
            } else if ft.is_file() && p.extension().is_some_and(|x| x == "py") {
                out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}
