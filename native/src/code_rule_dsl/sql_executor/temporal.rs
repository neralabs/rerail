//! Iteration-indexed recursive execution. Each rule cache reads only preceding-wave versions.
use super::super::logic::sql::select::{Query, Relation as Rel};
use super::maintenance::{Delta, Work};
use super::*;
use crate::code_rule_dsl::logic::native::aggregation::{Grouping, Reduction};
mod batch;
mod compose;
mod cold_loop;
mod batches;
mod fold;
mod frame;
pub(super) mod initial;
mod operator;
mod quiescent;
mod pipeline;
mod opaque;
use frame::Frame;
const UNSUPPORTED: &str = "unsupported temporal retention: ";

#[derive(Clone)]
struct Relation {
    indexes: Vec<usize>,
    width: usize,
    keys: usize,
    lattice: bool,
    top: i64,
    ops: Option<schedule::LatticeOperations>,
}
impl Relation {
    fn grouping(&self) -> Grouping {
        Grouping {
            keys: self.keys,
            reduction: if !self.lattice {
                Reduction::Opaque
            } else {
                self.ops
                    .as_ref()
                    .map_or(Reduction::Flat { top: self.top }, |ops| ops.reduction)
            },
        }
    }
    fn aggregate(&self, value: &str) -> String {
        match &self.ops {
            Some(ops) => format!("{}({value})", ops.aggregate),
            None => crate::code_rule_dsl::logic::sql::lattices::flat_aggregate(value, self.top),
        }
    }
    fn merge(&self, old: &str, new: &str) -> String {
        match &self.ops {
            Some(ops) => format!(
                "CASE WHEN {old}={} THEN {} ELSE {}({old},{new}) END",
                self.top, self.top, ops.merge
            ),
            None => format!("CASE WHEN {old}={new} THEN {old} ELSE {} END", self.top),
        }
    }
    fn widening(&self) -> bool {
        self.ops
            .as_ref()
            .is_some_and(|o| o.after.is_some() && o.widen.is_some())
    }
}

