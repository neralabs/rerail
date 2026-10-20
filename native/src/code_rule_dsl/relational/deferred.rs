//! Narrow physical stages with SQL views for explicitly deferred columns.
use super::{Connection, Result, Stage, inspect, quote, sql_error};
use std::collections::BTreeSet;

pub(in crate::code_rule_dsl) fn validate(db: &Connection, stage: &Stage) -> Result<bool> {
    if stage.deferred.is_empty() {
        return Ok(false);
    }
    let backing = format!("__stage_{}", stage.name);
    db.execute_batch(&format!(
        "CREATE TEMP TABLE {} AS SELECT {} FROM {} LIMIT 0",
        quote(&backing),
        stage
            .stored_columns
            .iter()
            .map(|s| quote(s))
            .collect::<Vec<_>>()
            .join(","),
        quote(&stage.name)
    ))
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
    db.execute_batch(&format!("DROP TABLE {}", quote(&backing)))
        .map_err(sql_error)?;
    Ok(reads_source)
}

pub(super) fn physical_name(stage: &Stage) -> String {
    if stage.deferred.is_empty() {
        stage.name.clone()
    } else {
        format!("__stage_{}", stage.name)
    }
}

pub(super) fn view(db: &Connection, stage: &Stage) -> Result<()> {
    if stage.deferred.is_empty() {
        return Ok(());
    }
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
    db.execute_batch(&format!(
        "CREATE TEMP VIEW {} AS SELECT {select} FROM {}",
        quote(&stage.name),
        quote(&physical_name(stage))
    ))
    .map_err(sql_error)
}

pub(super) fn release(db: &Connection, name: &str, stages: &[Stage]) -> Result<()> {
    let stage = stages.iter().find(|s| s.name == name);
    match stage {
        Some(stage) if stage.reachability.is_some() => db.execute_batch(&format!(
            "DROP VIEW {};DROP TABLE {};DROP TABLE {}",
            quote(name),
            quote(&format!("__reach_{name}_keys")),
            quote(&format!("__reach_{name}_pairs"))
        )),
        Some(stage) if !stage.deferred.is_empty() => db.execute_batch(&format!(
            "DROP VIEW {};DROP TABLE {}",
            quote(name),
            quote(&physical_name(stage))
        )),
        _ => db.execute_batch(&format!("DROP TABLE {}", quote(name))),
    }
    .map_err(sql_error)
}
