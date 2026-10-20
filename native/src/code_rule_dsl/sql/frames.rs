//! Frames in Rust memory (`docs/sql-set-composition-design.md`, "Speed levers", lever 2): a
//! session's frame and stage tables are instances of the `frame` virtual table, columnar buffers
//! the session owns, instead of TEMP B-trees. A statement appends to a frame through `xUpdate`
//! (`INSERT … SELECT`) and reads it through `xFilter`: a scan in the table's key order, or an
//! equality lookup through an index built on the frame's first read constrained on those columns
//! and kept until the frame is written again. The driver empties a frame directly (`CLEAR`), and
//! an emptied frame keeps its buffers for the next batch, up to `RETAIN` rows.
//!
//! Each frame is an eponymous virtual table: a module registered under the frame's name, whose
//! declaration the module's data carries (`FrameStore::register`). Creating one writes nothing to
//! the schema, so a session's setup takes time linear in its tables; a `CREATE TABLE` would scan
//! the whole schema to parse its own entry, and setup would grow with the square of the tables.
//!
//! A frame keeps the B-tree semantics the statements rely on:
//! - an `INTEGER PRIMARY KEY` column is the rowid: a NULL takes the largest key so far plus one,
//!   a key is unique, and scans run in key order;
//! - a table `PRIMARY KEY (…)` is unique, and scans run in its order;
//! - any other table scans in insertion order;
//! - column types (affinity) and `DEFAULT`s are declared, so SQLite applies them as before.
//!
//! No constraint is omitted: SQLite checks every row a lookup returns, so a lookup only has to
//! return every row SQLite's comparison would accept. It compares as SQLite does across storage
//! classes (NULL, numbers, text, blobs), with numeric affinity on a probe of an `INTEGER` column.
//!
//! Only tables that statements append to and empty whole are frames (`exec`): a table some
//! statement updates or deletes from by a condition stays a B-tree.

use super::nodes::integer;
use rusqlite::types::{ToSqlOutput, Value, ValueRef};
use rusqlite::vtab::{
    Context, CreateVTab, Filters, IndexConstraintOp, IndexFlags, IndexInfo, Inserts, Module,
    UpdateVTab, Updates, VTab, VTabConnection, VTabCursor, VTabKind, sqlite3_vtab,
    sqlite3_vtab_cursor,
};
use rusqlite::{Connection, Error, Result};
use std::borrow::Cow;
use std::cell::{RefCell, UnsafeCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_int};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as Atomic};

/// A step that empties the frame it names (`CLEAR` then the table's name): the driver runs it on
/// the frame's buffers instead of a `DELETE`.
pub(super) const CLEAR: &str = "/*CLEAR*/";

/// A step that makes a frame (`FRAME` then the table's name and its declaration in parentheses:
/// the column definitions and table constraints of the `CREATE TABLE` it stands for, then its
/// indexes as `INDEX (…)`): the driver registers it (`FrameStore::register`).
pub(super) const FRAME: &str = "/*FRAME*/";

/// The most rows a session's frames hold at once: a batch that would hold more fails.
pub(super) const BUDGET: usize = 1 << 26;

/// The rows a frame's buffers keep across batches; a frame that held more frees the rest when
/// the batch ends.
const RETAIN: usize = 1 << 16;

/// The `xBestIndex` cost of a full scan: a stat-less B-tree's row estimate, so that plans weigh
/// a frame as they weighed its table.
const SCAN_COST: f64 = 1_048_576.0;

/// The table a statement's text names after `prefix`, up to a space or parenthesis.
fn table_after<'s>(sql: &'s str, prefix: &str) -> Option<&'s str> {
    let rest = sql.strip_prefix(prefix)?;
    rest.split([' ', '('])
        .next()
        .filter(|name| !name.is_empty())
}

/// A stage's or clause's statements with its frames in Rust memory: each table they create that
/// no statement updates or deletes from by a condition becomes a `FRAME` step, its indexes
/// `INDEX (…)` entries of its declaration (the plans weigh them as before; a frame builds an
/// index on its first read on those columns), and emptying it whole becomes a `CLEAR` step.
pub(super) fn rewrite(statements: Vec<String>) -> Vec<String> {
    #[cfg(feature = "unbounded-work")]
    if std::env::var("RERAIL_SQL_FRAME_STORAGE").is_ok_and(|value| value == "btree") {
        return statements;
    }
    let mut mutated: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Each table's indexed columns, as `INDEX (…)` arguments.
    let mut indexes: HashMap<String, String> = HashMap::new();
    for sql in &statements {
        mutated.extend(table_after(sql, "UPDATE ").map(str::to_owned));
        if let Some(name) = table_after(sql, "DELETE FROM ")
            && sql.len() > "DELETE FROM ".len() + name.len()
        {
            mutated.insert(name.to_owned());
        }
        if sql.starts_with("CREATE INDEX ")
            && let Some((_, on)) = sql.split_once(" ON ")
            && let Some((name, columns)) = on.split_once('(')
        {
            let columns = columns.trim_end().trim_end_matches(')');
            indexes
                .entry(name.trim().to_owned())
                .or_default()
                .push_str(&format!(", INDEX ({columns})"));
        }
    }
    let mut frames = std::collections::HashSet::new();
    let mut rewritten = Vec::with_capacity(statements.len());
    for sql in statements {
        if let Some(rest) = sql.strip_prefix("CREATE TEMP TABLE ") {
            let rest = rest.strip_prefix("IF NOT EXISTS ").unwrap_or(rest);
            if let (Some(open), Some(close)) = (rest.find('('), rest.rfind(')'))
                && rest[close + 1..].trim().is_empty()
            {
                let name = rest[..open].trim();
                // A column default keeps the B-tree: SQLite passes an omitted column as NULL.
                if !mutated.contains(name) && !rest.contains(" DEFAULT ") {
                    let frame = format!(
                        "{FRAME}{name}({}{})",
                        &rest[open + 1..close],
                        indexes.get(name).map_or("", String::as_str)
                    );
                    frames.insert(name.to_owned());
                    rewritten.push(frame);
                    continue;
                }
            }
        }
        if sql.starts_with("CREATE INDEX ")
            && let Some((_, on)) = sql.split_once(" ON ")
            && let Some(name) = on.split([' ', '(']).next()
            && frames.contains(name)
        {
            continue;
        }
        if let Some(name) = sql.strip_prefix("DELETE FROM ")
            && frames.contains(name)
        {
            rewritten.push(format!("{CLEAR}{name}"));
            continue;
        }
        rewritten.push(sql);
    }
    rewritten
}

