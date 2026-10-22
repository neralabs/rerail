//! Edit script generation (step 4). Deterministic: a fixed seed drives every choice. Generated
//! contents are written to `spike-b/contents/<hash>.py` and their facts to the shared store, so the
//! replay starts from facts on disk.
use crate::base;
use crate::derive::module_of;
use crate::extract::extract_one;
use crate::facts::{self, dotted};
use crate::store::{self, Membership};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tree_sitter::Node;

pub struct Rng(u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

type H = [u8; 32];

struct Gen {
    rng: Rng,
    parser: tree_sitter::Parser,
    /// hash -> where its bytes live
    sources: BTreeMap<H, PathBuf>,
    contents_dir: PathBuf,
    store_dir: PathBuf,
    state: BTreeMap<u32, BTreeMap<String, H>>,
    /// per worktree: stack of (path, old, new) for revert
    history: BTreeMap<u32, Vec<(String, Option<H>, Option<H>)>>,
    events: Vec<serde_json::Value>,
    kinds: BTreeMap<String, usize>,
    seq: usize,
    new_contents: usize,
}

const KINDS: &[(&str, u64)] = &[("delete_fn", 25), ("rename_fn", 20), ("import", 15), ("call", 20), ("revert", 10), ("whitespace", 10)];

fn text<'a>(n: Node, src: &'a [u8]) -> &'a str {
    std::str::from_utf8(&src[n.byte_range()]).unwrap_or("")
}

fn line_start(src: &[u8], at: usize) -> usize {
    src[..at].iter().rposition(|&b| b == b'\n').map(|i| i + 1).unwrap_or(0)
}

fn line_end(src: &[u8], at: usize) -> usize {
    src[at..].iter().position(|&b| b == b'\n').map(|i| at + i + 1).unwrap_or(src.len())
}

/// Nodes in preorder satisfying `pred`.
fn collect<'t>(root: Node<'t>, pred: &mut dyn FnMut(Node<'t>) -> bool) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if pred(n) {
            out.push(n);
        }
        let mut c = n.walk();
        let ch: Vec<Node> = n.children(&mut c).collect();
        stack.extend(ch.into_iter().rev());
    }
    out
}

fn inside_function(n: Node) -> bool {
    let mut p = n.parent();
    while let Some(x) = p {
        if x.kind() == "function_definition" {
            return true;
        }
        p = x.parent();
    }
    false
}

fn whole_lines_only(src: &[u8], n: Node) -> bool {
    let (s, e) = (n.start_byte(), n.end_byte());
    let ls = line_start(src, s);
    let le = line_end(src, e.saturating_sub(1).max(s));
    src[ls..s].iter().all(|b| *b == b' ' || *b == b'\t') && src[e..le].iter().all(|b| b.is_ascii_whitespace())
}

impl Gen {
    fn bytes(&self, h: &H) -> Vec<u8> {
        std::fs::read(&self.sources[h]).unwrap()
    }

    fn ok(&mut self, src: &[u8]) -> bool {
        !facts::parse(&mut self.parser, src).root_node().has_error()
    }

    /// Store a new content: bytes on disk, facts in the store.
    fn put(&mut self, src: &[u8]) -> H {
        let h: H = blake3::hash(src).into();
        if !self.sources.contains_key(&h) {
            let p = self.contents_dir.join(format!("{}.py", facts::hex(&h)));
            std::fs::write(&p, src).unwrap();
            self.sources.insert(h, p);
            extract_one(&mut self.parser, &self.store_dir, &h, src);
            self.new_contents += 1;
        }
        h
    }

    fn emit(&mut self, group: &str, kind: &str, w: u32, changes: Vec<(String, Option<H>, Option<H>)>) {
        let st = self.state.get_mut(&w).unwrap();
        for (p, old, new) in &changes {
            assert_eq!(st.get(p), old.as_ref());
            match new {
                Some(h) => st.insert(p.clone(), *h),
                None => st.remove(p),
            };
        }
        let ch: Vec<_> = changes.iter().map(|(p, o, n)| json!([p, o.map(|h| facts::hex(&h)), n.map(|h| facts::hex(&h))])).collect();
        self.events.push(json!({"seq": self.seq, "group": group, "kind": kind, "worktree": w, "changes": ch}));
        self.seq += 1;
        *self.kinds.entry(format!("{group}:{kind}")).or_default() += 1;
    }

