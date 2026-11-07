//! `atoms(atom, value)`: the arena as an eponymous virtual table (decision 0049 §3), so the SQL
//! that joins atoms keeps its text. `atom = ?` is an array lookup, `value = ?` a hash probe, and a
//! range on `value` (the vocabulary's prefix ranges) a binary search of the ids sorted by string.
//! Pending atoms are visible: SQL never needs a flush first.
use super::{Atoms, Sym};
use rusqlite::{
    Connection, Result, ffi,
    types::Value,
    vtab::{Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection, VTabCursor},
};
use std::borrow::Cow;
use std::ffi::{CStr, c_int};
use std::sync::Arc;

pub(super) fn install(connection: &Connection, atoms: &Arc<Atoms>) -> Result<()> {
    const MODULE: Module<Table> = Module::eponymous_only_module();
    connection.create_module(c"atoms", &MODULE, Some(Arc::clone(atoms)))
}

#[repr(C)]
struct Table {
    base: ffi::sqlite3_vtab,
    atoms: Arc<Atoms>,
}

/// `idx_num`s: a scan, `atom = ?`, `value = ?`, or a range on `value` (bits: lower, upper,
/// lower inclusive, upper inclusive).
const SCAN: c_int = 0;
const BY_ATOM: c_int = 1;
const BY_VALUE: c_int = 2;
const RANGE: c_int = 1 << 8;
const LOWER: c_int = 1;
const UPPER: c_int = 2;

// SAFETY: SQLite's base is first; the table owns an `Arc` of the arena.
unsafe impl<'a> VTab<'a> for Table {
    type Aux = Arc<Atoms>;
    type Cursor = Cursor;
    fn connect(db: &mut VTabConnection, aux: Option<&Self::Aux>, _: &[u8], _: &[u8], _: &[u8], _: &[&[u8]]) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        let atoms = Arc::clone(aux.expect("atoms auxiliary data"));
        Ok((Cow::Borrowed(c"CREATE TABLE x(atom INTEGER, value TEXT)"), Table { base: ffi::sqlite3_vtab::default(), atoms }))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let len = self.atoms.len().max(1);
        // (constraint, column, operator: 0 `=`, 1 `>`/`>=`, 2 `<`/`<=`).
        let usable: Vec<(usize, c_int, u8)> = info
            .constraints()
            .enumerate()
            .filter(|(_, c)| c.is_usable())
            .filter_map(|(i, c)| {
                let op = match c.operator() {
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ => 0,
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GE | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GT => 1,
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LE | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LT => 2,
                    _ => return None,
                };
                Some((i, c.column(), op))
            })
            .collect();
        let binary = |info: &IndexInfo, i: usize| info.collation(i).is_ok_and(|c| c == "BINARY");
        let eq = 0;
        // SQLite rechecks every constraint (no `omit`): a bound of another type only widens.
        if let Some(&(i, _, _)) = usable.iter().find(|(_, column, op)| (*column == 0 || *column == -1) && *op == eq) {
            info.constraint_usage(i).set_argv_index(1);
            info.set_idx_num(BY_ATOM);
            info.set_estimated_rows(1);
            info.set_estimated_cost(1.0);
            return Ok(true);
        }
        if let Some(&(i, _, _)) = usable.iter().find(|(i, column, op)| *column == 1 && *op == eq && binary(info, *i)) {
            info.constraint_usage(i).set_argv_index(1);
            info.set_idx_num(BY_VALUE);
            info.set_estimated_rows(1);
            info.set_estimated_cost(2.0);
            return Ok(true);
        }
        let (mut lower, mut upper) = (None, None);
        for &(i, column, op) in &usable {
            if column != 1 || !binary(info, i) {
                continue;
            }
            match op {
                1 => lower = lower.or(Some(i)),
                2 => upper = upper.or(Some(i)),
                _ => {}
            }
        }
        if lower.is_some() || upper.is_some() {
            let mut flags = RANGE;
            let mut arg = 1;
            if let Some(i) = lower {
                info.constraint_usage(i).set_argv_index(arg);
                arg += 1;
                flags |= LOWER;
            }
            if let Some(i) = upper {
                info.constraint_usage(i).set_argv_index(arg);
                flags |= UPPER;
            }
            info.set_idx_num(flags);
            let rows = (len / if lower.is_some() && upper.is_some() { 100 } else { 4 }).max(1);
            info.set_estimated_rows(rows as i64);
            info.set_estimated_cost(rows as f64 + (len as f64).log2());
            return Ok(true);
        }
        info.set_idx_num(SCAN);
        info.set_estimated_rows(len as i64);
        info.set_estimated_cost(len as f64);
        Ok(true)
    }

    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor { base: ffi::sqlite3_vtab_cursor::default(), atoms: Arc::clone(&self.atoms), rows: Rows::Ids(Vec::new()), at: 0 })
    }
}

