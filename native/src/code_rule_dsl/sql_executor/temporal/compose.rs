//! Evaluate positive operator support, improvements, and wave folds in one SQL statement.
use super::*;

fn changes(
    w: &mut Work<'_>,
    queries: &mut Vec<String>,
    old: &versions::Version,
    new: &versions::Version,
    _keys: usize,
) -> Result<()> {
    if old.same(new) {
        return Ok(());
    }
    let d = delta(w, old, new)?;
    for (table, n) in [(d.plus, d.inserted), (d.minus, d.deleted)] {
        if n > 0 {
            queries.push(table);
        }
    }
    Ok(())
}
fn union(queries: &[String], width: usize) -> String {
    if queries.is_empty() {
        format!("SELECT {} WHERE 0", vec!["NULL"; width].join(","))
    } else {
        queries.join(" UNION ")
    }
}

fn covered(
    phase: &super::super::phase::Phase,
    decisions: &mut Vec<Value>,
    relation: &str,
    branch: &str,
    affected: &str,
    replaced: &str,
    width: usize,
) -> bool {
    let affected = phase.key_set(affected, width);
    let replaced = phase.key_set(replaced, width);
    let decision = affected.contained_in(&replaced);
    let proven = decision.proven;
    decisions.push(
        json!({"analysis":"key set containment","relation":relation,"branch":branch,
        "affected":affected,"replacement":replaced,"decision":decision,"omitted":proven,
        "strategy":if proven {"omit old branch"} else {"materialized key difference"}}),
    );
    proven
}

