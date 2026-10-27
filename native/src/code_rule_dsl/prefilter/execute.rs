//! Physical SQL boundaries for shared candidate sets. Temporary storage can spill to disk.
//! Adapted from Pulling-it-all-together's `matcher/anchors/sql/execute.rs`: statements run on the
//! content store's connection (`ContentStore::candidate_transaction`) rather than a private
//! anchor database, so there are no incremental tape readers to close and no structural fill.
use super::lower::{Statement, lifetime::Action};
use crate::content_store::{ContentStore, StoreResult};
use rusqlite::{Connection, Result, params_from_iter};

/// The candidate rows of `statement`: one transaction, which writes only when the statement
/// registers vocabulary predicates.
pub(super) fn candidates(
    store: &ContentStore,
    statement: &Statement,
    mut emit: impl FnMut(usize, i64, usize),
) -> StoreResult<()> {
    // A busy writer lock fails BEGIN IMMEDIATE before any row is emitted, so a retry repeats none.
    store.candidate_transaction(statement.writes(), |db| rows(db, statement, &mut emit))
}

/// Fills the `request_file` temp table with the requested tapes.
fn request(db: &Connection, files: &[i64]) -> Result<()> {
    db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS request_file(fid INTEGER PRIMARY KEY) WITHOUT ROWID")?;
    db.execute("DELETE FROM request_file", [])?;
    for files in files.chunks(128) {
        let sql = format!(
            "INSERT OR IGNORE INTO request_file VALUES {}",
            std::iter::repeat_n("(?)", files.len())
                .collect::<Vec<_>>()
                .join(",")
        );
        db.execute(&sql, params_from_iter(files))?;
    }
    Ok(())
}

/// Runs `statement` in the caller's transaction on `db`, emitting (plan, tape, preorder) rows.
fn rows(db: &Connection, statement: &Statement, emit: &mut impl FnMut(usize, i64, usize)) -> Result<()> {
    let relation_names: Vec<_> = statement.stages.iter().map(|s| s.name.as_str()).collect();
    let mut live = vec![false; statement.stages.len()];
    let mut populated = vec![false; statement.stages.len()];
    let mut live_count = 0usize;
    let mut owner_created = false;
    let result = (|| {
        request(db, &statement.files)?;
        for sql in &statement.ownership.setup {
            db.execute_batch(sql)?;
            owner_created = true;
        }
        for actions in statement
            .schedule
            .actions
            .chunk_by(|a, b| matches!(a, Action::Release(_)) && matches!(b, Action::Release(_)))
        {
            match actions[0] {
                Action::Prepare(i) => {
                    let stage = &statement.stages[i];
                    let mut create = db.prepare(&stage.sql)?;
                    let count = create.parameter_count();
                    let inserted =
                        create.execute(params_from_iter(&statement.parameters[..count]))?;
                    if stage.temporary {
                        if statement.ownership.slot(i).is_some() {
                            for &member in statement.ownership.members(i) {
                                live[member] = true;
                                populated[member] = inserted != 0;
                                live_count += 1;
                            }
                        } else {
                            live[i] = true;
                            live_count += 1;
                        }
                    }
                    drop(create);
                    if let Some(columns) = stage.index {
                        db.execute_batch(&format!(
                            "CREATE INDEX temp.\"{}_keys\" ON \"{}\"({columns})",
                            stage.name, stage.name
                        ))?;
                    }
                }
                Action::Query(q) => {
                    let sql = &statement.queries[q];
                    let mut query = db.prepare(sql)?;
                    if !query.readonly() {
                        return Err(rusqlite::Error::InvalidQuery);
                    }
                    let count = query.parameter_count();
                    let mut rows = query.query(params_from_iter(&statement.parameters[..count]))?;
                    while let Some(row) = rows.next()? {
                        emit(
                            row.get::<_, u32>(0)? as usize,
                            row.get(1)?,
                            row.get::<_, u32>(2)? as usize,
                        );
                    }
                    drop(rows);
                    drop(query);
                }
                Action::Release(_) => {
                    let indices: Vec<_> = actions
                        .iter()
                        .map(|action| {
                            let Action::Release(i) = *action else {
                                unreachable!()
                            };
                            i
                        })
                        .collect();
                    let sql = statement
                        .ownership
                        .release(&relation_names, &indices, &populated);
                    db.execute_batch(&sql)?;
                    for action in actions {
                        let Action::Release(i) = *action else {
                            unreachable!()
                        };
                        live[i] = false;
                        live_count -= 1;
                    }
                }
            }
            debug_assert!(live_count <= statement.schedule.peak_relations);
        }
        debug_assert_eq!(live_count, 0);
        Ok(())
    })();
    for (i, stage) in statement.stages.iter().enumerate().rev() {
        if !live[i] || statement.ownership.slot(i).is_some() {
            continue;
        }
        // Preserve the original query error; the caller's transaction then rolls back.
        let cleanup = db
            .execute_batch(&format!("DROP TABLE IF EXISTS temp.\"{}\"", stage.name));
        if result.is_ok() {
            cleanup?;
        }
    }
    if owner_created {
        let cleanup = db.execute_batch(&format!(
            "DROP TABLE {}temp.\"{}\"",
            if result.is_ok() { "" } else { "IF EXISTS " },
            super::lower::ownership::STORE
        ));
        if result.is_ok() {
            cleanup?;
        }
    }
    result
}

