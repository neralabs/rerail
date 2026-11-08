//! Retained nonrecursive rule contexts and invertible scalar reducers.
//! Checked query trees determine eligibility. Unsupported shapes use the ordinary stratum refresh.
use super::super::logic::sql::{
    select::{FromItem, Join, Query, Relation as Rel, Select},
    sql_ast::{SqlBinaryOperator as B, SqlExpression as E, SqlValue as V},
};
use super::maintenance::{Delta, Work};
use super::*;
mod join_tree;
mod lattice;
mod maintain;
mod plan;
mod reducer;
mod sums;
use plan::{Rule, column, integer, names, plain, source};
use reducer::Reducer;

#[derive(Clone)]
pub(super) struct Retained {
    output: String,
    heads: String,
    width: usize,
    rules: Vec<Rule>,
    lattice: Option<lattice::Lattice>,
    pub(super) ready: bool,
}

impl Retained {
    pub(super) fn initial_result(&self) -> String {
        let cols = names("", self.width);
        match &self.lattice {
            None => format!("SELECT {cols} FROM {} WHERE n>0", quote(&self.heads)),
            Some(lattice) => {
                let key = names("", lattice.keys);
                let v = format!("c{}", self.width - 1);
                let folded = match &lattice.aggregate {
                    Some(aggregate) => format!("{aggregate}({v})"),
                    None => format!(
                        "CASE WHEN count(DISTINCT {v})>1 OR max({v})={} THEN {} ELSE min({v}) END",
                        lattice.top, lattice.top
                    ),
                };
                format!(
                    "SELECT {key},{folded} FROM {} WHERE n>0 GROUP BY {key}",
                    quote(&self.heads)
                )
            }
        }
    }
    pub(super) fn reducer_inputs(&self) -> impl Iterator<Item = &str> {
        self.rules
            .iter()
            .flat_map(|r| &r.reducers)
            .map(|r| r.relation.as_str())
    }

    /// Drop support for an empty positive body, retaining global scalar input indexes.
    pub(super) fn clear_contexts(&self, store: &versions::Store) -> Result<()> {
        let mut objects = vec![self.heads.clone()];
        for rule in &self.rules {
            objects.extend([rule.contexts.clone(), rule.contributions.clone()]);
            objects.extend(rule.reducers.iter().filter_map(Reducer::dependency_object));
        }
        if let Some(tree) = self.lattice.as_ref().and_then(|l| l.tree.as_ref()) {
            objects.extend([tree.nodes.clone(), tree.roots.clone()]);
        }
        for name in objects {
            let version = store.version(&name);
            if version.len() > 0 {
                store.replace(&name, version.empty_like())?;
            }
        }
        Ok(())
    }
}

/// Sum signed deltas with a wider intermediate; fail atomically instead of storing SQLite REALs.
pub(super) fn register(db: &rusqlite::Connection) -> Result<()> {
    use rusqlite::functions::{Aggregate, Context, FunctionFlags};
    struct SignedSum;
    impl Aggregate<i128, i64> for SignedSum {
        fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<i128> {
            Ok(0)
        }
        fn step(&self, ctx: &mut Context<'_>, sum: &mut i128) -> rusqlite::Result<()> {
            *sum += i128::from(ctx.get::<i64>(0)?) * i128::from(ctx.get::<i64>(1)?);
            Ok(())
        }
        fn finalize(&self, _: &mut Context<'_>, sum: Option<i128>) -> rusqlite::Result<i64> {
            i64::try_from(sum.unwrap_or(0))
                .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))
        }
    }
    db.create_aggregate_function(
        c"rr_update_sum",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        SignedSum,
    )
    .map_err(error)?;
    sums::register(db)
}

fn owned(
    db: &rusqlite::Connection,
    objects: &mut Vec<(String, bool)>,
    name: &str,
    definition: &str,
    storage: Option<&versions::Store>,
) -> Result<()> {
    if let Some(storage) = storage {
        storage.declare_empty(db, name, versions::Schema::generated(definition)?)?;
        storage.restrict_indexes(name, &[])?;
    } else {
        db.execute_batch(&format!(
            "CREATE TEMP TABLE {}({definition}) WITHOUT ROWID",
            quote(name)
        ))
        .map_err(error)?;
    }
    objects.push((name.into(), true));
    Ok(())
}
fn insert(
    db: &rusqlite::Connection,
    table: &str,
    query: &str,
    storage: Option<&versions::Store>,
) -> Result<usize> {
    if let Some(store) = storage.filter(|s| s.contains(table) && s.version(table).len() == 0) {
        store.load_select(db, table, query, false)
    } else {
        db.execute(&format!("INSERT INTO {} {query}", quote(table)), [])
            .map_err(error)
    }
}
fn schema(width: usize) -> String {
    (0..width)
        .map(|i| format!("c{i} INTEGER NOT NULL"))
        .collect::<Vec<_>>()
        .join(",")
}
fn equal(a: &str, b: &str, width: usize) -> String {
    (0..width)
        .map(|i| format!("{a}.c{i}={b}.c{i}"))
        .collect::<Vec<_>>()
        .join(" AND ")
}
fn execute(db: &rusqlite::Connection, sql: &str) -> Result<usize> {
    db.execute(sql, []).map_err(error)
}
fn temporary(w: &mut Work<'_>, width: usize, extra: &str, keys: Option<usize>) -> Result<String> {
    let mut definition = schema(width);
    if !extra.is_empty() {
        definition += &format!(",{extra}");
    }
    if let Some(keys) = keys {
        definition += &format!(",PRIMARY KEY({})", names("", keys));
    }
    w.allocate(definition, keys.is_some())
}

