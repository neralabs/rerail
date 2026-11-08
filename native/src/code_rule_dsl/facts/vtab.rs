//! Read-only eponymous virtual tables over integer relations (decision 0008; 05 Phase 7 row 3).
//!
//! A table is registered once per connection and rebound per wave without DDL or a reprepare: its
//! [`Binding`] swaps the [`Source`] that later filters read, while open cursors keep the one they
//! started with. `xBestIndex` takes an `EQ` or `IN` constraint on an indexed column (the whole `IN`
//! list in one filter) or a range on a range-indexed column. Several `EQ` are one filter: a source's
//! sorted order over them all (`composite`), else a probe of the most selective column with the rest
//! checked here; estimates come from the source's distinct counts, so SQL written by hand plans well.
//! SQLite rechecks every constraint, so a source may answer with a superset. Every column is an integer: kinds, fields and symbols are atoms.
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

/// `idx_num` of a filter on several `=` columns, listed in `idx_str`.
const MULTI: c_int = 1 << 22;

fn failure(message: &str) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.to_owned().into())
}

/// A relation of integer columns, stable while SQL cursors read it.
/// A mutable owner must close every reader before changing rows, then refresh its binding
/// and planner estimates. Most producers expose immutable snapshots.
pub(crate) trait Source: Send + Sync {
    fn columns(&self) -> Vec<String>;
    fn len(&self) -> usize;
    fn estimated_len(&self) -> usize {
        self.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize;
    /// The rows estimated to match one value on each of `columns` at once. By default the columns are taken
    /// as independent (each keeps its share of the rows), bounded by the most selective one.
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        let n = self.estimated_len().max(1) as f64;
        let each: Vec<usize> = columns.iter().map(|&c| self.estimated_matches(c, false)).collect();
        let independent = each.iter().fold(n, |rows, &m| rows * (m as f64 / n).min(1.0));
        (independent.ceil() as usize).max(1).min(each.into_iter().min().unwrap_or(usize::MAX))
    }
    /// Whether `lookup_all` answers from one sorted order over several columns.
    fn composite(&self) -> bool {
        false
    }
    /// The rows whose columns equal the values (`(column, value)` by column, at least two), from one sorted
    /// order over them; `None` probes one column and filters the rest.
    fn lookup_all(&self, _equal: &[(usize, i64)]) -> Option<Vec<usize>> {
        None
    }
    /// The column holding a row's document, where a lookup can be kept to one document (`lookup_in`).
    fn doc_column(&self) -> Option<usize> {
        None
    }
    /// The rows whose `column` is one of `values` in document `doc` (by its key): a lookup over a column that
    /// spans the documents (a kind) where the statement binds the document too. `None`: not kept to one.
    fn lookup_in(&self, _column: usize, _values: &[i64], _doc: i64) -> Option<Vec<usize>> {
        None
    }
    /// Whether a lookup on `column` can be kept to one document (`lookup_in`).
    fn kept_to_doc(&self, _column: usize) -> bool {
        false
    }
    /// The rows whose columns hold every `(column, value)` of `eqs`, ascending: a scan, which a source may do faster
    /// than a `value` call per row.
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vec<u32> {
        (0..self.len()).filter(|&r| eqs.iter().all(|&(c, v)| self.value(r, c) == v)).map(|r| r as u32).collect()
    }
    /// Rows are made per looked-up value, so reading them all (a scan, a hash index of every row)
    /// makes every one: a join probes such a source through `lookup`, whatever its match estimate.
    fn made_on_lookup(&self) -> bool {
        false
    }
    /// The planner's statistics (`syntax::statistics`'s rows) where the source knows them without
    /// reading every row: a source whose rows are made on lookup.
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        None
    }
    /// Why rows could not be made, where a source makes them on demand and making them failed (a
    /// stored frame that does not decode). The statement reading it fails with this message rather
    /// than reading the rows as absent.
    fn failure(&self) -> Option<String> {
        None
    }
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
    /// The rows whose `column` is `value`, as a slice of the source's own sorted index (no allocation); `None`
    /// where the source keeps none.
    fn sorted_rows(&self, _column: usize, _value: i64) -> Option<&[u32]> {
        None
    }
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
        BOUND.lock().insert((unsafe { db.handle() } as usize, name.to_owned()), binding.clone());
        bindings.insert(name.into(), binding);
        Ok(())
    }
    pub(crate) fn get(&self, name: &str) -> Option<Binding> {
        self.bindings.borrow().get(name).cloned()
    }
    /// Releases the table without a schema write or a retained row source.
    pub(crate) fn unregister(&self, db: &Connection, name: &str) -> Result<()> {
        self.bindings.borrow_mut().remove(name);
        BOUND.lock().remove(&(unsafe { db.handle() } as usize, name.to_owned()));
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

/// The tables bound on each live connection (by its handle), for readers on that connection that read a source in
/// place rather than through SQL: the native engine's base inputs (`logic::native`).
static BOUND: std::sync::LazyLock<parking_lot::Mutex<HashMap<(usize, String), Binding>>> = std::sync::LazyLock::new(Default::default);

/// The source bound now to the table `name` on `db`'s connection, if one is.
pub(crate) fn bound_source(db: &Connection, name: &str) -> Option<Arc<dyn Source>> {
    BOUND.lock().get(&(unsafe { db.handle() } as usize, name.to_owned())).map(Binding::current)
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
        // The document bound by an `EQ`, where the source can keep a lookup to it (`lookup_in`).
        let doc = source.doc_column().filter(|&d| {
            info.constraints().enumerate().any(|(j, c)| {
                c.is_usable() && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ && c.column() == d as i32 && info.collation(j).is_ok_and(|v| v == "BINARY")
            })
        });
        // A lookup kept to one document reads that document's share of its matches.
        let in_doc = |col: usize, matches: usize| match doc {
            Some(d) if source.kept_to_doc(col) => (matches * source.estimated_matches(d, false) / source.estimated_len().max(1)).max(1),
            _ => matches,
        };
        // Every usable `=` on a column, one value each; an `IN` list only as the probed column.
        let mut equal: Vec<(usize, usize)> = Vec::new();
        let mut probe: Option<(usize, usize, bool, usize)> = None;
        for (i, c) in info.constraints().enumerate() {
            if !c.is_usable()
                || c.operator() != IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
                || c.column() < 0
                || !info.collation(i).is_ok_and(|v| v == "BINARY")
            {
                continue;
            }
            let col = c.column() as usize;
            let batch = info.is_in_constraint(i)?;
            if !batch && !equal.iter().any(|&(_, e)| e == col) {
                equal.push((i, col));
            }
            if source.indexed(col) {
                let matches = in_doc(col, source.estimated_matches(col, batch));
                if probe.is_none_or(|p| matches < p.3) {
                    probe = Some((i, col, batch, matches));
                }
            }
        }
        let columns: Vec<usize> = equal.iter().map(|&(_, c)| c).collect();
        let all = (columns.len() >= 2).then(|| source.estimated_matches_all(&columns));
        let selected = probe.map(|(i, col, _, matches)| (i, col as c_int, matches));
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
            && all.or(selected.map(|s| s.2)).is_none_or(|cost| cost > 32)
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
        if let Some(rows) = all {
            // Several `=`: one probe of a sorted order over them all (`composite`), else a probe of the most
            // selective column (or a scan) with the rest filtered here, before SQLite sees a row. `idx_str` is
            // a tag, `c` (composite), `p` / `b` (probe, of an `IN` list) or `s` (scan), then the columns in
            // argument order, the probed one first.
            let tag = match probe {
                _ if source.composite() && probe.is_none_or(|p| !p.2 || rows <= p.3) => 'c',
                Some((_, _, true, _)) => 'b',
                Some(_) => 'p',
                None => 's',
            };
            let mut order: Vec<(usize, usize)> = Vec::new();
            if let (Some((i, col, batch, _)), 'p' | 'b') = (probe, tag) {
                if batch {
                    info.set_in_constraint(i, true)?;
                }
                order.push((i, col));
            }
            let first = order.first().copied();
            order.extend(equal.iter().copied().filter(|e| Some(*e) != first));
            for (arg, &(i, _)) in order.iter().enumerate() {
                info.constraint_usage(i).set_argv_index(arg as c_int + 1);
            }
            let list: Vec<String> = order.iter().map(|&(_, c)| c.to_string()).collect();
            info.set_idx_num(MULTI);
            info.set_idx_str(&format!("{tag}{}", list.join(",")));
            // The cost is the rows read: the order's run, the probed column's run, or every row.
            let read = match (tag, probe) {
                ('s', _) => source.estimated_len(),
                ('p' | 'b', Some(p)) => p.3,
                _ => rows,
            };
            info.set_estimated_rows(rows.max(1) as i64);
            info.set_estimated_cost((read.max(1) + rows) as f64);
        } else if let Some((i, col, matches)) = selected {
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
    fn filter(&mut self, index: c_int, list: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let stats = &self.binding.stats;
        stats.scans.fetch_add(1, Ordering::Relaxed);
        self.source = self.binding.current();
        self.position = 0;
        self.selected = None;
        self.range = 0..0;
        if index == MULTI {
            let list = list.ok_or_else(|| failure("missing index columns"))?;
            let (tag, list) = list.split_at(1);
            let columns: Vec<usize> = list.split(',').map(|c| c.parse().map_err(|_| failure("bad index columns"))).collect::<Result<_>>()?;
            let mut values = Vec::new();
            let mut equal: Vec<(usize, i64)> = Vec::new();
            let mut matchless = false;
            for (arg, &c) in columns.iter().enumerate() {
                if arg == 0 && tag == "b" {
                    stats.batches.fetch_add(1, Ordering::Relaxed);
                    let mut iter = args.in_values(0)?;
                    while let Some(value) = iter.next()? {
                        values.extend(integer(&Value::try_from(value).map_err(|e| failure(&e.to_string()))?));
                    }
                    values.sort_unstable();
                    values.dedup();
                    continue;
                }
                // A value that is not an integer matches no cell (as a single-column probe answers).
                match integer(&args.get::<Value>(arg)?) {
                    Some(v) => equal.push((c, v)),
                    None => matchless = true,
                }
            }
            let source = &self.source;
            let keep = |r: &usize| equal.iter().all(|&(c, v)| source.value(*r, c) == v);
            self.selected = if matchless {
                Some(Vec::new())
            } else {
                match tag {
                    "c" => {
                        let mut by = equal.clone();
                        by.sort_unstable();
                        source.lookup_all(&by).or_else(|| source.lookup(by[0].0, &[by[0].1]).map(|r| r.into_iter().filter(keep).collect()))
                    }
                    // A probe over a column that spans the documents (a kind), where the document is bound as
                    // well: kept to that document, rather than every document's rows rechecked against it.
                    "p" | "b" => {
                        let probed: Vec<i64> = if tag == "p" { vec![equal[0].1] } else { values.clone() };
                        let doc = source.doc_column().filter(|_| source.kept_to_doc(columns[0])).and_then(|d| equal.iter().find(|&&(c, _)| c == d).map(|&(_, v)| v));
                        doc.and_then(|d| source.lookup_in(columns[0], &probed, d))
                            .or_else(|| source.lookup(columns[0], &probed))
                            .map(|r| r.into_iter().filter(keep).collect())
                    }
                    _ => Some(source.filter_eq(&equal).into_iter().map(|r| r as usize).collect()),
                }
            };
        } else if index & (1 << 20) != 0 {
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
        // Only a scan takes the length: a source whose rows are made on lookup makes all of them for it.
        if self.selected.is_none() {
            self.range = 0..self.source.len();
        }
        if let Some(message) = self.source.failure() {
            return Err(failure(&message));
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

/// S's `query/plan/relation.rs` as copied (0689eb25); merged into this module (decision 0008).
#[allow(dead_code)]
mod s_relation;
/// P's `relational/table.rs` as copied (2c92eb35), over P's test-only relational stage (O18).
#[cfg(test)]
#[allow(dead_code)]
mod p_table;
#[cfg(test)]
mod tests;
