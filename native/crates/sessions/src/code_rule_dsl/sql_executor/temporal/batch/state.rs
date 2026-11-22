//! Immutable wave roots and schedule cursors for the SQL dispatcher.
use super::super::cold_loop::rows;
use super::*;

#[derive(Clone)]
pub struct Roots {
    pub rows: versions::Version,
    pub frontier: versions::Version,
    pub counts: versions::Version,
}
impl Roots {
    pub fn read(&self, column: usize) -> &versions::Version {
        match column {
            0 => &self.rows,
            1 => &self.frontier,
            _ => &self.counts,
        }
    }
}

pub struct Wave {
    pub roots: Vec<Roots>,
    pub outputs: BTreeSet<usize>,
    pub candidates: Vec<Option<versions::Version>>,
}

pub struct Arena {
    pub plan: Arc<Plan>,
    pub history: Arc<Vec<Wave>>,
    changed: BTreeSet<String>,
    limit: u32,
    handle: i64,
    pending: BTreeSet<usize>,
    active: Vec<usize>,
    schedule: Vec<usize>,
    cursor: usize,
    pub initial: Vec<Roots>,
    pub next_roots: Vec<Roots>,
    pub candidates: Vec<Option<versions::Version>>,
    pub waves: Vec<Wave>,
    pub evaluated: usize,
    pub reused: usize,
    pub failure: Option<String>,
    pub external: BTreeMap<String, incremental::External>,
    pub differences: BTreeMap<usize, incremental::Differences>,
    pub version_nodes: usize,
    pub decisions: BTreeMap<usize, Value>,
    selection: Option<(i64, usize)>,
    pub reconciled: usize,
    pub affected_heads: usize,
    pub fold_differences: BTreeMap<usize, super::folds::Changes>,
    pub reused_folds: usize,
    pub signed_folds: usize,
    pub signed_scalars: usize,
    pub fold_keys: usize,
    pub fold_decisions: BTreeMap<usize, Value>,
    pub signed_reuses: BTreeSet<usize>,
}

impl Arena {
    pub fn new(
        plan: Arc<Plan>,
        history: Arc<Vec<Wave>>,
        changed: BTreeSet<String>,
        limit: u32,
        external: BTreeMap<String, incremental::External>,
    ) -> Self {
        let initial = plan
            .outputs
            .iter()
            .map(|o| o.empty.clone())
            .collect::<Vec<_>>();
        let mut arena = Self {
            pending: (0..plan.rules.len()).collect(),
            candidates: vec![None; plan.rules.len()],
            plan,
            history,
            changed,
            limit,
            handle: 0,
            active: vec![],
            schedule: vec![],
            cursor: 0,
            next_roots: initial.clone(),
            initial,
            waves: vec![],
            evaluated: 0,
            reused: 0,
            failure: None,
            external,
            differences: BTreeMap::new(),
            version_nodes: 0,
            decisions: BTreeMap::new(),
            selection: None,
            reconciled: 0,
            affected_heads: 0,
            fold_differences: BTreeMap::new(),
            reused_folds: 0,
            signed_folds: 0,
            signed_scalars: 0,
            fold_keys: 0,
            fold_decisions: BTreeMap::new(),
            signed_reuses: BTreeSet::new(),
        };
        arena.begin();
        arena
    }

    fn begin(&mut self) {
        let waiting = self
            .pending
            .iter()
            .map(|&id| self.plan.strata[id])
            .collect::<BTreeSet<_>>();
        let blocked = self
            .plan
            .after
            .iter()
            .filter(|(_, prior)| waiting.contains(prior))
            .map(|&(stratum, _)| stratum)
            .collect::<BTreeSet<_>>();
        self.active = self
            .pending
            .iter()
            .copied()
            .filter(|&id| !blocked.contains(&self.plan.strata[id]))
            .collect();
        self.schedule.clone_from(&self.active);
        let folds = self
            .active
            .iter()
            .map(|&id| self.plan.rules[id].relation)
            .collect::<BTreeSet<_>>();
        self.schedule
            .extend(folds.into_iter().map(|id| self.plan.rules.len() + id));
        self.cursor = 0;
    }

    pub fn current(&self) -> &[Roots] {
        self.waves.last().map_or(&self.initial, |w| &w.roots)
    }

