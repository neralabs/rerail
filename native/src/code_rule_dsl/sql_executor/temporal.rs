//! Iteration-indexed recursive execution. Each rule cache reads only preceding-wave versions.
use super::super::logic::sql::select::{Query, Relation as Rel};
use super::maintenance::{Delta, Work};
use super::*;
use crate::code_rule_dsl::logic::native::aggregation::{Grouping, Reduction};
mod compose;
mod fold;
mod frame;
pub(super) mod initial;
mod operator;
mod quiescent;
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
    let (plus, minus, visited) = old.difference(new);
    w.version_nodes += visited;
    let convert = |rows: Vec<Vec<Option<i64>>>| -> Result<Vec<Vec<i64>>> {
        rows.into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|v| v.ok_or_else(|| error("null in temporal fact relation")))
                    .collect()
            })
            .collect()
    };
    let plus = convert(plus)?;
    let minus = convert(minus)?;
    if let Some(operation) = operation {
        operation.finish(plus.len() + minus.len());
    }
    let (inserted, deleted) = (plus.len() as u64, minus.len() as u64);
    Ok(Delta {
        plus: frozen_rows(w, old.columns().len(), plus)?,
        minus: frozen_rows(w, old.columns().len(), minus)?,
        inserted,
        deleted,
    })
}

fn frozen_rows(w: &mut Work<'_>, width: usize, rows: Vec<Vec<i64>>) -> Result<String> {
    let name = w.output_table(width)?;
    let n = rows.len();
    let operation = w.profiler.map(|p| {
        p.operation(
            "bulk delta storage",
            ("sql-extension", 0),
            0,
            vec![("rows".into(), n as i64)],
        )
    });
    let _cpu = super::super::logic::profile::span(|| "bulk delta storage".into());
    w.versions
        .ok_or_else(|| error("temporal rows require native storage"))?
        .load_scratch(
            &name,
            rows.into_iter()
                .map(|r| r.into_iter().map(Some).collect())
                .collect(),
        )?;
    if let Some(operation) = operation {
        operation.finish(n);
    }
    Ok(name)
}

#[derive(Clone)]
pub(super) struct Trace {
    whole: bool,
    templates: Arc<BTreeMap<usize, frame::Prepared>>,
    prefix: String,
    emitted: Arc<plan::emit::Emitted>,
    relations: BTreeMap<String, Relation>,
    steps: Vec<(schedule::Step, Query)>,
    after: BTreeSet<(u32, u32)>,
    frames: Vec<Frame>,
    limit: u32,
}

impl Trace {
    pub fn stable_phases(&self, program: &schedule::Program) -> bool {
        self.whole
            && !self.frames.is_empty()
            && program.relations.len() == self.relations.len()
            && program
                .relations
                .iter()
                .all(|(r, _)| self.relations.contains_key(r))
            && self.frames.iter().all(|f| f.fused(self.relations.len()))
    }
    pub fn frames(&self) -> usize {
        self.frames.len()
    }
    fn ready(&self, pending: &BTreeSet<usize>) -> BTreeSet<usize> {
        pending
            .iter()
            .copied()
            .filter(|&i| {
                !pending.iter().any(|&j| {
                    self.after
                        .contains(&(self.steps[i].0.stratum, self.steps[j].0.stratum))
                })
            })
            .collect()
    }
    fn replay(
        &mut self,
        w: &mut Work<'_>,
        store: &versions::Store,
        changes: &BTreeMap<String, Delta>,
        limit: u32,
    ) -> Result<u32> {
        let mut pending = (0..self.steps.len()).collect::<BTreeSet<_>>();
        let preparing = self.frames.is_empty();
        let no_external_changes = BTreeMap::new();
        let (mut at, mut waves) = (0usize, 0u32);
        // Retain a quiescent frame after the last active wave. Its inputs include
        // the empty final frontier, so extending a later replay does not inherit
        // a large, obsolete frontier from the preceding active wave.
        let mut needs_close = false;
        let mut diagnostic_rows = vec![];
        while at < self.frames.len() || !pending.is_empty() || needs_close {
            let _context = w.profile_scope(||json!({"stage":if preparing {"recursive initialization"} else {"recursive update"},"component":self.prefix,"frame":at}));
            let _cpu = super::super::logic::profile::span(|| "recursive iteration".into());
            if !pending.is_empty() {
                if waves >= limit {
                    return Err(error(format!(
                        "{}: no fixpoint in {limit} waves",
                        self.emitted.program.name
                    )));
                }
                waves += 1;
            }
            let active = self.ready(&pending);
            if !pending.is_empty() && active.is_empty() {
                return Err(error("temporal strata have no ready step"));
            }
            pending.retain(|s| !active.contains(s));
            let fresh = at == self.frames.len();
            if fresh {
                let frame = if at == 0 {
                    Frame::create(w, store, self)?
                } else {
                    self.frames[at - 1].fork(store)?
                };
                self.frames.push(frame);
            }
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
            frame
                .advance(
                    w,
                    store,
                    &self.relations,
                    &self.steps,
                    desired,
                    if fresh { &no_external_changes } else { changes },
                    &active,
                    preparing,
                    &self.templates,
                    limit,
                )
                .map_err(|e| Error::new(e.status, format!("{} frame {at}: {e}", self.prefix)))?;
            if preparing {
                for (id, template) in frame.prepared(store) {
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
                                pending.insert(i);
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
        let before = self.frames.last().unwrap().versions(store);
        let waves = self.replay(w, store, changes, limit)?;
        let after = self.frames.last().unwrap().versions(store);
        let mut outputs = BTreeMap::new();
        for (r, spec) in &self.relations {
            let d = delta(w, &before.published[r], &after.published[r])?;
            if d.inserted + d.deleted == 0 && !spec.widening() {
                outputs.insert(r.clone(), d);
                continue;
            }
            let cols = names("", spec.width);
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
                    .map(|q| (s.clone(), q))
                    .ok_or_else(|| error("temporal constants require checked queries"))
            })
            .collect::<Result<Vec<_>>>()?;
        let recursive = steps.iter().any(|(_, q)| {
            q.reads()
                .iter()
                .filter_map(|r| r.program_relation())
                .any(|r| group.contains(r))
        });
        let nonmonotone = group
            .iter()
            .any(|r| e.program.lattices.iter().any(|(n, _, _)| n == r))
            || steps.iter().any(|(_, q)| maintenance::barrier(q));
        if !recursive || !nonmonotone {
            continue;
        }
        let probes = indexes::native(steps.iter().map(|(_, q)| q));
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
        let mut trace = Trace {
            whole: false,
            templates: Arc::new(BTreeMap::new()),
            prefix: format!("__it{namespace}_s{stratum}"),
            emitted: e.clone(),
            relations,
            steps,
            after,
            frames: vec![],
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
