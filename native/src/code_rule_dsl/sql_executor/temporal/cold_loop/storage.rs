//! Construct private patches and iteration roots from SQL results, without evaluating SQL.
use super::*;
use rusqlite::functions::{Aggregate, Context};

impl Arena {
    pub fn private(&self, handle: i64) -> rusqlite::Result<&Private> {
        handle
            .checked_neg()
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| self.private.get(n))
            .ok_or_else(|| sql_error("invalid private retained version"))
    }
    fn append_private(&mut self, private: Private) -> i64 {
        self.private.push(private);
        -(self.private.len() as i64)
    }
    pub fn clear(&mut self, parent: i64, id: usize) -> rusqlite::Result<i64> {
        let _span =
            crate::code_rule_dsl::logic::profile::span(|| "retained loop empty storage".into());
        let cpu = crate::code_rule_dsl::sql_profile::thread_cpu_ns();
        let stage = self.plan.stage(id as i64)?;
        let state = self.state(parent)?;
        let roots = stage
            .outputs
            .iter()
            .map(|&slot| (slot, state.roots[slot].empty_like()))
            .collect();
        let counts = stage
            .outputs
            .iter()
            .map(|&slot| {
                (self.plan.targets[&slot].clone(),phase::Counts{
            keys:state.roots[slot].len(),rows:0,
            strategy:crate::code_rule_dsl::logic::analysis::storage::Publication::BulkReplacement,
        })
            })
            .collect();
        let handle = self.append_private(Private {
            parent,
            roots,
            counts,
            cleared: vec![id],
        });
        self.storage_cpu_ns = self
            .storage_cpu_ns
            .zip(cpu.zip(crate::code_rule_dsl::sql_profile::thread_cpu_ns()))
            .map(|(total, (start, end))| total + end.saturating_sub(start));
        Ok(handle)
    }
    pub fn merge(&mut self, parent: i64, handles: &[i64]) -> rusqlite::Result<i64> {
        self.state(parent)?;
        let mut combined = Private {
            parent,
            roots: vec![],
            counts: BTreeMap::new(),
            cleared: vec![],
        };
        for &handle in handles {
            if handle == parent {
                continue;
            }
            let private = self.private(handle)?;
            if private.parent != parent {
                return Err(sql_error("mixed retained stage parents"));
            }
            combined.roots.extend(private.roots.iter().cloned());
            combined.cleared.extend(&private.cleared);
            combined
                .counts
                .extend(private.counts.iter().map(|(n, c)| (n.clone(), c.clone())));
        }
        Ok(self.append_private(combined))
    }
    fn complete(&mut self, id: i64, parent: i64, buffers: phase::Buffers) -> rusqlite::Result<i64> {
        let _span =
            crate::code_rule_dsl::logic::profile::span(|| "retained SQL loop storage".into());
        let cpu = crate::code_rule_dsl::sql_profile::thread_cpu_ns();
        let result = (|| {
            let stage = self.plan.stage(id)?;
            let snapshots = stage
                .snapshots
                .iter()
                .map(|&slot| self.root(parent, slot))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let built = stage
                .template
                .build(&snapshots, buffers)
                .map_err(|e| sql_error(e.to_string()))?;
            self.shared_roots += built.shared_roots;
            self.retained_roots += built.retained_roots;
            self.context_rows += self
                .plan
                .contexts
                .iter()
                .map(|s| s.phase_context_rows(&built.counts))
                .sum::<u64>();
            let roots = stage
                .outputs
                .iter()
                .copied()
                .zip(built.roots.into_iter().map(|(_, v)| v))
                .collect::<Vec<_>>();
            if id >= 0 {
                self.state(parent)?;
                return Ok(self.append_private(Private {
                    parent,
                    roots,
                    counts: built.counts,
                    cleared: vec![],
                }));
            }
            let mut combined = if parent < 0 {
                let p = self.private(parent)?;
                Private {
                    parent: p.parent,
                    roots: p.roots.clone(),
                    counts: p.counts.clone(),
                    cleared: p.cleared.clone(),
                }
            } else {
                Private {
                    parent,
                    roots: vec![],
                    counts: BTreeMap::new(),
                    cleared: vec![],
                }
            };
            combined.roots.extend(roots);
            combined.counts.extend(built.counts);
            self.finish(combined)
        })();
        self.storage_cpu_ns = self
            .storage_cpu_ns
            .zip(cpu.zip(crate::code_rule_dsl::sql_profile::thread_cpu_ns()))
            .map(|(total, (start, end))| total + end.saturating_sub(start));
        result
    }
    fn finish(&mut self, combined: Private) -> rusqlite::Result<i64> {
        let old = self.state(combined.parent)?.clone();
        let waves = old.waves + u32::from(!old.pending.is_empty());
        if waves > self.plan.limit {
            return Err(sql_error(format!(
                "no fixpoint in {} waves",
                self.plan.limit
            )));
        }
        let mut next = old.clone();
        next.changed.clear();
        next.empty_body.fill(false);
        for &id in &combined.cleared {
            next.empty_body[id] = true;
        }
        let mut publication = vec![];
        for (slot, root) in combined.roots {
            if !old.roots[slot].same(&root) {
                next.changed.push(slot);
                let name = &self.plan.targets[&slot];
                let c = &combined.counts[name];
                publication.push(json!({"table":name,"affected_keys":c.keys,"rows":c.rows,"strategy":c.strategy}));
                next.roots[slot] = root;
            }
        }
        for &(input, source) in &self.plan.inputs {
            if !old.roots[input].same(&old.roots[source]) {
                next.changed.push(input);
                next.roots[input] = old.roots[source].clone();
            }
        }
        for (&id, op) in &self.plan.operators {
            if old.active.contains(&id) {
                next.initialized[id] = true;
            }
            if old.active.contains(&id) || next.empty_body[id] {
                for &journal in &op.journals {
                    if next.roots[journal].len() > 0 {
                        next.changed.push(journal);
                        next.roots[journal] = next.roots[journal].empty_like();
                    }
                }
                for (_, seen, current) in &op.seen {
                    next.roots[*seen] = old.roots[*current].clone();
                }
            }
            if old.active.contains(&id) || old.ran.contains(&id) || next.empty_body[id] {
                next.roots[op.comparison.0] = old.roots[op.comparison.1].clone();
            }
        }
        next.pending.retain(|id| !old.active.contains(id));
        if !old.active.is_empty() {
            for (r, &slot) in &self.plan.frontiers {
                if next.roots[slot].len() > 0 {
                    for (id, (step, _)) in self.plan.steps.iter().enumerate() {
                        if step.reads.contains(r) {
                            next.pending.insert(id);
                        }
                    }
                }
            }
        }
        next.active = self.plan.ready(&next.pending);
        if !next.pending.is_empty() && next.active.is_empty() {
            return Err(sql_error("temporal strata have no ready step"));
        }
        next.ran = old.active;
        next.waves = waves;
        next.publication = json!(publication);
        let handle = self.states.len() as i64;
        self.states.push(next);
        Ok(handle)
    }
}

