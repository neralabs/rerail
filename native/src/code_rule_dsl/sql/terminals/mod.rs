//! Native terminals behind virtual tables, each evaluated once per batch (`pattern`) or once per
//! statement over its whole input set (`pattern`'s env-input variant, `text_match`, `fact`). SQL
//! never calls one per row.

mod fact;
mod leaves;
mod pattern;
mod text;

use super::plan::SlotKind;
use crate::code_rule_dsl::language::GrammarDialect;
use ast_grep_core::matcher::{Pattern, PatternNode};
use ast_grep_core::meta_var::MetaVariable;
pub(super) use fact::{
    BatchScopes, Binder, FACT_COLUMNS, FactTable, FactTerm, Leaf, Verdicts,
};
pub(super) use leaves::{
    BindingSites, BindsCandidates, BindsOthers, Exported, FormatPart, FormatSides, Occurrences,
    Parsed,
};
pub(super) use pattern::{PatternRow, PatternTable, fresh_rows};
use regex::Regex;
pub(super) use text::TextTable;

/// The most value columns a pattern terminal has (`v0`…): far above any pattern's captures, and
/// within SQLite's column limit (2000) with the frames' own columns.
pub(super) const PATTERN_COLUMNS: usize = 1024;

/// One terminal of a program, by its index.
pub(super) enum Terminal {
    Pattern(PatternTerm),
    Regex(Regex),
    /// An input-set terminal's three-valued verdicts (`fact`).
    Fact(FactTerm),
}

/// A `pattern` term: its captures in column order (single ones one column, lists four), the
/// kinds its root can match (`None`: any), and which of its captures an env-input statement
/// passes in (`reads`, indices into `vars`).
pub(super) struct PatternTerm {
    pub(super) pattern: Pattern,
    pub(super) dialect: GrammarDialect,
    pub(super) vars: Vec<(String, SlotKind)>,
    pub(super) kinds: Option<Vec<u16>>,
    pub(super) reads: Vec<usize>,
}

impl PatternTerm {
    pub(super) fn new(pattern: Pattern, dialect: GrammarDialect) -> Self {
        use ast_grep_core::matcher::Matcher;
        let mut vars = Vec::new();
        collect_vars(&pattern.node, &mut vars);
        vars.sort();
        vars.dedup();
        let kinds = pattern
            .potential_kinds()
            .map(|kinds| kinds.iter().map(|kind| kind as u16).collect());
        Self {
            pattern,
            dialect,
            vars,
            kinds,
            reads: Vec::new(),
        }
    }

    /// The number of value columns.
    pub(super) fn width(&self) -> usize {
        self.vars
            .iter()
            .map(|(_, kind)| match kind {
                SlotKind::List => 4,
                _ => 1,
            })
            .sum()
    }
}

/// A pattern's captures (single ones as nodes, `$$$` ones as lists), sorted and deduplicated.
pub(super) fn pattern_vars(pattern: &Pattern) -> Vec<(String, SlotKind)> {
    let mut vars = Vec::new();
    collect_vars(&pattern.node, &mut vars);
    vars.sort();
    vars.dedup();
    vars
}

fn collect_vars(node: &PatternNode, vars: &mut Vec<(String, SlotKind)>) {
    match node {
        PatternNode::MetaVar { meta_var } => match meta_var {
            MetaVariable::Capture(name, _) => vars.push((name.clone(), SlotKind::Node)),
            MetaVariable::MultiCapture(name) => vars.push((name.clone(), SlotKind::List)),
            MetaVariable::Dropped(_) | MetaVariable::Multiple => {}
        },
        PatternNode::Terminal { .. } => {}
        PatternNode::Internal { children, .. } => {
            for child in children {
                collect_vars(child, vars);
            }
        }
    }
}

/// How often, on this thread, the terminals' tables were filtered and a pattern's fresh matches
/// computed: what shows a terminal runs once per statement or batch, never per row.
#[cfg(test)]
pub(super) mod calls {
    use std::cell::Cell;

    thread_local! {
        pub(in crate::code_rule_dsl::sql) static PATTERN_FILTERS: Cell<usize> = const { Cell::new(0) };
        pub(in crate::code_rule_dsl::sql) static FRESH_MATCHES: Cell<usize> = const { Cell::new(0) };
        pub(in crate::code_rule_dsl::sql) static TEXT_FILTERS: Cell<usize> = const { Cell::new(0) };
        pub(in crate::code_rule_dsl::sql) static FACT_FILTERS: Cell<usize> = const { Cell::new(0) };
        /// Keys a fact terminal answered unknown: what shows a three-valued case reached one.
        pub(in crate::code_rule_dsl::sql) static UNKNOWN_VERDICTS: Cell<usize> = const { Cell::new(0) };
    }

    pub(in crate::code_rule_dsl::sql) fn note(
        counter: &'static std::thread::LocalKey<Cell<usize>>,
    ) {
        counter.set(counter.get() + 1);
    }
}
