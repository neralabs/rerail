//! Recursion tables in Rust memory (`docs/sql-set-composition-design.md`, "Speed levers", lever
//! 2, as frames are): a unit's demand and answer tables and a component's reads
//! (`emit::Emitter::recursion_loop`) are instances of the `rec` virtual table, row buffers the
//! session owns, instead of TEMP B-trees. They are the tables a statement updates or deletes from
//! by a condition, which a frame is not (`frames::rewrite`): a loop marks its keys dirty and
//! clean, replaces the answers of the keys it evaluated, and drops the reads they made.
//!
//! A table keeps, besides its rows, the indexes the loop reads it by, each kept as rows change:
//! - its `INTEGER PRIMARY KEY` column, the rowid (a NULL takes the largest key so far plus one,
//!   and a key is unique);
//! - a hash index per `INDEX (…)` of its declaration: a whole-key lookup (a demand row by its
//!   key, a read by the key it is from or the key it reads);
//! - a row set per `SET (column = value)`: the rows holding that value (the dirty keys).
//!
//! No constraint is omitted: SQLite checks every row a lookup returns, so a lookup returns a
//! superset of the rows SQLite's comparison accepts (a hash index chains every key of one hash),
//! comparing as SQLite does across storage classes (an integer equals a real of its value), with
//! numeric affinity on a probe of an `INTEGER` column.
//!
//! A deleted row stays in its slot until the table is emptied or compacted (`RecStore::settle`,
//! between statements): its index entries are passed over, so a delete is O(1) and a cursor
//! never sees a slot change under it. A row's indexed columns are never updated (the loop
//! updates `dirty` alone, which only a set reads).

use rusqlite::types::{ToSqlOutput, Value, ValueRef};
use rusqlite::vtab::{
    Context, CreateVTab, Filters, IndexConstraintOp, IndexFlags, IndexInfo, Inserts, Module,
    UpdateVTab, Updates, VTab, VTabConnection, VTabCursor, VTabKind, sqlite3_vtab,
    sqlite3_vtab_cursor,
};
use rusqlite::{Connection, Error, Result};
use rustc_hash::{FxHashMap, FxHasher};
use std::borrow::Cow;
use std::cell::UnsafeCell;
use std::collections::{BTreeSet, HashMap};
use std::ffi::{CStr, CString, c_int};
use std::hash::Hasher;
use std::marker::PhantomData;
use std::sync::Arc;

/// The end of a hash chain.
const NONE: u32 = u32::MAX;

/// The `xBestIndex` cost of a full scan, as `frames` weighs one: a stat-less B-tree's.
const SCAN_COST: f64 = 1_048_576.0;

/// A table is compacted once it holds at least this many deleted rows, and more than live ones.
const COMPACT: usize = 4096;

/// The `CREATE INDEX` of a recursion table as its declaration's entry: `INDEX (columns)`, or
/// `SET (column = value)` for a partial index `… WHERE column = value` (the rows it holds are
/// what a plan reads it for). `None` for another statement.
pub(super) fn index_entry(sql: &str) -> Option<(&str, String)> {
    let (_, on) = sql.strip_prefix("CREATE INDEX ")?.split_once(" ON ")?;
    let (table, rest) = on.split_once('(')?;
    let (columns, rest) = rest.split_once(')')?;
    let entry = match rest.trim().strip_prefix("WHERE ") {
        None => format!("INDEX ({columns})"),
        Some(condition) => format!("SET ({})", condition.trim()),
    };
    Some((table.trim(), entry))
}

/// A recursion table's declaration from its `CREATE TEMP TABLE`: `name(columns)`.
pub(super) fn declaration(create: &str) -> Option<String> {
    let rest = create.strip_prefix("CREATE TEMP TABLE ")?;
    let rest = rest.strip_prefix("IF NOT EXISTS ").unwrap_or(rest);
    Some(rest.trim().to_owned())
}

/// `declaration` with `entry` added (once).
pub(super) fn add_entry(declaration: &mut String, entry: &str) {
    // Every entry follows a column definition: `, ` then the entry, then `,` or the close.
    let entry = format!(", {entry}");
    let close = declaration.rfind(')').unwrap_or(declaration.len());
    let present = declaration[..close].match_indices(&entry).any(|(at, _)| {
        matches!(
            declaration.as_bytes().get(at + entry.len()),
            Some(b',' | b')')
        )
    });
    if !present {
        declaration.insert_str(close, &entry);
    }
}

/// The recursion tables of one session's connection, by name.
#[derive(Default)]
pub(super) struct RecStore {
    tables: parking_lot::Mutex<HashMap<String, Arc<Shared>>>,
}

/// The `rec` module: every table is an eponymous table of its own registration of it.
const MODULE: Module<'static, RecTable> = Module::update_module();

/// What a table's module registration carries.
pub(super) struct RecAux {
    shared: Arc<Shared>,
    schema: CString,
}

impl RecStore {
    /// Makes the table `declaration` declares on `connection`, unless made: a registration of
    /// the `rec` module under its name, whose eponymous table it is.
    pub(super) fn register(&self, connection: &Connection, declaration: &str) -> Result<()> {
        let bad = || Error::ModuleError(format!("rec: bad declaration {declaration}"));
        let open = declaration.find('(').ok_or_else(bad)?;
        let close = declaration
            .rfind(')')
            .filter(|&close| close > open)
            .ok_or_else(bad)?;
        let name = declaration[..open].trim();
        if self.tables.lock().contains_key(name) {
            return Ok(());
        }
        let (schema, table) = parse(&declaration[open + 1..close])?;
        let shared = Arc::new(Shared(UnsafeCell::new(table)));
        self.tables
            .lock()
            .insert(name.to_owned(), Arc::clone(&shared));
        connection.create_module(name, &MODULE, Some(RecAux { shared, schema }))
    }

