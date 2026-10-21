//! Dynamic read-only virtual tables over immutable sources. SQLite applies filters.
use super::syntax::{Counters, Source};
fn failure(message: &str) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.to_owned().into())
}
use fallible_iterator::FallibleIterator;
use rusqlite::{
    Connection, Result,
    types::Value,
    vtab::{
        Context, CreateVTab, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig,
        VTabConnection, VTabCursor, VTabKind,
    },
};
use std::{
    borrow::Cow,
    ffi::{CStr, CString, c_int},
    ops::Range,
    sync::{Arc, atomic::Ordering},
};
#[derive(Clone)]
struct Shared {
    source: Arc<dyn Source>,
    stats: Arc<Counters>,
}
#[repr(C)]
struct Table {
    base: rusqlite::ffi::sqlite3_vtab,
    shared: Shared,
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub(super) fn register(
    db: &Connection,
    name: &str,
    source: Arc<dyn Source>,
) -> Result<Arc<Counters>> {
    if name.is_empty()
        || name.len() > 120
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(failure("invalid native table name"));
    }
    let stats = Arc::new(Counters::default());
    let module = CString::new(format!("native_source_{name}"))
        .map_err(|_| failure("invalid module name"))?;
    const MODULE: Module<Table> = Module::read_only_module();
    db.create_module(
        module.as_c_str(),
        &MODULE,
        Some(Shared {
            source,
            stats: Arc::clone(&stats),
        }),
    )?;
    db.execute_batch(&format!(
        "CREATE VIRTUAL TABLE {} USING {}",
        quote(name),
        quote(module.to_str().unwrap())
    ))?;
    Ok(stats)
}
// SAFETY: SQLite's base is first; auxiliary data and cursors own Arc references.
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
        let shared = aux
            .ok_or_else(|| failure("missing native row source"))?
            .clone();
        let sql = CString::new(format!(
            "CREATE TABLE x({})",
            shared
                .source
                .columns()
                .iter()
                .zip(shared.source.column_types())
                .map(|(c, t)| format!("{} {}", quote(c), t))
                .collect::<Vec<_>>()
                .join(",")
        ))
        .map_err(|_| failure("invalid native columns"))?;
        Ok((
            Cow::Owned(sql),
            Self {
                base: rusqlite::ffi::sqlite3_vtab::default(),
                shared,
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let _reader = super::producer::ReadOnly::enter();
        let selected = info
            .constraints()
            .enumerate()
            .filter_map(|(i, c)| {
                if c.is_usable()
                    && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
                    && c.column() >= 0
                    && self.shared.source.indexed(c.column() as usize)
                    && info.collation(i).is_ok_and(|v| v == "BINARY")
                {
                    let batch = info.is_in_constraint(i).ok()?;
                    Some((
                        i,
                        c.column(),
                        self.shared
                            .source
                            .estimated_matches(c.column() as usize, batch),
                    ))
                } else {
                    None
                }
            })
            .min_by_key(|v| v.2);
        let range = info.constraints().enumerate().find_map(|(i, c)| {
            let col = c.column();
            if !c.is_usable()
                || col < 0
                || !self.shared.source.range_indexed(col as usize)
                || !info.collation(i).is_ok_and(|v| v == "BINARY")
            {
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
                    && self
                        .shared
                        .source
                        .range_upper_compatible(col as usize, c.column() as usize)
                    && matches!(
                        c.operator(),
                        IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LT
                            | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LE
                    );
                if !c.is_usable()
                    || (c.column() != col && !upper_equivalent)
                    || !info.collation(i).is_ok_and(|v| v == "BINARY")
                {
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
            info.set_estimated_rows(self.shared.source.estimated_len() as i64);
            info.set_estimated_cost(self.shared.source.estimated_len().max(1) as f64);
        }
        Ok(true)
    }
    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor {
            base: rusqlite::ffi::sqlite3_vtab_cursor::default(),
            shared: self.shared.clone(),
            range: 0..0,
            selected: None,
            position: 0,
        })
    }
}
#[repr(C)]
struct Cursor {
    base: rusqlite::ffi::sqlite3_vtab_cursor,
    shared: Shared,
    range: Range<usize>,
    selected: Option<Vec<usize>>,
    position: usize,
}
impl Cursor {
    fn row(&self) -> usize {
        self.selected
            .as_ref()
            .map_or(self.range.start, |r| r[self.position])
    }
}
impl<'a> CreateVTab<'a> for Table {
    const KIND: VTabKind = VTabKind::Default;
}
// SAFETY: SQLite's cursor base is first. No borrowed SQLite value escapes filter.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, index: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let _read_only = super::producer::ReadOnly::enter();
        self.shared.stats.scans.fetch_add(1, Ordering::Relaxed);
        self.position = 0;
        self.selected = None;
        self.range = 0..self.shared.source.len();
        if index & (1 << 20) != 0 {
            let column = ((index & ((1 << 20) - 1)) >> 4) - 1;
            let mut arg = 0;
            let lower = if index & 1 != 0 {
                let v = args.get::<Value>(arg)?;
                arg += 1;
                Some((v, index & 4 != 0))
            } else {
                None
            };
            let upper = if index & 2 != 0 {
                Some((args.get::<Value>(arg)?, index & 8 != 0))
            } else {
                None
            };
            self.selected = self.shared.source.range_lookup(
                column as usize,
                lower.as_ref().map(|(v, b)| (v, *b)),
                upper.as_ref().map(|(v, b)| (v, *b)),
            )?;
        } else if index != 0 {
            let batch = index % 2 == 1;
            let column = (index / 2 - 1) as usize;
            let values = if batch {
                self.shared.stats.batches.fetch_add(1, Ordering::Relaxed);
                let mut iter = args.in_values(0)?;
                let mut values = Vec::new();
                while let Some(value) = iter.next()? {
                    values.push(Value::try_from(value).map_err(|e| failure(&e.to_string()))?);
                }
                values
            } else {
                vec![args.get::<Value>(0)?]
            };
            self.selected = self.shared.source.lookup(column, &values)?;
        }
        let count = self.selected.as_ref().map_or(self.range.len(), Vec::len);
        self.shared
            .stats
            .candidates
            .fetch_add(count as u64, Ordering::Relaxed);
        self.shared.source.charge(1)?;
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        let _read_only = super::producer::ReadOnly::enter();
        self.shared.source.charge(1)?;
        if self.selected.is_some() {
            self.position += 1;
        } else {
            self.range.start += 1;
        }
        Ok(())
    }
    fn eof(&self) -> bool {
        self.selected
            .as_ref()
            .map_or(self.range.is_empty(), |r| self.position >= r.len())
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        let _read_only = super::producer::ReadOnly::enter();
        self.shared.stats.columns.fetch_add(1, Ordering::Relaxed);
        self.shared.source.column(self.row(), col as usize, ctx)
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.row() as i64 + 1)
    }
}
