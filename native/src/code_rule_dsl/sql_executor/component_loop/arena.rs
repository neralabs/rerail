//! Immutable version references shared by the host loop and recursive SQL comparison.
//! Functions collect SQL result rows and build storage. They never execute rule SQL.
use super::*;
use crate::code_rule_dsl::facts::vtab::{ReadRow, RowCells};
use rusqlite::functions::{Aggregate, Context};

#[derive(Clone)]
pub struct State {
    pub full: Vec<versions::Version>,
    pub delta: Vec<versions::Version>,
}
pub struct Arena {
    pub states: Vec<State>,
    pub widths: Vec<usize>,
    pub storage_cpu_ns: Option<u128>,
    pub produced: Vec<Vec<usize>>,
    pub failure: Option<String>,
}
impl Arena {
    pub fn new(widths: Vec<usize>) -> Self {
        let full = widths
            .iter()
            .map(|&width| {
                versions::Version::empty(Arc::new(versions::Schema::integers(width, width)))
            })
            .collect::<Vec<_>>();
        Self {
            states: vec![State {
                delta: full.clone(),
                full,
            }],
            widths,
            storage_cpu_ns: Some(0),
            produced: vec![],
            failure: None,
        }
    }
    pub fn state(&self, handle: i64) -> rusqlite::Result<&State> {
        usize::try_from(handle)
            .ok()
            .and_then(|id| self.states.get(id))
            .ok_or_else(|| {
                rusqlite::Error::UserFunctionError("invalid component version reference".into())
            })
    }
    pub fn active(&self, handle: i64) -> rusqlite::Result<bool> {
        Ok(self.state(handle)?.delta.iter().any(|root| root.len() > 0))
    }
}

pub struct Rows {
    pub arena: Arc<parking_lot::Mutex<Arena>>,
    pub width: usize,
}
struct Cells {
    params: [i64; 3],
    width: usize,
    row: Arc<dyn RowCells>,
}
impl RowCells for Cells {
    fn cell(&self, column: usize) -> SqlValue {
        if column < 3 {
            SqlValue::Integer(self.params[column])
        } else if column - 3 < self.width {
            self.row.cell(column - 3)
        } else {
            SqlValue::Null
        }
    }
}
impl Source for Rows {
    fn columns(&self) -> Vec<String> {
        ["rr_state", "rr_relation", "rr_frontier"]
            .map(str::to_owned)
            .into_iter()
            .chain((0..self.width).map(|c| format!("c{c}")))
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
    fn composite(&self) -> bool {
        true
    }
    fn indexed(&self, _: usize) -> bool {
        true
    }
    fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> {
        Some(vec![])
    }
    fn value(&self, _: usize, _: usize) -> i64 {
        unreachable!("component rows require a version reference")
    }
    fn failure(&self) -> Option<String> {
        self.arena.lock().failure.clone()
    }
    fn direct_all(&self, equal: &[(usize, i64)]) -> Option<Vec<ReadRow>> {
        let run = || -> rusqlite::Result<Vec<ReadRow>> {
            let params = (0..3)
                .map(|c| {
                    equal
                        .iter()
                        .find(|(column, _)| *column == c)
                        .map(|(_, value)| *value)
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    rusqlite::Error::UserFunctionError(
                        "component rows require state, relation, and frontier constraints".into(),
                    )
                })?;
            let root = {
                let arena = self.arena.lock();
                let state = arena.state(params[0])?;
                let relation = usize::try_from(params[1]).map_err(|_| {
                    rusqlite::Error::UserFunctionError("invalid component relation".into())
                })?;
                let roots = if params[2] == 0 {
                    &state.full
                } else {
                    &state.delta
                };
                roots.get(relation).cloned().ok_or_else(|| {
                    rusqlite::Error::UserFunctionError("invalid component relation".into())
                })?
            };
            let constraints = equal
                .iter()
                .filter_map(|&(c, v)| (c >= 3).then_some((c.saturating_sub(3), v)))
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
                    params: [params[0], params[1], params[2]],
                    width: root.columns().len(),
                    row: row.cells.clone(),
                });
            }
            Ok(rows)
        };
        match run() {
            Ok(rows) => Some(rows),
            Err(err) => {
                self.arena.lock().failure = Some(err.to_string());
                Some(vec![])
            }
        }
    }
}

pub struct Collect(pub Arc<parking_lot::Mutex<Arena>>);
pub struct Batch {
    parent: Option<i64>,
    rows: Vec<Vec<Vec<Option<i64>>>>,
}
impl Aggregate<Batch, i64> for Collect {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Batch> {
        Ok(Batch {
            parent: None,
            rows: vec![vec![]; self.0.lock().widths.len()],
        })
    }
    fn step(&self, context: &mut Context<'_>, batch: &mut Batch) -> rusqlite::Result<()> {
        let parent = context.get::<i64>(0)?;
        if batch.parent.is_some_and(|old| old != parent) {
            return Err(rusqlite::Error::UserFunctionError(
                "mixed component parent references".into(),
            ));
        }
        batch.parent = Some(parent);
        let relation = context.get::<i64>(1)?;
        if relation < 0 {
            return Ok(());
        }
        let relation = relation as usize;
        let width = *self.0.lock().widths.get(relation).ok_or_else(|| {
            rusqlite::Error::UserFunctionError("invalid component relation".into())
        })?;
        batch.rows[relation].push(
            (0..width)
                .map(|c| context.get(c + 2))
                .collect::<rusqlite::Result<_>>()?,
        );
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, batch: Option<Batch>) -> rusqlite::Result<i64> {
        let batch = batch.ok_or_else(|| {
            rusqlite::Error::UserFunctionError("missing component parent sentinel".into())
        })?;
        let cpu = super::cpu();
        let _span =
            crate::code_rule_dsl::logic::profile::span(|| "component version storage".into());
        let mut arena = self.0.lock();
        let parent = arena.state(batch.parent.unwrap())?.clone();
        let mut next = State {
            full: vec![],
            delta: vec![],
        };
        for (root, rows) in parent.full.iter().zip(batch.rows) {
            let delta = root
                .empty_like()
                .loaded(rows.clone(), false, false)
                .map_err(|e| rusqlite::Error::UserFunctionError(e.to_string().into()))?;
            let full = if delta.len() == 0 {
                root.clone()
            } else {
                root.inserted_rows(rows, false)
                    .map_err(|e| rusqlite::Error::UserFunctionError(e.to_string().into()))?
            };
            next.delta.push(delta);
            next.full.push(full);
        }
        let handle = arena.states.len() as i64;
        arena
            .produced
            .push(next.delta.iter().map(Source::len).collect());
        arena.states.push(next);
        arena.storage_cpu_ns = arena
            .storage_cpu_ns
            .zip(cpu.zip(super::cpu()))
            .map(|(total, (start, end))| total + end.saturating_sub(start));
        Ok(handle)
    }
}