/// The frames of one session's connection, by table name.
#[derive(Default)]
pub(super) struct FrameStore {
    frames: parking_lot::Mutex<HashMap<String, Arc<Shared>>>,
    /// Rows held across every frame.
    live: AtomicUsize,
    /// The most rows held at once since the store was made.
    peak: AtomicUsize,
}

/// The `frame` module: every frame is an eponymous table of its own registration of it.
const MODULE: Module<'static, FrameTable> = Module::update_module();

/// What a frame's module registration carries: the frame, and the store that counts its rows.
pub(super) struct FrameAux {
    store: Arc<FrameStore>,
    shared: Arc<Shared>,
    schema: CString,
}

/// `text` split at its top-level commas (none inside parentheses or quotes), as SQLite splits a
/// virtual table's arguments.
fn arguments(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quoted, mut from) = (0usize, false, 0);
    for (at, byte) in text.bytes().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b'(' if !quoted => depth += 1,
            b')' if !quoted => depth = depth.saturating_sub(1),
            b',' if !quoted && depth == 0 => {
                parts.push(text[from..at].trim());
                from = at + 1;
            }
            _ => {}
        }
    }
    parts.push(text[from..].trim());
    parts.retain(|part| !part.is_empty());
    parts
}

impl FrameStore {
    /// Reads typed pattern inputs by frame key, without a SQL-side environment copy.
    /// The callback cannot retain a reference into the frame or execute SQL on this connection.
    pub(super) fn pattern_inputs(
        &self,
        descriptor: &str,
        keys: &[i64],
        mut visit: impl FnMut(i64, u32, u32, &[Option<i64>]),
    ) -> Result<()> {
        let mut parts = descriptor.split(',');
        let name = parts.next().unwrap_or_default();
        let shared =
            self.frames.lock().get(name).cloned().ok_or_else(|| {
                Error::ModuleError(format!("pattern: missing input frame {name}"))
            })?;
        let frame = shared.get();
        if frame.columns.len() < 3 {
            return Err(Error::ModuleError(
                "pattern: input frame has no node columns".into(),
            ));
        }
        let columns = parts
            .map(|name| {
                frame
                    .names
                    .iter()
                    .position(|column| column == name)
                    .ok_or_else(|| Error::ModuleError(format!("pattern: no input column {name}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let Key::Rowid(column) = frame.key else {
            return Err(Error::ModuleError(
                "pattern: input requires an integer row key".into(),
            ));
        };
        let order = if frame.ascending {
            None
        } else {
            Some(frame.lookup(&[column])?)
        };
        let mut values = Vec::with_capacity(columns.len());
        for &key in keys {
            // Borrow only while copying this row's scalar inputs. No frame reference is
            // live across the callback, which evaluates the native pattern.
            let frame = shared.get();
            let at = if let Some(order) = &order {
                let at =
                    order.partition_point(|&row| frame.columns[column].integer(row as usize) < key);
                order.get(at).copied().map(|row| row as usize)
            } else if frame.dense && frame.len > 0 {
                usize::try_from(key.wrapping_sub(frame.columns[column].integer(0))).ok()
            } else {
                Some(partition(frame.len, |row| {
                    frame.columns[column].integer(row) < key
                }))
            };
            let Some(row) =
                at.filter(|&row| row < frame.len && frame.columns[column].integer(row) == key)
            else {
                continue;
            };
            let (Some(file), Some(node)) = (
                integer(frame.columns[1].get(row)),
                integer(frame.columns[2].get(row)),
            ) else {
                continue;
            };
            let (Ok(file), Ok(node)) = (u32::try_from(file), u32::try_from(node)) else {
                continue;
            };
            values.clear();
            let mut valid = true;
            for &column in &columns {
                let value = match frame.columns[column].get(row) {
                    ValueRef::Null => None,
                    ValueRef::Integer(value) => Some(value),
                    ValueRef::Text(value) => match std::str::from_utf8(value)
                        .ok()
                        .and_then(|text| text.parse().ok())
                    {
                        Some(value) => Some(value),
                        None => {
                            valid = false;
                            break;
                        }
                    },
                    _ => {
                        valid = false;
                        break;
                    }
                };
                values.push(value);
            }
            if !valid {
                continue;
            }
            visit(key, file, node, &values);
        }
        Ok(())
    }

    /// Makes the frame a `FRAME` step declares (`declaration` is the step after the marker) on
    /// `connection`: a registration of the `frame` module under the frame's name, whose eponymous
    /// table is the frame. A frame of that name already made stays (`CREATE TABLE IF NOT
    /// EXISTS`). Returns the frame's name.
    pub(super) fn register<'d>(
        self: &Arc<Self>,
        connection: &Connection,
        declaration: &'d str,
    ) -> Result<&'d str> {
        let bad = || Error::ModuleError(format!("frame: bad declaration {declaration}"));
        let open = declaration.find('(').ok_or_else(bad)?;
        let close = declaration
            .rfind(')')
            .filter(|&close| close > open)
            .ok_or_else(bad)?;
        let name = declaration[..open].trim();
        if self.frames.lock().contains_key(name) {
            return Ok(name);
        }
        let (schema, frame) = parse(&arguments(&declaration[open + 1..close]))?;
        let shared = Arc::new(Shared(UnsafeCell::new(frame)));
        self.frames
            .lock()
            .insert(name.to_owned(), Arc::clone(&shared));
        connection.create_module(
            name,
            &MODULE,
            Some(FrameAux {
                store: Arc::clone(self),
                shared,
                schema,
            }),
        )?;
        Ok(name)
    }

    /// How many frames the store holds.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.frames.lock().len()
    }

    /// Empties frame `name`, keeping its buffers. `false` where no frame has that name.
    pub(super) fn clear(&self, name: &str) -> bool {
        let Some(shared) = self.frames.lock().get(name).cloned() else {
            return false;
        };
        // SAFETY: the connection's thread runs every statement and clear; no statement runs now.
        let frame = unsafe { shared.get_mut() };
        self.live.fetch_sub(frame.len, Atomic::Relaxed);
        frame.clear();
        true
    }

    /// The rows frame `name` holds (exact: every insert and clear goes through its buffers).
    /// `None` where no frame has that name.
    pub(super) fn rows(&self, name: &str) -> Option<usize> {
        self.frames.lock().get(name).map(|shared| shared.get().len)
    }

    /// Frees what emptied frames keep beyond `RETAIN` rows: a large batch does not keep its
    /// memory for the batches after it.
    pub(super) fn trim(&self) {
        for shared in self.frames.lock().values() {
            // SAFETY: as in `clear`.
            let frame = unsafe { shared.get_mut() };
            if frame.len == 0 {
                for column in &mut frame.columns {
                    column.trim();
                }
            }
        }
    }

    /// The most rows the frames held at once.
    #[cfg(test)]
    pub(super) fn peak(&self) -> usize {
        self.peak.load(Atomic::Relaxed)
    }
}

/// A frame shared by its virtual table, its cursors and the store. A connection is used by one
/// thread at a time and SQLite calls a virtual table's methods one at a time, so a frame is
/// never written while it is read: a statement that reads the frame it inserts into reads it
/// whole first (SQLite materializes such a `SELECT`), and the driver clears frames between
/// statements. Reads borrow it for one method call only.
struct Shared(UnsafeCell<Frame>);

// SAFETY: see `Shared`: one thread at a time, and no read overlaps a write.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    fn get(&self) -> &Frame {
        // SAFETY: see `Shared`.
        unsafe { &*self.0.get() }
    }

    /// # Safety
    /// No other borrow of the frame is live (`Shared`).
    #[allow(clippy::mut_from_ref)]
    unsafe fn get_mut(&self) -> &mut Frame {
        unsafe { &mut *self.0.get() }
    }
}

