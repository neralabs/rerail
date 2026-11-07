//! One indexed SQL owner per execution domain; logical bindings have independent lifetimes.
use super::{
    lifetime::{self, Dependencies, Input},
    render::Definitions,
    waves,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const STORE: &str = "candidate_keys";
#[derive(Default)]
pub struct Layout {
    pub setup: Vec<String>,
    pub slots: Vec<Option<usize>>,
    pub members: Vec<Vec<usize>>,
}
impl Layout {
    pub fn slot(&self, i: usize) -> Option<usize> {
        self.slots.get(i).copied().flatten()
    }
    pub fn members(&self, i: usize) -> &[usize] {
        self.members.get(i).map(Vec::as_slice).unwrap_or(&[])
    }
    /// The last-use boundary is logical, even when its construction shares a statement.
    pub fn release(&self, names: &[&str], indices: &[usize], populated: &[bool]) -> String {
        let mut slots = Vec::new();
        let mut sql = String::new();
        for &i in indices {
            if let Some(slot) = self.slot(i) {
                if populated[i] {
                    slots.push(slot.to_string());
                }
            } else {
                sql.push_str(&format!("DROP TABLE temp.\"{}\";", names[i]));
            }
        }
        if !slots.is_empty() {
            sql.push_str(&format!(
                "DELETE FROM {STORE} WHERE producer IN ({});",
                slots.join(",")
            ));
        }
        sql
    }
}

pub struct Compiled {
    pub layout: Layout,
    pub sql: Vec<String>,
    pub queries: Vec<String>,
    pub schedule: lifetime::Schedule,
}

/// Generated references are identifiers; user strings remain bound values.
fn read(sql: &str, aliases: &HashMap<String, String>) -> String {
    let names: BTreeSet<_> = waves::identifiers(sql)
        .filter(|name| aliases.contains_key(*name))
        .collect();
    if names.is_empty() {
        return sql.into();
    }
    let views = names
        .into_iter()
        .map(|name| aliases[name].as_str())
        .collect::<Vec<_>>()
        .join(",");
    if let Some(tail) = sql
        .strip_prefix("WITH RECURSIVE ")
        .or_else(|| sql.strip_prefix("WITH "))
    {
        format!("WITH RECURSIVE {views},{tail}")
    } else {
        format!("WITH RECURSIVE {views} {sql}")
    }
}

pub fn compile(
    inputs: &[Input<'_>],
    indexes: &[Option<&str>],
    queries: &[String],
    graph: &[String],
) -> Compiled {
    let dependencies = Dependencies::new(inputs);
    let pairs: BTreeSet<_> = graph
        .iter()
        .filter_map(|cte| {
            let (name, tail) = cte.split_once('(').expect("generated columns");
            tail.starts_with("fid,pre) AS ").then_some(name)
        })
        .collect();
    let physical: BTreeSet<_> = inputs
        .iter()
        .filter(|s| s.temporary)
        .map(|s| s.name.to_owned())
        .collect();
    let mut layout = Layout {
        slots: vec![None; inputs.len()],
        members: (0..inputs.len()).map(|i| vec![i]).collect(),
        ..Layout::default()
    };
    let mut aliases = HashMap::new();
    for (i, input) in inputs.iter().enumerate() {
        if input.temporary && indexes[i] == Some("fid,pre") && pairs.contains(input.name) {
            layout.slots[i] = Some(i);
            aliases.insert(input.name.to_owned(), format!("{}(fid,pre) AS NOT MATERIALIZED (SELECT fid,pre FROM {STORE} WHERE producer={i})",input.name));
        }
    }
    if !aliases.is_empty() {
        layout.setup = vec![
            format!(
                "CREATE TEMP TABLE \"{STORE}\"(producer INTEGER NOT NULL,fid INTEGER,pre INTEGER)"
            ),
            format!("CREATE INDEX temp.\"{STORE}_keys\" ON \"{STORE}\"(producer,fid,pre)"),
        ];
    }
    // Use compiler expansion costs, with each physical cut resetting its dependency cost.
    // One WITH scope shares common inline sources. No member has a private rendered tree.
    let mut expansion = HashMap::new();
    let mut costs = HashMap::new();
    for cte in graph {
        let (name, _) = cte.split_once('(').expect("generated columns");
        let body = cte.split_once(" AS ").expect("generated body").1;
        let cost = waves::identifiers(body)
            .filter_map(|name| expansion.get(name))
            .fold(1usize, |n, c| n.saturating_add(*c));
        costs.insert(name, cost);
        expansion.insert(name, if physical.contains(name) { 1 } else { cost });
    }
    let mut depths = vec![0; inputs.len()];
    let mut frontiers = BTreeMap::<usize, Vec<usize>>::new();
    let mut groups = Vec::new();
    for (i, _) in inputs.iter().enumerate() {
        depths[i] = dependencies.prerequisites[i]
            .iter()
            .map(|&p| depths[p] + 1)
            .max()
            .unwrap_or(0);
        if layout.slot(i).is_some() {
            frontiers.entry(depths[i]).or_default().push(i);
        } else {
            groups.push(vec![i]);
        }
    }
    const PREPARATION_UNITS: usize = 128;
    for ready in frontiers.values() {
        let mut batch = Vec::new();
        let mut units = 0usize;
        for &i in ready {
            let cost = costs[inputs[i].name];
            if !batch.is_empty() && units.saturating_add(cost) > PREPARATION_UNITS {
                groups.push(std::mem::take(&mut batch));
                units = 0;
            }
            batch.push(i);
            units = units.saturating_add(cost);
        }
        if !batch.is_empty() {
            groups.push(batch);
        }
    }
    groups.sort_by_key(|members| members[0]);
    let schedule = lifetime::Schedule::batched(inputs, queries, dependencies, &groups);
    let definitions = Definitions::new(graph.iter().cloned());
    let mut sql: Vec<_> = inputs
        .iter()
        .enumerate()
        .map(|(i, input)| {
            if layout.slot(i).is_some() {
                String::new()
            } else if input.temporary {
                let (head, body) = input.sql.split_once(" AS ").expect("generated CTAS");
                format!("{head} AS {}", read(body, &aliases))
            } else {
                read(input.sql, &aliases)
            }
        })
        .collect();
    for members in groups {
        let leader = members[0];
        if layout.slot(leader).is_none() {
            continue;
        }
        let names: BTreeSet<_> = members.iter().map(|&i| inputs[i].name).collect();
        let arms: Vec<_> = members
            .iter()
            .map(|&i| format!("SELECT {i} AS producer,fid,pre FROM {}", inputs[i].name))
            .collect();
        let body = format!(
            "INSERT INTO {STORE} SELECT producer,CAST(fid AS INTEGER),CAST(pre AS INTEGER) FROM ({})",
            waves::compound("producer,fid,pre", arms)
        );
        sql[leader] = definitions.render_cuts(&body, &physical, &names, &aliases, &[]);
        for &i in &members[1..] {
            sql[i].clear();
            layout.members[i].clear();
        }
        layout.members[leader] = members;
    }
    Compiled {
        layout,
        sql,
        queries: queries.iter().map(|q| read(q, &aliases)).collect(),
        schedule,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, Result};
    fn plan(graph: &[&str], outputs: &[&str]) -> (Compiled, Vec<String>) {
        let graph: Vec<String> = graph.iter().map(|s| s.to_string()).collect();
        let names: Vec<String> = graph
            .iter()
            .map(|s| s.split_once('(').unwrap().0.into())
            .collect();
        let sources: Vec<_> = names
            .iter()
            .map(|n| {
                Definitions::new(graph.iter().cloned()).render(&format!(
                    "SELECT CAST(fid AS INTEGER) AS fid,CAST(pre AS INTEGER) AS pre FROM {n}"
                ))
            })
            .collect();
        let sql: Vec<_> = names
            .iter()
            .zip(sources)
            .map(|(n, s)| format!("CREATE TEMP TABLE \"{n}\" AS {s}"))
            .collect();
        let inputs: Vec<_> = names
            .iter()
            .zip(&sql)
            .map(|(n, s)| Input {
                name: n,
                sql: s,
                temporary: true,
            })
            .collect();
        (
            compile(
                &inputs,
                &vec![Some("fid,pre"); inputs.len()],
                &outputs.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                &graph,
            ),
            names,
        )
    }
    type Row = (i64, Option<i64>, Option<i64>);
    fn run(db: &Connection, p: &Compiled, names: &[String]) -> Result<Vec<Row>> {
        for sql in &p.layout.setup {
            db.execute_batch(sql)?;
        }
        let mut rows = Vec::new();
        let mut populated = vec![false; p.sql.len()];
        for actions in p.schedule.actions.chunk_by(|a, b| {
            matches!(a, lifetime::Action::Release(_)) && matches!(b, lifetime::Action::Release(_))
        }) {
            match actions[0] {
                lifetime::Action::Prepare(i) => {
                    db.execute_batch(&p.sql[i])?;
                    for &member in p.layout.members(i) {
                        populated[member] = db.changes() != 0;
                    }
                }
                lifetime::Action::Query(q) => {
                    rows.extend(
                        db.prepare(&p.queries[q])?
                            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                            .collect::<Result<Vec<_>>>()?,
                    );
                }
                lifetime::Action::Release(_) => {
                    let ids = actions
                        .iter()
                        .map(|a| {
                            if let lifetime::Action::Release(i) = a {
                                *i
                            } else {
                                unreachable!()
                            }
                        })
                        .collect::<Vec<_>>();
                    db.execute_batch(&p.layout.release(
                        &names.iter().map(String::as_str).collect::<Vec<_>>(),
                        &ids,
                        &populated,
                    ))?;
                }
            }
        }
        assert_eq!(
            db.query_row("SELECT count(*) FROM candidate_keys", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        db.execute_batch("DROP TABLE candidate_keys")?;
        Ok(rows)
    }
    #[test]
    fn independent_producers_preserve_bags_nulls_integer_affinity_and_empty_completion()
    -> Result<()> {
        let db = Connection::open_in_memory()?;
        let (p, names) = plan(
            &[
                "a(fid,pre) AS MATERIALIZED (SELECT '1','7' UNION ALL SELECT '1','7' UNION ALL SELECT NULL,NULL)",
                "b(fid,pre) AS MATERIALIZED (SELECT 1,8)",
                "empty(fid,pre) AS MATERIALIZED (SELECT 1,9 WHERE 0)",
            ],
            &[
                "SELECT 0,fid,pre FROM a UNION ALL SELECT 1,fid,pre FROM b UNION ALL SELECT 2,fid,pre FROM empty ORDER BY 1,2,3",
            ],
        );
        assert_eq!(
            p.schedule
                .actions
                .iter()
                .filter(|a| matches!(a, lifetime::Action::Prepare(_)))
                .count(),
            1
        );
        assert_eq!(p.layout.setup.len(), 2);
        assert!(p.layout.setup[1].contains("(producer,fid,pre)"));
        assert!(!p.layout.setup[1].contains("UNIQUE"));
        assert_eq!(
            run(&db, &p, &names)?,
            vec![
                (0, None, None),
                (0, Some(1), Some(7)),
                (0, Some(1), Some(7)),
                (1, Some(1), Some(8))
            ]
        );
        // A new domain gets fresh completion and rows after the prior owner is gone.
        assert_eq!(run(&db, &p, &names)?.len(), 4);
        Ok(())
    }
    #[test]
    fn physical_operands_are_read_once_without_reconstruction_and_retire_individually() -> Result<()>
    {
        let db = Connection::open_in_memory()?;
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        db.create_scalar_function(
            c"produce",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(7)
            },
        )?;
        let (p, names) = plan(
            &[
                "seed(fid,pre) AS MATERIALIZED (SELECT 1,produce())",
                "early(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM seed)",
                "late(fid,pre) AS MATERIALIZED (SELECT fid,pre+1 FROM seed)",
            ],
            &["SELECT 0,fid,pre FROM early", "SELECT 1,fid,pre FROM late"],
        );
        assert_eq!(
            run(&db, &p, &names)?,
            vec![(0, Some(1), Some(7)), (1, Some(1), Some(8))]
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        let at = |needle| {
            p.schedule
                .actions
                .iter()
                .position(|a| *a == needle)
                .unwrap()
        };
        assert!(at(lifetime::Action::Release(1)) < at(lifetime::Action::Query(1)));
        Ok(())
    }
    #[test]
    fn shared_inline_sources_construct_once_and_indexed_interval_reads_keep_both_bounds()
    -> Result<()> {
        let db = Connection::open_in_memory()?;
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        db.create_scalar_function(
            c"produce",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(7)
            },
        )?;
        let graph = vec![
            "seed(fid,pre) AS MATERIALIZED (SELECT 1,produce())".into(),
            "a(fid,pre) AS MATERIALIZED (SELECT * FROM seed)".into(),
            "b(fid,pre) AS MATERIALIZED (SELECT fid,pre+1 FROM seed)".into(),
        ];
        let sql = Definitions::new(graph.iter().cloned())
            .render("SELECT * FROM a UNION ALL SELECT * FROM b");
        let inputs = [
            Input {
                name: "a",
                sql: &sql,
                temporary: true,
            },
            Input {
                name: "b",
                sql: &sql,
                temporary: true,
            },
        ];
        // The compiler normally supplies each root's own live dependency closure.
        let roots: Vec<_> = ["a", "b"]
            .iter()
            .map(|name| {
                Definitions::new(graph.iter().cloned()).render(&format!("SELECT * FROM {name}"))
            })
            .collect();
        let inputs: Vec<_> = inputs
            .iter()
            .zip(&roots)
            .map(|(i, sql)| Input {
                name: i.name,
                sql,
                temporary: true,
            })
            .collect();
        let p = compile(
            &inputs,
            &[Some("fid,pre"), Some("fid,pre")],
            &["SELECT 0,fid,pre FROM a UNION ALL SELECT 1,fid,pre FROM b".into()],
            &graph,
        );
        assert_eq!(run(&db, &p, &["a".into(), "b".into()])?.len(), 2);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        for sql in &p.layout.setup {
            db.execute_batch(sql)?;
        }
        db.execute_batch("CREATE TABLE bounds(fid INTEGER,lo INTEGER,hi INTEGER)")?;
        let query=read("SELECT s.fid,s.pre FROM bounds r CROSS JOIN a s WHERE s.fid=r.fid AND s.pre>=r.lo AND s.pre<r.hi",&HashMap::from([("a".into(),"a(fid,pre) AS NOT MATERIALIZED (SELECT fid,pre FROM candidate_keys WHERE producer=0)".into())]));
        let detail = db
            .prepare(&format!("EXPLAIN QUERY PLAN {query}"))?
            .query_map([], |r| r.get::<_, String>(3))?
            .collect::<Result<Vec<_>>>()?
            .join("\n");
        assert!(
            detail.contains("producer=? AND fid=? AND pre>? AND pre<?"),
            "{detail}"
        );
        Ok(())
    }
}

#[cfg(test)]
mod retirement_tests {
    #[test]
    fn empty_batches_need_no_delete_but_nonempty_members_keep_their_last_use_retirement() {
        let layout = super::Layout {
            slots: vec![Some(0), Some(1)],
            ..Default::default()
        };
        assert_eq!(layout.release(&["a", "b"], &[0, 1], &[false, false]), "");
        assert_eq!(
            layout.release(&["a", "b"], &[0, 1], &[false, true]),
            "DELETE FROM candidate_keys WHERE producer IN (1);"
        );
    }
}

/// Logical release is immediate. Physical retirement is one bounded cohort delete;
/// the final owner DROP discards the remaining retired rows without deleting them first.
#[derive(Default)]
pub struct Retirement {
    pending: BTreeSet<usize>,
    rows: usize,
}
pub const RETIREMENT_ROWS: usize = 131_072;
impl Retirement {
    pub fn inserted(&mut self, rows: usize) {
        self.rows = self.rows.saturating_add(rows);
    }
    pub fn release(
        &mut self,
        layout: &Layout,
        names: &[&str],
        indices: &[usize],
        populated: &[bool],
    ) -> String {
        let mut private = Vec::new();
        for &i in indices {
            if let Some(slot) = layout.slot(i) {
                if populated[i] {
                    self.pending.insert(slot);
                }
            } else {
                private.push(i);
            }
        }
        layout.release(names, &private, populated)
    }
    pub fn before_prepare(&self) -> Option<String> {
        (self.rows >= RETIREMENT_ROWS && !self.pending.is_empty()).then(|| {
            format!(
                "DELETE FROM {STORE} WHERE producer IN ({})",
                self.pending
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
    }
    pub fn retired(&mut self, rows: usize) {
        self.rows = self.rows.saturating_sub(rows);
        self.pending.clear();
    }
}

#[cfg(test)]
mod cohort_tests {
    use super::*;
    #[test]
    fn defers_small_retirements_flushes_under_row_pressure_and_keeps_live_slots()
    -> rusqlite::Result<()> {
        let db = rusqlite::Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE candidate_keys(producer INTEGER,fid INTEGER,pre INTEGER);WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<131072) INSERT INTO candidate_keys SELECT 0,1,x FROM n;INSERT INTO candidate_keys VALUES(1,1,7)")?;
        let layout = Layout {
            slots: vec![Some(0), Some(1)],
            ..Default::default()
        };
        let mut retirement = Retirement::default();
        retirement.inserted(1);
        assert_eq!(
            retirement.release(&layout, &["a", "b"], &[0], &[true, true]),
            ""
        );
        assert!(retirement.before_prepare().is_none());
        retirement.inserted(RETIREMENT_ROWS);
        let deleted = db.execute(&retirement.before_prepare().unwrap(), [])?;
        retirement.retired(deleted);
        assert_eq!(deleted, RETIREMENT_ROWS);
        assert_eq!(
            db.query_row("SELECT producer FROM candidate_keys", [], |r| r
                .get::<_, i64>(0))?,
            1
        );
        assert!(retirement.before_prepare().is_none());
        assert_eq!(
            retirement.release(&layout, &["a", "b"], &[1], &[true, true]),
            ""
        );
        db.execute_batch("DROP TABLE candidate_keys")?;
        Ok(())
    }
}
