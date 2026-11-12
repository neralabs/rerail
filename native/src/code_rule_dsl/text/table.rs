//! A test's plans as one dense automaton table, for the device's text tests (`metal_join`'s `text_verdicts`).
//!
//! The device answers a text test once per distinct term, a SIMD group per term, by composing each byte's
//! transition function over a chunk of 32 bytes at a time (decision 0038's chunked matching). So it needs the plans
//! as one deterministic automaton with few states (each a byte, at most 255), whose final state alone says whether
//! any plan matched anywhere: a match state is made absorbing (every byte, and the end of the text, keep it), and
//! the end-of-text transition is applied last.
//!
//! The table is made from the same lowered plans the CPU's `Compiled` runs (the text layer's one canonical plan per
//! test), so both answer the same question. A plan the table cannot hold (a backtracking pattern, a Unicode word
//! boundary, more than 255 states) is refused, with why: the test then runs on the CPU.

use super::{Lowered, Plan};
use regex_automata::dfa::{Automaton, StartKind, dense};
use regex_automata::nfa::thompson;
use regex_automata::util::start;
use regex_automata::{Anchored, MatchKind};
use regex_syntax::hir::Hir;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// The most states a table holds (a state is a byte in the device's scratch).
pub(in crate::code_rule_dsl) const MAX_STATES: usize = 255;

/// A dense automaton: `bytes` is the byte classes (256), then `next` (`states` rows of `classes`), then the
/// end-of-text transition per state, then whether each state accepts (all bytes).
#[derive(Debug)]
pub(in crate::code_rule_dsl) struct Table {
    pub(in crate::code_rule_dsl) states: u32,
    pub(in crate::code_rule_dsl) classes: u32,
    pub(in crate::code_rule_dsl) start: u32,
    pub(in crate::code_rule_dsl) bytes: Vec<u8>,
}

impl Table {
    /// Whether `text` matches: the CPU's walk of the same table (tests only; the device walks it in chunks).
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn is_match(&self, text: &[u8]) -> bool {
        let (s, c) = (self.states as usize, self.classes as usize);
        let next = &self.bytes[256..256 + s * c];
        let eoi = &self.bytes[256 + s * c..256 + s * c + s];
        let accept = &self.bytes[256 + s * c + s..];
        let mut state = self.start as usize;
        for &b in text {
            state = next[state * c + self.bytes[b as usize] as usize] as usize;
        }
        accept[eoi[state] as usize] != 0
    }
}

/// A plan as a regex of the whole test (anchors included); `None` for a backtracking one.
fn hir(plan: &Plan) -> Option<Hir> {
    Some(match plan {
        Plan::Always => Hir::empty(),
        Plan::Never => Hir::fail(),
        Plan::Literals { anchor, literals } => {
            let core = Hir::alternation(literals.iter().map(|l| Hir::literal(l.clone())).collect());
            Plan::automaton_hir(*anchor, &core)
        }
        Plan::Automaton { anchor, core } => Plan::automaton_hir(*anchor, core),
        Plan::Backtracking { .. } => return None,
    })
}

/// The table of `plans` (any of them matching), made once per set of plans.
pub(in crate::code_rule_dsl) fn table(plans: &[Arc<Lowered>]) -> Result<Arc<Table>, String> {
    static TABLES: LazyLock<Mutex<HashMap<String, Result<Arc<Table>, String>>>> = LazyLock::new(Default::default);
    let key = plans.iter().map(|p| p.key.0.as_str()).collect::<Vec<_>>().join(",");
    if let Some(t) = TABLES.lock().expect("tables").get(&key) {
        return t.clone();
    }
    let made = build(plans).map(Arc::new);
    TABLES.lock().expect("tables").insert(key, made.clone());
    made
}

