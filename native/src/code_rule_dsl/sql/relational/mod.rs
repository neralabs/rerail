//! Direct DSL-to-relation compiler. Test-only until the transition's semantic coverage gate.
//! No frame IR, SQL reparsing, rule-ID recognizers, or native composition callbacks.
mod analysis;
mod demand;
mod emit;
mod ir;
mod leaves;
mod lower;
mod physical;
pub(super) mod tests;

use super::terminals::Terminal;
use crate::code_rule_dsl::matcher::GrammarMatcher;

pub(super) struct Compiled {
    pub sql: String,
    pub terminals: Vec<Terminal>,
    pub relations: serde_json::Value,
}

pub(super) fn compile(grammar: &GrammarMatcher) -> Result<Compiled, String> {
    analysis::admit(grammar)?;
    let mut program = lower::compile(grammar)?;
    // Check the original dependency graph before specializing any consumer.
    physical::plan(&program)?;
    demand::apply(&mut program);
    let plan = physical::plan(&program)?;
    let active = ir::reachable(&program);
    let relations = serde_json::json!({
        "scope": "capture-free total relations, ancestor closure, and proved subtree demand",
        "demandDecisions":program.demand_decisions,
        "regionRoots":active.iter().filter_map(|id|program.relations[*id].domain).collect::<std::collections::BTreeSet<_>>(),
        "key": ["file", "node"],
        "captureReads": [], "captureWrites": [], "multiplicity": "set",
        "outcomes": "two-valued", "nativeComposition": false,
        "limitations": ["Native utility work and stack/depth budgets are not reproduced by this test-only kernel."],
        "relations": program.relations.iter().enumerate().filter(|(id,_)|active.contains(id)).map(|(id, r)| serde_json::json!({
            "id":id, "source":r.source, "schema":ir::SCHEMA,
            "operator":format!("{:?}",r.expression),
            "domain":r.domain,
            "keyDemand":r.keys,
            "arms":match &plan[id] {physical::RelationPlan::Joins(arms)=>arms.len(),_=>2},
            "recursive":match &plan[id] {physical::RelationPlan::Joins(arms)=>arms.iter().any(|a| a.references.iter().any(|(_,r)| *r==id)),_=>true},
        })).collect::<Vec<_>>()
    });
    Ok(Compiled {
        sql: emit::emit(&program, &plan)?,
        terminals: program.terminals,
        relations,
    })
}
