//! Build immutable roots from SQL result rows. This layer has no connection or rule evaluator.
//! A complete set of validated roots is returned before its caller publishes anything.
use super::super::{error, versions};
use super::{Counts, Keys, Materialized, Output};
use crate::code_rule_dsl::facts::vtab::Source;
use napi::Result;
use std::collections::BTreeMap;

pub(super) type Rows = Vec<Vec<Option<i64>>>;

/// A reusable storage recipe keeps schemas and source identities, without retaining row roots.
/// Its caller supplies the actual versions for each SQL loop iteration.
#[derive(Clone)]
pub(super) struct Template {
    outputs: Vec<Output>,
}
impl Template {
    pub fn capture(outputs: Vec<Output>) -> (Self, Vec<versions::Version>) {
        let mut snapshots = Vec::new();
        let outputs = outputs
            .into_iter()
            .map(|mut output| {
                let empty = output.old.empty_like();
                snapshots.push(std::mem::replace(&mut output.old, empty));
                if let Keys::Native { version, .. } = &mut output.keys {
                    let empty = version.empty_like();
                    snapshots.push(std::mem::replace(version, empty));
                }
                output
            })
            .collect();
        (Self { outputs }, snapshots)
    }
    /// Uses stay ordered and distinct even when two snapshots have the same table name.
    pub fn inputs(&self) -> Vec<&str> {
        self.outputs
            .iter()
            .flat_map(|output| {
                let keys = if let Keys::Native { source, .. } = &output.keys {
                    Some(source.as_str())
                } else {
                    None
                };
                std::iter::once(output.table.as_str()).chain(keys)
            })
            .collect()
    }
    pub fn names(&self) -> Vec<&str> {
        self.outputs.iter().map(|o| o.table.as_str()).collect()
    }
    pub fn explain(&self, snapshots: &[versions::Version]) -> serde_json::Value {
        let mut at = 0;
        serde_json::json!(self.outputs.iter().map(|output| {
            let old=&snapshots[at];
            at+=1+usize::from(matches!(output.keys,Keys::Native {..}));
            serde_json::json!({"table":output.table,"previous_rows":old.len(),"strategy":if old.len()==0 {
                "bulk empty root"
            } else if matches!(output.keys,Keys::Replace) {"bulk replacement"}
            else {"validated patch; affected-key density selects bulk rebuilding"}})
        }).collect::<Vec<_>>())
    }
    pub fn build(
        &self,
        snapshots: &[versions::Version],
        rows: Vec<Rows>,
        keys: Vec<Rows>,
    ) -> Result<Materialized> {
        let expected = self
            .outputs
            .iter()
            .map(|o| 1 + usize::from(matches!(o.keys, Keys::Native { .. })))
            .sum::<usize>();
        if snapshots.len() != expected {
            return Err(error("SQL publication snapshots do not match their uses"));
        }
        let mut snapshots = snapshots.iter();
        let outputs = self
            .outputs
            .iter()
            .map(|output| {
                let mut bound = output.clone();
                bound.old = snapshots.next().unwrap().clone();
                if !bound.old.same_schema(&output.old) {
                    return Err(error("SQL publication target schema changed"));
                }
                if let Keys::Native { version, .. } = &mut bound.keys {
                    let current = snapshots.next().unwrap();
                    if !version.same_schema(current) {
                        return Err(error("SQL publication key schema changed"));
                    }
                    *version = current.clone();
                }
                Ok(bound)
            })
            .collect::<Result<Vec<_>>>()?;
        build(outputs, rows, keys)
    }
}

pub(super) fn build(
    outputs: Vec<Output>,
    rows: Vec<Rows>,
    key_rows: Vec<Rows>,
) -> Result<Materialized> {
    if rows.len() != outputs.len() || key_rows.len() != outputs.len() {
        return Err(error(
            "SQL publication buffers do not match their destinations",
        ));
    }
    let mut counts = outputs
        .iter()
        .zip(&rows)
        .zip(&key_rows)
        .map(|((o, rows), keys)| {
            let keys = match &o.keys {
                Keys::Native { version, .. } => version.len(),
                Keys::Query(_) => keys.len(),
                Keys::Incoming | Keys::Replace | Keys::Insert(_) => rows.len(),
            };
            (
                o.table.clone(),
                Counts {
                    keys,
                    rows: rows.len(),
                    strategy: if o.old.len() == 0 {
                        crate::code_rule_dsl::logic::analysis::storage::Publication::BulkEmpty
                    } else if matches!(o.keys, Keys::Replace) {
                        crate::code_rule_dsl::logic::analysis::storage::Publication::BulkReplacement
                    } else {
                        crate::code_rule_dsl::logic::analysis::storage::Publication::SparsePatch
                    },
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    // Prepare every replacement before changing any root. A failing later output
    // cannot leave an earlier output partially published, even without a savepoint.
    let mut reusable = BTreeMap::<(usize, usize), Vec<versions::Version>>::new();
    let mut shared_roots = 0;
    let mut retained_roots = 0;
    let prepared = outputs
        .into_iter()
        .zip(rows)
        .zip(key_rows)
        .map(|((o, rows), keys)| {
            let candidates = if o.old.len() == 0 && rows.len() >= 512 {
                reusable
                    .get(&(o.width, rows.len()))
                    .map_or(&[][..], Vec::as_slice)
            } else {
                &[][..]
            };
            let reused = if matches!(o.keys, Keys::Incoming | Keys::Replace)
                && o.old.len() == 0
                && rows.len() >= 512
            {
                candidates
                    .iter()
                    .find_map(|v| o.old.reuse_empty_rows(&rows, v))
            } else {
                None
            };
            let next = if let Some(next) = reused {
                shared_roots += 1;
                next
            } else {
                match o.keys {
                    Keys::Native { version: keys, .. } => {
                        if keys.columns().len() != o.key_width {
                            return Err(error("SQL phase patch requires the complete primary key"));
                        }
                        let (next, shared, strategy) =
                            o.old
                                .patched_rows_reusing(keys.rows(), rows, true, candidates)?;
                        counts.get_mut(&o.table).unwrap().strategy = strategy;
                        shared_roots += usize::from(shared);
                        next
                    }
                    Keys::Query(_) => {
                        let (next, shared, strategy) =
                            o.old.patched_rows_reusing(keys, rows, true, candidates)?;
                        counts.get_mut(&o.table).unwrap().strategy = strategy;
                        shared_roots += usize::from(shared);
                        next
                    }
                    Keys::Incoming => {
                        if o.old.len() != 0 {
                            return Err(error("SQL phase load requires an empty native table"));
                        }
                        o.old.loaded(rows, false, false)?
                    }
                    Keys::Replace => o.old.loaded(rows, false, true)?,
                    Keys::Insert(ignore) => {
                        let next = o.old.inserted_rows(rows, ignore)?;
                        let inserted = next.len() - o.old.len();
                        let count = counts.get_mut(&o.table).unwrap();
                        count.rows = inserted;
                        count.keys = inserted;
                        count.strategy =
                            crate::code_rule_dsl::logic::analysis::storage::publication(
                                o.old.len(),
                                inserted,
                            );
                        next
                    }
                }
            };
            if o.old.len() > 0 && o.old.shares_rows(&next) {
                retained_roots += 1;
            }
            if o.old.len() == 0 && next.len() >= 512 {
                reusable
                    .entry((o.width, next.len()))
                    .or_default()
                    .push(next.clone());
            }
            Ok((o.table, next))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Materialized {
        counts,
        roots: prepared,
        shared_roots,
        retained_roots,
    })
}
