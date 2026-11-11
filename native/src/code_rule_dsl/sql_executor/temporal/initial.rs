//! One global wave schedule constructs the retained state and produces the initial answer.
use super::*;

pub(in crate::code_rule_dsl::sql_executor) fn eligible(
    e: &plan::emit::Emitted,
    producers: &BTreeMap<String, Vec<String>>,
) -> bool {
    if e.queries.iter().zip(&e.program.steps)
        .any(|(q,s)| q.is_none() && !e.constant_rows.contains_key(&s.output) && !producers.contains_key(&s.output)) || e.program.steps.is_empty() {
        return false;
    }
    let positions = e.program.steps.iter().map(|s|(s.output.as_str(),s.stratum)).collect::<BTreeMap<_,_>>();
    let recursive = |s:&schedule::Step,q:&Query| s.stratum>0 && q.reads().iter()
        .filter_map(|r|r.program_relation()).any(|r|positions.get(r)==Some(&s.stratum));
    let recursive_steps=e.program.steps.iter().zip(&e.queries).filter(|(s,q)|q.as_ref().is_some_and(|q|recursive(s,q))).collect::<Vec<_>>();
    if recursive_steps.iter().any(|(s,q)|maintenance::barrier(q.as_ref().unwrap()) ||
        e.program.lattices.iter().any(|(r,_,_)|positions.get(r.as_str())==Some(&s.stratum))) {
        return true;
    }
    if !producers.is_empty() { return true; }
    if super::cold_loop::candidate(e) { return true; }
    // Preserve the specialized walk and partition kernels beyond the checked phase-size boundary.
    let composable=e.program.steps.len()<=32 && e.queries.iter().flatten().all(|q|q.ctes.is_empty());
    if !recursive_steps.is_empty() {
        composable && e.queries.iter().flatten().any(maintenance::barrier)
    } else {
        e.queries.iter().flatten().all(|q| q.ctes.is_empty())
    }
}

