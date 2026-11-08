//! Evaluate positive operator support, improvements, and wave folds in one SQL statement.
use super::*;

fn changes(
    w: &mut Work<'_>,
    queries: &mut Vec<String>,
    old: &versions::Version,
    new: &versions::Version,
    keys: usize,
) -> Result<()> {
    if old.same(new) {
        return Ok(());
    }
    let d = delta(w, old, new)?;
    for (table, n) in [(d.plus, d.inserted), (d.minus, d.deleted)] {
        if n > 0 {
            queries.push(format!("SELECT {} FROM {}", names("", keys), quote(&table)));
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

impl Frame {
    pub(super) fn advance_phase(
        &mut self,
        w: &mut Work<'_>,
        store: &versions::Store,
        relations: &BTreeMap<String, Relation>,
        steps: &[(schedule::Step, Query)],
        desired: Option<frame::Versions>,
        external: &BTreeMap<String, Delta>,
        active: &BTreeSet<usize>,
        preparing: bool,
        templates: &BTreeMap<usize, frame::Prepared>,
    ) -> Result<()> {
        let mut phase = super::super::phase::Phase::default();
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
        for (id, op) in self.operators.iter_mut().enumerate() {
            let enabled = active.contains(&id);
            let r = &steps[id].0.output;
            let spec = &relations[r];
            if enabled && !op.ready && !preparing {
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
            for (r, p) in &op.pending {
                if let Some(d) = external.get(r).filter(|d| d.inserted + d.deleted > 0) {
                    p.add(w, d)?;
                }
            }
            let old_improvements = store.version(&op.improvements);
            if op.ready && op.clear_empty_body(store)? {
                if op.active && old_improvements.len() > 0 {
                    let old = w.output_table(spec.width)?;
                    store.clone_scratch(&old, &old_improvements)?;
                    touched.get_mut(r).unwrap().push(format!(
                        "SELECT {} FROM {}",
                        names("", spec.keys),
                        quote(&old)
                    ));
                }
                store.replace(&op.improvements, old_improvements.empty_like())?;
                op.comparison = store.version(&self.inputs[r]);
                op.active = enabled;
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
            }
            let mut queries = vec![];
            let mut result = None;
            if enabled && (initial || !changed.is_empty()) {
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
                    op.run.reductions = Some(reductions::planned(w.db, &op.run, true)?);
                    let state = &mut op
                        .run
                        .reductions
                        .as_mut()
                        .unwrap()
                        .get_mut(&op.result)
                        .unwrap();
                    state.declare_phase(w.db, &mut op.run.objects, store)?;
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
                let (keys, next) = state.append_phase(w, &changed, &mut phase, full)?;
                queries.push(format!("SELECT * FROM {keys}"));
                result = Some((keys, next));
                op.ready = true;
            }
            let current = store.version(&self.inputs[r]);
            changes(w, &mut queries, &op.comparison, &current, spec.keys)?;
            op.comparison = current;
            if !queries.is_empty() {
                let affected = phase.relation(spec.keys, &union(&queries, spec.keys));
                let candidates = if let Some((keys, next)) = result {
                    format!(
                        "SELECT c.* FROM {affected} k CROSS JOIN {} c WHERE {} AND NOT EXISTS(SELECT 1 FROM {keys} d WHERE {}) UNION SELECT * FROM {next}",
                        quote(&op.result),
                        same("c", "k", spec.keys),
                        same("d", "c", spec.keys)
                    )
                } else {
                    format!(
                        "SELECT c.* FROM {affected} k CROSS JOIN {} c WHERE {}",
                        quote(&op.result),
                        same("c", "k", spec.keys)
                    )
                };
                let candidates = phase.relation(spec.width, &candidates);
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
                phase.patch_computed(store, &op.improvements, &affected, &values)?;
                touched
                    .get_mut(r)
                    .unwrap()
                    .push(format!("SELECT * FROM {affected}"));
                overlays.insert(id, (affected, values));
            }
            if enabled != op.active && old_improvements.len() > 0 {
                touched.get_mut(r).unwrap().push(format!(
                    "SELECT {} FROM {}",
                    names("", spec.keys),
                    quote(&op.improvements)
                ));
            }
            op.active = enabled;
            if enabled {
                for (r, v) in &mut op.seen {
                    *v = store.version(r);
                }
                // Pending roots are read by this statement. Clear them after it completes.
            }
        }
        for (r, spec) in relations {
            if touched[r].is_empty() {
                continue;
            }
            let keys = phase.relation(spec.keys, &union(&touched[r], spec.keys));
            let mut candidates = vec![];
            for (id, op) in self
                .operators
                .iter()
                .enumerate()
                .filter(|(id, op)| op.active && steps[*id].0.output == *r)
            {
                let exclude = overlays.get(&id).map_or(String::new(), |(affected, _)| {
                    format!(
                        " AND NOT EXISTS(SELECT 1 FROM {affected} d WHERE {})",
                        same("d", "c", spec.keys)
                    )
                });
                candidates.push(format!(
                    "SELECT c.* FROM {keys} k CROSS JOIN {} c WHERE {}{exclude}",
                    quote(&op.improvements),
                    same("c", "k", spec.keys)
                ));
                if let Some((_, values)) = overlays.get(&id) {
                    candidates.push(format!("SELECT * FROM {values}"));
                }
            }
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
                &union(&candidates, spec.width),
                true,
            )?;
        }
        phase.execute(w)?;
        for op in &mut self.operators {
            if op.active {
                op.run
                    .counts
                    .insert(op.result.clone(), store.version(&op.result).len() as u64);
                for p in op.pending.values() {
                    p.clear(store)?;
                }
            }
        }
        Ok(())
    }
}
