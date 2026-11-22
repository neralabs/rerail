//! Streaming adapter for packed posting segments (`nera_postings`), from PIAT's
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
pub fn append_delta(bytes: &mut Vec<u8>, mut delta: u32) {
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
    const SEGMENTS: Module<SegmentTable> = Module::eponymous_only_module();
    db.create_module(c"nera_segment", &SEGMENTS, None::<()>)
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
            0 => ctx.set_result(self.pre),
            _ => ctx.set_result(&self.bytes),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(i64::from(self.pre))
    }
}

/// A container segment (decision 0045 §5): one tape's positions as chunks of 2^16, each
/// `slice: u16, form: u8, len: u32` then its container, all little-endian:
///
/// - form 0, array: `len` sorted `u16` low bits;
/// - form 1, bitmap: `len` `u64` words, bit `i` of word `w` is low bits `64·w + i`;
/// - form 2, runs: `len` inclusive `(start: u16, last: u16)` intervals, sorted and apart.
///
/// Chunks are in ascending slice order. `space::NodeSet` writes and reads it without decoding
/// member by member; `nera_segment` gives SQL its positions as rows.
pub fn segment_positions(mut bytes: &[u8]) -> std::result::Result<Vec<u32>, &'static str> {
    let invalid = "invalid segment";
    let mut out = Vec::new();
    let mut last_slice: Option<u16> = None;
    while !bytes.is_empty() {
        let (head, rest) = bytes.split_at_checked(7).ok_or(invalid)?;
        let slice = u16::from_le_bytes([head[0], head[1]]);
        if last_slice.is_some_and(|l| l >= slice) {
            return Err(invalid);
        }
        last_slice = Some(slice);
        let len = u32::from_le_bytes([head[3], head[4], head[5], head[6]]) as usize;
        let high = u32::from(slice) << 16;
        let size = match head[2] {
            0 => 2 * len,
            1 => 8 * len,
            2 => 4 * len,
            _ => return Err(invalid),
        };
        let (payload, rest) = rest.split_at_checked(size).ok_or(invalid)?;
        match head[2] {
            0 => out.extend(payload.as_chunks::<2>().0.iter().map(|c| high | u32::from(u16::from_le_bytes([c[0], c[1]])))),
            1 => {
                for (w, c) in payload.as_chunks::<8>().0.iter().enumerate() {
                    let mut word = u64::from_le_bytes(*c);
                    while word != 0 {
                        out.push(high | (w as u32 * 64 + word.trailing_zeros()));
                        word &= word - 1;
                    }
                }
            }
            _ => {
                for c in payload.as_chunks::<4>().0.iter() {
                    let (s, l) = (u16::from_le_bytes([c[0], c[1]]), u16::from_le_bytes([c[2], c[3]]));
                    if s > l {
                        return Err(invalid);
                    }
                    out.extend((u32::from(s)..=u32::from(l)).map(|v| high | v));
                }
            }
        }
        bytes = rest;
    }
    Ok(out)
}

#[repr(C)]
struct SegmentTable {
    base: ffi::sqlite3_vtab,
}
#[repr(C)]
#[derive(Default)]
struct SegmentCursor {
    base: ffi::sqlite3_vtab_cursor,
    positions: Vec<u32>,
    at: usize,
}
// SAFETY: the SQLite base is first; every cursor owns its decoded positions.
unsafe impl<'a> VTab<'a> for SegmentTable {
    type Aux = ();
    type Cursor = SegmentCursor;
    fn connect(
        db: &mut VTabConnection,
        _: Option<&()>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        Ok((Cow::Borrowed(c"CREATE TABLE x(pre INTEGER,segment BLOB HIDDEN)"), Self { base: ffi::sqlite3_vtab::default() }))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut found = false;
        for (c, mut usage) in info.constraints_and_usages() {
            if c.column() == 1 && c.is_usable() && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ {
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
    fn open(&'a mut self) -> Result<SegmentCursor> {
        Ok(SegmentCursor::default())
    }
}
// SAFETY: the SQLite cursor base is first. No borrowed SQLite input escapes filter.
unsafe impl VTabCursor for SegmentCursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let bytes: Vec<u8> = args.get(0)?;
        self.positions = segment_positions(&bytes).map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?;
        self.at = 0;
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.at += 1;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.at >= self.positions.len()
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        match col {
            0 => ctx.set_result(self.positions[self.at]),
            _ => ctx.set_result(rusqlite::types::Null),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.at as i64)
    }
}
