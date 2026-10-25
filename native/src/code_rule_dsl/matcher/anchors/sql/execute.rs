//! Physical SQL boundaries for shared candidate sets. Temporary storage can spill to disk.
use super::{
    lower::{Statement, lifetime::Action},
    store::Store,
};
#[cfg(feature = "code-rule-profile")]
use crate::code_rule_dsl::profile::trace::Span;
use rusqlite::{Result, params_from_iter};

#[cfg(test)]
pub(super) fn rows(
    store: &Store,
    statement: &Statement,
    mut emit: impl FnMut(usize, i64, usize),
) -> Result<()> {
    rows_for(store, statement, &statement.files, false, &mut emit)
}

pub(super) fn request(store: &Store, files: &[i64]) -> Result<()> {
    store.readers.clear();
    super::session::request(&store.db, files)?;
    #[cfg(feature = "code-rule-profile")]
    crate::code_rule_dsl::profile::record_anchor_statements(1 + files.len().div_ceil(128));
    Ok(())
}

pub(super) fn prepare_structural(store: &Store, files: &[i64]) -> Result<()> {
    #[cfg(feature = "code-rule-profile")]
    store.db.context(serde_json::json!({"component":"ast-index", "stage":"candidate-preparation", "producer":"decoded-structure", "fileDomain":files}));
    #[cfg(feature = "code-rule-profile")]
    let _timer = crate::code_rule_dsl::profile::PhaseTimer::start(
        crate::code_rule_dsl::profile::Phase::Selection,
    );
    #[cfg(feature = "code-rule-profile")]
    let mut span = Span::start("anchor-fact-preparation", "structural");
    #[cfg(feature = "code-rule-profile")]
    let before = store.readers.0.lock().counters;
    store.readers.clear();
    let transaction = if store.db.is_autocommit() {
        Some(store.db.unchecked_transaction()?)
    } else {
        None
    };
    let result = (|| {
        request(store, files)?;
        store.db.execute("DELETE FROM structural_demand", [])?;
        store.db.execute("INSERT INTO structural_demand SELECT q.fid,EXISTS(SELECT 1 FROM decoded_properties p WHERE p.fid=q.fid) FROM request_file q LEFT JOIN decoded_files d ON d.fid=q.fid WHERE d.fid IS NULL",[])?;
        // Fresh files need neither per-node cache probes nor a SELECT that reads its own
        // INSERT target. SQLite can stream these rows directly into the retained B-tree.
        store.db.execute("INSERT INTO decoded_properties SELECT f.fid,p.pre,p.parent,p.end,p.kind,p.field,p.flags FROM structural_demand d JOIN file f ON f.fid=d.fid CROSS JOIN nera_nodes(f.fid,0,f.nodes) p WHERE d.partial=0",[])?;
        store.readers.clear();
        store.db.execute("WITH needed(fid,pre) AS MATERIALIZED (SELECT d.fid,p.value FROM structural_demand d JOIN file f ON f.fid=d.fid CROSS JOIN generate_series(0,f.nodes-1,1) p LEFT JOIN decoded_properties c ON c.fid=d.fid AND c.pre=p.value WHERE d.partial=1 AND c.fid IS NULL) INSERT INTO decoded_properties SELECT s.fid,p.pre,p.parent,p.end,p.kind,p.field,p.flags FROM needed s CROSS JOIN nera_nodes(s.fid,s.pre,s.pre+1) p",[])?;
        store.readers.clear();
        store.db.execute(
            "INSERT OR IGNORE INTO decoded_files SELECT fid FROM request_file",
            [],
        )?;
        Ok(())
    })();
    store.readers.clear();
    if let Some(transaction) = transaction {
        if result.is_ok() {
            transaction.commit()?;
        } else {
            let _ = transaction.rollback();
        }
    }
    #[cfg(feature = "code-rule-profile")]
    {
        crate::code_rule_dsl::profile::record_anchor_statements(5);
        span.detail(||serde_json::json!({"files":files,"reader":store.readers.0.lock().counters.since(before),"failed":result.is_err()}));
    }
    result
}

