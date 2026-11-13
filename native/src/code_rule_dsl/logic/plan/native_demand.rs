//! One immutable demand snapshot per completed relation in a run.
use super::{Error, Result};
use crate::code_rule_dsl::{facts::vtab::Source, logic::profile};
use rusqlite::Connection;
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
pub(super) struct Demands {
    rows: HashMap<String, Arc<Vec<i64>>>,
}
impl Demands {
    pub(super) fn forget(&mut self, rel: &str) {
        self.rows.remove(rel);
    }

    pub(super) fn read(
        &mut self,
        db: &Connection,
        rel: &str,
        column: &str,
        source: Option<Arc<dyn Source>>,
    ) -> Result<Arc<Vec<i64>>> {
        if let Some(rows) = self.rows.get(rel) {
            profile::decision("native demand read", || {
                format!("{rel}: cache rows={}", rows.len())
            });
            return Ok(Arc::clone(rows));
        }
        let _read = profile::span(|| format!("native demand snapshot {rel}"));
        let retained = source.is_some();
        let mut rows: Vec<i64> = if let Some(source) = source {
            let rows = (0..source.len()).map(|r| source.value(r, 0)).collect();
            if let Some(error) = source.failure() {
                return Err(Error::Plan(error));
            }
            rows
        } else {
            let mut statement = db.prepare(&format!(
                "SELECT DISTINCT {column} FROM {}",
                super::super::sql::ident(rel)
            ))?;
            statement
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        rows.sort_unstable();
        rows.dedup();
        profile::decision("native demand read", || {
            format!(
                "{rel}: {} rows={}",
                if retained { "retained" } else { "SQL" },
                rows.len()
            )
        });
        let rows = Arc::new(rows);
        self.rows.insert(rel.to_owned(), Arc::clone(&rows));
        Ok(rows)
    }
}

#[cfg(test)]
pub mod tests;
