//! Explicit outcomes with sequencing barriers for reached atomic value leaves.
//! SQL owns composition. Bindings are typed columns; unknown is never SQL NULL.
mod atomic_predicate_tests;
#[cfg(test)]
mod b615_binding_boundary_spike;
#[cfg(test)]
mod b615_captured_relation_spike;
mod binding_facts;
mod binding_occurrence_tests;
mod binding_tests;
#[cfg(test)]
mod cost_relation_spike;
#[cfg(test)]
mod demand_relation_spike;
#[cfg(test)]
mod sim905_effect_spike;
#[cfg(test)]
mod sim905_full_spike;
#[cfg(test)]
mod sim905_pure_cost_spike;
#[cfg(test)]
mod sim905_relation_spike;
pub(super) use binding_facts::register as register_binding_facts;
mod binding_region_tests;
mod binds_tests;
mod bound_tests;
mod budget_tests;
mod call_tests;
mod capture_predicate_tests;
mod capture_walk_tests;
mod catalog_tests;
mod chain_tests;
mod composition_budget_tests;
mod constraint_tests;
mod constraint_walk_tests;
mod downward_tests;
mod effects;
mod emit;
mod field_end_tests;
mod field_tests;
mod finite_all_tests;
mod finite_helper_tests;
pub(super) mod finite_lower;
mod finite_lower_tests;
mod finite_recursive_tests;
mod finite_shared_tests;
mod format_tests;
mod list_tests;
mod lower;
mod memo_tests;
mod mixed_recursive_tests;
mod nested_memo_tests;
mod nested_shape_tests;
mod nested_tests;
mod nth_tests;
mod parse_export_tests;
mod parse_tests;
mod pattern_tests;
mod predicate_memo;
#[cfg(test)]
mod predicate_memo_tests;
mod predicate_run_tests;
mod pruning_tests;
mod publication_tests;
mod pure_recursive_tests;
mod reached_tests;
mod recursive_search_tests;
mod recursive_shape_tests;
mod repeat_any_tests;
mod repeat_utility_tests;
mod repeat_walk_tests;
mod shared_memo_tests;
mod tests;
mod traversal_memo_tests;
mod utility_memo;
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

/// Cost choice for dynamic parsed children. Both guarded forms retain the
/// enclosing utility contract and the same atomic parse/child semantics.
#[derive(Clone)]
enum ParseSetup {
    Plain,
    HostKinds(Vec<u16>),
    Parsed,
}

#[derive(Clone)]
enum Op {
    /// Transparent finite source DAG reference; no native utility boundary.
    Shared(std::rc::Rc<Plan>),
    FiniteProgram(std::rc::Rc<finite_lower::Program>),
    ParsedRecursive {
        program: std::rc::Rc<lower::parse_recursive_lower::Program>,
        entry: usize,
    },
    MixedRecursive {
        program: std::rc::Rc<lower::mixed_recursive_lower::Program>,
        entry: usize,
    },
    FormatArgs {
        term: usize,
        inputs: Vec<usize>,
        unused: Option<Box<Plan>>,
        missing: Option<Box<Plan>>,
    },
    Predicate(Set),
    Unknown(usize),
    NthChild {
        step: i32,
        offset: i32,
        reverse: bool,
        of_rule: Option<Box<Plan>>,
        term: usize,
        memoize: bool,
    },
    Value(usize),
    Charge(usize, std::rc::Rc<Plan>),
    /// Physical cache of a proved total, utility-free predicate walk.
    MemoPredicate(usize, std::rc::Rc<Plan>),
    /// Parsed documents spend utility entry work but native never memoizes them.
    TransientCharge(std::rc::Rc<Plan>),
    /// Functional recursive utility; its summary already includes utility entry.
    Chain(usize, std::rc::Rc<super::recursive_chain::Chain>),
    Downward(usize, std::rc::Rc<super::recursive_downward::Downward>),
    PureRecursive(std::rc::Rc<lower::pure_recursive_lower::Program>),
    RecursiveSearch {
        id: usize,
        component: std::rc::Rc<lower::recursive_search_lower::RecursiveSearch>,
    },
    Bind(usize),
    CaptureBind(usize),
    Host {
        parse: usize,
        child: Box<Plan>,
        mapping: Vec<(usize, usize)>,
    },
    /// Atomic structural pattern, with outputs in terminal capture order.
    Pattern(usize, Vec<usize>, Option<Vec<u16>>),
    BoundLeaf {
        term: usize,
        inputs: Vec<usize>,
        outputs: Vec<usize>,
        kinds: Option<Vec<u16>>,
    },
    Binding {
        core: Box<Plan>,
        children: Vec<(usize, Plan)>,
        occurrence_slot: usize,
        occurrence: Option<Box<Plan>>,
    },
    Parse {
        term: usize,
        setup: ParseSetup,
        child: Box<Plan>,
        exports: Vec<(usize, usize)>,
        hosted: bool,
    },
    CallFrame {
        body: std::rc::Rc<Plan>,
        mapping: Vec<(usize, usize)>,
    },
    Constrained(Box<Plan>, Vec<(usize, Plan)>),
    FieldRule {
        inside: bool,
        field: u16,
        child: Box<Plan>,
        stop: Box<Plan>,
    },
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

#[derive(Clone)]
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

#[derive(Clone)]
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

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