fn build(plans: &[Arc<Lowered>]) -> Result<Table, String> {
    let hirs = plans.iter().map(|p| hir(&p.plan).ok_or_else(|| "a backtracking pattern".to_owned())).collect::<Result<Vec<_>, _>>()?;
    let hirs = if hirs.is_empty() { vec![Hir::fail()] } else { hirs };
    let nfa = thompson::Compiler::new()
        .configure(thompson::Config::new().utf8(false))
        .build_many_from_hir(&hirs)
        .map_err(|e| format!("its automaton: {e}"))?;
    let dfa = dense::Builder::new()
        .configure(dense::Config::new().match_kind(MatchKind::All).start_kind(StartKind::Unanchored))
        .build_from_nfa(&nfa)
        .map_err(|e| format!("its automaton: {e}"))?;
    let first = dfa.start_state(&start::Config::new().anchored(Anchored::No)).map_err(|e| format!("its start: {e}"))?;
    // Every state reachable from the start, numbered in the order found.
    let mut ids = vec![first];
    let mut index: HashMap<regex_automata::util::primitives::StateID, usize> = HashMap::from([(first, 0)]);
    let mut next: Vec<[usize; 256]> = Vec::new();
    let mut eoi: Vec<usize> = Vec::new();
    let mut at = 0;
    while at < ids.len() {
        let id = ids[at];
        if dfa.is_quit_state(id) {
            return Err("its automaton quits on some bytes".into());
        }
        let mut row = [0usize; 256];
        let mut see = |to| {
            let n = index.len();
            *index.entry(to).or_insert_with(|| {
                ids.push(to);
                n
            })
        };
        for b in 0..=255u8 {
            row[b as usize] = see(dfa.next_state(id, b));
        }
        let e = see(dfa.next_eoi_state(id));
        next.push(row);
        eoi.push(e);
        if ids.len() > MAX_STATES {
            return Err(format!("its automaton has more than {MAX_STATES} states"));
        }
        at += 1;
    }
    let n = ids.len();
    let accept: Vec<bool> = ids.iter().map(|&id| dfa.is_match_state(id)).collect();
    // A match seen stays seen.
    for s in 0..n {
        if accept[s] {
            next[s] = [s; 256];
            eoi[s] = s;
        }
    }
    // Bytes with the same column of transitions are one class.
    let mut class_of: HashMap<Vec<usize>, u8> = HashMap::new();
    let mut classes = [0u8; 256];
    let mut columns: Vec<Vec<usize>> = Vec::new();
    for b in 0..256 {
        let column: Vec<usize> = (0..n).map(|s| next[s][b]).collect();
        let k = class_of.len();
        let c = *class_of.entry(column.clone()).or_insert_with(|| {
            columns.push(column);
            k as u8
        });
        classes[b] = c;
    }
    let nc = columns.len();
    let mut bytes = classes.to_vec();
    for s in 0..n {
        bytes.extend(columns.iter().map(|col| col[s] as u8));
    }
    bytes.extend(eoi.iter().map(|&e| e as u8));
    bytes.extend(accept.iter().map(|&a| u8::from(a)));
    Ok(Table { states: n as u32, classes: nc as u32, start: 0, bytes })
}

/// A regex's automata for `replace_all` on the device (`metal_join`'s `text_replace`, decision 0038's chunked
/// matching), as `regex_automata`'s DFA regex finds matches: a forward unanchored leftmost-first automaton whose last
/// match before it dies is a match's end, and a reverse anchored automaton (every match) run back from that end whose
/// furthest match is its start. Their states are bytes, at most `MAX_STATES`. `bytes`: a header of sixteen words
/// (forward states, classes, start at the text's start, start elsewhere, dead; reverse states, classes, start at the
/// text's end, start elsewhere, dead; the two tables' offsets), then each table as `Table`'s: byte classes, the
/// transitions, the end-of-text transition per state, whether each state is a match (a byte late: a match state after
/// byte k is a match ending, or in reverse starting, at k).
#[derive(Debug)]
pub(in crate::code_rule_dsl) struct Replacer {
    pub(in crate::code_rule_dsl) bytes: Vec<u8>,
    /// The larger table's states: a SIMD group's scratch holds two rows of them per lane.
    pub(in crate::code_rule_dsl) states: u32,
}

