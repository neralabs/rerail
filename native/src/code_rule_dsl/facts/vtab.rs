//! Read-only eponymous virtual tables over integer relations (decision 0008; 05 Phase 7 row 3).
//!
//! A table is registered once per connection and rebound per wave without DDL or a reprepare: its
//! [`Binding`] swaps the [`Source`] that later filters read, while open cursors keep the one they
//! started with. `xBestIndex` takes an `EQ` or `IN` constraint on an indexed column (the whole `IN`
//! list in one filter) or a range on a range-indexed column. SQLite rechecks every constraint, so a
//! source may answer with a superset. Every column is an integer: kinds, fields and symbols are atoms.
use fallible_iterator::FallibleIterator;
use rusqlite::{
    Connection, Result,
    types::Value,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor,
    },
};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    ffi::{CStr, CString, c_int},
    ops::Range,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
pub(in crate::code_rule_dsl) mod schema;
pub(in crate::code_rule_dsl) mod syntax;

fn failure(message: &str) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.to_owned().into())
}

/// An immutable relation of integer columns, read row by row.
pub(crate) trait Source: Send + Sync {
    fn columns(&self) -> Vec<String>;
    fn len(&self) -> usize;
    fn estimated_len(&self) -> usize {
        self.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize;
    fn indexed(&self, column: usize) -> bool;
    /// Whether an upper bound on `upper` also bounds `lower` (an end bounds its start).
    fn range_upper_compatible(&self, _lower: usize, _upper: usize) -> bool {
        false
    }
    fn range_indexed(&self, _column: usize) -> bool {
        false
    }
    /// The rows whose `column` lies between the bounds (each with its inclusiveness); `None` scans.
    fn range_lookup(&self, _column: usize, _lower: Option<(i64, bool)>, _upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        None
    }
    /// The rows whose `column` is one of `values`; `None` scans.
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>>;
    fn value(&self, row: usize, column: usize) -> i64;
}

#[derive(Default)]
pub(crate) struct Counters {
    pub scans: AtomicU64,
    pub batches: AtomicU64,
    pub candidates: AtomicU64,
    pub columns: AtomicU64,
}

#[derive(Clone)]
pub(crate) struct Binding {
    current: Arc<parking_lot::RwLock<Arc<dyn Source>>>,
    schema: Arc<Metadata>,
    stats: Arc<Counters>,
}
impl Binding {
    pub fn clear(&self) {
        *self.current.write() = self.schema.clone();
    }
    pub fn set(&self, source: Arc<dyn Source>) -> Result<()> {
        if self.schema.columns != source.columns() {
            return Err(failure("native binding schema changed"));
        }
        *self.current.write() = source;
        Ok(())
    }
    pub fn counters(&self) -> &Counters {
        &self.stats
    }
    fn current(&self) -> Arc<dyn Source> {
        self.current.read().clone()
    }
}
struct Metadata {
    columns: Vec<String>,
}
impl Source for Metadata {
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }
    fn len(&self) -> usize {
        0
    }
    fn estimated_matches(&self, _: usize, _: bool) -> usize {
        0
    }
    fn indexed(&self, _: usize) -> bool {
        false
    }
    fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> {
        None
    }
    fn value(&self, _: usize, _: usize) -> i64 {
        unreachable!("an empty binding has no rows")
    }
}

/// A hash over each source's size and index shape, in buckets: statements prepared under one
/// signature keep their plans while sources of the same shape are rebound.
pub(crate) fn plan_signature(sources: &BTreeMap<String, Arc<dyn Source>>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    fn bucket(n: usize) -> u32 {
        usize::BITS - n.leading_zeros()
    }
    for (name, source) in sources {
        name.hash(&mut hash);
        bucket(source.estimated_len()).hash(&mut hash);
        for column in 0..source.columns().len() {
            source.indexed(column).hash(&mut hash);
            source.range_indexed(column).hash(&mut hash);
            if source.indexed(column) {
                bucket(source.estimated_matches(column, false)).hash(&mut hash);
                bucket(source.estimated_matches(column, true)).hash(&mut hash);
            }
        }
    }
    hash.finish()
}

