//! Bounded fact production only. SQL retains all matching and answer semantics.
use super::Meter;
use crate::reader::{Column, Shared};
use rusqlite::{
    Connection, Result, ffi,
    vtab::{Context, Filters, IndexInfo, Module, VTab, VTabConfig, VTabConnection, VTabCursor},
};
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    ffi::{CStr, c_int},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) const WAVE_ROWS: usize = 65_536;
#[derive(Clone, Copy)]
pub(super) struct Task {
    pub fid: i64,
    pub lo: usize,
    pub hi: usize,
}
struct Batch {
    reader: Shared,
    tasks: Vec<Task>,
    rows: usize,
    filters: AtomicUsize,
    waves: AtomicUsize,
    decoded: AtomicUsize,
}
#[repr(C)]
struct Table {
    base: ffi::sqlite3_vtab,
    batch: Arc<Batch>,
}
#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    batch: Arc<Batch>,
    task: usize,
    next_pre: usize,
    columns: [Vec<i64>; 7],
    row: usize,
    ordinal: i64,
}
fn invalid(message: &str) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.into())
}
pub(super) fn validate(tasks: &[Task], bounds: &[(i64, usize)]) -> Result<()> {
    for task in tasks {
        let max = bounds
            .iter()
            .find(|(fid, _)| *fid == task.fid)
            .map(|(_, n)| *n)
            .ok_or_else(|| invalid("missing batch file"))?;
        if task.lo > task.hi || task.hi > max {
            return Err(invalid("invalid batch range"));
        }
    }
    Ok(())
}
// SAFETY: SQLite base is first. Arc ownership retains descriptors and reader for each cursor.
unsafe impl<'a> VTab<'a> for Table {
    type Aux = Arc<Batch>;
    type Cursor = Cursor;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<Batch>>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        Ok((Cow::Borrowed(c"CREATE TABLE x(fid INTEGER,pre INTEGER,parent INTEGER,end INTEGER,kind INTEGER,field INTEGER,flags INTEGER)"),Self {base:ffi::sqlite3_vtab::default(),batch:aux.expect("batch supplied").clone()}))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        info.set_estimated_rows(self.batch.rows as i64);
        info.set_estimated_cost(self.batch.rows as f64);
        Ok(true)
    }
    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            batch: self.batch.clone(),
            task: 0,
            next_pre: 0,
            columns: std::array::from_fn(|_| Vec::new()),
            row: 0,
            ordinal: 0,
        })
    }
}
impl Cursor {
    fn fill(&mut self) -> Result<()> {
        for c in &mut self.columns {
            c.clear();
        }
        self.row = 0;
        let mut reader = self.batch.reader.lock();
        while self.columns[0].len() < WAVE_ROWS && self.task < self.batch.tasks.len() {
            let task = self.batch.tasks[self.task];
            if self.next_pre >= task.hi {
                self.task += 1;
                self.next_pre = self.batch.tasks.get(self.task).map_or(0, |t| t.lo);
                continue;
            }
            let pre = self.next_pre;
            let rec = reader.rec(task.fid, pre)?;
            let flags = reader.flags(task.fid, pre)?;
            let parent =
                pre as i64 - i64::from(reader.value(task.fid, Column::Parent, pre, rec[1])?);
            let end = pre as i64 + i64::from(reader.value(task.fid, Column::Size, pre, rec[2])?);
            let kind = if flags & 8 != 0 { 65535 } else { rec[0] & 1023 };
            let values = [
                task.fid,
                pre as i64,
                parent,
                end,
                i64::from(kind),
                i64::from(rec[0] >> 10),
                i64::from(flags),
            ];
            for (c, v) in self.columns.iter_mut().zip(values) {
                c.push(v);
            }
            self.next_pre += 1;
        }
        if !self.columns[0].is_empty() {
            self.batch.waves.fetch_add(1, Ordering::Relaxed);
            self.batch
                .decoded
                .fetch_add(self.columns[0].len(), Ordering::Relaxed);
        }
        Ok(())
    }
}
// SAFETY: cursor coordinates and completed columns belong to this cursor. No xColumn decoding occurs.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, _: &Filters<'_>) -> Result<()> {
        self.batch.filters.fetch_add(1, Ordering::Relaxed);
        self.task = 0;
        self.next_pre = self.batch.tasks.first().map_or(0, |t| t.lo);
        self.ordinal = 0;
        self.fill()
    }
    fn next(&mut self) -> Result<()> {
        self.row += 1;
        self.ordinal += 1;
        if self.row >= self.columns[0].len() {
            self.fill()?;
        }
        Ok(())
    }
    fn eof(&self) -> bool {
        self.columns[0].is_empty()
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        ctx.set_result(&self.columns[col as usize][self.row])
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.ordinal)
    }
}
fn install(
    db: &Connection,
    reader: Shared,
    tasks: Vec<Task>,
    bounds: &[(i64, usize)],
) -> Result<Arc<Batch>> {
    validate(&tasks, bounds)?;
    let rows = tasks
        .iter()
        .try_fold(0usize, |sum, t| sum.checked_add(t.hi - t.lo))
        .ok_or_else(|| invalid("batch size overflow"))?;
    let batch = Arc::new(Batch {
        reader,
        tasks,
        rows,
        filters: AtomicUsize::new(0),
        waves: AtomicUsize::new(0),
        decoded: AtomicUsize::new(0),
    });
    const MODULE: Module<Table> = Module::eponymous_only_module();
    db.create_module(c"batch_nodes", &MODULE, Some(batch.clone()))?;
    Ok(batch)
}

