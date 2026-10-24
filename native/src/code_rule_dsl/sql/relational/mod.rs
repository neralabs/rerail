//! Direct DSL-to-relation compiler. Test-only until the transition's semantic coverage gate.
//! No frame IR, SQL reparsing, rule-ID recognizers, or native composition callbacks.
mod analysis;
mod answers;
mod bound;
mod bound_leaf;
mod budget_leaf;
mod call_unify;
mod capture_cell;
mod capture_env;
mod classify;
mod continuation_store;
mod cutover_tests;
mod demand;
mod emit;
mod failure;
mod file_condition_tests;
mod file_guard;
mod format_facts;
mod ir;
pub(super) mod leaves;
mod lower;
mod memo_store;
mod outcomes;
mod parse_export;
mod parse_facts;
mod pattern_capture;
mod pattern_leaf;
mod peak_tracker;
mod physical;
mod recursion;
mod recursion_tests;
mod recursive_chain;
mod recursive_downward;
mod row;
mod sibling_state;
mod span_capture;
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
    if grammar.file.is_some() {
        return outcomes::compile(grammar);
    }
    match classify::classify(grammar)? {
        classify::Representation::OrderedOutcomes => return outcomes::compile(grammar),
        classify::Representation::CapturedRelations => {
            return match answers::prepare(grammar)? {
                answers::Eligibility::Applicable(plan) => plan.emit(),
                answers::Eligibility::NotApplicable(reason) => {
                    let mut compiled = outcomes::compile(grammar)?;
                    compiled.relations["frontierInapplicable"] =
                        serde_json::json!(reason.to_string());
                    Ok(compiled)
                }
            };
        }
        classify::Representation::TotalRelations => {}
    }
    match compile_total(grammar) {
        Ok(compiled) => Ok(compiled),
        Err(reason) => {
            let mut compiled = outcomes::compile(grammar)
                .map_err(|error| format!("{error}; total representation: {reason}"))?;
            compiled.relations["totalInapplicable"] = serde_json::json!(reason);
            Ok(compiled)
        }
    }
}

fn compile_total(grammar: &GrammarMatcher) -> Result<Compiled, String> {
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