    fn table(&self, name: &str) -> Option<Arc<Shared>> {
        self.tables.lock().get(name).cloned()
    }

    /// Empties table `name`, keeping its buffers: the rows it held, `None` where no table has
    /// that name.
    pub(super) fn clear(&self, name: &str) -> Option<usize> {
        let shared = self.table(name)?;
        // SAFETY: the connection's thread runs every statement and clear; no statement runs now.
        let table = unsafe { shared.get_mut() };
        let rows = table.live;
        table.clear();
        Some(rows)
    }

    /// Compacts table `name` where deletes left it mostly dead rows. Between statements only: a
    /// position-keyed row's rowid changes.
    pub(super) fn settle(&self, name: &str) {
        if let Some(shared) = self.table(name) {
            // SAFETY: as in `clear`.
            let table = unsafe { shared.get_mut() };
            let dead = table.slots() - table.live;
            if dead >= COMPACT && dead > table.live {
                table.compact();
            }
        }
    }

    /// Frees every emptied table's buffers, when a batch ends, as `FrameStore::release` frees the
    /// frames': a session has hundreds of recursion tables and a batch runs few of them, so
    /// buffers kept for the next batch add up to every table's largest batch (120 MiB over four
    /// sessions on workspace 01's Tempo and data-layer policies, where every table is empty when
    /// its batch ends). Within a batch, `clear` keeps them for the loop's next pass.
    pub(super) fn release(&self) {
        for shared in self.tables.lock().values() {
            // SAFETY: as in `clear`.
            let table = unsafe { shared.get_mut() };
            if table.live == 0 {
                table.release();
            }
        }
    }

    /// Every table's rows and indexes as they stand (mutant 11 restores them: SQLite's
    /// savepoints do not reach a virtual table's buffers).
    #[cfg(any(test, feature = "sql-mutants"))]
    pub(super) fn snapshot(&self) -> Snapshot {
        Snapshot(
            self.tables
                .lock()
                .values()
                .map(|shared| (Arc::clone(shared), shared.get().clone()))
                .collect(),
        )
    }

    /// Puts back the tables `snapshot` took. Between statements only.
    #[cfg(any(test, feature = "sql-mutants"))]
    pub(super) fn restore(&self, snapshot: Snapshot) {
        for (shared, table) in snapshot.0 {
            // SAFETY: as in `clear`.
            *unsafe { shared.get_mut() } = table;
        }
    }

    /// The bytes the tables' buffers hold allocated (a text or blob value's own bytes aside).
    pub(super) fn bytes(&self) -> usize {
        self.tables
            .lock()
            .values()
            .map(|shared| shared.get().bytes())
            .sum()
    }

    /// The rows table `name` holds.
    #[cfg(test)]
    pub(super) fn rows(&self, name: &str) -> Option<usize> {
        self.table(name).map(|shared| shared.get().live)
    }
}

/// The recursion tables as `RecStore::snapshot` took them.
#[cfg(any(test, feature = "sql-mutants"))]
pub(super) struct Snapshot(Vec<(Arc<Shared>, Table)>);

/// A table shared by its virtual table, its cursors and the store. A connection is used by one
/// thread at a time and SQLite calls a virtual table's methods one at a time; a write never moves
/// a row a cursor holds (`Table`), and the store compacts and clears between statements only.
struct Shared(UnsafeCell<Table>);

// SAFETY: see `Shared`.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    fn get(&self) -> &Table {
        // SAFETY: see `Shared`.
        unsafe { &*self.0.get() }
    }

    /// # Safety
    /// No other borrow of the table is live (`Shared`).
    #[allow(clippy::mut_from_ref)]
    unsafe fn get_mut(&self) -> &mut Table {
        unsafe { &mut *self.0.get() }
    }
}

/// A value's storage: an integer in `values`, NULL, or another value in `others`.
const INTEGER: u8 = 0;
const NULL: u8 = 1;
const OTHER: u8 = 2;

/// A hash index on `columns`: each hash's newest row, then each row's next of its hash.
#[derive(Clone)]
struct Index {
    columns: Vec<usize>,
    heads: FxHashMap<u64, u32>,
    next: Vec<u32>,
}

/// The rows holding `value` in `column`.
#[derive(Clone)]
struct Set {
    column: usize,
    value: i64,
    rows: BTreeSet<u32>,
}

#[derive(Clone)]
struct Table {
    width: usize,
    /// Per column: declared `INTEGER` (a value takes numeric affinity), and its default.
    integer: Vec<bool>,
    defaults: Vec<Option<i64>>,
    /// The `INTEGER PRIMARY KEY` column, the rowid; else a row's rowid is its slot plus one.
    key: Option<usize>,
    values: Vec<i64>,
    tags: Vec<u8>,
    others: Vec<Value>,
    alive: Vec<bool>,
    live: usize,
    largest: Option<i64>,
    /// `key`: the slot of each live rowid.
    slot_of: FxHashMap<i64, u32>,
    indexes: Vec<Index>,
    sets: Vec<Set>,
}

