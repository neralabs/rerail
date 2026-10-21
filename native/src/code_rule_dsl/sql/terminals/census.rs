//! `RERAIL_SQL_TERMINAL_CENSUS=<file>`: how much of the terminals' native work repeats. For each
//! terminal kind, the evaluations performed (a node matched against a pattern, a node's text
//! tested against a regex, a program started at a node, a fact key evaluated), the distinct
//! (terminal, input) pairs among them, and the distinct (canonical predicate, input) pairs
//! (`content`), an input being a node of a batch document, with an env-input pattern's or a fact
//! key's capture values. A batch file is in one batch of an apply, so the distinct pairs of the
//! apply are the sum of its batches'. Each batch adds its counts when it ends and the totals so
//! far are written to the file. Unset, nothing is counted.

use super::{Terminal, content};
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

static PATH: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("RERAIL_SQL_TERMINAL_CENSUS").ok());

/// The kinds counted apart, in report order.
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl::sql) enum Kind {
    PatternFresh,
    PatternEnv,
    Regex,
    Program,
    Fact,
}

const KINDS: [&str; 5] = [
    "pattern_fresh",
    "pattern_env",
    "text_match",
    "program_match",
    "fact",
];

#[derive(Default)]
struct Counts {
    performed: u64,
    by_term: FxHashSet<(u32, u64)>,
    by_content: FxHashSet<(u32, u64)>,
}

#[derive(Default, Clone, Copy)]
struct Totals {
    performed: u64,
    by_term: u64,
    by_content: u64,
}

static TOTALS: parking_lot::Mutex<[Totals; 5]> = parking_lot::Mutex::new(
    [Totals {
        performed: 0,
        by_term: 0,
        by_content: 0,
    }; 5],
);

/// A batch's counts, added to the totals when it ends.
pub(in crate::code_rule_dsl::sql) struct BatchCensus {
    /// Per terminal, its canonical predicate's number (a terminal with no canonical form is its
    /// own).
    content: Vec<u32>,
    counts: parking_lot::Mutex<[Counts; 5]>,
}

/// An input's 64-bit hash: a key text's, or a node key itself.
pub(in crate::code_rule_dsl::sql) fn text_key(text: &str) -> u64 {
    let mut hasher = FxHasher::default();
    text.hash(&mut hasher);
    hasher.finish()
}

impl BatchCensus {
    /// A census of a batch of `terminals`, where `RERAIL_SQL_TERMINAL_CENSUS` asks for one.
    pub(in crate::code_rule_dsl::sql) fn of(terminals: &[Terminal]) -> Option<Self> {
        PATH.as_ref()?;
        let mut numbers: FxHashMap<String, u32> = FxHashMap::default();
        let content = terminals
            .iter()
            .enumerate()
            .map(|(index, terminal)| {
                let form = form(terminal).unwrap_or_else(|| format!("#{index}"));
                let next = numbers.len() as u32;
                *numbers.entry(form).or_insert(next)
            })
            .collect();
        Some(Self {
            content,
            counts: parking_lot::Mutex::default(),
        })
    }

    /// One evaluation of terminal `term` at `input`.
    pub(in crate::code_rule_dsl::sql) fn note(&self, kind: Kind, term: usize, input: u64) {
        let mut counts = self.counts.lock();
        let counts = &mut counts[kind as usize];
        counts.performed += 1;
        counts.by_term.insert((term as u32, input));
        counts.by_content.insert((self.content[term], input));
    }
}

/// A terminal's canonical form, by kind (`content`).
fn form(terminal: &Terminal) -> Option<String> {
    match terminal {
        Terminal::Pattern(term) => {
            let pattern = content::pattern(&term.pattern, term.dialect)?;
            Some(format!("pattern:{:?}:{pattern}", term.reads))
        }
        Terminal::Regex(regex) => Some(format!("regex:{}", regex.as_str())),
        Terminal::Program(_) => None,
        Terminal::Fact(term) => {
            let leaf = term.verdicts.content()?;
            Some(format!("fact:{:?}:{leaf}", term.reads))
        }
    }
}

impl Drop for BatchCensus {
    fn drop(&mut self) {
        let Some(path) = PATH.as_ref() else {
            return;
        };
        let counts = self.counts.get_mut();
        let mut totals = TOTALS.lock();
        for (total, counts) in totals.iter_mut().zip(counts.iter()) {
            total.performed += counts.performed;
            total.by_term += counts.by_term.len() as u64;
            total.by_content += counts.by_content.len() as u64;
        }
        let report: serde_json::Map<String, serde_json::Value> = KINDS
            .iter()
            .zip(totals.iter())
            .map(|(name, total)| {
                (
                    (*name).to_owned(),
                    serde_json::json!({
                        "performed": total.performed,
                        "distinct_by_term": total.by_term,
                        "distinct_by_content": total.by_content,
                    }),
                )
            })
            .collect();
        let _ = std::fs::write(path, serde_json::Value::Object(report).to_string());
    }
}