/// How a frame's rows are identified and ordered.
#[derive(Clone, PartialEq)]
enum Key {
    /// Insertion order; the rowid is the position plus one.
    Position,
    /// An `INTEGER PRIMARY KEY` column: the rowid.
    Rowid(usize),
    /// A table `PRIMARY KEY` over these columns.
    Primary(Vec<usize>),
}

/// An index built on a frame's first read constrained on some columns: the columns, and the rows
/// sorted by them, then in scan order.
type Lookup = (Vec<usize>, Rc<[u32]>);

struct Frame {
    names: Vec<String>,
    columns: Vec<Column>,
    len: usize,
    key: Key,
    /// The column lists the table it stands for was indexed on (its key's included): plans
    /// weigh a lookup on a prefix of one as a B-tree index's.
    indexed: Vec<Vec<usize>>,
    /// `Key::Rowid`: whether the keys were appended in ascending order, and whether they are
    /// consecutive besides; the largest so far.
    ascending: bool,
    dense: bool,
    largest: i64,
    /// Indexes built since the last write.
    lookups: RefCell<Vec<Lookup>>,
    /// The scan order where the rows are not stored in it.
    order: RefCell<Option<Rc<[u32]>>>,
}

impl Frame {
    fn clear(&mut self) {
        for column in &mut self.columns {
            column.clear();
        }
        self.len = 0;
        self.ascending = true;
        self.dense = true;
        self.largest = 0;
        self.lookups.get_mut().clear();
        *self.order.get_mut() = None;
    }

    fn rowid(&self, row: usize) -> i64 {
        match self.key {
            Key::Rowid(column) => self.columns[column].integer(row),
            _ => row as i64 + 1,
        }
    }