/// A value as an index hashes and compares it: SQLite's equality across storage classes, with
/// an integral real as its integer.
#[derive(Clone, Copy, PartialEq)]
enum Canon<'a> {
    Null,
    Integer(i64),
    Real(u64),
    Text(&'a [u8]),
    Blob(&'a [u8]),
}

impl<'a> Canon<'a> {
    fn of(value: ValueRef<'a>, integer: bool) -> Self {
        match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(value) => Self::Integer(value),
            ValueRef::Real(value) => Self::real(value),
            ValueRef::Text(text) if integer => {
                numeric(text).map_or(Self::Text(text), |value| match value {
                    ValueRef::Integer(value) => Self::Integer(value),
                    ValueRef::Real(value) => Self::real(value),
                    _ => Self::Text(text),
                })
            }
            ValueRef::Text(text) => Self::Text(text),
            ValueRef::Blob(blob) => Self::Blob(blob),
        }
    }

    fn real(value: f64) -> Self {
        if value.fract() == 0.0 && value.abs() < 9.0e18 {
            Self::Integer(value as i64)
        } else {
            Self::Real(value.to_bits())
        }
    }

    fn hash(self, hasher: &mut FxHasher) {
        match self {
            Self::Null => hasher.write_u8(4),
            Self::Integer(value) => {
                hasher.write_u8(0);
                hasher.write_i64(value);
            }
            Self::Real(bits) => {
                hasher.write_u8(1);
                hasher.write_u64(bits);
            }
            Self::Text(bytes) => {
                hasher.write_u8(2);
                hasher.write(bytes);
            }
            Self::Blob(bytes) => {
                hasher.write_u8(3);
                hasher.write(bytes);
            }
        }
    }
}

/// A numeric text as numeric affinity converts it (`INTEGER` columns and their probes).
fn numeric(text: &[u8]) -> Option<ValueRef<'static>> {
    let text = std::str::from_utf8(text).ok()?.trim();
    if let Ok(value) = text.parse::<i64>() {
        return Some(ValueRef::Integer(value));
    }
    let value = text.parse::<f64>().ok().filter(|value| value.is_finite())?;
    Some(if value.fract() == 0.0 && value.abs() < 9.0e18 {
        ValueRef::Integer(value as i64)
    } else {
        ValueRef::Real(value)
    })
}

impl Table {
    fn slots(&self) -> usize {
        self.alive.len()
    }

