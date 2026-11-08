//! Atomic batches of fact changes and retained-result maintenance.
use super::maintenance::{self, Delta, Work};
use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    name: String,
    #[serde(default)]
    insert: Vec<Vec<Value>>,
    #[serde(default)]
    delete: Vec<Vec<Value>>,
}
pub(super) struct Prepared {
    pub name: String,
    pub source: Arc<dyn Source>,
    pub live: Option<Arc<live::Live>>,
    pub plus: Vec<Vec<i64>>,
    pub minus: Vec<Vec<i64>>,
    pub migrated: bool,
}

pub(super) fn contains(source: &dyn Source, row: &[i64]) -> bool {
    let col = (0..row.len())
        .min_by_key(|c| source.estimated_matches(*c, false))
        .unwrap();
    let rows = source
        .lookup(col, &[row[col]])
        .unwrap_or_else(|| (0..source.len()).collect());
    rows.into_iter().any(|i| {
        row.iter()
            .enumerate()
            .all(|(c, v)| source.value(i, c) == *v)
    })
}

impl CodeRuleSqlExecutor {
    pub(super) fn change_facts(&mut self, json: String, max_waves: Option<u32>) -> Result<String> {
        self.db.ensure_idle().map_err(error)?;
        if !self.db.is_autocommit() {
            return Err(error("fact changes require an autocommit session"));
        }
        let patches: Vec<Patch> = serde_json::from_str(&json).map_err(error)?;
        let names: BTreeSet<&str> = patches
            .iter()
            .flat_map(|p| p.insert.iter().chain(&p.delete))
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let names: Vec<&str> = names.into_iter().collect();
        let atoms = self.store.intern_atoms(&names).map_err(error)?;
        let symbols: BTreeMap<_, _> = names.into_iter().zip(atoms).collect();
        let mut wanted = BTreeMap::<String, (BTreeSet<Vec<i64>>, BTreeSet<Vec<i64>>)>::new();
        for patch in &patches {
            if !self.mutable_inputs.contains(&patch.name) {
                return Err(error(format!(
                    "{}: changes require a relation bound with bindRelation",
                    patch.name
                )));
            }
            let (arity, types) = &self.input_shapes[&patch.name];
            let convert = |rows: &[Vec<Value>]| -> Result<BTreeSet<Vec<i64>>> {
                rows.iter()
                    .map(|r| {
                        if r.len() != *arity {
                            return Err(error("changed fact row has the wrong logical arity"));
                        }
                        if *arity == 0 {
                            return Ok(vec![1]);
                        }
                        r.iter()
                            .enumerate()
                            .map(|(i, v)| {
                                wire::fact(&symbols, v, types.as_ref().map(|t| t[i].as_str()))
                            })
                            .collect()
                    })
                    .collect()
            };
            let entry = wanted.entry(patch.name.clone()).or_default();
            entry.0.extend(convert(&patch.insert)?);
            entry.1.extend(convert(&patch.delete)?);
        }
        let mut prepared = Vec::new();
        for (name, (insert, delete)) in wanted {
            let source = self.inputs[&name].as_ref();
            if !insert.iter().any(|r| !contains(source, r))
                && !delete
                    .iter()
                    .any(|r| !insert.contains(r) && contains(source, r))
            {
                continue;
            }
            let migrated = !self.live_inputs.contains_key(&name);
            let live = self
                .live_inputs
                .get(&name)
                .cloned()
                .unwrap_or_else(|| Arc::new(live::Live::from_source(source)));
            let (plus, minus) = live.changes(&insert, &delete);
            prepared.push(Prepared {
                name,
                source: live.clone(),
                live: Some(live),
                plus,
                minus,
                migrated,
            });
        }
        self.apply_prepared(prepared, max_waves)
    }

