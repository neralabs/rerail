//! Immutable wave roots and schedule cursors for the SQL dispatcher.
use super::super::cold_loop::rows;
use super::*;

#[derive(Clone)]
pub(super) struct Roots {
    pub rows: versions::Version,
    pub frontier: versions::Version,
    pub counts: versions::Version,
}
impl Roots {
    fn read(&self, column: usize) -> &versions::Version {
        match column {
            0 => &self.rows,
            1 => &self.frontier,
            _ => &self.counts,
        }
    }
}

pub(super) struct Wave {
    pub roots: Vec<Roots>,
    pub outputs: BTreeSet<usize>,
    candidates: Vec<Option<versions::Version>>,
}

pub(super) struct Arena {
    plan: Arc<Plan>,
    history: Arc<Vec<Wave>>,
    changed: BTreeSet<String>,
    limit: u32,
    handle: i64,
    pending: BTreeSet<usize>,
    active: Vec<usize>,
    schedule: Vec<usize>,
    cursor: usize,
    initial: Vec<Roots>,
    next_roots: Vec<Roots>,
    candidates: Vec<Option<versions::Version>>,
    pub waves: Vec<Wave>,
    pub evaluated: usize,
    pub reused: usize,
    pub failure: Option<String>,
}

impl Arena {
    pub fn new(
        plan: Arc<Plan>,
        history: Arc<Vec<Wave>>,
        changed: BTreeSet<String>,
        limit: u32,
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

    fn current(&self) -> &[Roots] {
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

    fn check_handle(&self, handle: i64) -> rusqlite::Result<()> {
        if self.handle == handle {
            Ok(())
        } else {
            Err(sql_error("invalid SQL wave state handle"))
        }
    }

    pub fn next(&self, handle: i64) -> rusqlite::Result<i64> {
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
        Ok(if self.prior(stage).is_some() {
            self.plan.reuse_id
        } else {
            stage
        } as i64)
    }

    pub fn reuse(&mut self, handle: i64) -> rusqlite::Result<i64> {
        self.check_handle(handle)?;
        let stage = self.schedule[self.cursor];
        let prior = self
            .prior(stage)
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
    ) -> rusqlite::Result<i64> {
        self.check_handle(handle)?;
        if self.schedule.get(self.cursor) != Some(&stage) {
            return Err(sql_error("SQL wave result arrived out of order"));
        }
        if let Some(rule) = self.plan.rules.get(stage) {
            self.candidates[stage] = Some(
                rule.candidate
                    .loaded(rows, true, false)
                    .map_err(sql_error)?,
            );
            self.evaluated += 1;
        } else {
            let id = stage - self.plan.rules.len();
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

    fn advance(&mut self) -> rusqlite::Result<i64> {
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
        let mut candidates =
            std::mem::replace(&mut self.candidates, vec![None; self.plan.rules.len()]);
        for (candidate, rule) in candidates.iter_mut().zip(&self.plan.rules) {
            if !rule.reusable {
                *candidate = None;
            }
        }
        self.waves.push(Wave {
            roots,
            outputs,
            candidates,
        });
        self.begin();
        Ok(self.handle)
    }
}

impl rows::Roots for Arena {
    fn root_at(&mut self, handle: i64, slot: usize) -> rusqlite::Result<versions::Version> {
        self.check_handle(handle)?;
        let count = self.plan.outputs.len() * 3;
        if slot < count {
            Ok(self.current()[slot / 3].read(slot % 3).clone())
        } else {
            let id = slot - count;
            Ok(self.candidates[id]
                .as_ref()
                .unwrap_or(&self.plan.rules[id].candidate)
                .clone())
        }
    }
    fn failure(&mut self) -> &mut Option<String> {
        &mut self.failure
    }
}
