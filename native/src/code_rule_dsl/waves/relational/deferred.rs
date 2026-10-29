//! Narrow physical stages with SQL views for explicitly deferred columns.
use super::{Connection, Result, Stage, inspect, quote, sql_error};
use std::collections::BTreeSet;

pub(in crate::code_rule_dsl) fn validate(
    db: &Connection,
    stage: &Stage,
    columns: &[super::schema::Column],
) -> Result<bool> {
    if stage.deferred.is_empty() {
        return Ok(false);
    }
    let backing = format!("__stage_{}", stage.name);
    let columns = stage
        .stored_columns
        .iter()
        .map(|name| {
            columns
                .iter()
                .find(|c| &c.name == name)
                .expect("validated stored column")
                .clone()
        })
        .collect::<Vec<_>>();
    super::schema::bind(db, &backing, super::schema::declaration(&backing, &columns))
        .map_err(sql_error)?;
    let allowed = BTreeSet::from([backing.clone()]);
    let mut reads_source = false;
    for (field, expression) in &stage.deferred {
        let (_, _, source) = inspect(
            db,
            &format!(
                "SELECT ({expression}) AS {} FROM {}",
                quote(field),
                quote(&backing)
            ),
            &allowed,
        )?;
        reads_source |= source;
    }
    Ok(reads_source)
}

pub(in crate::code_rule_dsl) fn physical_name(stage: &Stage) -> String {
    if stage.deferred.is_empty() {
        stage.name.clone()
    } else {
        format!("__stage_{}", stage.name)
    }
}

fn projection(stage: &Stage) -> String {
    let select = stage
        .output_columns
        .iter()
        .map(|name| {
            stage.deferred.get(name).map_or_else(
                || quote(name),
                |expr| format!("({expr}) AS {}", quote(name)),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("SELECT {select} FROM {}", quote(&physical_name(stage)))
}

pub(in crate::code_rule_dsl) fn expose(
    db: &Connection,
    stage: &Stage,
    physical: super::schema::SharedColumns,
    created: bool,
) -> Result<super::schema::SharedColumns> {
    if stage.deferred.is_empty() {
        return Ok(physical);
    }
    let backing = physical_name(stage);
    let input = super::schema::describe(db, &backing).map_err(sql_error)?;
    let schemas = super::schema::Registry::from([(backing.clone(), std::sync::Arc::new(input))]);
    let query = projection(stage);
    let key = super::schema::cache::Key::new(&query, &schemas, schemas.keys(), std::iter::empty());
    let mut cache = super::schema::cache::shared();
    let columns = if let Some(columns) = cache.get(&key) {
        columns.clone()
    } else {
        let name = format!("__deferred_schema_{}", stage.name);
        db.execute_batch(&format!(
            "CREATE TEMP TABLE {} AS SELECT * FROM ({query}) LIMIT 0",
            quote(&name)
        ))
        .map_err(sql_error)?;
        let columns = std::sync::Arc::new(super::schema::describe(db, &name).map_err(sql_error)?);
        db.execute_batch(&format!("DROP TABLE {}", quote(&name)))
            .map_err(sql_error)?;
        cache.insert(key, columns.clone());
        columns
    };
    if created {
        db.execute_batch(&format!("DROP TABLE {}", quote(&stage.name)))
            .map_err(sql_error)?;
    }
    super::schema::bind(
        db,
        &stage.name,
        super::schema::declaration(&stage.name, &columns),
    )
    .map_err(sql_error)?;
    Ok(columns)
}

pub(super) fn view(db: &super::Database, stage: &Stage) -> Result<()> {
    if stage.deferred.is_empty() {
        return Ok(());
    }
    let query = projection(stage);
    db.storage
        .borrow_mut()
        .declare(
            db,
            &stage.name,
            format!("CREATE TEMP VIEW {} AS {query}", quote(&stage.name)),
            false,
        )
        .map_err(sql_error)
}

pub(super) fn release(db: &Connection, name: &str, stages: &[Stage]) -> Result<()> {
    let stage = stages.iter().find(|s| s.name == name);
    match stage {
        Some(stage)
            if stage
                .reachability
                .as_ref()
                .is_some_and(|s| s.syntax.is_none()) =>
        {
            let forest = stage.reachability.as_ref().unwrap().forest;
            db.execute_batch(&format!(
                "DROP VIEW {};DROP TABLE {};DROP {} {}{}",
                quote(name),
                quote(&format!("__reach_{name}_keys")),
                if forest { "VIEW" } else { "TABLE" },
                quote(&format!("__reach_{name}_pairs")),
                if forest {
                    format!(
                        ";DROP TABLE {};DROP TABLE {}",
                        quote(&format!("__reach_{name}_ranges")),
                        quote(&format!("__reach_{name}_positions"))
                    )
                } else {
                    String::new()
                }
            ))
        }
        Some(stage) if !stage.deferred.is_empty() => db.execute_batch(&format!(
            "DROP VIEW {};DROP TABLE {}",
            quote(name),
            quote(&physical_name(stage))
        )),
        _ => db.execute_batch(&format!("DROP TABLE {}", quote(name))),
    }
    .map_err(sql_error)
}
