//! Tuple maintenance over retained checked queries. Positive recursion uses overdelete/rederive.
use super::super::logic::sql::select::{FromItem, Join, Query, Relation as SqlRelation};
use super::super::logic::sql::sql_ast::{SqlBinaryOperator as B, SqlExpression as E};
use super::*;

#[derive(Clone)]
pub(super) struct Delta {
    pub plus: String,
    pub minus: String,
    pub inserted: u64,
    pub deleted: u64,
}
pub(super) use super::scratch::Work;
impl Work<'_> {
    pub fn table(&mut self, width: usize) -> Result<String> {
        let cols = (0..width).map(|i| format!("c{i}")).collect::<Vec<_>>();
        self.allocate(
            format!(
                "{},PRIMARY KEY({})",
                cols.iter()
                    .map(|c| format!("{c} INTEGER NOT NULL"))
                    .collect::<Vec<_>>()
                    .join(","),
                cols.join(",")
            ),
            true,
        )
    }
    pub fn rows(&mut self, width: usize, rows: &[Vec<i64>]) -> Result<String> {
        let name = self.table(width)?;
        plan::insert_many(
            self.db,
            &format!("INSERT OR IGNORE INTO {}", quote(&name)),
            rows.iter()
                .map(|r| r.iter().copied().map(SqlValue::Integer).collect())
                .collect(),
        )
        .map_err(error)?;
        Ok(name)
    }
    pub fn difference(&mut self, now: &str, old: &str, width: usize) -> Result<Delta> {
        let plus = self.table(width)?;
        let minus = self.table(width)?;
        let inserted = self
            .db
            .execute(
                &format!(
                    "INSERT INTO {plus} SELECT * FROM {} EXCEPT SELECT * FROM {}",
                    quote(now),
                    quote(old)
                ),
                [],
            )
            .map_err(error)? as u64;
        let deleted = self
            .db
            .execute(
                &format!(
                    "INSERT INTO {minus} SELECT * FROM {} EXCEPT SELECT * FROM {}",
                    quote(old),
                    quote(now)
                ),
                [],
            )
            .map_err(error)? as u64;
        Ok(Delta {
            plus,
            minus,
            inserted,
            deleted,
        })
    }
    pub fn snapshot(&mut self, rel: &str, width: usize) -> Result<(String, u64)> {
        let old = self.table(width)?;
        let n = self
            .db
            .execute(
                &format!("INSERT OR IGNORE INTO {old} SELECT * FROM {}", quote(rel)),
                [],
            )
            .map_err(error)? as u64;
        Ok((old, n))
    }
}

pub(super) fn track_views(w: &mut Work<'_>, run: &mut session::Execution) -> Result<usize> {
    let e = Arc::make_mut(&mut run.emitted);
    let views = std::mem::take(&mut e.views);
    let mut names = BTreeSet::new();
    for (name, q) in &views {
        let width = q.body.columns.len();
        let (old, count) = w.snapshot(name, width)?;
        w.db.execute_batch(&format!("DROP VIEW {}", quote(name)))
            .map_err(error)?;
        let cols = (0..width).map(|i| format!("c{i}")).collect::<Vec<_>>();
        for table in [name.clone(), format!("new_{name}"), format!("delta_{name}")] {
            w.db.execute_batch(&format!(
                "CREATE TEMP TABLE {}({},PRIMARY KEY({})) WITHOUT ROWID",
                quote(&table),
                cols.iter()
                    .map(|c| format!("{c} INTEGER"))
                    .collect::<Vec<_>>()
                    .join(","),
                cols.join(",")
            ))
            .map_err(error)?;
            if table != *name {
                run.objects.push((table, true));
            }
        }
        w.db.execute_batch(&format!("INSERT INTO {} SELECT * FROM {old}", quote(name)))
            .map_err(error)?;
        if let Some((_, table)) = run.objects.iter_mut().find(|(n, _)| n == name) {
            *table = true;
        }
        run.counts.insert(name.clone(), count);
        e.program.relations.push((name.clone(), width));
        e.program.steps.push(schedule::Step {
            output: name.clone(),
            stratum: 0,
            reads: vec![],
            sql: format!("INSERT OR IGNORE INTO new_{name} {}", q.render()),
        });
        e.queries.push(Some(q.clone()));
        names.insert(name.clone());
    }
    for (step, q) in e.program.steps.iter_mut().zip(&e.queries) {
        if let Some(q) = q {
            for r in q.reads() {
                if let Some(r) = r.program_relation()
                    && names.contains(r)
                    && !step.reads.iter().any(|name| name == r)
                {
                    step.reads.push(r.into());
                }
            }
        }
    }
    for (table, cols) in indexes::wanted(e) {
        session::index(w.db, &table, &cols).map_err(error)?;
    }
    Ok(views.len())
}

