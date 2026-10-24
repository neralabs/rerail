//! Numeric SQL continuation records. The caller supplies every state transition.
use rusqlite::{
    Connection, Error, Result,
    functions::FunctionFlags,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor, sqlite3_vtab, sqlite3_vtab_cursor,
    },
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    ffi::{CStr, CString, c_int},
    marker::PhantomData,
    sync::{Arc, Mutex},
};
const CELLS: usize = 12;
const OUTPUTS: usize = 2 + CELLS;
const MODE: usize = OUTPUTS;
const HANDLE: usize = MODE + 1;
const PARENT: usize = MODE + 2;
const VALUES: usize = MODE + 3;
const WIDTH: usize = VALUES + CELLS;
#[derive(Clone)]
struct Row {
    parent: Option<i64>,
    cells: [Option<i64>; CELLS],
    children: usize,
}
#[derive(Default)]
pub(super) struct Store {
    next: i64,
    rows: BTreeMap<i64, Row>,
    peak: usize,
}
impl Store {
    pub(super) fn reset(&mut self) {
        self.rows.clear();
        self.peak = 0;
    }
}
fn invalid(s: &str) -> Error {
    Error::ModuleError(format!("continuation: {s}"))
}
pub(super) fn register(db: &Connection) -> Result<Arc<Mutex<Store>>> {
    let store = Arc::new(Mutex::new(Store::default()));
    let rows = store.clone();
    let peak = store.clone();
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DIRECTONLY;
    db.create_scalar_function("relation_continuation_rows", 0, flags, move |_| {
        Ok(rows.lock().unwrap().rows.len() as i64)
    })?;
    db.create_scalar_function("relation_continuation_peak", 0, flags, move |_| {
        Ok(peak.lock().unwrap().peak as i64)
    })?;
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(c"relation_continuation", &MODULE, Some(store.clone()))?;
    Ok(store)
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    store: Arc<Mutex<Store>>,
}
// SAFETY: SQLite's base is first; every cursor retains its numeric store.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = Arc<Mutex<Store>>;
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        store: Option<&Self::Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let mut cols = vec!["handle INTEGER".into(), "parent INTEGER".into()];
        cols.extend((0..CELLS).map(|i| format!("c{i} INTEGER")));
        cols.extend(["mode", "h", "parent_input"].map(|s| format!("{s} INTEGER HIDDEN")));
        cols.extend((0..CELLS).map(|i| format!("v{i} INTEGER HIDDEN")));
        Ok((
            Cow::Owned(CString::new(format!("CREATE TABLE x({})", cols.join(","))).unwrap()),
            Self {
                base: sqlite3_vtab::default(),
                store: store.unwrap().clone(),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut inputs = BTreeMap::new();
        for (index, c) in info.constraints().enumerate() {
            if c.is_usable()
                && (MODE as i32..WIDTH as i32).contains(&c.column())
                && matches!(
                    c.operator(),
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_IS
                        | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_ISNULL
                )
            {
                inputs.entry(c.column() as usize).or_insert((
                    index,
                    c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_ISNULL,
                ));
            }
        }
        if !inputs.contains_key(&MODE) {
            return Ok(false);
        }
        let mut arg = 0;
        for &(index, null) in inputs.values() {
            let mut u = info.constraint_usage(index);
            if !null {
                arg += 1;
                u.set_argv_index(arg);
            }
            u.set_omit(true);
        }
        info.set_idx_str(
            &inputs
                .iter()
                .map(|(c, (_, n))| format!("{}{c}", if *n { "n" } else { "" }))
                .collect::<Vec<_>>()
                .join(","),
        );
        info.set_estimated_cost(1.);
        info.set_estimated_rows(1);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Self::Cursor> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            store: self.store.clone(),
            handle: 0,
            row: None,
            marker: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    store: Arc<Mutex<Store>>,
    handle: i64,
    row: Option<Row>,
    marker: PhantomData<&'v Table>,
}
// SAFETY: SQLite's base is first; projected cells belong to this cursor.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, _: c_int, plan: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.row = None;
        let mut values = BTreeMap::new();
        let mut arg = 0;
        for c in plan.unwrap().split(',') {
            if c.starts_with('n') {
                continue;
            }
            let c = c.parse::<usize>().unwrap();
            let v = args.get::<Option<i64>>(arg)?;
            arg += 1;
            values.insert(c, v);
        }
        let get = |c| values.get(&c).copied().flatten();
        let Some(mode) = get(MODE) else {
            return Ok(());
        };
        let mut store = self.store.lock().unwrap();
        match mode {
            1 => {
                let parent = get(PARENT);
                if let Some(parent) = parent {
                    let p = store
                        .rows
                        .get_mut(&parent)
                        .ok_or_else(|| invalid("missing parent"))?;
                    p.children += 1;
                }
                store.next = store
                    .next
                    .checked_add(1)
                    .ok_or_else(|| invalid("handle overflow"))?;
                self.handle = store.next;
                let row = Row {
                    parent,
                    cells: std::array::from_fn(|i| get(VALUES + i)),
                    children: 0,
                };
                store.rows.insert(self.handle, row.clone());
                store.peak = store.peak.max(store.rows.len());
                self.row = Some(row);
            }
            0 | 2 => {
                let h = get(HANDLE).ok_or_else(|| invalid("missing handle"))?;
                self.handle = h;
                if mode == 0 {
                    self.row = store.rows.get(&h).cloned();
                } else if let Some(row) = store.rows.get(&h) {
                    if row.children != 0 {
                        return Err(invalid("pop has active children"));
                    }
                    let row = store.rows.remove(&h).unwrap();
                    if let Some(parent) = row.parent {
                        store
                            .rows
                            .get_mut(&parent)
                            .ok_or_else(|| invalid("missing parent"))?
                            .children -= 1;
                    }
                    self.row = Some(row);
                }
            }
            _ => return Err(invalid("invalid mode")),
        }
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.row = None;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.row.is_none()
    }
    fn column(&self, ctx: &mut Context, c: c_int) -> Result<()> {
        let row = self.row.as_ref().ok_or_else(|| invalid("empty row"))?;
        match c {
            0 => ctx.set_result(&self.handle),
            1 => ctx.set_result(&row.parent),
            2..=13 => ctx.set_result(&row.cells[c as usize - 2]),
            _ => Err(invalid("hidden output")),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.handle)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_chains_pop_exact_records_and_release_storage() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let push = |parent: Option<i64>, value: i64| {
            db.query_row::<i64,_,_>("SELECT handle FROM relation_continuation WHERE mode IS 1 AND parent_input IS ?1 AND v0 IS ?2",rusqlite::params![parent,value],|r|r.get(0)).unwrap()
        };
        let a = push(None, 7);
        let b = push(Some(a), 9);
        let c = push(None, 11);
        assert!(
            db.query_row::<i64, _, _>(
                "SELECT c0 FROM relation_continuation WHERE mode IS 2 AND h IS ?1",
                [a],
                |r| r.get(0)
            )
            .is_err()
        );
        for (h, value) in [(b, 9), (a, 7), (c, 11)] {
            assert_eq!(
                db.query_row::<i64, _, _>(
                    "SELECT c0 FROM relation_continuation WHERE mode IS 2 AND h IS ?1",
                    [h],
                    |r| r.get(0)
                )
                .unwrap(),
                value
            );
        }
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT relation_continuation_rows()", [], |r| r.get(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT relation_continuation_peak()", [], |r| r.get(0))
                .unwrap(),
            3
        );
    }
    #[test]
    fn null_mode_and_nonnumeric_cells_cannot_mutate_storage() {
        let db = Connection::open_in_memory().unwrap();
        let store = register(&db).unwrap();
        assert_eq!(
            db.query_row::<i64, _, _>(
                "SELECT count(*) FROM relation_continuation WHERE mode IS NULL",
                [],
                |r| r.get(0)
            )
            .unwrap(),
            0
        );
        assert!(
            db.query_row::<i64, _, _>(
                "SELECT handle FROM relation_continuation WHERE mode IS 1 AND v0 IS 'x'",
                [],
                |r| r.get(0)
            )
            .is_err()
        );
        assert!(store.lock().unwrap().rows.is_empty());
    }
}