enum Rows {
    /// Every atom below an end, in id order.
    Span(usize),
    /// These atoms, in this order.
    Ids(Vec<Sym>),
}

#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    atoms: Arc<Atoms>,
    rows: Rows,
    at: usize,
}

impl Cursor {
    fn atom(&self) -> Sym {
        match &self.rows {
            Rows::Span(_) => self.at as Sym,
            Rows::Ids(ids) => ids[self.at],
        }
    }
}

fn error(error: super::StoreError) -> rusqlite::Error {
    rusqlite::Error::ModuleError(error.to_string())
}

/// A text bound, or `None` when the bound is of another type (no bound: SQLite rechecks).
fn text(value: Value) -> Option<Vec<u8>> {
    match value {
        Value::Text(text) => Some(text.into_bytes()),
        _ => None,
    }
}

// SAFETY: SQLite's base is first; the cursor owns its rows and an `Arc` of the arena.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.at = 0;
        self.rows = match idx_num {
            BY_ATOM => {
                let atom = match args.get::<Value>(0)? {
                    Value::Integer(atom) => u32::try_from(atom).ok(),
                    _ => None,
                };
                match atom {
                    Some(atom) if (atom as usize) < self.atoms.len() => Rows::Ids(vec![atom]),
                    Some(atom) if self.atoms.value(atom).map_err(error)?.is_some() => Rows::Ids(vec![atom]),
                    // An id never given out, or not an integer (SQLite rechecks a scan).
                    Some(_) => Rows::Ids(Vec::new()),
                    None => self.scan()?,
                }
            }
            BY_VALUE => match args.get::<Value>(0)? {
                Value::Text(value) => Rows::Ids(self.atoms.lookup(&value).map_err(error)?.into_iter().collect()),
                _ => self.scan()?,
            },
            flags if flags & RANGE != 0 => {
                let mut arg = 0;
                let lower = if flags & LOWER != 0 {
                    arg += 1;
                    text(args.get::<Value>(arg - 1)?)
                } else {
                    None
                };
                let upper = if flags & UPPER != 0 { text(args.get::<Value>(arg)?) } else { None };
                Rows::Ids(self.range(lower.as_deref(), upper.as_deref())?)
            }
            _ => self.scan()?,
        };
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.at += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        match &self.rows {
            Rows::Span(end) => self.at >= *end,
            Rows::Ids(ids) => self.at >= ids.len(),
        }
    }

    fn column(&self, ctx: &mut Context, column: c_int) -> Result<()> {
        let atom = self.atom();
        match column {
            0 => ctx.set_result(&i64::from(atom)),
            _ => self.atoms.with_str(atom, |value| ctx.set_result(&value)).expect("the cursor's atoms are in the arena"),
        }
    }

    fn rowid(&self) -> Result<i64> {
        Ok(i64::from(self.atom()))
    }
}

impl Cursor {
    fn scan(&self) -> Result<Rows> {
        self.atoms.catch_up().map_err(error)?;
        Ok(Rows::Span(self.atoms.len()))
    }

    /// The atoms whose strings lie between `lower` and `upper` (both inclusive here: SQLite
    /// rechecks strictness), in string order.
    fn range(&self, lower: Option<&[u8]>, upper: Option<&[u8]>) -> Result<Vec<Sym>> {
        self.atoms.catch_up().map_err(error)?;
        let mut sorted = self.atoms.sorted.lock();
        let state = self.atoms.state.read();
        let covered = sorted.len();
        if covered < state.atoms.len() {
            let mut new: Vec<Sym> = (covered as Sym..state.atoms.len() as Sym).collect();
            new.sort_unstable_by(|&a, &b| state.atoms.str(a).as_bytes().cmp(state.atoms.str(b).as_bytes()));
            let old = std::mem::take(&mut *sorted);
            sorted.reserve(old.len() + new.len());
            let (mut i, mut j) = (0, 0);
            while i < old.len() && j < new.len() {
                if state.atoms.str(old[i]).as_bytes() <= state.atoms.str(new[j]).as_bytes() {
                    sorted.push(old[i]);
                    i += 1;
                } else {
                    sorted.push(new[j]);
                    j += 1;
                }
            }
            sorted.extend_from_slice(&old[i..]);
            sorted.extend_from_slice(&new[j..]);
        }
        let key = |atom: &Sym| state.atoms.str(*atom).as_bytes();
        let start = lower.map_or(0, |lower| sorted.partition_point(|atom| key(atom) < lower));
        let end = upper.map_or(sorted.len(), |upper| sorted.partition_point(|atom| key(atom) <= upper));
        Ok(sorted[start..end.max(start)].to_vec())
    }
}
