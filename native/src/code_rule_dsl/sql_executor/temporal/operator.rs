//! Retained wave steps. Inactive caches accumulate net changes without evaluating their queries.
use super::*;

#[derive(Clone)]
pub(super) struct Operator {
    pub logical_analysis: Arc<Value>,
    pub run: session::Execution,
    pub result: String,
    pub improvements: String,
    pub active: bool,
    pub(super) empty_body: bool,
    pub comparison: versions::Version,
    pub(super) ready: bool,
    pub(super) phase_eligible: bool,
    pub(super) phase_arms: usize,
    pub(super) sql_kernel: bool,
    pub(super) opaque: Option<opaque::Plan>,
    pub(super) seen: BTreeMap<String, versions::Version>,
    pub(super) pending: BTreeMap<String, Pending>,
}
#[derive(Clone)]
pub(super) struct Pending {
    pub(super) plus: String,
    pub(super) minus: String,
    width: usize,
}
impl Pending {
    pub(super) fn add(&self, w: &mut Work<'_>, d: &Delta) -> Result<()> {
        if let Some(store) = w.versions {
            let operation = w.profiler.map(|p| p.operation("native journal composition", ("sql-extension",0),0,
                vec![("insertions".into(),d.inserted as i64),("deletions".into(),d.deleted as i64)]));
            if store.compose(&self.plus, &self.minus, &d.plus, &d.minus)? {
                if let Some(operation) = operation { operation.finish(store.version(&self.plus).len()+store.version(&self.minus).len()); }
                return Ok(());
            }
        }
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

pub(super) fn order(query: &mut Query, db: &rusqlite::Connection) {
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
    pub fn ready(&self) -> bool {
        self.ready
    }
    pub fn adopt(&mut self, template: &Self) {
        // Preserve this frame's journals: they contain all external edits since the
        // initial facts used by the immutable template. Native input roots reconcile
        // through `seen`. Improvements are rebuilt from the previously empty result.
        self.run = template.run.clone();
        self.seen = template.seen.clone();
        self.ready = true;
    }
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
        let opaque=if query.is_none() {
            if !trace.producers.contains_key(&step.output) {return Err(error("unchecked retained step is not a SQL producer"));}
            Some(opaque::Plan::new(step,inputs)?)
        } else {None};
        let sql_kernel = !spec.lattice && query.as_ref().is_some_and(walks::kernel);
        let result = format!("{prefix}_o{id}");
        let mut query = query.clone();
        if let Some(query)=&mut query {
            bind(query, inputs);
            order(query, w.db);
        }
        let mut emitted = (*trace.emitted).clone();
        emitted.setup.clear();
        emitted.views.clear();
        let dependencies = if let Some(query)=&query {
            query.reads().iter().filter_map(|r|r.program_relation().map(str::to_owned)).collect::<BTreeSet<_>>()
        } else {
            opaque.as_ref().unwrap().inputs.values().cloned()
                .chain(trace.producers[&step.output].iter().cloned()).collect()
        };
        let mut properties = crate::code_rule_dsl::logic::analysis::ProgramAnalysis::default();
        for relation in &dependencies {
            let original = inputs.iter().find(|(_, table)| *table == relation).map_or(relation.as_str(), |(logical, _)| logical.as_str());
            let original = original.strip_prefix("delta_").unwrap_or(original);
            if let Some(keys) = trace.emitted.analysis.relation_keys.get(original) {
                properties.relation_keys.insert(relation.clone(), keys.clone());
            }
        }
        emitted.analysis = Arc::new(properties);
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
            sql: format!("INSERT OR IGNORE INTO new_{result} {}", query.as_ref().map(Query::render).unwrap_or_else(||opaque.as_ref().unwrap().select.clone())),
        }];
        emitted.queries = vec![query];
        let mut run = session::Execution {
            loop_analyses:Arc::new(vec![]),
            logical_relations: Arc::new(emitted.program.relations.clone()),
            inputs: dependencies,
            objects: vec![(result.clone(), true)],
            emitted: Arc::new(emitted),
            counts: BTreeMap::from([(result.clone(), 0)]),
            outputs: vec![result.clone()],
            max_waves: trace.limit,
            reductions: None,
            recursive: BTreeMap::new(),
            temporal: BTreeMap::new(),
            // A temporal trace's one step is a rule's query, not a producer's.
            program: Arc::new(crate::code_rule_dsl::logic::front::Program::default()),
            producers: BTreeMap::new(),
        };
        let planned = if sql_kernel || opaque.is_some() { reductions::pool::Plans::default() } else { reductions::planned(w.db, &run, true)? };
        let phase_eligible = planned.get(&result).is_some_and(|s| s.common_phase().proven);
        let phase_arms = planned.get(&result).map_or(0, |s| s.phase_arms());
        // This wrapper never executes a cold INSERT frontier. It initializes
        // from retained support, then publishes native patches from SQL results.
        store.create_readonly(w.db, &result, versions::Schema::integers(spec.width, spec.keys))?;
        // An acyclic successor probes this cache as its current input. Keep
        // the same access paths as the ordinary frozen relation facade.
        let staged_input = phase_eligible && trace.stage_decisions.get(&step.output).is_some_and(|d| d.proven);
        store.restrict_indexes(&result, if staged_input { &spec.indexes } else { &[] })?;
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
                store.create_readonly(w.db, table, versions::Schema::integers(width, width))?;
                store.restrict_indexes(table, &[])?;
            }
            pending.insert(name.clone(), p);
        }
        let improvements = format!("{result}_improve");
        store.create_readonly(
            w.db,
            &improvements,
            versions::Schema::integers(spec.width, spec.keys),
        )?;
        store.restrict_indexes(&improvements, &[])?;
        run.reductions = (!planned.is_empty()).then_some(planned.clone());
        Ok(Self {
            logical_analysis: trace.frames.first().and_then(|f|f.operators.get(id)).map(|op|Arc::clone(&op.logical_analysis)).unwrap_or_else(||Arc::new(if opaque.is_some() {
                json!({"execution":"opaque SQL producer","dependency_gate":step.stratum,"resolved_reads":step.reads,"external_inputs":trace.producers[&step.output],"invalidation":"Changed input versions or pending external journals","visibility":"Publish frozen SQL-owned dependencies before the producer SELECT; avoid duplicate final publication","storage":"One retained result root; checked neighbors retain their own maintenance"})
            } else if sql_kernel { json!({"execution":"recursive SQL kernel","proof":"Generated walk CTEs and allowlisted pure expressions; set output with original SQL UNION and NULL semantics","invalidation":"Equal input versions and empty journals skip evaluation","limits":"SQL step, time and memory guards; outer Datalog wave limits remain unchanged","storage":"One result root; no lifted walk or support tables"}) } else {json!(planned.values().map(reductions::Retained::explain).collect::<Vec<_>>())})),
            run,
            result,
            improvements,
            active: false,
            empty_body: false,
            comparison: store.version(&inputs[&step.output]),
            ready: false,
            phase_eligible,
            phase_arms,
            sql_kernel,
            opaque,
            seen,
            pending,
        })
    }
    fn initialize(&mut self, w: &mut Work<'_>, store: &versions::Store) -> Result<()> {
        if self.opaque.is_some() {
            self.evaluate_kernel(w,store)?;
            self.ready=true;
            return Ok(());
        }
        order(
            Arc::make_mut(&mut self.run.emitted).queries[0]
                .as_mut()
                .unwrap(),
            w.db,
        );
        if self.sql_kernel {
            self.evaluate_kernel(w, store)?;
            self.ready = true;
            return Ok(());
        }
        if self.run.emitted.queries[0].as_ref().unwrap().ctes.is_empty() {
            let mut plans = reductions::planned(w.db, &self.run, true)?;
            if let Some(state) = plans.get_mut(&self.result).filter(|s| s.isolated_phase().proven) {
                let _context = w.profile_scope(||json!({"stage":"isolated checked support initialization","logicalAnalysis":state.explain()}));
                state.declare_phase(w.db, &mut self.run.objects, store)?;
                let mut phase = super::super::phase::Phase::default();
                state.append_phase(w, &BTreeMap::new(), &mut phase, true)?;
                phase.execute(w)?;
                state.ready = true;
                self.run.counts.insert(self.result.clone(), store.version(&self.result).len() as u64);
                self.run.reductions = Some(plans);
                self.ready = true;
                return Ok(());
            }
        }
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
        let state = &self.run.reductions.as_ref().unwrap()[&self.result];
        let sql = state.initial_result();
        let n = store.load_select(w.db, &self.result, &sql, true)?;
        self.run.counts.insert(self.result.clone(), n as u64);
        self.ready = true;
        Ok(())
    }
    fn evaluate_kernel(&mut self, w: &mut Work<'_>, store: &versions::Store) -> Result<()> {
        let _context = w.profile_scope(|| json!({"stage":if self.opaque.is_some(){"opaque SQL producer"}else{"recursive SQL kernel"},"logicalAnalysis":self.logical_analysis.as_ref()}));
        let query = self.opaque.as_ref().map(|p|p.select.clone()).unwrap_or_else(||self.run.emitted.queries[0].as_ref().unwrap().render());
        // Match the original INSERT OR IGNORE: a NULL head does not publish a row.
        let count = crate::code_rule_dsl::logic::lanes::sql_executor().local(
            rerail_accel::Lane::Sql,
            if self.opaque.is_some() {"SQL opaque producer"} else {"SQL recursive kernel"},
            0.0,|| store.select_available(w.db, &self.result, &query, true, true),
        )?;
        self.run.counts.insert(self.result.clone(), count as u64);
        Ok(())
    }
    pub(super) fn append_kernel(&self, phase: &mut super::super::phase::Phase, width: usize) -> String {
        phase.owner_only=true;
        let query = self.run.emitted.queries[0].as_ref().unwrap();
        let evaluated = phase.checked_relation(width, query);
        let present = (0..width).map(|c|format!("c{c} IS NOT NULL")).collect::<Vec<_>>().join(" AND ");
        // Preserve INSERT OR IGNORE's NULL-head behavior before bulk validation.
        phase.view(width, &format!("SELECT * FROM {evaluated} WHERE {present}"))
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
        if self.ready && self.clear_empty_body(w.db,store)? {
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
        } else if self.sql_kernel || self.opaque.is_some() {
            if self.seen.iter().any(|(r,v)| !v.same(&store.version(r)))
                || self.pending.values().any(|p| store.version(&p.plus).len() + store.version(&p.minus).len() > 0) {
                self.evaluate_kernel(w, store)?;
            }
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
                let dense = w.preparing
                    && changes.values().any(|d| {
                        d.inserted + d.deleted >= 512
                            && (d.inserted + d.deleted)
                                >= (self.run.counts.get(&self.result).copied().unwrap_or(0) / 2)
                    })
                    && self.run.emitted.program.relations.len() == 1
                    && self
                        .run
                        .reductions
                        .as_ref()
                        .is_some_and(|r| r.values().all(|s| s.rebuildable()));
                if dense {
                    let _reuse = store.reuse_owned(&self.run.objects);
                    for (name, _) in &self.run.objects {
                        let old = store.version(name);
                        store.replace(name, old.empty_like())?;
                    }
                    self.run.objects.truncate(3);
                    self.run.reductions = None;
                    self.initialize(w, store)?;
                } else {
                    let report = maintenance::maintain(w, &mut self.run, &mut changes, limit)?;
                    w.reducer_groups += report["reducer_groups"].as_u64().unwrap_or(0);
                    w.context_rows += report["context_rows"].as_u64().unwrap_or(0);
                    if report["refreshed_strata"] != json!([]) {
                        return Err(error("temporal operator unexpectedly refreshed a stratum"));
                    }
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

pub(super) fn combine_summaries(operators: &mut [Operator]) {
    let mut plans = reductions::pool::Plans::default();
    let mut queries = vec![];
    for (id,op) in operators.iter_mut().enumerate() {
        queries.push(op.run.emitted.queries[0].clone());
        if let Some(local) = &mut op.run.reductions {
            for (name,mut state) in std::mem::take(&mut local.states) {
                state.set_step(id);
                plans.states.insert(name,state);
            }
        }
    }
    reductions::pool::combine_cursors(&mut plans,&queries);
    let storage = plans.storage;
    for op in operators {
        if let Some(state) = plans.states.remove(&op.result) {
            let local = op.run.reductions.as_mut().unwrap();
            op.phase_arms = state.phase_arms();
            op.phase_eligible = state.common_phase().proven;
            op.logical_analysis = Arc::new(json!([state.explain()]));
            local.states.insert(op.result.clone(),state);
            local.storage=storage.clone();
        }
    }
}
