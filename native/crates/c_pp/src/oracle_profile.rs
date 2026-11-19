//! `clang -E`'s profile, shared by the oracle (`examples/pp_oracle.rs`) and the parallel spike's
//! corpus test (decision 0327, "Correctness"; 0421). The profile is clang's own: its predefined
//! macros (`clang -dM -E`), its search path (`clang -E -v`), and its answers to the `__has_*` tests
//! the pass asks (probed, and kept for the run).
//!
//! A corpus file includes its project's headers, which are not here. Each header the pass does not
//! find gets an empty stub, after every other directory (`-idirafter`), so that every reading
//! reads the same files. A development tool: built for the tests and the example alone.

// Each includer uses a part.
#![allow(dead_code)]

use rerail_c_pp::files::{DirKind, Search};
use rerail_c_pp::macros::Tk;
use rerail_c_pp::sink::{At, FactSink, Sink, Tee};
use rerail_c_pp::{Config, Options, Pp};
use rerail_c_lean::pp::{P, PpLexer};
use std::collections::HashMap;
use std::process::Command;

#[derive(Default)]
pub struct Spellings(pub Vec<Vec<u8>>);

impl Sink for Spellings {
    fn token(&mut self, _t: &Tk, text: &[u8], _at: Option<At>, _off: u32) {
        self.0.push(text.to_vec());
    }
}

pub struct Lang {
    pub opts: Options,
    pub args: Vec<&'static str>,
}

pub fn clang(args: &[&str], input: Option<&[u8]>) -> (bool, Vec<u8>, Vec<u8>) {
    let mut c = Command::new("clang");
    // `__DATE__` and `__TIME__` as the configuration fixes them (the Unix epoch).
    c.args(args).env("SOURCE_DATE_EPOCH", "0");
    if input.is_some() {
        c.stdin(std::process::Stdio::piped());
    }
    c.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = c.spawn().expect("clang runs");
    if let Some(i) = input {
        use std::io::Write;
        child.stdin.take().unwrap().write_all(i).unwrap();
    }
    let o = child.wait_with_output().unwrap();
    (o.status.success(), o.stdout, o.stderr)
}

/// clang's predefined macros and search path.
pub fn profile(lang: &Lang) -> (Vec<(String, Option<String>)>, Vec<(String, DirKind)>) {
    let mut a = lang.args.clone();
    a.extend(["-dM", "-E", "-"]);
    let (_, out, _) = clang(&a, Some(b""));
    let mut defs = Vec::new();
    for l in String::from_utf8_lossy(&out).lines() {
        let Some(rest) = l.strip_prefix("#define ") else { continue };
        // The name runs to the first blank outside its parameter list.
        let mut depth = 0;
        let mut end = rest.len();
        for (k, c) in rest.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ' ' if depth == 0 => {
                    end = k;
                    break;
                }
                _ => {}
            }
        }
        let body = rest.get(end + 1..).unwrap_or("").to_string();
        defs.push((rest[..end].to_string(), Some(body)));
    }
    let mut a = lang.args.clone();
    a.extend(["-E", "-v", "-"]);
    let (_, _, err) = clang(&a, Some(b""));
    let mut dirs = Vec::new();
    let mut on = false;
    for l in String::from_utf8_lossy(&err).lines() {
        if l.starts_with("#include <...> search starts here") {
            on = true;
        } else if l.starts_with("End of search list") {
            on = false;
        } else if on {
            let l = l.trim();
            match l.strip_suffix(" (framework directory)") {
                Some(d) => dirs.push((d.to_string(), DirKind::Framework)),
                None => dirs.push((l.to_string(), DirKind::System)),
            }
        }
    }
    (defs, dirs)
}

/// clang's values of `tests` (`(builtin, operand)`), each line its own: a test clang does not
/// read is 0 alone.
pub fn probe(lang: &Lang, tests: &[(String, String)]) -> Vec<i64> {
    let mut src = String::new();
    for (k, (b, x)) in tests.iter().enumerate() {
        src.push_str(&format!("__rr_probe_{k} {b}({x})\n"));
    }
    let mut a = lang.args.clone();
    a.extend(["-E", "-P", "-w", "-"]);
    let (_, out, _) = clang(&a, Some(src.as_bytes()));
    let mut vals = vec![0; tests.len()];
    for l in String::from_utf8_lossy(&out).lines() {
        let Some(rest) = l.trim().strip_prefix("__rr_probe_") else { continue };
        let Some((k, v)) = rest.split_once(' ') else { continue };
        if let (Ok(k), Ok(v)) = (k.parse::<usize>(), v.trim().parse::<i64>()) {
            if k < vals.len() {
                vals[k] = v;
            }
        }
    }
    vals
}