/// The tables registered on one connection, by name. The connection's owner holds it.
#[derive(Default)]
pub(crate) struct Modules {
    bindings: RefCell<HashMap<String, Binding>>,
}
impl Modules {
    /// Binds `source` to the table `name`, registering it on `db` the first time.
    pub(crate) fn bind(&self, db: &Connection, name: &str, source: Arc<dyn Source>) -> Result<()> {
        let mut bindings = self.bindings.borrow_mut();
        if let Some(binding) = bindings.get(name) {
            return binding.set(source);
        }
        let binding = Binding {
            schema: Arc::new(Metadata { columns: source.columns() }),
            current: Arc::new(parking_lot::RwLock::new(source)),
            stats: Arc::new(Counters::default()),
        };
        register(db, name, binding.clone())?;
        bindings.insert(name.into(), binding);
        Ok(())
    }
    pub(crate) fn get(&self, name: &str) -> Option<Binding> {
        self.bindings.borrow().get(name).cloned()
    }
    /// Releases the table without a schema write or a retained row source.
    pub(crate) fn unregister(&self, db: &Connection, name: &str) -> Result<()> {
        self.bindings.borrow_mut().remove(name);
        let name = CString::new(name).map_err(|_| failure("invalid native module name"))?;
        // SAFETY: the live connection owns its module. SQLite releases its auxiliary
        // data and keeps any already-open cursor's Arc ownership intact.
        let code = unsafe {
            rusqlite::ffi::sqlite3_create_module_v2(db.handle(), name.as_ptr(), std::ptr::null(), std::ptr::null_mut(), None)
        };
        if code != rusqlite::ffi::SQLITE_OK {
            return Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None));
        }
        Ok(())
    }
}

#[repr(C)]
struct Table {
    base: rusqlite::ffi::sqlite3_vtab,
    binding: Binding,
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn register(db: &Connection, name: &str, binding: Binding) -> Result<()> {
    if name.is_empty() || name.len() > 120 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(failure("invalid native table name"));
    }
    let module = CString::new(name).map_err(|_| failure("invalid module name"))?;
    const MODULE: Module<Table> = Module::eponymous_only_module();
    db.create_module(module.as_c_str(), &MODULE, Some(binding))?;
    // Resolve xConnect before an authored-query authorizer is installed.
    db.prepare(&format!("SELECT * FROM {}", quote(name)))?;
    Ok(())
}

/// A constant's integer under the column's INTEGER affinity, if it has one: only such constants can
/// equal an integer cell.
fn integer(value: &Value) -> Option<i64> {
    let real = |r: f64| (r.fract() == 0.0 && r >= i64::MIN as f64 && r < i64::MAX as f64).then_some(r as i64);
    match value {
        Value::Integer(i) => Some(*i),
        Value::Real(r) => real(*r),
        Value::Text(t) => t.trim().parse().ok().or_else(|| t.trim().parse().ok().and_then(real)),
        _ => None,
    }
}
/// A range bound widened to an inclusive integer one; `None` when only a scan is safe.
fn bound(value: &Value, inclusive: bool, lower: bool) -> Option<(i64, bool)> {
    if let Some(i) = integer(value) {
        return Some((i, inclusive));
    }
    match value {
        Value::Real(r) if r.is_finite() => {
            let r = if lower { r.floor() } else { r.ceil() };
            (r >= i64::MIN as f64 && r < i64::MAX as f64).then_some((r as i64, true))
        }
        _ => None,
    }
}

