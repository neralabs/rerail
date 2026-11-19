//! The passes' per-element calls on the words (decision 0430): an arm's test (`F_TEST`) and an
//! invocation's expansion (`F_EXPAND`), each one run of the preprocessing words (0348) over a
//! file made for it. A run of one element is what 0408's lockstep batch runs a thread of.
//!
//! An element's file is gathered tokens, every token pointing into the unit's own text (a
//! token's buffer is its own, so nothing is copied but the tokens):
//! - the `#define` lines of the definitions the element reaches at its position: from the
//!   names of its tokens, the definition each names there (a search of the positional table),
//!   and the names of that one's body, to a fixed point; a built-in the position has undefined,
//!   an `#undef` line;
//! - then the element: an invocation's tokens, or an arm's line as `#if …` / `1` / `#else` /
//!   `0` / `#endif`, whose output is its value. After an invocation, the `(` after it where one
//!   follows in its stretch, then a `)` of no file: an expansion that takes the `(` (it asks
//!   for the call after it) or the `)` (a call left open) is a merge.
//!
//! The element's file stands where the element does: a file for each file of the instance's
//! include chain, each including the next (mode (a)'s resolved `includes`), the element's
//! last. Then the constants and the names made here, then the pass's files (their text only),
//! which a `_Pragma`'s lexing may add to while the element is read. So `__FILE__`, `__BASE_FILE__`, `__INCLUDE_LEVEL__` and `__has_include_next` read
//! the chain as the serial pass does, and a token's `__LINE__` its own file's lines.
//!
//! A name made by a paste is one this closure cannot see. The run asks (`Env::asks`): each name
//! its lookups missed comes back, and one defined at the position is given and the element run
//! again. A header name `__has_include` asks for is found by the host the same way (the import
//! fixpoint's split), and a `_Pragma`'s text is lexed as the lean pass lexes it.

use rerail_c_lean::T;
use rerail_c_lean::pp::{DIGRAPH, P};
use rerail_c_pp::Config;
use rerail_c_pp::directives::Kind;
use rerail_c_pp::macros::{BOL, SPACE, Tk};
use rerail_c_pp::stream::parallel::host::Unit;
use rerail_compiler::pp_words::{self, BUILTINS, Bounds, Env, Expanded, File, NONE, Named, Refused, Tok};
use rerail_compiler::program::Words as Program;
use std::collections::HashMap;

/// The words' runner: `crate::expand::native`, or the interpreter (`pp_words::run`).
pub type Run = fn(&Program, &[File], &Env, &Bounds) -> Result<Expanded, Refused>;

/// The readings an element may take: the first, and one for each round of names it asked for.
const READINGS: usize = 8;

/// The constant tokens an element's file needs, one buffer.
const CONSTS: &[u8] = b"# define undef if ifdef ifndef else endif include 1 0 ) \"x\"";

/// The words' side of the passes: the program, its runner, the unit's environment, and the
/// spellings of the tokens expansions made (`handle`).
pub struct Words {
    program: &'static Program,
    run: Run,
    /// The waves' runner: every element's reading of a wave in one batch.
    batch: Option<Batch>,
    cpp: bool,
    date: Vec<u8>,
    time: Vec<u8>,
    timestamp: Vec<u8>,
    features: Vec<(Vec<u8>, Vec<u8>, i64)>,
    /// By the pass's file: its line table (`Unit::lines`), made when first read.
    lines: Vec<Option<Vec<u32>>>,
    /// The `_Pragma` texts lexed: the pass's file, and its tokens.
    pragmas: HashMap<Vec<u8>, (u32, Vec<Tk>)>,
    /// The spellings of the tokens expansions made: handle `-1 - k` is `made[k]`.
    pub made: Vec<Vec<u8>>,
    pub stats: WordStats,
}

/// What the words did.
#[derive(Clone, Default, Debug)]
pub struct WordStats {
    pub tests: u64,
    pub expansions: u64,
    /// Readings again: a name asked for, a header name, a `_Pragma`.
    pub again: u64,
    /// The definitions given, summed over the readings.
    pub given: u64,
    /// Wall time in the words' runs, and in making the elements' files, in µs.
    pub run_us: u64,
    pub make_us: u64,
    /// Wall time in the calls (`test`, `expand`), in µs.
    pub call_us: u64,
    /// On the device (`Words::with_batch`): the batches, their dispatches, their GPU time in µs.
    pub batches: u64,
    pub dispatches: u64,
    pub gpu_us: u64,
}

