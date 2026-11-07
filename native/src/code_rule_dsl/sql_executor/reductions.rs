//! Retained nonrecursive rule contexts and invertible scalar reducers.
//! Checked query trees determine eligibility. Unsupported shapes use the ordinary stratum refresh.
use super::super::logic::sql::{
    select::{FromItem, Join, Query, Relation as Rel, Select},
    sql_ast::{SqlBinaryOperator as B, SqlExpression as E, SqlValue as V},
};
use super::maintenance::{Delta, Work};
use super::*;
mod maintain;
mod plan;
mod reducer;
use plan::{Rule, column, integer, names, plain, source};
use reducer::Reducer;

#[derive(Clone)]
pub(super) struct Retained {
    output: String,
    heads: String,
    width: usize,
    rules: Vec<Rule>,
    pub(super) ready: bool,
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
    .map_err(error)
}

fn owned(
    db: &rusqlite::Connection,
    objects: &mut Vec<(String, bool)>,
    name: &str,
    definition: &str,
) -> Result<()> {
    db.execute_batch(&format!(
        "CREATE TEMP TABLE {}({definition}) WITHOUT ROWID",
        quote(name)
    ))
    .map_err(error)?;
    objects.push((name.into(), true));
    Ok(())
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
    let name = format!("__rr_update_{}", w.sequence);
    w.sequence += 1;
    let mut definition = schema(width);
    if !extra.is_empty() {
        definition += &format!(",{extra}");
    }
    if let Some(keys) = keys {
        definition += &format!(",PRIMARY KEY({})", names("", keys));
    }
    w.db.execute_batch(&format!(
        "CREATE TEMP TABLE {name}({definition}){}",
        if keys.is_some() { " WITHOUT ROWID" } else { "" }
    ))
    .map_err(error)?;
    w.scratch.push(name.clone());
    Ok(name)
}

/// Bootstrap once against the old facts, before any source in the atomic batch changes.
pub(super) fn prepare(
    w: &mut Work<'_>,
    run: &mut session::Execution,
    potential: &mut BTreeSet<String>,
) -> Result<Vec<String>> {
    if run.reductions.is_none() {
        run.reductions = Some(planned(w.db, run)?);
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
            state.initialize(w.db, &mut run.objects)?;
            state.ready = true;
            initialized.push(state.output.clone());
        }
    }
    Ok(initialized)
}
fn planned(
    db: &rusqlite::Connection,
    run: &session::Execution,
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
        if e.program.lattices.iter().any(|(r, _, _)| r == output) {
            continue;
        }
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
        let mut nonmonotone = false;
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
            match plan::rule(db, output, id, q)? {
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
        if !eligible || !nonmonotone {
            continue;
        }
        let state = Retained {
            output: output.clone(),
            heads: format!("__sql_state_{output}_heads"),
            width: *width,
            rules,
            ready: false,
        };
        retained.insert(output.clone(), state);
    }
    Ok(retained)
}

impl Retained {
    fn initialize(
        &self,
        db: &rusqlite::Connection,
        objects: &mut Vec<(String, bool)>,
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
        )?;
        for rule in &self.rules {
            for reducer in &rule.reducers {
                reducer.initialize(db, objects)?;
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
            )?;
            execute(
                db,
                &format!(
                    "INSERT INTO {} {}",
                    rule.contexts,
                    rule.context_query.render()
                ),
            )?;
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
            )?;
            execute(
                db,
                &format!(
                    "INSERT INTO {} {}",
                    rule.contributions,
                    rule.evaluation(&rule.contexts).render()
                ),
            )?;
            let heads = (0..self.width)
                .map(|i| format!("h{i}"))
                .collect::<Vec<_>>()
                .join(",");
            db.execute_batch(&format!(
                "CREATE INDEX {} ON {}({heads})",
                quote(&format!("{}_heads", rule.contributions)),
                quote(&rule.contributions)
            ))
            .map_err(error)?;
            for atom in &rule.atoms {
                rule.index(
                    db,
                    &(atom.offset..atom.offset + atom.columns.len()).collect::<Vec<_>>(),
                )?;
            }
            for reducer in &rule.reducers {
                rule.index(db, &reducer.context_keys(rule))?;
            }
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
