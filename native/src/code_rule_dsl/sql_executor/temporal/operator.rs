//! Retained wave steps. Inactive caches accumulate net changes without evaluating their queries.
use super::*;

#[derive(Clone)]
pub(super) struct Operator {
    pub run: session::Execution,
    pub result: String,
    pub improvements: String,
    pub active: bool,
    pub comparison: versions::Version,
    spec: Relation,
    ready: bool,
    pub(super) seen: BTreeMap<String, versions::Version>,
    pub(super) pending: BTreeMap<String, Pending>,
}
#[derive(Clone)]
pub(super) struct Pending {
    plus: String,
    minus: String,
    width: usize,
}
impl Pending {
    fn add(&self, w: &mut Work<'_>, d: &Delta) -> Result<()> {
        let cols = names("", self.width);
        // Compose actual set transitions. Opposite changes cancel before evaluation.
        for (incoming, keep, cancel) in [
            (&d.plus, &self.plus, &self.minus),
            (&d.minus, &self.minus, &self.plus),
        ] {
            let remaining = w.table(self.width)?;
            w.db.execute(&format!("INSERT INTO {remaining} SELECT d.* FROM {incoming} d WHERE NOT EXISTS(SELECT 1 FROM {} c WHERE {})",quote(cancel),same("c","d",self.width)),[]).map_err(error)?;
            w.db.execute(
                &format!(
                    "DELETE FROM {} WHERE ({cols}) IN(SELECT {cols} FROM {incoming})",
                    quote(cancel)
                ),
                [],
            )
            .map_err(error)?;
            w.db.execute(
                &format!(
                    "INSERT OR IGNORE INTO {} SELECT * FROM {remaining}",
                    quote(keep)
                ),
                [],
            )
            .map_err(error)?;
        }
        Ok(())
    }
    pub(super) fn clear(&self, store: &versions::Store) -> Result<()> {
        for name in [&self.plus, &self.minus] {
            let version = store.version(name);
            if version.len() > 0 {
                store.replace(name, version.empty_like())?;
            }
        }
        Ok(())
    }
}
fn bind(query: &mut Query, names: &BTreeMap<String, String>) {
    for block in query
        .ctes
        .iter_mut()
        .flat_map(|c| &mut c.arms)
        .chain([&mut query.body])
    {
        block.relations_mut(&mut |r| {
            let Some(name) = read(r).and_then(|name| names.get(&name)) else {
                return;
            };
            *r = Rel::Bound {
                rel: name.clone(),
                table: quote(name),
            };
        });
    }
}

