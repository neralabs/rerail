//! Explicit outcomes with sequencing barriers for reached atomic value leaves.
//! SQL owns composition. Bindings are typed columns; unknown is never SQL NULL.
mod atomic_predicate_tests;
mod binds_tests;
mod bound_tests;
mod budget_tests;
mod capture_predicate_tests;
mod capture_walk_tests;
mod catalog_tests;
mod chain_tests;
mod composition_budget_tests;
mod constraint_tests;
mod constraint_walk_tests;
mod effects;
mod emit;
mod field_end_tests;
mod field_tests;
mod list_tests;
mod lower;
mod memo_tests;
mod nested_memo_tests;
mod nested_tests;
mod pattern_tests;
mod predicate_run_tests;
mod pruning_tests;
mod publication_tests;
mod reached_tests;
mod repeat_any_tests;
mod repeat_utility_tests;
mod repeat_walk_tests;
mod shared_memo_tests;
mod tests;
mod traversal_memo_tests;
mod utility_plan_tests;

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
    Charge(usize, std::rc::Rc<Plan>),
    /// Functional recursive utility; its summary already includes utility entry.
    Chain(usize, std::rc::Rc<super::recursive_chain::Chain>),
    Bind(usize),
    /// Atomic structural pattern, with outputs in terminal capture order.
    Pattern(usize, Vec<usize>, Option<Vec<u16>>),
    BoundLeaf {
        term: usize,
        inputs: Vec<usize>,
        outputs: Vec<usize>,
        kinds: Option<Vec<u16>>,
    },
    Constrained(Box<Plan>, Vec<(usize, Plan)>),
    FieldNeighbor {
        inside: bool,
        field: u16,
        child: Box<Plan>,
    },
    FieldEnd {
        inside: bool,
        field: u16,
        child: Box<Plan>,
    },
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
    effects: effects::Effects,
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
    kind: CaptureKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptureKind {
    Node,
    List,
    Count,
}

impl CaptureKind {
    fn name(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::List => "list",
            Self::Count => "count",
        }
    }

    fn payload(self, column: &str, file: &str) -> String {
        match self {
            Self::Node => column.into(),
            Self::Count => format!("CAST({column} AS TEXT)"),
            Self::List => format!("json(relation_list_json({column},{file}))"),
        }
    }
}

pub(super) fn compile(grammar: &GrammarMatcher) -> Result<Compiled, String> {
    if grammar.transform.is_some() {
        return Err("outcome relations: transformed captures need a typed output contract".into());
    }
    let mut lower = lower::Lower::new(grammar.language, grammar.three_valued);
    let plan = lower.plan(&grammar.matcher)?;
    let mut profile = lower.profile(0);
    let (sql, mut relations) = if plan.fallible() {
        let (sql, mut relations) = emit::compile_jobs(
            &[(&plan, &lower.slots, grammar.applies, &root_filter(grammar))],
            i64::MAX,
        )?;
        relations["terminalOwners"] =
            serde_json::json!(vec![0; lower.pure.program.terminals.len()]);
        (sql, relations)
    } else {
        emit::compile(&plan, &lower.slots, grammar.applies)?
    };
    record_scope_contract(&mut relations);
    profile["compositionOperators"] = relations["compositionOperators"][0].clone();
    relations["lowering"] = serde_json::json!([profile]);
    Ok(Compiled {
        sql,
        relations,
        terminals: lower.pure.program.terminals,
    })
}

impl Plan {
    fn uses_memo(&self) -> bool {
        self.effects.uses_memo
    }
    fn reads_value(&self) -> bool {
        self.effects.reads_value
    }
    fn fallible(&self) -> bool {
        self.effects.fallible
    }
}

fn root_filter(grammar: &GrammarMatcher) -> String {
    grammar.anchors.root_kinds().map_or("1".into(), |kinds| {
        format!(
            "n.kind IN ({})",
            kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    })
}

/// Bounded workspace publication kernel. Terminal IDs share one batch namespace.
pub(super) fn compile_job(grammars: &[&GrammarMatcher], work: i64) -> Result<Compiled, String> {
    let mut terminals = vec![];
    let mut plans = vec![];
    let mut terminal_owners = vec![];
    let mut lowering = vec![];
    for (clause, grammar) in grammars.iter().enumerate() {
        if grammar.transform.is_some() {
            return Err("publication: transforms need their publication contract".into());
        }
        let mut lower = lower::Lower::new(grammar.language, grammar.three_valued);
        lower.scope_owner = clause + 1;
        let terminal_base = terminals.len();
        lower.pure.program.terminals = terminals;
        let plan = lower.plan(&grammar.matcher)?;
        let profile = lower.profile(terminal_base);
        lowering.push(profile);
        terminals = lower.pure.program.terminals;
        let roots = root_filter(grammar);
        terminal_owners.resize(terminals.len(), clause);
        plans.push((plan, lower.slots, grammar.applies, roots));
    }
    let borrowed: Vec<_> = plans
        .iter()
        .map(|(plan, slots, applies, roots)| (plan, slots.as_slice(), *applies, roots.as_str()))
        .collect();
    let (sql, mut relations) = emit::compile_jobs(&borrowed, work)?;
    relations["terminalOwners"] = serde_json::json!(terminal_owners);
    record_scope_contract(&mut relations);
    for (index, profile) in lowering.iter_mut().enumerate() {
        profile["compositionOperators"] = relations["compositionOperators"][index].clone();
    }
    relations["lowering"] = serde_json::json!(lowering);
    Ok(Compiled {
        sql,
        relations,
        terminals,
    })
}

fn record_scope_contract(profile: &mut serde_json::Value) {
    profile["scopeOwnership"] = serde_json::json!({
        "identity": "immutable clause index plus one within this compiled plan",
        "lifetime": "one BatchScopes job; each job resets parked scope state",
        "limitations": "Separately compiled groups sharing one scope lifetime need job-global clause tokens before production cutover."
    });
}
