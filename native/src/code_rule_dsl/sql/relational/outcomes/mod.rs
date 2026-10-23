//! Explicit outcomes with sequencing barriers for reached atomic value leaves.
//! SQL owns composition. Bindings are typed columns; unknown is never SQL NULL.
mod capture_walk_tests;
mod emit;
mod local;
mod lower;
mod nested_tests;
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
    let mut lower = lower::Lower::new(grammar.language);
    let plan = lower.plan(&grammar.matcher)?;
    let (sql, relations) = emit::compile(&plan, &lower.slots, grammar.applies)?;
    Ok(Compiled {
        sql,
        relations,
        terminals: lower.pure.program.terminals,
    })
}

impl Plan {
    fn fallible(&self) -> bool {
        match &self.op {
            Op::Value(_) => true,
            Op::All(items) | Op::Any(items) => items.iter().any(Self::fallible),
            Op::Not(child) => child.fallible(),
            Op::Search(_, _, child, stop)
            | Op::Count {
                rule: child, stop, ..
            } => child.fallible() || stop.as_ref().is_some_and(|s| s.fallible()),
            _ => false,
        }
    }
}