    fn get(&self, slot: usize, column: usize) -> ValueRef<'_> {
        let at = slot * self.width + column;
        match self.tags[at] {
            INTEGER => ValueRef::Integer(self.values[at]),
            NULL => ValueRef::Null,
            _ => ValueRef::from(&self.others[self.values[at] as usize]),
        }
    }

    fn canon(&self, slot: usize, column: usize) -> Canon<'_> {
        Canon::of(self.get(slot, column), false)
    }

    fn rowid(&self, slot: usize) -> i64 {
        match self.key {
            Some(column) => self.values[slot * self.width + column],
            None => slot as i64 + 1,
        }
    }

    fn slot(&self, rowid: i64) -> Option<usize> {
        match self.key {
            Some(_) => self.slot_of.get(&rowid).map(|&slot| slot as usize),
            None => usize::try_from(rowid - 1)
                .ok()
                .filter(|&slot| slot < self.slots() && self.alive[slot]),
        }
    }

    fn row_hash(&self, slot: usize, columns: &[usize]) -> u64 {
        let mut hasher = FxHasher::default();
        for &column in columns {
            self.canon(slot, column).hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Stores `value` in `column` of the last slot, as the column's affinity and default make it.
    fn push(&mut self, column: usize, value: ValueRef<'_>) {
        let value = match (value, self.defaults[column]) {
            (ValueRef::Null, Some(default)) => ValueRef::Integer(default),
            (ValueRef::Text(text), _) if self.integer[column] => numeric(text).unwrap_or(value),
            (ValueRef::Real(real), _)
                if self.integer[column] && real.fract() == 0.0 && real.abs() < 9.0e18 =>
            {
                ValueRef::Integer(real as i64)
            }
            (value, _) => value,
        };
        match value {
            ValueRef::Integer(value) => {
                self.values.push(value);
                self.tags.push(INTEGER);
            }
            ValueRef::Null => {
                self.values.push(0);
                self.tags.push(NULL);
            }
            value => {
                self.values.push(self.others.len() as i64);
                self.tags.push(OTHER);
                self.others.push(match value {
                    ValueRef::Text(text) => Value::Text(String::from_utf8_lossy(text).into_owned()),
                    value => Value::try_from(value).unwrap_or(Value::Null),
                });
            }
        }
    }

    /// Links the last slot into the indexes and sets.
    fn link(&mut self) {
        let slot = self.slots() - 1;
        for at in 0..self.indexes.len() {
            let hash = self.row_hash(slot, &self.indexes[at].columns);
            let index = &mut self.indexes[at];
            let head = index.heads.insert(hash, slot as u32).unwrap_or(NONE);
            index.next.push(head);
        }
        for at in 0..self.sets.len() {
            let set = &self.sets[at];
            if self.canon(slot, set.column) == Canon::Integer(set.value) {
                self.sets[at].rows.insert(slot as u32);
            }
        }
    }

    fn insert(&mut self, args: &Inserts<'_>) -> Result<i64> {
        if args.len() != self.width + 2 {
            return Err(Error::ModuleError("rec: wrong column count".into()));
        }
        let rowid = match self.key {
            Some(key) => {
                let given = match args.iter().nth(key + 2) {
                    Some(ValueRef::Null) | None => args.iter().nth(1).unwrap_or(ValueRef::Null),
                    Some(value) => value,
                };
                let rowid = match Canon::of(given, true) {
                    Canon::Integer(rowid) => rowid,
                    Canon::Null => match self.largest {
                        None => 1,
                        Some(largest) => largest
                            .checked_add(1)
                            .ok_or_else(|| Error::ModuleError("rec: the rowid overflows".into()))?,
                    },
                    _ => return Err(Error::ModuleError("rec: datatype mismatch".into())),
                };
                if self.slot_of.contains_key(&rowid) {
                    return Err(Error::ModuleError("rec: UNIQUE constraint failed".into()));
                }
                self.largest = Some(self.largest.map_or(rowid, |largest| largest.max(rowid)));
                self.slot_of.insert(rowid, self.slots() as u32);
                Some(rowid)
            }
            None => None,
        };
        for (column, value) in args.iter().skip(2).enumerate() {
            match (rowid, self.key) {
                (Some(rowid), Some(key)) if key == column => {
                    self.push(column, ValueRef::Integer(rowid));
                }
                _ => self.push(column, value),
            }
        }
        self.alive.push(true);
        self.live += 1;
        self.link();
        Ok(rowid.unwrap_or(self.slots() as i64))
    }

    fn delete(&mut self, rowid: i64) -> Result<()> {
        let slot = self
            .slot(rowid)
            .ok_or_else(|| Error::ModuleError("rec: no row to delete".into()))?;
        self.alive[slot] = false;
        self.live -= 1;
        if self.key.is_some() {
            self.slot_of.remove(&rowid);
        }
        for set in &mut self.sets {
            set.rows.remove(&(slot as u32));
        }
        Ok(())
    }

    fn update(&mut self, args: &Updates<'_>) -> Result<()> {
        let old: i64 = args.get(0)?;
        let new: i64 = args.get(1)?;
        if old != new {
            return Err(Error::ModuleError("rec: a rowid is never updated".into()));
        }
        let slot = self
            .slot(old)
            .ok_or_else(|| Error::ModuleError("rec: no row to update".into()))?;
        for (column, value) in args.iter().skip(2).enumerate() {
            let canon = Canon::of(value, self.integer[column]);
            if canon == self.canon(slot, column) {
                continue;
            }
            if self
                .indexes
                .iter()
                .any(|index| index.columns.contains(&column))
                || self.key == Some(column)
            {
                return Err(Error::ModuleError(
                    "rec: an indexed column is never updated".into(),
                ));
            }
            let at = slot * self.width + column;
            match canon {
                Canon::Integer(value) => {
                    self.values[at] = value;
                    self.tags[at] = INTEGER;
                }
                Canon::Null => self.tags[at] = NULL,
                _ => {
                    self.values[at] = self.others.len() as i64;
                    self.tags[at] = OTHER;
                    self.others.push(match value {
                        ValueRef::Text(text) => {
                            Value::Text(String::from_utf8_lossy(text).into_owned())
                        }
                        value => Value::try_from(value).unwrap_or(Value::Null),
                    });
                }
            }
            for set in &mut self.sets {
                if set.column == column {
                    if canon == Canon::Integer(set.value) {
                        set.rows.insert(slot as u32);
                    } else {
                        set.rows.remove(&(slot as u32));
                    }
                }
            }
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.values.clear();
        self.tags.clear();
        self.others.clear();
        self.alive.clear();
        self.live = 0;
        self.largest = None;
        self.slot_of.clear();
        for index in &mut self.indexes {
            index.heads.clear();
            index.next.clear();
        }
        for set in &mut self.sets {
            set.rows.clear();
        }
    }

    /// Drops the deleted rows: the live ones move to the first slots, in order.
    fn compact(&mut self) {
        let width = self.width;
        let mut values = Vec::with_capacity(self.live * width);
        let mut tags = Vec::with_capacity(self.live * width);
        let mut others = Vec::new();
        for slot in (0..self.slots()).filter(|&slot| self.alive[slot]) {
            for column in 0..width {
                let at = slot * width + column;
                if self.tags[at] == OTHER {
                    values.push(others.len() as i64);
                    others.push(self.others[self.values[at] as usize].clone());
                } else {
                    values.push(self.values[at]);
                }
                tags.push(self.tags[at]);
            }
        }
        let (largest, live) = (self.largest, self.live);
        self.clear();
        self.largest = largest;
        self.values = values;
        self.tags = tags;
        self.others = others;
        // `link` links the last slot: each in turn.
        for slot in 0..live {
            self.alive.push(true);
            self.live += 1;
            if let Some(key) = self.key {
                self.slot_of
                    .insert(self.values[slot * width + key], slot as u32);
            }
            self.link();
        }
    }

    /// Empties a table without live rows (its deleted ones dropped, as `compact` drops them) and
    /// frees its buffers. `largest` stays: a NULL key still takes the largest so far plus one.
    fn release(&mut self) {
        let largest = self.largest;
        self.clear();
        self.largest = largest;
        self.values.shrink_to(0);
        self.tags.shrink_to(0);
        self.others.shrink_to(0);
        self.alive.shrink_to(0);
        self.slot_of.shrink_to(0);
        for index in &mut self.indexes {
            index.heads.shrink_to(0);
            index.next.shrink_to(0);
        }
    }

    fn bytes(&self) -> usize {
        let indexes: usize = self
            .indexes
            .iter()
            .map(|index| index.heads.capacity() * 16 + index.next.capacity() * 4)
            .sum();
        let sets: usize = self.sets.iter().map(|set| set.rows.len() * 8).sum();
        self.values.capacity() * 8
            + self.tags.capacity()
            + self.others.capacity() * size_of::<Value>()
            + self.alive.capacity()
            + self.slot_of.capacity() * 16
            + indexes
            + sets
    }
}

/// A table's declared schema and empty buffers from its declaration's entries: its column
/// definitions, then `INDEX (…)` and `SET (column = value)` entries.
fn parse(text: &str) -> Result<(CString, Table)> {
    let mut parts: Vec<&str> = Vec::new();
    let (mut depth, mut from) = (0usize, 0);
    for (at, byte) in text.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(text[from..at].trim());
                from = at + 1;
            }
            _ => {}
        }
    }
    parts.push(text[from..].trim());
    let mut declared = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    let (mut integer, mut defaults) = (Vec::new(), Vec::new());
    let mut key = None;
    let mut index_lists: Vec<&str> = Vec::new();
    let mut set_entries: Vec<&str> = Vec::new();
    let inside = |entry: &'static str, part: &str| -> Option<String> {
        let rest = part.strip_prefix(entry)?.trim();
        Some(
            rest.trim_start_matches('(')
                .trim_end_matches(')')
                .to_owned(),
        )
    };
    let mut owned: Vec<String> = Vec::new();
    for part in parts.iter().copied().filter(|part| !part.is_empty()) {
        if let Some(list) = inside("INDEX", part) {
            owned.push(list);
            index_lists.push(part);
            continue;
        }
        if inside("SET", part).is_some() {
            set_entries.push(part);
            continue;
        }
        let (name, rest) = part.split_at(part.find(' ').unwrap_or(part.len()));
        let rest = rest.trim();
        if rest.contains("PRIMARY KEY") {
            key = Some(names.len());
        }
        let default =
            match rest.split_once("DEFAULT") {
                None => None,
                Some((_, value)) => Some(value.trim().parse::<i64>().map_err(|_| {
                    Error::ModuleError(format!("rec: unsupported default in {part}"))
                })?),
            };
        let decl = rest.replace("PRIMARY KEY", "");
        let decl = decl
            .split("DEFAULT")
            .next()
            .unwrap_or_default()
            .replace("NOT NULL", "");
        integer.push(
            decl.split_whitespace()
                .next()
                .is_some_and(|word| word.contains("INT")),
        );
        defaults.push(default);
        declared.push(format!("{name} {}", decl.trim()));
        names.push(name);
    }
    let position = |name: &str| {
        names
            .iter()
            .position(|column| *column == name.trim())
            .ok_or_else(|| Error::ModuleError(format!("rec: no column {name}")))
    };
    let mut indexes = Vec::new();
    for list in &owned {
        let columns = list.split(',').map(position).collect::<Result<Vec<_>>>()?;
        indexes.push(Index {
            columns,
            heads: FxHashMap::default(),
            next: Vec::new(),
        });
    }
    let mut sets = Vec::new();
    for entry in set_entries {
        let condition = inside("SET", entry).unwrap_or_default();
        let (column, value) = condition
            .split_once('=')
            .ok_or_else(|| Error::ModuleError(format!("rec: bad set {entry}")))?;
        sets.push(Set {
            column: position(column)?,
            value: value
                .trim()
                .parse()
                .map_err(|_| Error::ModuleError(format!("rec: bad set {entry}")))?,
            rows: BTreeSet::new(),
        });
    }
    let schema = CString::new(format!("CREATE TABLE x({})", declared.join(", ")))
        .map_err(|_| Error::ModuleError("rec: a NUL in the schema".into()))?;
    Ok((
        schema,
        Table {
            width: names.len(),
            integer,
            defaults,
            key,
            values: Vec::new(),
            tags: Vec::new(),
            others: Vec::new(),
            alive: Vec::new(),
            live: 0,
            largest: None,
            slot_of: FxHashMap::default(),
            indexes,
            sets,
        },
    ))
}

