//! Overlay relationship rows written by publication, and their revalidation before commit.

use super::publication::{stored_object_identities, stored_path_ids};
use super::{
    HashKey, OverlayFilePublication, SourceId, StoreError, StoreResult, TapeId, WorkspaceId,
};
use rusqlite::Connection;

pub(super) struct PreparedSqlStatement {
    sql: String,
    values: Vec<rusqlite::types::Value>,
}

pub(super) struct PreparedRelationshipPass {
    statements: Vec<PreparedSqlStatement>,
}

impl PreparedRelationshipPass {
    /// The data statements `execute_relationship_pass` runs.
    pub(super) fn statement_count(&self) -> usize {
        self.statements.len()
    }
}

pub(super) fn prepare_relationship_pass<'a>(
    workspace_id: WorkspaceId,
    rows: &[(
        i64,
        i64,
        SourceId,
        Option<TapeId>,
        OverlayFilePublication<'a>,
    )],
) -> StoreResult<PreparedRelationshipPass> {
    let mut statements = Vec::new();
    let mut overlay_sql = String::from(
        "INSERT INTO workspace_overlay(workspace_id,path_id,state_id,source_id,tape_id) VALUES ",
    );
    let mut overlay_values = Vec::with_capacity(rows.len() * 5);
    for (index, (path_id, state_id, source_id, tape_id, _)) in rows.iter().enumerate() {
        if index != 0 {
            overlay_sql.push(',');
        }
        overlay_sql.push_str("(?,?,?,?,?)");
        overlay_values.push(workspace_id.0.into());
        overlay_values.push((*path_id).into());
        overlay_values.push((*state_id).into());
        overlay_values.push(source_id.0.into());
        overlay_values.push(tape_id.map_or(rusqlite::types::Value::Null, |id| id.0.into()));
    }
    overlay_sql.push_str(" ON CONFLICT(workspace_id,path_id) DO UPDATE SET state_id=excluded.state_id,source_id=excluded.source_id,tape_id=excluded.tape_id");
    statements.push(PreparedSqlStatement {
        sql: overlay_sql,
        values: overlay_values,
    });

    let pass = PreparedRelationshipPass { statements };
    if pass.statement_count() > 10 {
        return Err(StoreError::Invalid(format!(
            "relationship publication prepared {} statements; maximum is 10",
            pass.statement_count()
        )));
    }
    Ok(pass)
}

pub(super) fn relationship_references_current(
    transaction: &Connection,
    rows: &[(
        i64,
        i64,
        SourceId,
        Option<TapeId>,
        OverlayFilePublication<'_>,
    )],
    path_hashes: &[(&str, HashKey)],
    hashes: &[(HashKey, Option<HashKey>)],
) -> StoreResult<bool> {
    if hashes.len() != rows.len() || path_hashes.len() != rows.len() {
        return Err(StoreError::Corrupt(
            "relationship hash input omitted a publication".to_owned(),
        ));
    }
    let paths = stored_path_ids(transaction, path_hashes)?;
    if rows
        .iter()
        .any(|(path_id, _, _, _, publication)| paths.get(publication.path) != Some(path_id))
    {
        return Ok(false);
    }
    let identities = rows
        .iter()
        .zip(hashes)
        .map(|((_, _, _, _, publication), (source_hash, tape_hash))| {
            (publication, *source_hash, *tape_hash)
        })
        .collect::<Vec<_>>();
    let stored = stored_object_identities(transaction, &identities)?;
    if rows.iter().zip(stored).any(
        |((_, _, source_id, tape_id, _), (stored_source, stored_tape))| {
            stored_source != Some(*source_id) || stored_tape != *tape_id
        },
    ) {
        return Ok(false);
    }
    Ok(true)
}

/// Runs the pass in `transaction`.
pub(super) fn execute_relationship_pass(transaction: &Connection, pass: &PreparedRelationshipPass) -> StoreResult<()> {
    for statement in &pass.statements {
        transaction
            .prepare_cached(&statement.sql)?
            .execute(rusqlite::params_from_iter(statement.values.iter()))?;
    }
    Ok(())
}
