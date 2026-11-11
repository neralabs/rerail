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

#[cfg(test)]
mod tests {
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