    fn prior(&self, id: usize) -> Option<&versions::Version> {
        let rule = self.plan.rules.get(id)?;
        if !rule.reusable
            || !rule.external.is_disjoint(&self.changed)
            || rule.external.contains("") && !self.changed.is_empty()
        {
            return None;
        }
        let at = self.waves.len();
        let prior = self.history.get(at)?.candidates[id].as_ref()?;
        let old = at
            .checked_sub(1)
            .map_or(&self.initial, |i| &self.history[i].roots);
        rule.reads
            .iter()
            .all(|&slot| {
                self.current()[slot / 3]
                    .read(slot % 3)
                    .same(old[slot / 3].read(slot % 3))
            })
            .then_some(prior)
    }

    pub fn check_handle(&self, handle: i64) -> rusqlite::Result<()> {
        if self.handle == handle {
            Ok(())
        } else {
            Err(sql_error("invalid SQL wave state handle"))
        }
    }

    pub fn previous_candidate(&self, id: usize) -> Option<&versions::Version> {
        self.history
            .get(self.waves.len())?
            .candidates
            .get(id)?
            .as_ref()
    }

    pub fn next(&mut self, handle: i64) -> rusqlite::Result<i64> {
        self.check_handle(handle)?;
        if self.pending.is_empty() {
            return Ok(-1);
        }
        if self.waves.len() >= self.limit as usize {
            return Err(sql_error(format!("no fixpoint in {} waves", self.limit)));
        }
        let stage = *self
            .schedule
            .get(self.cursor)
            .ok_or_else(|| sql_error("SQL wave strata have no ready rule"))?;
        if let Some((prior, choice)) = self.selection
            && prior == handle {
                return Ok(choice as i64);
            }
        let choice = if stage >= self.plan.rules.len() {
            self.fold_stage(stage - self.plan.rules.len())?
                .unwrap_or(stage)
        } else if self.prior(stage).is_some() {
            self.plan.reuse_id
        } else {
            self.incremental_stage(stage)?.unwrap_or(stage)
        };
        self.selection = Some((handle, choice));
        Ok(choice as i64)
    }

    pub fn reuse(&mut self, handle: i64) -> rusqlite::Result<i64> {
        self.check_handle(handle)?;
        let stage = self.schedule[self.cursor];
        if stage >= self.plan.rules.len() {
            return self.reuse_fold(stage - self.plan.rules.len());
        }
        let prior = self
            .prior(stage)
            .or_else(|| {
                self.signed_reuses
                    .contains(&stage)
                    .then(|| self.previous_candidate(stage))
                    .flatten()
            })
            .ok_or_else(|| sql_error("SQL wave reuse proof changed"))?
            .clone();
        self.candidates[stage] = Some(prior);
        self.reused += 1;
        self.advance()
    }

    pub fn accept(
        &mut self,
        handle: i64,
        stage: usize,
        mut rows: Vec<Vec<Option<i64>>>,
        keys: Vec<Vec<Option<i64>>>,
        counts: Vec<Vec<Option<i64>>>,
    ) -> rusqlite::Result<i64> {
        self.check_handle(handle)?;
        if self.schedule.get(self.cursor) != Some(&stage) {
            return Err(sql_error("SQL wave result arrived out of order"));
        }
        if let Some(rule) = self.plan.rules.get(stage) {
            let incremental = rule
                .incremental
                .as_ref()
                .is_some_and(|r| self.selection == Some((handle, r.expression)));
            let candidate = if incremental {
                if rule.incremental.as_ref().unwrap().scalar {
                    self.signed_scalars += 1;
                }
                self.affected_heads += keys.len();
                self.reconciled += 1;
                self.previous_candidate(stage)
                    .ok_or_else(|| sql_error("missing signed projection history"))?
                    .patched_rows(keys, rows, true)
                    .map_err(|err| sql_error(format!("signed rule {stage}: {err}")))?
            } else {
                self.evaluated += 1;
                self.previous_candidate(stage)
                    .unwrap_or(&rule.candidate)
                    .loaded(rows, true, true)
                    .map_err(sql_error)?
            };
            self.candidates[stage] = Some(candidate);
        } else {
            let id = stage - self.plan.rules.len();
            if self.selection == self.plan.fold_branches[id].map(|branch| (handle, branch)) {
                return self.accept_fold(id, rows, keys, counts);
            }
            let spec = &self.plan.outputs[id].spec;
            let v = &mut self.next_roots[id];
            if spec.widening() && !rows.is_empty() {
                let counts = rows
                    .iter_mut()
                    .map(|row| {
                        let n = row.pop().unwrap();
                        let mut count = row[..spec.keys].to_vec();
                        count.push(n);
                        count
                    })
                    .collect::<Vec<_>>();
                let keys = counts.iter().map(|row| row[..spec.keys].to_vec()).collect();
                v.counts = v
                    .counts
                    .patched_rows(keys, counts, true)
                    .map_err(sql_error)?;
            }
            if !rows.is_empty() {
                let keys = rows.iter().map(|row| row[..spec.keys].to_vec()).collect();
                v.rows = v
                    .rows
                    .patched_rows(keys, rows.clone(), true)
                    .map_err(sql_error)?;
                v.frontier = v.frontier.loaded(rows, false, false).map_err(sql_error)?;
            }
        }
        self.advance()
    }

