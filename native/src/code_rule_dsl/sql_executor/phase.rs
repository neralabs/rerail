//! One SQL statement evaluates dependent relational outputs against frozen inputs.
//! Native code only validates and publishes the resulting storage patches.
use super::scratch::Work;
use super::*;

enum Keys {
    Native(versions::Version),
    Query(String),
}

struct Output {
    table: String,
    source: String,
    old: versions::Version,
    width: usize,
    keys: Keys,
    key_width: usize,
}
#[derive(Default)]
pub(super) struct Phase {
    definitions: Vec<String>,
    outputs: Vec<Output>,
}
impl Phase {
    pub fn relation(&mut self, width: usize, query: &str) -> String {
        let name = format!("__rr_phase_{}", self.definitions.len());
        let columns = (0..width)
            .map(|i| format!("c{i}"))
            .collect::<Vec<_>>()
            .join(",");
        self.definitions
            .push(format!("{name}({columns}) AS MATERIALIZED ({query})"));
        name
    }
    pub fn patch(
        &mut self,
        store: &versions::Store,
        table: &str,
        keys: &str,
        source: &str,
    ) -> Result<()> {
        let keys = store
            .available(keys)
            .ok_or_else(|| error("missing SQL phase keys"))?;
        self.output(store, table, source, Keys::Native(keys))
    }
    pub fn patch_computed(
        &mut self,
        store: &versions::Store,
        table: &str,
        keys: &str,
        source: &str,
    ) -> Result<()> {
        self.output(store, table, source, Keys::Query(keys.into()))
    }
    fn output(
        &mut self,
        store: &versions::Store,
        table: &str,
        source: &str,
        keys: Keys,
    ) -> Result<()> {
        if self.outputs.iter().any(|o| o.table == table) {
            return Err(error("duplicate SQL phase output"));
        }
        let old = store
            .available(table)
            .ok_or_else(|| error("missing SQL phase output"))?;
        self.outputs.push(Output {
            table: table.into(),
            source: source.into(),
            width: old.columns().len(),
            key_width: old.primary_key_len().unwrap(),
            old,
            keys,
        });
        Ok(())
    }
    pub fn execute(self, w: &Work<'_>) -> Result<()> {
        if self.outputs.is_empty() {
            return Ok(());
        }
        let store = w
            .versions
            .ok_or_else(|| error("SQL phase requires native storage"))?;
        let width = self.outputs.iter().map(|o| o.width).max().unwrap();
        let mut arms = vec![];
        for (id, o) in self.outputs.iter().enumerate() {
            let columns = |n| {
                (0..width)
                    .map(|c| {
                        if c < n {
                            format!("c{c}")
                        } else {
                            "NULL".into()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            };
            arms.push(format!(
                "SELECT {},{} FROM {}",
                id * 2,
                columns(o.width),
                o.source
            ));
            if let Keys::Query(keys) = &o.keys {
                arms.push(format!(
                    "SELECT {},{} FROM {keys}",
                    id * 2 + 1,
                    columns(o.key_width)
                ));
            }
        }
        let sql = format!(
            "WITH {} {}",
            self.definitions.join(","),
            arms.join(" UNION ALL ")
        );
        let _context = w.profile_scope(||json!({"stage":"fused relational phase","phaseOutputs":self.outputs.iter().map(|o|&o.table).collect::<Vec<_>>()}));
        let mut statement = w.db.prepare(&sql).map_err(error)?;
        let mut cursor = statement.query([]).map_err(error)?;
        let mut rows = vec![Vec::new(); self.outputs.len()];
        let mut key_rows = vec![Vec::new(); self.outputs.len()];
        while let Some(row) = cursor.next().map_err(error)? {
            let tag = row.get::<_, i64>(0).map_err(error)? as usize;
            let id = tag / 2;
            let output = self
                .outputs
                .get(id)
                .ok_or_else(|| error("invalid SQL phase output tag"))?;
            if tag % 2 == 1 && !matches!(output.keys, Keys::Query(_)) {
                return Err(error("unexpected SQL phase key tag"));
            }
            let size = if tag % 2 == 0 {
                output.width
            } else {
                output.key_width
            };
            let cells = (1..=size)
                .map(|c| row.get(c))
                .collect::<rusqlite::Result<Vec<Option<i64>>>>()
                .map_err(error)?;
            if tag % 2 == 0 {
                rows[id].push(cells);
            } else {
                key_rows[id].push(cells);
            }
        }
        drop(cursor);
        drop(statement);
        let operation = w
            .profiler
            .map(|p| p.operation("SQL phase publication", ("sql-extension", 0), 0, vec![]));
        let _cpu = super::super::logic::profile::span(|| "SQL phase publication".into());
        // Prepare every replacement before changing any root. A failing later output
        // cannot leave an earlier output partially published, even without a savepoint.
        let prepared = self
            .outputs
            .into_iter()
            .zip(rows)
            .zip(key_rows)
            .map(|((o, rows), keys)| {
                let next = match o.keys {
                    Keys::Native(keys) => o.old.patched(&keys, rows, w.preparing)?,
                    Keys::Query(_) => o.old.patched_rows(keys, rows, w.preparing)?,
                };
                Ok((o.table, next))
            })
            .collect::<Result<Vec<_>>>();
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                if let Some(operation) = operation {
                    operation.finish_unobserved();
                }
                return Err(error);
            }
        };
        for (table, version) in prepared {
            store.replace_available(&table, version)?;
        }
        if let Some(operation) = operation {
            operation.finish_unobserved();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