fn order(query: &mut Query, db: &rusqlite::Connection) {
    for block in query
        .ctes
        .iter_mut()
        .flat_map(|c| &mut c.arms)
        .chain([&mut query.body])
    {
        if block.from.len() < 2 {
            continue;
        }
        let at = block
            .from
            .iter()
            .position(|i| matches!(i.relation, Rel::Cte(_)))
            .unwrap_or_else(|| {
                block
                    .from
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, i)| {
                        i.relation
                            .program_relation()
                            .and_then(|r| super::super::super::facts::vtab::bound_source(db, r))
                            .map_or(1000, |s| s.estimated_len())
                    })
                    .unwrap()
                    .0
            });
        let mut q = Query {
            ctes: vec![],
            body: block.clone(),
        };
        // Native aliases have current estimates, but exhaustive SQL join planning repeats
        // for every new cache. Bind the smallest source, then connected equality probes.
        maintenance::anchor(&mut q, at);
        *block = q.body;
    }
}
impl Operator {
    pub fn create(
        w: &mut Work<'_>,
        store: &versions::Store,
        trace: &Trace,
        id: usize,
        prefix: &str,
        inputs: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let (step, query) = &trace.steps[id];
        let spec = &trace.relations[&step.output];
        let result = format!("{prefix}_o{id}");
        for (name, keys) in [
            (result.clone(), spec.keys),
            (format!("new_{result}"), spec.width),
            (format!("delta_{result}"), spec.keys),
        ] {
            store.create(w.db, &name, versions::Schema::integers(spec.width, keys))?;
        }
        let mut query = query.clone();
        bind(&mut query, inputs);
        let mut emitted = (*trace.emitted).clone();
        emitted.setup.clear();
        emitted.views.clear();
        let dependencies = query
            .reads()
            .iter()
            .filter_map(|r| r.program_relation().map(str::to_owned))
            .collect::<BTreeSet<_>>();
        emitted.inputs = dependencies.iter().cloned().collect();
        emitted.program.relations = vec![(result.clone(), spec.width)];
        emitted.program.partitions.clear();
        emitted.program.lattices = if spec.lattice {
            vec![(result.clone(), spec.keys, spec.top)]
        } else {
            vec![]
        };
        emitted.program.lattice_operations = spec
            .ops
            .as_ref()
            .map(|ops| {
                let mut ops = ops.clone();
                // This cache is a pure result, rather than an ascending recursive sequence.
                ops.after = None;
                ops.widen = None;
                BTreeMap::from([(result.clone(), ops)])
            })
            .unwrap_or_default();
        emitted.program.column_types = trace
            .emitted
            .program
            .column_types
            .get(&step.output)
            .map(|t| BTreeMap::from([(result.clone(), t.clone())]))
            .unwrap_or_default();
        emitted.program.steps = vec![schedule::Step {
            output: result.clone(),
            stratum: 1,
            reads: dependencies.iter().cloned().collect(),
            sql: format!("INSERT OR IGNORE INTO new_{result} {}", query.render()),
        }];
        emitted.queries = vec![Some(query)];
        let run = session::Execution {
            inputs: dependencies,
            objects: vec![
                (result.clone(), true),
                (format!("new_{result}"), true),
                (format!("delta_{result}"), true),
            ],
            emitted: Arc::new(emitted),
            counts: BTreeMap::from([(result.clone(), 0)]),
            outputs: vec![result.clone()],
            max_waves: trace.limit,
            reductions: None,
            recursive: BTreeMap::new(),
            temporal: BTreeMap::new(),
        };
        let mut seen = BTreeMap::new();
        let mut pending = BTreeMap::new();
        for (at, name) in run.inputs.iter().enumerate() {
            if store.contains(name) {
                seen.insert(name.clone(), store.version(name));
                continue;
            }
            let width =
                w.db.prepare(&format!("SELECT * FROM {} LIMIT 0", quote(name)))
                    .map_err(error)?
                    .column_count();
            let p = Pending {
                plus: format!("{result}_p{at}"),
                minus: format!("{result}_m{at}"),
                width,
            };
            for table in [&p.plus, &p.minus] {
                store.create(w.db, table, versions::Schema::integers(width, width))?;
            }
            pending.insert(name.clone(), p);
        }
        let improvements = format!("{result}_improve");
        store.create(
            w.db,
            &improvements,
            versions::Schema::integers(spec.width, spec.keys),
        )?;
        Ok(Self {
            run,
            result,
            improvements,
            active: false,
            comparison: store.version(&inputs[&step.output]),
            spec: spec.clone(),
            ready: false,
            seen,
            pending,
        })
    }
    fn initialize(&mut self, w: &mut Work<'_>, store: &versions::Store) -> Result<()> {
        order(
            Arc::make_mut(&mut self.run.emitted).queries[0]
                .as_mut()
                .unwrap(),
            w.db,
        );
        let query = self.run.emitted.queries[0].as_ref().unwrap();
        let sql = if self.spec.lattice {
            let keys = names("", self.spec.keys);
            let value = format!("c{}", self.spec.width - 1);
            format!(
                "SELECT {keys},{} FROM ({}) GROUP BY {keys}",
                self.spec.aggregate(&value),
                query.render()
            )
        } else {
            query.render()
        };
        w.db.execute(
            &format!("INSERT OR IGNORE INTO {} {sql}", quote(&self.result)),
            [],
        )
        .map_err(|e| error(format!("{}: initial temporal result: {e}", self.result)))?;
        self.run.counts.insert(
            self.result.clone(),
            store.version(&self.result).len() as u64,
        );
        walks::prepare(w, &mut self.run, Some(store))
            .map_err(|e| error(format!("{}: temporal walks: {e}", self.result)))?;
        let mut potential = self.run.inputs.clone();
        potential.extend(
            self.run
                .emitted
                .program
                .relations
                .iter()
                .map(|(r, _)| r.clone()),
        );
        reductions::prepare(w, &mut self.run, &mut potential, Some(store))
            .map_err(|e| error(format!("{}: temporal support: {e}", self.result)))?;
        for (output, _) in &self.run.emitted.program.relations {
            let recursive = self
                .run
                .emitted
                .program
                .steps
                .iter()
                .zip(&self.run.emitted.queries)
                .any(|(s, q)| {
                    s.output == *output
                        && q.as_ref().is_some_and(|q| {
                            q.reads()
                                .iter()
                                .any(|r| r.program_relation() == Some(output))
                        })
                });
            if !recursive
                && !self
                    .run
                    .reductions
                    .as_ref()
                    .is_some_and(|r| r.get(output).is_some_and(|s| s.ready))
            {
                return Err(Error::new(
                    napi::Status::InvalidArg,
                    format!("{UNSUPPORTED}{output}"),
                ));
            }
        }
        self.ready = true;
        Ok(())
    }
    pub fn update(
        &mut self,
        w: &mut Work<'_>,
        store: &versions::Store,
        changes: &BTreeMap<String, Delta>,
        active: bool,
        preparing: bool,
        limit: u32,
    ) -> Result<bool> {
        for (r, p) in &self.pending {
            if let Some(d) = changes.get(r).filter(|d| d.inserted + d.deleted > 0) {
                p.add(w, d)?;
            }
        }
        if self.ready && self.clear_empty_body(store)? {
            return Ok(true);
        }
        if !active {
            return Ok(false);
        }
        if !self.ready {
            if !preparing {
                return Err(error(
                    "unprepared temporal step became active during an edit",
                ));
            }
            self.initialize(w, store)?;
        } else {
            let mut changes = BTreeMap::new();
            for (r, v) in &self.seen {
                let now = store.version(r);
                if !v.same(&now) {
                    changes.insert(r.clone(), delta(w, v, &now)?);
                }
            }
            for (r, p) in &self.pending {
                let (inserted, deleted) = (
                    store.version(&p.plus).len() as u64,
                    store.version(&p.minus).len() as u64,
                );
                if inserted + deleted > 0 {
                    changes.insert(
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
            if !changes.is_empty() {
                let report = maintenance::maintain(w, &mut self.run, &mut changes, limit)?;
                if report["refreshed_strata"] != json!([]) {
                    return Err(error("temporal operator unexpectedly refreshed a stratum"));
                }
            }
        }
        for (r, v) in &mut self.seen {
            *v = store.version(r);
        }
        for p in self.pending.values() {
            p.clear(store)?;
        }
        Ok(false)
    }
    pub fn objects(&self) -> impl Iterator<Item = (String, bool)> + '_ {
        self.run
            .objects
            .iter()
            .cloned()
            .chain([(self.improvements.clone(), true)])
            .chain(
                self.pending
                    .values()
                    .flat_map(|p| [(p.plus.clone(), true), (p.minus.clone(), true)]),
            )
    }
}
