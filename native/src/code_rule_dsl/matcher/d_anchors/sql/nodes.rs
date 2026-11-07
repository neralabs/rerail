//! Streaming packed structural columns for SQL preparation. No rule semantics live here.
use super::reader::{Column, Shared};
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

#[repr(C)]
struct Table {
    base: ffi::sqlite3_vtab,
    reader: Shared,
}
#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    reader: Shared,
    fid: i64,
    pre: usize,
    lo: usize,
    end: usize,
    values: [i64; 5],
}
pub(super) fn install(db: &Connection, reader: Shared) -> Result<()> {
    const MODULE: Module<Table> = Module::eponymous_only_module();
    db.create_module(c"nera_nodes", &MODULE, Some(reader))
}
// SAFETY: SQLite's base is first. The reader belongs to the owning connection.
unsafe impl<'a> VTab<'a> for Table {
    type Aux = Shared;
    type Cursor = Cursor;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Shared>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        Ok((Cow::Borrowed(c"CREATE TABLE x(pre INTEGER,parent INTEGER,end INTEGER,kind INTEGER,field INTEGER,flags INTEGER,fid INTEGER HIDDEN,lo INTEGER HIDDEN,hi INTEGER HIDDEN)"), Self { base: ffi::sqlite3_vtab::default(), reader: aux.expect("reader supplied").clone() }))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut found = [false; 3];
        for (constraint, mut usage) in info.constraints_and_usages() {
            if constraint.is_usable()
                && constraint.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
                && (6..=8).contains(&constraint.column())
            {
                let slot = (constraint.column() - 6) as usize;
                if !found[slot] {
                    found[slot] = true;
                    usage.set_argv_index(slot as c_int + 1);
                    usage.set_omit(true);
                }
            }
        }
        info.set_estimated_cost(10.0);
        info.set_estimated_rows(256);
        Ok(found.into_iter().all(|v| v))
    }
    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            reader: self.reader.clone(),
            fid: 0,
            pre: 0,
            lo: 0,
            end: 0,
            values: [0; 5],
        })
    }
}
impl Cursor {
    fn decode(&mut self) -> Result<()> {
        if self.pre >= self.end {
            return Ok(());
        }
        let mut reader = self.reader.lock();
        let rec = reader.rec(self.fid, self.pre)?;
        let flags = reader.flags(self.fid, self.pre)?;
        let parent = self.pre as i64
            - i64::from(reader.value(self.fid, Column::Parent, self.pre, rec[1])?);
        let end =
            self.pre as i64 + i64::from(reader.value(self.fid, Column::Size, self.pre, rec[2])?);
        let kind = if flags & 8 != 0 { 65535 } else { rec[0] & 1023 };
        self.values = [
            parent,
            end,
            i64::from(kind),
            i64::from(rec[0] >> 10),
            i64::from(flags),
        ];
        #[cfg(feature = "code-rule-profile")]
        {
            reader.counters.decoded_nodes += 1;
        }
        Ok(())
    }
}
// SAFETY: the cursor owns its coordinates and only reads through the shared connection guard.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.fid = args.get(0)?;
        self.pre = usize::try_from(args.get::<i64>(1)?)
            .map_err(|_| rusqlite::Error::UserFunctionError("negative node bound".into()))?;
        self.lo = self.pre;
        self.end = usize::try_from(args.get::<i64>(2)?)
            .map_err(|_| rusqlite::Error::UserFunctionError("negative node bound".into()))?;
        self.decode()
    }
    fn next(&mut self) -> Result<()> {
        self.pre += 1;
        self.decode()
    }
    fn eof(&self) -> bool {
        self.pre >= self.end
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        match col {
            0 => ctx.set_result(&(self.pre as i64)),
            1..=5 => ctx.set_result(&self.values[col as usize - 1]),
            6 => ctx.set_result(&self.fid),
            7 => ctx.set_result(&(self.lo as i64)),
            _ => ctx.set_result(&(self.end as i64)),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.pre as i64)
    }
}
