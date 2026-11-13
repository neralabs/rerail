//! Infer storage only for selected physical producers. Logical SQL scopes retain SQLite types.
use crate::code_rule_dsl::{
    json::invalid,
    waves::relational::{self, Plan, schema},
};
use napi::Result;
use std::sync::Arc;

pub(super) fn finish(
    plan: &mut Plan,
    db: &relational::database::Database,
    schemas: &mut schema::Registry,
) -> Result<()> {
    let mut inference = schema::inference::Inference::default();
    for stage in &plan.stages {
        if stage.reachability.is_some() {
            // This is the declared native output contract, not an inferred SQL expression.
            let columns = Arc::new(vec![
                schema::Column {
                    name: "__path".into(),
                    declared_type: "TEXT".into(),
                },
                schema::Column {
                    name: "root".into(),
                    declared_type: String::new(),
                },
                schema::Column {
                    name: "node".into(),
                    declared_type: String::new(),
                },
            ]);
            schema::bind(db, &stage.name, schema::declaration(&stage.name, &columns))
                .map_err(relational::sql_error)?;
            schemas.insert(stage.name.clone(), columns);
            continue;
        }
        let reads = stage.inputs.iter().cloned().collect::<Vec<_>>();
        inference
            .before(db, schemas, &reads, &stage.query, &stage.output_columns)
            .map_err(relational::sql_error)?;
        let key = schema::cache::Key::new(&stage.query, schemas, reads.iter(), std::iter::empty());
        let cached = schema::cache::shared().get(&key).cloned();
        if let Some(columns) = cached {
            schema::bind(db, &stage.name, schema::declaration(&stage.name, &columns))
                .map_err(relational::sql_error)?;
            schemas.insert(stage.name.clone(), columns);
        } else {
            inference
                .queue(
                    db,
                    schemas,
                    &stage.name,
                    &stage.query,
                    stage.output_columns.clone(),
                    key,
                )
                .map_err(relational::sql_error)?;
        }
    }
    inference
        .flush(db, schemas)
        .map_err(relational::sql_error)?;
    for stage in &mut plan.stages {
        stage.declaration = schema::declaration(&stage.name, &schemas[&stage.name])
            .replacen("CREATE TABLE", "CREATE TEMP TABLE", 1)
            .into();
    }
    let allowed = plan
        .inputs
        .iter()
        .map(|(name, _)| name.clone())
        .chain(plan.stages.iter().map(|stage| stage.name.clone()))
        .chain(plan.syntax.iter().flat_map(|syntax| {
            [
                Some(syntax.name.clone()),
                syntax.edges.clone(),
                syntax.ownership.clone(),
            ]
            .into_iter()
            .flatten()
        }))
        .chain(plan.references.iter().map(|_| "reference_edges".to_owned()))
        .collect();
    relational::inspect_fields(db, &plan.query, &allowed)
        .map_err(|error| invalid(format!("composed relational query: {}", error.reason)))?;
    Ok(())
}
