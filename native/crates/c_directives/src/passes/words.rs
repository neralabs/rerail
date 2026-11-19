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
}

/// An element as the words read it.
enum Element<'a> {
    /// An arm's test: its directive's kind and line.
    Test(Kind, &'a [Tk]),
    /// An invocation's tokens, and the `(` after them in its stretch (held by value: an
    /// `Option<&Tk>` read through a reference faulted under Relax's tiered code, O2).
    Expand(&'a [Tk], Option<Tk>),
}

impl Words {
    pub fn new(cfg: &Config, run: Run) -> Result<Words, &'static str> {
        let features = cfg.features.iter().map(|((b, n), &v)| (b.as_bytes().to_vec(), n.as_bytes().to_vec(), v)).collect();
        Ok(Words {
            program: crate::expand::program()?,
            run,
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
        // SAFETY of the borrow: the directive's tokens live in the unit's tables, which a reading
        // only adds to (a `_Pragma`'s file); copied, so the unit is free to change.
        let line = line.to_vec();
        match self.read(unit, inst, pos, keys, vals, Element::Test(kind, &line), 0, looked) {
            Ok(e) => match &e.out[..] {
                [v] if v == b"1" || v == b"0" => (v == b"1", 0),
                _ => (false, unit.reason_code("a test's reading is no value") + 1),
            },
            Err(why) => (false, unit.reason_code(why) + 1),
        }
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
        let mut looked = Vec::new();
        let r = self.read(unit, inst, pos, keys, vals, Element::Expand(&toks, after), counter as u32, &mut looked).map_err(|why| unit.reason_code(why) + 1)?;
        for s in r.out {
            self.made.push(s);
            out.push(-(self.made.len() as i64));
        }
        Ok(i64::from(r.counter) - counter)
    }

    /// An element read: its file made, run, and made again while it asks for more.
    #[allow(clippy::too_many_arguments)]
    fn read(&mut self, unit: &mut Unit, inst: i64, pos: i64, keys: &[i64], vals: &[i64], el: Element, counter: u32, looked: &mut Vec<(i64, i64)>) -> Result<Reading, &'static str> {
        let (file, chain) = unit.instance(inst);
        let chain = chain.to_vec();
        debug_assert_eq!(chain.last(), Some(&file));
        // The chain's files come first, then the constants and the names made here: the pass's
        // file `k` is the element's files' `base + k`.
        let d = chain.len() as u32;
        let base = d + 2;
        let (consts_buf, extra_buf) = (d + 1, d + 2);
        // The names of the element's own tokens start the closure.
        let mut start: Vec<u32> = match &el {
            Element::Test(_, line) => line.iter().filter(|t| t.kind == P::Ident).map(|t| t.sym).collect(),
            Element::Expand(toks, _) => toks.iter().filter(|t| t.kind == P::Ident).map(|t| t.sym).collect(),
        };
        let mut given: HashMap<u32, i64> = HashMap::new();
        let mut defs: Vec<Tok> = Vec::new();
        let mut extra: Vec<u8> = Vec::new();
        let mut named: Vec<Named> = Vec::new();
        for reading in 0..READINGS {
            if reading > 0 {
                self.stats.again += 1;
            }
            let t0 = std::time::Instant::now();
            let nf = unit.nfiles();
            self.close(unit, keys, vals, pos, &mut start, &mut given, &mut defs, &mut extra, base, consts_buf, extra_buf);
            self.stats.given += given.values().filter(|&&v| v > 0).count() as u64;
            // The element's file: the definitions, then the element.
            let c = |w: &[u8], flags: u8| -> Tok {
                let at = find_word(CONSTS, w);
                Tok { kind: kind_of_const(w), flags, buf: consts_buf, start: at as u32, end: (at + w.len()) as u32, exp: 0 }
            };
            let mut toks = defs.clone();
            let mut chains = Vec::new();
            let mut follower = None;
            let mut sentinel = None;
            match &el {
                Element::Test(kind, line) => {
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
                Element::Expand(list, after) => {
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
            let mut named_now = named.clone();
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
                let mut ts: Vec<Tok> = self.pragmas.values().find(|p| p.0 == f).map_or(Vec::new(), |p| p.1.iter().map(|t| word_tok(t, base)).collect());
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
            let paths: Vec<Vec<u8>> = chain.iter().map(|&f| unit.file(f as usize).0.as_bytes().to_vec()).collect();
            let path = paths.last().unwrap().clone();
            let include = [c(b"#", pp_words::BOL), c(b"include", 0), c(b"\"x\"", pp_words::SPACE)];
            let incs: Vec<[(u32, u32); 1]> = (1..d).map(|k| [(0, k)]).collect();
            let mut files: Vec<File> = Vec::with_capacity(base as usize + used.len());
            for k in 0..d as usize - 1 {
                files.push(File { path: &paths[k], toks: &include, includes: &incs[k], ..File::default() });
            }
            files.push(File { path: &path, bytes: &[], lines: &[], toks: &toks, chains: &chains, includes: &[], named: &named_now });
            files.push(File { bytes: CONSTS, ..File::default() });
            files.push(File { bytes: &extra, ..File::default() });
            for (j, &f) in used.iter().enumerate() {
                let (p, text) = unit.file(f as usize);
                files.push(File { path: p.as_bytes(), bytes: text, lines: self.lines[f as usize].as_deref().unwrap_or(&[]), toks: &file_toks[j], ..File::default() });
            }
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
            let features: Vec<(&[u8], &[u8], i64)> = self.features.iter().map(|(b, n, v)| (&b[..], &n[..], *v)).collect();
            let env = Env { cpp: self.cpp, date: &self.date, time: &self.time, timestamp: &self.timestamp, features: &features, counter, asks: true };
            let t1 = std::time::Instant::now();
            let e = (self.run)(self.program, &files, &env, &bounds).map_err(|r| r.why)?;
            let t2 = std::time::Instant::now();
            self.stats.make_us += (t1 - t0).as_micros() as u64;
            self.stats.run_us += (t2 - t1).as_micros() as u64;
            let spell: Vec<Vec<u8>> = e.toks.iter().map(|t| e.text(&files, t).to_vec()).collect();
            drop(files);
            // What it asked for: names, header names, `_Pragma`s.
            let mut more = false;
            for m in &e.missed {
                let s = unit.sym(m);
                if let std::collections::hash_map::Entry::Vacant(v) = given.entry(s) {
                    let x = at(keys, vals, s, pos);
                    v.insert(x);
                    if x > 0 {
                        given.remove(&s);
                        start.push(s);
                        more = true;
                    }
                }
            }
            let (wanted, out_toks, next_counter) = (e.wanted, e.toks, e.counter);
            for (from, mut w) in wanted {
                if from != d - 1 {
                    return Err("an element's reading asks from another file");
                }
                if w.embed {
                    return Err("an element's reading embeds a file");
                }
                if named.iter().any(|n| (&n.name, n.angle, n.next, n.embed, n.pragma) == (&w.name, w.angle, w.next, w.embed, w.pragma)) {
                    continue;
                }
                w.to = if w.pragma {
                    if !self.pragmas.contains_key(&w.name) {
                        let (pf, lexed) = unit.lex(format!("<pragma {}>", String::from_utf8_lossy(&path)), w.name.clone());
                        self.pragmas.insert(w.name.clone(), (pf, lexed));
                    }
                    self.pragmas[&w.name].0 + base
                } else if unit.found(file, &w.name, w.angle, w.next) {
                    // Found: any file of text answers `__has_include` (the constants').
                    consts_buf - 1
                } else {
                    NONE
                };
                named.push(w);
                more = true;
            }
            if more {
                continue;
            }
            // By name: the memo compares a test's pairs in order, and a map's order is its seed's.
            let at = looked.len();
            looked.extend(given.iter().map(|(&s, &v)| (i64::from(s), v)));
            looked[at..].sort_unstable();
            let mut out = spell;
            if let Some(s) = sentinel {
                // The `)` after it, read as it was: no call left open.
                match out_toks.last() {
                    Some(t) if (t.buf, t.start, t.end) == (s.buf, s.start, s.end) => {
                        out.pop();
                    }
                    _ => return Err("a merge"),
                }
            }
            let out_toks = &out_toks[..out.len()];
            if let Some(f) = follower.map(|i| toks[i]) {
                // The `(` after it, read as it was: no merge. Else the expansion took it.
                match out_toks.last() {
                    Some(t) if (t.buf, t.start, t.end) == (f.buf, f.start, f.end) => {
                        out.pop();
                    }
                    _ => return Err("a merge"),
                }
            }
            return Ok(Reading { out, counter: next_counter });
        }
        Err("past an element's readings")
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