    pub(super) fn apply_prepared(
        &mut self,
        prepared: Vec<Prepared>,
        max_waves: Option<u32>,
    ) -> Result<String> {
        if prepared.is_empty() {
            return Ok(
                json!({"inserted_facts":0,"deleted_facts":0,"programs":[],"migrated_inputs":[],"initializations":[]})
                    .to_string(),
            );
        }
        let changed = prepared
            .iter()
            .filter(|p| !p.plus.is_empty() || !p.minus.is_empty())
            .map(|p| p.name.clone())
            .collect();
        let affected = self.affected(&changed);
        let mut executions = self.executions.clone();
        let mut work = Work::new(&self.db, &self.scratch);
        work.versions = Some(&self.versions);
        work.profiler = self.db.profiler();
        let mut touched = Vec::new();
        let limits = self.limits.clone();
        self.db
            .execute_batch("SAVEPOINT sql_fact_changes")
            .map_err(error)?;
        if let Err(e) = self.versions.begin() {
            let _ = self.db.execute_batch("RELEASE sql_fact_changes");
            return Err(e);
        }
        let result = (|| -> Result<Value> {
            let _budget = limits.enter(&self.db)?;
            let mut deltas = BTreeMap::new();
            for p in &prepared {
                touched.push(p);
                if p.plus.is_empty() && p.minus.is_empty() {
                    continue;
                }
                let width = p.source.columns().len();
                let plus = work.readonly_rows(width, &p.plus)?;
                let minus = work.readonly_rows(width, &p.minus)?;
                if let Some(live) = &p.live {
                    live.apply(&p.plus, &p.minus);
                }
                deltas.insert(
                    p.name.clone(),
                    Delta {
                        plus,
                        minus,
                        inserted: p.plus.len() as u64,
                        deleted: p.minus.len() as u64,
                    },
                );
            }
            self.db
                .bind_many(prepared.iter().map(|p| (p.name.as_str(), p.source.clone())))
                .map_err(error)?;
            super::super::logic::native::forget_inputs(&self.db);
            let mut reports = Vec::new();
            for i in affected {
                let run = &mut executions[i];
                if !run
                    .inputs
                    .iter()
                    .any(|name| deltas.get(name).is_some_and(|d| d.inserted + d.deleted > 0))
                {
                    continue;
                }
                let limit = max_waves.unwrap_or(run.max_waves);
                let mut report = maintenance::maintain(&mut work, run, &mut deltas, limit)?;
                report["program"] = json!(i);
                report["migrated_views"] = json!(0);
                reports.push(report);
            }
            if !self.stats_ready.get() {
                self.db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS wave_stats(tbl TEXT PRIMARY KEY, rows INTEGER NOT NULL) WITHOUT ROWID").map_err(error)?;
            }
            {
                let mut rows = Vec::new();
                for run in &executions {
                    for (name, count) in &run.counts {
                        rows.push(vec![
                            SqlValue::Text(name.clone()),
                            SqlValue::Integer(*count as i64),
                        ]);
                    }
                }
                for p in &prepared {
                    rows.push(vec![
                        SqlValue::Text(p.name.clone()),
                        SqlValue::Integer(p.source.estimated_len() as i64),
                    ]);
                }
                plan::insert_many(
                    &self.db,
                    "INSERT OR REPLACE INTO wave_stats(tbl,rows)",
                    rows,
                )
                .map_err(error)?;
            }
            work.cleanup()?;
            self.db
                .execute_batch("RELEASE sql_fact_changes")
                .map_err(error)?;
            Ok(
                json!({"inserted_facts":prepared.iter().map(|p|p.plus.len()).sum::<usize>(),"deleted_facts":prepared.iter().map(|p|p.minus.len()).sum::<usize>(),"programs":reports,"initializations":[],"scratch_tables_created":work.created,"version_nodes":work.version_nodes,"migrated_inputs":prepared.iter().filter(|p|p.migrated).map(|p|p.name.clone()).collect::<Vec<_>>()}),
            )
        })();
        match result {
            Ok(report) => {
                self.stats_ready.set(true);
                self.control_ready.set(true);
                self.versions.commit();
                self.scratch = work.pool;
                for p in prepared {
                    self.inputs.insert(p.name.clone(), p.source.clone());
                    if let Some(live) = p.live {
                        self.live_inputs.insert(p.name, live);
                    }
                }
                self.generated = executions
                    .iter()
                    .flat_map(|e| e.objects.iter().cloned())
                    .collect();
                self.executions = executions;
                let stable_phases = !self.executions.is_empty()
                    && self.executions.iter().all(|run| {
                        run.temporal.len() == 1
                            && run
                                .temporal
                                .get(&0)
                                .is_some_and(|trace| trace.stable_phases(&run.emitted.program))
                    });
                if stable_phases {
                    self.db.reset_execution_storage();
                } else {
                    self.db.reset_execution();
                }
                Ok(report.to_string())
            }
            Err(e) => {
                let native_rollback = self.versions.rollback(&self.db);
                let _ = self
                    .db
                    .execute_batch("ROLLBACK TO sql_fact_changes; RELEASE sql_fact_changes");
                for p in touched.into_iter().rev() {
                    if let Some(live) = &p.live {
                        live.apply(&p.minus, &p.plus);
                    }
                    let _ = self.db.bind(&p.name, Arc::clone(&self.inputs[&p.name]));
                }
                self.db.reset_execution();
                native_rollback?;
                Err(e)
            }
        }
    }
}
