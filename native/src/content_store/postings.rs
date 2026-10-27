//! Streaming adapter for packed posting segments (`nera_postings`), from Pulling-it-all-together's
//! anchor store. Every store connection has it (`configure_connection`), so the prefilter's SQL
//! decodes postings where it selects them. SQLite owns all selection and joins.
use rusqlite::{
    Connection, Result, ffi,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor,
    },
};
use std::{
    borrow::Cow,
    ffi::{CStr, c_int},
};

#[allow(dead_code)]
pub(crate) fn append_delta(bytes: &mut Vec<u8>, mut delta: u32) {
    while delta >= 128 {
        bytes.push(delta as u8 | 128);
        delta >>= 7;
    }
    bytes.push(delta as u8);
}

#[repr(C)]
struct Table {
    base: ffi::sqlite3_vtab,
}
#[repr(C)]
#[derive(Default)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    bytes: Vec<u8>,
    at: usize,
    pre: u32,
    done: bool,
}
pub(super) fn install(db: &Connection) -> Result<()> {
    const MODULE: Module<Table> = Module::eponymous_only_module();
    db.create_module(c"nera_postings", &MODULE, None::<()>)?;
    const EDGES: Module<EdgeTable> = Module::eponymous_only_module();
    db.create_module(c"nera_edges", &EDGES, None::<()>)
}
// SAFETY: the SQLite base is first; every cursor owns its input segment.
unsafe impl<'a> VTab<'a> for Table {
    type Aux = ();
    type Cursor = Cursor;
    fn connect(
        db: &mut VTabConnection,
        _: Option<&()>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(pre INTEGER,segment BLOB HIDDEN)"),
            Self {
                base: ffi::sqlite3_vtab::default(),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut found = false;
        for (c, mut usage) in info.constraints_and_usages() {
            if c.column() == 1
                && c.is_usable()
                && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                usage.set_argv_index(1);
                usage.set_omit(true);
                found = true;
                break;
            }
        }
        info.set_estimated_cost(10.0);
        info.set_estimated_rows(32);
        Ok(found)
    }
    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor::default())
    }
}
impl Cursor {
    fn advance(&mut self) -> Result<()> {
        self.done = self.at == self.bytes.len();
        if self.done {
            return Ok(());
        }
        let mut delta = 0u32;
        for shift in (0..35).step_by(7) {
            let byte = *self
                .bytes
                .get(self.at)
                .ok_or_else(|| rusqlite::Error::UserFunctionError("truncated posting".into()))?;
            self.at += 1;
            if shift == 28 && byte > 15 {
                break;
            }
            delta |= u32::from(byte & 127) << shift;
            if byte & 128 == 0 {
                self.pre = self
                    .pre
                    .checked_add(delta)
                    .ok_or_else(|| rusqlite::Error::UserFunctionError("posting overflow".into()))?;
                return Ok(());
            }
        }
        Err(rusqlite::Error::UserFunctionError("invalid posting".into()))
    }
}
// SAFETY: the SQLite cursor base is first. No borrowed SQLite input escapes filter.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.bytes = args.get(0)?;
        self.at = 0;
        self.pre = 0;
        self.advance()
    }
    fn next(&mut self) -> Result<()> {
        self.advance()
    }
    fn eof(&self) -> bool {
        self.done
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        match col {
            0 => ctx.set_result(&self.pre),
            _ => ctx.set_result(&self.bytes),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(i64::from(self.pre))
    }
}

/// A tape's direct edges as a segment: (parent kind, child kind, field, count) quads of varints,
/// as `nera_edges` decodes them. The delete trigger on `prefilter_sets` subtracts them from
/// `edge_freq`, so a collected tape counts down exactly (PIAT spec §2.5's `edge_counts`).
pub(crate) fn pack_edges(edges: &[(u32, u32, u16, u32)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for &(parent, child, field, n) in edges {
        for value in [parent, child, u32::from(field), n] {
            append_delta(&mut bytes, value);
        }
    }
    bytes
}

fn varint(bytes: &[u8], at: &mut usize) -> Result<u32> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = *bytes
            .get(*at)
            .ok_or_else(|| rusqlite::Error::UserFunctionError("truncated edge segment".into()))?;
        *at += 1;
        if shift == 28 && byte > 15 {
            break;
        }
        value |= u32::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Ok(value);
        }
    }
    Err(rusqlite::Error::UserFunctionError("invalid edge segment".into()))
}

#[repr(C)]
struct EdgeTable {
    base: ffi::sqlite3_vtab,
}
#[repr(C)]
#[derive(Default)]
struct EdgeCursor {
    base: ffi::sqlite3_vtab_cursor,
    bytes: Vec<u8>,
    at: usize,
    row: i64,
    edge: [u32; 4],
    done: bool,
}
// SAFETY: the SQLite base is first; every cursor owns its input segment.
unsafe impl<'a> VTab<'a> for EdgeTable {
    type Aux = ();
    type Cursor = EdgeCursor;
    fn connect(
        db: &mut VTabConnection,
        _: Option<&()>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(pkind INTEGER,ckind INTEGER,field INTEGER,n INTEGER,segment BLOB HIDDEN)"),
            Self {
                base: ffi::sqlite3_vtab::default(),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut found = false;
        for (c, mut usage) in info.constraints_and_usages() {
            if c.column() == 4
                && c.is_usable()
                && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                usage.set_argv_index(1);
                usage.set_omit(true);
                found = true;
                break;
            }
        }
        info.set_estimated_cost(10.0);
        info.set_estimated_rows(64);
        Ok(found)
    }
    fn open(&'a mut self) -> Result<EdgeCursor> {
        Ok(EdgeCursor::default())
    }
}
impl EdgeCursor {
    fn advance(&mut self) -> Result<()> {
        self.done = self.at == self.bytes.len();
        if self.done {
            return Ok(());
        }
        for slot in 0..4 {
            self.edge[slot] = varint(&self.bytes, &mut self.at)?;
        }
        self.row += 1;
        Ok(())
    }
}
// SAFETY: the SQLite cursor base is first. No borrowed SQLite input escapes filter.
unsafe impl VTabCursor for EdgeCursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.bytes = args.get(0)?;
        self.at = 0;
        self.row = 0;
        self.advance()
    }
    fn next(&mut self) -> Result<()> {
        self.advance()
    }
    fn eof(&self) -> bool {
        self.done
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        match col {
            0..=3 => ctx.set_result(&self.edge[col as usize]),
            _ => ctx.set_result(&self.bytes),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.row)
    }
}
