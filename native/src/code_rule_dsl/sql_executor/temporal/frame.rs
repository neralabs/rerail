//! Retained input, output, and reducer versions for one cold wave.
use super::*;
use operator::Operator;

#[derive(Clone)]
pub(super) struct Frame {
    scope: usize,
    inputs: BTreeMap<String, String>,
    input_counters: BTreeMap<String, String>,
    pub published: BTreeMap<String, String>,
    pub frontiers: BTreeMap<String, String>,
    counters: BTreeMap<String, String>,
    operators: Vec<Operator>,
}
pub(super) struct Versions {
    pub published: BTreeMap<String, versions::Version>,
    pub frontiers: BTreeMap<String, versions::Version>,
    pub counters: BTreeMap<String, versions::Version>,
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
pub(super) fn keys(w: &mut Work<'_>, target: &str, d: &Delta, width: usize) -> Result<()> {
    let cols = names("", width);
    for table in [&d.plus, &d.minus] {
        w.db.execute(
            &format!(
                "INSERT OR IGNORE INTO {target} SELECT {cols} FROM {}",
                quote(table)
            ),
            [],
        )
        .map_err(error)?;
    }
    Ok(())
}

impl Frame {
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
                store.create(
                    w.db,
                    &table,
                    versions::Schema::integers(spec.width, spec.keys),
                )?;
                store.restrict_indexes(&table, &spec.indexes)?;
                inputs.insert(key, table);
            }
            let table = format!("{prefix}_r{id}");
            store.create(
                w.db,
                &table,
                versions::Schema::integers(spec.width, spec.keys),
            )?;
            store.restrict_indexes(&table, &spec.indexes)?;
            published.insert(r.clone(), table);
            let table = format!("{prefix}_f{id}");
            store.create(
                w.db,
                &table,
                versions::Schema::integers(spec.width, spec.keys),
            )?;
            store.restrict_indexes(&table, &spec.indexes)?;
            frontiers.insert(r.clone(), table);
            if spec.widening() {
                let table = format!("{prefix}_q{id}");
                store.create(
                    w.db,
                    &table,
                    versions::Schema::integers(spec.keys + 1, spec.keys),
                )?;
                store.restrict_indexes(&table, &[])?;
                input_counters.insert(r.clone(), table);
                let table = format!("{prefix}_c{id}");
                store.create(
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
        Ok(Self {
            scope,
            inputs,
            input_counters,
            published,
            frontiers,
            counters,
            operators,
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
        steps: &[(schedule::Step, Query)],
        desired: Option<Versions>,
        external: &BTreeMap<String, Delta>,
        active: &BTreeSet<usize>,
        preparing: bool,
        limit: u32,
    ) -> Result<()> {
        let _scope = store.activate(self.scope)?;
        let mut touched = BTreeMap::new();
        for (r, spec) in relations {
            touched.insert(r.clone(), w.table(spec.keys)?);
        }
        if let Some(desired) = desired {
            for (r, spec) in relations {
                for (logical, version) in [
                    (r.clone(), &desired.published[r]),
                    (format!("delta_{r}"), &desired.frontiers[r]),
                ] {
                    let table = &self.inputs[&logical];
                    let old = store.version(table);
                    if !old.same(version) {
                        if logical == *r {
                            let d = delta(w, &old, version)?;
                            keys(w, &touched[r], &d, spec.keys)?;
                        }
                        store.replace(table, version.clone())?;
                    }
                }
                if spec.widening() {
                    let table = &self.input_counters[r];
                    let old = store.version(table);
                    if !old.same(&desired.counters[r]) {
                        let d = delta(w, &old, &desired.counters[r])?;
                        keys(w, &touched[r], &d, spec.keys)?;
                        store.replace(table, desired.counters[r].clone())?;
                    }
                }
            }
        }
        for (id, op) in self.operators.iter_mut().enumerate() {
            let _context = w.profile_scope(
                || json!({"stage":"rule cache","ruleStep":id,"relation":steps[id].0.output}),
            );
            let _cpu = super::super::super::logic::profile::span(|| "rule cache".into());
            let mark = w.mark();
            let before = store.version(&op.result);
            let enabled = active.contains(&id);
            let r = &steps[id].0.output;
            let spec = &relations[r];
            let cols = names("", spec.keys);
            let old_improvements = store.version(&op.improvements);
            if op.update(w, store, external, enabled, preparing, limit)? {
                if op.active && old_improvements.len() > 0 {
                    w.db.execute(
                        &format!(
                            "INSERT OR IGNORE INTO {} SELECT {cols} FROM {}",
                            touched[r],
                            quote(&op.improvements)
                        ),
                        [],
                    )
                    .map_err(error)?;
                }
                store.replace(&op.improvements, old_improvements.empty_like())?;
                op.comparison = store.version(&self.inputs[r]);
                op.active = enabled;
                w.rewind(mark)?;
                continue;
            }
            let d = delta(w, &before, &store.version(&op.result))?;
            let affected = w.table(spec.keys)?;
            keys(w, &affected, &d, spec.keys)?;
            let previous = &self.inputs[r];
            let current = store.version(previous);
            if !op.comparison.same(&current) {
                let d = delta(w, &op.comparison, &current)?;
                keys(w, &affected, &d, spec.keys)?;
            }
            op.comparison = current;
            w.db.execute(
                &format!(
                    "DELETE FROM {} WHERE ({cols}) IN(SELECT {cols} FROM {affected})",
                    quote(&op.improvements)
                ),
                [],
            )
            .map_err(error)?;
            let improve = if spec.lattice {
                let value = format!("c{}", spec.width - 1);
                format!(
                    "p.{value} IS NULL OR {}<>p.{value}",
                    spec.merge(&format!("p.{value}"), &format!("c.{value}"))
                )
            } else {
                "p.c0 IS NULL".into()
            };
            w.db.execute(&format!("INSERT INTO {} SELECT c.* FROM {affected} k CROSS JOIN {} c LEFT JOIN {} p ON {} WHERE {} AND ({improve})",
                quote(&op.improvements),quote(&op.result),quote(previous),same("p","c",spec.keys),same("c","k",spec.keys)),[]).map_err(error)?;
            let d = delta(w, &old_improvements, &store.version(&op.improvements))?;
            keys(w, &touched[r], &d, spec.keys)?;
            if enabled != op.active {
                w.db.execute(
                    &format!(
                        "INSERT OR IGNORE INTO {} SELECT {cols} FROM {}",
                        touched[r],
                        quote(&op.improvements)
                    ),
                    [],
                )
                .map_err(error)?;
            }
            op.active = enabled;
            w.rewind(mark)?;
        }
        for (r, spec) in relations {
            let mark = w.mark();
            let _context = w.profile_scope(|| json!({"stage":"merge and widen","relation":r}));
            let _cpu = super::super::super::logic::profile::span(|| "merge and widen".into());
            fold::apply(
                w,
                spec,
                &touched[r],
                &self.inputs[r],
                self.input_counters.get(r).map(String::as_str),
                &self.published[r],
                &self.frontiers[r],
                self.counters.get(r).map(String::as_str),
                self.operators
                    .iter()
                    .zip(steps)
                    .filter(|(op, (step, _))| op.active && step.output == *r)
                    .map(|(op, _)| op.improvements.as_str())
                    .collect(),
            )?;
            w.rewind(mark)?;
        }
        Ok(())
    }
}
