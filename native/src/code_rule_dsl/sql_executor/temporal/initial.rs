//! One global wave schedule constructs the retained state and produces the initial answer.
use super::*;

pub(in crate::code_rule_dsl::sql_executor) fn run(
    w: &mut Work<'_>,
    store: &versions::Store,
    run: &mut session::Execution,
    namespace: usize,
    original: &schedule::Program,
) -> Result<Option<u32>> {
    let e = run.emitted.clone();
    let outputs = original
        .relations
        .iter()
        .map(|(r, _)| r.clone())
        .collect::<BTreeSet<_>>();
    let positions = original
        .steps
        .iter()
        .map(|s| (s.output.clone(), s.stratum))
        .fold(BTreeMap::new(), |mut m, (r, n)| {
            m.entry(r)
                .and_modify(|v: &mut u32| *v = (*v).max(n))
                .or_insert(n);
            m
        });
    let eligible = original.steps.iter().zip(&e.queries).any(|(s, q)| {
        s.stratum > 0
            && q.as_ref().is_some_and(|q| {
                q.reads()
                    .iter()
                    .filter_map(|r| r.program_relation())
                    .any(|r| positions.get(r) == Some(&s.stratum))
            })
            && (original
                .lattices
                .iter()
                .any(|(r, _, _)| positions.get(r) == Some(&s.stratum))
                || q.as_ref().is_some_and(maintenance::barrier))
    });
    if !eligible {
        return Ok(None);
    }
    let steps = e
        .program
        .steps
        .iter()
        .zip(&e.queries)
        .filter(|(s, _)| outputs.contains(&s.output))
        .map(|(s, q)| {
            Ok((
                s.clone(),
                q.clone()
                    .ok_or_else(|| error("unchecked initial constant"))?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let probes = indexes::native(steps.iter().map(|(_, q)| q));
    let relations = original
        .relations
        .iter()
        .map(|(r, width)| {
            let lattice = original.lattices.iter().find(|(n, _, _)| n == r);
            (
                r.clone(),
                Relation {
                    indexes: probes
                        .get(r)
                        .into_iter()
                        .flatten()
                        .chain(probes.get(&format!("delta_{r}")).into_iter().flatten())
                        .copied()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect(),
                    width: *width,
                    keys: lattice.map_or(*width, |(_, k, _)| *k),
                    lattice: lattice.is_some(),
                    top: lattice.map_or(0, |(_, _, t)| *t),
                    ops: original.lattice_operations.get(r).cloned(),
                },
            )
        })
        .collect();
    let mut after = BTreeSet::new();
    for (s, _) in &steps {
        for (p, _) in &steps {
            if s.stratum != p.stratum && s.reads.contains(&p.output) {
                after.insert((s.stratum, p.stratum));
            }
        }
    }
    loop {
        let n = after.len();
        for (a, b) in after.clone() {
            for (c, d) in after.clone() {
                if b == c && a != d {
                    after.insert((a, d));
                }
            }
        }
        if after.len() == n {
            break;
        }
    }
    let mut trace = Trace {
        whole: true,
        templates: Arc::new(BTreeMap::new()),
        prefix: format!("__it{namespace}_global"),
        emitted: e,
        relations,
        steps,
        after,
        frames: vec![],
        limit: run.max_waves,
    };
    // Unsupported SQL shapes can retain their established fallback. Only changes made
    // during this attempt enter the nested undo log; existing sessions are not scanned.
    let pool = w.pool.clone();
    let mut attempt = Work::new(w.db, &pool);
    attempt.versions = Some(store);
    attempt.profiler = w.profiler;
    attempt.preparing = true;
    w.db.execute_batch("SAVEPOINT sql_initial_retained")
        .map_err(error)?;
    store.savepoint();
    let result: Result<(u32, frame::Versions)> = (|| {
        let _cpu =
            super::super::super::logic::profile::span(|| "initial retained execution".into());
        if super::super::super::logic::profile::on() {
            super::super::super::logic::profile::stratum(
                &format!("{}: initial retained execution", original.name),
                "SQL retained waves",
                || "SQL queries and one global wave schedule construct the retained roots".into(),
            );
        }
        let _context = attempt.profile_scope(|| json!({"stage":"initial retained execution"}));
        let waves = trace.replay(&mut attempt, store, &BTreeMap::new(), run.max_waves)?;
        let final_versions = trace.frames.last().unwrap().versions(store);
        for (r, spec) in &trace.relations {
            let frame = trace.frames.last().unwrap();
            let _scope = store.activate(frame.scope())?;
            w.db.execute(
                &format!(
                    "INSERT INTO {} SELECT * FROM {}",
                    quote(r),
                    quote(&frame.published[r])
                ),
                [],
            )
            .map_err(error)?;
            if spec.widening() {
                w.db.execute(
                    &format!(
                        "INSERT INTO __sql_changes_{r} SELECT * FROM {}",
                        quote(&frame.counters[r])
                    ),
                    [],
                )
                .map_err(error)?;
            }
        }
        attempt.cleanup()?;
        w.db.execute_batch("RELEASE sql_initial_retained")
            .map_err(error)?;
        Ok((waves, final_versions))
    })();
    match result {
        Ok((waves, versions)) => {
            store.commit();
            w.pool = attempt.pool;
            w.created += attempt.created;
            w.version_nodes += attempt.version_nodes;
            for (r, v) in versions.published {
                run.counts.insert(r, v.len() as u64);
            }
            run.objects.extend(trace.objects());
            run.temporal.insert(0, trace);
            Ok(Some(waves))
        }
        Err(err) => {
            store.rollback(w.db)?;
            w.db.execute_batch("ROLLBACK TO sql_initial_retained; RELEASE sql_initial_retained")
                .map_err(error)?;
            if err.status == napi::Status::InvalidArg && err.reason.contains(UNSUPPORTED) {
                Ok(None)
            } else {
                Err(err)
            }
        }
    }
}

pub(in crate::code_rule_dsl::sql_executor) fn apply(
    w: &mut Work<'_>,
    run: &mut session::Execution,
    changes: &mut BTreeMap<String, Delta>,
    limit: u32,
) -> Result<Value> {
    let mut trace = run.temporal.remove(&0).unwrap();
    let result = (|| {
        // Base-only projection views remain ordinary retained inputs. They are updated
        // before the global schedule, matching their original lazy SQL view semantics.
        let mut views = run.clone();
        let e = Arc::make_mut(&mut views.emitted);
        e.program
            .relations
            .retain(|(r, _)| !trace.relations.contains_key(r));
        let keep = e
            .program
            .steps
            .iter()
            .enumerate()
            .filter_map(|(id, s)| (!trace.relations.contains_key(&s.output)).then_some(id))
            .collect::<Vec<_>>();
        e.program.steps = keep.iter().map(|&i| e.program.steps[i].clone()).collect();
        e.queries = keep.iter().map(|&i| e.queries[i].clone()).collect();
        views.reductions = None;
        views.recursive.clear();
        let view_names = e
            .program
            .relations
            .iter()
            .map(|(r, _)| r.clone())
            .collect::<Vec<_>>();
        let mut report = maintenance::maintain(w, &mut views, changes, limit)?;
        let store = w
            .versions
            .ok_or_else(|| error("retained initial state requires native storage"))?;
        let (deltas, waves) = trace.apply(w, store, changes, limit)?;
        let outputs = report["outputs"].as_object_mut().unwrap();
        let mut incremental = Vec::new();
        for (r, d) in deltas {
            let n = run.counts.get(&r).copied().unwrap_or(0) + d.inserted - d.deleted;
            run.counts.insert(r.clone(), n);
            if run.outputs.contains(&r) {
                outputs.insert(
                    r.clone(),
                    json!({"inserted":d.inserted,"deleted":d.deleted}),
                );
            }
            incremental.push(r.clone());
            changes.insert(r, d);
        }
        for r in view_names {
            run.counts.insert(r.clone(), views.counts[&r]);
        }
        report["positive_waves"] = json!(waves);
        report["recursive_frames"] = json!(waves);
        report["incremental_strata"]
            .as_array_mut()
            .unwrap()
            .extend(incremental.into_iter().map(Value::String));
        let owned = run
            .objects
            .iter()
            .map(|(n, _)| n.clone())
            .collect::<BTreeSet<_>>();
        run.objects
            .extend(trace.objects().filter(|(n, _)| !owned.contains(n)));
        Ok(report)
    })();
    run.temporal.insert(0, trace);
    result
}
