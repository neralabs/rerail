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

/// The most states a replacement's automaton holds (a state is a 16-bit word).
pub(in crate::code_rule_dsl) const MAX_REPLACE_STATES: usize = 4096;

/// A regex's anchored, leftmost-first automaton, for `replace_all` on the device (`metal_join`'s `TEXTFN_REGEX`):
/// from a start, the end of the match Rust's `regex` prefers there. `bytes`: a header of eight words (states,
/// classes, the start at the text's start, the start elsewhere, the dead state), the byte classes (256), the
/// transitions (`states` rows of `classes` 16-bit states), the end-of-text transition per state, then whether each
/// state is a match (the DFA's matches are a byte late: a match state after byte k is a match ending at k).
#[derive(Debug)]
pub(in crate::code_rule_dsl) struct Replacer {
    pub(in crate::code_rule_dsl) bytes: Vec<u8>,
}

/// The replacement automaton of regex `pattern`, made once per pattern. Refused (the replacement runs on the
/// CPU): a pattern that may match the empty text (Rust's rules for empty matches are not the device's), a look
/// other than the text's start and end, an automaton past `MAX_REPLACE_STATES`.
pub(in crate::code_rule_dsl) fn replacer(pattern: &str) -> Result<Arc<Replacer>, String> {
    static MADE: LazyLock<Mutex<HashMap<String, Result<Arc<Replacer>, String>>>> = LazyLock::new(Default::default);
    if let Some(r) = MADE.lock().expect("replacers").get(pattern) {
        return r.clone();
    }
    let made = build_replacer(pattern).map(Arc::new);
    MADE.lock().expect("replacers").insert(pattern.to_owned(), made.clone());
    made
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
    let nfa = thompson::Compiler::new().build_from_hir(&hir).map_err(|e| format!("its automaton: {e}"))?;
    let dfa = dense::Builder::new()
        .configure(dense::Config::new().match_kind(MatchKind::LeftmostFirst).start_kind(StartKind::Anchored))
        .build_from_nfa(&nfa)
        .map_err(|e| format!("its automaton: {e}"))?;
    let at = |behind: Option<u8>| dfa.start_state(&start::Config::new().anchored(Anchored::Yes).look_behind(behind)).map_err(|e| format!("its start: {e}"));
    let first = at(None)?;
    let elsewhere = at(Some(b'a'))?;
    for b in 0..=255u8 {
        if at(Some(b))? != elsewhere {
            return Err("a regex whose start depends on the byte before".into());
        }
    }
    let mut ids = vec![first];
    let mut index: HashMap<regex_automata::util::primitives::StateID, usize> = HashMap::from([(first, 0)]);
    let mut see = |to, ids: &mut Vec<_>| {
        let n = index.len();
        *index.entry(to).or_insert_with(|| {
            ids.push(to);
            n
        })
    };
    let start1 = see(elsewhere, &mut ids);
    let mut next: Vec<[usize; 256]> = Vec::new();
    let mut eoi: Vec<usize> = Vec::new();
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
        if ids.len() > MAX_REPLACE_STATES {
            return Err(format!("its automaton has more than {MAX_REPLACE_STATES} states"));
        }
        at += 1;
    }
    let n = ids.len();
    let dead = ids.iter().position(|&id| dfa.is_dead_state(id)).unwrap_or(u32::MAX as usize);
    let accept: Vec<u8> = ids.iter().map(|&id| u8::from(dfa.is_match_state(id))).collect();
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
    let nc = columns.len();
    let mut bytes = Vec::new();
    for w in [n as u32, nc as u32, 0, start1 as u32, dead as u32, 0, 0, 0] {
        bytes.extend(w.to_le_bytes());
    }
    bytes.extend(classes);
    for s in 0..n {
        for col in &columns {
            bytes.extend((col[s] as u16).to_le_bytes());
        }
    }
    for &e in &eoi {
        bytes.extend((e as u16).to_le_bytes());
    }
    bytes.extend(accept);
    Ok(Replacer { bytes })
}

impl Replacer {
    /// `text` with every match replaced by `with` (a literal), as the device does it (tests only).
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn replace(&self, text: &str, with: &str) -> String {
        let w = |i: usize| u32::from_le_bytes(self.bytes[4 * i..4 * i + 4].try_into().unwrap()) as usize;
        let (n, nc, dead) = (w(0), w(1), w(4));
        let cls = &self.bytes[32..288];
        let half = |at: usize| u16::from_le_bytes([self.bytes[at], self.bytes[at + 1]]) as usize;
        let next = |s: usize, c: usize| half(288 + 2 * (s * nc + c));
        let eoi = |s: usize| half(288 + 2 * (n * nc + s));
        let accept = |s: usize| self.bytes[288 + 2 * (n * nc + n) + s] != 0;
        let t = text.as_bytes();
        let end = |from: usize| {
            let (mut st, mut last) = (if from == 0 { w(2) } else { w(3) }, None);
            for (k, &b) in t.iter().enumerate().skip(from) {
                st = next(st, cls[b as usize] as usize);
                if st == dead {
                    return last;
                }
                if accept(st) {
                    last = Some(k);
                }
            }
            if accept(eoi(st)) { Some(t.len()) } else { last }
        };
        let (mut out, mut pos, mut s) = (Vec::new(), 0, 0);
        while s < t.len() {
            if t[s] & 0xc0 != 0x80 && let Some(e) = end(s).filter(|&e| e > s) {
                out.extend_from_slice(&t[pos..s]);
                out.extend_from_slice(with.as_bytes());
                pos = e;
                s = e;
                continue;
            }
            s += 1;
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
        let texts = ["", "  a b  ", "x\tsrc/lib.ts  ", "node:fs", "__init__", "#x\n#y", "tURt", "a%2Cb%2cc", "@ts-ignore here", "αβ  γ", "n", "nn", "a.b.c", "x*y"];
        let patterns = ["^\\s+|\\s+$", "\\s+", "#[^\\n]*", "_", "n$", "^node:", "^_+", "[rtU]", "%2[Cc]", "\\*", "\\.", "/", "@ts-ignore", "\\s"];
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
