//! `kept_facts`: the store's kept relations as an eponymous virtual table (decisions 0401 F5, 0402 K6 step 6), read
//! where they lie in each space's chain of mapped files. A row of the table is a row of one entry:
//!
//! ```sql
//! -- every row of `calls`, part 0, in one space
//! SELECT c0, c1 FROM kept_facts WHERE space = ?1 AND rel = 'calls' AND part = 0;
//! -- the rows of `calls` whose first column is 42 (a binary search when it leads the entry's order)
//! SELECT c1 FROM kept_facts WHERE space = ?1 AND rel = 'calls' AND c0 = 42;
//! ```
//!
//! Columns: `space`, `rel`, `ident`, `part`, `level` (the row's level, decision 0090, or NULL), then `c0` to `c15`
//! (a column past the relation's arity is NULL). Values are as the store numbers them: an atom is the store's id,
//! as the `atoms` table answers. `space = ?`, `rel = ?` and `part = ?` choose the entries; `c0 = ?` narrows each
//! entry to a range where its first column leads its canonical order. SQLite rechecks every constraint.
use super::{KeptSpace, chain_rows};
use rerail_blocks::kept::KeptRead as _;
use rerail_blocks::packed::Packed;
use rusqlite::{
    Connection, Result, ffi,
    types::Value,
    vtab::{Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection, VTabCursor},
};
use std::borrow::Cow;
use std::ffi::{CStr, c_int};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The value columns `c0` to `c15`: a relation's first 16 columns.
pub const VALUES: usize = 16;

/// Registers `kept_facts` on `connection`, a connection to the store at `store`.
pub(in crate::content_store) fn install(connection: &Connection, store: &Path) -> Result<()> {
    const MODULE: Module<Table> = Module::eponymous_only_module();
    connection.create_module(c"kept_facts", &MODULE, Some(store.to_path_buf()))
}

#[repr(C)]
struct Table {
    base: ffi::sqlite3_vtab,
    store: PathBuf,
    /// The connection the table is on: a filter reads which files a space's chain has (`facts_chain`) there.
    db: *mut ffi::sqlite3,
}

/// `idx_num` bits: the constraints given, their arguments in this order.
const SPACE: c_int = 1;
const REL: c_int = 2;
const PART: c_int = 4;
const C0: c_int = 8;

/// Columns: `space`, `rel`, `ident`, `part`, `level`, then the values.
const FIRST_VALUE: c_int = 5;

fn schema() -> &'static CStr {
    c"CREATE TABLE x(space BLOB, rel TEXT, ident BLOB, part INTEGER, level INTEGER, c0 INTEGER, c1 INTEGER, c2 INTEGER, c3 INTEGER, c4 INTEGER, c5 INTEGER, c6 INTEGER, c7 INTEGER, c8 INTEGER, c9 INTEGER, c10 INTEGER, c11 INTEGER, c12 INTEGER, c13 INTEGER, c14 INTEGER, c15 INTEGER)"
}

// SAFETY: SQLite's base is first; the table owns its path, and its handle is the connection SQLite calls it on.
unsafe impl<'a> VTab<'a> for Table {
    type Aux = PathBuf;
    type Cursor = Cursor;

    fn connect(db: &mut VTabConnection, aux: Option<&Self::Aux>, _: &[u8], _: &[u8], _: &[u8], _: &[&[u8]]) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        let store = aux.expect("the store's path").clone();
        // SAFETY: the handle of the connection this table is created on; it outlives the table.
        let handle = unsafe { db.handle() };
        Ok((Cow::Borrowed(schema()), Table { base: ffi::sqlite3_vtab::default(), store, db: handle }))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut found: [Option<usize>; 4] = [None; 4];
        for (i, c) in info.constraints().enumerate() {
            if !c.is_usable() || c.operator() != IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ {
                continue;
            }
            let slot = match c.column() {
                0 => 0,
                1 => 1,
                3 => 2,
                FIRST_VALUE => 3,
                _ => continue,
            };
            found[slot].get_or_insert(i);
        }
        let (mut flags, mut arg, mut rows) = (0, 1, 1e7f64);
        for (slot, (bit, narrows)) in [(SPACE, 100.0), (REL, 1000.0), (PART, 10.0), (C0, 100.0)].into_iter().enumerate() {
            if let Some(i) = found[slot] {
                info.constraint_usage(i).set_argv_index(arg);
                arg += 1;
                flags |= bit;
                rows /= narrows;
            }
        }
        info.set_idx_num(flags);
        info.set_estimated_rows(rows.max(1.0) as i64);
        info.set_estimated_cost(rows.max(1.0));
        Ok(true)
    }

    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor { base: ffi::sqlite3_vtab_cursor::default(), store: self.store.clone(), db: self.db, entries: Vec::new(), at: 0, row: 0, rowid: 0 })
    }
}