    fn pick_kind(&mut self) -> &'static str {
        let total: u64 = KINDS.iter().map(|k| k.1).sum();
        let mut r = self.rng.next() % total;
        for (k, wgt) in KINDS {
            if r < *wgt {
                return k;
            }
            r -= wgt;
        }
        unreachable!()
    }

    /// Try one content transformation of `kind` on a random file of w; None if this file has no target.
    fn transform(&mut self, w: u32, kind: &str, path: &str, src: &[u8]) -> Option<(Vec<u8>, String)> {
        let tree = facts::parse(&mut self.parser, src);
        let root = tree.root_node();
        if root.has_error() {
            return None;
        }
        let decl_fns = collect(root, &mut |n| n.kind() == "function_definition" && !inside_function(n));
        match kind {
            "delete_fn" => {
                if decl_fns.is_empty() {
                    return None;
                }
                let f = decl_fns[self.rng.below(decl_fns.len())];
                let node = f.parent().filter(|p| p.kind() == "decorated_definition").unwrap_or(f);
                let (s, e) = (line_start(src, node.start_byte()), line_end(src, node.end_byte() - 1));
                let mut out = src[..s].to_vec();
                out.extend_from_slice(&src[e..]);
                let name = f.child_by_field_name("name").map(|n| text(n, src)).unwrap_or("");
                Some((out, format!("delete {name}")))
            }
            "rename_fn" => {
                let cands: Vec<Node> = decl_fns
                    .into_iter()
                    .filter(|f| f.child_by_field_name("name").is_some_and(|n| !text(n, src).starts_with("__")))
                    .collect();
                if cands.is_empty() {
                    return None;
                }
                let f = cands[self.rng.below(cands.len())];
                let name = text(f.child_by_field_name("name").unwrap(), src).to_string();
                let new = format!("{name}_r{}", self.seq);
                let ids = collect(root, &mut |n| n.kind() == "identifier" && text(n, src) == name);
                let mut out = src.to_vec();
                for id in ids.iter().rev() {
                    out.splice(id.start_byte()..id.end_byte(), new.bytes());
                }
                Some((out, format!("rename {name} -> {new} ({} sites)", ids.len())))
            }
            "import" => {
                let imports: Vec<Node> = {
                    let mut c = root.walk();
                    root.children(&mut c)
                        .filter(|n| (n.kind() == "import_statement" || n.kind() == "import_from_statement") && !text(*n, src).contains("__future__"))
                        .collect()
                };
                if self.rng.below(2) == 0 && !imports.is_empty() {
                    let n = imports[self.rng.below(imports.len())];
                    if !whole_lines_only(src, n) {
                        return None;
                    }
                    let (s, e) = (line_start(src, n.start_byte()), line_end(src, n.end_byte() - 1));
                    let mut out = src[..s].to_vec();
                    out.extend_from_slice(&src[e..]);
                    Some((out, format!("remove import {}", text(n, src).lines().next().unwrap_or(""))))
                } else {
                    // Import a top-level function of another file of the same worktree.
                    let (m, f) = self.random_function(w, path)?;
                    let line = format!("from {m} import {f}\n");
                    let at = match imports.last() {
                        Some(n) => line_end(src, n.end_byte() - 1),
                        None => {
                            // after a leading `from __future__` or docstring block: put at the end of the file instead
                            src.len()
                        }
                    };
                    let mut out = src[..at].to_vec();
                    if !out.is_empty() && *out.last().unwrap() != b'\n' {
                        out.push(b'\n');
                    }
                    out.extend_from_slice(line.as_bytes());
                    out.extend_from_slice(&src[at..]);
                    Some((out, format!("add {}", line.trim())))
                }
            }
            "call" => {
                if self.rng.below(2) == 0 {
                    let stmts = collect(root, &mut |n| {
                        n.kind() == "expression_statement"
                            && n.named_child_count() == 1
                            && n.named_child(0).is_some_and(|c| c.kind() == "call")
                            && inside_function(n)
                            && n.parent().is_some_and(|b| b.kind() == "block" && b.named_child_count() > 1)
                    });
                    let stmts: Vec<Node> = stmts.into_iter().filter(|n| whole_lines_only(src, *n)).collect();
                    if stmts.is_empty() {
                        return None;
                    }
                    let n = stmts[self.rng.below(stmts.len())];
                    let (s, e) = (line_start(src, n.start_byte()), line_end(src, n.end_byte() - 1));
                    let mut out = src[..s].to_vec();
                    out.extend_from_slice(&src[e..]);
                    Some((out, format!("remove call {}", text(n, src).lines().next().unwrap_or(""))))
                } else {
                    if decl_fns.is_empty() {
                        return None;
                    }
                    let f = decl_fns[self.rng.below(decl_fns.len())];
                    let body = f.child_by_field_name("body")?;
                    let first = body.named_child(0)?;
                    if body.start_position().row == f.start_position().row {
                        return None; // one-line body
                    }
                    // Callees bound in this file: top-level functions and module-scope symbol imports.
                    let mut names: Vec<String> = Vec::new();
                    let mut c = root.walk();
                    for n in root.children(&mut c) {
                        let d = if n.kind() == "decorated_definition" { n.child_by_field_name("definition") } else { Some(n) };
                        if let Some(d) = d.filter(|d| d.kind() == "function_definition") {
                            names.push(text(d.child_by_field_name("name").unwrap(), src).to_string());
                        }
                        if n.kind() == "import_from_statement" && !text(n, src).contains("__future__") {
                            let mut c2 = n.walk();
                            for it in n.children_by_field_name("name", &mut c2) {
                                let local = if it.kind() == "aliased_import" { it.child_by_field_name("alias") } else { Some(it) };
                                if let Some(l) = local.and_then(|l| dotted(l, src)) {
                                    if !l.contains('.') {
                                        names.push(l);
                                    }
                                }
                            }
                        }
                    }
                    if names.is_empty() {
                        return None;
                    }
                    let callee = names[self.rng.below(names.len())].clone();
                    let ls = line_start(src, first.start_byte());
                    let indent: Vec<u8> = src[ls..first.start_byte()].to_vec();
                    if !indent.iter().all(|b| *b == b' ' || *b == b'\t') {
                        return None;
                    }
                    let mut out = src[..ls].to_vec();
                    out.extend_from_slice(&indent);
                    out.extend_from_slice(format!("{callee}()\n").as_bytes());
                    out.extend_from_slice(&src[ls..]);
                    let fname = text(f.child_by_field_name("name").unwrap(), src);
                    Some((out, format!("add call {callee}() in {fname}")))
                }
            }
            "whitespace" => {
                let mut out = src.to_vec();
                out.push(b'\n');
                Some((out, "append newline".into()))
            }
            _ => unreachable!(),
        }
    }

    fn random_function(&mut self, w: u32, not_path: &str) -> Option<(String, String)> {
        let paths: Vec<(String, H)> = self.state[&w].iter().map(|(p, h)| (p.clone(), *h)).collect();
        for _ in 0..50 {
            let (p, h) = &paths[self.rng.below(paths.len())];
            if p == not_path || !in_scope(p) {
                continue;
            }
            let src = self.bytes(h);
            let tree = facts::parse(&mut self.parser, &src);
            let root = tree.root_node();
            let mut c = root.walk();
            let fns: Vec<String> = root
                .children(&mut c)
                .filter_map(|n| if n.kind() == "decorated_definition" { n.child_by_field_name("definition") } else { Some(n) })
                .filter(|d| d.kind() == "function_definition")
                .map(|d| text(d.child_by_field_name("name").unwrap(), &src).to_string())
                .collect();
            if !fns.is_empty() {
                let f = fns[self.rng.below(fns.len())].clone();
                return Some((module_of(p), f));
            }
        }
        None
    }

    fn single_edit(&mut self, group: &str, w: u32) {
        let mut kind = self.pick_kind();
        if kind == "revert" {
            if let Some((p, old, new)) = self.history.get_mut(&w).and_then(|h| h.pop()) {
                self.emit(group, "revert", w, vec![(p, new, old)]);
                return;
            }
            kind = "whitespace"; // nothing to revert yet in this worktree
            *self.kinds.entry("revert_fallback_to_whitespace".into()).or_default() += 1;
        }
        let paths: Vec<(String, H)> = self.state[&w].iter().filter(|(p, _)| in_scope(p)).map(|(p, h)| (p.clone(), *h)).collect();
        for _ in 0..500 {
            let (p, h) = paths[self.rng.below(paths.len())].clone();
            let src = self.bytes(&h);
            let Some((out, what)) = self.transform(w, kind, &p, &src) else { continue };
            if out == src || !self.ok(&out) {
                continue;
            }
            if kind == "whitespace" {
                let a = facts::extract(&src, &facts::parse(&mut self.parser, &src));
                let b = facts::extract(&out, &facts::parse(&mut self.parser, &out));
                assert_eq!(a, b, "whitespace edit changed facts");
            }
            let nh = self.put(&out);
            self.history.entry(w).or_default().push((p.clone(), Some(h), Some(nh)));
            let _ = what;
            self.emit(group, kind, w, vec![(p, Some(h), Some(nh))]);
            return;
        }
        panic!("no {kind} edit found for worktree {w}");
    }

    fn bulk(&mut self, w: u32, v: u32) {
        let a = self.state[&w].clone();
        let b = self.state[&v].clone();
        let mut diff: Vec<(String, Option<H>, Option<H>)> =
            b.iter().filter(|(p, h)| a.get(*p) != Some(h)).map(|(p, h)| (p.clone(), a.get(p).copied(), Some(*h))).collect();
        // deterministic sample of 200
        let mut out = Vec::new();
        while out.len() < 200 && !diff.is_empty() {
            let i = self.rng.below(diff.len());
            out.push(diff.swap_remove(i));
        }
        out.sort();
        self.emit("bulk", &format!("replace {} files with worktree {v:02}", out.len()), w, out);
    }
}

