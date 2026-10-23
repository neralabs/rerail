//! Render generated SQL from indexed dependencies, without rescanning every prior definition.
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

fn tokens(sql: &str) -> impl Iterator<Item = &str> {
    static IDENTIFIER: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"[A-Za-z_][A-Za-z_0-9]*").expect("generated identifier expression")
    });
    IDENTIFIER.find_iter(sql).map(|token| token.as_str())
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