pub(in crate::code_rule_dsl::sql_executor) fn run(
    w: &mut Work<'_>,
    store: &versions::Store,
    run: &mut session::Execution,
    namespace: usize,
    original: &schedule::Program,
    declared_counts: bool,
    native_relations: BTreeSet<String>,
    mut evaluation: Option<&mut planning::Evaluation>,
) -> Result<Option<u32>> {
    if !eligible(&run.emitted, &run.producers) { return Ok(None); }
    let e = run.emitted.clone();
    let outputs = original.relations.iter().map(|(r,_)|r.clone()).collect::<BTreeSet<_>>();
    let mut steps = e
        .program
        .steps
        .iter()
        .zip(&e.queries)
        .filter(|(s, _)| outputs.contains(&s.output))
        .map(|(s, q)| (s.clone(), q.clone()))
        .collect::<Vec<_>>();
    let batch_selected = batch::selected(steps.len(), w);
    if let Some(eval) = evaluation.as_deref_mut() {
        let strategy = if batch_selected {
            planning::Strategy::FullWaveBatch
        } else {
            planning::Strategy::SparseRetained
        };
        eval.choose(strategy, batch::wide(steps.len()));
    }
    let lattice_names = original.lattices.iter().map(|(r, _, _)| r.clone()).collect();
    let relation_widths = original.relations.iter().cloned().collect();
    let (cache_decisions, stage_decisions) = if batch_selected {
        (Arc::new(vec![]), BTreeMap::new())
    } else {
        let decisions = compact(&mut steps, &lattice_names);
        let stages = plan::acyclic::analyze(&steps, &relation_widths, &lattice_names);
        (decisions, stages)
    };
    let probes = indexes::native(steps.iter().filter_map(|(_, q)| q.as_ref()));
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
    let after = dependency_gates(&steps);
    let components=Arc::new(components(&steps));
    let mut trace = Trace {
        whole: true,
        producers: Arc::new(run.producers.clone()),
        publications: opaque::Publications::default(),
        native_relations,
        templates: Arc::new(BTreeMap::new()),
        prefix: format!("__it{namespace}_global"),
        emitted: e,
        relations,
        components,
        steps: Arc::new(steps),
        stage_decisions,
        cache_decisions,
        after,
        frames: vec![],
        batch: None,
        limit: run.max_waves,
    };
    if batch_selected {
        // The batch consumes the original delta variants at their emitted gates.
        trace.batch = batch::History::prepare(&trace, w.db)?;
    }
    if batch_selected && trace.batch.is_none() {
        if let Some(eval) = evaluation {
            eval.choose(planning::Strategy::SparseRetainedAfterBatchAttempt, false);
        }
        let steps = Arc::make_mut(&mut trace.steps);
        trace.cache_decisions = compact(steps, &lattice_names);
        trace.stage_decisions = plan::acyclic::analyze(steps, &relation_widths, &lattice_names);
        trace.components = Arc::new(super::components(steps));
        trace.after = dependency_gates(steps);
    }
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
        let waves = if let Some(batch) = &mut trace.batch {
            batch.replay(&mut attempt, &BTreeMap::new(), run.max_waves, true)?
        } else {
            match super::cold_loop::run(&mut trace, &mut attempt, store, run.max_waves)? {
                Some(waves) => waves,
                None => trace.replay(&mut attempt, store, &BTreeMap::new(), run.max_waves)?,
            }
        };
        let final_versions = trace.final_versions(store);
        for (r, spec) in &trace.relations {
            if trace.native_relations.contains(r) {
                let _scope = store.activate(0)?;
                store.alias_readonly(w.db,r,final_versions.published[r].clone(),
                    (0..spec.width).map(|c|format!("c{c}")).collect())?;
            } else {
                let _scope = store.activate(trace.frames.last().map_or(0, Frame::scope))?;
                trace.publications.publish(&mut attempt,r,final_versions.published[r].clone())?;
            }
            if spec.widening() {
                let counters=final_versions.counters[r].clone();
                let _public=store.activate(0)?;
                let table=format!("__sql_changes_{r}");
                if declared_counts && !store.contains(&table) {
                    w.db.execute_batch(&format!("DROP TABLE temp.{}",quote(&table))).map_err(error)?;
                }
                store.alias_readonly(w.db,&table,counters,
                    (0..spec.keys).map(|c|format!("c{c}")).chain(["changes".into()]).collect())?;
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
            w.summary_reuses += attempt.summary_reuses;
            for (r, v) in versions.published {
                run.counts.insert(r, v.len() as u64);
            }
            let unused = original.relations.iter().flat_map(|(r,_)|[format!("new_{r}"),format!("delta_{r}")]).collect::<BTreeSet<_>>();
            if !declared_counts {
                run.objects.retain(|(name,_)|!unused.contains(name));
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

fn compact(
    steps: &mut Vec<(schedule::Step, Option<Query>)>,
    lattice_names: &BTreeSet<String>,
) -> Arc<Vec<Value>> {
    let decisions = plan::acyclic::compact_retained(steps, lattice_names);
    plan::acyclic::order(steps);
    Arc::new(decisions)
}

fn dependency_gates(steps: &[(schedule::Step, Option<Query>)]) -> BTreeSet<(u32, u32)> {
    let mut after = BTreeSet::new();
    for (step, _) in steps {
        for (producer, _) in steps {
            if step.stratum != producer.stratum && step.reads.contains(&producer.output) {
                after.insert((step.stratum, producer.stratum));
            }
        }
    }
    loop {
        let before = after.len();
        let direct = after.clone();
        for &(a, b) in &direct {
            for &(c, d) in &direct {
                if b == c && a != d {
                    after.insert((a, d));
                }
            }
        }
        if after.len() == before { return after; }
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
        let mut report = json!({"positive_waves":0,"overdelete_waves":0,"refreshed_strata":[],"incremental_strata":[],"recursive_partitions":0,"recursive_frames":0,"reducer_groups":0,"context_rows":0,"outputs":{}});
        let store = w
            .versions
            .ok_or_else(|| error("retained initial state requires native storage"))?;
        let before = (w.reducer_groups,w.context_rows);
        let (deltas, waves) = trace.apply(w, store, changes, limit)?;
        report["reducer_groups"]=json!(w.reducer_groups-before.0);
        report["context_rows"]=json!(w.context_rows-before.1);
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
            if d.inserted+d.deleted>0 { incremental.push(r.clone()); }
            changes.insert(r, d);
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
