//! Retained input, output, and reducer versions for one cold wave.
use super::*;
use operator::Operator;

#[derive(Clone)]
pub(super) struct Frame {
    pub(super) scope: usize,
    pub(super) inputs: BTreeMap<String, String>,
    pub(super) input_counters: BTreeMap<String, String>,
    pub published: BTreeMap<String, String>,
    pub frontiers: BTreeMap<String, String>,
    pub(super) counters: BTreeMap<String, String>,
    pub(super) operators: Vec<Operator>,
    pub(super) pipeline: BTreeSet<usize>,
    pub(super) key_set_decisions: Vec<Value>,
    pub(super) publication_decisions: Vec<Value>,
    pub(super) composed: bool,
    pub(super) phase_batches: usize,
    pub(super) observation: Value,
}
pub(super) struct Versions {
    pub published: BTreeMap<String, versions::Version>,
    pub frontiers: BTreeMap<String, versions::Version>,
    pub counters: BTreeMap<String, versions::Version>,
}
#[derive(Clone)]
pub(super) struct Prepared {
    pub(super) operator: Operator,
    pub(super) roots: Vec<(String, versions::Version)>,
}
fn snapshot(
    store: &versions::Store,
    scope: usize,
    tables: &BTreeMap<String, String>,
) -> BTreeMap<String, versions::Version> {
    tables
        .iter()
        .map(|(r, t)| (r.clone(), store.version_at(scope, t)))
        .collect()
}

impl Frame {
    pub fn prepared(&self, store: &versions::Store) -> Vec<(usize, Prepared)> {
        self.operators
            .iter()
            .enumerate()
            .filter(|(_, op)| op.ready())
            .map(|(id, op)| {
                (
                    id,
                    Prepared {
                        operator: op.clone(),
                        roots: op
                            .run
                            .objects
                            .iter()
                            .map(|(name, _)| (name.clone(), store.version_at(self.scope, name)))
                            .collect(),
                    },
                )
            })
            .collect()
    }
    pub fn scope(&self) -> usize {
        self.scope
    }
    pub fn create(w: &mut Work<'_>, store: &versions::Store, trace: &Trace) -> Result<Self> {
        let prefix = format!("{}_f0", trace.prefix);
        let scope = store.new_scope()?;
        let _scope = store.activate(scope)?;
        let (mut inputs, mut input_counters, mut published, mut frontiers, mut counters) = (
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        for (id, (r, spec)) in trace.relations.iter().enumerate() {
            for (table, key) in [
                (format!("{prefix}_i{id}"), r.clone()),
                (format!("{prefix}_d{id}"), format!("delta_{r}")),
            ] {
                store.create_readonly(
                    w.db,
                    &table,
                    versions::Schema::integers(spec.width, spec.keys),
                )?;
                store.restrict_indexes(&table, &spec.indexes)?;
                inputs.insert(key, table);
            }
            let table = format!("{prefix}_r{id}");
            store.create_readonly(
                w.db,
                &table,
                versions::Schema::integers(spec.width, spec.keys),
            )?;
            store.restrict_indexes(&table, &spec.indexes)?;
            published.insert(r.clone(), table);
            let table = format!("{prefix}_f{id}");
            store.create_readonly(
                w.db,
                &table,
                versions::Schema::integers(spec.width, spec.keys),
            )?;
            store.restrict_indexes(&table, &spec.indexes)?;
            frontiers.insert(r.clone(), table);
            if spec.widening() {
                let table = format!("{prefix}_q{id}");
                store.create_readonly(
                    w.db,
                    &table,
                    versions::Schema::integers(spec.keys + 1, spec.keys),
                )?;
                store.restrict_indexes(&table, &[])?;
                input_counters.insert(r.clone(), table);
                let table = format!("{prefix}_c{id}");
                store.create_readonly(
                    w.db,
                    &table,
                    versions::Schema::integers(spec.keys + 1, spec.keys),
                )?;
                store.restrict_indexes(&table, &[])?;
                counters.insert(r.clone(), table);
            }
        }
        let mut operators = vec![];
        for id in 0..trace.steps.len() {
            operators.push(Operator::create(w, store, trace, id, &prefix, &inputs)?);
        }
        operator::combine_summaries(&mut operators);
        let pipeline = operators.iter().enumerate().filter(|(id, op)| op.phase_eligible
            && trace.stage_decisions.get(&trace.steps[*id].0.output).is_some_and(|d| d.proven))
            .map(|(id,_)| id).collect();
        Ok(Self {
            scope,
            inputs,
            input_counters,
            published,
            frontiers,
            counters,
            operators,
            pipeline,
            key_set_decisions: vec![],
            publication_decisions: vec![],
            composed: false,
            phase_batches: 0,
            observation: json!({}),
        })
    }
    pub fn fork(&self, store: &versions::Store) -> Result<Self> {
        let mut next = self.clone();
        next.scope = store.fork_scope(self.scope)?;
        Ok(next)
    }
    pub fn versions(&self, store: &versions::Store) -> Versions {
        Versions {
            published: snapshot(store, self.scope, &self.published),
            frontiers: snapshot(store, self.scope, &self.frontiers),
            counters: snapshot(store, self.scope, &self.counters),
        }
    }
    pub fn has_frontier(&self, store: &versions::Store, r: &str) -> bool {
        store.version_at(self.scope, &self.frontiers[r]).len() > 0
    }
    pub fn objects(&self) -> impl Iterator<Item = (String, bool)> + '_ {
        self.inputs
            .values()
            .chain(self.input_counters.values())
            .chain(self.published.values())
            .chain(self.frontiers.values())
            .chain(self.counters.values())
            .cloned()
            .map(|n| (n, true))
            .chain(self.operators.iter().flat_map(Operator::objects))
    }