// SAFETY: SQLite's base is first; auxiliary data and cursors own Arc references.
unsafe impl<'a> VTab<'a> for Table {
    type Aux = Binding;
    type Cursor = Cursor;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Binding>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        let binding = aux.ok_or_else(|| failure("missing native row source"))?.clone();
        let columns: Vec<String> = binding.schema.columns.iter().map(|c| format!("{} INTEGER", quote(c))).collect();
        let sql = CString::new(format!("CREATE TABLE x({})", columns.join(","))).map_err(|_| failure("invalid native columns"))?;
        Ok((Cow::Owned(sql), Self { base: rusqlite::ffi::sqlite3_vtab::default(), binding }))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let source = self.binding.current();
        let selected = info
            .constraints()
            .enumerate()
            .filter_map(|(i, c)| {
                if c.is_usable()
                    && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
                    && c.column() >= 0
                    && source.indexed(c.column() as usize)
                    && info.collation(i).is_ok_and(|v| v == "BINARY")
                {
                    let batch = info.is_in_constraint(i).ok()?;
                    Some((i, c.column(), source.estimated_matches(c.column() as usize, batch)))
                } else {
                    None
                }
            })
            .min_by_key(|v| v.2);
        let range = info.constraints().enumerate().find_map(|(i, c)| {
            let col = c.column();
            if !c.is_usable() || col < 0 || !source.range_indexed(col as usize) || !info.collation(i).is_ok_and(|v| v == "BINARY") {
                return None;
            }
            if !matches!(
                c.operator(),
                IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GT
                    | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GE
                    | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LT
                    | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LE
            ) {
                return None;
            }
            Some(col)
        });
        if let Some(col) = range
            && selected.as_ref().is_none_or(|(_, _, cost)| *cost > 32)
        {
            let mut lower = None;
            let mut upper = None;
            for (i, c) in info.constraints().enumerate() {
                let upper_equivalent = c.column() >= 0
                    && source.range_upper_compatible(col as usize, c.column() as usize)
                    && matches!(
                        c.operator(),
                        IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LT | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LE
                    );
                if !c.is_usable() || (c.column() != col && !upper_equivalent) || !info.collation(i).is_ok_and(|v| v == "BINARY") {
                    continue;
                }
                // An end bound also bounds the start. SQLite rechecks the original
                // end predicate; this only narrows the candidate start range.
                match c.operator() {
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GE => lower = Some((i, true)),
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GT => lower = Some((i, false)),
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LE => upper = Some((i, true)),
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LT => upper = Some((i, false)),
                    _ => {}
                }
            }
            let mut flags = 1 << 20 | ((col + 1) << 4);
            let mut arg = 1;
            if let Some((i, inclusive)) = lower {
                info.constraint_usage(i).set_argv_index(arg);
                arg += 1;
                flags |= 1;
                if inclusive {
                    flags |= 4;
                }
            }
            if let Some((i, inclusive)) = upper {
                info.constraint_usage(i).set_argv_index(arg);
                flags |= 2;
                if inclusive {
                    flags |= 8;
                }
            }
            info.set_idx_num(flags);
            info.set_estimated_rows(32);
            info.set_estimated_cost(32.0);
            return Ok(true);
        }
        if let Some((i, col, matches)) = selected {
            let batch = info.is_in_constraint(i)?;
            if batch {
                info.set_in_constraint(i, true)?;
            }
            info.constraint_usage(i).set_argv_index(1);
            // No affinity or collation assumptions: SQLite rechecks every filter.
            info.set_idx_num((col + 1) * 2 + i32::from(batch));
            info.set_estimated_rows(matches as i64);
            info.set_estimated_cost(matches.max(1) as f64);
        } else {
            info.set_idx_num(0);
            info.set_estimated_rows(source.estimated_len() as i64);
            info.set_estimated_cost(source.estimated_len().max(1) as f64);
        }
        Ok(true)
    }
    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor {
            base: rusqlite::ffi::sqlite3_vtab_cursor::default(),
            binding: self.binding.clone(),
            source: self.binding.current(),
            range: 0..0,
            selected: None,
            position: 0,
        })
    }
}
#[repr(C)]
struct Cursor {
    base: rusqlite::ffi::sqlite3_vtab_cursor,
    binding: Binding,
    source: Arc<dyn Source>,
    range: Range<usize>,
    selected: Option<Vec<usize>>,
    position: usize,
}
impl Cursor {
    fn row(&self) -> usize {
        self.selected.as_ref().map_or(self.range.start, |r| r[self.position])
    }
}
// SAFETY: SQLite's cursor base is first. No borrowed SQLite value escapes filter.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, index: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let stats = &self.binding.stats;
        stats.scans.fetch_add(1, Ordering::Relaxed);
        self.source = self.binding.current();
        self.position = 0;
        self.selected = None;
        self.range = 0..self.source.len();
        if index & (1 << 20) != 0 {
            let column = ((index & ((1 << 20) - 1)) >> 4) - 1;
            let mut arg = 0;
            let lower = if index & 1 != 0 {
                let v = args.get::<Value>(arg)?;
                arg += 1;
                Some(bound(&v, index & 4 != 0, true))
            } else {
                None
            };
            let upper = if index & 2 != 0 { Some(bound(&args.get::<Value>(arg)?, index & 8 != 0, false)) } else { None };
            // A bound that is not a number leaves the scan to SQLite's recheck.
            if lower.is_none_or(|b| b.is_some()) && upper.is_none_or(|b| b.is_some()) {
                self.selected = self.source.range_lookup(column as usize, lower.flatten(), upper.flatten());
            }
        } else if index != 0 {
            let batch = index % 2 == 1;
            let column = (index / 2 - 1) as usize;
            let mut values = Vec::new();
            if batch {
                stats.batches.fetch_add(1, Ordering::Relaxed);
                let mut iter = args.in_values(0)?;
                while let Some(value) = iter.next()? {
                    values.extend(integer(&Value::try_from(value).map_err(|e| failure(&e.to_string()))?));
                }
                values.sort_unstable();
                values.dedup();
            } else {
                values.extend(integer(&args.get::<Value>(0)?));
            }
            self.selected = self.source.lookup(column, &values);
        }
        let count = self.selected.as_ref().map_or(self.range.len(), Vec::len);
        stats.candidates.fetch_add(count as u64, Ordering::Relaxed);
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        if self.selected.is_some() {
            self.position += 1;
        } else {
            self.range.start += 1;
        }
        Ok(())
    }
    fn eof(&self) -> bool {
        self.selected.as_ref().map_or(self.range.is_empty(), |r| self.position >= r.len())
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        self.binding.stats.columns.fetch_add(1, Ordering::Relaxed);
        ctx.set_result(&self.source.value(self.row(), col as usize))
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.row() as i64 + 1)
    }
}