    /// Compares rows `a` and `b` on `columns`.
    fn compare_rows(&self, columns: &[usize], a: u32, b: u32) -> Ordering {
        for &column in columns {
            let ordering = self.columns[column].compare_rows(a as usize, b as usize);
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        Ordering::Equal
    }

    /// The rows in scan order, where they are not stored in it.
    fn scan_order(&self) -> Result<Option<Rc<[u32]>>> {
        let columns: &[usize] = match &self.key {
            Key::Position => return Ok(None),
            Key::Rowid(_) if self.ascending => return Ok(None),
            Key::Rowid(column) => std::slice::from_ref(column),
            Key::Primary(columns) => columns,
        };
        if let Some(order) = &*self.order.borrow() {
            return Ok(Some(Rc::clone(order)));
        }
        let identity: Vec<u32> = (0..self.len as u32).collect();
        let sorted = identity
            .windows(2)
            .all(|pair| self.compare_rows(columns, pair[0], pair[1]) == Ordering::Less);
        if sorted {
            // In key order already (and unique): scans need no permutation, but it is
            // remembered so the check runs once.
            let order: Rc<[u32]> = identity.into();
            *self.order.borrow_mut() = Some(Rc::clone(&order));
            return Ok(Some(order));
        }
        let mut rows = identity;
        rows.sort_by(|&a, &b| self.compare_rows(columns, a, b));
        if rows
            .windows(2)
            .any(|pair| self.compare_rows(columns, pair[0], pair[1]) == Ordering::Equal)
        {
            return Err(Error::ModuleError("frame: UNIQUE constraint failed".into()));
        }
        let order: Rc<[u32]> = rows.into();
        *self.order.borrow_mut() = Some(Rc::clone(&order));
        Ok(Some(order))
    }

    /// The index on `columns` (ascending column numbers), built on first use.
    fn lookup(&self, columns: &[usize]) -> Result<Rc<[u32]>> {
        if let Some((_, rows)) = self
            .lookups
            .borrow()
            .iter()
            .find(|(indexed, _)| indexed == columns)
        {
            return Ok(Rc::clone(rows));
        }
        let mut rows: Vec<u32> = match self.scan_order()? {
            Some(order) => order.to_vec(),
            None => (0..self.len as u32).collect(),
        };
        // Stable: rows of one value stay in scan order.
        rows.sort_by(|&a, &b| self.compare_rows(columns, a, b));
        let rows: Rc<[u32]> = rows.into();
        self.lookups
            .borrow_mut()
            .push((columns.to_vec(), Rc::clone(&rows)));
        Ok(rows)
    }
}

/// A value as a lookup compares it: SQLite's order across storage classes.
#[derive(Clone, Copy)]
enum Probe<'a> {
    Null,
    Integer(i64),
    Real(f64),
    Text(&'a [u8]),
    Blob(&'a [u8]),
}

impl<'a> Probe<'a> {
    fn of(value: ValueRef<'a>) -> Self {
        match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(value) => Self::Integer(value),
            ValueRef::Real(value) => Self::Real(value),
            ValueRef::Text(text) => Self::Text(text),
            ValueRef::Blob(blob) => Self::Blob(blob),
        }
    }

    /// `self` compared with an `INTEGER` column: numeric affinity converts a numeric text.
    fn numeric(self) -> Self {
        let Self::Text(text) = self else {
            return self;
        };
        let Ok(text) = std::str::from_utf8(text) else {
            return self;
        };
        let text = text.trim();
        if let Ok(value) = text.parse::<i64>() {
            Self::Integer(value)
        } else if let Ok(value) = text.parse::<f64>()
            && value.is_finite()
        {
            if value.fract() == 0.0 && value.abs() < 9.0e18 {
                Self::Integer(value as i64)
            } else {
                Self::Real(value)
            }
        } else {
            self
        }
    }

    fn class(self) -> u8 {
        match self {
            Self::Null => 0,
            Self::Integer(_) | Self::Real(_) => 1,
            Self::Text(_) => 2,
            Self::Blob(_) => 3,
        }
    }

    fn compare(self, other: Self) -> Ordering {
        match (self, other) {
            (Self::Integer(a), Self::Integer(b)) => a.cmp(&b),
            (Self::Integer(a), Self::Real(b)) => compare_mixed(a, b),
            (Self::Real(a), Self::Integer(b)) => compare_mixed(b, a).reverse(),
            (Self::Real(a), Self::Real(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
            (Self::Text(a), Self::Text(b)) | (Self::Blob(a), Self::Blob(b)) => a.cmp(b),
            (a, b) => a.class().cmp(&b.class()),
        }
    }
}

/// An integer against a real, numerically.
fn compare_mixed(integer: i64, real: f64) -> Ordering {
    if real.is_nan() {
        return Ordering::Greater;
    }
    if real >= 9.3e18 {
        return Ordering::Less;
    }
    if real <= -9.3e18 {
        return Ordering::Greater;
    }
    let truncated = real.trunc() as i64;
    integer.cmp(&truncated).then_with(|| {
        0.0f64
            .partial_cmp(&(real - real.trunc()))
            .unwrap_or(Ordering::Equal)
    })
}

/// One column's values: integers packed while they fit, any value once one does not.
enum Data {
    Narrow(Vec<i32>),
    Wide(Vec<i64>),
    Any(Vec<Value>),
}

struct Column {
    data: Data,
    /// `Narrow`/`Wide`: the rows that are NULL, a bit each (missing words are all clear).
    nulls: Vec<u64>,
    /// Declared `INTEGER`: a probe takes numeric affinity.
    integer: bool,
    /// Declared `NOT NULL DEFAULT` an integer: what a NULL stores. SQLite passes a column an
    /// `INSERT` omits as NULL, and a B-tree would refuse an explicit NULL.
    default: Option<i64>,
}

impl Column {
    fn new(integer: bool, default: Option<i64>) -> Self {
        Self {
            data: Data::Narrow(Vec::new()),
            nulls: Vec::new(),
            integer,
            default,
        }
    }

    fn is_null(&self, row: usize) -> bool {
        self.nulls
            .get(row / 64)
            .is_some_and(|word| word >> (row % 64) & 1 == 1)
    }

    fn get(&self, row: usize) -> ValueRef<'_> {
        match &self.data {
            Data::Narrow(values) if !self.is_null(row) => ValueRef::Integer(values[row].into()),
            Data::Wide(values) if !self.is_null(row) => ValueRef::Integer(values[row]),
            Data::Any(values) => ValueRef::from(&values[row]),
            _ => ValueRef::Null,
        }
    }

    /// Row `row`'s value as an integer (a key column holds integers only).
    fn integer(&self, row: usize) -> i64 {
        match self.get(row) {
            ValueRef::Integer(value) => value,
            _ => 0,
        }
    }

    fn compare_rows(&self, a: usize, b: usize) -> Ordering {
        if self.nulls.is_empty() {
            match &self.data {
                Data::Narrow(values) => return values[a].cmp(&values[b]),
                Data::Wide(values) => return values[a].cmp(&values[b]),
                Data::Any(_) => {}
            }
        }
        Probe::of(self.get(a)).compare(Probe::of(self.get(b)))
    }
}

/// A value of SQLite's as an owned one (a text that is not UTF-8 keeps what it can).
fn owned(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Text(text) => Value::Text(String::from_utf8_lossy(text).into_owned()),
        value => Value::try_from(value).unwrap_or(Value::Null),
    }
}

impl Column {
    fn push(&mut self, row: usize, value: ValueRef<'_>) {
        let value = match (value, self.default) {
            (ValueRef::Null, Some(default)) => ValueRef::Integer(default),
            (value, _) => value,
        };
        match (value, &mut self.data) {
            (ValueRef::Null, Data::Any(values)) => values.push(Value::Null),
            (ValueRef::Null, data) => {
                let word = row / 64;
                if self.nulls.len() <= word {
                    self.nulls.resize(word + 1, 0);
                }
                self.nulls[word] |= 1 << (row % 64);
                match data {
                    Data::Narrow(values) => values.push(0),
                    Data::Wide(values) => values.push(0),
                    Data::Any(_) => unreachable!("matched above"),
                }
            }
            (ValueRef::Integer(value), Data::Narrow(values)) => match i32::try_from(value) {
                Ok(narrow) => values.push(narrow),
                Err(_) => {
                    let mut wide: Vec<i64> = values.iter().map(|&value| value.into()).collect();
                    wide.push(value);
                    self.data = Data::Wide(wide);
                }
            },
            (ValueRef::Integer(value), Data::Wide(values)) => values.push(value),
            (value, Data::Any(values)) => values.push(owned(value)),
            (value, _) => {
                let mut any: Vec<Value> = (0..row).map(|row| owned(self.get(row))).collect();
                any.push(owned(value));
                self.data = Data::Any(any);
                self.nulls.clear();
            }
        }
    }

    fn clear(&mut self) {
        match &mut self.data {
            Data::Narrow(values) => values.clear(),
            _ => self.data = Data::Narrow(Vec::new()),
        }
        self.nulls.clear();
    }