    fn payload(self, column: &str, file: &str, spans: bool) -> String {
        match self {
            Self::Node if spans => format!(
                "CASE WHEN typeof({column})='blob' THEN json(relation_span_json({column},{file})) ELSE {column} END"
            ),
            Self::Node => column.into(),
            Self::Count => format!("CAST({column} AS TEXT)"),
            Self::List => format!("json(relation_list_json({column},{file}))"),
        }
    }
}

fn resource_refusal(error: &str) -> bool {
    error.contains("operator count exceeds") || error.contains("SQL text exceeds")
}
/// Physical selection is automatic and happens before any SQL execution.
/// Existing recursive kernels stay intact unless their repeated bodies exceed
/// the compilation budget. Only then expose their functions to the shared IR.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Composition {
    Inline,
    Functions,
    RecursiveFunctions,
}
impl Composition {
    fn name(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Functions => "shared finite functions",
            Self::RecursiveFunctions => "shared finite and recursive functions",
        }
    }
}
fn record_composition(profile: &mut serde_json::Value, shape: Composition, plan: &Plan) {
    profile["compositionShape"] = serde_json::json!(shape.name());
    if let Op::FiniteProgram(program) = &plan.op {
        profile["finiteControlOperators"] = serde_json::json!(program.code.len());
        profile["finiteFunctions"] = serde_json::json!(program.functions.len());
        profile["finiteRegions"] = serde_json::json!(program.regions.len());
    }
}
fn select_composition(
    mut compile: impl FnMut(Composition) -> Result<Compiled, String>,
) -> Result<Compiled, String> {
    let mut refusals = Vec::new();
    for shape in [
        Composition::Inline,
        Composition::Functions,
        Composition::RecursiveFunctions,
    ] {
        match compile(shape) {
            Ok(compiled) => return Ok(compiled),
            Err(error) if resource_refusal(&error) => {
                refusals.push(format!("{}: {error}", shape.name()))
            }
            Err(error) => return Err(error),
        }
    }
    Err(refusals.join("; "))
}
fn finite_plan(mut plan: Plan, share_recursion: bool) -> Result<Plan, String> {
    let program = finite_lower::lower_with_recursion(&plan, share_recursion)?;
    plan.node_writes = program.node_writes.clone();
    plan.op = Op::FiniteProgram(std::rc::Rc::new(program));
    plan.effects = effects::Effects::of(&plan.op);
    Ok(plan)
}
pub(super) fn compile(grammar: &GrammarMatcher) -> Result<Compiled, String> {
    select_composition(|shape| compile_mode(grammar, shape))
}
pub(super) fn compile_namespaced(
    grammar: &GrammarMatcher,
    prefix: usize,
    clause: usize,
) -> Result<Compiled, String> {
    select_composition(|shape| compile_mode_namespaced(grammar, shape, Some((prefix, clause))))
}
fn compile_mode(grammar: &GrammarMatcher, shape: Composition) -> Result<Compiled, String> {
    compile_mode_namespaced(grammar, shape, None)
}
fn compile_mode_namespaced(
    grammar: &GrammarMatcher,
    shape: Composition,
    namespace: Option<(usize, usize)>,
) -> Result<Compiled, String> {
    let finite = shape != Composition::Inline;
    if grammar.transform.is_some() {
        return Err("outcome relations: transformed captures need a typed output contract".into());
    }
    let mut lower = if finite {
        lower::Lower::new_finite(grammar.language, grammar.three_valued)
    } else {
        lower::Lower::new(grammar.language, grammar.three_valued)
    };
    if let Some((prefix, clause)) = namespace {
        lower.pure.program.terminals = super::seed_terminals(prefix);
        lower.scope_owner = clause + 1;
    }
    lower.root_kinds = grammar.anchors.root_kinds().cloned();
    let plan = predicate_memo::prepare(lower.plan(&grammar.matcher)?);
    let plan = if finite {
        finite_plan(plan, shape == Composition::RecursiveFunctions)?
    } else {
        plan
    };
    let file_guard = grammar
        .file
        .as_ref()
        .map(|condition| super::file_guard::push(&mut lower.pure.program.terminals, condition));
    let mut profile = lower.profile(0);
    record_composition(&mut profile, shape, &plan);
    let memo_layout = lower.memo_layout(plan.effects.needs_peak);
    let (sql, mut relations) = if plan.fallible() || grammar.file.is_some() || namespace.is_some() {
        let (sql, mut relations) = emit::compile_jobs_with_base(
            &[(
                &plan,
                &lower.slots,
                grammar.applies,
                &root_filter(grammar, file_guard),
            )],
            i64::MAX,
            &[memo_layout],
            namespace.map_or(0, |(_, clause)| clause),
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

fn root_filter(grammar: &GrammarMatcher, file_guard: Option<usize>) -> String {
    let kinds = grammar.anchors.root_kinds().map_or("1".into(), |kinds| {
        format!(
            "n.kind IN ({})",
            kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    });
    file_guard.map_or_else(
        || kinds.clone(),
        |term| format!("({kinds}) AND n.file IN (SELECT file FROM relation_file_guard({term}))"),
    )
}

/// Bounded workspace publication kernel. Terminal IDs share one batch namespace.
pub(super) fn compile_job(grammars: &[&GrammarMatcher], work: i64) -> Result<Compiled, String> {
    select_composition(|shape| compile_job_mode(grammars, work, shape))
}
fn compile_job_finite(grammars: &[&GrammarMatcher], work: i64) -> Result<Compiled, String> {
    compile_job_mode(grammars, work, Composition::RecursiveFunctions)
}
fn compile_job_mode(
    grammars: &[&GrammarMatcher],
    work: i64,
    shape: Composition,
) -> Result<Compiled, String> {
    let finite = shape != Composition::Inline;
    let mut terminals = vec![];
    let mut plans = vec![];
    let mut terminal_owners = vec![];
    let mut lowering = vec![];
    let mut memo_layouts = vec![];
    for (clause, grammar) in grammars.iter().enumerate() {
        if grammar.transform.is_some() {
            return Err("publication: transforms need their publication contract".into());
        }
        let mut lower = if finite {
            lower::Lower::new_finite(grammar.language, grammar.three_valued)
        } else {
            lower::Lower::new(grammar.language, grammar.three_valued)
        };
        lower.root_kinds = grammar.anchors.root_kinds().cloned();
        lower.scope_owner = clause + 1;
        let terminal_base = terminals.len();
        lower.pure.program.terminals = terminals;
        let plan = predicate_memo::prepare(lower.plan(&grammar.matcher)?);
        let plan = if finite {
            finite_plan(plan, shape == Composition::RecursiveFunctions)?
        } else {
            plan
        };
        let file_guard = grammar
            .file
            .as_ref()
            .map(|condition| super::file_guard::push(&mut lower.pure.program.terminals, condition));
        let mut profile = lower.profile(terminal_base);
        record_composition(&mut profile, shape, &plan);
        lowering.push(profile);
        memo_layouts.push(lower.memo_layout(plan.effects.needs_peak));
        terminals = lower.pure.program.terminals;
        let roots = root_filter(grammar, file_guard);
        terminal_owners.resize(terminals.len(), clause);
        plans.push((plan, lower.slots, grammar.applies, roots));
    }
    let borrowed: Vec<_> = plans
        .iter()
        .map(|(plan, slots, applies, roots)| (plan, slots.as_slice(), *applies, roots.as_str()))
        .collect();
    let (sql, mut relations) = emit::compile_jobs(&borrowed, work, &memo_layouts)?;
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

#[cfg(test)]
#[path = "parse_recursive_tests.rs"]
mod parse_recursive_tests;

#[cfg(test)]
#[path = "capture_recursive_tests.rs"]
mod capture_recursive_tests;

#[cfg(test)]
mod finite_tests;

#[cfg(test)]
#[path = "field_stopped_tests.rs"]
mod field_stopped_tests;

#[cfg(test)]
#[path = "finite_guard.rs"]
mod finite_guard;
