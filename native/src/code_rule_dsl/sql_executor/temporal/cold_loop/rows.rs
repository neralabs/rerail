//! Indexed access to immutable iteration roots. No SQL or rule evaluation occurs here.
use super::*;
use crate::code_rule_dsl::facts::vtab::{ReadRow, RowCells};

pub trait Roots:
    Send
{
    fn sources_at(
        &mut self,
        _state: i64,
        _slot: usize,
    ) -> rusqlite::Result<Option<(usize, Vec<Arc<dyn Source>>)>> {
        Ok(None)
    }
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
pub struct Rows<A>(
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
struct SourceCells {
    source: Arc<dyn Source>,
    row: usize,
}
impl RowCells for SourceCells {
    fn cell(&self, column: usize) -> SqlValue {
        self.source.cell(self.row, column)
    }
}

fn source_rows(source: Arc<dyn Source>, equal: &[(usize, i64)]) -> Vec<ReadRow> {
    if let Some(rows) = source.direct_all(equal) {
        return rows;
    }
    let selected = source.lookup_all(equal).or_else(|| {
        equal
            .iter()
            .filter(|(c, _)| source.indexed(*c))
            .min_by_key(|(c, _)| source.estimated_matches(*c, false))
            .and_then(|&(c, v)| source.lookup(c, &[v]))
    });
    if let Some(selected) = selected {
        return selected
            .into_iter()
            .filter(|&row| equal.iter().all(|&(c, v)| source.value(row, c) == v))
            .map(|row| ReadRow {
                id: source.row_id(row),
                cells: source
                    .row_cells(row)
                    .map(|(_, cells)| cells)
                    .unwrap_or_else(|| {
                        Arc::new(SourceCells {
                            source: source.clone(),
                            row,
                        })
                    }),
            })
            .collect();
    }
    if let Some(mut stream) = source.row_stream(0..source.len(), false) {
        let mut rows = Vec::new();
        while let Some(row) = stream.next() {
            if equal
                .iter()
                .all(|&(c, v)| row.cells.cell(c) == SqlValue::Integer(v))
            {
                rows.push(row);
            }
        }
        return rows;
    }
    (0..source.len())
        .filter(|&row| equal.iter().all(|&(c, v)| source.value(row, c) == v))
        .map(|row| ReadRow {
            id: source.row_id(row),
            cells: Arc::new(SourceCells {
                source: source.clone(),
                row,
            }),
        })
        .collect()
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
            let slot_index =
                usize::try_from(slot).map_err(|_| sql_error("invalid version slot"))?;
            let sources = self.0.lock().sources_at(state, slot_index)?;
            let constraints = equal
                .iter()
                .filter_map(|&(c, v)| c.checked_sub(2).map(|c| (c, v)))
                .collect::<Vec<_>>();
            let (mut rows, width) = if let Some((width, sources)) = sources {
                let mut rows = Vec::new();
                for source in sources {
                    rows.extend(source_rows(source.clone(), &constraints));
                    if let Some(failure) = source.failure() {
                        return Err(sql_error(failure));
                    }
                }
                (rows, width)
            } else {
                let root = {
                    let mut arena = self.0.lock();
                    arena.root_at(
                        state,
                        usize::try_from(slot).map_err(|_| sql_error("invalid version slot"))?,
                    )?
                };
                let rows = if let Some(rows) = root.direct_all(&constraints) {
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
                (rows, root.arity())
            };
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
