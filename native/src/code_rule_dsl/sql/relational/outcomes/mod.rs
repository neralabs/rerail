//! Explicit outcomes for total, effect-free three-valued operators.
//! SQL owns composition. Bindings are typed columns; unknown is never SQL NULL.
mod emit;
mod local;
mod lower;
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
    Bind(usize),
    All(Vec<Plan>),
    Any(Vec<Plan>),
    Not(Box<Plan>),
    Search(Axis, bool, Box<Plan>),
    Count {
        axis: Axis,
        end: bool,
        rule: Box<Plan>,
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