#[repr(C)]
pub(super) struct RecTable {
    base: sqlite3_vtab,
    shared: Arc<Shared>,
}

impl RecTable {
    fn of(aux: Option<&RecAux>) -> Result<(Cow<'static, CStr>, Self)> {
        let aux = aux.ok_or_else(|| Error::ModuleError("rec: no table".into()))?;
        Ok((
            Cow::Owned(aux.schema.clone()),
            Self {
                base: sqlite3_vtab::default(),
                shared: Arc::clone(&aux.shared),
            },
        ))
    }
}

/// `xBestIndex`'s plans (`idx_num`); `idx_str` says which index or set and, per argument,
/// whether it is an `IS` (`~`) or an `=` (`=`).
const SCAN: c_int = 0;
const ROWID: c_int = 1;
const HASHED: c_int = 2;
const SET: c_int = 3;

unsafe impl<'vtab> VTab<'vtab> for RecTable {
    type Aux = RecAux;
    type Cursor = RecCursor<'vtab>;

    fn connect(
        _db: &mut VTabConnection,
        aux: Option<&RecAux>,
        _module: &[u8],
        _database: &[u8],
        _table: &[u8],
        _args: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        Self::of(aux)
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        use IndexConstraintOp as Op;
        let table = self.shared.get();
        // Per column, its first usable `=` (`=`), `IS` (`~`) or `IS NULL` (`n`, which SQLite
        // passes no value for) constraint.
        let mut used: Vec<(usize, usize, char)> = Vec::new();
        for (index, constraint) in info.constraints().enumerate() {
            if !constraint.is_usable() {
                continue;
            }
            let Ok(column) = usize::try_from(constraint.column()) else {
                continue;
            };
            let code = match constraint.operator() {
                Op::SQLITE_INDEX_CONSTRAINT_EQ => '=',
                Op::SQLITE_INDEX_CONSTRAINT_IS => '~',
                Op::SQLITE_INDEX_CONSTRAINT_ISNULL => 'n',
                _ => continue,
            };
            if !used.iter().any(|&(_, used, _)| used == column) {
                used.push((index, column, code));
            }
        }
        let constraint_on = |column: usize| used.iter().find(|&&(_, used, _)| used == column);
        // The columns looked up, in argument order, and the plan.
        let (plan, label, columns, cost, rows): (c_int, String, Vec<usize>, f64, i64) =
            if let Some(&(_, key, _)) = table
                .key
                .and_then(constraint_on)
                .filter(|&&(_, _, code)| code != 'n')
            {
                (ROWID, String::new(), vec![key], 20.0, 1)
            } else if let Some((at, index)) = table
                .indexes
                .iter()
                .enumerate()
                .filter(|(_, index)| {
                    index
                        .columns
                        .iter()
                        .all(|&column| constraint_on(column).is_some())
                })
                .max_by_key(|(_, index)| index.columns.len())
            {
                (HASHED, at.to_string(), index.columns.clone(), 40.0, 10)
            } else if let Some(at) = table.sets.iter().position(|set| {
                constraint_on(set.column).is_some_and(|&(index, _, code)| {
                    code == '=' && matches!(
                        info.rhs_value(index).ok().flatten().map(|value| Canon::of(value, true)),
                        Some(Canon::Integer(value)) if value == set.value
                    )
                })
            }) {
                (SET, at.to_string(), Vec::new(), 40.0, 10)
            } else {
                // A recursion table grows over a loop's passes, and a scan per pass of another
                // input's rows would make the loop quadratic: a plan scans one only where it
                // must (an acyclic check's, a count's), weighed above any frame scan.
                info.set_idx_num(SCAN);
                info.set_estimated_cost(4.0 * SCAN_COST);
                info.set_estimated_rows(SCAN_COST as i64);
                return Ok(true);
            };
        // `@`, as a frame's lookup reads (`tests::differential::inner_scans`), its codes, then
        // the index or set.
        let mut codes = String::from("@");
        let mut argv = 0;
        for &column in &columns {
            let &(index, _, code) = constraint_on(column).expect("a constrained column");
            if code != 'n' {
                argv += 1;
                let mut usage = info.constraint_usage(index);
                usage.set_argv_index(argv);
                usage.set_omit(false);
            }
            codes.push(code);
        }
        codes.push_str(&label);
        info.set_idx_num(plan);
        info.set_idx_str(&codes);
        if plan == ROWID {
            info.set_idx_flags(IndexFlags::SQLITE_INDEX_SCAN_UNIQUE);
        }
        info.set_estimated_cost(cost);
        info.set_estimated_rows(rows);
        Ok(true)
    }