    pub fn advance(&mut self) -> rusqlite::Result<i64> {
        self.handle += 1;
        self.cursor += 1;
        if self.cursor < self.schedule.len() {
            return Ok(self.handle);
        }
        for id in &self.active {
            self.pending.remove(id);
        }
        let mut outputs = self
            .active
            .iter()
            .map(|&id| self.plan.rules[id].relation)
            .collect::<BTreeSet<_>>();
        for (id, output) in self.plan.outputs.iter().enumerate() {
            if self.current()[id].frontier.len() > 0 {
                outputs.insert(id);
            }
            if self.next_roots[id].frontier.len() > 0 {
                self.pending.extend(output.readers.iter().copied());
            }
        }
        let next = self
            .next_roots
            .iter()
            .map(|r| Roots {
                rows: r.rows.clone(),
                frontier: r.frontier.empty_like(),
                counts: r.counts.clone(),
            })
            .collect();
        let roots = std::mem::replace(&mut self.next_roots, next);
        let candidates = std::mem::replace(&mut self.candidates, vec![None; self.plan.rules.len()]);
        self.waves.push(Wave {
            roots,
            outputs,
            candidates,
        });
        self.differences.clear();
        self.fold_differences.clear();
        self.signed_reuses.clear();
        self.begin();
        Ok(self.handle)
    }
}

impl rows::Roots for Arena {
    fn sources_at(
        &mut self,
        handle: i64,
        slot: usize,
    ) -> rusqlite::Result<Option<(usize, Vec<Arc<dyn Source>>)>> {
        self.check_handle(handle)?;
        let base = self.plan.outputs.len() * 3 + self.plan.rules.len();
        let Some(extra) = slot.checked_sub(base) else {
            return Ok(None);
        };
        let Some((input, incremental::VersionKind::Superset)) =
            self.plan.changes.get(extra).cloned()
        else {
            return Ok(None);
        };
        let (current, removed): (Arc<dyn Source>, versions::Version) = match input {
            incremental::Input::Derived(slot) => {
                let current = self.current()[slot / 3].read(slot % 3).clone();
                let minus = self.differences(slot)?.minus.clone();
                (Arc::new(current), minus)
            }
            incremental::Input::External(name) => {
                let input = &self.external[&name];
                (input.source.clone(), input.minus.clone())
            }
        };
        Ok(Some((
            current.columns().len(),
            vec![current, Arc::new(removed)],
        )))
    }
    fn root_at(&mut self, handle: i64, slot: usize) -> rusqlite::Result<versions::Version> {
        self.check_handle(handle)?;
        let count = self.plan.outputs.len() * 3;
        if slot < count {
            Ok(self.current()[slot / 3].read(slot % 3).clone())
        } else {
            let id = slot - count;
            if let Some(rule) = self.plan.rules.get(id) {
                return Ok(self.candidates[id]
                    .as_ref()
                    .unwrap_or(&rule.candidate)
                    .clone());
            }
            let (input, kind) = self
                .plan
                .changes
                .get(id - self.plan.rules.len())
                .cloned()
                .ok_or_else(|| sql_error("invalid signed batch slot"))?;
            let negative = match kind {
                incremental::VersionKind::Added => false,
                incremental::VersionKind::Removed => true,
                incremental::VersionKind::Superset => {
                    return Err(sql_error("superset requires source access"));
                }
            };
            match input {
                incremental::Input::Derived(slot) => {
                    let difference = self.differences(slot)?;
                    Ok(if negative {
                        difference.minus.clone()
                    } else {
                        difference.plus.clone()
                    })
                }
                incremental::Input::External(name) => {
                    let difference = &self.external[&name];
                    Ok(if negative {
                        difference.minus.clone()
                    } else {
                        difference.plus.clone()
                    })
                }
            }
        }
    }
    fn failure(&mut self) -> &mut Option<String> {
        &mut self.failure
    }
}