    pub fn advance(
        &mut self,
        w: &mut Work<'_>,
        store: &versions::Store,
        relations: &BTreeMap<String, Relation>,
        steps: &[(schedule::Step, Option<Query>)],
        desired: Option<Versions>,
        external: &BTreeMap<String, Delta>,
        active: &BTreeSet<usize>,
        preparing: bool,
        templates: &BTreeMap<usize, Prepared>,
        publications: &mut opaque::Publications,
        limit: u32,
    ) -> Result<()> {
        let _scope = store.activate(self.scope)?;
        self.composed = true;
        if active.is_empty() {
            if let Some(desired) = desired.as_ref() {
                // No rule contributes at this gate. Its exact output is the
                // predecessor's published roots and counters, with no frontier.
                // Keep suspended operator caches and journals at their cursors.
                for (r, version) in &desired.published {
                    for table in [&self.inputs[r], &self.published[r]] {
                        if !store.version(table).same(version) { store.replace(table, version.clone())?; }
                    }
                    let delta = &self.inputs[&format!("delta_{r}")];
                    if !store.version(delta).same(&desired.frontiers[r]) { store.replace(delta, desired.frontiers[r].clone())?; }
                    let frontier = &self.frontiers[r];
                    let previous = store.version(frontier);
                    if previous.len() > 0 { store.replace(frontier, previous.empty_like())?; }
                }
                for (r, table) in &self.counters {
                    for table in [table, &self.input_counters[r]] {
                        if !store.version(table).same(&desired.counters[r]) { store.replace(table, desired.counters[r].clone())?; }
                    }
                }
                for op in &mut self.operators { op.active = false; }
                self.phase_batches = 0;
                self.key_set_decisions.clear();
                self.publication_decisions.clear();
                return Ok(());
            }
        }
        self.advance_phase(
            w, store, relations, steps, desired, external, active, preparing, templates, publications, limit,
        )
    }

    /// Reuse a frame only when its input roots, gate mask, and active maintenance
    /// cursors still describe the exact transition that produced its outputs.
    pub(super) fn journal(&self, w: &mut Work<'_>, store: &versions::Store, external: &BTreeMap<String, Delta>) -> Result<()> {
        let _scope = store.activate(self.scope)?;
        for op in &self.operators {
            for (r, p) in &op.pending {
                if let Some(d) = external.get(r).filter(|d| d.inserted + d.deleted > 0) { p.add(w, d)?; }
            }
        }
        Ok(())
    }
    pub(super) fn reusable(
        &self, store: &versions::Store, desired: Option<&Versions>, active: &BTreeSet<usize>,
    ) -> Result<bool> {
        if !self.composed { return Ok(false); }
        let _scope = store.activate(self.scope)?;
        if let Some(desired) = desired {
            for (r, version) in &desired.published {
                if !store.version(&self.inputs[r]).same(version)
                    || !store.version(&self.inputs[&format!("delta_{r}")]).same(&desired.frontiers[r]) {
                    return Ok(false);
                }
            }
            for (r, table) in &self.input_counters {
                if !store.version(table).same(&desired.counters[r]) { return Ok(false); }
            }
        }
        for (id, op) in self.operators.iter().enumerate() {
            if op.active != active.contains(&id) || op.active && !op.ready() { return Ok(false); }
            for p in op.pending.values().filter(|_| op.active) {
                if store.version(&p.plus).len() > 0 || store.version(&p.minus).len() > 0 {
                    return Ok(false);
                }
            }
            if op.active && op.seen.iter().any(|(r, v)| !store.version(r).same(v)) { return Ok(false); }
        }
        Ok(true)
    }
}