fn names(alias: &str, width: usize) -> String {
    (0..width)
        .map(|i| {
            if alias.is_empty() {
                format!("c{i}")
            } else {
                format!("{alias}.c{i}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}
fn same(a: &str, b: &str, width: usize) -> String {
    (0..width)
        .map(|i| format!("{a}.c{i}={b}.c{i}"))
        .collect::<Vec<_>>()
        .join(" AND ")
}
fn read(r: &Rel) -> Option<String> {
    match r {
        Rel::Derived { rel, delta: true } => Some(format!("delta_{rel}")),
        _ => r.program_relation().map(str::to_owned),
    }
}
pub(super) fn delta(
    w: &mut Work<'_>,
    old: &versions::Version,
    new: &versions::Version,
) -> Result<Delta> {
    let operation = w.profiler.map(|p| {
        p.operation(
            "native version difference",
            ("sql-extension", 0),
            0,
            vec![
                ("old".into(), old.len() as i64),
                ("new".into(), new.len() as i64),
            ],
        )
    });
    let _cpu = super::super::logic::profile::span(|| "native version difference".into());
    // Empty and identical versions have an exact difference without a row walk.
    // Share the immutable roots instead of decoding, hashing and indexing them again.
    if old.len() == 0 || new.len() == 0 || old.same(new) {
        let plus = w.output_table(old.columns().len())?;
        let minus = w.output_table(old.columns().len())?;
        let same = old.same(new);
        let empty = old.empty_like();
        let added = if same { &empty } else { new };
        let removed = if same { &empty } else { old };
        let store = w.versions.ok_or_else(|| error("temporal rows require native storage"))?;
        if store.clone_scratch(&plus, added)? && store.clone_scratch(&minus, removed)? {
            w.version_nodes += 1;
            if let Some(operation) = operation { operation.finish(added.len()+removed.len()); }
            return Ok(Delta {plus,minus,inserted:added.len() as u64,deleted:removed.len() as u64});
        }
    }
    let plus=w.output_table(old.columns().len())?;
    let minus=w.output_table(old.columns().len())?;
    let (inserted,deleted,visited)=w.versions.ok_or_else(||error("temporal rows require native storage"))?.difference_scratch(&plus,&minus,old,new)?;
    w.version_nodes+=visited;
    if let Some(operation)=operation {operation.finish(inserted+deleted);}
    Ok(Delta{plus,minus,inserted:inserted as u64,deleted:deleted as u64})
}

#[derive(Clone)]
pub(super) struct Trace {
    whole: bool,
    native_relations: BTreeSet<String>,
    templates: Arc<BTreeMap<usize, frame::Prepared>>,
    prefix: String,
    emitted: Arc<plan::emit::Emitted>,
    relations: BTreeMap<String, Relation>,
    steps: Arc<Vec<(schedule::Step, Option<Query>)>>,
    producers: Arc<BTreeMap<String, Vec<String>>>,
    publications: opaque::Publications,
    stage_decisions: BTreeMap<String, crate::code_rule_dsl::logic::analysis::Decision>,
    cache_decisions:Arc<Vec<Value>>,
    components: Arc<crate::code_rule_dsl::logic::analysis::stages::Components>,
    after: BTreeSet<(u32, u32)>,
    frames: Vec<Frame>,
    batch: Option<batch::History>,
    limit: u32,
}

fn components(steps: &[(schedule::Step, Option<Query>)]) -> crate::code_rule_dsl::logic::analysis::stages::Components {
    let mut edges = BTreeMap::<String, BTreeSet<String>>::new();
    for (s, q) in steps {
        edges.entry(s.output.clone()).or_default().extend(plan::acyclic::reads(s,q.as_ref()));
    }
    crate::code_rule_dsl::logic::analysis::stages::Components::analyze(&edges)
}

pub(super) fn loop_analyses(emitted:&plan::emit::Emitted)->Arc<Vec<Value>> {
    let steps=emitted.program.steps.iter().cloned().zip(emitted.queries.iter().cloned()).collect::<Vec<_>>();
    let components=components(&steps);
    let lattices=emitted.program.lattices.iter().map(|(r,_,_)|r.clone()).collect();
    let groups=components.recursive.iter().map(|r|components.members[r].clone()).collect::<BTreeSet<_>>();
    Arc::new(groups.into_iter().map(|members|json!({"component":members,"analysis":plan::acyclic::version_loop(&steps,&members,&lattices),"purpose":"Logical eligibility only; actual scheduling is reported separately"})).collect())
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    #[test]
    fn equal_versions_and_cursors_reuse_frames_without_sql_phases() {
        let (_dir, mut e) = super::super::tests::executor();
        e.bind_relation(json!({"name":"seed","columns":["v"],"rows":[[1],[2]]}).to_string()).unwrap();
        e.run(".decl seed(v:number)\n.input seed\n.decl r(v:number)\nr(v) :- seed(v).\nr(v) :- r(v), n=count : {seed(_)}, n>0.\n.output r\n".into(),"python".into(),None).unwrap();
        let pool = e.scratch.clone();
        let mut work = Work::new(&e.db, &pool);
        work.versions = Some(&e.versions);
        let trace = e.executions[0].temporal.get_mut(&0).unwrap();
        assert!(trace.frames.iter().any(|f| !f.observation["recursive_components"].as_array().unwrap().is_empty()));
        let before = trace.frames.last().unwrap().versions(&e.versions);
        trace.replay(&mut work, &e.versions, &BTreeMap::new(), 100).unwrap();
        assert!(trace.frames.iter().all(|f| f.observation["reused"] == true), "{}", trace.explain());
        assert!(trace.frames.iter().all(|f| f.phase_batches == 0));
        let after = trace.frames.last().unwrap().versions(&e.versions);
        for (r, version) in before.published { assert!(version.same(&after.published[&r])); }
        work.cleanup().unwrap();
    }
}

impl Trace {
    pub fn planning_strategy(&self) -> planning::Strategy {
        if self.batch.is_some() {
            planning::Strategy::FullWaveBatch
        } else {
            planning::Strategy::SparseRetained
        }
    }
    pub(in crate::code_rule_dsl::sql_executor) fn retained_explanations(&self) -> Vec<Value> {
        self.templates.iter().flat_map(|(id,p)|p.operator.run.reductions.iter().flat_map(|s|s.values()).map(move |state| {
            let mut value=state.explain();
            value["operator_relation"]=value["relation"].clone();
            value["relation"]=json!(self.steps[*id].0.output);
            value
        })).collect()
    }
    pub(in crate::code_rule_dsl::sql_executor) fn scalar_storage_explanations(&self) -> Vec<Value> {
        let mut seen = BTreeSet::new();
        self.templates.values().flat_map(|p|p.operator.run.reductions.iter().flat_map(|s|s.storage.iter()))
            .filter(|s|seen.insert(Arc::as_ptr(&s.analysis) as usize)).map(|s|json!(s)).collect()
    }
    pub(in crate::code_rule_dsl::sql_executor) fn explain(&self) -> Value {
        if let Some(batch) = &self.batch {
            return json!({
                "execution_shape":"SQL wave batch",
                "native_relation_facades":self.native_relations,
                "dependency_components":self.components.as_ref(),
                "sql_wave_batch":batch.explain(),
            });
        }
        json!({"execution_shape":if self.whole {"One retained relation graph, including checked projections"} else {"Retained recursive component"},"native_relation_facades":self.native_relations,"acyclic_stages":self.stage_decisions,"component_cache_scheduling":self.cache_decisions.as_ref(),"acyclic_sql_depth_bound":pipeline::Pipeline::MAX_DEPTH,"sql_phase_expansion_budget":phase::Phase::EXPANSION_BUDGET,"dependency_components":self.components.as_ref(),"journal_policy":"Share empty-journal inputs and cancel equal roots; bulk set composition for large deltas; sparse patches for small deltas","public_storage":if self.whole {"Declared outputs keep SQL result tables and hooks; intermediate relations are immutable query facades"}else{"SQL-owned component results"},"phase_policy":"Compatible acyclic stages pass signed deltas through one SQL graph; recursive steps read frozen preceding-frame inputs; bounded batches preserve staged deltas","scalar_storage_boundary":"Wave gates reuse transitions only for equal input versions, prior summary roots, and pending journals", "summary_transition_proof":crate::code_rule_dsl::logic::analysis::scalars::maintenance_equivalent(true,true),"frames":self.frames.iter().enumerate().map(|(i, f)|json!({"frame":i,"native_scope":f.scope(),"composed":f.composed,"phase_batches":f.phase_batches,"acyclic_pipeline":f.pipeline,"work":f.observation,"key_set_decisions":f.key_set_decisions,"publication_decisions":f.publication_decisions,"operators":f.operators.iter().enumerate().map(|(id,op)|json!({"relation":self.steps[id].0.output,"result_root":op.result,"active":op.active,"empty_positive_body":op.empty_body,"phase_eligible":op.phase_eligible,"boundary":if op.opaque.is_some() {"Opaque SQL producer; frozen SQL inputs at resolved dependency gate"}else if op.sql_kernel {"One recursive SQL kernel; versioned result root"}else if op.phase_eligible {"Composed support and fold"}else{"Separate retained operator evaluation; shared wave publication"},"analysis":op.logical_analysis.as_ref()})).collect::<Vec<_>>()})).collect::<Vec<_>>()})
    }
    pub fn stable_phases(&self, program: &schedule::Program) -> bool {
        self.whole
            && (self.batch.is_some() || !self.frames.is_empty())
            && program.relations.len() == self.relations.len()
            && program
                .relations
                .iter()
                .all(|(r, _)| self.relations.contains_key(r))
            && self.frames.iter().all(|f| f.composed)
    }
    pub fn frames(&self) -> usize {
        self.batch.as_ref().map_or(self.frames.len(), batch::History::len)
    }
    fn final_versions(&self, store: &versions::Store) -> frame::Versions {
        if let Some(batch) = &self.batch {
            batch.versions()
        } else {
            self.frames.last().unwrap().versions(store)
        }
    }
    fn ready(&self, pending: &BTreeSet<usize>) -> BTreeSet<usize> {
        let mut ready = pending
            .iter()
            .copied()
            .filter(|&i| {
                !pending.iter().any(|&j| {
                    self.after
                        .contains(&(self.steps[i].0.stratum, self.steps[j].0.stratum))
                })
            })
            .collect::<BTreeSet<_>>();
        if let Some(frame) = self.frames.first() {
            loop {
                let before = ready.len();
                for &id in pending {
                    if frame.pipeline.contains(&id) && pending.iter().all(|&j| {
                        !self.after.contains(&(self.steps[id].0.stratum, self.steps[j].0.stratum))
                            || ready.contains(&j) && frame.pipeline.contains(&j)
                    }) { ready.insert(id); }
                }
                if ready.len() == before { break; }
            }
        }
        ready
    }
    fn replay(
        &mut self,
        w: &mut Work<'_>,
        store: &versions::Store,
        changes: &BTreeMap<String, Delta>,
        limit: u32,
    ) -> Result<u32> {
        let mut pending = (0..self.steps.len()).collect::<BTreeSet<_>>();
        let preparing = self.frames.iter().all(|f| !f.composed);
        let no_external_changes = BTreeMap::new();
        let (mut at, mut waves) = (0usize, 0u32);
        // Retain a quiescent frame after the last active wave. Its inputs include
        // the empty final frontier, so extending a later replay does not inherit
        // a large, obsolete frontier from the preceding active wave.
        let mut needs_close = false;
        let mut diagnostic_rows = vec![];
        let components = Arc::clone(&self.components);
        let mut sql_phases=0usize;
        let mut declined = BTreeMap::new();
        while at < self.frames.len() || !pending.is_empty() || needs_close {
            let low_budget = super::super::sql_admission::current()
                .is_some_and(|ledger| ledger.available() <= 20 + 2 * self.relations.len());
            if at > 0 && !pending.is_empty() && (preparing || sql_phases > 0)
                && (sql_phases >= cold_loop::HOST_PHASE_LIMIT || low_budget) {
                let ready = self.ready(&pending);
                if declined.get(&ready).is_none_or(|last| sql_phases >= last + 8) {
                    if let Some(progress) = cold_loop::resume(self, w, store, &pending, at, waves, limit, preparing, &mut diagnostic_rows)? {
                        waves = progress.waves;
                        pending = progress.pending;
                        needs_close = progress.needs_close;
                        at = self.frames.len();
                        if pending.is_empty() && !needs_close { break; }
                    } else {
                        declined.insert(ready, sql_phases);
                    }
                }
            }
            let _context = w.profile_scope(||json!({"stage":if preparing {"retained initialization"} else {"retained update"},"component":self.prefix,"frame":at}));
            let _cpu = super::super::logic::profile::span(|| "retained frame replay".into());
            if !pending.is_empty() {
                if waves >= limit {
                    return Err(error(format!(
                        "{}: no fixpoint in {limit} waves",
                        self.emitted.program.name
                    )));
                }
                waves += 1;
            }
            let fresh = at == self.frames.len();
            if fresh {
                let frame = if at == 0 {
                    Frame::create(w, store, self)?
                } else {
                    self.frames[at - 1].fork(store)?
                };
                self.frames.push(frame);
            }
            let active = self.ready(&pending);
            if !pending.is_empty() && active.is_empty() {
                return Err(error("temporal strata have no ready step"));
            }
            pending.retain(|s| !active.contains(s));
            let desired = if at == 0 {
                None
            } else {
                Some(self.frames[at - 1].versions(store))
            };
            let had = if preparing && self.whole {
                desired
                    .as_ref()
                    .map(|d| {
                        d.frontiers
                            .iter()
                            .filter(|(_, v)| v.len() > 0)
                            .map(|(r, _)| r.clone())
                            .collect::<BTreeSet<_>>()
                    })
                    .unwrap_or_default()
            } else {
                BTreeSet::new()
            };
            let mark = w.mark();
            let frame = &mut self.frames[at];
            frame.journal(w, store, if fresh { &no_external_changes } else { changes })?;
            let skipped = !fresh && frame.reusable(store, desired.as_ref(), &active)?;
            if !skipped {
            frame
                .advance(
                    w,
                    store,
                    &self.relations,
                    &self.steps,
                    desired,
                    &no_external_changes,
                    &active,
                    preparing,
                    &self.templates,
                    &mut self.publications,
                    limit,
                )
                .map_err(|e| Error::new(e.status, format!("{} frame {at}: {e}", self.prefix)))?;
            } else {
                frame.phase_batches = 0;
            }
            let recursive_components = active.iter().filter_map(|&id| {
                let (s, q) = &self.steps[id];
                let members = &components.members[&s.output];
                plan::acyclic::reads(s,q.as_ref()).iter().any(|r| members.contains(r))
                    .then(|| members.clone())
            }).collect::<BTreeSet<_>>();
            let component_work = recursive_components.iter().map(|members| {
                let produced = members.iter().map(|r| store.version_at(frame.scope(), &frame.frontiers[r]).len()).sum::<usize>();
                let mut seed_steps = 0;
                let mut recursive_steps = 0;
                let mut recursive_candidate_rows = 0;
                let mut recursive_inputs_nonempty = false;
                for &id in &active {
                    let (s,q) = &self.steps[id];
                    if !members.contains(&s.output) { continue; }
                    let reads = q.iter().flat_map(|q|q.reads()).filter(|r| r.program_relation().is_some_and(|r| members.contains(r))).collect::<Vec<_>>();
                    if reads.is_empty() { seed_steps += 1; } else {
                        recursive_steps += 1;
                        recursive_candidate_rows += store.version_at(frame.scope(), &frame.operators[id].improvements).len();
                    }
                    recursive_inputs_nonempty |= reads.iter().filter_map(|r|read(r))
                        .filter_map(|r| frame.inputs.get(&r)).any(|table| store.version_at(frame.scope(), table).len()>0);
                }
                json!({"relations":members,"frontier_rows":produced,"productive":produced>0,
                    "executed":!skipped,"seed_steps":seed_steps,"recursive_steps":recursive_steps,
                    "recursive_inputs_nonempty":recursive_inputs_nonempty,
                    "recursive_candidate_rows":recursive_candidate_rows,
                    "productive_feedback":!skipped && recursive_inputs_nonempty && recursive_candidate_rows>0 && produced>0})
            }).collect::<Vec<_>>();
            frame.observation = json!({"purpose":if active.is_empty() {
                if needs_close {"closing"} else {"retained history"}
            } else if !recursive_components.is_empty() {"recursive evaluation"} else {"acyclic dependency stage"},
                "scheduled_steps":active.len(),"reused":skipped,"sql_phases":frame.phase_batches,
                "inactive_carry":!skipped && active.is_empty(),
                "recursive_components":component_work,
                "reuse_proof":if skipped {Some("Equal input and counter versions, identical gate mask, synchronized active cursors, and empty active journals; inactive journals retain edits")}else{None}});
            if let Some(profiler) = w.profiler {
                let _context = w.profile_scope(|| json!({"frameWork":frame.observation}));
                profiler.operation("retained frame work", ("sql-extension",0),0,
                    vec![("scheduled_steps".into(),active.len() as i64),
                        ("recursive_components".into(),recursive_components.len() as i64),
                        ("reused".into(),i64::from(skipped)),
                        ("sql_phases".into(),frame.phase_batches as i64)]).finish_unobserved();
            }
            if preparing {
                for (id, template) in frame.prepared_missing(store, &self.templates) {
                    Arc::make_mut(&mut self.templates)
                        .entry(id)
                        .or_insert(template);
                }
            }
            if !active.is_empty() {
                for r in frame.frontiers.keys() {
                    if frame.has_frontier(store, r) {
                        for (i, (s, _)) in self.steps.iter().enumerate() {
                            if s.reads.contains(r) {
                                let consumed = active.contains(&i) && frame.pipeline.contains(&i)
                                    && active.iter().any(|id| frame.pipeline.contains(id) && self.steps[*id].0.output == *r);
                                if !consumed { pending.insert(i); }
                            }
                        }
                    }
                }
            }
            if preparing && self.whole && !active.is_empty() {
                let written = active
                    .iter()
                    .map(|&id| self.steps[id].0.output.as_str())
                    .collect::<BTreeSet<_>>();
                let rows: Vec<Vec<SqlValue>> = self
                    .relations
                    .keys()
                    .filter(|r| written.contains(r.as_str()) || had.contains(*r))
                    .map(|r| {
                        vec![
                            SqlValue::Integer(waves as i64),
                            SqlValue::Text(r.clone()),
                            SqlValue::Integer(
                                store.version_at(frame.scope(), &frame.frontiers[r]).len() as i64,
                            ),
                        ]
                    })
                    .collect();
                diagnostic_rows.extend(rows);
            }
            sql_phases+=frame.phase_batches;
            w.rewind(mark)?;
            needs_close = !active.is_empty();
            at += 1;
        }
        plan::insert_many(
            w.db,
            "INSERT INTO temp.wave_delta(wave,rel,rows)",
            diagnostic_rows,
        )
        .map_err(error)?;
        Ok(waves)
    }
    pub fn objects(&self) -> impl Iterator<Item = (String, bool)> + '_ {
        self.frames
            .iter()
            .flat_map(Frame::objects)
            .collect::<BTreeSet<_>>()
            .into_iter()
    }
    pub fn apply(
        &mut self,
        w: &mut Work<'_>,
        store: &versions::Store,
        changes: &BTreeMap<String, Delta>,
        limit: u32,
    ) -> Result<(BTreeMap<String, Delta>, u32)> {
        let before = self.final_versions(store);
        let waves = if let Some(batch) = &mut self.batch {
            batch.replay(w, changes, limit, false)?
        } else {
            self.replay(w, store, changes, limit)?
        };
        let after = self.final_versions(store);
        let mut outputs = BTreeMap::new();
        for (r, spec) in &self.relations {
            let d = delta(w, &before.published[r], &after.published[r])?;
            if self.whole && !self.native_relations.contains(r) {
                self.publications.publish(w,r,after.published[r].clone())?;
            }
            if d.inserted + d.deleted == 0 && !spec.widening() {
                outputs.insert(r.clone(), d);
                continue;
            }
            let cols = names("", spec.width);
            if self.native_relations.contains(r) {
                let _scope = store.activate(0)?;
                store.alias_readonly(w.db,r,after.published[r].clone(),
                    (0..spec.width).map(|c|format!("c{c}")).collect())?;
            } else if !self.whole {
            if d.deleted > 0 {
                w.db.execute(
                    &format!(
                        "DELETE FROM {} WHERE ({cols}) IN(SELECT {cols} FROM {})",
                        quote(r),
                        d.minus
                    ),
                    [],
                )
                .map_err(error)?;
            }
            if d.inserted > 0 {
                w.db.execute(
                    &format!("INSERT INTO {} SELECT * FROM {}", quote(r), d.plus),
                    [],
                )
                .map_err(error)?;
            }
            }
            if spec.widening() {
                let table = format!("__sql_changes_{r}");
                if store.contains(&table) {
                    store.alias_readonly(
                        w.db,
                        &table,
                        after.counters[r].clone(),
                        (0..spec.keys)
                            .map(|c| format!("c{c}"))
                            .chain(["changes".into()])
                            .collect(),
                    )?;
                    outputs.insert(r.clone(), d);
                    continue;
                }
                let counts = delta(w, &before.counters[r], &after.counters[r])?;
                let keys = names("", spec.keys);
                w.db.execute(
                    &format!(
                        "DELETE FROM __sql_changes_{r} WHERE ({keys}) IN(SELECT {keys} FROM {})",
                        counts.minus
                    ),
                    [],
                )
                .map_err(error)?;
                w.db.execute(
                    &format!(
                        "INSERT INTO __sql_changes_{r} SELECT * FROM {}",
                        counts.plus
                    ),
                    [],
                )
                .map_err(error)?;
            }
            outputs.insert(r.clone(), d);
        }
        Ok((outputs, waves))
    }
}

pub(super) fn prepare(
    w: &mut Work<'_>,
    store: &versions::Store,
    run: &mut session::Execution,
    namespace: usize,
) -> Result<()> {
    let e = run.emitted.clone();
    let mut positions = BTreeMap::<String, u32>::new();
    for s in &e.program.steps {
        positions
            .entry(s.output.clone())
            .and_modify(|n| *n = (*n).max(s.stratum))
            .or_insert(s.stratum);
    }
    let mut groups = BTreeMap::<u32, BTreeSet<String>>::new();
    for (r, s) in positions {
        if s > 0 {
            groups.entry(s).or_default().insert(r);
        }
    }
    for (stratum, group) in groups {
        // A SQL producer's step has no checked query: its stratum is recomputed whole (`maintenance`).
        if group.iter().any(|r| run.producers.contains_key(r)) {
            continue;
        }
        let steps = e
            .program
            .steps
            .iter()
            .zip(&e.queries)
            .filter(|(s, _)| group.contains(&s.output))
            .map(|(s, q)| {
                q.clone()
                    .map(|q| (s.clone(), Some(q)))
                    .ok_or_else(|| error("temporal constants require checked queries"))
            })
            .collect::<Result<Vec<_>>>()?;
        let recursive = steps.iter().any(|(_, q)| {
            q.as_ref().unwrap().reads()
                .iter()
                .filter_map(|r| r.program_relation())
                .any(|r| group.contains(r))
        });
        let nonmonotone = group
            .iter()
            .any(|r| e.program.lattices.iter().any(|(n, _, _)| n == r))
            || steps.iter().any(|(_, q)| maintenance::barrier(q.as_ref().unwrap()));
        if !recursive || !nonmonotone {
            continue;
        }
        let probes = indexes::native(steps.iter().filter_map(|(_, q)| q.as_ref()));
        let relations = e
            .program
            .relations
            .iter()
            .filter(|(r, _)| group.contains(r))
            .map(|(r, width)| {
                let lattice = e.program.lattices.iter().find(|(n, _, _)| n == r);
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
                        ops: e.program.lattice_operations.get(r).cloned(),
                    },
                )
            })
            .collect();
        let mut after = BTreeSet::new();
        for (s, _) in &steps {
            for (p, _) in &steps {
                if s.stratum != p.stratum && s.reads.contains(&p.output) {
                    after.insert((s.stratum, p.stratum));
                }
            }
        }
        loop {
            let before = after.len();
            for (a, b) in after.clone() {
                for (c, d) in after.clone() {
                    if b == c && a != d {
                        after.insert((a, d));
                    }
                }
            }
            if after.len() == before {
                break;
            }
        }
        let components=Arc::new(components(&steps));
        let mut trace = Trace {
            whole: false,
            producers: Arc::new(BTreeMap::new()),
            publications: opaque::Publications::default(),
            native_relations: BTreeSet::new(),
            templates: Arc::new(BTreeMap::new()),
            prefix: format!("__it{namespace}_s{stratum}"),
            emitted: e.clone(),
            relations,
            components,
            steps: Arc::new(steps),
            stage_decisions: BTreeMap::new(),
            cache_decisions:Arc::new(vec![]),
            after,
            frames: vec![],
            batch: None,
            limit: run.max_waves,
        };
        let mark = w.mark();
        let scopes = store.scope_mark();
        match trace.replay(w, store, &BTreeMap::new(), run.max_waves) {
            Err(e) if e.status == napi::Status::InvalidArg && e.reason.contains(UNSUPPORTED) => {
                store.discard_scopes_after(w.db, scopes)?;
                w.rewind(mark)?;
                continue;
            }
            Err(e) => return Err(e),
            Ok(_) => w.rewind(mark)?,
        }
        for (r, table) in &trace.frames.last().unwrap().published {
            let differs = w.db.query_row(&format!("SELECT EXISTS(SELECT * FROM {} EXCEPT SELECT * FROM {}) OR EXISTS(SELECT * FROM {} EXCEPT SELECT * FROM {})",quote(r),quote(table),quote(table),quote(r)),[],|r|r.get::<_,bool>(0)).map_err(error)?;
            if differs {
                return Err(error(format!(
                    "{r}: retained iterations differ from cold SQL execution"
                )));
            }
        }
        run.objects.extend(trace.objects());
        run.recursive.remove(&stratum);
        run.temporal.insert(stratum, trace);
    }
    Ok(())
}
