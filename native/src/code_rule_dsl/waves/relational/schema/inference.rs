//! SQLite derives affinities for independent ready producers in one bounded CTAS.
use super::{Column, Columns, Registry, bind, cache, declaration, describe};
use rusqlite::{Connection, Result};
use std::sync::Arc;

struct Pending {
    name: String,
    query: String,
    columns: Vec<String>,
    key: cache::Key,
}

#[derive(Default)]
pub(crate) struct Inference {
    pending: Vec<Pending>,
    serial: usize,
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn bytes(query: &str, columns: &[String]) -> usize {
    query.len()
        + columns
            .iter()
            .map(|name| name.len() * 2 + 48)
            .sum::<usize>()
        + 128
}

// SQLite may initialize a shared CTE before checking its own LIMIT. The empty
// left operand prevents entering the entire bounded query, including that setup.
pub(crate) fn empty(query: &str) -> String {
    format!(
        "SELECT output.* FROM (SELECT 1 WHERE 0 LIMIT 0) guard CROSS JOIN (SELECT * FROM ({query}) LIMIT 0) output"
    )
}

impl Inference {
    pub(crate) fn before(
        &mut self,
        db: &Connection,
        schemas: &mut Registry,
        reads: &[String],
        query: &str,
        columns: &[String],
    ) -> Result<()> {
        let width = self.pending.iter().map(|p| p.columns.len()).sum::<usize>() + columns.len();
        let size = self
            .pending
            .iter()
            .map(|p| bytes(&p.query, &p.columns))
            .sum::<usize>()
            + bytes(query, columns);
        if !self.pending.is_empty()
            && (self.pending.len() == 16
                || width > 1500
                || size > 1024 * 1024 - 1024
                || reads
                    .iter()
                    .any(|name| self.pending.iter().any(|p| &p.name == name)))
        {
            self.flush(db, schemas)?;
        }
        Ok(())
    }

    pub(crate) fn queue(
        &mut self,
        db: &Connection,
        schemas: &mut Registry,
        name: &str,
        query: &str,
        columns: Vec<String>,
        key: cache::Key,
    ) -> Result<Arc<Columns>> {
        // Unusually wide queries retain SQLite's single-query inference boundary.
        if columns.len() > 1500 || bytes(query, &columns) > 1024 * 1024 - 1024 {
            self.flush(db, schemas)?;
            db.execute_batch(&format!(
                "CREATE TEMP TABLE {} AS {}",
                quote(name),
                empty(query)
            ))?;
            let actual = Arc::new(describe(db, name)?);
            cache::shared().insert(key, actual.clone());
            schemas.insert(name.to_owned(), actual.clone());
            return Ok(actual);
        }
        let provisional = Arc::new(
            columns
                .iter()
                .map(|name| Column {
                    name: name.clone(),
                    declared_type: String::new(),
                })
                .collect::<Columns>(),
        );
        bind(db, name, declaration(name, &provisional))?;
        schemas.insert(name.to_owned(), provisional.clone());
        self.pending.push(Pending {
            name: name.to_owned(),
            query: query.to_owned(),
            columns,
            key,
        });
        Ok(provisional)
    }

    pub(crate) fn flush(&mut self, db: &Connection, schemas: &mut Registry) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let mut projection = Vec::new();
        let mut sources = Vec::new();
        for (i, stage) in self.pending.iter().enumerate() {
            sources.push(format!("({}) q{i}", stage.query));
            projection.extend(stage.columns.iter().enumerate().map(|(j, name)| {
                format!("q{i}.{} AS {}", quote(name), quote(&format!("s{i}c{j}")))
            }));
        }
        let table = format!("__inference_{}", self.serial);
        self.serial += 1;
        let query = format!(
            "SELECT {} FROM {}",
            projection.join(","),
            sources.join(" CROSS JOIN ")
        );
        db.execute_batch(&format!(
            "CREATE TEMP TABLE {} AS {}",
            quote(&table),
            empty(&query)
        ))?;
        let actual = describe(db, &table)?;
        let mut offset = 0;
        let mut cache = cache::shared();
        for pending in self.pending.drain(..) {
            let columns = Arc::new(
                pending
                    .columns
                    .into_iter()
                    .enumerate()
                    .map(|(i, name)| Column {
                        name,
                        declared_type: actual[offset + i].declared_type.clone(),
                    })
                    .collect::<Columns>(),
            );
            offset += columns.len();
            bind(db, &pending.name, declaration(&pending.name, &columns))?;
            cache.insert(pending.key, columns.clone());
            schemas.insert(pending.name, columns);
        }
        assert_eq!(offset, actual.len());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_cte_initialization_cannot_execute_sources_or_authored_functions() -> Result<()> {
        let db = Connection::open_in_memory()?;
        let columns = Arc::new(super::super::register(
            &db,
            "input",
            "CREATE TABLE input(path TEXT,id INTEGER,value REAL)".into(),
        )?);
        db.create_scalar_function(
            "source_text",
            3,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            |_| Err::<String, _>(rusqlite::Error::InvalidQuery),
        )?;
        let mut schemas = Registry::from([("input".into(), columns)]);
        let definitions=(0..8).map(|i| {
            let prior=if i==0 {"input".into()} else {format!("level_{}",i-1)};
            format!("level_{i} AS MATERIALIZED (SELECT a.* FROM {prior} a JOIN {prior} b ON a.id=b.id)")
        }).collect::<Vec<_>>().join(",");
        let query = format!(
            "WITH {definitions} SELECT a.*,source_text('x',0,1) never FROM level_7 a JOIN level_7 b ON a.id=b.id"
        );
        let key = cache::Key::new(&query, &schemas, schemas.keys(), std::iter::empty());
        let mut inference = Inference::default();
        inference.queue(
            &db,
            &mut schemas,
            "output",
            &query,
            vec!["path".into(), "id".into(), "value".into(), "never".into()],
            key,
        )?;
        inference.flush(&db, &mut schemas)?;
        assert_eq!(
            schemas["output"]
                .iter()
                .map(|c| c.declared_type.as_str())
                .collect::<Vec<_>>(),
            vec!["TEXT", "INT", "REAL", ""]
        );
        Ok(())
    }
}