pub(super) fn barrier(q: &Query) -> bool {
    if !q.ctes.is_empty() {
        return true;
    }
    let mut result = false;
    for e in q
        .body
        .columns
        .iter()
        .map(|(e, _)| e)
        .chain(&q.body.predicates)
        .chain(q.body.from.iter().flat_map(|item| match &item.join {
            Join::On(on) => on.iter(),
            Join::Cross => [].iter(),
        }))
    {
        e.visit(&mut |e| {
            if matches!(e, E::Exists { .. } | E::Scalar(_)) {
                result = true;
            }
        });
    }
    result
}
fn atoms(q: &Query) -> Vec<(usize, String)> {
    q.body
        .from
        .iter()
        .enumerate()
        .filter_map(|(i, f)| f.relation.program_relation().map(|r| (i, r.to_owned())))
        .collect()
}
fn source(table: &str, width: usize) -> SqlRelation {
    SqlRelation::Batch {
        rel: table.into(),
        source: select::Source::positional(
            table,
            &(0..width)
                .map(|i| format!("c{i}"))
                .collect::<Vec<_>>()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        ),
    }
}
fn same(a: &str, b: &str, width: usize) -> String {
    (0..width)
        .map(|i| format!("{a}.c{i}={b}.c{i}"))
        .collect::<Vec<_>>()
        .join(" AND ")
}
fn insert_query(db: &rusqlite::Connection, table: &str, q: &Query) -> Result<usize> {
    db.execute(
        &format!("INSERT OR IGNORE INTO {} {}", quote(table), q.render()),
        [],
    )
    .map_err(error)
}
pub(super) fn anchor(q: &mut Query, at: usize) {
    let mut delta = q.body.from.remove(at);
    if let Join::On(conditions) = std::mem::replace(&mut delta.join, Join::Cross) {
        q.body.predicates.extend(conditions);
    }
    for item in &mut q.body.from {
        if let Join::On(conditions) = std::mem::replace(&mut item.join, Join::Cross) {
            q.body.predicates.extend(conditions);
        }
    }
    // A changed document can bind a native node before it binds a retained walk. Keep the
    // small delta outermost, then prefer joins connected to aliases already bound. Keeping
    // the original tail order can scan every walk row before testing the document.
    let mut bound = BTreeSet::from([delta.alias.clone()]);
    let mut ordered = vec![delta];
    while !q.body.from.is_empty() {
        let score = |alias: &str| {
            q.body
                .predicates
                .iter()
                .filter(|e| {
                    let E::Binary {
                        operator: B::Equal,
                        left,
                        right,
                    } = e
                    else {
                        return false;
                    };
                    let mut aliases = BTreeSet::new();
                    for expr in [left, right] {
                        expr.visit(&mut |e| {
                            if let E::Qualified { alias, .. } = e {
                                aliases.insert(alias.clone());
                            }
                        });
                    }
                    aliases.contains(alias)
                        && aliases.iter().any(|a| bound.contains(a))
                        && aliases.iter().all(|a| a == alias || bound.contains(a))
                })
                .count()
        };
        let at = q
            .body
            .from
            .iter()
            .enumerate()
            .max_by_key(|(i, f)| (score(&f.alias), std::cmp::Reverse(*i)))
            .unwrap()
            .0;
        let next = q.body.from.remove(at);
        bound.insert(next.alias.clone());
        ordered.push(next);
    }
    q.body.from = ordered;
}
fn targeted(mut q: Query, table: &str) -> Query {
    let alias = "rr_update_target";
    for (i, (expr, _)) in q.body.columns.clone().iter().enumerate() {
        q.body.predicates.push(E::Binary {
            operator: B::Equal,
            left: Box::new(expr.clone()),
            right: Box::new(E::Qualified {
                alias: alias.into(),
                column: format!("c{i}"),
            }),
        });
    }
    let columns = (0..q.body.columns.len())
        .map(|i| {
            (
                E::Qualified {
                    alias: alias.into(),
                    column: format!("c{i}"),
                },
                Some(format!("c{i}")),
            )
        })
        .collect();
    q.body.columns = vec![(
        E::Value(super::super::logic::sql::sql_ast::SqlValue::Integer(1)),
        None,
    )];
    q.body.distinct = false;
    for item in &mut q.body.from {
        if item.join == Join::Cross {
            item.join = Join::On(vec![]);
        }
    }
    // A support probe stops at the first surviving derivation. Its correlated head values
    // let SQLite choose indexed body probes without moving the small target set inward.
    Query {
        ctes: vec![],
        body: select::Select {
            distinct: false,
            columns,
            from: vec![FromItem::cross(
                SqlRelation::Bound {
                    rel: table.into(),
                    table: table.into(),
                },
                alias,
            )],
            predicates: vec![E::Exists {
                negated: false,
                query: Box::new(q.body),
            }],
        },
    }
}

