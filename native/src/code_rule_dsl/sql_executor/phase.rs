//! One SQL statement evaluates dependent relational outputs against frozen inputs.
//! Native code only validates and publishes the resulting storage patches.
use super::scratch::Work;
use super::*;

enum Keys {
    Native(versions::Version),
    Query(String),
    Incoming,
    Replace,
}

struct Output {
    table: String,
    source: String,
    old: versions::Version,
    width: usize,
    keys: Keys,
    key_width: usize,
}
pub(super) struct Counts {
    pub keys: usize,
    pub rows: usize,
}
#[derive(Default)]
pub(super) struct Phase {
    pub analyses: Vec<Value>,
    definitions: Vec<String>,
    outputs: Vec<Output>,
}
impl Phase {
    pub fn relation(&mut self, width: usize, query: &str) -> String {
        self.define(width, query, "MATERIALIZED")
    }
    /// Projections, set operations, and pure aggregates over frozen inputs.
    /// Checked rule expressions and native functions stay behind `relation` fences.
    pub fn view(&mut self, width: usize, query: &str) -> String {
        self.define(width, query, "NOT MATERIALIZED")
    }
    fn define(&mut self, width: usize, query: &str, hint: &str) -> String {
        let name = format!("__rr_phase_{}", self.definitions.len());
        let columns = (0..width)
            .map(|i| format!("c{i}"))
            .collect::<Vec<_>>()
            .join(",");
        self.definitions
            .push(format!("{name}({columns}) AS {hint} ({query})"));
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
    pub fn load_empty(&mut self, store: &versions::Store, table: &str, source: &str) -> Result<()> {
        if store.available(table).is_none_or(|v| v.len() != 0) {
            return Err(error("SQL phase load requires an empty native table"));
        }
        self.output(store, table, source, Keys::Incoming)
    }
    pub fn replace_all(
        &mut self,
        store: &versions::Store,
        table: &str,
        source: &str,
    ) -> Result<()> {
        self.output(store, table, source, Keys::Replace)
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
    pub fn execute(self, w: &Work<'_>) -> Result<BTreeMap<String, Counts>> {
        if self.outputs.is_empty() {
            return Ok(BTreeMap::new());
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
        let _context = w.profile_scope(||json!({"stage":"fused relational phase","phaseOutputs":self.outputs.iter().map(|o|&o.table).collect::<Vec<_>>(),"publication":self.outputs.iter().map(|o|json!({"table":o.table,"previous_rows":o.old.len(),"strategy":if o.old.len()==0{"bulk empty root"}else if matches!(o.keys,Keys::Replace){"bulk replacement"}else{"validated persistent patch; dense preparation can bulk rebuild"}})).collect::<Vec<_>>(),"logicalAnalyses":self.analyses}));
        // Delta-first CROSS JOINs and primary-key probes fix the sparse access
        // paths. Reuse preparation across wave scopes while xFilter reads their
        // current immutable roots; rows and expressions are never cached here.
        let mut statement = w.db.prepare_cached(&sql).map_err(error)?;
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
        let counts = self
            .outputs
            .iter()
            .zip(&rows)
            .zip(&key_rows)
            .map(|((o, rows), keys)| {
                let keys = match &o.keys {
                    Keys::Native(k) => k.len(),
                    Keys::Query(_) => keys.len(),
                    Keys::Incoming | Keys::Replace => rows.len(),
                };
                (
                    o.table.clone(),
                    Counts {
                        keys,
                        rows: rows.len(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
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
                    Keys::Incoming => {
                        if o.old.len() != 0 {
                            return Err(error("SQL phase load requires an empty native table"));
                        }
                        o.old.loaded(rows, false, false)?
                    }
                    Keys::Replace => o.old.loaded(rows, false, true)?,
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
        Ok(counts)
    }
}

#[cfg(test)]
mod tests;
