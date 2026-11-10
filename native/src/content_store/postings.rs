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
    const SEGMENTS: Module<SegmentTable> = Module::eponymous_only_module();
    db.create_module(c"nera_segment", &SEGMENTS, None::<()>)?;
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
            0 => ctx.set_result(self.pre),
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
            0..=3 => ctx.set_result(self.edge[col as usize]),
            _ => ctx.set_result(&self.bytes),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.row)
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
pub(crate) fn segment_positions(mut bytes: &[u8]) -> std::result::Result<Vec<u32>, &'static str> {
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
            0 => out.extend(payload.chunks_exact(2).map(|c| high | u32::from(u16::from_le_bytes([c[0], c[1]])))),
            1 => {
                for (w, c) in payload.chunks_exact(8).enumerate() {
                    let mut word = u64::from_le_bytes(c.try_into().expect("8 bytes"));
                    while word != 0 {
                        out.push(high | (w as u32 * 64 + word.trailing_zeros()));
                        word &= word - 1;
                    }
                }
            }
            _ => {
                for c in payload.chunks_exact(4) {
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