pub(super) fn rows_for(
    store: &Store,
    statement: &Statement,
    files: &[i64],
    covered: bool,
    mut emit: impl FnMut(usize, i64, usize),
) -> Result<()> {
    #[cfg(feature = "code-rule-profile")]
    store.db.context(serde_json::json!({"component":"ast-index", "stage":"candidate-preparation", "producer":"anchor-frontier", "binding":statement.language, "fileDomain":files}));
    store.readers.clear();
    #[cfg(feature = "code-rule-profile")]
    let nodes = if crate::code_rule_dsl::profile::trace::enabled()
        || std::env::var_os("RERAIL_SQL_PROFILE_DIR").is_some()
    {
        store
            .graph
            .lock()
            .register(&statement.graph, &statement.parameters, statement.language)
    } else {
        std::collections::HashMap::new()
    };
    #[cfg(feature = "code-rule-profile")]
    super::capture::save(store, statement, &nodes, files)?;
    // Reuse an owning transaction (including snapshot leases); otherwise commit the batch once.
    let transaction = if store.db.is_autocommit() {
        Some(store.db.unchecked_transaction()?)
    } else {
        None
    };
    #[cfg(feature = "code-rule-profile")]
    let mut batch = Span::start("anchor-batch", "candidates");
    #[cfg(feature = "code-rule-profile")]
    let before = store.readers.0.lock().counters;
    #[cfg(feature = "code-rule-profile")]
    let mut emitted = 0u64;
    let relation_names: Vec<_> = statement.stages.iter().map(|s| s.name.as_str()).collect();
    let mut live = vec![false; statement.stages.len()];
    let mut populated = vec![false; statement.stages.len()];
    let mut retirement = super::lower::ownership::Retirement::default();
    let mut live_count = 0usize;
    let mut owner_created = false;
    #[cfg(feature = "code-rule-profile")]
    let mut next_sample = 128;
    let result = (|| {
        if statement.file_scope {
            request(store, files)?;
        }
        for sql in &statement.ownership.setup {
            store.db.execute_batch(sql)?;
            owner_created = true;
        }
        #[cfg(feature = "code-rule-profile")]
        working_set(store, "start", 0, live_count)?;
        #[cfg(feature = "code-rule-profile")]
        let mut operation = 0;
        for actions in statement
            .schedule
            .actions
            .chunk_by(|a, b| matches!(a, Action::Release(_)) && matches!(b, Action::Release(_)))
        {
            #[cfg(feature = "code-rule-profile")]
            {
                operation += actions.len();
            }
            match actions[0] {
                Action::Prepare(i) => {
                    let stage = &statement.stages[i];
                    if covered && stage.covering {
                        continue;
                    }
                    // Incremental BLOB handles count as active SQLite statements. Close them before
                    // schema changes, even when the changed table is in the temporary database.
                    store.readers.clear();
                    #[cfg(feature = "code-rule-profile")]
                    let mut prepare = Span::start("anchor-prepare", &stage.name);
                    if let Some(sql) = retirement.before_prepare() {
                        let deleted = store.db.execute(&sql, [])?;
                        retirement.retired(deleted);
                    }
                    let mut create = store.db.prepare(&stage.sql)?;
                    #[cfg(feature = "code-rule-profile")]
                    {
                        prepare.detail(|| serde_json::json!({"sqlBytes":stage.sql.len()}));
                        drop(prepare);
                    }
                    let count = create.parameter_count();
                    #[cfg(feature = "code-rule-profile")]
                    let mut run = Span::start("anchor-materialize", &stage.name);
                    #[cfg(feature = "code-rule-profile")]
                    let logical = nodes.get(&stage.name);
                    #[cfg(feature = "code-rule-profile")]
                    let previous = logical.map(|node| store.graph.lock().constructed(node.id));
                    let inserted =
                        create.execute(params_from_iter(&statement.parameters[..count]))?;
                    if stage.temporary {
                        if statement.ownership.slot(i).is_some() {
                            retirement.inserted(inserted);
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
                    #[cfg(feature = "code-rule-profile")]
                    {
                        run.detail(|| {
                    serde_json::json!({
                        "vmSteps":create.get_status(rusqlite::StatementStatus::VmStep),
                        "sorts":create.get_status(rusqlite::StatementStatus::Sort),
                        "fullscanSteps":create.get_status(rusqlite::StatementStatus::FullscanStep),
                        "relationId":logical.map(|node|node.id),
                        "dependencyClass":logical.map(|node|format!("{:?}",node.class)),
                        "priorConstructions":previous,
                    })
                });
                        drop(run);
                    }
                    drop(create);
                    store.readers.clear();
                    if let Some(columns) = stage.index {
                        #[cfg(feature = "code-rule-profile")]
                        let _index = Span::start("anchor-index", &stage.name);
                        store.db.execute_batch(&format!(
                            "CREATE INDEX temp.\"{}_keys\" ON \"{}\"({columns})",
                            stage.name, stage.name
                        ))?;
                    }
                }
                Action::Query(q) => {
                    let sql = &statement.queries[q];
                    #[cfg(feature = "code-rule-profile")]
                    let before_query_rows = emitted;
                    #[cfg(feature = "code-rule-profile")]
                    let mut prepare = Span::start("anchor-prepare", "results");
                    let mut query = store.db.prepare(sql)?;
                    if !query.readonly() {
                        return Err(rusqlite::Error::InvalidQuery);
                    }
                    #[cfg(feature = "code-rule-profile")]
                    {
                        prepare.detail(|| serde_json::json!({"sqlBytes":sql.len()}));
                        drop(prepare);
                    }
                    let count = query.parameter_count();
                    #[cfg(feature = "code-rule-profile")]
                    let mut run = Span::start("anchor-results", "results");
                    let mut rows = query.query(params_from_iter(&statement.parameters[..count]))?;
                    while let Some(row) = rows.next()? {
                        #[cfg(feature = "code-rule-profile")]
                        {
                            emitted += 1;
                        }
                        emit(
                            row.get::<_, u32>(0)? as usize,
                            row.get(1)?,
                            row.get::<_, u32>(2)? as usize,
                        );
                    }
                    drop(rows);
                    #[cfg(feature = "code-rule-profile")]
                    {
                        run.detail(|| {
                    serde_json::json!({
                        "vmSteps":query.get_status(rusqlite::StatementStatus::VmStep),
                        "sorts":query.get_status(rusqlite::StatementStatus::Sort),
                        "fullscanSteps":query.get_status(rusqlite::StatementStatus::FullscanStep),
                        "emittedRows":emitted-before_query_rows,
                    })
                });
                        drop(run);
                    }
                    drop(query);
                    #[cfg(feature = "code-rule-profile")]
                    working_set(store, "results", operation, live_count)?;
                }
                Action::Release(_) => {
                    store.readers.clear();
                    #[cfg(feature = "code-rule-profile")]
                    {
                        let _explain = Span::start("anchor-explain", "release");
                        store.db.explain_pending();
                    }
                    #[cfg(feature = "code-rule-profile")]
                    let mut release = Span::start("anchor-release", "batch");
                    let indices: Vec<_> = actions
                        .iter()
                        .map(|action| {
                            let Action::Release(i) = *action else {
                                unreachable!()
                            };
                            i
                        })
                        .collect();
                    let sql = retirement.release(
                        &statement.ownership,
                        &relation_names,
                        &indices,
                        &populated,
                    );
                    store.db.execute_batch(&sql)?;
                    for action in actions {
                        let Action::Release(i) = *action else {
                            unreachable!()
                        };
                        live[i] = false;
                        live_count -= 1;
                    }
                    #[cfg(feature = "code-rule-profile")]
                release.detail(||serde_json::json!({"relations":actions.len(),"liveRelations":live_count}));
                }
            }
            debug_assert!(live_count <= statement.schedule.peak_relations);
            #[cfg(feature = "code-rule-profile")]
            if operation >= next_sample {
                working_set(store, "progress", operation, live_count)?;
                next_sample = (operation / 128 + 1) * 128;
            }
        }
        debug_assert_eq!(live_count, 0);
        #[cfg(feature = "code-rule-profile")]
        working_set(store, "finished", operation, live_count)?;
        store.readers.clear();
        #[cfg(feature = "code-rule-profile")]
        if crate::code_rule_dsl::profile::trace::enabled() {
            let mut storage = Span::start("anchor-storage", "workspace");
            let pages=store.db.query_row("SELECT (SELECT page_count FROM pragma_page_count),(SELECT freelist_count FROM pragma_freelist_count),(SELECT page_size FROM pragma_page_size),(SELECT page_count FROM pragma_page_count('temp'))",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?)))?;
            let path = store.db.path().unwrap_or_default();
            let wal = std::fs::metadata(format!("{path}-wal"))
                .map(|m| m.len())
                .unwrap_or(0);
            let resident = store.db.resident_bytes()?;
            storage.detail(||serde_json::json!({"mainReservedBytes":pages.0*pages.2,"mainFreeBytes":pages.1*pages.2,"tempReservedBytes":pages.3*pages.2,"walBytes":wal,"sqliteResidentBytes":resident}));
            drop(storage);
        }
        // Explain SELECT bodies while their materialized inputs still exist.
        #[cfg(feature = "code-rule-profile")]
        {
            let _explain = Span::start("anchor-explain", "checkpoint");
            store.db.checkpoint()?;
        }
        Ok(())
    })();
    store.readers.clear();
    for (i, stage) in statement.stages.iter().enumerate().rev() {
        if !live[i] || statement.ownership.slot(i).is_some() {
            continue;
        }
        #[cfg(feature = "code-rule-profile")]
        let _cleanup = Span::start("anchor-release", &stage.name);
        // Preserve the original query error. The owning temporary database is discarded on drop.
        let cleanup = store
            .db
            .execute_batch(&format!("DROP TABLE IF EXISTS temp.\"{}\"", stage.name));
        if result.is_ok() {
            cleanup?;
        }
    }
    if owner_created {
        let cleanup = store.db.execute_batch(&format!(
            "DROP TABLE {}temp.\"{}\"",
            if result.is_ok() { "" } else { "IF EXISTS " },
            super::lower::ownership::STORE
        ));
        if result.is_ok() {
            cleanup?;
        }
    }
    if statement.source_batches {
        let cleanup = store.db.execute_batch(super::source_batch::CLEAR);
        if result.is_ok() {
            cleanup?;
        }
    }
    #[cfg(feature = "code-rule-profile")]
    batch.detail(|| serde_json::json!({"files":files,"reader":store.readers.0.lock().counters.since(before),"readerColumns":["rec","parentOverflow","sizeOverflow","flags","spans","checkpoints","source"],"emittedRows":emitted,"peakLiveRelations":statement.schedule.peak_relations,"materializations":statement.preparations(covered),"failed":result.is_err()}));
    if let Some(transaction) = transaction {
        if result.is_ok() {
            transaction.commit()?;
        } else {
            let _ = transaction.rollback();
        }
    }
    result
}

#[cfg(feature = "code-rule-profile")]
fn working_set(store: &Store, name: &str, operation: usize, live: usize) -> Result<()> {
    if crate::code_rule_dsl::profile::trace::enabled() {
        let mut span = Span::start("anchor-working-set", name);
        let mut sample = super::lower::working_set::snapshot(&store.db, live)?;
        sample["operation"] = operation.into();
        span.detail(|| sample);
    }
    Ok(())
}
