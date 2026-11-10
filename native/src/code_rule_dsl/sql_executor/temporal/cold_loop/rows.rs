//! Indexed access to immutable iteration roots. No SQL or rule evaluation occurs here.
use super::*;
use crate::code_rule_dsl::facts::vtab::{ReadRow, RowCells};

pub(super) struct Rows(pub Arc<parking_lot::Mutex<Arena>>, pub usize);
struct Cells {
    state: i64,
    slot: i64,
    width: usize,
    row: Arc<dyn RowCells>,
}
impl RowCells for Cells {
    fn cell(&self, c: usize) -> SqlValue {
        match c {
            0 => SqlValue::Integer(self.state),
            1 => SqlValue::Integer(self.slot),
            _ if c - 2 < self.width => self.row.cell(c - 2),
            _ => SqlValue::Null,
        }
    }
}
impl Source for Rows {
    fn columns(&self) -> Vec<String> {
        ["rr_state".into(), "rr_slot".into()]
            .into_iter()
            .chain((0..self.1).map(|c| format!("c{c}")))
            .collect()
    }
    fn len(&self) -> usize {
        0
    }
    fn estimated_len(&self) -> usize {
        1000
    }
    fn estimated_matches(&self, _: usize, _: bool) -> usize {
        1000
    }
    fn estimated_matches_all(&self, _: &[usize]) -> usize {
        8
    }
    fn indexed(&self, _: usize) -> bool {
        true
    }
    fn composite(&self) -> bool {
        true
    }
    fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> {
        Some(vec![])
    }
    fn value(&self, _: usize, _: usize) -> i64 {
        unreachable!("version rows require a state and slot")
    }
    fn failure(&self) -> Option<String> {
        self.0.lock().failure.clone()
    }
    fn direct_all(&self, equal: &[(usize, i64)]) -> Option<Vec<ReadRow>> {
        let read = || -> rusqlite::Result<Vec<ReadRow>> {
            let get = |c| {
                equal
                    .iter()
                    .find(|(col, _)| *col == c)
                    .map(|(_, v)| *v)
                    .ok_or_else(|| sql_error("version rows require state and slot constraints"))
            };
            let (state, slot) = (get(0)?, get(1)?);
            let root = {
                let mut arena = self.0.lock();
                arena.root(
                    state,
                    usize::try_from(slot).map_err(|_| sql_error("invalid version slot"))?,
                )?
            };
            let constraints = equal
                .iter()
                .filter_map(|&(c, v)| c.checked_sub(2).map(|c| (c, v)))
                .collect::<Vec<_>>();
            let mut rows = if let Some(rows) = root.direct_all(&constraints) {
                rows
            } else if let Some(indices) = root.lookup_all(&constraints) {
                indices
                    .into_iter()
                    .map(|index| {
                        let (id, cells) = root.row_cells(index).unwrap();
                        ReadRow { id, cells }
                    })
                    .collect()
            } else {
                let mut stream = root.row_stream(0..root.len(), false).unwrap();
                let mut rows = vec![];
                while let Some(row) = stream.next() {
                    if constraints
                        .iter()
                        .all(|&(c, v)| row.cells.cell(c) == SqlValue::Integer(v))
                    {
                        rows.push(row);
                    }
                }
                rows
            };
            for row in &mut rows {
                row.cells = Arc::new(Cells {
                    state,
                    slot,
                    width: root.columns().len(),
                    row: row.cells.clone(),
                });
            }
            Ok(rows)
        };
        match read() {
            Ok(rows) => Some(rows),
            Err(error) => {
                self.0.lock().failure = Some(error.to_string());
                Some(vec![])
            }
        }
    }
}