pub struct Collect(pub Arc<parking_lot::Mutex<Arena>>);
pub struct Batch {
    parent: Option<i64>,
    id: Option<i64>,
    stage: Option<Arc<Stage>>,
    buffers: Option<phase::Buffers>,
}
impl Aggregate<Batch, i64> for Collect {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Batch> {
        Ok(Batch {
            parent: None,
            id: None,
            stage: None,
            buffers: None,
        })
    }
    fn step(&self, c: &mut Context<'_>, batch: &mut Batch) -> rusqlite::Result<()> {
        let id = c.get::<i64>(0)?;
        let parent = c.get::<i64>(1)?;
        if batch.parent.is_some_and(|old| old != parent) || batch.id.is_some_and(|old| old != id) {
            return Err(sql_error("mixed retained stage references"));
        }
        if batch.stage.is_none() {
            let stage = self.0.lock().plan.stage(id)?;
            batch.buffers = Some(stage.template.buffers());
            batch.stage = Some(stage);
            batch.parent = Some(parent);
            batch.id = Some(id);
        }
        let tag = c.get::<i64>(2)?;
        if tag == -1 {
            return Ok(());
        }
        let template = &batch.stage.as_ref().unwrap().template;
        let cells = (3..3 + template.width())
            .map(|i| c.get::<Option<i64>>(i))
            .collect::<rusqlite::Result<_>>()?;
        template
            .collect(tag, cells, batch.buffers.as_mut().unwrap())
            .map_err(|e| sql_error(e.to_string()))
    }
    fn finalize(&self, _: &mut Context<'_>, batch: Option<Batch>) -> rusqlite::Result<i64> {
        let batch = batch.ok_or_else(|| sql_error("missing retained stage sentinel"))?;
        self.0.lock().complete(
            batch
                .id
                .ok_or_else(|| sql_error("missing retained stage"))?,
            batch.parent.unwrap(),
            batch.buffers.unwrap(),
        )
    }
}
