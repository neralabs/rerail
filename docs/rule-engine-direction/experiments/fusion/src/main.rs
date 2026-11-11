//! Fusion experiment: a minimal faithful copy of the Tempo transducer core loop (path-enumerating,
//! one u32 word per instruction, fork-per-navigation-result) over a regular node table, plus the
//! proposed EXISTS / NOT_EXISTS / ANCESTOR / ACCEPT extensions. Compares separate execution with
//! fused execution for multi-rule batches and for AND / OR / NOT on one root.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path as FsPath;
use std::time::Instant;

// ---------- regular node table (like Tempo's packed batch node table) ----------
struct Doc {
    kind: Vec<u16>,
    field: Vec<u16>,
    named: Vec<bool>,
    parent: Vec<u32>,   // u32::MAX for root
    next_sib: Vec<u32>, // u32::MAX if none
    first_child: Vec<u32>,
    end: Vec<u32>, // preorder exclusive end of subtree
}

const NONE: u32 = u32::MAX;

fn build_doc(tree: &tree_sitter::Tree) -> Doc {
    let mut d = Doc { kind: vec![], field: vec![], named: vec![], parent: vec![], next_sib: vec![], first_child: vec![], end: vec![] };
    let mut cursor = tree.walk();
    let mut stack: Vec<u32> = vec![]; // open ancestors
    let mut last_child: Vec<u32> = vec![]; // per open ancestor: last child seen
    loop {
        let node = cursor.node();
        let id = d.kind.len() as u32;
        d.kind.push(node.kind_id());
        d.field.push(cursor.field_id().map_or(0, |f| f.get()));
        d.named.push(node.is_named());
        let parent = stack.last().copied().unwrap_or(NONE);
        d.parent.push(parent);
        d.next_sib.push(NONE);
        d.first_child.push(NONE);
        d.end.push(0);
        if parent != NONE {
            let lc = last_child.last_mut().unwrap();
            if *lc == NONE { d.first_child[parent as usize] = id; } else { d.next_sib[*lc as usize] = id; }
            *lc = id;
        }
        if cursor.goto_first_child() {
            stack.push(id);
            last_child.push(NONE);
            continue;
        }
        d.end[id as usize] = id + 1;
        loop {
            if cursor.goto_next_sibling() { break; }
            if !cursor.goto_parent() {
                return d;
            }
            let p = stack.pop().unwrap();
            last_child.pop();
            d.end[p as usize] = d.kind.len() as u32;
        }
    }
}

// ---------- IR ----------
const STOP: u8 = 0;
const KIND: u8 = 1;
const FIELD: u8 = 2;
const NAMEDF: u8 = 3; // require named
const CHILD: u8 = 0x10;
const NEXT: u8 = 0x11;
const PARENT: u8 = 0x12;
const DESCEND: u8 = 0x13;
const ANCESTOR: u8 = 0x17; // proposed: fork one path per proper ancestor
const FORK: u8 = 0x30;
const EMIT: u8 = 0x40;
const EXISTS: u8 = 0x50; // proposed: isolated sub-walk, accept/reject, no paths leak
const NOT_EXISTS: u8 = 0x51;
const ACCEPT: u8 = 0x52; // ends a sub-walk with success
const DESCEND_KIND: u8 = 0x18; // peephole: DESCEND then KIND k, filtered before forking
const NOP: u8 = 0x60;

/// Resumes a lazy descendant scan: `path.node` is the next candidate, `path.end` the range end,
/// `path.pc` the DESCEND/DESCEND_KIND instruction. Returns (continuation, remaining scan).
#[inline]
fn resume(p: &Program, d: &Doc, path: PathState) -> Option<(PathState, Option<PathState>)> {
    let (op, mode, operand) = split(p.words[path.pc as usize]);
    let named = mode == 1;
    let mut c = path.node;
    while c < path.end {
        let ok = (!named || d.named[c as usize]) && (op != DESCEND_KIND || d.kind[c as usize] == operand);
        if ok {
            let rest = if c + 1 < path.end { Some(PathState { pc: path.pc, node: c + 1, end: path.end }) } else { None };
            return Some((PathState { pc: path.pc + 1, node: c, end: 0 }, rest));
        }
        c += 1;
    }
    None
}

fn w(op: u8, mode: u8, operand: u16) -> u32 { (op as u32) << 24 | (mode as u32) << 16 | operand as u32 }
fn split(word: u32) -> (u8, u8, u16) { ((word >> 24) as u8, (word >> 16) as u8, word as u16) }

#[derive(Clone)]
struct Program { words: Vec<u32>, masks: Vec<u64> }

impl Program {
    /// Labels reachable from each pc (jumps are forward-only in these programs).
    fn new(mut words: Vec<u32>) -> Self {
        if std::env::var("PEEP").is_ok() {
            for i in 0..words.len().saturating_sub(1) {
                let (op, mode, _) = split(words[i]);
                let (op2, _, k) = split(words[i + 1]);
                if op == DESCEND && op2 == KIND { words[i] = w(DESCEND_KIND, mode, k); words[i + 1] = w(NOP, 0, 0); }
            }
        }
        let mut masks = vec![0u64; words.len() + 1];
        for pc in (0..words.len()).rev() {
            let (op, _, operand) = split(words[pc]);
            masks[pc] = match op {
                EMIT => 1u64 << (operand & 63),
                STOP | ACCEPT => 0,
                FORK => masks[pc + 1] | masks[operand as usize],
                _ => masks[pc + 1],
            };
        }
        Program { words, masks }
    }
}