    fn open(&'vtab mut self) -> Result<RecCursor<'vtab>> {
        Ok(RecCursor {
            base: sqlite3_vtab_cursor::default(),
            shared: Arc::clone(&self.shared),
            rows: Rows::Done,
            row: 0,
            listed: Vec::new(),
            phantom: PhantomData,
        })
    }
}

impl CreateVTab<'_> for RecTable {
    // Eponymous: `xCreate` is `xConnect`, so a table is never in the schema.
    const KIND: VTabKind = VTabKind::Eponymous;

    fn create(
        _db: &mut VTabConnection,
        aux: Option<&RecAux>,
        _module: &[u8],
        _database: &[u8],
        _table: &[u8],
        _args: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        Self::of(aux)
    }
}

impl UpdateVTab<'_> for RecTable {
    fn delete(&mut self, rowid: ValueRef<'_>) -> Result<()> {
        let rowid = rowid
            .as_i64()
            .map_err(|_| Error::ModuleError("rec: a bad rowid".into()))?;
        // SAFETY: SQLite calls `xUpdate` with no method of the table running (`Shared`).
        unsafe { self.shared.get_mut() }.delete(rowid)
    }

    fn insert(&mut self, args: &Inserts<'_>) -> Result<i64> {
        // SAFETY: as in `delete`.
        unsafe { self.shared.get_mut() }.insert(args)
    }

    fn update(&mut self, args: &Updates<'_>) -> Result<()> {
        // SAFETY: as in `delete`.
        unsafe { self.shared.get_mut() }.update(args)
    }
}

/// The slots a cursor yields: every slot from `next`, a hash chain from `next`, or `listed`
/// from `next`.
enum Rows {
    Scan { next: usize },
    Chain { index: usize, next: u32 },
    Listed { next: usize },
    Done,
}

