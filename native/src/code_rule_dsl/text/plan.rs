//! Plans: the families of matcher the engines run, each a canonical form with a key. Equal plans
//! have equal keys, so a test spelt two ways is one job.
//!
//! A new family is a `Plan` variant with its case in `Plan::canonical`, `Compiled::of`,
//! `vocabulary::evaluate` and `scan::how`.

use regex_syntax::hir::Hir;
use sha2::{Digest as _, Sha256};

/// Where a literal or an automaton must match in the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Anchor {
    /// The whole text.
    Whole,
    /// A prefix of it.
    Start,
    /// A suffix of it.
    End,
    /// Anywhere in it.
    Within,
}

impl Anchor {
    pub(in crate::code_rule_dsl) fn of(start: bool, end: bool) -> Self {
        match (start, end) {
            (true, true) => Self::Whole,
            (true, false) => Self::Start,
            (false, true) => Self::End,
            (false, false) => Self::Within,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Every text.
    Always,
    /// No text.
    Never,
    /// One of a finite set of strings, at `anchor` (sorted, distinct, none empty unless `Whole`).
    Literals { anchor: Anchor, literals: Vec<Vec<u8>> },
    /// A regular language at `anchor`: `core` holds no whole-text anchor (they are `anchor`).
    Automaton { anchor: Anchor, core: Hir },
    /// A pattern only a backtracking engine runs (backreferences), in `value.regex` syntax.
    Backtracking { pattern: String },
}

/// A plan's identity: its canonical form's digest. Equal keys run as one job, and results are kept
/// by key (`vocabulary`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(pub String);

impl Plan {
    /// The plan as text: what the key digests. Literals are hex, so the text is unambiguous.
    pub(in crate::code_rule_dsl) fn canonical(&self) -> String {
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        match self {
            Self::Always => "always".to_owned(),
            Self::Never => "never".to_owned(),
            Self::Literals { anchor, literals } => format!("literals|{anchor:?}|{}", literals.iter().map(|l| hex(l)).collect::<Vec<_>>().join(",")),
            Self::Automaton { anchor, core } => format!("automaton|{anchor:?}|{core}"),
            Self::Backtracking { pattern } => format!("backtracking|{pattern}"),
        }
    }

    pub(in crate::code_rule_dsl) fn key(&self) -> Key {
        let digest = Sha256::digest(self.canonical().as_bytes());
        Key(format!("text1:{}", digest[..12].iter().map(|b| format!("{b:02x}")).collect::<String>()))
    }

    /// The regex of an automaton plan, anchors included.
    pub(in crate::code_rule_dsl) fn automaton_hir(anchor: Anchor, core: &Hir) -> Hir {
        use regex_syntax::hir::Look;
        let mut parts = Vec::new();
        if matches!(anchor, Anchor::Whole | Anchor::Start) {
            parts.push(Hir::look(Look::Start));
        }
        parts.push(core.clone());
        if matches!(anchor, Anchor::Whole | Anchor::End) {
            parts.push(Hir::look(Look::End));
        }
        Hir::concat(parts)
    }
}

/// A plan as a test of one string at a time: a vocabulary term, a node's text, a Datalog value.
/// `None` from `is_match` where a backtracking pattern could not decide (its limits).
pub enum Compiled {
    Always,
    Never,
    Whole(std::collections::HashSet<Vec<u8>>),
    /// Literals at the start (anchored search), the end (anchored over the reversed text) or within.
    Literals { anchor: Anchor, searcher: aho_corasick::AhoCorasick },
    Automaton(regex_automata::meta::Regex),
    Backtracking(fancy_regex::Regex),
    /// The plan could not be compiled (an automaton too large): it matches nothing it is asked
    /// about, and says so (`is_match` gives `None`).
    Failed,
}

impl Compiled {
    pub(in crate::code_rule_dsl) fn of(plan: &Plan) -> Self {
        match plan {
            Plan::Always => Self::Always,
            Plan::Never => Self::Never,
            Plan::Literals { anchor: Anchor::Whole, literals } => Self::Whole(literals.iter().cloned().collect()),
            Plan::Literals { anchor, literals } => {
                let patterns = if *anchor == Anchor::End { literals.iter().map(|l| l.iter().rev().copied().collect()).collect() } else { literals.clone() };
                match literal_searcher(&patterns) {
                    Some(searcher) => Self::Literals { anchor: *anchor, searcher },
                    None => Self::Failed,
                }
            }
            Plan::Automaton { anchor, core } => {
                match regex_automata::meta::Builder::new().build_from_hir(&Plan::automaton_hir(*anchor, core)) {
                    Ok(regex) => Self::Automaton(regex),
                    Err(_) => Self::Failed,
                }
            }
            Plan::Backtracking { pattern } => match fancy_regex::RegexBuilder::new(pattern).backtrack_limit(crate::code_rule_dsl::value::text_regex::BACKTRACK_LIMIT).build() {
                Ok(regex) => Self::Backtracking(regex),
                Err(_) => Self::Failed,
            },
        }
    }

    pub fn is_match(&self, text: &[u8]) -> Option<bool> {
        use aho_corasick::{Anchored, Input};
        Some(match self {
            Self::Always => true,
            Self::Never => false,
            Self::Whole(set) => set.contains(text),
            Self::Literals { anchor: Anchor::Start, searcher } => searcher.find(Input::new(text).anchored(Anchored::Yes)).is_some(),
            Self::Literals { anchor: Anchor::End, searcher } => {
                let reversed = text.iter().rev().copied().collect::<Vec<_>>();
                searcher.find(Input::new(&reversed).anchored(Anchored::Yes)).is_some()
            }
            Self::Literals { searcher, .. } => searcher.is_match(text),
            Self::Automaton(regex) => regex.is_match(text),
            Self::Backtracking(regex) => {
                let text = std::str::from_utf8(text).ok()?;
                if text.len() > crate::code_rule_dsl::value::MAX_STRING_BYTES {
                    return None;
                }
                regex.is_match(text).ok()?
            }
            Self::Failed => return None,
        })
    }
}

/// A multi-literal searcher that reports every literal, overlapping, and supports anchored search.
pub(in crate::code_rule_dsl) fn literal_searcher(literals: &[Vec<u8>]) -> Option<aho_corasick::AhoCorasick> {
    aho_corasick::AhoCorasick::builder()
        .match_kind(aho_corasick::MatchKind::Standard)
        .start_kind(aho_corasick::StartKind::Both)
        .build(literals)
        .ok()
}