/// An entry the cursor reads: its rows `positions` of its `Packed`, which lies in `chain`'s mapped files.
struct Entry {
    chain: Arc<KeptSpace>,
    space: [u8; 32],
    rel: String,
    ident: [u8; 32],
    part: i64,
    rows: *const Packed,
    positions: Range<usize>,
}

impl Entry {
    fn rows(&self) -> &Packed {
        // SAFETY: the `Packed` lies in a file `chain` holds mapped, which lives as long as the entry.
        let _held = &self.chain;
        unsafe { &*self.rows }
    }
}

#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    store: PathBuf,
    db: *mut ffi::sqlite3,
    entries: Vec<Entry>,
    at: usize,
    row: usize,
    rowid: i64,
}

fn error(e: super::StoreError) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(e.to_string().into())
}

impl Cursor {
    /// Skips to the first row at or past (`at`, `row`).
    fn settle(&mut self) {
        while let Some(e) = self.entries.get(self.at) {
            if self.row < e.positions.start {
                self.row = e.positions.start;
            }
            if self.row < e.positions.end {
                return;
            }
            self.at += 1;
            self.row = 0;
        }
    }
}

// SAFETY: SQLite's base is first; the cursor owns its entries, each holding its chain mapped.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let mut arg = 0;
        let mut next = |bit: c_int| -> Result<Option<Value>> {
            if idx_num & bit == 0 {
                return Ok(None);
            }
            arg += 1;
            args.get::<Value>(arg - 1).map(Some)
        };
        let (space, rel, part, c0) = (next(SPACE)?, next(REL)?, next(PART)?, next(C0)?);
        // A constraint of another type matches nothing.
        let rel = match rel {
            None => None,
            Some(Value::Text(r)) => Some(r),
            Some(_) => return self.empty(),
        };
        let part = match part {
            None => None,
            Some(Value::Integer(p)) => Some(p),
            Some(_) => return self.empty(),
        };
        let c0 = match c0 {
            None => None,
            Some(Value::Integer(v)) => Some(v),
            Some(_) => return self.empty(),
        };
        // SAFETY: the connection SQLite calls this filter on; not closed by the wrapper.
        let connection = unsafe { Connection::from_handle(self.db) }?;
        let spaces: Vec<[u8; 32]> = match space {
            Some(Value::Blob(b)) => b.as_slice().try_into().ok().into_iter().collect(),
            Some(_) => Vec::new(),
            None => {
                let mut st = connection.prepare_cached("SELECT DISTINCT space FROM main.facts_chain")?;
                let rows = st.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
                rows.filter_map(|r| r.ok().and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())).collect()
            }
        };
        let mut entries = Vec::new();
        for space in spaces {
            let rows = chain_rows(&connection, &space).map_err(error)?;
            if rows.is_empty() {
                continue;
            }
            let chain = Arc::new(KeptSpace::open(&self.store, &space, &rows).map_err(error)?);
            let view = chain.view();
            let kept: Box<dyn Iterator<Item = _>> = match &rel {
                Some(r) => view.of(r),
                None => view.all(),
            };
            for k in kept.filter(|k| part.is_none_or(|p| k.key.part == p)) {
                let n = k.rows.len();
                let positions = match c0 {
                    Some(v) if k.rows.arity > 0 && k.rows.leads(0) => {
                        let (lo, hi) = k.rows.col(0).dec().equal_range(0, n, v);
                        lo..hi
                    }
                    _ => 0..n,
                };
                if positions.is_empty() {
                    continue;
                }
                entries.push(Entry { chain: Arc::clone(&chain), space, rel: k.key.rel.to_owned(), ident: *k.key.ident, part: k.key.part, rows: k.rows as *const Packed, positions });
            }
        }
        self.entries = entries;
        self.at = 0;
        self.row = 0;
        self.rowid = 0;
        self.settle();
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.row += 1;
        self.rowid += 1;
        self.settle();
        Ok(())
    }

    fn eof(&self) -> bool {
        self.at >= self.entries.len()
    }

    fn column(&self, ctx: &mut Context, column: c_int) -> Result<()> {
        let e = &self.entries[self.at];
        match column {
            0 => ctx.set_result(&e.space[..]),
            1 => ctx.set_result(&e.rel),
            2 => ctx.set_result(&e.ident[..]),
            3 => ctx.set_result(&e.part),
            4 => match e.rows().levels() {
                Some(levels) => ctx.set_result(&levels.get(self.row)),
                None => ctx.set_result(&rusqlite::types::Null),
            },
            c => {
                let c = (c - FIRST_VALUE) as usize;
                let rows = e.rows();
                if c < rows.arity as usize { ctx.set_result(&rows.get(self.row, c)) } else { ctx.set_result(&rusqlite::types::Null) }
            }
        }
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.rowid)
    }
}

impl Cursor {
    fn empty(&mut self) -> Result<()> {
        self.entries.clear();
        self.at = 0;
        Ok(())
    }
}
