//! Reconcile changed fold keys using the original SQL merge and widening expressions.
use super::*;

pub(super) struct Changes {
    sources: Vec<versions::Version>,
    pub equal: bool,
    pub changed_rows: usize,
}
impl Arena {
    pub(super) fn fold_changes(&mut self, id: usize) -> rusqlite::Result<Option<&Changes>> {
        if self.history.get(self.waves.len()).is_none() {
            return Ok(None);
        }
        if !self.fold_differences.contains_key(&id) {
            let prior = &self.history[self.waves.len()];
            let old = if self.waves.is_empty() {
                &self.initial
            } else {
                &self.history[self.waves.len() - 1].roots
            };
            let current = self.current();
            let mut pairs = vec![
                (old[id].rows.clone(), current[id].rows.clone()),
                (old[id].counts.clone(), current[id].counts.clone()),
            ];
            for (at, rule) in self
                .plan
                .rules
                .iter()
                .enumerate()
                .filter(|(_, r)| r.relation == id)
            {
                pairs.push((
                    prior.candidates[at]
                        .as_ref()
                        .unwrap_or(&rule.candidate)
                        .clone(),
                    self.candidates[at]
                        .as_ref()
                        .unwrap_or(&rule.candidate)
                        .clone(),
                ));
            }
            let equal = pairs.iter().all(|(a, b)| a.same(b));
            let mut sources = Vec::new();
            let mut changed_rows = 0;
            if !equal {
                for (before, after) in pairs {
                    let (plus, minus, visited) =
                        before.difference_roots(&after).map_err(sql_error)?;
                    self.version_nodes += visited;
                    for root in [plus, minus] {
                        changed_rows += root.len();
                        if root.len() > 0 {
                            sources.push(root);
                        }
                    }
                }
            }
            self.fold_differences.insert(
                id,
                Changes {
                    sources,
                    equal,
                    changed_rows,
                },
            );
        }
        Ok(self.fold_differences.get(&id))
    }
    pub(super) fn fold_sources(
        &mut self,
        handle: i64,
        id: usize,
    ) -> rusqlite::Result<Option<(usize, Vec<Arc<dyn Source>>)>> {
        self.check_handle(handle)?;
        let width = self.plan.width;
        let changes = self
            .fold_changes(id)?
            .ok_or_else(|| sql_error("missing prior fold"))?;
        Ok(Some((
            width,
            changes
                .sources
                .iter()
                .cloned()
                .map(|v| Arc::new(v) as Arc<dyn Source>)
                .collect(),
        )))
    }
    pub(super) fn fold_stage(&mut self, id: usize) -> rusqlite::Result<Option<usize>> {
        if incremental::force_full() {
            return Ok(None);
        }
        let branch = self.plan.fold_branches[id];
        let reuse_id = self.plan.reuse_id;
        let Some(changes) = self.fold_changes(id)? else {
            return Ok(None);
        };
        let equal = changes.equal;
        let changed = changes.changed_rows;
        self.fold_decisions.insert(
            id,
            json!({"changed_rows":changed,"reused":equal,"sparse":!equal&&branch.is_some()}),
        );
        Ok(if equal { Some(reuse_id) } else { branch })
    }
    pub(super) fn reuse_fold(&mut self, id: usize) -> rusqlite::Result<i64> {
        self.next_roots[id] = self.history[self.waves.len()].roots[id].clone();
        self.reused_folds += 1;
        self.advance()
    }
    pub(super) fn accept_fold(
        &mut self,
        id: usize,
        rows: Vec<Vec<Option<i64>>>,
        keys: Vec<Vec<Option<i64>>>,
        counts: Vec<Vec<Option<i64>>>,
    ) -> rusqlite::Result<i64> {
        let spec = &self.plan.outputs[id].spec;
        let keys = keys
            .into_iter()
            .map(|r| r[..spec.keys].to_vec())
            .collect::<Vec<_>>();
        self.fold_keys += keys.len();
        let prior = &self.history[self.waves.len()].roots[id];
        let rows = prior
            .rows
            .patched_rows(
                keys.clone(),
                rows.into_iter().map(|r| r[..spec.width].to_vec()).collect(),
                true,
            )
            .map_err(sql_error)?;
        let counts = if spec.widening() {
            prior
                .counts
                .patched_rows(
                    keys,
                    counts
                        .into_iter()
                        .map(|r| r[..spec.keys + 1].to_vec())
                        .collect(),
                    true,
                )
                .map_err(sql_error)?
        } else {
            self.current()[id].counts.clone()
        };
        let previous = &self.current()[id].rows;
        let old_previous = if self.waves.is_empty() {
            &self.initial
        } else {
            &self.history[self.waves.len() - 1].roots
        };
        let frontier = if previous.same(&old_previous[id].rows) && rows.same(&prior.rows) {
            prior.frontier.clone()
        } else if previous.len() == 0 {
            rows.clone()
        } else {
            let (plus, minus, visited) = previous.difference_roots(&rows).map_err(sql_error)?;
            self.version_nodes += visited;
            if !spec.lattice && minus.len() > 0 {
                return Err(sql_error("SQL fold removed preceding-wave rows"));
            }
            plus
        };
        self.next_roots[id] = state::Roots {
            rows,
            frontier,
            counts,
        };
        self.signed_folds += 1;
        self.advance()
    }
}

