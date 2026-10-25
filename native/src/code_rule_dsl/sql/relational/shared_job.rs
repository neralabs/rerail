//! Invocation-local producers shared by distinct, pure logical clauses.
//!
//! Equality is lowered Set equality with terminals interned by the same Lowerer.
//! Nothing here identifies policies or rewrites SQL produced by another compiler.
use super::{
    emit,
    ir::Set,
    job::{Arm, Pack},
    lower::Lowerer,
    planner_stats::PlannerStats,
};
use crate::code_rule_dsl::matcher::{AstMatcher, GrammarMatcher, StopBy};

/// These atoms and immediate edges spend no native work, mutate no environment,
/// and cannot return Unknown. In particular, utility calls are not pure atoms:
/// even a utility whose body is Kind spends an entry and observes memo state.
fn zero_effect(rule: &AstMatcher, operators: &mut usize) -> bool {
    *operators += 1;
    if *operators > 256 {
        return false;
    }
    match rule {
        AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => true,
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            items.iter().all(|item| zero_effect(item, operators))
        }
        AstMatcher::Not(child) => zero_effect(child, operators),
        AstMatcher::Inside(child, StopBy::Neighbor)
        | AstMatcher::Has(child, StopBy::Neighbor)
        | AstMatcher::Follows(child, StopBy::Neighbor)
        | AstMatcher::Precedes(child, StopBy::Neighbor) => zero_effect(child.rule(), operators),
        _ => false,
    }
}

/// Automatic physical alternative for a batch containing repeated pure IR.
/// Each execution builds its MATERIALIZED producers afresh, including an empty
/// execution, so prepared reuse needs neither persistent tables nor reset calls.
pub(super) fn try_compile(
    grammars: &[&GrammarMatcher],
    _stats: &PlannerStats,
) -> Result<Option<Pack>, String> {
    let Some(first) = grammars.first() else {
        return Ok(None);
    };
    if grammars.len() < 2
        || grammars.iter().any(|grammar| {
            grammar.language != first.language
                || grammar.file.is_some()
                || grammar.transform.is_some()
                || grammar.three_valued
                || !zero_effect(&grammar.matcher, &mut 0)
        })
    {
        return Ok(None);
    }
    let mut lower = Lowerer::new(first.language);
    let mut families = Vec::<Set>::new();
    let mut demands = Vec::with_capacity(grammars.len());
    for grammar in grammars {
        let expression = if grammar.applies {
            lower.set(&grammar.matcher)?
        } else {
            Set::Empty
        };
        let family = match families.iter().position(|other| *other == expression) {
            Some(family) => family,
            None => {
                families.push(expression);
                families.len() - 1
            }
        };
        demands.push(family);
    }
    // A closed producer must have more than one logical consumer. An all-unique
    // batch keeps the ordinary compiler and avoids materialization overhead.
    if families.len() == demands.len() {
        return Ok(None);
    }
    let mut definitions = Vec::with_capacity(families.len() + 2);
    for (family, expression) in families.iter().enumerate() {
        let condition = emit::condition(expression, "n")?;
        definitions.push(format!(
            "shared_family_{family}(file,node) AS MATERIALIZED (SELECT n.file,n.id FROM node n WHERE {condition})"
        ));
    }
    definitions.push(format!(
        "shared_demands(clause,family) AS (VALUES {})",
        demands
            .iter()
            .enumerate()
            .map(|(clause, family)| format!("({clause},{family})"))
            .collect::<Vec<_>>()
            .join(",")
    ));
    definitions.push(format!(
        "shared_answers(family,file,node) AS ({})",
        (0..families.len())
            .map(|family| format!("SELECT {family},file,node FROM shared_family_{family}"))
            .collect::<Vec<_>>()
            .join(" UNION ALL ")
    ));
    let sql = format!(
        "WITH {} SELECT d.clause,p.file,p.node,json_array(),0,9223372036854775807 FROM shared_demands d JOIN shared_answers p ON p.family=d.family ORDER BY d.clause,p.file,p.node",
        definitions.join(",")
    );
    if sql.len() > 256_000 {
        return Err(format!(
            "shared pure job: {} SQL bytes exceed hard statement budget256000",
            sql.len()
        ));
    }
    let arms = demands
        .iter()
        .enumerate()
        .map(|(clause, family)| Arm {
            clause,
            // The job publisher uses clause identity and capture names; this
            // fragment also documents the exact producer selected by the arm.
            sql: format!("SELECT file,node FROM shared_family_{family}"),
            columns: 6,
            names: Vec::new(),
        })
        .collect();
    Ok(Some(Pack {
        sql,
        stages: Vec::new(),
        terminals: lower.program.terminals,
        arms,
    }))
}

#[cfg(test)]
#[path = "shared_job_tests.rs"]
mod tests;
