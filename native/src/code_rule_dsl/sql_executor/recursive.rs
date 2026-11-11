//! Restart only proven independent recursive partitions, preserving cold wave semantics.
use super::super::logic::sql::{
    select::{FromItem, Join, Query, Relation as Rel, Select},
    sql_ast::{SqlBinaryOperator as B, SqlExpression as E},
};
use super::maintenance::{Delta, Work};
use super::*;
mod proof;
mod seeds;

#[derive(Clone)]
pub(super) struct Partitioned {
    columns: BTreeMap<String, usize>,
    rules: Vec<(schedule::Step, Query)>,
    steps: Vec<(schedule::Step, Query)>,
    keys: BTreeMap<String, usize>,
    fields: BTreeMap<String, Vec<String>>,
}
fn field(alias: &str, at: usize) -> E {
    E::Qualified {
        alias: alias.into(),
        column: format!("c{at}"),
    }
}
fn plain(e: &E) -> bool {
    let mut yes = true;
    e.visit(&mut |e| yes &= !matches!(e, E::Scalar(_) | E::Exists { .. } | E::Parameter(_)));
    yes
}
fn equal(a: E, b: E) -> E {
    E::Binary {
        operator: B::Equal,
        left: Box::new(a),
        right: Box::new(b),
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
fn fields(db: &rusqlite::Connection, r: &Rel) -> Result<Vec<String>> {
    Ok(db
        .prepare(&format!("SELECT * FROM {} LIMIT 0", r.render()))
        .map_err(error)?
        .column_names()
        .map(str::to_owned)
        .collect())
}
fn source(table: &str, columns: &[String]) -> Rel {
    Rel::Bound {
        rel: table.into(),
        table: format!(
            "(SELECT {} FROM {})",
            columns
                .iter()
                .enumerate()
                .map(|(i, c)| format!("c{i} AS {}", quote(c)))
                .collect::<Vec<_>>()
                .join(","),
            quote(table)
        ),
    }
}

pub(super) fn prepare(db: &rusqlite::Connection, run: &mut session::Execution) -> Result<()> {
    let e = &run.emitted;
    let widths = e
        .program
        .relations
        .iter()
        .cloned()
        .collect::<BTreeMap<_, _>>();
    let mut positions = BTreeMap::<String, u32>::new();
    for step in &e.program.steps {
        positions
            .entry(step.output.clone())
            .and_modify(|s| *s = (*s).max(step.stratum))
            .or_insert(step.stratum);
    }
    let mut groups = BTreeMap::<u32, BTreeSet<String>>::new();
    for (r, s) in positions {
        if s > 0 {
            groups.entry(s).or_default().insert(r);
        }
    }
    for (stratum, group) in groups {
        if run.temporal.contains_key(&stratum) {
            continue;
        }
        let rules = e
            .program
            .steps
            .iter()
            .zip(&e.queries)
            .filter(|(s, _)| group.contains(&s.output))
            .filter_map(|(s, q)| q.as_ref().map(|q| (s.clone(), q.clone())))
            .filter(|(_, q)| {
                !q.reads()
                    .iter()
                    .any(|r| matches!(r, Rel::Derived { delta: true, .. }))
            })
            .collect::<Vec<_>>();
        let recursive = rules.iter().any(|(_, q)| {
            q.reads()
                .iter()
                .filter_map(|r| r.program_relation())
                .any(|r| group.contains(r))
        });
        let nonmonotone = group
            .iter()
            .any(|r| e.program.lattices.iter().any(|(n, _, _)| n == r))
            || rules.iter().any(|(_, q)| maintenance::barrier(q));
        if !recursive
            || !nonmonotone
            || rules.is_empty()
            || rules.iter().any(|(_, q)| !q.ctes.is_empty())
        {
            continue;
        }
        let keys = group
            .iter()
            .map(|r| {
                (
                    r.clone(),
                    e.program
                        .lattices
                        .iter()
                        .find(|(n, _, _)| n == r)
                        .map_or(widths[r], |(_, keys, _)| *keys),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let Some(columns) = proof::partition(&rules, &keys) else {
            continue;
        };
        let mut inputs = BTreeMap::new();
        for (_, q) in &rules {
            for r in q.reads() {
                if let Some(name) = r.program_relation() {
                    inputs.entry(name.to_string()).or_insert(fields(db, r)?);
                }
            }
        }
        for (r, &col) in &columns {
            if col > 0 {
                session::index(db, r, &[col as u32]).map_err(error)?;
            }
            if col > 0
                && e.program
                    .lattice_operations
                    .get(r)
                    .is_some_and(|o| o.after.is_some())
            {
                session::index(db, &format!("__sql_changes_{r}"), &[col as u32]).map_err(error)?;
            }
        }
        let steps = e
            .program
            .steps
            .iter()
            .zip(&e.queries)
            .filter(|(s, _)| group.contains(&s.output))
            .map(|(s, q)| {
                (
                    s.clone(),
                    q.as_ref()
                        .expect("constant steps have checked queries")
                        .clone(),
                )
            })
            .collect();
        run.recursive.insert(
            stratum,
            Partitioned {
                columns,
                rules,
                steps,
                keys,
                fields: inputs,
            },
        );
    }
    Ok(())
}

impl Partitioned {
    pub fn apply(
        &self,
        w: &mut Work<'_>,
        run: &session::Execution,
        changes: &BTreeMap<String, Delta>,
        limit: u32,
    ) -> Result<(BTreeMap<String, Delta>, u64, u64)> {
        let parts = w.table(1)?;
        self.seed(w, changes, &parts, false)?;
        self.seed(w, changes, &parts, true)?;
        let partitions =
            w.db.query_row(&format!("SELECT count(*) FROM {parts}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(error)? as u64;
        let widths = run
            .emitted
            .program
            .relations
            .iter()
            .cloned()
            .collect::<BTreeMap<_, _>>();
        let mut old = BTreeMap::new();
        for (r, &col) in &self.columns {
            let table = w.table(widths[r])?;
            w.db.execute(&format!("INSERT INTO {table} SELECT {} FROM {parts} p CROSS JOIN {} r WHERE r.c{col}=p.c0",names("r",widths[r]),quote(r)),[]).map_err(error)?;
            w.db.execute(
                &format!(
                    "DELETE FROM {} WHERE c{col} IN(SELECT c0 FROM {parts})",
                    quote(r)
                ),
                [],
            )
            .map_err(error)?;
            if run
                .emitted
                .program
                .lattice_operations
                .get(r)
                .is_some_and(|o| o.after.is_some())
            {
                w.db.execute(
                    &format!(
                        "DELETE FROM __sql_changes_{r} WHERE c{col} IN(SELECT c0 FROM {parts})"
                    ),
                    [],
                )
                .map_err(error)?;
            }
            old.insert(r.clone(), table);
        }
        let mut p = run.emitted.program.clone();
        p.relations.retain(|(r, _)| self.columns.contains_key(r));
        p.lattices.retain(|(r, _, _)| self.columns.contains_key(r));
        p.lattice_operations
            .retain(|r, _| self.columns.contains_key(r));
        p.steps.clear();
        for (step, q) in &self.steps {
            let mut step = step.clone();
            let mut q = q.clone();
            let part = q.body.columns[self.columns[&step.output]].0.clone();
            q.body.from.insert(
                0,
                FromItem::cross(
                    Rel::Bound {
                        rel: parts.clone(),
                        table: quote(&parts),
                    },
                    "rr_partition",
                ),
            );
            for target in proof::targets(&q.body, &part) {
                q.body
                    .predicates
                    .push(equal(target, field("rr_partition", 0)));
            }
            for item in &mut q.body.from {
                if let Join::On(on) = std::mem::replace(&mut item.join, Join::Cross) {
                    q.body.predicates.extend(on);
                }
            }
            step.sql = format!("INSERT OR IGNORE INTO new_{} {}", step.output, q.render());
            // Keep the cold kernel's exact step order, dependencies, and delta variants.
            // Closed partitions change only the rows these statements can produce.
            p.steps.push(step);
        }
        let waves = if partitions > 0 {
            schedule::execute_existing(w.db, &p, limit).map_err(error)?
        } else {
            0
        };
        let mut result = BTreeMap::new();
        for (r, &col) in &self.columns {
            let now = w.table(widths[r])?;
            w.db.execute(&format!("INSERT INTO {now} SELECT {} FROM {parts} p CROSS JOIN {} r WHERE r.c{col}=p.c0",names("r",widths[r]),quote(r)),[]).map_err(error)?;
            result.insert(r.clone(), w.difference(&now, &old[r], widths[r])?);
        }
        Ok((result, partitions, u64::from(waves)))
    }
}