/// Bootstrap once against the old facts, before any source in the atomic batch changes.
pub(super) fn prepare(
    w: &mut Work<'_>,
    run: &mut session::Execution,
    potential: &mut BTreeSet<String>,
    storage: Option<&versions::Store>,
) -> Result<Vec<String>> {
    if run.reductions.is_none() {
        run.reductions = Some(planned(w.db, run, storage.is_some())?);
    }
    loop {
        let before = potential.len();
        for (step, q) in run.emitted.program.steps.iter().zip(&run.emitted.queries) {
            if q.as_ref().is_some_and(|q| {
                q.reads()
                    .iter()
                    .filter_map(|r| r.program_relation())
                    .any(|r| potential.contains(r))
            }) {
                potential.insert(step.output.clone());
            }
        }
        if potential.len() == before {
            break;
        }
    }
    let mut initialized = Vec::new();
    for state in run.reductions.as_mut().unwrap().values_mut() {
        if !state.ready && potential.contains(&state.output) {
            let _context = w.profile_scope(
                || json!({"stage":"initialize retained support","relation":state.output}),
            );
            let _cpu = super::super::logic::profile::span(|| "initialize retained support".into());
            let operation = w.profiler.map(|p| {
                p.operation(
                    "initialize retained support",
                    ("sql-extension", 0),
                    0,
                    vec![],
                )
            });
            state.initialize(w.db, &mut run.objects, storage)?;
            if let Some(operation) = operation {
                operation.finish_unobserved();
            }
            state.ready = true;
            initialized.push(state.output.clone());
        }
    }
    Ok(initialized)
}
fn planned(
    db: &rusqlite::Connection,
    run: &session::Execution,
    native: bool,
) -> Result<BTreeMap<String, Retained>> {
    let e = Arc::clone(&run.emitted);
    let mut positions = BTreeMap::<String, u32>::new();
    for s in &e.program.steps {
        positions
            .entry(s.output.clone())
            .and_modify(|n| *n = (*n).max(s.stratum))
            .or_insert(s.stratum);
    }
    let mut retained = BTreeMap::new();
    for (output, width) in &e.program.relations {
        let lattice =
            e.program
                .lattices
                .iter()
                .find(|(r, _, _)| r == output)
                .map(|(_, keys, top)| lattice::Lattice {
                    keys: *keys,
                    top: *top,
                    aggregate: e
                        .program
                        .lattice_operations
                        .get(output)
                        .map(|o| o.aggregate.clone()),
                    tree: e
                        .program
                        .lattice_operations
                        .get(output)
                        .filter(|o| !matches!(o.aggregate.as_str(), "min" | "max"))
                        .map(|o| join_tree::JoinTree {
                            nodes: format!("__sql_state_{output}_join_nodes"),
                            roots: format!("__sql_state_{output}_join_roots"),
                            merge: o.merge.clone(),
                            keys: *keys,
                            top: *top,
                        }),
                });
        let Some(stratum) = positions.get(output) else {
            continue;
        };
        if *stratum > 0 && positions.iter().any(|(r, n)| r != output && n == stratum) {
            continue;
        }
        let steps: Vec<_> = e
            .program
            .steps
            .iter()
            .zip(&e.queries)
            .enumerate()
            .filter(|(_, (s, _))| s.output == *output)
            .collect();
        let mut rules = Vec::new();
        let mut nonmonotone = lattice.is_some();
        let mut eligible = true;
        for (id, (_, q)) in steps {
            let Some(q) = q else {
                eligible = false;
                break;
            };
            if q.reads()
                .iter()
                .any(|r| r.program_relation() == Some(output.as_str()))
            {
                eligible = false;
                break;
            }
            match plan::rule(db, output, id, q, native)? {
                Some(rule) => {
                    nonmonotone |= !rule.reducers.is_empty();
                    rules.push(rule);
                }
                None => {
                    eligible = false;
                    break;
                }
            }
        }
        if !eligible || (!nonmonotone && !native) {
            continue;
        }
        let state = Retained {
            output: output.clone(),
            heads: format!("__sql_state_{output}_heads"),
            width: *width,
            rules,
            lattice,
            ready: false,
        };
        retained.insert(output.clone(), state);
    }
    Ok(retained)
}

