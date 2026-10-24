//! Render generated SQL from indexed dependencies, without rescanning every prior definition.
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

fn tokens(sql: &str) -> impl Iterator<Item = &str> {
    static IDENTIFIER: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"[A-Za-z_][A-Za-z_0-9]*").expect("generated identifier expression")
    });
    IDENTIFIER.find_iter(sql).map(|token| token.as_str())
}

/// A private set's CTAS is already its physical boundary. Keep upstream shared CTEs
/// materialized, but stream this final projection directly into its owner.
pub(super) fn owned(mut sql: String, name: &str) -> String {
    let marker = format!("{name}(fid,pre) AS MATERIALIZED ");
    if let Some(start) = sql.find(&marker) {
        sql.insert_str(start + marker.len() - "MATERIALIZED ".len(), "NOT ");
    }
    sql
}

struct Entry {
    sql: String,
    dependencies: Vec<usize>,
}

#[derive(Default)]
pub(super) struct Definitions {
    entries: Vec<Entry>,
    names: HashMap<String, usize>,
}

impl Definitions {
    pub fn new(ctes: impl IntoIterator<Item = String>) -> Self {
        let mut definitions = Self::default();
        for cte in ctes {
            definitions.push(cte);
        }
        definitions
    }

    pub fn push(&mut self, sql: String) {
        let (name, _) = sql.split_once('(').expect("generated CTE columns");
        let body = sql.split_once(" AS ").expect("generated CTE body").1;
        let mut dependencies: Vec<_> = tokens(body)
            .filter_map(|token| self.names.get(token).copied())
            .collect();
        dependencies.sort_unstable();
        dependencies.dedup();
        self.names.insert(name.to_owned(), self.entries.len());
        self.entries.push(Entry { sql, dependencies });
    }

    pub fn pop(&mut self) {
        let entry = self.entries.pop().expect("generated CTE");
        let name = entry.sql.split_once('(').expect("generated CTE columns").0;
        self.names.remove(name);
    }

    pub fn live(&self, body: &str) -> Vec<&String> {
        let mut pending: Vec<_> = tokens(body)
            .filter_map(|token| self.names.get(token).copied())
            .collect();
        let mut live = BTreeSet::new();
        while let Some(index) = pending.pop() {
            if live.insert(index) {
                pending.extend(&self.entries[index].dependencies);
            }
        }
        live.into_iter().map(|i| &self.entries[i].sql).collect()
    }

    pub fn render(&self, body: &str) -> String {
        let live = self.live(body);
        if live.is_empty() {
            body.to_owned()
        } else {
            format!(
                "WITH RECURSIVE {} {body}",
                live.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }

    pub fn with_extra(&mut self, ctes: Vec<String>, body: &str) -> String {
        let count = ctes.len();
        for cte in ctes {
            self.push(cte);
        }
        let sql = self.render(body);
        for _ in 0..count {
            self.pop();
        }
        sql
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_owner_removes_a_private_ephemeral_copy_and_preserves_bags_and_nulls() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let before = "WITH s(fid,pre) AS MATERIALIZED (SELECT 1,7 UNION ALL SELECT 1,7 UNION ALL SELECT 1,NULL) SELECT CAST(fid AS INTEGER) AS fid,CAST(pre AS INTEGER) AS pre FROM s";
        let after = owned(before.into(), "s");
        assert_eq!(owned(after.clone(), "s"), after);
        let ops = |name: &str, body: &str| {
            db.prepare(&format!("EXPLAIN CREATE TEMP TABLE {name} AS {body}"))
                .unwrap()
                .query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        let before_ops = ops("before", before);
        let after_ops = ops("after", &after);
        assert_eq!(
            before_ops
                .iter()
                .filter(|s| s.as_str() == "OpenEphemeral")
                .count(),
            after_ops
                .iter()
                .filter(|s| s.as_str() == "OpenEphemeral")
                .count()
                + 1
        );
        db.execute_batch(&format!(
            "CREATE TEMP TABLE before AS {before};CREATE TEMP TABLE after AS {after}"
        ))
        .unwrap();
        let rows = |name: &str| {
            db.prepare(&format!("SELECT fid,pre FROM {name} ORDER BY fid,pre"))
                .unwrap()
                .query_map([], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(rows("before"), rows("after"));
        assert_eq!(rows("after"), vec![(1, None), (1, Some(7)), (1, Some(7))]);
    }
    #[test]
    fn physical_projection_keeps_the_shared_source_materialized_once() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        db.create_scalar_function(
            c"produce",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, Ordering::Relaxed);
                Ok(7)
            },
        )
        .unwrap();
        let sql=owned("WITH seed(fid,pre) AS MATERIALIZED (SELECT 1,produce()),output(fid,pre) AS MATERIALIZED (SELECT * FROM seed UNION ALL SELECT * FROM seed) SELECT * FROM output".into(),"output");
        assert!(sql.contains("seed(fid,pre) AS MATERIALIZED"));
        db.execute_batch(&format!("CREATE TEMP TABLE output AS {sql}"))
            .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            db.query_row("SELECT count(*) FROM output", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
    #[test]
    fn dependencies_preserve_order_and_ignore_unrelated_definitions() {
        let definitions = Definitions::new([
            "seed(x) AS MATERIALIZED (VALUES(1))".into(),
            "unused(x) AS MATERIALIZED (VALUES(2))".into(),
            "walk(x) AS MATERIALIZED (SELECT x FROM seed UNION SELECT x+1 FROM walk WHERE x<3)"
                .into(),
            "last(x) AS MATERIALIZED (SELECT x FROM walk)".into(),
        ]);
        let sql = definitions.render("SELECT * FROM last");
        assert!(!sql.contains("unused"));
        let db = rusqlite::Connection::open_in_memory().unwrap();
        assert_eq!(db.query_row(&sql, [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(definitions.live("SELECT * FROM last").len(), 3);
    }

    #[test]
    fn physical_cuts_and_temporary_definitions_do_not_retain_stale_dependencies() {
        let mut definitions = Definitions::new(["seed(x) AS MATERIALIZED (VALUES(1))".into()]);
        definitions.push("cut(x) AS MATERIALIZED (SELECT x FROM seed)".into());
        assert_eq!(definitions.live("SELECT * FROM cut").len(), 2);
        definitions.pop();
        definitions.push("output(x) AS MATERIALIZED (SELECT x FROM cut)".into());
        assert_eq!(definitions.live("SELECT * FROM output").len(), 1);
        let sql = definitions.with_extra(
            vec!["needed(x) AS MATERIALIZED (SELECT x FROM seed)".into()],
            "SELECT * FROM needed",
        );
        assert!(sql.contains("seed(x)"));
        assert!(!sql.contains("output(x)"));
        assert_eq!(
            definitions.render("SELECT * FROM needed"),
            "SELECT * FROM needed"
        );
    }
}