    fn trim(&mut self) {
        if let Data::Narrow(values) = &mut self.data
            && values.capacity() > RETAIN
        {
            values.shrink_to(RETAIN);
        }
        if self.nulls.capacity() > RETAIN / 64 {
            self.nulls.shrink_to(RETAIN / 64);
        }
    }
}

/// A frame's declared schema and empty buffers, from its declaration's entries (`FRAME`): the
/// column definitions and table constraints of the `CREATE TABLE` it stands for, then the
/// table's indexes (`INDEX (…)`, `rewrite`).
fn parse(args: &[&str]) -> Result<(CString, Frame)> {
    let mut declared = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut columns = Vec::new();
    let mut key = Key::Position;
    let mut primary: Option<Vec<String>> = None;
    let mut index_lists: Vec<Vec<String>> = Vec::new();
    let list = |text: &str| -> Vec<String> {
        let text = text.trim().trim_start_matches('(').trim_end_matches(')');
        text.split(',').map(|name| name.trim().to_owned()).collect()
    };
    for &arg in args {
        let arg = arg.trim();
        if let Some(columns) = arg.strip_prefix("PRIMARY KEY") {
            primary = Some(list(columns));
            continue;
        }
        if let Some(columns) = arg.strip_prefix("INDEX ") {
            index_lists.push(list(columns));
            continue;
        }
        let (name, rest) = if let Some(quoted) = arg.strip_prefix('"') {
            let end = quoted
                .find('"')
                .ok_or_else(|| Error::ModuleError(format!("frame: bad column {arg}")))?;
            (&arg[..end + 2], &quoted[end + 1..])
        } else {
            arg.split_at(arg.find(' ').unwrap_or(arg.len()))
        };
        let rest = rest.trim();
        if rest.contains("PRIMARY KEY") {
            key = Key::Rowid(columns.len());
        }
        let decl = rest.replace("PRIMARY KEY", "");
        let default = match decl.split_once("DEFAULT") {
            None => None,
            Some((_, value)) if decl.contains("NOT NULL") => {
                Some(value.trim().parse::<i64>().map_err(|_| {
                    Error::ModuleError(format!("frame: unsupported default in {arg}"))
                })?)
            }
            Some(_) => {
                return Err(Error::ModuleError(format!(
                    "frame: a nullable default in {arg}"
                )));
            }
        };
        let decl = decl.replace("NOT NULL", "");
        let integer = decl
            .split_whitespace()
            .next()
            .is_some_and(|word| word.contains("INT"));
        declared.push(format!("{name} {}", decl.trim()));
        names.push(name.to_owned());
        columns.push(Column::new(integer, default));
    }
    let position = |name: &String| {
        names
            .iter()
            .position(|column| column == name)
            .ok_or_else(|| Error::ModuleError(format!("frame: no column {name}")))
    };
    if let Some(primary) = primary {
        key = Key::Primary(primary.iter().map(position).collect::<Result<Vec<_>>>()?);
    }
    let mut indexed = index_lists
        .iter()
        .map(|columns| columns.iter().map(position).collect::<Result<Vec<_>>>())
        .collect::<Result<Vec<_>>>()?;
    match &key {
        Key::Position => {}
        Key::Rowid(column) => indexed.push(vec![*column]),
        Key::Primary(columns) => indexed.push(columns.clone()),
    }
    let schema = CString::new(format!("CREATE TABLE x({})", declared.join(", ")))
        .map_err(|_| Error::ModuleError("frame: a NUL in the schema".into()))?;
    Ok((
        schema,
        Frame {
            names,
            columns,
            len: 0,
            key,
            indexed,
            ascending: true,
            dense: true,
            largest: 0,
            lookups: RefCell::default(),
            order: RefCell::default(),
        },
    ))
}

#[repr(C)]
pub(super) struct FrameTable {
    base: sqlite3_vtab,
    shared: Arc<Shared>,
    store: Arc<FrameStore>,
}

impl FrameTable {
    fn of(aux: Option<&FrameAux>) -> Result<(Cow<'static, CStr>, Self)> {
        let aux = aux.ok_or_else(|| Error::ModuleError("frame: no frame".into()))?;
        Ok((
            Cow::Owned(aux.schema.clone()),
            Self {
                base: sqlite3_vtab::default(),
                shared: Arc::clone(&aux.shared),
                store: Arc::clone(&aux.store),
            },
        ))
    }
}

unsafe impl<'vtab> VTab<'vtab> for FrameTable {
    type Aux = FrameAux;
    type Cursor = FrameCursor<'vtab>;

    fn connect(
        _db: &mut VTabConnection,
        aux: Option<&FrameAux>,
        _module: &[u8],
        _database: &[u8],
        _table: &[u8],
        _args: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        Self::of(aux)
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        use IndexConstraintOp as Op;
        #[cfg(not(feature = "unbounded-work"))]
        let scan_cost = SCAN_COST;
        #[cfg(feature = "unbounded-work")]
        let scan_cost =
            {
                static COST: std::sync::LazyLock<f64> =
                    std::sync::LazyLock::new(|| {
                        std::env::var("RERAIL_SQL_FRAME_SCAN_COST").map_or(SCAN_COST, |value| {
                    value.parse::<f64>().ok().filter(|n| n.is_finite() && (1.0..=1e12).contains(n))
                        .expect("RERAIL_SQL_FRAME_SCAN_COST must be finite and between 1 and 1e12")
                })
                    });
                *COST
            };
        let frame = self.shared.get();
        // Per column, its first usable `=` or `IS` constraint.
        let mut used: Vec<(usize, usize, bool)> = Vec::new();
        for (index, constraint) in info.constraints().enumerate() {
            if !constraint.is_usable() {
                continue;
            }
            let Ok(column) = usize::try_from(constraint.column()) else {
                continue;
            };
            let is = match constraint.operator() {
                Op::SQLITE_INDEX_CONSTRAINT_EQ => false,
                Op::SQLITE_INDEX_CONSTRAINT_IS => true,
                _ => continue,
            };
            if !used.iter().any(|&(_, used, _)| used == column) {
                used.push((index, column, is));
            }
        }
        used.sort_unstable_by_key(|&(_, column, _)| column);
        let scan_key: Vec<c_int> = match &frame.key {
            Key::Position => vec![-1],
            Key::Rowid(column) => vec![*column as c_int],
            Key::Primary(columns) => columns.iter().map(|&column| column as c_int).collect(),
        };
        let ordered = {
            let count = info.num_of_order_by();
            count > 0
                && count <= scan_key.len()
                && info
                    .order_bys()
                    .zip(&scan_key)
                    .all(|(order, &column)| order.column() == column && !order.is_order_by_desc())
        };
        if ordered {
            info.set_order_by_consumed(true);
        }
        if used.is_empty() {
            info.set_idx_num(0);
            info.set_estimated_cost(scan_cost);
            info.set_estimated_rows(scan_cost as i64);
            return Ok(true);
        }
        let unique = match &frame.key {
            Key::Position => false,
            Key::Rowid(key) => used.iter().any(|&(_, column, _)| column == *key),
            Key::Primary(key) => key
                .iter()
                .all(|key| used.iter().any(|&(_, column, _)| column == *key)),
        };
        // The estimate the table's B-tree plan had: an index's, an automatic index's for a join
        // on other columns, and a filtered scan's for constants on other columns (SQLite's
        // heuristic: an `=` keeps a quarter of the rows, half for -1, 0 or 1).
        let indexed = frame
            .indexed
            .iter()
            .any(|columns| used.iter().any(|&(_, column, _)| column == columns[0]));
        let mut constants = Vec::with_capacity(used.len());
        for &(index, _, _) in &used {
            constants.push(
                info.rhs_value(index)
                    .ok()
                    .flatten()
                    .map(|value| matches!(Probe::of(value).numeric(), Probe::Integer(-1..=1))),
            );
        }
        let (cost, rows) = if unique {
            (20.0, 1)
        } else if indexed {
            (40.0, 10)
        } else if constants.iter().any(Option::is_none) {
            (60.0, 20)
        } else {
            let rows = constants.iter().fold(scan_cost, |rows, small| {
                if *small == Some(true) {
                    rows / 2.0
                } else {
                    rows / 4.0
                }
            });
            (scan_cost, rows as i64)
        };
        let mut codes = String::from("@");
        for (argv, &(index, column, is)) in used.iter().enumerate() {
            let mut usage = info.constraint_usage(index);
            usage.set_argv_index(argv as c_int + 1);
            usage.set_omit(false);
            codes.push(if is { '~' } else { '=' });
            codes.push_str(&column.to_string());
            codes.push(',');
        }
        info.set_idx_num(1);
        info.set_idx_str(&codes);
        if unique {
            info.set_idx_flags(IndexFlags::SQLITE_INDEX_SCAN_UNIQUE);
        }
        info.set_estimated_cost(cost);
        info.set_estimated_rows(rows);
        Ok(true)
    }