/// An element as the words read it: its tokens copied out of the unit's tables (which a
/// reading only adds to: a `_Pragma`'s file), so the unit is free to change.
enum Owned {
    /// An arm's test: its directive's kind and line.
    Test(Kind, Vec<Tk>),
    /// An invocation's tokens, and the `(` after them in its stretch (held by value: an
    /// `Option<&Tk>` read through a reference faulted under Relax's tiered code, O2).
    Expand(Vec<Tk>, Option<Tk>),
}

/// An element to read: what it is, its instance, its position, its first `__COUNTER__`.
struct Item {
    el: Owned,
    inst: i64,
    pos: i64,
    counter: u32,
}

/// An element being read: its instance's file and include chain, its closure so far (the names
/// to follow, those given, the definitions' lines, the names' bytes), the header names and
/// pragmas it asked for, and its outcome once it has one.
struct Pending {
    item: Item,
    file: u32,
    chain: Vec<u32>,
    start: Vec<u32>,
    given: HashMap<u32, i64>,
    defs: Vec<Tok>,
    extra: Vec<u8>,
    named: Vec<Named>,
    done: Option<Result<Reading, &'static str>>,
}

/// One reading's own data, which its files borrow: the element's file's tokens and chains, the
/// names it asked for (renumbered), the pass's files it uses and their tokens, the chain's
/// paths and includes, and its bounds.
struct JobData {
    toks: Vec<Tok>,
    chains: Vec<(u32, u32)>,
    named_now: Vec<Named>,
    file_toks: Vec<Vec<Tok>>,
    used: Vec<u32>,
    paths: Vec<Vec<u8>>,
    incs: Vec<[(u32, u32); 1]>,
    include: [Tok; 3],
    follower: Option<usize>,
    sentinel: Option<Tok>,
    bounds: Bounds,
    counter: u32,
}

/// A constant token (`CONSTS`'s word `w`) in the constants' file `buf`.
fn konst(w: &[u8], flags: u8, buf: u32) -> Tok {
    let at = find_word(CONSTS, w);
    Tok { kind: kind_of_const(w), flags, buf, start: at as u32, end: (at + w.len()) as u32, exp: 0 }
}

/// A reading's files: one for each file of the include chain, each including the next, the
/// element's last; the constants; the names made here; then the pass's files it uses.
fn files_of<'a>(jd: &'a JobData, p: &'a Pending, unit: &'a Unit, lines: &'a [Option<Vec<u32>>]) -> Vec<File<'a>> {
    let d = p.chain.len();
    let mut files: Vec<File> = Vec::with_capacity(d + 2 + jd.used.len());
    for k in 0..d - 1 {
        files.push(File { path: &jd.paths[k], toks: &jd.include, includes: &jd.incs[k], ..File::default() });
    }
    files.push(File { path: jd.paths.last().unwrap(), bytes: &[], lines: &[], toks: &jd.toks, chains: &jd.chains, includes: &[], named: &jd.named_now });
    files.push(File { bytes: CONSTS, ..File::default() });
    files.push(File { bytes: &p.extra, ..File::default() });
    for (j, &f) in jd.used.iter().enumerate() {
        let (path, text) = unit.file(f as usize);
        files.push(File { path: path.as_bytes(), bytes: text, lines: lines[f as usize].as_deref().unwrap_or(&[]), toks: &jd.file_toks[j], ..File::default() });
    }
    files
}

