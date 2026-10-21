//! Immutable (run,file) memberships, shared with selected roots for the duration of a call.
use parking_lot::RwLock;
use rusqlite::{
    Connection, Error, Result,
    types::Value,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor, sqlite3_vtab, sqlite3_vtab_cursor,
    },
};
use std::{
    borrow::Cow,
    ffi::{CStr, c_int},
    marker::PhantomData,
    sync::Arc,
};

#[derive(Default)]
pub(super) struct RunInputs {
    rows: RwLock<Arc<Vec<(i64, i64)>>>,
}
impl RunInputs {
    pub fn register(self: &Arc<Self>, db: &Connection) -> Result<()> {
        const MODULE: Module<'static, Table> = Module::eponymous_only_module();
        db.create_module(c"runs", &MODULE, Some(Arc::clone(self)))
    }
    pub fn install(&self, mut rows: Vec<(i64, i64)>) {
        rows.sort_unstable();
        rows.dedup();
        *self.rows.write() = Arc::new(rows);
    }
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    inputs: Arc<RunInputs>,
}
// SAFETY: SQLite base is first, and cursors own immutable snapshots.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = Arc<RunInputs>;
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Self::Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(run INTEGER NOT NULL,file INTEGER NOT NULL)"),
            Self {
                base: sqlite3_vtab::default(),
                inputs: Arc::clone(
                    aux.ok_or_else(|| Error::ModuleError("runs: no inputs".into()))?,
                ),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        if let Some(i) = info.constraints().enumerate().find_map(|(i, c)| {
            (c.is_usable()
                && c.column() == 0
                && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ)
                .then_some(i)
        }) {
            info.constraint_usage(i).set_argv_index(1);
            // SQLite retains the comparison to preserve its affinity semantics.
            info.set_idx_num(1);
            info.set_estimated_cost(64.);
            info.set_estimated_rows(64);
        } else {
            info.set_estimated_cost(1000000.);
            info.set_estimated_rows(1000000);
        }
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Self::Cursor> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            inputs: Arc::clone(&self.inputs),
            rows: Arc::default(),
            at: 0,
            end: 0,
            marker: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    inputs: Arc<RunInputs>,
    rows: Arc<Vec<(i64, i64)>>,
    at: usize,
    end: usize,
    marker: PhantomData<&'v Table>,
}
// SAFETY: SQLite base is first; every row is read from the cursor's immutable snapshot.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, index: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.rows = Arc::clone(&self.inputs.rows.read());
        self.at = 0;
        self.end = self.rows.len();
        if index == 1
            && let Value::Integer(run) = args.get::<Value>(0)?
        {
            self.at = self.rows.partition_point(|r| r.0 < run);
            self.end = self.rows.partition_point(|r| r.0 <= run);
        }
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.at += 1;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.at >= self.end
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        ctx.set_result(&if col == 0 {
            self.rows[self.at].0
        } else {
            self.rows[self.at].1
        })
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.at as i64)
    }
}