fn in_scope(p: &str) -> bool {
    let first = p.split('/').next().unwrap();
    p.contains('/') && !p.split('/').any(|d| d == "tests" || d == "test") && (first.starts_with('l') || ["impl", "data_layer", "_internal", "api"].contains(&first))
}

pub fn run(out_dir: &Path, seed: u64, corpus: &Path, commits_repo: Option<&Path>) {
    std::fs::create_dir_all(out_dir).unwrap();
    let contents_dir = store::spike_dir().join("contents");
    std::fs::create_dir_all(&contents_dir).unwrap();
    let mdir = store::spike_dir().join("membership");
    let base_m: Membership = store::read_membership(&mdir.join("base.tsv"));
    let ms = base::load_memberships();
    let mut sources: BTreeMap<H, PathBuf> = BTreeMap::new();
    let all: Vec<String> = (1..=47).map(|i| format!("{i:02}")).collect();
    for w in &all {
        for (p, h) in store::read_membership(&mdir.join(format!("{w}.tsv"))) {
            sources.entry(h).or_insert_with(|| corpus.join(w).join(&p));
        }
    }
    for e in std::fs::read_dir(&contents_dir).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if let Some(hex) = n.strip_suffix(".py") {
            sources.entry(store::parse_hex(hex)).or_insert(e.path());
        }
    }
    let mut g = Gen {
        rng: Rng(seed),
        parser: facts::parser(),
        sources,
        contents_dir,
        store_dir: store::store_dir(),
        state: ms.iter().enumerate().map(|(i, (_, m))| (i as u32 + 1, m.iter().cloned().collect())).collect(),
        history: BTreeMap::new(),
        events: Vec::new(),
        kinds: BTreeMap::new(),
        seq: 1,
        new_contents: 0,
    };
    let base_state: BTreeMap<String, H> = base_m.iter().cloned().collect();
    let bulk_at = [150, 350, 550, 750, 950];
    let bulk_pairs = [(1u32, 40u32), (10, 30), (20, 5), (35, 15), (40, 2)];
    let new_at = [100, 300, 500, 700, 900];
    for i in 0..1000 {
        if let Some(k) = new_at.iter().position(|&x| x == i) {
            let w = 41 + k as u32;
            g.state.insert(w, base_state.clone());
            g.events.push(json!({"seq": g.seq, "group": "new_worktree", "kind": "clone of base", "worktree": w, "changes": []}));
            g.seq += 1;
            for _ in 0..10 {
                g.single_edit("new_worktree_edit", w);
            }
        }
        if let Some(k) = bulk_at.iter().position(|&x| x == i) {
            let (w, v) = bulk_pairs[k];
            g.bulk(w, v);
        }
        let w = (i % base::WORKTREES) as u32 + 1;
        g.single_edit("edit", w);
    }
    // Real commits around 2b1504adc, replayed as bulk events on a fresh worktree.
    let mut commit_summary = json!(null);
    if let Some(repo) = commits_repo {
        commit_summary = commits(&mut g, repo, &base_state);
    }
    let mut s = String::new();
    for e in &g.events {
        s.push_str(&serde_json::to_string(e).unwrap());
        s.push('\n');
    }
    std::fs::write(out_dir.join("events.jsonl"), s).unwrap();
    let summary = json!({"seed": seed, "events": g.events.len(), "kinds": g.kinds, "new_contents": g.new_contents, "commits": commit_summary});
    std::fs::write(out_dir.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
}

fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let o = std::process::Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    o.stdout
}

fn tree_py(repo: &Path, commit: &str) -> BTreeMap<String, String> {
    let out = git(repo, &["ls-tree", "-r", "--full-tree", commit]);
    String::from_utf8(out)
        .unwrap()
        .lines()
        .filter_map(|l| {
            let (meta, path) = l.split_once('\t')?;
            let mut it = meta.split_whitespace();
            let (mode, typ, sha) = (it.next()?, it.next()?, it.next()?);
            (typ == "blob" && mode != "120000" && path.ends_with(".py")).then(|| (path.to_string(), sha.to_string()))
        })
        .collect()
}

fn commits(g: &mut Gen, repo: &Path, base_state: &BTreeMap<String, H>) -> serde_json::Value {
    let list = String::from_utf8(git(repo, &["rev-list", "--first-parent", "-n", "51", "2b1504adc"])).unwrap();
    let mut cs: Vec<String> = list.lines().map(String::from).collect();
    cs.reverse();
    let w = 50u32;
    let mut blob_hash: BTreeMap<String, H> = BTreeMap::new();
    let mut resolve = |g: &mut Gen, sha: &str| -> H {
        if let Some(h) = blob_hash.get(sha) {
            return *h;
        }
        let bytes = git(repo, &["cat-file", "blob", sha]);
        let h = g.put(&bytes);
        blob_hash.insert(sha.to_string(), h);
        h
    };
    g.state.insert(w, base_state.clone());
    let mut prev: BTreeMap<String, H> = base_state.clone();
    let mut sizes = Vec::new();
    for (i, c) in cs.iter().enumerate() {
        let t = tree_py(repo, c);
        let cur: BTreeMap<String, H> = t.iter().map(|(p, sha)| (p.clone(), resolve(g, sha))).collect();
        let mut changes = Vec::new();
        for (p, h) in &cur {
            if prev.get(p) != Some(h) {
                changes.push((p.clone(), prev.get(p).copied(), Some(*h)));
            }
        }
        for (p, h) in &prev {
            if !cur.contains_key(p) {
                changes.push((p.clone(), Some(*h), None));
            }
        }
        changes.sort();
        sizes.push(changes.len());
        let group = if i == 0 { "commit_init" } else { "commit" };
        g.emit(group, &c[..10], w, changes);
        prev = cur;
    }
    json!({"repo": repo.display().to_string(), "first": cs[0], "last": cs[cs.len() - 1], "changes_per_event": sizes})
}
