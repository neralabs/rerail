//! Indexed access to completed output. No producer runs in callbacks.
use super::{Completed, MAX_OUTPUT_BYTES, Output, capacity, source, sql_error};
use crate::code_rule_dsl::residency::admission::relational::{syntax::Source, table};
use rusqlite::{Connection, types::Value, vtab::Context};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Meter {
    work: Arc<AtomicUsize>,
    steps: AtomicUsize,
}
impl Meter {
    fn charge(&self, n: usize) -> rusqlite::Result<()> {
        let before = self.steps.fetch_add(n, Ordering::Relaxed);
        source::charge(&self.work, (before + n).div_ceil(64) - before.div_ceil(64))
            .map_err(|e| rusqlite::Error::UserFunctionError(e.reason.into()))
    }
}
struct Index {
    offsets: Vec<usize>,
    rows: Vec<u32>,
    distinct: usize,
}
impl Index {
    fn build(values: &[u32], size: usize, meter: &Meter) -> rusqlite::Result<Self> {
        let mut offsets = vec![0usize; size];
        for &v in values {
            meter.charge(1)?;
            offsets[v as usize + 1] += 1;
        }
        let distinct = offsets.iter().filter(|&&v| v != 0).count();
        for i in 1..offsets.len() {
            meter.charge(1)?;
            offsets[i] += offsets[i - 1];
        }
        meter.charge(offsets.len())?;
        let mut next = offsets.clone();
        let mut rows = vec![0; values.len()];
        for (row, &v) in values.iter().enumerate() {
            meter.charge(1)?;
            rows[next[v as usize]] = row as u32;
            next[v as usize] += 1;
        }
        Ok(Self {
            offsets,
            rows,
            distinct,
        })
    }
    fn get(&self, value: i64) -> &[u32] {
        let Ok(v) = usize::try_from(value) else {
            return &[];
        };
        if v >= self.offsets.len() - 1 {
            return &[];
        }
        &self.rows[self.offsets[v]..self.offsets[v + 1]]
    }
}
struct CompletedSource {
    output: Output,
    paths: Vec<String>,
    root: Index,
    node: Index,
    meter: Meter,
}
impl Source for CompletedSource {
    fn columns(&self) -> Vec<String> {
        vec!["__path".into(), "root".into(), "node".into()]
    }
    fn column_types(&self) -> Vec<String> {
        vec!["TEXT".into(), String::new(), String::new()]
    }
    fn len(&self) -> usize {
        self.output.roots.len()
    }
    fn indexed(&self, c: usize) -> bool {
        (1..=2).contains(&c)
    }
    fn estimated_matches(&self, c: usize, batch: bool) -> usize {
        let distinct = if c == 1 {
            self.root.distinct
        } else {
            self.node.distinct
        };
        if batch {
            self.len().min(1000)
        } else {
            self.len().div_ceil(distinct.max(1)).max(1)
        }
    }
    fn lookup(&self, c: usize, values: &[Value]) -> rusqlite::Result<Option<Vec<usize>>> {
        let index = if c == 1 { &self.root } else { &self.node };
        let mut result = Vec::new();
        for v in values {
            self.meter.charge(1)?;
            match v {
                Value::Integer(v) => {
                    let rows = index.get(*v);
                    self.meter.charge(rows.len())?;
                    result.extend(rows.iter().map(|&v| v as usize));
                }
                Value::Null => {}
                // SQLite retains all affinity, real-number, and collation decisions.
                _ => return Ok(None),
            }
        }
        result.sort_unstable();
        result.dedup();
        Ok(Some(result))
    }
    fn column(&self, row: usize, c: usize, ctx: &mut Context) -> rusqlite::Result<()> {
        match c {
            0 => ctx.set_result(&self.paths[self.output.files[row] as usize]),
            1 => ctx.set_result(&i64::from(self.output.roots[row])),
            _ => ctx.set_result(&i64::from(self.output.nodes[row])),
        }
    }
    fn charge(&self, n: usize) -> rusqlite::Result<()> {
        self.meter.charge(n)
    }
}
pub(in crate::code_rule_dsl) fn publish(
    completed: Completed,
    db: &Connection,
    name: &str,
    work: Arc<AtomicUsize>,
) -> super::Result<()> {
    let Completed { output, paths } = completed;
    let size = output
        .roots
        .iter()
        .chain(&output.nodes)
        .max()
        .copied()
        .unwrap_or(0) as usize
        + 2;
    // Include retained numeric output and both indexes, with a scratch-offset allowance.
    capacity(
        output
            .roots
            .len()
            .checked_mul(20)
            .and_then(|n| size.checked_mul(32).and_then(|s| n.checked_add(s))),
        MAX_OUTPUT_BYTES,
    )?;
    let meter = Meter {
        work,
        steps: AtomicUsize::new(0),
    };
    let root = Index::build(&output.roots, size, &meter).map_err(sql_error)?;
    let node = Index::build(&output.nodes, size, &meter).map_err(sql_error)?;
    table::register(
        db,
        name,
        Arc::new(CompletedSource {
            output,
            paths,
            root,
            node,
            meter,
        }),
    )
    .map_err(sql_error)?;
    Ok(())
}