/// A batch's runner of readings: each job's result, in order, and the batch's GPU seconds and
/// dispatches (`crate::expand::on_metal_counted`, the device's lockstep batch).
pub type Batch = fn(&[crate::expand::Job<'_>]) -> (Vec<Result<Expanded, Refused>>, f64, usize);

impl Words {
    pub fn new(cfg: &Config, run: Run) -> Result<Words, &'static str> {
        let features = cfg.features.iter().map(|((b, n), &v)| (b.as_bytes().to_vec(), n.as_bytes().to_vec(), v)).collect();
        Ok(Words {
            program: crate::expand::program()?,
            run,
            batch: None,
            cpp: cfg.opts.cpp,
            date: cfg.date.as_bytes().to_vec(),
            time: cfg.time.as_bytes().to_vec(),
            timestamp: cfg.timestamp.as_bytes().to_vec(),
            features,
            lines: Vec::new(),
            pragmas: HashMap::new(),
            made: Vec::new(),
            stats: WordStats::default(),
        })
    }

    /// The words run a wave of readings in batches by `batch` (the device's lockstep batch).
    pub fn with_batch(mut self, batch: Batch) -> Words {
        self.batch = Some(batch);
        self
    }

    /// A token's spelling (`-1 - k`: an expansion's).
    pub fn made(&self, handle: i64) -> &[u8] {
        &self.made[(-handle - 1) as usize]
    }

    /// Arm `dir`'s test in instance `inst` at `pos`, as `Unit::test` answers it: its value, a
    /// refusal's code plus one, and to `looked` each name it may have looked up, with the
    /// table's value there (the memo's pairs).
    pub fn test(&mut self, unit: &mut Unit, inst: i64, dir: i64, pos: i64, keys: &[i64], vals: &[i64], looked: &mut Vec<(i64, i64)>) -> (bool, i64) {
        let t = std::time::Instant::now();
        let r = self.test_(unit, inst, dir, pos, keys, vals, looked);
        self.stats.call_us += t.elapsed().as_micros() as u64;
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn test_(&mut self, unit: &mut Unit, inst: i64, dir: i64, pos: i64, keys: &[i64], vals: &[i64], looked: &mut Vec<(i64, i64)>) -> (bool, i64) {
        let (kind, line) = unit.directive(inst, dir);
        if kind == Kind::Else {
            return (true, 0);
        }
        self.stats.tests += 1;
        let item = Item { el: Owned::Test(kind, line.to_vec()), inst, pos, counter: 0 };
        let (r, l) = read_across(&mut [Share { words: self, unit, keys, vals }], vec![vec![item]]).pop().unwrap().pop().unwrap();
        looked.extend(l);
        test_value(unit, r)
    }

    /// The invocation of text tokens `first..=last` of instance `inst` (its stretch's last
    /// `stop`) at `pos`, `__COUNTER__` from `counter`, as `Unit::expand` answers it: its tokens'
    /// handles to `out`, and its count of `__COUNTER__`s.
    #[allow(clippy::too_many_arguments)]
    pub fn expand(&mut self, unit: &mut Unit, inst: i64, first: i64, last: i64, stop: i64, pos: i64, counter: i64, keys: &[i64], vals: &[i64], out: &mut Vec<i64>) -> Result<i64, i64> {
        let t = std::time::Instant::now();
        let r = self.expand_(unit, inst, first, last, stop, pos, counter, keys, vals, out);
        self.stats.call_us += t.elapsed().as_micros() as u64;
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_(&mut self, unit: &mut Unit, inst: i64, first: i64, last: i64, stop: i64, pos: i64, counter: i64, keys: &[i64], vals: &[i64], out: &mut Vec<i64>) -> Result<i64, i64> {
        self.stats.expansions += 1;
        let (toks, after) = unit.text_toks(inst, first, last, stop);
        let (toks, after) = (toks.to_vec(), after.filter(|t| t.is(T::LParen)).copied());
        let item = Item { el: Owned::Expand(toks, after), inst, pos, counter: counter as u32 };
        let (r, _) = read_across(&mut [Share { words: self, unit, keys, vals }], vec![vec![item]]).pop().unwrap().pop().unwrap();
        self.expanded(unit, r, counter, out)
    }

    /// An expansion's reading as `expand` answers it: its tokens made, its count.
    fn expanded(&mut self, unit: &mut Unit, r: Result<Reading, &'static str>, counter: i64, out: &mut Vec<i64>) -> Result<i64, i64> {
        let r = r.map_err(|why| unit.reason_code(why) + 1)?;
        for s in r.out {
            self.made.push(s);
            out.push(-(self.made.len() as i64));
        }
        Ok(i64::from(r.counter) - counter)
    }

    /// An element to be read, its closure started from its own tokens' names.
    fn pending(unit: &Unit, item: Item) -> Pending {
        let (file, chain) = unit.instance(item.inst);
        let chain = chain.to_vec();
        debug_assert_eq!(chain.last(), Some(&file));
        let start: Vec<u32> = match &item.el {
            Owned::Test(_, line) => line.iter().filter(|t| t.kind == P::Ident).map(|t| t.sym).collect(),
            Owned::Expand(toks, _) => toks.iter().filter(|t| t.kind == P::Ident).map(|t| t.sym).collect(),
        };
        Pending { item, file, chain, start, given: HashMap::new(), defs: Vec::new(), extra: Vec::new(), named: Vec::new(), done: None }
    }

    /// An element's next reading: its closure grown, its file's tokens, its files' numbering.
    fn job(&mut self, unit: &mut Unit, keys: &[i64], vals: &[i64], p: &mut Pending) -> Result<JobData, &'static str> {
        // The chain's files come first, then the constants and the names made here: the pass's
        // file `k` is the element's files' `base + k`.
        let d = p.chain.len() as u32;
        let base = d + 2;
        let (consts_buf, extra_buf) = (d + 1, d + 2);
        let nf = unit.nfiles();
        self.close(unit, keys, vals, p.item.pos, &mut p.start, &mut p.given, &mut p.defs, &mut p.extra, base, consts_buf, extra_buf);
        self.stats.given += p.given.values().filter(|&&v| v > 0).count() as u64;
        // The element's file: the definitions, then the element.
        let c = |w: &[u8], flags: u8| konst(w, flags, consts_buf);
        let mut toks = p.defs.clone();
        let mut chains = Vec::new();
        let mut follower = None;
        let mut sentinel = None;
        match &p.item.el {
            Owned::Test(kind, line) => {
                let kw: &[u8] = match kind {
                    Kind::If | Kind::Elif => b"if",
                    Kind::Ifdef | Kind::Elifdef => b"ifdef",
                    Kind::Ifndef | Kind::Elifndef => b"ifndef",
                    _ => return Err("a test of no conditional"),
                };
                let i_if = toks.len() as u32;
                toks.push(c(b"#", pp_words::BOL));
                toks.push(c(kw, 0));
                for (k, t) in line.iter().enumerate() {
                    let mut x = word_tok(t, base);
                    x.flags &= !pp_words::BOL;
                    if k == 0 {
                        x.flags |= pp_words::SPACE;
                    }
                    toks.push(x);
                }
                toks.push(c(b"1", pp_words::BOL));
                let i_else = toks.len() as u32;
                toks.push(c(b"#", pp_words::BOL));
                toks.push(c(b"else", 0));
                toks.push(c(b"0", pp_words::BOL));
                let i_endif = toks.len() as u32;
                toks.push(c(b"#", pp_words::BOL));
                toks.push(c(b"endif", 0));
                chains = vec![(i_if, i_else), (i_else, i_endif)];
            }
            Owned::Expand(list, after) => {
                for (k, t) in list.iter().enumerate() {
                    let mut x = word_tok(t, base);
                    if k == 0 {
                        // Its first token ends the last definition's line.
                        x.flags |= pp_words::BOL;
                    }
                    toks.push(x);
                }
                if let Some(a) = *after {
                    follower = Some(toks.len());
                    toks.push(word_tok(&a, base));
                }
                let s = c(b")", 0);
                sentinel = Some(s);
                toks.push(s);
            }
        }
        // The files: the chain's and the element's, the constants, the names made here, then
        // the pass's files the element's tokens point into and the `_Pragma`s' it names (their
        // text; nothing reads a pass's file but a pragma's). Tokens and names are numbered in
        // the pass's files (`base + k`) until here, then renumbered to these.
        if self.lines.len() < nf {
            self.lines.resize(nf, None);
        }
        let mut used: Vec<u32> = Vec::new();
        let mut slot: HashMap<u32, u32> = HashMap::new();
        let mut place = |k: u32, used: &mut Vec<u32>| -> u32 {
            *slot.entry(k).or_insert_with(|| {
                used.push(k);
                base + used.len() as u32 - 1
            })
        };
        let mut named_now = p.named.clone();
        for n in &mut named_now {
            if n.to != NONE && n.to >= base {
                n.to = place(n.to - base, &mut used);
            }
        }
        for t in &mut toks {
            if t.buf > base {
                t.buf = place(t.buf - base - 1, &mut used) + 1;
            }
        }
        let mut file_toks: Vec<Vec<Tok>> = Vec::new();
        let mut k = 0;
        while k < used.len() {
            // A pragma's file: its tokens (its own text), renumbered.
            let f = used[k];
            let mut ts: Vec<Tok> = self.pragmas.values().find(|q| q.0 == f).map_or(Vec::new(), |q| q.1.iter().map(|t| word_tok(t, base)).collect());
            for t in &mut ts {
                t.buf = place(t.buf - base - 1, &mut used) + 1;
            }
            file_toks.push(ts);
            k += 1;
        }
        for &f in &used {
            if self.lines[f as usize].is_none() {
                self.lines[f as usize] = Some(unit.lines(f as usize));
            }
        }
        let paths: Vec<Vec<u8>> = p.chain.iter().map(|&f| unit.file(f as usize).0.as_bytes().to_vec()).collect();
        let incs: Vec<[(u32, u32); 1]> = (1..d).map(|k| [(0, k)]).collect();
        let bytes: usize = toks.iter().map(|t| (t.end - t.start) as usize).sum();
        let n = toks.len();
        let bounds = Bounds {
            arena: 64 * n + 2 * bytes + 4096,
            scratch: 64 * bytes + 4096,
            depth: 256,
            macros: n / 2 + 64,
            output: 64 * n + 2 * bytes + 4096,
            steps: 4096 * (n as u64 + 64) + 64 * bytes as u64,
            calls: 64 * n + 2 * bytes + 4096,
            lists: 64 * n + 2 * bytes + 4096,
            list_bytes: 64 * bytes + 4096,
        };
        let include = [c(b"#", pp_words::BOL), c(b"include", 0), c(b"\"x\"", pp_words::SPACE)];
        Ok(JobData { toks, chains, named_now, file_toks, used, paths, incs, include, follower, sentinel, bounds, counter: p.item.counter })
    }

    /// A reading's result: what it asked for given (`None`: read it again), else its output.
    #[allow(clippy::too_many_arguments)]
    fn after(&mut self, unit: &mut Unit, keys: &[i64], vals: &[i64], p: &mut Pending, jd: &JobData, e: Expanded, spell: Vec<Vec<u8>>, looked: &mut Vec<(i64, i64)>) -> Option<Result<Reading, &'static str>> {
        let d = p.chain.len() as u32;
        let base = d + 2;
        let consts_buf = d + 1;
        let path = jd.paths.last().unwrap().clone();
        // What it asked for: names, header names, `_Pragma`s.
        let mut more = false;
        for m in &e.missed {
            let s = unit.sym(m);
            if let std::collections::hash_map::Entry::Vacant(v) = p.given.entry(s) {
                let x = at(keys, vals, s, p.item.pos);
                v.insert(x);
                if x > 0 {
                    p.given.remove(&s);
                    p.start.push(s);
                    more = true;
                }
            }
        }
        let (wanted, out_toks, next_counter) = (e.wanted, e.toks, e.counter);
        for (from, mut w) in wanted {
            if from != d - 1 {
                return Some(Err("an element's reading asks from another file"));
            }
            if w.embed {
                return Some(Err("an element's reading embeds a file"));
            }
            if p.named.iter().any(|n| (&n.name, n.angle, n.next, n.embed, n.pragma) == (&w.name, w.angle, w.next, w.embed, w.pragma)) {
                continue;
            }
            w.to = if w.pragma {
                if !self.pragmas.contains_key(&w.name) {
                    let (pf, lexed) = unit.lex(format!("<pragma {}>", String::from_utf8_lossy(&path)), w.name.clone());
                    self.pragmas.insert(w.name.clone(), (pf, lexed));
                }
                self.pragmas[&w.name].0 + base
            } else if unit.found(p.file, &w.name, w.angle, w.next) {
                // Found: any file of text answers `__has_include` (the constants').
                consts_buf - 1
            } else {
                NONE
            };
            p.named.push(w);
            more = true;
        }
        if more {
            return None;
        }
        // By name: the memo compares a test's pairs in order, and a map's order is its seed's.
        let at = looked.len();
        looked.extend(p.given.iter().map(|(&s, &v)| (i64::from(s), v)));
        looked[at..].sort_unstable();
        let mut out = spell;
        if let Some(s) = jd.sentinel {
            // The `)` after it, read as it was: no call left open.
            match out_toks.last() {
                Some(t) if (t.buf, t.start, t.end) == (s.buf, s.start, s.end) => {
                    out.pop();
                }
                _ => return Some(Err("a merge")),
            }
        }
        let out_toks = &out_toks[..out.len()];
        if let Some(f) = jd.follower.map(|i| jd.toks[i]) {
            // The `(` after it, read as it was: no merge. Else the expansion took it.
            match out_toks.last() {
                Some(t) if (t.buf, t.start, t.end) == (f.buf, f.start, f.end) => {
                    out.pop();
                }
                _ => return Some(Err("a merge")),
            }
        }
        Some(Ok(Reading { out, counter: next_counter }))
    }

    /// The closure of the names in `start` at `pos`: each one's value there into `given`, and
    /// for each definition, its `#define` line into `defs` and its body's names in turn; a
    /// built-in undefined there, an `#undef` line (its name's bytes into `extra`).
    #[allow(clippy::too_many_arguments)]
    fn close(&self, unit: &Unit, keys: &[i64], vals: &[i64], pos: i64, start: &mut Vec<u32>, given: &mut HashMap<u32, i64>, defs: &mut Vec<Tok>, extra: &mut Vec<u8>, base: u32, consts_buf: u32, extra_buf: u32) {
        let c = |w: &[u8], flags: u8| -> Tok {
            let at = find_word(CONSTS, w);
            Tok { kind: kind_of_const(w), flags, buf: consts_buf, start: at as u32, end: (at + w.len()) as u32, exp: 0 }
        };
        while let Some(s) = start.pop() {
            if given.contains_key(&s) {
                continue;
            }
            let v = at(keys, vals, s, pos);
            given.insert(s, v);
            if v > 0 {
                let Some((line, names)) = unit.definition((v - 1) as u32) else { continue };
                defs.push(c(b"#", pp_words::BOL));
                defs.push(c(b"define", 0));
                for t in line {
                    let mut x = word_tok(t, base);
                    x.flags &= !pp_words::BOL;
                    defs.push(x);
                }
                start.extend(names);
            } else {
                let name = unit.sym_text(s);
                if BUILTINS.iter().any(|b| b.0.as_bytes() == name) {
                    let lo = extra.len() as u32;
                    extra.extend_from_slice(name);
                    defs.push(c(b"#", pp_words::BOL));
                    defs.push(c(b"undef", 0));
                    defs.push(Tok { kind: pp_words::IDENT, flags: pp_words::SPACE, buf: extra_buf, start: lo, end: extra.len() as u32, exp: 0 });
                }
            }
        }
    }
}

/// An element's reading: its tokens' spellings, and `__COUNTER__`'s next value.
struct Reading {
    out: Vec<Vec<u8>>,
    counter: u32,
}

/// A test's reading as its value: `1` or `0`, else refused.
fn test_value(unit: &mut Unit, r: Result<Reading, &'static str>) -> (bool, i64) {
    match r {
        Ok(e) => match &e.out[..] {
            [v] if v == b"1" || v == b"0" => (v == b"1", 0),
            _ => (false, unit.reason_code("a test's reading is no value") + 1),
        },
        Err(why) => (false, unit.reason_code(why) + 1),
    }
}

/// The table's value for `sym` at `pos`: the last row of `sym` at or before it (a definition
/// plus one; 0, none).
fn at(keys: &[i64], vals: &[i64], sym: u32, pos: i64) -> i64 {
    let q = (i64::from(sym) << 32) | pos;
    let i = keys.partition_point(|&k| k <= q);
    if i > 0 && keys[i - 1] >> 32 == i64::from(sym) { vals[i - 1] } else { 0 }
}

/// A lean token as the words read it (`expand::lean_tu`'s kinds): the pass's file `buf - 1` is
/// the element's files' `buf - 1 + base`.
fn word_tok(t: &Tk, base: u32) -> Tok {
    debug_assert!(t.buf != 0, "a file's token");
    let kind = match t.kind {
        P::Ident => pp_words::IDENT,
        P::Number => pp_words::NUMBER,
        P::Char | P::Str | P::RawStr => pp_words::LITERAL,
        P::Hash => pp_words::HASH,
        P::HashHash => pp_words::HASHHASH,
        P::Punct if t.flags & DIGRAPH != 0 => pp_words::DIGRAPH,
        P::Punct => pp_words::PUNCT,
        _ => pp_words::OTHER,
    };
    let flags = if t.flags & BOL != 0 { pp_words::BOL } else { 0 } | if t.flags & SPACE != 0 { pp_words::SPACE } else { 0 };
    Tok { kind, flags, buf: t.buf + base, start: t.start, end: t.end, exp: 0 }
}

fn find_word(hay: &[u8], w: &[u8]) -> usize {
    hay.split(|&b| b == b' ').scan(0, |at, x| {
        let here = *at;
        *at += x.len() + 1;
        Some((here, x))
    })
    .find(|(_, x)| *x == w)
    .map(|(a, _)| a)
    .expect("a constant token")
}

fn kind_of_const(w: &[u8]) -> u8 {
    match w {
        b"#" => pp_words::HASH,
        b"1" | b"0" => pp_words::NUMBER,
        b")" => pp_words::PUNCT,
        b"\"x\"" => pp_words::LITERAL,
        _ => pp_words::IDENT,
    }
}

/// A unit's share of a read across units: its words, its tables, and its positional table.
pub struct Share<'s, 'a> {
    pub words: &'s mut Words,
    pub unit: &'s mut Unit<'a>,
    pub keys: &'s [i64],
    pub vals: &'s [i64],
}

/// Arms' tests of many units at once, each unit's `(inst, dir, pos)`s (`Words::test`): each
/// one's value, refusal code and looked pairs. Every unit's readings run a wave at a time in one
/// batch (the first unit's `batch`, the device's lockstep batch).
pub fn tests_across(us: &mut [Share<'_, '_>], arms: &[&[(i64, i64, i64)]]) -> Vec<Vec<(bool, i64, Vec<(i64, i64)>)>> {
    let t = std::time::Instant::now();
    let mut outs: Vec<Vec<Option<(bool, i64, Vec<(i64, i64)>)>>> = Vec::with_capacity(us.len());
    let mut items: Vec<Vec<Item>> = Vec::with_capacity(us.len());
    let mut ats: Vec<Vec<usize>> = Vec::with_capacity(us.len());
    for (s, arms) in us.iter_mut().zip(arms) {
        let mut out = vec![None; arms.len()];
        let (mut its, mut at) = (Vec::new(), Vec::new());
        for (k, &(inst, dir, pos)) in arms.iter().enumerate() {
            let (kind, line) = s.unit.directive(inst, dir);
            if kind == Kind::Else {
                out[k] = Some((true, 0, Vec::new()));
                continue;
            }
            s.words.stats.tests += 1;
            its.push(Item { el: Owned::Test(kind, line.to_vec()), inst, pos, counter: 0 });
            at.push(k);
        }
        outs.push(out);
        items.push(its);
        ats.push(at);
    }
    let rs = read_across(us, items);
    for (((s, out), at), r) in us.iter_mut().zip(&mut outs).zip(ats).zip(rs) {
        for (k, (r, l)) in at.into_iter().zip(r) {
            let (v, code) = test_value(s.unit, r);
            out[k] = Some((v, code, l));
        }
    }
    if let Some(s) = us.first_mut() {
        s.words.stats.call_us += t.elapsed().as_micros() as u64;
    }
    outs.into_iter().map(|o| o.into_iter().map(Option::unwrap).collect()).collect()
}

/// Invocations of many units at once, each unit's `(inst, first, last, stop, pos, counter)`s
/// (`Words::expand`): each one's count, or its refusal code, and its tokens' handles, made in
/// their order. The readings run as `tests_across`'s.
pub fn expands_across(us: &mut [Share<'_, '_>], calls: &[&[[i64; 6]]]) -> Vec<Vec<(Result<i64, i64>, Vec<i64>)>> {
    let t = std::time::Instant::now();
    let items: Vec<Vec<Item>> = us
        .iter_mut()
        .zip(calls)
        .map(|(s, calls)| {
            calls
                .iter()
                .map(|&[inst, first, last, stop, pos, counter]| {
                    s.words.stats.expansions += 1;
                    let (toks, after) = s.unit.text_toks(inst, first, last, stop);
                    let (toks, after) = (toks.to_vec(), after.filter(|t| t.is(T::LParen)).copied());
                    Item { el: Owned::Expand(toks, after), inst, pos, counter: counter as u32 }
                })
                .collect()
        })
        .collect();
    let rs = read_across(us, items);
    let out = us
        .iter_mut()
        .zip(calls)
        .zip(rs)
        .map(|((s, calls), r)| {
            r.into_iter()
                .zip(calls.iter())
                .map(|((r, _), c)| {
                    let mut toks = Vec::new();
                    let n = s.words.expanded(s.unit, r, c[5], &mut toks);
                    (n, toks)
                })
                .collect()
        })
        .collect();
    if let Some(s) = us.first_mut() {
        s.words.stats.call_us += t.elapsed().as_micros() as u64;
    }
    out
}

/// Elements of many units read together: each one's file made, each wave of readings (every
/// unit's) run in one batch (the first unit's `batch`, the device's lockstep batch, else one
/// run each), and each element made again while it asks for more; each one's reading and its
/// looked pairs. A unit's asks are answered in its elements' order, as one by one. A batch's
/// counts and times go to the first unit's stats.
fn read_across(us: &mut [Share<'_, '_>], items: Vec<Vec<Item>>) -> Vec<Vec<(Result<Reading, &'static str>, Vec<(i64, i64)>)>> {
    let mut ps: Vec<Vec<Pending>> = us.iter().zip(items).map(|(s, its)| its.into_iter().map(|it| Words::pending(s.unit, it)).collect()).collect();
    let mut looked: Vec<Vec<Vec<(i64, i64)>>> = ps.iter().map(|p| vec![Vec::new(); p.len()]).collect();
    for reading in 0..READINGS {
        let t0 = std::time::Instant::now();
        // Each unit's wave: its elements still reading, each one's next job.
        let mut jds: Vec<Vec<(usize, JobData)>> = Vec::with_capacity(us.len());
        for (s, p) in us.iter_mut().zip(&mut ps) {
            let mut mine = Vec::new();
            for (k, q) in p.iter_mut().enumerate().filter(|q| q.1.done.is_none()) {
                if reading > 0 {
                    s.words.stats.again += 1;
                }
                match s.words.job(s.unit, s.keys, s.vals, q) {
                    Ok(jd) => mine.push((k, jd)),
                    Err(why) => q.done = Some(Err(why)),
                }
            }
            jds.push(mine);
        }
        if jds.iter().all(Vec::is_empty) {
            break;
        }
        // The wave's files, environments and bounds, every unit's, then its runs.
        let view: &[Share] = us;
        let features: Vec<Vec<(&[u8], &[u8], i64)>> = view.iter().map(|s| s.words.features.iter().map(|(b, n, v)| (&b[..], &n[..], *v)).collect()).collect();
        let files: Vec<Vec<Vec<File>>> = view.iter().zip(&jds).zip(&ps).map(|((s, j), p)| j.iter().map(|(k, jd)| files_of(jd, &p[*k], s.unit, &s.words.lines)).collect()).collect();
        let envs: Vec<Vec<Env>> = view
            .iter()
            .zip(&jds)
            .zip(&features)
            .map(|((s, j), f)| {
                let w = &*s.words;
                j.iter().map(|(_, jd)| Env { cpp: w.cpp, date: &w.date, time: &w.time, timestamp: &w.timestamp, features: f, counter: jd.counter, asks: true }).collect()
            })
            .collect();
        let jobs: Vec<crate::expand::Job> = view
            .iter()
            .zip(&jds)
            .zip(&files)
            .zip(&envs)
            .flat_map(|(((s, j), f), e)| j.iter().zip(f).zip(e).map(|(((_, jd), f), e)| (s.words.program, &f[..], e, &jd.bounds)))
            .collect();
        let (run, batch) = (view[0].words.run, view[0].words.batch);
        let t1 = std::time::Instant::now();
        let mut device = None;
        let results: Vec<Result<Expanded, Refused>> = match batch {
            Some(b) => {
                let (r, gpu, dispatches) = b(&jobs);
                device = Some((gpu, dispatches));
                r
            }
            None => jobs.iter().map(|&(w, f, e, b)| run(w, f, e, b)).collect(),
        };
        let t2 = std::time::Instant::now();
        let mut results = results.into_iter();
        let spelled: Vec<Vec<Result<(Expanded, Vec<Vec<u8>>), &'static str>>> = files
            .iter()
            .map(|fs| fs.iter().map(|f| results.next().expect("a result a job").map(|e| { let sp = e.toks.iter().map(|t| e.text(f, t).to_vec()).collect(); (e, sp) }).map_err(|r| r.why)).collect())
            .collect();
        drop(jobs);
        drop(envs);
        drop(files);
        drop(features);
        {
            let st = &mut us[0].words.stats;
            st.make_us += (t1 - t0).as_micros() as u64;
            st.run_us += (t2 - t1).as_micros() as u64;
            if let Some((gpu, dispatches)) = device {
                st.batches += 1;
                st.dispatches += dispatches as u64;
                st.gpu_us += (gpu * 1e6) as u64;
            }
        }
        for (((s, p), j), (sp, lk)) in us.iter_mut().zip(&mut ps).zip(jds).zip(spelled.into_iter().zip(&mut looked)) {
            for ((k, jd), r) in j.into_iter().zip(sp) {
                let q = &mut p[k];
                q.done = match r {
                    Err(why) => Some(Err(why)),
                    Ok((e, spell)) => s.words.after(s.unit, s.keys, s.vals, q, &jd, e, spell, &mut lk[k]),
                };
            }
        }
    }
    ps.into_iter().zip(looked).map(|(p, l)| p.into_iter().zip(l).map(|(q, l)| (q.done.unwrap_or(Err("past an element's readings")), l)).collect()).collect()
}