pub(super) fn maintain(
    w: &mut Work<'_>,
    run: &mut session::Execution,
    changes: &mut BTreeMap<String, Delta>,
    limit: u32,
) -> Result<Value> {
    let e = Arc::clone(&run.emitted);
    let arities: BTreeMap<_, _> = e.program.relations.iter().cloned().collect();
    let mut positions = BTreeMap::<String, u32>::new();
    for s in &e.program.steps {
        positions
            .entry(s.output.clone())
            .and_modify(|n| *n = (*n).max(s.stratum))
            .or_insert(s.stratum);
    }
    let mut groups = BTreeMap::<(u32, String), BTreeSet<String>>::new();
    for (r, n) in positions {
        groups
            .entry((n, if n == 0 { r.clone() } else { String::new() }))
            .or_default()
            .insert(r);
    }
    let (mut positives, mut deletions) = (0u64, 0u64);
    let mut refreshed = Vec::new();
    let mut incremental = Vec::new();
    let mut recursive_partitions = 0u64;
    let mut recursive_frames = 0u64;
    let (mut reducer_groups, mut context_rows) = (0u64, 0u64);
    let mut outputs = BTreeMap::new();
    for (_, group) in groups {
        // Only seed/full statements: recursive delta variants are generated from these for edits.
        let steps: Vec<_> = e
            .program
            .steps
            .iter()
            .zip(&e.queries)
            .filter(|(s, q)| {
                group.contains(&s.output)
                    && q.as_ref().is_none_or(|q| {
                        !q.reads()
                            .iter()
                            .any(|r| matches!(r, SqlRelation::Derived { delta: true, .. }))
                    })
            })
            .collect();
        let affected = steps
            .iter()
            .filter_map(|(_, q)| q.as_ref())
            .flat_map(atoms)
            .any(|(_, r)| {
                !group.contains(&r) && changes.get(&r).is_some_and(|d| d.inserted + d.deleted > 0)
            });
        // Walks/aggregates may read their dependencies inside subqueries or CTEs.
        let indirect = steps.iter().filter_map(|(_, q)| q.as_ref()).any(|q| {
            q.reads()
                .iter()
                .filter_map(|r| r.program_relation())
                .any(|r| {
                    !group.contains(r) && changes.get(r).is_some_and(|d| d.inserted + d.deleted > 0)
                })
        });
        if !affected && !indirect {
            continue;
        }
        let nonmonotone = group
            .iter()
            .any(|r| e.program.lattices.iter().any(|(n, _, _)| n == r))
            || steps.iter().filter_map(|(_, q)| q.as_ref()).any(barrier);
        if group.len() == 1
            && let Some(r) = group.first()
            && let Some(state) = run
                .reductions
                .as_ref()
                .and_then(|states| states.get(r))
                .filter(|state| state.ready)
        {
            let (d, contexts, groups) = state.apply(w, changes)?;
            context_rows += contexts;
            reducer_groups += groups;
            let count = run.counts.get(r).copied().unwrap_or(0) + d.inserted - d.deleted;
            run.counts.insert(r.clone(), count);
            if run.outputs.contains(r) {
                outputs.insert(
                    r.clone(),
                    json!({"inserted":d.inserted,"deleted":d.deleted}),
                );
            }
            changes.insert(r.clone(), d);
            incremental.push(r.clone());
            continue;
        }
        if nonmonotone {
            let stratum = e.program.steps.iter().filter(|s|group.contains(&s.output)).map(|s|s.stratum).max().unwrap_or(0);
            if let Some(state) = run.temporal.get_mut(&stratum) {
                let store = w.versions.ok_or_else(||error("temporal execution requires versioned storage"))?;
                let (deltas,waves) = state.apply(w,store,changes,limit)?;
                positives += u64::from(waves);
                recursive_frames += u64::from(waves);
                let objects = state.objects().collect::<Vec<_>>();
                let owned = run.objects.iter().map(|(n,_)|n.clone()).collect::<BTreeSet<_>>();
                run.objects.extend(objects.into_iter().filter(|(n,_)|!owned.contains(n)));
                for (r,d) in deltas {
                    let count = run.counts.get(&r).copied().unwrap_or(0)+d.inserted-d.deleted;
                    run.counts.insert(r.clone(),count);
                    if run.outputs.contains(&r) {outputs.insert(r.clone(),json!({"inserted":d.inserted,"deleted":d.deleted}));}
                    incremental.push(r.clone());
                    changes.insert(r,d);
                }
                continue;
            }
            if let Some(state) = group
                .first()
                .and_then(|r| {
                    run.emitted
                        .program
                        .steps
                        .iter()
                        .filter(|s| s.output == *r)
                        .map(|s| s.stratum)
                        .max()
                })
                .and_then(|s| run.recursive.get(&s))
            {
                let (deltas, partitions, waves) = state.apply(w, run, changes, limit)?;
                positives += waves;
                recursive_partitions += partitions;
                for (r, d) in deltas {
                    let count = run.counts.get(&r).copied().unwrap_or(0) + d.inserted - d.deleted;
                    run.counts.insert(r.clone(), count);
                    if run.outputs.contains(&r) {
                        outputs.insert(
                            r.clone(),
                            json!({"inserted":d.inserted,"deleted":d.deleted}),
                        );
                    }
                    incremental.push(r.clone());
                    changes.insert(r, d);
                }
                continue;
            }
            let mut old = BTreeMap::new();
            for r in &group {
                let snapshot = w.snapshot(r, arities[r])?;
                old.insert(r.clone(), snapshot);
                w.db.execute_batch(&format!("DELETE FROM {}", quote(r)))
                    .map_err(error)?;
                if e.program
                    .lattice_operations
                    .get(r)
                    .is_some_and(|o| o.after.is_some())
                {
                    w.db.execute_batch(&format!("DELETE FROM __sql_changes_{r}"))
                        .map_err(error)?;
                }
            }
            let mut p = e.program.clone();
            p.relations.retain(|(r, _)| group.contains(r));
            p.steps.retain(|s| group.contains(&s.output));
            p.lattices.retain(|(r, _, _)| group.contains(r));
            p.lattice_operations.retain(|r, _| group.contains(r));
            let waves = schedule::execute_existing(w.db, &p, limit).map_err(error)?;
            positives += u64::from(waves);
            refreshed.push(group.iter().cloned().collect::<Vec<_>>());
            for r in &group {
                let d = w.difference(r, &old[r].0, arities[r])?;
                let count = old[r].1 + d.inserted - d.deleted;
                run.counts.insert(r.clone(), count);
                if run.outputs.contains(r) {
                    outputs.insert(
                        r.clone(),
                        json!({"inserted":d.inserted,"deleted":d.deleted}),
                    );
                }
                changes.insert(r.clone(), d);
            }
            continue;
        }
        // Native inputs keep their indexed overlays. SQL results temporarily restore their
        // old membership inside the batch savepoint, using only row deltas and existing indexes.
        // This costs only the deletion set, without distributing joins over 2^N alternatives.
        let mut overlays = overlay::Overlays::new(w.db, w.profiler);
        let mut old_inputs = BTreeMap::new();
        let mut sql_deleted = BTreeMap::new();
        for (_, q) in &steps {
            if let Some(q) = q {
                for (_, r) in atoms(q) {
                    if !group.contains(&r)
                        && changes.get(&r).is_some_and(|d| d.deleted > 0)
                        && !old_inputs.contains_key(&r)
                        && !sql_deleted.contains_key(&r)
                    {
                        let d = &changes[&r];
                        if let Some(binding) = overlays.bind(&r, &d.minus)? {
                            old_inputs.insert(r, binding);
                        } else {
                            let width =
                                w.db.prepare(&format!("SELECT * FROM {} LIMIT 0", quote(&r)))
                                    .map_err(error)?
                                    .column_count();
                            sql_deleted.insert(r, (d.minus.clone(), width));
                        }
                    }
                }
            }
        }
        for (r, (minus, width)) in &sql_deleted {
            // A lattice key can have different old and new values. Remove insertions first
            // so the old value fits its unique key. Never materialize the complete relation.
            if changes[r].inserted > 0 {
                let cols = (0..*width)
                    .map(|i| format!("c{i}"))
                    .collect::<Vec<_>>()
                    .join(",");
                w.db.execute(
                    &format!(
                        "DELETE FROM {} WHERE ({cols}) IN (SELECT {cols} FROM {})",
                        quote(r),
                        quote(&changes[r].plus)
                    ),
                    [],
                )
                .map_err(error)?;
            }
            w.db.execute(
                &format!(
                    "INSERT OR IGNORE INTO {} SELECT * FROM {}",
                    quote(r),
                    quote(minus)
                ),
                [],
            )
            .map_err(error)?;
        }
        let mut over = BTreeMap::new();
        let mut front = BTreeMap::new();
        let mut candidates = BTreeMap::new();
        let mut journals = BTreeMap::new();
        let mut front_counts = BTreeMap::new();
        for r in &group {
            over.insert(r.clone(), w.table(arities[r])?);
            front.insert(r.clone(), w.table(arities[r])?);
            candidates.insert(r.clone(), w.table(arities[r])?);
            journals.insert(r.clone(), w.table(arities[r])?);
            front_counts.insert(r.clone(), 0usize);
        }
        let mut round = 0;
        loop {
            if round >= limit {
                return Err(error("overdelete wave limit exceeded"));
            }
            let mut added = 0;
            for (s, q) in &steps {
                let Some(q) = q else { continue };
                for (i, r) in atoms(q) {
                    let delta = if group.contains(&r) {
                        if front_counts[&r] > 0 {
                            Some(front[&r].as_str())
                        } else {
                            None
                        }
                    } else if round == 0 {
                        changes
                            .get(&r)
                            .filter(|d| d.deleted > 0)
                            .map(|d| d.minus.as_str())
                    } else {
                        None
                    };
                    let Some(delta) = delta else { continue };
                    let mut variant = q.clone();
                    for (j, rel) in atoms(q) {
                        if let Some((table, width)) = old_inputs.get(&rel) {
                            variant.body.from[j].relation = source(table, *width);
                        }
                    }
                    let width =
                        w.db.prepare(&format!("SELECT * FROM {delta} LIMIT 0"))
                            .map_err(error)?
                            .column_count();
                    variant.body.from[i].relation = source(delta, width);
                    anchor(&mut variant, i);
                    w.db.execute(&format!("INSERT OR IGNORE INTO {} SELECT n.* FROM ({}) n WHERE EXISTS(SELECT 1 FROM {} r WHERE {})",candidates[&s.output],variant.render(),quote(&s.output),same("r","n",arities[&s.output])),[]).map_err(error)?;
                }
            }
            for r in &group {
                w.db.execute_batch(&format!("DELETE FROM {}", front[r]))
                    .map_err(error)?;
                let n=w.db.execute(&format!("INSERT INTO {} SELECT n.* FROM {} n WHERE NOT EXISTS(SELECT 1 FROM {} r WHERE {})",front[r],candidates[r],over[r],same("r","n",arities[r])),[]).map_err(error)?;
                w.db.execute_batch(&format!(
                    "INSERT INTO {} SELECT * FROM {}; DELETE FROM {}",
                    over[r], front[r], candidates[r]
                ))
                .map_err(error)?;
                front_counts.insert(r.clone(), n);
                added += n;
            }
            round += 1;
            if added == 0 {
                break;
            }
            deletions += 1;
        }
        // Restore current membership before rederivation or downstream work. Any error above
        // rolls back the whole savepoint, including temporarily removed insertions.
        for (r, (minus, width)) in &sql_deleted {
            let cols = (0..*width)
                .map(|i| format!("c{i}"))
                .collect::<Vec<_>>()
                .join(",");
            w.db.execute(
                &format!(
                    "DELETE FROM {} WHERE ({cols}) IN (SELECT {cols} FROM {})",
                    quote(r),
                    quote(minus)
                ),
                [],
            )
            .map_err(error)?;
            if changes[r].inserted > 0 {
                w.db.execute(
                    &format!(
                        "INSERT OR IGNORE INTO {} SELECT * FROM {}",
                        quote(r),
                        quote(&changes[r].plus)
                    ),
                    [],
                )
                .map_err(error)?;
            }
        }
        drop(overlays);
        let mut removed = BTreeMap::new();
        let mut added = BTreeMap::new();
        for r in &group {
            let columns = (0..arities[r])
                .map(|i| format!("c{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let n =
                w.db.execute(
                    &format!(
                        "DELETE FROM {} WHERE ({columns}) IN (SELECT {columns} FROM {})",
                        quote(r),
                        over[r]
                    ),
                    [],
                )
                .map_err(error)? as u64;
            removed.insert(r.clone(), n);
            added.insert(r.clone(), 0u64);
            w.db.execute_batch(&format!("DELETE FROM {}", front[r]))
                .map_err(error)?;
            front_counts.insert(r.clone(), 0);
        }
        let mut round = 0;
        loop {
            if round >= limit {
                return Err(error("insertion wave limit exceeded"));
            }
            for (s, q) in &steps {
                if let Some(q) = q {
                    if round == 0 {
                        insert_query(
                            w.db,
                            &candidates[&s.output],
                            &targeted(q.clone(), &over[&s.output]),
                        )?;
                    }
                    for (i, r) in atoms(q) {
                        let delta = if group.contains(&r) {
                            if front_counts[&r] > 0 {
                                Some(front[&r].as_str())
                            } else {
                                None
                            }
                        } else if round == 0 {
                            changes
                                .get(&r)
                                .filter(|d| d.inserted > 0)
                                .map(|d| d.plus.as_str())
                        } else {
                            None
                        };
                        let Some(delta) = delta else { continue };
                        let mut variant = q.clone();
                        let width =
                            w.db.prepare(&format!("SELECT * FROM {delta} LIMIT 0"))
                                .map_err(error)?
                                .column_count();
                        variant.body.from[i].relation = source(delta, width);
                        anchor(&mut variant, i);
                        insert_query(w.db, &candidates[&s.output], &variant)?;
                    }
                } else if round == 0 && removed[&s.output] > 0 {
                    w.db.execute_batch(&s.sql).map_err(error)?;
                    w.db.execute_batch(&format!(
                        "INSERT OR IGNORE INTO {} SELECT * FROM new_{}; DELETE FROM new_{}",
                        candidates[&s.output], s.output, s.output
                    ))
                    .map_err(error)?;
                }
            }
            let mut grew = 0;
            for r in &group {
                let n = schedule::sets::fold(w.db, r, arities[r], &candidates[r], &front[r])
                    .map_err(error)?;
                w.db.execute_batch(&format!(
                    "INSERT OR IGNORE INTO {} SELECT * FROM {}",
                    journals[r], front[r]
                ))
                .map_err(error)?;
                front_counts.insert(r.clone(), n);
                *added.get_mut(r).unwrap() += n as u64;
                grew += n;
            }
            round += 1;
            positives += 1;
            if grew == 0 {
                break;
            }
        }
        for r in &group {
            let minus = w.table(arities[r])?;
            let plus = w.table(arities[r])?;
            let deleted=w.db.execute(&format!("INSERT INTO {minus} SELECT n.* FROM {} n WHERE NOT EXISTS(SELECT 1 FROM {} r WHERE {})",over[r],quote(r),same("r","n",arities[r])),[]).map_err(error)? as u64;
            // Every new row outside the overdeleted set is an insertion. Candidate additions
            // are retained in a journal, rather than comparing whole result tables.
            let count = run.counts.get(r).copied().unwrap_or(0) + added[r] - removed[r];
            run.counts.insert(r.clone(), count);
            let inserted =
                w.db.execute(
                    &format!(
                        "INSERT INTO {plus} SELECT * FROM {} EXCEPT SELECT * FROM {}",
                        journals[r], over[r]
                    ),
                    [],
                )
                .map_err(error)? as u64;
            if run.outputs.contains(r) {
                outputs.insert(r.clone(), json!({"inserted":inserted,"deleted":deleted}));
            }
            changes.insert(
                r.clone(),
                Delta {
                    plus,
                    minus,
                    inserted,
                    deleted,
                },
            );
        }
    }
    Ok(
        json!({"positive_waves":positives,"overdelete_waves":deletions,"refreshed_strata":refreshed,"incremental_strata":incremental,"recursive_partitions":recursive_partitions,"recursive_frames":recursive_frames,"reducer_groups":reducer_groups,"context_rows":context_rows,"outputs":outputs}),
    )
}
