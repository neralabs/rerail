//! Compact integer relations exposed without per-row INSERT statements.
//! A cursor only reads completed facts. It cannot call a matcher or producer.
use rusqlite::{
    Connection, Result, ffi,
    vtab::{Context, Filters, IndexInfo, Module, VTab, VTabConfig, VTabConnection, VTabCursor},
};
use std::{
    borrow::Cow,
    ffi::{CStr, CString, c_int},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub(super) struct Relation {
    columns: &'static [&'static str],
    cells: Vec<i64>,
    pub filters: AtomicU64,
    pub nexts: AtomicU64,
}
impl Relation {
    pub(super) fn new(columns: &'static [&'static str]) -> Self {
        assert!(!columns.is_empty());
        Self {
            columns,
            cells: Vec::new(),
            filters: AtomicU64::new(0),
            nexts: AtomicU64::new(0),
        }
    }
    pub(super) fn push(&mut self, row: &[i64]) {
        assert_eq!(row.len(), self.columns.len());
        self.cells.extend_from_slice(row);
    }
    pub(super) fn len(&self) -> usize {
        self.cells.len() / self.columns.len()
    }
    pub(super) fn bind(self, db: &Connection, name: &str) -> Result<Arc<Self>> {
        let rows = Arc::new(self);
        let module = CString::new(name).unwrap();
        const MODULE: Module<Table> = Module::eponymous_only_module();
        db.create_module(module.as_c_str(), &MODULE, Some(Arc::clone(&rows)))?;
        Ok(rows)
    }
}
#[repr(C)]
struct Table {
    base: ffi::sqlite3_vtab,
    rows: Arc<Relation>,
}
#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    rows: Arc<Relation>,
    at: usize,
}
// SAFETY: SQLite bases are first. Both table and cursors own the immutable data.
unsafe impl<'a> VTab<'a> for Table {
    type Aux = Arc<Relation>;
    type Cursor = Cursor;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Self::Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        let rows = Arc::clone(aux.expect("relation auxiliary data"));
        let fields = rows
            .columns
            .iter()
            .map(|name| format!("\"{name}\" INTEGER NOT NULL"))
            .collect::<Vec<_>>()
            .join(",");
        let schema = CString::new(format!("CREATE TABLE x({fields})")).unwrap();
        Ok((
            Cow::Owned(schema),
            Self {
                base: ffi::sqlite3_vtab::default(),
                rows,
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        info.set_estimated_rows(self.rows.len() as i64);
        info.set_estimated_cost(self.rows.len().max(1) as f64);
        Ok(true)
    }
    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            rows: Arc::clone(&self.rows),
            at: 0,
        })
    }
}
// SAFETY: the cursor owns rows; no borrowed input escapes filter.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, _: &Filters<'_>) -> Result<()> {
        self.rows.filters.fetch_add(1, Ordering::Relaxed);
        self.at = 0;
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.rows.nexts.fetch_add(1, Ordering::Relaxed);
        self.at += 1;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.at >= self.rows.len()
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        ctx.set_result(&self.rows.cells[self.at * self.rows.columns.len() + col as usize])
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.at as i64)
    }
}