impl Frame {
    pub(super) fn advance_phase(
        &mut self,
        w: &mut Work<'_>,
        store: &versions::Store,
        relations: &BTreeMap<String, Relation>,
        steps: &[(schedule::Step, Query)],
        desired: Option<frame::Versions>,
        _external: &BTreeMap<String, Delta>,
        active: &BTreeSet<usize>,
        preparing: bool,
        templates: &BTreeMap<usize, frame::Prepared>,
        limit: u32,
    ) -> Result<()> {
        let mut phase = super::super::phase::Phase::default();
        let mut summaries = super::super::reductions::reducer::staged::Batch::default();
        let mut key_set_decisions = vec![];
        let mut touched = relations
            .keys()
            .map(|r| (r.clone(), vec![]))
            .collect::<BTreeMap<_, Vec<String>>>();
        if let Some(desired) = desired {
            for (r, spec) in relations {
                for (logical, version) in [
                    (r.clone(), &desired.published[r]),
                    (format!("delta_{r}"), &desired.frontiers[r]),
                ] {
                    let table = &self.inputs[&logical];
                    let old = store.version(table);
                    if logical == *r {
                        changes(w, touched.get_mut(r).unwrap(), &old, version, spec.keys)?;
                    }
                    if !old.same(version) {
                        store.replace(table, version.clone())?;
                    }
                }
                if spec.widening() {
                    let table = &self.input_counters[r];
                    let old = store.version(table);
                    changes(
                        w,
                        touched.get_mut(r).unwrap(),
                        &old,
                        &desired.counters[r],
                        spec.keys,
                    )?;
                    if !old.same(&desired.counters[r]) {
                        store.replace(table, desired.counters[r].clone())?;
                    }
                }
            }
        }
        // Each overlay contains only this statement's changed improvement keys.
        let mut overlays = BTreeMap::new();
        let mut batches = super::batches::Batches::default();
        let mut pipeline = super::pipeline::Pipeline::default();
        let stage_outputs = active.iter().filter(|id| self.pipeline.contains(id))
            .flat_map(|&id| steps[id].0.reads.iter().cloned()).collect::<BTreeSet<_>>();
        let last_reads = self.operators.iter().enumerate().flat_map(|(id,op)|
            op.run.inputs.iter().map(move |r|(r.clone(),id))).collect::<BTreeMap<_,_>>();
        let mut batch_mark = w.mark();
        for (id, op) in self.operators.iter_mut().enumerate() {
            let enabled = active.contains(&id);
            let r = &steps[id].0.output;
            let spec = &relations[r];
            if phase.arm_bound() > 0
                && (phase.arm_bound() + op.phase_arms.max(4) + overlays.len() + 1 + pipeline.changes.len()*2 >= 470
                    || pipeline.depth(&op.run.inputs) >= super::pipeline::Pipeline::MAX_DEPTH
                    || phase.expansion_cost() >= phase::Phase::EXPANSION_BUDGET)
            {
                pipeline.retain_from(id, &last_reads);
                let slots = pipeline.stage(&mut phase, w, store)?;
                batches.cache(
                    &mut phase,
                    &mut touched,
                    relations,
                    w,
                    store,
                    &key_set_decisions,
                )?;
                w.reducer_groups += summaries.finish(w, &batches.counts)?;
                let snapshots = slots.into_iter().map(|(r,p,m)|(r,store.available(&p).unwrap(),store.available(&m).unwrap())).collect();
                w.rewind(batch_mark)?;
                batch_mark = w.mark();
                pipeline.resume(w, store, snapshots)?;
                overlays.clear();
            }
            let previous_result = store.version(&op.result);
            let adopted = enabled && !op.ready && !preparing;
            if adopted {
                let template = templates
                    .get(&id)
                    .ok_or_else(|| error("missing initial cache for an earlier wave gate"))?;
                for (name, version) in &template.roots {
                    if store.contains(name) {
                        store.replace(name, version.clone())?;
                    } else {
                        store.fork_version(w.db, name, version.clone())?;
                    }
                }
                op.adopt(&template.operator);
            }
            let old_improvements = store.version(&op.improvements);
            let pipelined = self.pipeline.contains(&id) && enabled && stage_outputs.contains(r);
            let staged_input = op.seen.keys().any(|r| pipeline.bindings.contains_key(r));
            if op.ready && !staged_input && op.clear_empty_body(w.db,store)? {
                if op.active && old_improvements.len() > 0 {
                    let old = w.output_table(spec.width)?;
                    store.clone_scratch(&old, &old_improvements)?;
                    touched.get_mut(r).unwrap().push(old);
                }
                store.replace(&op.improvements, old_improvements.empty_like())?;
                op.comparison = store.version(&self.inputs[r]);
                op.active = enabled;
                if pipelined {
                    let d = delta(w, &previous_result, &store.version(&op.result))?;
                    pipeline.native(&self.inputs[r], &op.result, d);
                }
                continue;
            }
            if !enabled && !op.active {
                continue;
            }
            let mut changed = BTreeMap::new();
            let initial = !op.ready;
            if enabled && !initial {
                for (r, v) in &op.seen {
                    let now = store.version(r);
                    if !v.same(&now) {
                        changed.insert(r.clone(), delta(w, v, &now)?);
                    }
                }
                for (r, p) in &op.pending {
                    let (inserted, deleted) = (
                        store.version(&p.plus).len() as u64,
                        store.version(&p.minus).len() as u64,
                    );
                    if inserted + deleted > 0 {
                        changed.insert(
                            r.clone(),
                            Delta {
                                plus: p.plus.clone(),
                                minus: p.minus.clone(),
                                inserted,
                                deleted,
                            },
                        );
                    }
                }
                for r in &op.run.inputs {
                    if let Some(d) = pipeline.changes.get(r) { changed.insert(r.clone(), d.clone()); }
                }
            }
            let mut queries = vec![];
            let mut result = None;
            if adopted && (op.phase_eligible || op.sql_kernel) {
                changes(
                    w,
                    &mut queries,
                    &previous_result,
                    &store.version(&op.result),
                    spec.keys,
                )?;
            }
            if op.sql_kernel {
                if enabled && (initial || !changed.is_empty()) {
                    let next = op.append_kernel(&mut phase, spec.width);
                    let keys = phase.union_keys(spec.keys, &[op.result.clone(), next.clone()]);
                    phase.replace_all(store, &op.result, &next)?;
                    queries.push(keys.clone());
                    result = Some((keys, next, true));
                    op.ready = true;
                    if w.profiler.is_some() {
                        phase.analyses.push(json!({"operation":"recursive SQL kernel","relation":r,"proof":op.logical_analysis.as_ref()}));
                    }
                }
            } else if !op.phase_eligible {
                // This boundary consumes its own journals. Compatible neighbors remain in this phase.
                op.update(w, store, &BTreeMap::new(), enabled, preparing, limit)?;
                changes(
                    w,
                    &mut queries,
                    &previous_result,
                    &store.version(&op.result),
                    spec.keys,
                )?;
            } else if enabled && (initial || !changed.is_empty()) {
                if initial {
                    if !preparing {
                        return Err(error(
                            "unprepared temporal step became active during an edit",
                        ));
                    }
                    operator::order(
                        Arc::make_mut(&mut op.run.emitted).queries[0]
                            .as_mut()
                            .unwrap(),
                        w.db,
                    );
                    let state = &mut op
                        .run
                        .reductions
                        .as_mut()
                        .unwrap()
                        .get_mut(&op.result)
                        .unwrap();
                    state.declare_phase(w.db, &mut op.run.objects, store)?;
                    state.initialize_summaries(w, &mut op.run.objects, store, &mut phase, &mut summaries)?;
                    state.ready = true;
                }
                let state = &op.run.reductions.as_ref().unwrap()[&op.result];
                let full = initial
                    || w.preparing
                        && changed.values().any(|d| {
                            d.inserted + d.deleted >= 512
                                && d.inserted + d.deleted
                                    >= op.run.counts.get(&op.result).copied().unwrap_or(0) / 2
                        });
                let (keys, next) = if staged_input { state.bind_inputs(&pipeline.bindings).append_phase_with(w, &changed, &mut phase, full, &mut summaries)? }
                    else { state.append_phase_with(w, &changed, &mut phase, full, &mut summaries)? };
                queries.push(keys.clone());
                result = Some((keys, next, full));
                op.ready = true;
            }
            if pipelined {
                if let Some((keys,next,full)) = &result {
                    let depth = pipeline.depth(&op.run.inputs) + 1;
                    pipeline.append(&mut phase, &self.inputs[r], &op.result, spec.width, keys, next, *full, depth);
                } else {
                    pipeline.unchanged(&self.inputs[r], &op.result);
                }
            }
            let current = store.version(&self.inputs[r]);
            changes(w, &mut queries, &op.comparison, &current, spec.keys)?;
            op.comparison = current;
            if !queries.is_empty() {
                let affected = phase.union_keys(spec.keys, &queries);
                // A full cache transition covers every old result key. Its
                // overlay therefore contains only the new result, including
                // when comparison changes add keys outside that result.
                let full = result.as_ref().is_some_and(|(_, _, full)| *full);
                let candidates = if let Some((_, next, true)) = &result {
                    format!("SELECT * FROM {next}")
                } else if let Some((keys, next, _)) = &result {
                    if covered(
                        &phase,
                        &mut key_set_decisions,
                        r,
                        "old result",
                        &affected,
                        keys,
                        spec.keys,
                    ) {
                        format!("SELECT * FROM {next}")
                    } else {
                        let remaining = phase.difference_keys(spec.keys, &affected, keys);
                        format!(
                            "SELECT c.* FROM {remaining} k CROSS JOIN {} c WHERE {} UNION SELECT * FROM {next}",
                            quote(&op.result),
                            same("c", "k", spec.keys)
                        )
                    }
                } else {
                    format!(
                        "SELECT c.* FROM {affected} k CROSS JOIN {} c WHERE {}",
                        quote(&op.result),
                        same("c", "k", spec.keys)
                    )
                };
                let candidates = if full {
                    phase.view(spec.width, &candidates)
                } else {
                    phase.relation(spec.width, &candidates)
                };
                let improve = if spec.lattice {
                    let value = format!("c{}", spec.width - 1);
                    format!(
                        "p.{value} IS NULL OR {}<>p.{value}",
                        spec.merge(&format!("p.{value}"), &format!("c.{value}"))
                    )
                } else {
                    "p.c0 IS NULL".into()
                };
                let values = phase.relation(
                    spec.width,
                    &format!(
                        "SELECT c.* FROM {candidates} c LEFT JOIN {} p ON {} WHERE {improve}",
                        quote(&self.inputs[r]),
                        same("p", "c", spec.keys)
                    ),
                );
                if full {
                    phase.replace_all(store, &op.improvements, &values)?;
                } else {
                    phase.patch_computed(store, &op.improvements, &affected, &values)?;
                }
                touched.get_mut(r).unwrap().push(affected.clone());
                overlays.insert(id, (affected, values, full));
            }
            if enabled != op.active && old_improvements.len() > 0 {
                touched.get_mut(r).unwrap().push(op.improvements.clone());
            }
            op.active = enabled;
            if enabled {
                for (r, v) in &mut op.seen {
                    *v = store.version(r);
                }
                // Pending roots are read by this statement. Clear them after it completes.
            }
        }
        if batches.cache_batches > 0
            || phase.arm_bound() + touched.values().filter(|q| !q.is_empty()).count() * 6 >= 480
            || phase.expansion_cost() >= phase::Phase::EXPANSION_BUDGET
        {
            batches.cache(
                &mut phase,
                &mut touched,
                relations,
                w,
                store,
                &key_set_decisions,
            )?;
            w.reducer_groups += summaries.finish(w, &batches.counts)?;
            w.rewind(batch_mark)?;
            batch_mark=w.mark();
            batches.bind(&mut touched, w, store)?;
            overlays.clear();
        }
        for (r, spec) in relations {
            if touched[r].is_empty() {
                continue;
            }
            if phase.expansion_cost() >= phase::Phase::EXPANSION_BUDGET {
                // Remaining folds can still reference this phase's staged caches and keys.
                // Publish and rebind those roots before discarding its SQL definitions.
                batches.cache(&mut phase,&mut touched,relations,w,store,&key_set_decisions)?;
                w.reducer_groups += summaries.finish(w, &batches.counts)?;
                w.rewind(batch_mark)?;
                batch_mark=w.mark();
                batches.bind(&mut touched,w,store)?;
                overlays.clear();
            } else if phase.arm_bound() + 6 >= 480 {
                batches.execute(&mut phase, w, &key_set_decisions)?;
                w.reducer_groups += summaries.finish(w, &batches.counts)?;
            }
            let keys = phase.union_keys(spec.keys, &touched[r]);
            let mut candidates = vec![];
            let mut contributors = 0;
            for (id, op) in self
                .operators
                .iter()
                .enumerate()
                .filter(|(id, op)| op.active && steps[*id].0.output == *r)
            {
                contributors += 1;
                let overlay = overlays.get(&id);
                // Old improvements are a subset of the old result. A complete
                // replacement excludes all of them without an anti-join.
                if !overlay.is_some_and(|(affected, _, full)| {
                    *full
                        || covered(
                            &phase,
                            &mut key_set_decisions,
                            r,
                            "old improvements",
                            &keys,
                            affected,
                            spec.keys,
                        )
                }) {
                    let remaining = overlay.map_or_else(
                        || keys.clone(),
                        |(affected, _, _)| phase.difference_keys(spec.keys, &keys, affected),
                    );
                    candidates.push(format!(
                        "SELECT c.* FROM {remaining} k CROSS JOIN {} c WHERE {}",
                        quote(&op.improvements),
                        same("c", "k", spec.keys)
                    ));
                }
                if let Some((_, values, _)) = overlay {
                    candidates.push(format!("SELECT * FROM {values}"));
                }
            }
            // Each operator is already folded by this relation's keys. Its old
            // and replacement branches cover disjoint keys. Multiple operators
            // still require a fold, but builtin joins do not need deduplication.
            let candidates =
                if spec.grouping().reduction.duplicate_insensitive() && !candidates.is_empty() {
                    candidates.join(" UNION ALL ")
                } else {
                    union(&candidates, spec.width)
                };
            fold::append(
                w,
                &mut phase,
                spec,
                &keys,
                &self.inputs[r],
                self.input_counters.get(r).map(String::as_str),
                &self.published[r],
                &self.frontiers[r],
                self.counters.get(r).map(String::as_str),
                &candidates,
                (contributors == 1).then(|| spec.grouping()),
                true,
            )?;
        }
        batches.execute(&mut phase, w, &key_set_decisions)?;
        w.reducer_groups += summaries.finish(w, &batches.counts)?;
        self.phase_batches = batches.phases;
        let counts = batches.counts;
        self.key_set_decisions = key_set_decisions;
        self.publication_decisions = counts
            .iter()
            .map(|(table, c)| {
                json!({
                    "table":table,"affected_keys":c.keys,"rows":c.rows,"strategy":c.strategy
                })
            })
            .collect();
        for op in &mut self.operators {
            if op.phase_eligible && op.active {
                if let Some(states) = &op.run.reductions {
                    w.context_rows += states
                        .values()
                        .map(|s| s.phase_context_rows(&counts))
                        .sum::<u64>();
                }
            }
            if op.active {
                op.run
                    .counts
                    .insert(op.result.clone(), store.version(&op.result).len() as u64);
                // Empty-body suspension already clears unprotected journals.
                // Protected scalar inputs must reach their original maintenance cursor.
                if !op.empty_body {
                    for p in op.pending.values() {
                        p.clear(store)?;
                    }
                }
            }
        }
        Ok(())
    }
}