pub fn read_fs(p: &str) -> Option<Vec<u8>> {
    std::fs::read(p).ok().filter(|_| std::path::Path::new(p).is_file())
}

/// The tokens of clang's `-E -P` text, its `#pragma` lines left out.
pub fn clang_tokens(text: &[u8], opts: Options) -> Vec<Vec<u8>> {
    let mut lx = PpLexer::new(text, opts);
    let mut v = Vec::new();
    loop {
        let t = lx.next();
        if t.kind == P::Eof {
            break;
        }
        if t.kind == P::Hash && t.flags & rerail_c_lean::pp::BOL != 0 {
            lx.skip_line();
            continue;
        }
        v.push(text[t.start as usize..t.end as usize].to_vec());
    }
    v
}

/// `name` under `dir`, `..` read; `None` where it leaves `root`.
pub fn under(root: &std::path::Path, dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let mut p = dir.to_path_buf();
    for c in name.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                if !p.pop() || !p.starts_with(root) || p == root {
                    return None;
                }
            }
            c => p.push(c),
        }
    }
    p.starts_with(root).then_some(p)
}

impl Lang {
    pub fn c() -> Lang {
        Lang { opts: Options::c(17, true), args: vec!["-x", "c", "-std=gnu17"] }
    }

    pub fn cpp() -> Lang {
        Lang { opts: Options::cpp(20, true), args: vec!["-x", "c++", "-std=gnu++20"] }
    }
}

/// A language's profile, its probed answers kept across files, and the stubs' directory: one
/// directory a settled file, so that files settled together (a batch) each keep their own.
pub struct Profile {
    pub lang: Lang,
    defs: Vec<(String, Option<String>)>,
    dirs: Vec<(String, DirKind)>,
    features: HashMap<(String, String), i64>,
    stubs: std::path::PathBuf,
    settled: usize,
}

/// A file's settled reading: the configuration and search path every reading of it uses (the
/// stubs' directory last), and the serial pass's tokens and facts under them.
pub struct Settled {
    pub cfg: Config,
    pub dirs: Vec<(String, DirKind)>,
    pub stub_dir: String,
    pub tokens: Vec<Vec<u8>>,
    pub facts: FactSink,
}

impl Profile {
    pub fn new(lang: Lang) -> Profile {
        let (defs, dirs) = profile(&lang);
        let stubs = std::env::temp_dir().join(format!("rerail-pp-oracle-{}", std::process::id()));
        Profile { lang, defs, dirs, features: HashMap::new(), stubs, settled: 0 }
    }

    /// `path` read by the serial pass until no header is missing and no test unanswered.
    pub fn settle(&mut self, path: &str) -> Settled {
        self.settled += 1;
        let stubs = self.stubs.join(self.settled.to_string());
        let _ = std::fs::remove_dir_all(&stubs);
        // Stubs for `../x.h` too: the directory is nested, so that they stay inside.
        let nest = stubs.join("a/b/c/d");
        std::fs::create_dir_all(&nest).unwrap();
        let stub_dir = nest.to_string_lossy().into_owned();
        // At most four readings, as the oracle has always read.
        for round in 0.. {
            let mut cfg = Config::new(self.lang.opts);
            cfg.predefines = self.defs.clone();
            cfg.features = self.features.clone();
            let mut d = self.dirs.clone();
            d.push((stub_dir.clone(), DirKind::After));
            let mut io = Search::new(d.clone(), read_fs);
            let mut sp = Spellings::default();
            let mut facts = FactSink::default();
            let mut tee = Tee(&mut sp, &mut facts);
            let mut pp = Pp::new(cfg.clone(), &mut io, &mut tee);
            pp.run(path);
            let unanswered: Vec<_> = pp.unanswered.iter().filter(|k| !self.features.contains_key(*k)).cloned().collect();
            drop(pp);
            let mut again = false;
            if !unanswered.is_empty() {
                let mut u = unanswered;
                u.sort();
                u.dedup();
                for (k, v) in u.iter().zip(probe(&self.lang, &u)) {
                    self.features.insert(k.clone(), v);
                }
                again = true;
            }
            for (_, _, name, _) in &facts.missing {
                let n = String::from_utf8_lossy(name).into_owned();
                if n.starts_with('/') {
                    continue;
                }
                let Some(s) = under(&stubs, &nest, &n) else { continue };
                if !s.exists() {
                    std::fs::create_dir_all(s.parent().unwrap()).unwrap();
                    std::fs::write(&s, b"").unwrap();
                    again = true;
                }
            }
            if !again || round == 3 {
                cfg.features = self.features.clone();
                return Settled { cfg, dirs: d, stub_dir, tokens: sp.0, facts };
            }
        }
        unreachable!("the readings end")
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.stubs);
    }
}
