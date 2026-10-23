//! Explicit outcomes with sequencing barriers for reached atomic value leaves.
//! SQL owns composition. Bindings are typed columns; unknown is never SQL NULL.
mod budget_tests;
mod capture_walk_tests;
mod emit;
mod lower;
mod memo_tests;
mod nested_tests;
mod pruning_tests;
mod publication_tests;
mod reached_tests;
mod tests;

use super::{Compiled, ir::Set};
use crate::code_rule_dsl::matcher::GrammarMatcher;
use std::collections::BTreeSet;

type Slots = BTreeSet<usize>;

#[derive(Clone, Copy, Debug)]
enum Axis {
    Parent,
    Child,
    Previous,
    Next,
}

enum Op {
    Predicate(Set),
    Unknown(usize),
    Value(usize),
    Charge(usize, Box<Plan>),
    Bind(usize),
    All(Vec<Plan>),
    Any(Vec<Plan>),
    Not(Box<Plan>),
    Search(Axis, bool, Box<Plan>, Option<Box<Plan>>),
    Count {
        axis: Axis,
        end: bool,
        rule: Box<Plan>,
        stop: Option<Box<Plan>>,
        min: i64,
        max: Option<i64>,
        binding: Option<(usize, i64)>,
    },
}

struct Plan {
    op: Op,
    mentions: Slots,
    writes: Slots,
    // Unlike counts, these bindings participate in conjunction withholding.
    node_writes: Slots,
    // Native two-valued search admission, applied before this candidate's body or charges.
    walk_filter: Option<WalkFilter>,
}

struct WalkFilter {
    kinds: Vec<u16>,
    // Required-text CNF lowered to one atomic regex per disjunction. Descendant/end only.
    text: Vec<usize>,
}

#[derive(Clone)]
struct Slot {
    name: String,
    count: bool,
}

pub(super) fn compile(grammar: &GrammarMatcher) -> Result<Compiled, String> {
    if grammar.transform.is_some() {
        return Err("outcome relations: transformed captures need a typed output contract".into());
    }
    let mut lower = lower::Lower::new(grammar.language, grammar.three_valued);
    let plan = lower.plan(&grammar.matcher)?;
    emit::memo::admit(&plan)?;
    if plan.fallible() {
        return compile_job(&[grammar], i64::MAX);
    }
    let (sql, relations) = emit::compile(&plan, &lower.slots, grammar.applies)?;
    Ok(Compiled {
        sql,
        relations,
        terminals: lower.pure.program.terminals,
    })
}

impl Plan {
    fn uses_memo(&self) -> bool {
        match &self.op {
            Op::Charge(_, child) => child.reads_value(),
            Op::All(items) | Op::Any(items) => items.iter().any(Self::uses_memo),
            Op::Not(child) => child.uses_memo(),
            Op::Search(_, _, child, stop)
            | Op::Count {
                rule: child, stop, ..
            } => child.uses_memo() || stop.as_ref().is_some_and(|s| s.uses_memo()),
            _ => false,
        }
    }

    fn reads_value(&self) -> bool {
        match &self.op {
            Op::Value(_) => true,
            Op::Not(child) | Op::Charge(_, child) => child.reads_value(),
            Op::All(items) | Op::Any(items) => items.iter().any(Self::reads_value),
            Op::Search(_, _, child, stop)
            | Op::Count {
                rule: child, stop, ..
            } => child.reads_value() || stop.as_ref().is_some_and(|p| p.reads_value()),
            _ => false,
        }
    }

    fn fallible(&self) -> bool {
        match &self.op {
            Op::Value(_) | Op::Charge(..) | Op::Count { .. } => true,
            Op::All(items) | Op::Any(items) => items.iter().any(Self::fallible),
            Op::Not(child) => child.fallible(),
            Op::Search(_, _, child, stop) => {
                child.fallible() || stop.as_ref().is_some_and(|s| s.fallible())
            }
            _ => false,
        }
    }
}

/// Bounded workspace publication kernel. Terminal IDs share one batch namespace.
pub(super) fn compile_job(grammars: &[&GrammarMatcher], work: i64) -> Result<Compiled, String> {
    let mut terminals = vec![];
    let mut plans = vec![];
    let mut terminal_owners = vec![];
    for (clause, grammar) in grammars.iter().enumerate() {
        if grammar.transform.is_some() {
            return Err("publication: transforms need their publication contract".into());
        }
        let mut lower = lower::Lower::new(grammar.language, grammar.three_valued);
        lower.pure.program.terminals = terminals;
        let plan = lower.plan(&grammar.matcher)?;
        emit::memo::admit(&plan)?;
        terminals = lower.pure.program.terminals;
        let roots = grammar.anchors.root_kinds().map_or("1".into(), |kinds| {
            format!(
                "n.kind IN ({})",
                kinds
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        });
        terminal_owners.resize(terminals.len(), clause);
        plans.push((plan, lower.slots, grammar.applies, roots));
    }
    let borrowed: Vec<_> = plans
        .iter()
        .map(|(plan, slots, applies, roots)| (plan, slots.as_slice(), *applies, roots.as_str()))
        .collect();
    let (sql, mut relations) = emit::compile_jobs(&borrowed, work)?;
    relations["terminalOwners"] = serde_json::json!(terminal_owners);
    Ok(Compiled {
        sql,
        relations,
        terminals,
    })
}