pub(super) fn run(
    store: &crate::store::Store,
    files: &[i64],
    variant: &str,
    m: &mut Meter,
) -> Result<(Vec<Vec<i64>>, Value)> {
    let ids = files
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    // One query obtains every descriptor. No per-file or per-node SQL preparation.
    let descriptors = m.statement(
        &store.db,
        &format!("SELECT fid,nodes FROM file WHERE fid IN ({ids}) ORDER BY fid"),
    )?;
    if descriptors.len() != files.len() {
        return Err(invalid("missing or repeated batch file"));
    }
    let bounds: Vec<_> = descriptors.iter().map(|r| (r[0], r[1] as usize)).collect();
    let tasks = bounds
        .iter()
        .map(|(fid, hi)| Task {
            fid: *fid,
            lo: 0,
            hi: *hi,
        })
        .collect();
    let batch = install(&store.db, store.readers.0.clone(), tasks, &bounds)?;
    let sql = match variant {
        "scalar" => format!(
            "SELECT f.fid,g.value,nera_parent(f.fid,g.value),nera_end(f.fid,g.value),nera_kind(f.fid,g.value),nera_field(f.fid,g.value),nera_flags(f.fid,g.value) FROM file f CROSS JOIN generate_series(0,f.nodes-1) g WHERE f.fid IN ({ids})"
        ),
        "stream" => format!(
            "SELECT f.fid,n.pre,n.parent,n.end,n.kind,n.field,n.flags FROM file f CROSS JOIN nera_nodes(f.fid,0,f.nodes) n WHERE f.fid IN ({ids})"
        ),
        "columns" => "SELECT fid,pre,parent,end,kind,field,flags FROM batch_nodes".into(),
        _ => return Err(invalid("unknown batch variant")),
    };
    let rows = m.statement(&store.db, &sql)?;
    Ok((
        rows,
        json!({"filters":batch.filters.load(Ordering::Relaxed),"waves":batch.waves.load(Ordering::Relaxed),"decodedRows":batch.decoded.load(Ordering::Relaxed),"tasks":bounds.len(),"scalarCallsExpected":if variant=="scalar" {batch.rows*5}else{0},"maxColumnPayloadBytes":WAVE_ROWS.min(batch.rows)*7*8}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn column_payload_is_bounded_across_many_descriptors_and_empty_tasks() {
        let (_dir, store) = crate::composition::tests::fixture();
        let mut tasks = vec![Task {
            fid: 1,
            lo: 12,
            hi: 12,
        }];
        tasks.extend(std::iter::repeat_n(
            Task {
                fid: 1,
                lo: 0,
                hi: 12,
            },
            6000,
        ));
        let batch = install(&store.db, store.readers.0.clone(), tasks, &[(1, 12)]).unwrap();
        let rows = Meter::default()
            .statement(&store.db, "SELECT count(*),sum(pre) FROM batch_nodes")
            .unwrap();
        assert_eq!(rows, vec![vec![72000, 396000]]);
        assert_eq!(batch.filters.load(Ordering::Relaxed), 1);
        assert_eq!(batch.waves.load(Ordering::Relaxed), 2);
        assert_eq!(batch.decoded.load(Ordering::Relaxed), 72000);
        let weak = Arc::downgrade(&batch);
        drop(batch);
        assert!(weak.upgrade().is_some()); // The SQLite module retains the immutable batch.
        drop(store);
        assert!(weak.upgrade().is_none());
    }
}