/// Row-major integer cells with every column indexed: each column's rows sorted by value, built on
/// first use. Producers' compact relations (S `relation.rs`) and small derived relations.
pub(crate) struct Relation {
    columns: Vec<String>,
    cells: Vec<i64>,
    indexes: Box<[OnceLock<ColumnIndex>]>,
}
struct ColumnIndex {
    rows: Vec<u32>,
    distinct: usize,
}
impl Relation {
    pub(crate) fn new(columns: &[&str]) -> Self {
        assert!(!columns.is_empty());
        Self {
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
            cells: Vec::new(),
            indexes: columns.iter().map(|_| OnceLock::new()).collect(),
        }
    }
    pub(crate) fn push(&mut self, row: &[i64]) {
        assert_eq!(row.len(), self.columns.len());
        assert!(self.len() < u32::MAX as usize);
        self.cells.extend_from_slice(row);
        self.indexes.iter_mut().for_each(|i| *i = OnceLock::new());
    }
    fn index(&self, column: usize) -> &ColumnIndex {
        self.indexes[column].get_or_init(|| {
            let mut rows: Vec<u32> = (0..self.len() as u32).collect();
            rows.sort_by_key(|&r| self.value(r as usize, column));
            let distinct = 1 + rows.windows(2).filter(|w| self.value(w[0] as usize, column) != self.value(w[1] as usize, column)).count();
            ColumnIndex { rows, distinct }
        })
    }
    /// The positions in `column`'s index of the rows with values in `lo..=hi`.
    fn between(&self, column: usize, lo: i64, hi: i64) -> Range<usize> {
        let rows = &self.index(column).rows;
        let start = rows.partition_point(|&r| self.value(r as usize, column) < lo);
        let end = rows.partition_point(|&r| self.value(r as usize, column) <= hi);
        start..end.max(start)
    }
}
impl Source for Relation {
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }
    fn len(&self) -> usize {
        self.cells.len() / self.columns.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let one = self.len().div_ceil(self.index(column).distinct.max(1));
        if batch { one * 8 } else { one }
    }
    fn indexed(&self, _: usize) -> bool {
        true
    }
    fn range_indexed(&self, _: usize) -> bool {
        true
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        let lo = match lower {
            None => i64::MIN,
            Some((v, true)) => v,
            Some((v, false)) => match v.checked_add(1) {
                Some(v) => v,
                None => return Some(Vec::new()),
            },
        };
        let hi = match upper {
            None => i64::MAX,
            Some((v, true)) => v,
            Some((v, false)) => match v.checked_sub(1) {
                Some(v) => v,
                None => return Some(Vec::new()),
            },
        };
        let rows = &self.index(column).rows;
        Some(rows[self.between(column, lo, hi)].iter().map(|&r| r as usize).collect())
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let rows = &self.index(column).rows;
        Some(values.iter().flat_map(|&v| rows[self.between(column, v, v)].iter().map(|&r| r as usize)).collect())
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.cells[row * self.columns.len() + column]
    }
}

#[cfg(test)]
mod tests;
