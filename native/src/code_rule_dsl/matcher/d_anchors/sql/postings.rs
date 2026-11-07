//! Streaming adapter for packed posting segments. SQLite owns all selection and joins.
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

pub(super) fn append_delta(bytes: &mut Vec<u8>, mut delta: u32) {
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
    db.create_module(c"nera_postings", &MODULE, None::<()>)
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