#[repr(C)]
pub(super) struct RecCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    shared: Arc<Shared>,
    rows: Rows,
    /// The current slot (valid while not done).
    row: usize,
    /// A set's rows or a rowid's slot, copied at `filter` (a set may change under the cursor).
    listed: Vec<u32>,
    phantom: PhantomData<&'vtab RecTable>,
}

impl RecCursor<'_> {
    /// Moves to the next live slot.
    fn step(&mut self) {
        let table = self.shared.get();
        loop {
            let slot = match &mut self.rows {
                Rows::Scan { next } => {
                    if *next >= table.slots() {
                        None
                    } else {
                        *next += 1;
                        Some(*next - 1)
                    }
                }
                Rows::Chain { index, next } => {
                    if *next == NONE {
                        None
                    } else {
                        let slot = *next as usize;
                        *next = table.indexes[*index].next[slot];
                        Some(slot)
                    }
                }
                Rows::Listed { next } => {
                    if *next >= self.listed.len() {
                        None
                    } else {
                        *next += 1;
                        Some(self.listed[*next - 1] as usize)
                    }
                }
                Rows::Done => None,
            };
            match slot {
                None => {
                    self.rows = Rows::Done;
                    return;
                }
                Some(slot) if table.alive[slot] => {
                    self.row = slot;
                    return;
                }
                Some(_) => {}
            }
        }
    }
}