/// The replacement automata of regex `pattern`, made once per pattern. Refused (the replacement runs on the CPU): a
/// pattern that may match the empty text (Rust's rules for empty matches are not the device's), a look other than the
/// text's start and end, an automaton past `MAX_STATES`.
pub(in crate::code_rule_dsl) fn replacer(pattern: &str) -> Result<Arc<Replacer>, String> {
    static MADE: LazyLock<Mutex<HashMap<String, Result<Arc<Replacer>, String>>>> = LazyLock::new(Default::default);
    if let Some(r) = MADE.lock().expect("replacers").get(pattern) {
        return r.clone();
    }
    let made = build_replacer(pattern).map(Arc::new);
    MADE.lock().expect("replacers").insert(pattern.to_owned(), made.clone());
    made
}

/// One direction's automaton as a dense table: its starts (at the text's edge, elsewhere), dead state and bytes.
fn dense_table(dfa: &dense::DFA<Vec<u32>>, anchored: Anchored) -> Result<(u32, u32, u32, u32, u32, Vec<u8>), String> {
    let at = |behind: Option<u8>| dfa.start_state(&start::Config::new().anchored(anchored).look_behind(behind)).map_err(|e| format!("its start: {e}"));
    let edge = at(None)?;
    let elsewhere = at(Some(b'a'))?;
    for b in 0..=255u8 {
        if at(Some(b))? != elsewhere {
            return Err("a regex whose start depends on the byte beside it".into());
        }
    }
    let mut ids = vec![edge];
    let mut index: HashMap<regex_automata::util::primitives::StateID, usize> = HashMap::from([(edge, 0)]);
    let mut see = |to, ids: &mut Vec<_>| {
        let n = index.len();
        *index.entry(to).or_insert_with(|| {
            ids.push(to);
            n
        })
    };
    let mid = see(elsewhere, &mut ids);
    let (mut next, mut eoi): (Vec<[usize; 256]>, Vec<usize>) = (Vec::new(), Vec::new());
    let mut at = 0;
    while at < ids.len() {
        let id = ids[at];
        if dfa.is_quit_state(id) {
            return Err("its automaton quits on some bytes".into());
        }
        let mut row = [0usize; 256];
        for b in 0..=255u8 {
            row[b as usize] = see(dfa.next_state(id, b), &mut ids);
        }
        eoi.push(see(dfa.next_eoi_state(id), &mut ids));
        next.push(row);
        if ids.len() > MAX_STATES {
            return Err(format!("its automaton has more than {MAX_STATES} states"));
        }
        at += 1;
    }
    let n = ids.len();
    // A state for the dead one even when none is reached (it is never entered then).
    let dead = ids.iter().position(|&id| dfa.is_dead_state(id)).unwrap_or(n.min(MAX_STATES - 1));
    let mut class_of: HashMap<Vec<usize>, u8> = HashMap::new();
    let mut classes = [0u8; 256];
    let mut columns: Vec<Vec<usize>> = Vec::new();
    for b in 0..256 {
        let column: Vec<usize> = (0..n).map(|s| next[s][b]).collect();
        let k = class_of.len();
        classes[b] = *class_of.entry(column.clone()).or_insert_with(|| {
            columns.push(column);
            k as u8
        });
    }
    let mut bytes = classes.to_vec();
    for s in 0..n {
        bytes.extend(columns.iter().map(|col| col[s] as u8));
    }
    bytes.extend(eoi.iter().map(|&e| e as u8));
    bytes.extend(ids.iter().map(|&id| u8::from(dfa.is_match_state(id))));
    Ok((n as u32, columns.len() as u32, 0, mid as u32, dead as u32, bytes))
}