pub(super) fn compile(
    spec: &Relation,
    candidates: &str,
    previous: &str,
    id: usize,
    stage: usize,
) -> String {
    let key_fields = names("", spec.keys);
    let keys = if spec.keys == 0 {
        "1 AS rr_key".to_owned()
    } else {
        key_fields.clone()
    };
    let equality = |a: &str, b: &str| {
        if spec.keys == 0 {
            "1".to_owned()
        } else {
            same(a, b, spec.keys)
        }
    };
    let restricted = candidates;
    let (improved, improved_width) = compile::fold(spec, restricted, previous, id);
    let count_width = spec.keys + 1;
    let width = improved_width;
    let pad = |columns: Vec<String>| {
        columns
            .into_iter()
            .chain((0..width.saturating_sub(spec.width)).map(|_| "NULL".to_owned()))
            .collect::<Vec<_>>()
            .join(",")
    };
    let output = pad((0..spec.width).map(|c| format!("i.c{c}")).collect());
    let preceding = pad((0..spec.width).map(|c| format!("p.c{c}")).collect());
    let key_output = (0..width)
        .map(|c| {
            if c < spec.keys {
                format!("k.c{c}")
            } else {
                "NULL".into()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let sentinel = (0..width)
        .map(|c| format!("NULL AS c{c}"))
        .collect::<Vec<_>>()
        .join(",");
    let columns = names("", width);
    let mut arms = vec![
        format!("SELECT 0 AS rr_present,{sentinel}"),
        format!("SELECT 2,{key_output} FROM __rr_fold_keys k"),
        format!("SELECT 1,{output} FROM __rr_improved i"),
        format!(
            "SELECT 1,{preceding} FROM __rr_fold_keys k CROSS JOIN {previous} p WHERE {} AND NOT EXISTS(SELECT 1 FROM __rr_improved i WHERE {})",
            equality("p", "k"),
            equality("i", "k")
        ),
    ];
    if spec.widening() {
        let counts = compile::source(ROWS, id * 3 + 2, count_width);
        let count_fields = (0..width)
            .map(|c| {
                if c < count_width {
                    format!("q.c{c}")
                } else {
                    "NULL".into()
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let next_count = (0..width)
            .map(|c| {
                if c < spec.keys {
                    format!("i.c{c}")
                } else if c == spec.keys {
                    format!("i.c{}", spec.width)
                } else {
                    "NULL".into()
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        arms.push(format!("SELECT 3,{count_fields} FROM __rr_fold_keys k CROSS JOIN {counts} q WHERE {} AND NOT EXISTS(SELECT 1 FROM __rr_improved i WHERE {})",equality("q","k"),equality("i","k")));
        arms.push(format!("SELECT 3,{next_count} FROM __rr_improved i"));
    }
    let improved_columns = names("", improved_width);
    format!(
        "(WITH __rr_fold_keys AS MATERIALIZED (SELECT DISTINCT {keys} FROM {FOLD_CHANGES} WHERE rr_state={} AND rr_slot={id}),__rr_improved({improved_columns}) AS MATERIALIZED ({improved}) SELECT {COLLECT}({},{stage},rr_present,{columns}) FROM ({}))",
        compile::PARENT,
        compile::PARENT,
        arms.join(" UNION ALL ")
    )
}