unsafe impl VTabCursor for RecCursor<'_> {
    fn filter(&mut self, idx_num: c_int, idx_str: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let table = self.shared.get();
        let bad = || Error::ModuleError("rec: bad plan".into());
        let label = idx_str
            .and_then(|label| label.strip_prefix('@'))
            .unwrap_or("");
        let split = label
            .find(|c: char| !matches!(c, '=' | '~' | 'n'))
            .unwrap_or(label.len());
        let (codes, which) = label.split_at(split);
        // Each looked-up column's probe: its argument, or NULL for an `IS NULL`.
        let probes = || {
            let mut values = args.iter();
            codes.bytes().map(move |code| match code {
                b'n' => (code, ValueRef::Null),
                _ => (code, values.next().unwrap_or(ValueRef::Null)),
            })
        };
        // A NULL `=` probe matches nothing.
        let null_eq = probes().any(|(code, value)| code == b'=' && matches!(value, ValueRef::Null));
        self.rows = match idx_num {
            SCAN => Rows::Scan { next: 0 },
            _ if null_eq => Rows::Done,
            ROWID => {
                self.listed.clear();
                let key = args.iter().next().ok_or_else(bad)?;
                if let Canon::Integer(rowid) = Canon::of(key, true)
                    && let Some(slot) = table.slot(rowid)
                {
                    self.listed.push(slot as u32);
                }
                Rows::Listed { next: 0 }
            }
            HASHED => {
                let at: usize = which.parse().map_err(|_| bad())?;
                let index = table.indexes.get(at).ok_or_else(bad)?;
                let mut hasher = FxHasher::default();
                for (&column, (_, value)) in index.columns.iter().zip(probes()) {
                    Canon::of(value, table.integer[column]).hash(&mut hasher);
                }
                Rows::Chain {
                    index: at,
                    next: index.heads.get(&hasher.finish()).copied().unwrap_or(NONE),
                }
            }
            SET => {
                let at: usize = which.parse().map_err(|_| bad())?;
                let set = table.sets.get(at).ok_or_else(bad)?;
                self.listed.clear();
                self.listed.extend(set.rows.iter().copied());
                Rows::Listed { next: 0 }
            }
            _ => return Err(bad()),
        };
        self.step();
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.step();
        Ok(())
    }

    fn eof(&self) -> bool {
        matches!(self.rows, Rows::Done)
    }

    fn column(&self, context: &mut Context, column: c_int) -> Result<()> {
        let value = self.shared.get().get(self.row, column as usize);
        context.set_result(&ToSqlOutput::Borrowed(value))
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.shared.get().rowid(self.row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(declarations: &[&str]) -> (Connection, RecStore) {
        let connection = Connection::open_in_memory().expect("opens");
        let store = RecStore::default();
        for declaration in declarations {
            store.register(&connection, declaration).expect("registers");
        }
        (connection, store)
    }

    fn rows(connection: &Connection, sql: &str) -> Vec<Vec<Value>> {
        let mut statement = connection.prepare(sql).expect("prepares");
        let width = statement.column_count();
        statement
            .query_map([], |row| {
                (0..width).map(|index| row.get::<_, Value>(index)).collect()
            })
            .expect("queries")
            .collect::<Result<_>>()
            .expect("rows")
    }

    fn ints(rows: &[&[i64]]) -> Vec<Vec<Value>> {
        rows.iter()
            .map(|row| row.iter().map(|&value| Value::Integer(value)).collect())
            .collect()
    }

    #[test]
    fn a_demand_table_numbers_keys_finds_them_whole_and_tracks_dirty_ones() {
        let (connection, store) = store(&[
            "d(id INTEGER PRIMARY KEY, file, node, c0, dirty INTEGER DEFAULT 1, INDEX (file, node, c0), SET (dirty = 1))",
        ]);
        connection
            .execute_batch(
                "INSERT INTO d(file, node, c0) VALUES (0, 5, NULL), (0, 5, 7), (1, 5, 7.0);",
            )
            .expect("inserts");
        assert_eq!(
            rows(
                &connection,
                "SELECT id FROM d WHERE file = 0 AND node = 5 AND c0 IS NULL"
            ),
            ints(&[&[1]])
        );
        // `IS NULL` is a constraint of its own, without a value: the whole key still looks up.
        let plan: Vec<String> = connection
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id FROM d WHERE file = 0 AND node = 5 AND c0 IS NULL",
            )
            .expect("prepares")
            .query_map([], |row| row.get::<_, String>(3))
            .expect("plans")
            .collect::<Result<_>>()
            .expect("plan");
        assert!(
            plan.iter().any(|detail| detail.contains(":@==n0")),
            "{plan:?}"
        );
        assert_eq!(
            rows(
                &connection,
                "SELECT id FROM d WHERE file = 1 AND node = 5 AND c0 IS 7"
            ),
            ints(&[&[3]])
        );
        assert_eq!(
            rows(
                &connection,
                "SELECT id FROM d WHERE file = 0 AND node = 5 AND c0 = NULL"
            ),
            ints(&[])
        );
        connection
            .execute_batch("UPDATE d SET dirty = 0 WHERE id IN (1, 3)")
            .expect("updates");
        assert_eq!(
            rows(&connection, "SELECT id FROM d WHERE dirty = 1"),
            ints(&[&[2]])
        );
        assert_eq!(
            rows(&connection, "SELECT count(*) FROM d WHERE dirty = 1"),
            ints(&[&[1]])
        );
        assert!(
            connection
                .execute_batch("UPDATE d SET node = 6 WHERE id = 2")
                .is_err()
        );
        assert_eq!(store.clear("d"), Some(3));
        connection
            .execute_batch("INSERT INTO d(file, node, c0) VALUES (2, 2, 2)")
            .expect("refills");
        assert_eq!(
            rows(&connection, "SELECT id, dirty FROM d"),
            ints(&[&[1, 1]])
        );
    }

    /// Each clause that makes a shared table adds its indexes again: each is declared once.
    #[test]
    fn an_index_entry_is_added_once() {
        let mut declaration = declaration(
            "CREATE TEMP TABLE IF NOT EXISTS d(id INTEGER PRIMARY KEY, file, node, dirty INTEGER DEFAULT 1)",
        )
        .expect("a declaration");
        for _ in 0..2 {
            for sql in [
                "CREATE INDEX IF NOT EXISTS d_at ON d(file, node)",
                "CREATE INDEX IF NOT EXISTS d_dirty ON d(id) WHERE dirty = 1",
                "CREATE INDEX IF NOT EXISTS d_file ON d(file)",
            ] {
                let (table, entry) = index_entry(sql).expect("an index");
                assert_eq!(table, "d");
                add_entry(&mut declaration, &entry);
            }
        }
        assert_eq!(
            declaration,
            "d(id INTEGER PRIMARY KEY, file, node, dirty INTEGER DEFAULT 1, INDEX (file, node), SET (dirty = 1), INDEX (file))"
        );
    }

    #[test]
    fn an_answer_table_replaces_rows_by_key_and_refuses_a_repeated_key() {
        let (connection, store) = store(&["a(id INTEGER PRIMARY KEY, c0)"]);
        connection
            .execute_batch("INSERT INTO a VALUES (4, 40), (2, 20)")
            .expect("inserts");
        assert!(
            connection
                .execute_batch("INSERT INTO a VALUES (4, 41)")
                .is_err()
        );
        connection
            .execute_batch("DELETE FROM a WHERE id IN (4); INSERT INTO a VALUES (4, 42)")
            .expect("replaces");
        assert_eq!(
            rows(&connection, "SELECT c0 FROM a WHERE id = 4"),
            ints(&[&[42]])
        );
        assert_eq!(store.rows("a"), Some(2));
    }

    #[test]
    fn reads_are_found_by_either_key_and_compacted_after_deletes() {
        let (connection, store) = store(&["e(su, s, du, d, INDEX (du, d), INDEX (su, s))"]);
        connection
            .execute_batch(
                "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 9999) \
                 INSERT INTO e SELECT 1, i, 2, i % 10 FROM n",
            )
            .expect("inserts");
        assert_eq!(
            rows(&connection, "SELECT count(*) FROM e WHERE du = 2 AND d = 3"),
            ints(&[&[1000]])
        );
        connection
            .execute_batch(
                "DELETE FROM e WHERE su = 1 AND s IN (SELECT value FROM json_each('[1,2,3]'))",
            )
            .expect("deletes");
        assert_eq!(store.rows("e"), Some(9997));
        connection
            .execute_batch("DELETE FROM e WHERE s >= 100")
            .expect("deletes");
        store.settle("e");
        assert_eq!(store.get_slots("e"), 97);
        assert_eq!(
            rows(
                &connection,
                "SELECT s FROM e WHERE du = 2 AND d = 3 ORDER BY s"
            ),
            ints(&[
                &[13],
                &[23],
                &[33],
                &[43],
                &[53],
                &[63],
                &[73],
                &[83],
                &[93]
            ])
        );
        assert_eq!(
            rows(&connection, "SELECT d FROM e WHERE su = 1 AND s = 57"),
            ints(&[&[7]])
        );
    }

    impl RecStore {
        fn get_slots(&self, name: &str) -> usize {
            self.table(name).map_or(0, |shared| shared.get().slots())
        }
    }
}
