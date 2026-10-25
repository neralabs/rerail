//! Bounded job composition. Every branch receives all compatible files.
//! Terminal and clause identities are allocated before SQL generation.
use super::{Compiled, compile_namespaced_with_stats, planner_stats::PlannerStats};
use crate::code_rule_dsl::{matcher::GrammarMatcher, sql::terminals::Terminal};
pub(super) struct Arm {
    pub(super) clause: usize,
    pub(super) sql: String,
    pub(super) columns: usize,
    pub(super) names: Vec<String>,
}
pub(super) struct Pack {
    pub(super) sql: String,
    pub(super) stages: Vec<String>,
    pub(super) terminals: Vec<Terminal>,
    pub(super) arms: Vec<Arm>,
}
pub(super) fn names(compiled: &Compiled) -> Vec<String> {
    if let Some(c) = compiled.relations["captures"].as_array() {
        c.iter()
            .map(|c| c["name"].as_str().unwrap().into())
            .collect()
    } else {
        compiled.relations["capture"]
            .as_str()
            .map(String::from)
            .into_iter()
            .collect()
    }
}
fn packed_sql(arms: &[Arm]) -> String {
    let definitions = arms
        .iter()
        .enumerate()
        .map(|(index, arm)| {
            format!(
                "branch_{index}({}) AS MATERIALIZED ({})",
                if arm.columns == 6 {
                    "clause,file,node,payload,failure,work"
                } else {
                    "clause,file,node,payload"
                },
                arm.sql
            )
        })
        .collect::<Vec<_>>();
    let selects = arms
        .iter()
        .enumerate()
        .map(|(index, arm)| {
            format!(
                "SELECT {} clause,file,node,payload,{} failure,{} work FROM branch_{index}",
                arm.clause,
                if arm.columns == 6 { "failure" } else { "0" },
                if arm.columns == 6 {
                    "work"
                } else {
                    "9223372036854775807"
                }
            )
        })
        .collect::<Vec<_>>();
    format!(
        "WITH RECURSIVE {} {} ORDER BY clause,file,node",
        definitions.join(","),
        selects.join(" UNION ALL ")
    )
}
pub(super) fn compile(
    grammars: &[&GrammarMatcher],
    stats: &PlannerStats,
) -> Result<Vec<Pack>, String> {
    if grammars.iter().any(|g| {
        grammars
            .first()
            .is_some_and(|first| first.language != g.language)
    }) {
        return Err(
            "job composition: split incompatible document languages before compilation".into(),
        );
    }
    if let Some(pack) = super::shared_job::try_compile(grammars, stats)? {
        return Ok(vec![pack]);
    }
    let mut packs = vec![];
    let mut arms = vec![];
    let mut terminals = vec![];
    let mut stages = vec![];
    for (clause, grammar) in grammars.iter().enumerate() {
        loop {
            let prefix = terminals.len();
            let mut compiled = compile_namespaced_with_stats(grammar, prefix, clause, stats)?;
            let capture_names = names(&compiled);
            arms.push(Arm {
                clause,
                sql: compiled.sql,
                columns: if compiled.relations["failureColumn"].as_u64() == Some(4) {
                    6
                } else {
                    4
                },
                names: capture_names,
            });
            let sql = packed_sql(&arms);
            if arms.len() <= 32 && sql.len() <= 256_000 {
                stages.extend(
                    compiled.relations["physicalStages"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s.as_str())
                        .map(str::to_owned),
                );
                terminals.extend(compiled.terminals.drain(prefix..));
                break;
            }
            let rejected = arms.pop().unwrap();
            if arms.is_empty() {
                return Err(format!(
                    "clause {clause} closed branch exceeds packing bound: {} bytes",
                    sql.len()
                ));
            }
            let sql = packed_sql(&arms);
            packs.push(Pack {
                sql,
                stages: std::mem::take(&mut stages),
                terminals: std::mem::take(&mut terminals),
                arms: std::mem::take(&mut arms),
            });
            drop(rejected);
        }
    }
    if !arms.is_empty() {
        packs.push(Pack {
            sql: packed_sql(&arms),
            terminals,
            stages,
            arms,
        })
    }
    let executions: usize = packs.iter().map(|p| 1 + p.stages.len()).sum();
    if executions > 100 {
        return Err(format!(
            "job composition: {} executions exceed hard budget100",
            executions
        ));
    }
    Ok(packs)
}
