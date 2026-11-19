//! Projection syntax does not choose a different storage or maintenance engine.
use super::*;

pub fn normalize_views(
    e: &mut plan::emit::Emitted,
) -> usize {
    // Keep the checked queries for semantic domains and explain; their execution is a relation step.
    let views = e.views.clone();
    let count = views.len();
    let setup = views
        .iter()
        .map(|(r, q)| plan::view_sql(r, q))
        .collect::<BTreeSet<_>>();
    e.setup.retain(|sql| !setup.contains(sql));
    let names = views
        .iter()
        .map(|(r, _)| r.clone())
        .collect::<BTreeSet<_>>();
    for (name, query) in views {
        e.program
            .relations
            .push((name.clone(), query.body.columns.len()));
        e.program.steps.push(schedule::Step {
            output: name.clone(),
            stratum: 0,
            reads: vec![],
            sql: format!("INSERT OR IGNORE INTO new_{name} {}", query.render()),
        });
        e.queries.push(Some(query));
    }
    for (step, query) in e.program.steps.iter_mut().zip(&e.queries) {
        if let Some(query) = query {
            for relation in query.reads().iter().filter_map(|r| r.program_relation()) {
                if names.contains(relation) && !step.reads.iter().any(|r| r == relation) {
                    step.reads.push(relation.into());
                }
            }
        }
    }
    count
}