fn build_replacer(pattern: &str) -> Result<Replacer, String> {
    use regex_syntax::hir::{Look, LookSet};
    let hir = regex_syntax::ParserBuilder::new().build().parse(pattern).map_err(|e| format!("its regex: {e}"))?;
    let props = hir.properties();
    if props.minimum_len().is_none_or(|m| m == 0) {
        return Err("a regex that may match the empty text".into());
    }
    let ends = LookSet::empty().insert(Look::Start).insert(Look::End);
    if !props.look_set().subtract(ends).is_empty() {
        return Err("a regex with a look other than the text's start and end".into());
    }
    let forward = thompson::Compiler::new().build_from_hir(&hir).map_err(|e| format!("its automaton: {e}"))?;
    let fdfa = dense::Builder::new()
        .configure(dense::Config::new().match_kind(MatchKind::LeftmostFirst).start_kind(StartKind::Unanchored))
        .build_from_nfa(&forward)
        .map_err(|e| format!("its automaton: {e}"))?;
    let reverse = thompson::Compiler::new()
        .configure(thompson::Config::new().reverse(true).which_captures(thompson::WhichCaptures::None))
        .build_from_hir(&hir)
        .map_err(|e| format!("its reverse automaton: {e}"))?;
    let rdfa = dense::Builder::new()
        .configure(dense::Config::new().match_kind(MatchKind::All).start_kind(StartKind::Anchored))
        .build_from_nfa(&reverse)
        .map_err(|e| format!("its reverse automaton: {e}"))?;
    let (fs, fc, f0, f1, fd, ft) = dense_table(&fdfa, Anchored::No)?;
    let (rs, rc, r0, r1, rd, rt) = dense_table(&rdfa, Anchored::Yes)?;
    let foff = 64;
    let roff = foff + ft.len();
    let mut bytes = Vec::with_capacity(roff + rt.len());
    for w in [fs, fc, f0, f1, fd, rs, rc, r0, r1, rd, foff as u32, roff as u32, 0, 0, 0, 0] {
        bytes.extend(w.to_le_bytes());
    }
    bytes.extend(ft);
    bytes.extend(rt);
    Ok(Replacer { bytes, states: fs.max(rs) })
}

impl Replacer {
    /// `text` with every match replaced by `with` (a literal), walking the automata as the device does, a byte at a
    /// time (tests only; the device composes 32 bytes' transitions at once).
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn replace(&self, text: &str, with: &str) -> String {
        let w = |i: usize| u32::from_le_bytes(self.bytes[4 * i..4 * i + 4].try_into().unwrap()) as usize;
        struct Dir<'a> { t: &'a [u8], s: usize, c: usize, edge: usize, mid: usize, dead: usize }
        impl Dir<'_> {
            fn next(&self, q: usize, b: u8) -> usize { self.t[256 + q * self.c + self.t[b as usize] as usize] as usize }
            fn eoi(&self, q: usize) -> usize { self.t[256 + self.s * self.c + q] as usize }
            fn accept(&self, q: usize) -> bool { self.t[256 + self.s * self.c + self.s + q] != 0 }
        }
        let f = Dir { t: &self.bytes[w(10)..w(11)], s: w(0), c: w(1), edge: w(2), mid: w(3), dead: w(4) };
        let r = Dir { t: &self.bytes[w(11)..], s: w(5), c: w(6), edge: w(7), mid: w(8), dead: w(9) };
        let t = text.as_bytes();
        let n = t.len();
        let (mut out, mut pos) = (Vec::new(), 0);
        while pos < n {
            let (mut st, mut last, mut dead) = (if pos == 0 { f.edge } else { f.mid }, None, false);
            for (k, &b) in t.iter().enumerate().skip(pos) {
                st = f.next(st, b);
                if st == f.dead {
                    dead = true;
                    break;
                }
                if f.accept(st) {
                    last = Some(k);
                }
            }
            if !dead && f.accept(f.eoi(st)) {
                last = Some(n);
            }
            let Some(e) = last else { break };
            let lo = pos.saturating_sub(1);
            let (mut st, mut first, mut dead) = (if e == n { r.edge } else { r.mid }, None, false);
            for k in (lo..e).rev() {
                st = r.next(st, t[k]);
                if st == r.dead {
                    dead = true;
                    break;
                }
                if r.accept(st) {
                    first = Some(k + 1);
                }
            }
            if !dead && pos == 0 && r.accept(r.eoi(st)) {
                first = Some(0);
            }
            let Some(s) = first.filter(|&s| s >= pos) else { break };
            out.extend_from_slice(&t[pos..s]);
            out.extend_from_slice(with.as_bytes());
            pos = e;
        }
        out.extend_from_slice(&t[pos..]);
        String::from_utf8(out).unwrap()
    }
}