    fn open(&'vtab mut self) -> Result<FrameCursor<'vtab>> {
        Ok(FrameCursor {
            base: sqlite3_vtab_cursor::default(),
            shared: Arc::clone(&self.shared),
            rows: Rows::Range { next: 0, end: 0 },
            row: 0,
            phantom: PhantomData,
        })
    }
}

impl CreateVTab<'_> for FrameTable {
    // Eponymous: `xCreate` is `xConnect`, so a frame is never in the schema.
    const KIND: VTabKind = VTabKind::Eponymous;

    fn create(
        _db: &mut VTabConnection,
        aux: Option<&FrameAux>,
        _module: &[u8],
        _database: &[u8],
        _table: &[u8],
        _args: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        Self::of(aux)
    }
}

impl UpdateVTab<'_> for FrameTable {
    fn delete(&mut self, _: ValueRef<'_>) -> Result<()> {
        Err(Error::ModuleError(
            "frame: rows are appended and the frame emptied whole".into(),
        ))
    }

    fn insert(&mut self, args: &Inserts<'_>) -> Result<i64> {
        // SAFETY: SQLite calls `xUpdate` with no cursor of the frame inside a method (`Shared`).
        let frame = unsafe { self.shared.get_mut() };
        if args.len() != frame.columns.len() + 2 {
            return Err(Error::ModuleError("frame: wrong column count".into()));
        }
        let live = self.store.live.fetch_add(1, Atomic::Relaxed) + 1;
        if live > BUDGET {
            self.store.live.fetch_sub(1, Atomic::Relaxed);
            return Err(Error::ModuleError(format!(
                "frame: the batch's frames would hold more than {BUDGET} rows"
            )));
        }
        self.store.peak.fetch_max(live, Atomic::Relaxed);
        let row = frame.len;
        let mut values = args.iter();
        let _old = values.next();
        let new_rowid = values.next().unwrap_or(ValueRef::Null);
        let rowid = match frame.key {
            Key::Rowid(key) => {
                let given = args.iter().nth(key + 2).unwrap_or(ValueRef::Null);
                let value = match given {
                    ValueRef::Null => new_rowid,
                    value => value,
                };
                let rowid = match Probe::of(value).numeric() {
                    Probe::Integer(value) => value,
                    Probe::Real(value) if value.fract() == 0.0 => value as i64,
                    Probe::Null if row == 0 => 1,
                    Probe::Null => frame
                        .largest
                        .checked_add(1)
                        .ok_or_else(|| Error::ModuleError("frame: the rowid overflows".into()))?,
                    _ => return Err(Error::ModuleError("frame: datatype mismatch".into())),
                };
                if row > 0 {
                    if rowid <= frame.largest {
                        frame.ascending = false;
                    }
                    if Some(rowid) != frame.largest.checked_add(1) {
                        frame.dense = false;
                    }
                }
                frame.largest = if row == 0 {
                    rowid
                } else {
                    frame.largest.max(rowid)
                };
                for (column, value) in values.enumerate() {
                    let value = if column == key {
                        ValueRef::Integer(rowid)
                    } else {
                        value
                    };
                    frame.columns[column].push(row, value);
                }
                rowid
            }
            _ => {
                for (column, value) in values.enumerate() {
                    frame.columns[column].push(row, value);
                }
                row as i64 + 1
            }
        };
        frame.len += 1;
        if !frame.lookups.get_mut().is_empty() || frame.order.get_mut().is_some() {
            frame.lookups.get_mut().clear();
            *frame.order.get_mut() = None;
        }
        Ok(rowid)
    }

    fn update(&mut self, _: &Updates<'_>) -> Result<()> {
        Err(Error::ModuleError(
            "frame: rows are appended and the frame emptied whole".into(),
        ))
    }
}

/// The rows a cursor yields, in order.
enum Rows {
    /// Stored rows `next..end`.
    Range { next: usize, end: usize },
    /// `rows[next..end]`.
    Listed {
        rows: Rc<[u32]>,
        next: usize,
        end: usize,
    },
}

#[repr(C)]
pub(super) struct FrameCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    shared: Arc<Shared>,
    rows: Rows,
    /// The current row (valid while not at the end).
    row: usize,
    phantom: PhantomData<&'vtab FrameTable>,
}

/// The most constraints a lookup takes without allocating.
const INLINE: usize = 8;