/// Rebuild checked descriptors after a copy-on-write fork. The support rows are already shared.
pub(super) fn fork_plan(db: &rusqlite::Connection, run: &mut session::Execution) -> Result<()> {
    let mut states = planned(db, run, true)?;
    for state in states.values_mut() {
        state.ready = true;
    }
    run.reductions = Some(states);
    Ok(())
}

impl Retained {
    pub(super) fn rebuildable(&self) -> bool {
        self.rules.iter().all(|r| r.reducers.is_empty())
            && self.lattice.as_ref().is_none_or(|l| l.tree.is_none())
    }
    fn initialize(
        &self,
        db: &rusqlite::Connection,
        objects: &mut Vec<(String, bool)>,
        storage: Option<&versions::Store>,
    ) -> Result<()> {
        owned(
            db,
            objects,
            &self.heads,
            &format!(
                "{},n INTEGER NOT NULL CHECK(n>=0),PRIMARY KEY({})",
                schema(self.width),
                names("", self.width)
            ),
            storage,
        )?;
        for rule in &self.rules {
            for reducer in &rule.reducers {
                reducer.initialize(db, objects, storage)?;
            }
            owned(
                db,
                objects,
                &rule.contexts,
                &format!(
                    "{},PRIMARY KEY({})",
                    schema(rule.width),
                    names("", rule.width)
                ),
                storage,
            )?;
            insert(db, &rule.contexts, &rule.context_query.render(), storage)?;
            let hcols = (0..self.width)
                .map(|i| format!("h{i} INTEGER NOT NULL"))
                .collect::<Vec<_>>()
                .join(",");
            owned(
                db,
                objects,
                &rule.contributions,
                &format!(
                    "{},{hcols},PRIMARY KEY({})",
                    schema(rule.width),
                    names("", rule.width)
                ),
                storage,
            )?;
            insert(
                db,
                &rule.contributions,
                &rule.evaluation(&rule.contexts).render(),
                storage,
            )?;
            let heads = (0..self.width)
                .map(|i| format!("h{i}"))
                .collect::<Vec<_>>()
                .join(",");
            if storage.is_none() {
                db.execute_batch(&format!(
                    "CREATE INDEX {} ON {}({heads})",
                    quote(&format!("{}_heads", rule.contributions)),
                    quote(&rule.contributions)
                ))
                .map_err(error)?;
            } else if let Some(source) =
                super::super::facts::vtab::bound_source(db, &rule.contributions)
            {
                source.prepare_index(&(rule.width..rule.width + self.width).collect::<Vec<_>>());
            }
            for atom in &rule.atoms {
                rule.index(
                    db,
                    &(atom.offset..atom.offset + atom.columns.len()).collect::<Vec<_>>(),
                )?;
            }
            for reducer in &rule.reducers {
                reducer.index_context(db, rule, storage)?;
                if let Some(table) = reducer.dependency_object() {
                    objects.push((table, true));
                }
            }
            if storage.is_some() {
                let hjoin = (0..self.width)
                    .map(|i| format!("h.c{i}=d.h{i}"))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                if storage.is_some_and(|s| s.version(&self.heads).len() == 0) {
                    insert(
                        db,
                        &self.heads,
                        &format!(
                            "SELECT {heads},count(*) FROM {} GROUP BY {heads}",
                            rule.contributions
                        ),
                        storage,
                    )?;
                } else {
                    execute(
                        db,
                        &format!(
                            "INSERT OR REPLACE INTO {} SELECT {heads},count(*)+coalesce((SELECT h.n FROM {} h WHERE {hjoin}),0) FROM {} d GROUP BY {heads}",
                            self.heads, self.heads, rule.contributions
                        ),
                    )?;
                }
            } else {
                execute(
                    db,
                    &format!(
                        "INSERT INTO {} SELECT {heads},count(*) FROM {} GROUP BY {heads} ON CONFLICT({}) DO UPDATE SET n=n+excluded.n",
                        self.heads,
                        rule.contributions,
                        names("", self.width)
                    ),
                )?;
            }
        }
        if let Some(tree) = self.lattice.as_ref().and_then(|l| l.tree.as_ref()) {
            tree.initialize(db, &self.heads, objects, storage)?;
        }
        Ok(())
    }
    pub(super) fn apply(
        &self,
        w: &mut Work<'_>,
        changes: &BTreeMap<String, Delta>,
    ) -> Result<(Delta, u64, u64)> {
        maintain::apply(self, w, changes)
    }
}