#[derive(Clone, Copy)]
struct PathState { pc: u16, node: u32, end: u32 }

struct Exec { pending: Vec<PathState>, sub_pending: Vec<PathState>, steps: u64, pushes: u64, lazy: bool }

impl Exec {
    fn new() -> Self { Exec { pending: Vec::with_capacity(256), sub_pending: Vec::with_capacity(256), steps: 0, pushes: 0, lazy: std::env::var("LAZY").is_ok() } }

    /// Runs one task (program, root) from `pc`. Emits (label, root). With `set_mode`, a label is
    /// emitted at most once per task and a single-label task stops at its first emit.
    fn run(&mut self, p: &Program, d: &Doc, root: u32, pc: u16, set_mode: bool, single_label: bool, out: &mut Vec<(u16, u32)>) {
        self.pending.clear();
        self.pushes += 1; self.pending.push(PathState { pc, node: root, end: 0 });
        let mut emitted: u64 = 0; // label bitset (labels < 64 within a task in set mode)
        let prune = set_mode && !single_label;
        'paths: while let Some(mut path) = self.pending.pop() {
            if path.end != 0 {
                match resume(p, d, path) {
                    Some((cont, rest)) => { if let Some(r) = rest { self.pushes += 1; self.pending.push(r); } path = cont; }
                    None => continue 'paths,
                }
            }
            loop {
                if prune && p.masks[path.pc as usize] & !emitted == 0 { continue 'paths; }
                self.steps += 1;
                let (op, mode, operand) = split(p.words[path.pc as usize]);
                let next = path.pc + 1;
                let n = path.node as usize;
                match op {
                    STOP => continue 'paths,
                    KIND => if d.kind[n] != operand { continue 'paths },
                    FIELD => if d.field[n] != operand { continue 'paths },
                    NAMEDF => if !d.named[n] { continue 'paths },
                    CHILD => {
                        let named = mode == 1;
                        let mut c = d.first_child[n];
                        let mut first = NONE;
                        // collect like Tempo (forks pushed in reverse so first child runs first)
                        let base = self.pending.len();
                        while c != NONE {
                            if !named || d.named[c as usize] {
                                if first == NONE { first = c; } else { self.pushes += 1; self.pending.push(PathState { pc: next, node: c, end: 0 }); }
                            }
                            c = d.next_sib[c as usize];
                        }
                        self.pending[base..].reverse();
                        if first == NONE { continue 'paths; }
                        path.node = first;
                    }
                    NOP => {}
                    DESCEND | DESCEND_KIND if self.lazy => {
                        let start = PathState { pc: path.pc, node: path.node + 1, end: d.end[n] };
                        if start.node >= start.end { continue 'paths; }
                        match resume(p, d, start) {
                            Some((cont, rest)) => { if let Some(r) = rest { self.pushes += 1; self.pending.push(r); } path = cont; continue; }
                            None => continue 'paths,
                        }
                    }
                    DESCEND_KIND => {
                        let named = mode == 1;
                        let (s, e) = (path.node + 1, d.end[n]);
                        let mut first = NONE;
                        for c in (s..e).rev() {
                            if (!named || d.named[c as usize]) && d.kind[c as usize] == operand {
                                if first != NONE { self.pushes += 1; self.pending.push(PathState { pc: next, node: first, end: 0 }); }
                                first = c;
                            }
                        }
                        if first == NONE { continue 'paths; }
                        path.node = first;
                    }
                    DESCEND => {
                        let named = mode == 1;
                        let (s, e) = (path.node + 1, d.end[n]);
                        let mut first = NONE;
                        for c in (s..e).rev() {
                            if !named || d.named[c as usize] {
                                if first != NONE { self.pushes += 1; self.pending.push(PathState { pc: next, node: first, end: 0 }); }
                                first = c;
                            }
                        }
                        if first == NONE { continue 'paths; }
                        path.node = first;
                    }
                    ANCESTOR => {
                        let mut a = d.parent[n];
                        if a == NONE { continue 'paths; }
                        let first = a;
                        a = d.parent[a as usize];
                        let base = self.pending.len();
                        while a != NONE { self.pushes += 1; self.pending.push(PathState { pc: next, node: a, end: 0 }); a = d.parent[a as usize]; }
                        self.pending[base..].reverse();
                        path.node = first;
                    }
                    NEXT => {
                        let named = mode == 1;
                        let mut s = d.next_sib[n];
                        while named && s != NONE && !d.named[s as usize] { s = d.next_sib[s as usize]; }
                        if s == NONE { continue 'paths; }
                        path.node = s;
                    }
                    PARENT => {
                        let a = d.parent[n];
                        if a == NONE { continue 'paths; }
                        path.node = a;
                    }
                    FORK => { self.pushes += 1; self.pending.push(PathState { pc: operand, node: path.node, end: 0 }); }
                    EMIT => {
                        if set_mode {
                            let bit = 1u64 << (operand & 63);
                            if emitted & bit == 0 { emitted |= bit; out.push((operand, root)); }
                            if single_label { self.pending.clear(); return; }
                        } else {
                            out.push((operand, root));
                        }
                        continue 'paths;
                    }
                    EXISTS | NOT_EXISTS => {
                        let found = self.exists(p, d, path.node, operand);
                        if found != (op == EXISTS) { continue 'paths; }
                    }
                    _ => panic!("bad op {op}"),
                }
                path.pc = next;
            }
        }
    }

    /// Isolated sub-walk starting at `pc` from `node`: true at the first ACCEPT.
    fn exists(&mut self, p: &Program, d: &Doc, node: u32, pc: u16) -> bool {
        let base = self.sub_pending.len();
        self.pushes += 1; self.sub_pending.push(PathState { pc, node, end: 0 });
        let mut result = false;
        'paths: while self.sub_pending.len() > base {
            let mut path = self.sub_pending.pop().unwrap();
            if path.end != 0 {
                match resume(p, d, path) {
                    Some((cont, rest)) => { if let Some(r) = rest { self.pushes += 1; self.sub_pending.push(r); } path = cont; }
                    None => continue 'paths,
                }
            }
            loop {
                self.steps += 1;
                let (op, mode, operand) = split(p.words[path.pc as usize]);
                let next = path.pc + 1;
                let n = path.node as usize;
                match op {
                    STOP => continue 'paths,
                    KIND => if d.kind[n] != operand { continue 'paths },
                    FIELD => if d.field[n] != operand { continue 'paths },
                    NAMEDF => if !d.named[n] { continue 'paths },
                    CHILD => {
                        let named = mode == 1;
                        let mut c = d.first_child[n];
                        let mut first = NONE;
                        let b2 = self.sub_pending.len();
                        while c != NONE {
                            if !named || d.named[c as usize] {
                                if first == NONE { first = c; } else { self.pushes += 1; self.sub_pending.push(PathState { pc: next, node: c, end: 0 }); }
                            }
                            c = d.next_sib[c as usize];
                        }
                        self.sub_pending[b2..].reverse();
                        if first == NONE { continue 'paths; }
                        path.node = first;
                    }
                    NOP => {}
                    DESCEND | DESCEND_KIND if self.lazy => {
                        let start = PathState { pc: path.pc, node: path.node + 1, end: d.end[n] };
                        if start.node >= start.end { continue 'paths; }
                        match resume(p, d, start) {
                            Some((cont, rest)) => { if let Some(r) = rest { self.pushes += 1; self.sub_pending.push(r); } path = cont; continue; }
                            None => continue 'paths,
                        }
                    }
                    DESCEND_KIND => {
                        let named = mode == 1;
                        let (s, e) = (path.node + 1, d.end[n]);
                        let mut first = NONE;
                        for c in (s..e).rev() {
                            if (!named || d.named[c as usize]) && d.kind[c as usize] == operand {
                                if first != NONE { self.pushes += 1; self.sub_pending.push(PathState { pc: next, node: first, end: 0 }); }
                                first = c;
                            }
                        }
                        if first == NONE { continue 'paths; }
                        path.node = first;
                    }
                    DESCEND => {
                        // existence over a contiguous preorder range: no forks needed when the
                        // remaining sub-program is a single test sequence; keep it faithful (fork).
                        let named = mode == 1;
                        let (s, e) = (path.node + 1, d.end[n]);
                        let mut first = NONE;
                        for c in (s..e).rev() {
                            if !named || d.named[c as usize] {
                                if first != NONE { self.pushes += 1; self.sub_pending.push(PathState { pc: next, node: first, end: 0 }); }
                                first = c;
                            }
                        }
                        if first == NONE { continue 'paths; }
                        path.node = first;
                    }
                    ANCESTOR => {
                        let mut a = d.parent[n];
                        if a == NONE { continue 'paths; }
                        let first = a;
                        a = d.parent[a as usize];
                        let b2 = self.sub_pending.len();
                        while a != NONE { self.pushes += 1; self.sub_pending.push(PathState { pc: next, node: a, end: 0 }); a = d.parent[a as usize]; }
                        self.sub_pending[b2..].reverse();
                        path.node = first;
                    }
                    NEXT => {
                        let named = mode == 1;
                        let mut s = d.next_sib[n];
                        while named && s != NONE && !d.named[s as usize] { s = d.next_sib[s as usize]; }
                        if s == NONE { continue 'paths; }
                        path.node = s;
                    }
                    PARENT => {
                        let a = d.parent[n];
                        if a == NONE { continue 'paths; }
                        path.node = a;
                    }
                    FORK => { self.pushes += 1; self.sub_pending.push(PathState { pc: operand, node: path.node, end: 0 }); }
                    ACCEPT => { result = true; break 'paths; }
                    EXISTS | NOT_EXISTS => {
                        let found = self.exists(p, d, path.node, operand);
                        if found != (op == EXISTS) { continue 'paths; }
                    }
                    _ => panic!("bad sub op {op}"),
                }
                path.pc = next;
            }
        }
        self.sub_pending.truncate(base);
        result
    }
}

// ---------- program templates ----------
#[derive(Clone, Debug)]
enum Tmpl {
    ChildField { p: u16, f: u16, c: u16 },   // KIND p; CHILD; FIELD f; KIND c; EMIT
    Has { p: u16, c: u16 },                  // KIND p; DESCEND named; KIND c; EMIT   (path-enumerating)
    Inside { c: u16, p: u16 },               // KIND c; ANCESTOR; KIND p; EMIT
    Sibling { c: u16, s: u16 },              // KIND c; NEXT named; KIND s; EMIT
}

impl Tmpl {
    fn root(&self) -> u16 { match *self { Tmpl::ChildField { p, .. } | Tmpl::Has { p, .. } => p, Tmpl::Inside { c, .. } | Tmpl::Sibling { c, .. } => c } }
    fn words(&self, label: u16) -> Vec<u32> {
        match *self {
            Tmpl::ChildField { p, f, c } => vec![w(KIND, 0, p), w(CHILD, 0, 0), w(FIELD, 0, f), w(KIND, 0, c), w(EMIT, 0, label)],
            Tmpl::Has { p, c } => vec![w(KIND, 0, p), w(DESCEND, 1, 0), w(KIND, 0, c), w(EMIT, 0, label)],
            Tmpl::Inside { c, p } => vec![w(KIND, 0, c), w(ANCESTOR, 0, 0), w(KIND, 0, p), w(EMIT, 0, label)],
            Tmpl::Sibling { c, s } => vec![w(KIND, 0, c), w(NEXT, 1, 0), w(KIND, 0, s), w(EMIT, 0, label)],
        }
    }
}

/// Merges straight-line programs (no jumps; each ends in EMIT) into one trie program: a shared
/// prefix runs once and a FORK splits at each divergence. Every program keeps its own EMIT label.
fn merge_trie(progs: &[Vec<u32>]) -> Vec<u32> {
    #[derive(Default)]
    struct T { kids: BTreeMap<u32, T>, order: Vec<u32> }
    let mut root = T::default();
    for p in progs {
        let mut t = &mut root;
        for &word in p {
            if !t.kids.contains_key(&word) { t.order.push(word); }
            t = t.kids.entry(word).or_default();
        }
    }
    fn lower(t: &T, out: &mut Vec<u32>) {
        // children c0..cn: FORK to c1..cn (patched), fall through to c0.
        let n = t.order.len();
        let mut fork_slots = vec![];
        for _ in 1..n { fork_slots.push(out.len()); out.push(0); }
        for (i, word) in t.order.iter().enumerate() {
            if i > 0 { let at = out.len() as u16; out[fork_slots[i - 1]] = w(FORK, 0, at); }
            out.push(*word);
            let (op, _, _) = split(*word);
            if op != EMIT { lower(&t.kids[word], out); }
        }
    }
    let mut out = vec![];
    lower(&root, &mut out);
    out
}

// ---------- harness ----------
fn load(dir: &FsPath, lang: &tree_sitter::Language, max_files: usize) -> Vec<Doc> {
    let mut files = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(p) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&p) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    let name = path.file_name().unwrap().to_string_lossy().to_string();
                    if name != "site-packages" && name != "__pycache__" { stack.push(path); }
                } else if path.extension().is_some_and(|x| x == "py") { files.push(path); }
            }
        }
    }
    files.sort();
    files.truncate(max_files);
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(lang).unwrap();
    files.iter().filter_map(|f| {
        let src = std::fs::read(f).ok()?;
        let tree = parser.parse(&src, None)?;
        Some(build_doc(&tree))
    }).collect()
}

fn time<F: FnMut() -> R, R>(reps: usize, mut f: F) -> (f64, R) {
    let mut best = f64::MAX;
    let mut r = None;
    for _ in 0..reps {
        let t = Instant::now();
        let x = f();
        best = best.min(t.elapsed().as_secs_f64());
        r = Some(x);
    }
    (best, r.unwrap())
}

type Found = Vec<(u16, u32, u32)>; // (label, doc, root)

fn norm(mut v: Found) -> Found { v.sort_unstable(); v.dedup(); v }

fn main() {
    let dir = std::env::args().nth(1).expect("corpus dir");
    let max_files: usize = std::env::args().nth(2).map_or(100_000, |s| s.parse().unwrap());
    let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    let t0 = Instant::now();
    let docs = load(FsPath::new(&dir), &lang, max_files);
    let nodes: usize = docs.iter().map(|d| d.kind.len()).sum();
    println!("corpus: {} files, {} nodes, load {:.2}s", docs.len(), nodes, t0.elapsed().as_secs_f64());

    // Kind postings per doc: the stand-in for the FTS kind index (built once, shared).
    let (t_post, postings) = time(3, || docs.iter().map(|d| {
        let mut m: HashMap<u16, Vec<u32>> = HashMap::new();
        for (i, &k) in d.kind.iter().enumerate() { m.entry(k).or_default().push(i as u32); }
        m
    }).collect::<Vec<_>>());
    println!("kind postings build: {:.1} ms ({:.2} ns/node)", t_post * 1e3, t_post * 1e9 / nodes as f64);

    // ---- harvest realistic templates from corpus statistics ----
    let mut cf: HashMap<(u16, u16, u16), usize> = HashMap::new();
    let mut sib: HashMap<(u16, u16), usize> = HashMap::new();
    let mut kindfreq: HashMap<u16, usize> = HashMap::new();
    for d in &docs {
        for i in 0..d.kind.len() {
            if !d.named[i] { continue; }
            *kindfreq.entry(d.kind[i]).or_default() += 1;
            let p = d.parent[i];
            if p != NONE && d.field[i] != 0 { *cf.entry((d.kind[p as usize], d.field[i], d.kind[i])).or_default() += 1; }
            let mut s = d.next_sib[i];
            while s != NONE && !d.named[s as usize] { s = d.next_sib[s as usize]; }
            if s != NONE { *sib.entry((d.kind[i], d.kind[s as usize])).or_default() += 1; }
        }
    }
    let mut cfv: Vec<_> = cf.into_iter().collect();
    cfv.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut sibv: Vec<_> = sib.into_iter().collect();
    sibv.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    // Pick templates by striding through the frequency ranking (mix of common and rare).
    let mut all: Vec<Tmpl> = vec![];
    for (i, ((p, f, c), _)) in cfv.iter().enumerate() {
        if i % 2 == 0 { all.push(Tmpl::ChildField { p: *p, f: *f, c: *c }); }
        if i % 5 == 1 { all.push(Tmpl::Has { p: *p, c: *c }); }
        if i % 7 == 3 { all.push(Tmpl::Inside { c: *c, p: *p }); }
    }
    for (i, ((c, s), _)) in sibv.iter().enumerate() { if i % 3 == 0 { all.push(Tmpl::Sibling { c: *c, s: *s }); } }
    // interleave deterministically
    let mut rng = 0x9e3779b97f4a7c15u64;
    for i in (1..all.len()).rev() { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; all.swap(i, (rng % (i as u64 + 1)) as usize); }
    println!("template pool: {}", all.len());

    let reps = 5;
    println!("\n== Multi-rule co-execution (set semantics; outputs = distinct (rule, doc, root)) ==");
    println!("{:>4} {:>8} | {:>11} {:>11} {:>11} {:>11} | {:>9} {:>9} {:>9}", "K", "outputs", "sep-unanch", "sep-anch", "fused-disp", "fused-trie", "tasks-sa", "tasks-fd", "tasks-ft");
    for &k in &[8usize, 32, 128, 512] {
        let k = k.min(all.len());
        let tmpls = &all[..k];
        let progs: Vec<Program> = tmpls.iter().enumerate().map(|(i, t)| Program::new(t.words(i as u16))).collect();
        // (a) separate, unanchored: one task per (program, node), as with no index.
        let run_unanchored = k <= 32;
        let (ta, ra) = time(if run_unanchored { 2 } else { 0 }.max(1), || {
            if !run_unanchored { return Found::new(); }
            let mut ex = Exec::new();
            let mut found: Found = vec![];
            let mut out = vec![];
            for (pi, p) in progs.iter().enumerate() {
                for (di, d) in docs.iter().enumerate() {
                    for root in 0..d.kind.len() as u32 {
                        out.clear();
                        ex.run(p, d, root, 0, true, true, &mut out);
                        found.extend(out.iter().map(|&(_, r)| (pi as u16, di as u32, r)));
                    }
                }
            }
            found
        });
        // (b) separate, anchored: one task per (program, root in kind posting).
        let (tb, (rb, tasks_b)) = time(reps, || {
            let mut ex = Exec::new();
            let mut found: Found = vec![];
            let mut out = vec![];
            let mut tasks = 0u64;
            for (pi, p) in progs.iter().enumerate() {
                let rk = tmpls[pi].root();
                for (di, d) in docs.iter().enumerate() {
                    if let Some(roots) = postings[di].get(&rk) {
                        for &root in roots {
                            tasks += 1;
                            out.clear();
                            ex.run(p, d, root, 1, true, true, &mut out);
                            found.extend(out.iter().map(|&(_, r)| (pi as u16, di as u32, r)));
                        }
                    }
                }
            }
            (found, tasks)
        });
        // (c) fused dispatch: one walk over nodes; a kind table selects the programs to run.
        let mut by_kind: HashMap<u16, Vec<usize>> = HashMap::new();
        for (pi, t) in tmpls.iter().enumerate() { by_kind.entry(t.root()).or_default().push(pi); }
        let nkinds = 65536usize;
        let mut disp: Vec<Vec<usize>> = vec![Vec::new(); nkinds];
        for (&kind, ps) in &by_kind { disp[kind as usize] = ps.clone(); }
        let (tc, (rc, tasks_c)) = time(reps, || {
            let mut ex = Exec::new();
            let mut found: Found = vec![];
            let mut out = vec![];
            let mut tasks = 0u64;
            for (di, d) in docs.iter().enumerate() {
                for (root, k) in d.kind.iter().enumerate() {
                    let ps = &disp[*k as usize];
                    if !ps.is_empty() {
                        tasks += 1;
                        for &pi in ps {
                            out.clear();
                            ex.run(&progs[pi], d, root as u32, 1, true, true, &mut out);
                            found.extend(out.iter().map(|&(_, r)| (pi as u16, di as u32, r)));
                        }
                    }
                }
            }
            (found, tasks)
        });
        // (d) fused trie: per root kind, one merged program (shared prefixes, FORK at divergence).
        let tries: HashMap<u16, (Program, usize)> = by_kind.iter().map(|(&kind, ps)| {
            let words: Vec<Vec<u32>> = ps.iter().enumerate().map(|(local, &pi)| tmpls[pi].words(local as u16)[1..].to_vec()).collect();
            (kind, (Program::new(merge_trie(&words)), ps.len()))
        }).collect();
        let sep_words: usize = progs.iter().map(|p| p.words.len()).sum();
        let trie_words: usize = tries.values().map(|(p, _)| p.words.len() + 1).sum();
        let max_group = by_kind.values().map(Vec::len).max().unwrap_or(0);
        eprintln!("K={k}: root kinds {}, max programs per root kind {}, words separate {} vs fused trie {} ({:.2}x)", by_kind.len(), max_group, sep_words, trie_words, sep_words as f64 / trie_words as f64);
        let mut tdisp: Vec<Option<(Program, usize)>> = vec![None; nkinds];
        for (&kind, v) in &tries { tdisp[kind as usize] = Some(v.clone()); }
        let (td, (rd, tasks_d)) = time(reps, || {
            let mut ex = Exec::new();
            let mut found: Found = vec![];
            let mut out = vec![];
            let mut tasks = 0u64;
            for (di, d) in docs.iter().enumerate() {
                for (root, k) in d.kind.iter().enumerate() {
                    if let Some((p, nlabels)) = &tdisp[*k as usize] {
                        tasks += 1;
                        out.clear();
                        // set mode with labels < 64 per task; otherwise dedupe after.
                        let set = *nlabels <= 64;
                        ex.run(p, d, root as u32, 0, set, *nlabels == 1, &mut out);
                        let ps = &disp[*k as usize];
                        found.extend(out.iter().map(|&(l, r)| (ps[l as usize] as u16, di as u32, r)));
                    }
                }
            }
            (found, tasks)
        });
        let (ra, rb, rc, rd) = (norm(ra), norm(rb), norm(rc), norm(rd));
        let ra = if run_unanchored { ra } else { rb.clone() };
        let ta = if run_unanchored { ta } else { f64::NAN };
        assert!(ra == rb && rb == rc && rc == rd, "mismatch at K={k}: {} {} {} {}", ra.len(), rb.len(), rc.len(), rd.len());
        let ns = |t: f64| t * 1e9 / nodes as f64;
        println!("{:>4} {:>8} | {:>9.2}ns {:>9.2}ns {:>9.2}ns {:>9.2}ns | {:>9} {:>9} {:>9}", k, ra.len(), ns(ta), ns(tb), ns(tc), ns(td), tasks_b, tasks_c, tasks_d);
    }

    // ---- shared one-pass relation masks for has/inside (tree "Aho-Corasick") ----
    {
        println!("\n== has/inside batch: per-program walks vs one shared mask pass ==");
        let has: Vec<(u16, u16)> = all.iter().filter_map(|t| if let Tmpl::Has { p, c } = *t { Some((p, c)) } else { None }).collect();
        let ins: Vec<(u16, u16)> = all.iter().filter_map(|t| if let Tmpl::Inside { c, p } = *t { Some((c, p)) } else { None }).collect();
        // targets: kinds tested under DESCEND (has) and ANCESTOR (inside); up to 64 each.
        let mut ht: Vec<u16> = has.iter().map(|x| x.1).collect(); ht.sort(); ht.dedup(); ht.truncate(64);
        let mut it: Vec<u16> = ins.iter().map(|x| x.1).collect(); it.sort(); it.dedup(); it.truncate(64);
        let has: Vec<(u16, u16)> = has.into_iter().filter(|x| ht.contains(&x.1)).collect();
        let ins: Vec<(u16, u16)> = ins.into_iter().filter(|x| it.contains(&x.1)).collect();
        let mut hbit = vec![0u64; 65536]; for (i, &k) in ht.iter().enumerate() { hbit[k as usize] = 1 << i; }
        let mut ibit = vec![0u64; 65536]; for (i, &k) in it.iter().enumerate() { ibit[k as usize] = 1 << i; }
        println!("   {} has-programs over {} target kinds; {} inside-programs over {} target kinds", has.len(), ht.len(), ins.len(), it.len());
        let hp: Vec<Program> = has.iter().enumerate().map(|(i, &(p, c))| Program::new(Tmpl::Has { p, c }.words(i as u16))).collect();
        let ip: Vec<Program> = ins.iter().enumerate().map(|(i, &(c, p))| Program::new(Tmpl::Inside { c, p }.words(i as u16))).collect();
        let (t_walk, n_walk) = time(3, || {
            let mut ex = Exec::new(); let mut out = vec![]; let mut n = 0usize;
            for (progs, roots) in [(&hp, has.iter().map(|x| x.0).collect::<Vec<_>>()), (&ip, ins.iter().map(|x| x.0).collect::<Vec<_>>())] {
                for (pi, p) in progs.iter().enumerate() {
                    for (di, d) in docs.iter().enumerate() {
                        if let Some(rs) = postings[di].get(&roots[pi]) {
                            for &r in rs { out.clear(); ex.run(p, d, r, 1, true, true, &mut out); n += out.len(); }
                        }
                    }
                }
            }
            n
        });
        let (t_mask, n_mask) = time(3, || {
            let mut n = 0usize;
            let mut sub: Vec<u64> = vec![];
            let mut anc: Vec<u64> = vec![];
            for (di, d) in docs.iter().enumerate() {
                let len = d.kind.len();
                // one reverse-preorder pass: strict-subtree kind masks
                sub.clear(); sub.resize(len, 0);
                for i in (1..len).rev() {
                    let p = d.parent[i] as usize;
                    sub[p] |= sub[i] | hbit[d.kind[i] as usize];
                }
                // one preorder pass: strict-ancestor kind masks
                anc.clear(); anc.resize(len, 0);
                for i in 1..len {
                    let p = d.parent[i] as usize;
                    anc[i] = anc[p] | ibit[d.kind[p] as usize];
                }
                for &(p, c) in &has { if let Some(rs) = postings[di].get(&p) { let b = hbit[c as usize]; n += rs.iter().filter(|&&r| sub[r as usize] & b != 0).count(); } }
                for &(c, p) in &ins { if let Some(rs) = postings[di].get(&c) { let b = ibit[p as usize]; n += rs.iter().filter(|&&r| anc[r as usize] & b != 0).count(); } }
            }
            n
        });
        assert_eq!(n_walk, n_mask);
        let ns = |t: f64| t * 1e9 / nodes as f64;
        println!("   per-program walks: {:.2} ns/node; shared mask passes + bit tests: {:.2} ns/node ({:.1}x); outputs {}", ns(t_walk), ns(t_mask), t_walk / t_mask, n_walk);
    }

    // ---- AND / OR / NOT on one root ----
    println!("\n== Boolean composition on one root (ns per corpus node; outputs = distinct roots) ==");
    let id = |name: &str| lang.id_for_node_kind(name, true);
    let fid = |name: &str| lang.field_id_for_name(name).map_or(0, |f| f.get());
    struct Case { name: &'static str, root: u16, a: Vec<u32>, b: Vec<u32> }
    // a and b are sub-walk bodies (from the root, ending in ACCEPT), also usable as separate programs.
    let cases = vec![
        Case { name: "function_definition: has return / has yield", root: id("function_definition"),
            a: vec![w(DESCEND, 1, 0), w(KIND, 0, id("return_statement")), w(ACCEPT, 0, 0)],
            b: vec![w(DESCEND, 1, 0), w(KIND, 0, id("yield")), w(ACCEPT, 0, 0)] },
        Case { name: "call: function is attribute / has keyword_argument", root: id("call"),
            a: vec![w(CHILD, 0, 0), w(FIELD, 0, fid("function")), w(KIND, 0, id("attribute")), w(ACCEPT, 0, 0)],
            b: vec![w(CHILD, 0, 0), w(FIELD, 0, fid("arguments")), w(CHILD, 1, 0), w(KIND, 0, id("keyword_argument")), w(ACCEPT, 0, 0)] },
        Case { name: "except_clause: inside function / has raise", root: id("except_clause"),
            a: vec![w(ANCESTOR, 0, 0), w(KIND, 0, id("function_definition")), w(ACCEPT, 0, 0)],
            b: vec![w(DESCEND, 1, 0), w(KIND, 0, id("raise_statement")), w(ACCEPT, 0, 0)] },
    ];
    for c in &cases {
        // Separate programs (path-enumerating, as today: EMIT once per witness path; no set mode),
        // then a set operation over root sets. Also the set-mode variant (dedupe + early exit).
        let as_prog = |body: &[u32]| {
            let mut v = vec![w(KIND, 0, c.root)];
            v.extend(body.iter().map(|&x| if split(x).0 == ACCEPT { w(EMIT, 0, 0) } else { x }));
            Program::new(v)
        };
        let pa = as_prog(&c.a);
        let pb = as_prog(&c.b);
        let run_sep = |p: &Program, set_mode: bool| {
            let mut ex = Exec::new();
            let mut out = vec![];
            let mut roots: Vec<(u32, u32)> = vec![];
            let mut raw = 0usize;
            for (di, d) in docs.iter().enumerate() {
                if let Some(rs) = postings[di].get(&c.root) {
                    for &r in rs {
                        out.clear();
                        ex.run(p, d, r, 1, set_mode, true, &mut out);
                        raw += out.len();
                        if !out.is_empty() { roots.push((di as u32, r)); }
                    }
                }
            }
            (roots, raw, ex.pushes)
        };
        // fused programs: KIND root; then EXISTS/NOT_EXISTS subs; subs appended after.
        let fused = |kind: &str| {
            let mut v: Vec<u32> = vec![];
            let sub_a_at; let sub_b_at;
            match kind {
                "and" => { v = vec![w(KIND, 0, c.root), 0, 0, w(EMIT, 0, 0)]; }
                "and_ba" => { v = vec![w(KIND, 0, c.root), 0, 0, w(EMIT, 0, 0)]; }
                "or" => { v = vec![w(KIND, 0, c.root), 0, 0, w(EMIT, 0, 0), 0, w(EMIT, 0, 0)]; }
                "not" => { v = vec![w(KIND, 0, c.root), 0, 0, w(EMIT, 0, 0)]; }
                _ => unreachable!(),
            }
            sub_a_at = v.len() as u16; v.extend(&c.a);
            sub_b_at = v.len() as u16; v.extend(&c.b);
            match kind {
                "and" => { v[1] = w(EXISTS, 0, sub_a_at); v[2] = w(EXISTS, 0, sub_b_at); }
                "and_ba" => { v[1] = w(EXISTS, 0, sub_b_at); v[2] = w(EXISTS, 0, sub_a_at); }
                "or" => { v[1] = w(FORK, 0, 4); v[2] = w(EXISTS, 0, sub_a_at); v[4] = w(EXISTS, 0, sub_b_at); }
                "not" => { v[1] = w(EXISTS, 0, sub_a_at); v[2] = w(NOT_EXISTS, 0, sub_b_at); }
                _ => unreachable!(),
            }
            Program::new(v)
        };
        let run_fused = |p: &Program| {
            let mut ex = Exec::new();
            let mut out = vec![];
            let mut roots: Vec<(u32, u32)> = vec![];
            for (di, d) in docs.iter().enumerate() {
                if let Some(rs) = postings[di].get(&c.root) {
                    for &r in rs {
                        out.clear();
                        ex.run(p, d, r, 1, true, true, &mut out);
                        if !out.is_empty() { roots.push((di as u32, r)); }
                    }
                }
            }
            (roots, ex.pushes)
        };
        let ns = |t: f64| t * 1e9 / nodes as f64;
        let nroots: usize = postings.iter().map(|m| m.get(&c.root).map_or(0, Vec::len)).sum();
        println!("\n-- {} ({} roots)", c.name, nroots);
        // separate, path-enumerating (no set mode): today's semantics
        let (t_pa, (ra_p, raw_a, st_a)) = time(reps, || run_sep(&pa, false));
        let (t_pb, (rb_p, raw_b, st_b)) = time(reps, || run_sep(&pb, false));
        // separate, set mode (dedupe + early exit per task)
        let (t_sa, (ra_s, _, st_sa)) = time(reps, || run_sep(&pa, true));
        let (t_sb, (rb_s, _, st_sb)) = time(reps, || run_sep(&pb, true));
        assert_eq!(ra_p, ra_s); assert_eq!(rb_p, rb_s);
        let sa: HashSet<_> = ra_s.iter().copied().collect();
        let sb: HashSet<_> = rb_s.iter().copied().collect();
        // join cost (hash set ops over root sets)
        let (t_join, (and_n, or_n, not_n)) = time(reps, || {
            let and_n = sa.iter().filter(|x| sb.contains(x)).count();
            let or_n = sa.union(&sb).count();
            let not_n = sa.iter().filter(|x| !sb.contains(x)).count();
            (and_n, or_n, not_n)
        });
        println!("   separate A path-enum: {:.2} ns/node ({} witness paths, {} roots, {} pushes); set-mode: {:.2} ns/node ({} pushes)", ns(t_pa), raw_a, ra_p.len(), st_a, ns(t_sa), st_sa);
        println!("   separate B path-enum: {:.2} ns/node ({} witness paths, {} roots, {} pushes); set-mode: {:.2} ns/node ({} pushes)", ns(t_pb), raw_b, rb_p.len(), st_b, ns(t_sb), st_sb);
        println!("   join (3 set ops over root sets): {:.3} ns/node", ns(t_join) / 3.0);
        for (kind, label, expect) in [("and", "AND  (A then B)", and_n), ("and_ba", "AND  (B then A)", and_n), ("or", "OR", or_n), ("not", "A AND NOT B", not_n)] {
            let p = fused(kind);
            let (t_f, (rf, st_f)) = time(reps, || run_fused(&p));
            assert_eq!(rf.len(), expect, "{} {}", c.name, kind);
            let sep_path = t_pa + t_pb + t_join / 3.0;
            let sep_set = t_sa + t_sb + t_join / 3.0;
            println!("   fused {:<16} {:>7.2} ns/node ({:>6} roots, {:>9} pushes) | vs separate path-enum {:>5.2}x, vs separate set-mode {:>5.2}x", label, ns(t_f), rf.len(), st_f, sep_path / t_f, sep_set / t_f);
        }
        let _ = (st_b, st_sb);
    }
}
