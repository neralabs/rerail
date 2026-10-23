//! Direct DSL-to-relation compiler. Test-only until the transition's semantic coverage gate.
//! No frame IR, SQL reparsing, rule-ID recognizers, or native composition callbacks.
mod analysis;
mod answers;
mod demand;
mod emit;
mod failure;
mod ir;
pub(super) mod leaves;
mod lower;
mod outcomes;
mod physical;
mod recursion;
mod recursion_tests;
mod row;
pub(super) mod tests;
pub(super) mod witness;
pub(super) mod witness_tests;

use super::terminals::Terminal;
use crate::code_rule_dsl::matcher::GrammarMatcher;

pub(super) struct Compiled {
    pub sql: String,
    pub terminals: Vec<Terminal>,
    pub relations: serde_json::Value,
}

pub(super) fn compile(grammar: &GrammarMatcher) -> Result<Compiled, String> {
    if grammar.three_valued {
        return outcomes::compile(grammar);
    }
    if !crate::code_rule_dsl::matcher::CaptureAnalysis::default()
        .mentions(&grammar.matcher)
        .is_empty()
    {
        if grammar.three_valued
            || grammar.transform.is_some()
            || !crate::code_rule_dsl::matcher::CaptureAnalysis::default()
                .counts(&grammar.matcher)
                .is_empty()
        {
            return Err("relational compiler: unsupported capture effects or outcomes".into());
        }
        return answers::compile(grammar)
            .map_err(|e| format!("relational compiler: capture-bearing rules: {e}"));
    }
    analysis::admit(grammar)?;
    let mut program = lower::compile(grammar)?;
    // Check the original dependency graph before specializing any consumer.
    physical::plan(&program)?;
    demand::apply(&mut program);
    let plan = physical::plan(&program)?;
    let active = ir::reachable(&program);
    let relations = serde_json::json!({
        "scope": "capture-free total relations, structural components, ancestor closure, and proved subtree demand",
        "components":plan.iter().enumerate().filter_map(|(id,p)|match p {
            physical::RelationPlan::Component {leader,members,..} if id==*leader => Some(serde_json::json!({
                "id":leader,"members":members,"key":["state","file","node"],
                "termination":"strict common preorder direction", "multiplicity":"set", "storage":"materialized recursive UNION",
                "demand":"shared complete answers; component-wide demand propagation pending"
            })),_=>None
        }).collect::<Vec<_>>(),
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
            "arms":match &plan[id] {physical::RelationPlan::Joins(arms)|physical::RelationPlan::Component{arms,..}=>arms.len(),_=>2},
            "recursive":match &plan[id] {physical::RelationPlan::Joins(arms)=>arms.iter().any(|a| a.references.iter().any(|(_,r)| *r==id)),_=>true},
        })).collect::<Vec<_>>()
    });
    Ok(Compiled {
        sql: emit::emit(&program, &plan)?,
        terminals: program.terminals,
        relations,
    })
}