#[cfg(test)]
mod tests {
    /// The replacement automaton replaces as Rust's `regex` does, over the packs' patterns; the empty-matching and
    /// word-boundary ones are refused.
    #[test]
    fn a_replacers_automaton_replaces_as_regex_does() {
        let texts = ["", "  a b  ", "x\tsrc/lib.ts  ", "node:fs", "__init__", "#x\n#y", "tURt", "a%2Cb%2cc", "@ts-ignore here", "αβ  γ", "n", "nn", "a.b.c", "x*y", "`a${b}`", "q&display=x&y", "aabbc abbbc", "abcd abc", "xαβy é xαy", "\u{2003}a\u{2003}"];
        let patterns = ["^\\s+|\\s+$", "\\s+", "#[^\\n]*", "_", "n$", "^node:", "^_+", "[rtU]", "%2[Cc]", "\\*", "\\.", "/", "@ts-ignore", "\\s", "^`|\\$\\{[\\s\\S]*$|`$", "&display[=&][\\s\\S]*", "a+b|ab+c", "(ab|a)(c|bcd)", "x[αβ]+y|é"];
        for p in patterns {
            let r = super::replacer(p).unwrap_or_else(|e| panic!("{p}: {e}"));
            let re = regex::Regex::new(p).unwrap();
            for t in texts {
                assert_eq!(r.replace(t, "<>"), re.replace_all(t, "<>"), "replace_all({t:?}, {p:?})");
            }
        }
        for p in ["a*", "\\bx", "(?m)^x", "x?"] {
            assert!(super::replacer(p).is_err(), "{p} refused");
        }
    }


    use crate::code_rule_dsl::logic::strings::Matcher;

    /// The table answers as the CPU's matcher does, for every kind of test.
    #[test]
    fn a_tests_table_answers_as_its_matcher() {
        let texts = ["", "foo", "foobar", "barfoo", "xfoox", "Foo", "a.b.c", "src/lib/x.ts", "αβγ", "fo", "test_case", "TODO: x"];
        let cases: [(&str, &[&str]); 12] = [
            ("prefix", &["foo"]),
            ("suffix", &["foo", ".ts"]),
            ("contains", &["oo", "β"]),
            ("equals", &["foo", "fo"]),
            ("iprefix", &["FOO"]),
            ("glob", &["src/**/*.ts"]),
            ("rx", &["^[a-z]+_[a-z]+$"]),
            ("rx", &["(?i)todo"]),
            ("rx", &["o{2}"]),
            ("rx", &["(?s).*"]),
            ("rx", &["a\\.b"]),
            ("word", &["case"]),
        ];
        for (op, patterns) in cases {
            let m = Matcher::new(op, &patterns.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>()).unwrap();
            let t = match super::table(&m.plans) {
                Ok(t) => t,
                // A Unicode word boundary has no table: the test runs on the CPU.
                Err(e) => {
                    assert_eq!(op, "word", "{op} {patterns:?}: {e}");
                    continue;
                }
            };
            for text in texts {
                assert_eq!(t.is_match(text.as_bytes()), m.is_match(text), "@{op}({text:?}, {patterns:?})");
            }
        }
    }
}
