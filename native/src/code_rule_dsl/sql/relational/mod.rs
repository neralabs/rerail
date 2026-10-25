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
mod capture_state;
mod classify;
#[cfg(test)]
mod compiler_shape_tests;
mod continuation_store;
mod cutover_tests;
mod demand;
mod emit;
mod failure;
mod file_condition_tests;
mod file_guard;
mod format_facts;
mod ir;
mod job;
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
mod planner_stats;
mod recursion;
mod recursion_tests;
mod recursive_chain;
mod recursive_downward;
mod row;
mod shared_job;
mod sibling_state;
mod span_capture;
#[cfg(test)]
mod spike_stages;
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
    compile_with_stats(grammar, &planner_stats::PlannerStats::default())
}

pub(super) fn compile_with_stats(
    grammar: &GrammarMatcher,
    stats: &planner_stats::PlannerStats,
) -> Result<Compiled, String> {
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
    match compile_total_with_prefix(grammar, 0, stats.for_dialect(grammar.language)) {
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
    compile_total_with_prefix(grammar, 0, None)
}
fn compile_total_with_prefix(
    grammar: &GrammarMatcher,
    prefix: usize,
    stats: Option<&planner_stats::PlannerStats>,
) -> Result<Compiled, String> {
    analysis::admit(grammar)?;
    let mut program = if prefix == 0 {
        lower::compile(grammar)?
    } else {
        lower::compile_with_prefix(grammar, prefix)?
    };
    // Check the original dependency graph before specializing any consumer.
    physical::plan(&program)?;
    demand::apply(&mut program);
    let plan = physical::plan(&program)?;
    let active = ir::reachable(&program);
    let mut relations = serde_json::json!({
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
    let sql = if let Some(stats) = stats {
        let (sql, decisions, edges) = emit::emit_with_stats_detailed(&program, &plan, stats)?;
        relations["physicalPlanning"] = serde_json::json!({
            "statistics": "exact atomic counts for this language and file batch",
            "anchors": decisions.iter().map(|d| serde_json::json!({"variable":d.variable,"estimatedRows":d.estimated_rows,"reason":d.reason})).collect::<Vec<_>>(),
            "edges": edges.iter().map(|d| serde_json::json!({"from":d.from,"to":d.to,"known":d.known,"target":d.target,"observedEdges":d.observed_edges,"knownRows":d.known_rows})).collect::<Vec<_>>(),
            "storage": planner_stats::materialization_decisions(&program,&plan,stats).iter().map(|d| serde_json::json!({"relation":d.relation,"consumers":d.consumers,"materialized":d.materialized,"reason":d.reason})).collect::<Vec<_>>(),
            "semanticReordering": false
        });
        sql
    } else {
        emit::emit(&program, &plan)?
    };
    Ok(Compiled {
        sql,
        terminals: program.terminals,
        relations,
    })
}

// Prefix IDs are assigned before lowering. SQL is never reparsed or rewritten.
fn seed_terminals(prefix: usize) -> Vec<Terminal> {
    if prefix == 0 {
        return vec![];
    }
    let empty = regex::Regex::new(r"\b\B").expect("empty regex");
    (0..prefix)
        .map(|_| Terminal::Regex(empty.clone()))
        .collect()
}

#[cfg(test)]
mod job_relation_spike;
#[cfg(test)]
mod staged_job_spike;

pub(super) fn compile_namespaced_with_stats(
    grammar: &GrammarMatcher,
    prefix: usize,
    clause: usize,
    stats: &planner_stats::PlannerStats,
) -> Result<Compiled, String> {
    // Packed jobs expose remaining work. Set-only compilation can erase utility
    // charges even when result truth is total; retain the ordered work contract.
    if grammar.file.is_some() || !grammar.utils.names.is_empty() {
        return outcomes::compile_namespaced(grammar, prefix, clause);
    }
    match classify::classify(grammar)? {
        classify::Representation::OrderedOutcomes => {
            outcomes::compile_namespaced(grammar, prefix, clause)
        }
        classify::Representation::CapturedRelations => {
            match answers::prepare_with_prefix(grammar, prefix)? {
                answers::Eligibility::Applicable(plan) => plan.emit(),
                answers::Eligibility::NotApplicable(_) => {
                    outcomes::compile_namespaced(grammar, prefix, clause)
                }
            }
        }
        classify::Representation::TotalRelations => {
            match compile_total_with_prefix(grammar, prefix, stats.for_dialect(grammar.language)) {
                Ok(compiled) => Ok(compiled),
                Err(_) => outcomes::compile_namespaced(grammar, prefix, clause),
            }
        }
    }
}

// Reuse SQLite loop instrumentation for every common-compiler statement.
use super::scan;