impl FrameCursor<'_> {
    /// Moves to the next row of `rows`.
    fn step(&mut self) {
        match &mut self.rows {
            Rows::Range { next, end } => {
                if *next < *end {
                    self.row = *next;
                    *next += 1;
                } else {
                    *next = end.saturating_add(1);
                }
            }
            Rows::Listed { rows, next, end } => {
                if *next < *end {
                    self.row = rows[*next] as usize;
                    *next += 1;
                } else {
                    *next = end.saturating_add(1);
                }
            }
        }
    }

    /// The rows matching `probes` (column, value) on the ascending columns `columns`.
    fn find(&mut self, columns: &[usize], probes: &[Probe<'_>]) -> Result<Rows> {
        let frame = self.shared.get();
        // A key lookup in a frame whose keys ascend: by position or by binary search.
        if let ([column], [probe]) = (columns, probes)
            && frame.key == Key::Rowid(*column)
            && frame.ascending
        {
            let key = match *probe {
                Probe::Integer(key) => key,
                Probe::Real(key) if key.fract() == 0.0 && key.abs() < 9.0e18 => key as i64,
                _ => return Ok(Rows::Range { next: 0, end: 0 }),
            };
            let values = &frame.columns[*column];
            let at = if frame.dense && frame.len > 0 {
                let first = values.integer(0);
                usize::try_from(key.wrapping_sub(first))
                    .ok()
                    .filter(|&at| at < frame.len)
            } else {
                let at = partition(frame.len, |row| values.integer(row) < key);
                (at < frame.len && values.integer(at) == key).then_some(at)
            };
            return Ok(match at {
                Some(at) => Rows::Range {
                    next: at,
                    end: at + 1,
                },
                None => Rows::Range { next: 0, end: 0 },
            });
        }
        let rows = frame.lookup(columns)?;
        let compare = |row: u32| -> Ordering {
            for (&column, probe) in columns.iter().zip(probes) {
                let ordering = Probe::of(frame.columns[column].get(row as usize)).compare(*probe);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            Ordering::Equal
        };
        let start = rows.partition_point(|&row| compare(row) == Ordering::Less);
        let end = start + rows[start..].partition_point(|&row| compare(row) == Ordering::Equal);
        Ok(Rows::Listed {
            rows,
            next: start,
            end,
        })
    }
}

/// The first of `0..len` where `before` turns false (it holds on a prefix).
fn partition(len: usize, before: impl Fn(usize) -> bool) -> usize {
    let (mut low, mut high) = (0, len);
    while low < high {
        let middle = low + (high - low) / 2;
        if before(middle) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

unsafe impl VTabCursor for FrameCursor<'_> {
    fn filter(&mut self, idx_num: c_int, idx_str: Option<&str>, args: &Filters<'_>) -> Result<()> {
        if idx_num == 0 {
            let frame = self.shared.get();
            self.rows = match frame.scan_order()? {
                Some(rows) => Rows::Listed {
                    end: rows.len(),
                    rows,
                    next: 0,
                },
                None => Rows::Range {
                    next: 0,
                    end: frame.len,
                },
            };
            self.step();
            return Ok(());
        }
        let codes = idx_str
            .and_then(|codes| codes.strip_prefix('@'))
            .ok_or_else(|| Error::ModuleError("frame: bad plan".into()))?;
        let mut columns = [0usize; INLINE];
        let mut probes = [Probe::Null; INLINE];
        let mut spilled: Option<(Vec<usize>, Vec<Probe<'_>>)> = None;
        let mut empty = false;
        let frame = self.shared.get();
        for (at, (code, value)) in codes.split_terminator(',').zip(args.iter()).enumerate() {
            let (is, column) = match code.as_bytes().first() {
                Some(b'~') => (true, &code[1..]),
                Some(b'=') => (false, &code[1..]),
                _ => return Err(Error::ModuleError("frame: bad plan".into())),
            };
            let column: usize = column
                .parse()
                .map_err(|_| Error::ModuleError("frame: bad plan".into()))?;
            let mut probe = Probe::of(value);
            if frame.columns[column].integer {
                probe = probe.numeric();
            }
            if matches!(probe, Probe::Null) && !is {
                empty = true;
            }
            match &mut spilled {
                Some((more_columns, more_probes)) => {
                    more_columns.push(column);
                    more_probes.push(probe);
                }
                None if at < INLINE => {
                    columns[at] = column;
                    probes[at] = probe;
                }
                None => {
                    let mut more_columns = columns.to_vec();
                    let mut more_probes = probes.to_vec();
                    more_columns.push(column);
                    more_probes.push(probe);
                    spilled = Some((more_columns, more_probes));
                }
            }
        }
        self.rows = if empty {
            Rows::Range { next: 0, end: 0 }
        } else {
            match &spilled {
                Some((columns, probes)) => self.find(columns, probes)?,
                None => {
                    let count = codes.split_terminator(',').count().min(INLINE);
                    self.find(&columns[..count], &probes[..count])?
                }
            }
        };
        self.step();
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.step();
        Ok(())
    }

    fn eof(&self) -> bool {
        match &self.rows {
            Rows::Range { next, end } | Rows::Listed { next, end, .. } => *next > *end,
        }
    }

    fn column(&self, context: &mut Context, column: c_int) -> Result<()> {
        let frame = self.shared.get();
        let value = frame.columns[column as usize].get(self.row);
        context.set_result(&ToSqlOutput::Borrowed(value))
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.shared.get().rowid(self.row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A connection and its frame store, with `sql` run on it: its statements in order, a
    /// `FRAME` step registered.
    fn connection(sql: &str) -> (Connection, Arc<FrameStore>) {
        let store = Arc::new(FrameStore::default());
        let connection = Connection::open_in_memory().expect("opens");
        run(&connection, &store, sql);
        (connection, store)
    }

    fn run(connection: &Connection, store: &Arc<FrameStore>, sql: &str) {
        for statement in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            match statement.strip_prefix(FRAME) {
                Some(declaration) => {
                    store.register(connection, declaration).expect("registers");
                }
                None => connection.execute_batch(statement).expect("runs"),
            }
        }
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

    #[test]
    fn a_rowid_frame_numbers_null_keys_and_scans_in_key_order() {
        let (connection, store) = connection(
            "/*FRAME*/f(k INTEGER PRIMARY KEY, file, node); \
             INSERT INTO f VALUES (5, 0, 50), (2, 0, 20); \
             INSERT INTO f SELECT NULL, 1, 60; \
             INSERT INTO f(file, node) VALUES (1, 70);",
        );
        assert_eq!(
            rows(&connection, "SELECT k, file, node FROM f"),
            [[2, 0, 20], [5, 0, 50], [6, 1, 60], [7, 1, 70]]
                .map(|row| row.map(Value::Integer).to_vec())
                .to_vec()
        );
        assert_eq!(
            rows(&connection, "SELECT node FROM f WHERE k = 5"),
            vec![vec![Value::Integer(50)]]
        );
        assert_eq!(
            rows(&connection, "SELECT rowid FROM f WHERE file = 1"),
            vec![vec![Value::Integer(6)], vec![Value::Integer(7)]]
        );
        assert!(store.clear("f"));
        connection
            .execute_batch("INSERT INTO f SELECT NULL, 3, 30")
            .expect("refills");
        assert_eq!(
            rows(&connection, "SELECT k, file FROM f"),
            vec![vec![Value::Integer(1), Value::Integer(3)]]
        );
    }

    #[test]
    fn a_duplicate_key_fails() {
        let (connection, _) = connection(
            "/*FRAME*/f(k INTEGER PRIMARY KEY, file); \
             INSERT INTO f VALUES (2, 0), (1, 0), (2, 1);",
        );
        assert!(
            connection
                .prepare("SELECT * FROM f")
                .expect("prepares")
                .query([])
                .expect("runs")
                .next()
                .is_err()
        );
    }

    #[test]
    fn defaults_apply_and_a_primary_key_orders_scans() {
        let (connection, _) = connection(
            "/*FRAME*/s(file INTEGER, node INTEGER, unk INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (file, node)); \
             INSERT INTO s(file, node) VALUES (1, 4), (0, 9), (1, 2);",
        );
        assert_eq!(
            rows(&connection, "SELECT file, node, unk FROM s"),
            [[0, 9, 0], [1, 2, 0], [1, 4, 0]]
                .map(|row| row.map(Value::Integer).to_vec())
                .to_vec()
        );
        assert_eq!(
            rows(
                &connection,
                "SELECT unk FROM s WHERE file = '1' AND node = 4"
            ),
            vec![vec![Value::Integer(0)]]
        );
    }

    #[test]
    fn lookups_compare_as_sqlite_does() {
        let (connection, _) = connection(
            "/*FRAME*/f(k INTEGER PRIMARY KEY, file, node, s0); \
             INSERT INTO f SELECT NULL, 0, 1, NULL; \
             INSERT INTO f SELECT NULL, 0, 1, 'x'; \
             INSERT INTO f SELECT NULL, 0, 1, 3000000000; \
             INSERT INTO f SELECT NULL, 0, 1, 2.0; \
             CREATE TABLE p(v); INSERT INTO p VALUES (NULL), ('x'), (3000000000), (2), (7);",
        );
        assert_eq!(
            rows(
                &connection,
                "SELECT p.v, f.k FROM p JOIN f ON f.file = 0 AND f.node = 1 AND f.s0 IS p.v ORDER BY p.rowid"
            ),
            vec![
                vec![Value::Null, Value::Integer(1)],
                vec![Value::Text("x".into()), Value::Integer(2)],
                vec![Value::Integer(3_000_000_000), Value::Integer(3)],
                vec![Value::Integer(2), Value::Integer(4)],
            ]
        );
        assert_eq!(
            rows(
                &connection,
                "SELECT p.v, f.k FROM p JOIN f ON f.s0 = p.v ORDER BY p.rowid"
            )
            .len(),
            3
        );
    }

    #[test]
    fn a_frame_reads_itself_while_appending() {
        let (connection, store) = connection(
            "/*FRAME*/f(k INTEGER PRIMARY KEY, node); \
             INSERT INTO f(node) VALUES (1), (2); \
             INSERT INTO f SELECT NULL, node + 10 FROM f;",
        );
        assert_eq!(
            rows(&connection, "SELECT k, node FROM f"),
            [[1, 1], [2, 2], [3, 11], [4, 12]]
                .map(|row| row.map(Value::Integer).to_vec())
                .to_vec()
        );
        assert_eq!(store.peak(), 4);
    }

    /// What a frame write costs, by path: a TEMP B-tree's `INSERT … SELECT` (with its index), a
    /// frame's `INSERT … SELECT` (`xUpdate` per row), and a sink (the driver steps the `SELECT`
    /// and appends each row to the frame's buffers itself). Run by hand:
    /// `cargo test --release --lib sql::frames::tests::write_paths -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, run by hand"]
    fn write_paths() {
        const ROWS: i64 = 2_000_000;
        let (connection, store) = connection(
            "PRAGMA temp_store = MEMORY; \
             CREATE TEMP TABLE b(k INTEGER PRIMARY KEY, file, node, s0); \
             CREATE INDEX b_at ON b(file, node); \
             /*FRAME*/f(k INTEGER PRIMARY KEY, file, node, s0, INDEX (file, node)); \
             /*FRAME*/s(k INTEGER PRIMARY KEY, file, node, s0, INDEX (file, node));",
        );
        let source = format!(
            "WITH RECURSIVE g(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM g WHERE i < {ROWS}) \
             SELECT NULL, i / 1000, i, i % 7 FROM g"
        );
        let time = |label: &str, run: &mut dyn FnMut()| {
            let mut best = f64::MAX;
            for _ in 0..3 {
                let started = std::time::Instant::now();
                run();
                best = best.min(started.elapsed().as_secs_f64());
            }
            eprintln!(
                "{label}: {:.1} ns/row (best of 3)",
                best * 1e9 / ROWS as f64
            );
        };
        time("generate only", &mut || {
            let mut statement = connection.prepare_cached(&source).expect("prepares");
            let mut rows = statement.query([]).expect("runs");
            while rows.next().expect("a row").is_some() {}
        });
        time("B-tree INSERT … SELECT", &mut || {
            connection.execute("DELETE FROM b", []).expect("clears");
            connection
                .execute(&format!("INSERT INTO b {source}"), [])
                .expect("fills");
        });
        time("frame INSERT … SELECT (xUpdate)", &mut || {
            store.clear("f");
            connection
                .execute(&format!("INSERT INTO f {source}"), [])
                .expect("fills");
        });
        let shared = Arc::clone(store.frames.lock().get("s").expect("frame s"));
        time("frame sink (driver appends)", &mut || {
            store.clear("s");
            let mut statement = connection.prepare_cached(&source).expect("prepares");
            let width = statement.column_count();
            let mut rows = statement.query([]).expect("runs");
            // SAFETY: no statement reads `s` while it is filled.
            let frame = unsafe { shared.get_mut() };
            while let Some(row) = rows.next().expect("a row") {
                let at = frame.len;
                for column in 0..width {
                    let value = row.get_ref(column).expect("a value");
                    let value = if column == 0 {
                        ValueRef::Integer(at as i64 + 1)
                    } else {
                        value
                    };
                    frame.columns[column].push(at, value);
                }
                frame.len += 1;
            }
            store.live.fetch_add(frame.len, Atomic::Relaxed);
        });
        assert_eq!(
            rows(&connection, "SELECT count(*) FROM f"),
            rows(&connection, "SELECT count(*) FROM s")
        );
    }
}
