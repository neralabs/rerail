//! Indexed access to immutable iteration roots. No SQL or rule evaluation occurs here.
use super::*;
use crate::code_rule_dsl::facts::vtab::{ReadRow, RowCells};

pub(in crate::code_rule_dsl::sql_executor::temporal) trait Roots:
    Send
{
    fn root_at(&mut self, state: i64, slot: usize) -> rusqlite::Result<versions::Version>;
    fn failure(&mut self) -> &mut Option<String>;
}
impl Roots for Arena {
    fn root_at(&mut self, state: i64, slot: usize) -> rusqlite::Result<versions::Version> {
        self.root(state, slot)
    }
    fn failure(&mut self) -> &mut Option<String> {
        &mut self.failure
    }
}
pub(in crate::code_rule_dsl::sql_executor::temporal) struct Rows<A>(
    pub Arc<parking_lot::Mutex<A>>,
    pub usize,
    pub Option<usize>,
);
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
impl<A: Roots + 'static> Source for Rows<A> {
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
        self.2.unwrap_or(1000)
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        match (self.2, column >= 2, batch) {
            // The slot and state must be known for every lookup. Leave IN lists
            // to SQLite instead of choosing a standalone visible-column probe.
            (Some(scan), _, true) => scan + 1,
            (Some(_), true, false) => 2,
            _ => self.estimated_len(),
        }
    }
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        match self.2 {
            None => 8,
            Some(scan) if columns.iter().all(|&c| c < 2) => scan,
            Some(_) => 2,
        }
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
        self.0.lock().failure().clone()
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
                arena.root_at(
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
            let width = root.arity();
            for row in &mut rows {
                row.cells = Arc::new(Cells {
                    state,
                    slot,
                    width,
                    row: row.cells.clone(),
                });
            }
            Ok(rows)
        };
        match read() {
            Ok(rows) => Some(rows),
            Err(error) => {
                *self.0.lock().failure() = Some(error.to_string());
                Some(vec![])
            }
        }
    }
}
