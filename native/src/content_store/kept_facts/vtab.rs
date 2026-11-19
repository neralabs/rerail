//! The kept facts through SQL (decision 0401 F5), read where they lie in each space's mapped blocks, as the atoms are
//! (`atoms::vtab`):
//!
//! ```sql
//! -- Each kept entry: its key, what a run checks of it, its live rows' count.
//! SELECT rel, part, digest, rows, types, reads FROM kept_entries WHERE space = ?1;
//! -- One entry's live rows, its columns c0, c1, ... (NULL past its width), with each row's id and level.
//! SELECT id, c0, c1 FROM kept_rows(?1, 'returns', ?2, 0);
//! ```
//!
//! A space's blocks are those its `facts_blocks` row names: as this process holds the row (`Snapshot::kept`), else
//! as the store's last commit has it. An `s` column holds the store's atom ids, as the `atoms` table answers them, so
//! the two join. No row is copied out of the mapping: a cursor reads its entry's columns where they lie.
use super::{KeptSpace, SpaceRow, space_row};
use rerail_blocks::kept::{FactKey, KeptRead as _};
use rusqlite::{
    Connection, Result, ffi,
    types::Value,
    vtab::{Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection, VTabCursor},
};
use std::borrow::Cow;
use std::ffi::{CStr, c_int};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The widest entry `kept_rows` reads: its columns `c0` to `c31`.
const COLUMNS: usize = 32;

/// Registers `kept_entries` and `kept_rows` on `connection`, the store at `path`'s.
pub(in crate::content_store) fn install(connection: &Connection, path: &Path) -> Result<()> {
    const ENTRIES: Module<Entries> = Module::eponymous_only_module();
    const ROWS: Module<RowsTable> = Module::eponymous_only_module();
    connection.create_module(c"kept_entries", &ENTRIES, Some(path.to_path_buf()))?;
    connection.create_module(c"kept_rows", &ROWS, Some(path.to_path_buf()))
}

fn error(e: impl std::fmt::Display) -> rusqlite::Error {
    rusqlite::Error::ModuleError(e.to_string())
}

/// The store's spaces and their rows: as this process holds them, else read from the store's last commit (a
/// connection of its own: a virtual table's callbacks have none to ask).
fn rows_of(path: &Path, space: Option<[u8; 32]>) -> Result<Vec<([u8; 32], SpaceRow)>> {
    let held = crate::content_store::frames::snapshot_kept(path, |kept| {
        kept.as_ref().map(|rows| match space {
            Some(s) => rows.get(&s).map(|r| vec![(s, r.clone())]).unwrap_or_default(),
            None => rows.iter().map(|(s, r)| (*s, r.clone())).collect(),
        })
    })
    .flatten();
    if let Some(rows) = held {
        return Ok(rows);
    }
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    match space {
        Some(s) => Ok(space_row(&c, &s).map_err(error)?.map(|r| vec![(s, r)]).unwrap_or_default()),
        None => {
            let spaces: Vec<Vec<u8>> = c.prepare("SELECT space FROM facts_blocks")?.query_map([], |r| r.get(0))?.collect::<Result<_>>()?;
            let mut out = Vec::new();
            for s in spaces.iter().filter_map(|s| <[u8; 32]>::try_from(s.as_slice()).ok()) {
                if let Some(r) = space_row(&c, &s).map_err(error)? {
                    out.push((s, r));
                }
            }
            Ok(out)
        }
    }
}

fn space_of(v: Value) -> Option<[u8; 32]> {
    match v {
        Value::Blob(b) => <[u8; 32]>::try_from(b.as_slice()).ok(),
        _ => None,
    }
}

/// The usable `=` constraints on `columns`, each with its place among the filter's arguments, in that order.
fn equalities(info: &mut IndexInfo, columns: &[c_int]) -> Vec<Option<usize>> {
    let found: Vec<Option<usize>> = columns
        .iter()
        .map(|col| info.constraints().enumerate().find(|(_, c)| c.is_usable() && c.column() == *col && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ).map(|(i, _)| i))
        .collect();
    let mut arg = 0;
    for i in found.iter().flatten() {
        arg += 1;
        info.constraint_usage(*i).set_argv_index(arg);
    }
    found
}

/// `kept_entries(space, rel, ident, part, digest, rows, types, reads)`: every entry of every space (or of `space`).
#[repr(C)]
struct Entries {
    base: ffi::sqlite3_vtab,
    path: PathBuf,
}

// SAFETY: SQLite's base is first; the table owns its path.
unsafe impl<'a> VTab<'a> for Entries {
    type Aux = PathBuf;
    type Cursor = EntriesCursor;
    fn connect(db: &mut VTabConnection, aux: Option<&PathBuf>, _: &[u8], _: &[u8], _: &[u8], _: &[&[u8]]) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        let path = aux.expect("the store's path").clone();
        Ok((Cow::Borrowed(c"CREATE TABLE x(space BLOB, rel TEXT, ident BLOB, part INTEGER, digest INTEGER, rows INTEGER, types TEXT, reads TEXT)"), Entries { base: ffi::sqlite3_vtab::default(), path }))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let by_space = equalities(info, &[0])[0].is_some();
        info.set_idx_num(c_int::from(by_space));
        info.set_estimated_cost(if by_space { 100.0 } else { 10_000.0 });
        Ok(true)
    }

    fn open(&'a mut self) -> Result<EntriesCursor> {
        Ok(EntriesCursor { base: ffi::sqlite3_vtab_cursor::default(), path: self.path.clone(), rows: Vec::new(), at: 0 })
    }
}

/// An entry's columns, read from its block when the cursor reached it.
struct EntryRow {
    space: [u8; 32],
    rel: String,
    ident: [u8; 32],
    part: i64,
    digest: u64,
    rows: usize,
    types: String,
    reads: String,
}

#[repr(C)]
struct EntriesCursor {
    base: ffi::sqlite3_vtab_cursor,
    path: PathBuf,
    rows: Vec<EntryRow>,
    at: usize,
}

/// Whether `kept`'s row `id` is live: dead neither in its entry (a maintained relation's kills) nor in its relation.
fn is_live(kept: &rerail_blocks::kept::Kept<'_>, id: u32) -> bool {
    kept.view().live(id) && kept.dead.get(id as usize / 64).is_none_or(|w| w & (1 << (id % 64)) == 0)
}

/// `kept`'s live rows.
fn live<'k>(kept: &'k rerail_blocks::kept::Kept<'_>) -> impl Iterator<Item = u32> + 'k {
    (0..kept.view().total()).filter(move |&id| is_live(kept, id))
}

/// `kept`'s first live row at or past `id`, else its rows' end.
fn live_from(kept: &rerail_blocks::kept::Kept<'_>, mut id: u32) -> u32 {
    let total = kept.view().total();
    while id < total && !is_live(kept, id) {
        id += 1;
    }
    id
}

// SAFETY: SQLite's base is first; the cursor owns what it read.
unsafe impl VTabCursor for EntriesCursor {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.at = 0;
        self.rows.clear();
        let space = if idx_num == 1 {
            match space_of(args.get::<Value>(0)?) {
                Some(s) => Some(s),
                None => return Ok(()),
            }
        } else {
            None
        };
        for (s, row) in rows_of(&self.path, space)? {
            let blocks = KeptSpace::open(&self.path, &s, &row).map_err(error)?;
            for k in blocks.view().all() {
                let reads: Vec<String> = k.reads().map(|(r, d)| format!("{r}:{d:016x}")).collect();
                self.rows.push(EntryRow {
                    space: s,
                    rel: k.key.rel.to_owned(),
                    ident: *k.key.ident,
                    part: k.key.part,
                    digest: k.digest,
                    rows: live(&k).count(),
                    types: String::from_utf8_lossy(k.types).into_owned(),
                    reads: reads.join(" "),
                });
            }
        }
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.at += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.at >= self.rows.len()
    }

    fn column(&self, ctx: &mut Context, column: c_int) -> Result<()> {
        let r = &self.rows[self.at];
        match column {
            0 => ctx.set_result(&r.space.as_slice()),
            1 => ctx.set_result(&r.rel),
            2 => ctx.set_result(&r.ident.as_slice()),
            3 => ctx.set_result(&r.part),
            4 => ctx.set_result(&(r.digest as i64)),
            5 => ctx.set_result(&(r.rows as i64)),
            6 => ctx.set_result(&r.types),
            _ => ctx.set_result(&r.reads),
        }
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.at as i64)
    }
}

/// `kept_rows(space, rel, ident, part)`: that entry's live rows, `id`, `level`, `c0` ... `c31`.
#[repr(C)]
struct RowsTable {
    base: ffi::sqlite3_vtab,
    path: PathBuf,
}

/// The hidden arguments' columns: past `id`, `level` and the values.
const SPACE: c_int = 2 + COLUMNS as c_int;

// SAFETY: SQLite's base is first; the table owns its path.
unsafe impl<'a> VTab<'a> for RowsTable {
    type Aux = PathBuf;
    type Cursor = RowsCursor;
    fn connect(db: &mut VTabConnection, aux: Option<&PathBuf>, _: &[u8], _: &[u8], _: &[u8], _: &[&[u8]]) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        let path = aux.expect("the store's path").clone();
        let values: Vec<String> = (0..COLUMNS).map(|c| format!("c{c} INTEGER")).collect();
        let schema = format!("CREATE TABLE x(id INTEGER, level INTEGER, {}, space BLOB HIDDEN, rel TEXT HIDDEN, ident BLOB HIDDEN, part INTEGER HIDDEN)", values.join(", "));
        let schema = std::ffi::CString::new(schema).expect("no NUL in the schema");
        Ok((Cow::Owned(schema), RowsTable { base: ffi::sqlite3_vtab::default(), path }))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        // Every argument is needed: an entry is named by its whole key.
        let found = equalities(info, &[SPACE, SPACE + 1, SPACE + 2, SPACE + 3]);
        if found.iter().any(Option::is_none) {
            return Err(rusqlite::Error::ModuleError("kept_rows(space, rel, ident, part): every argument is needed".into()));
        }
        for i in found.into_iter().flatten() {
            info.constraint_usage(i).set_omit(true);
        }
        info.set_estimated_cost(1_000.0);
        Ok(true)
    }

    fn open(&'a mut self) -> Result<RowsCursor> {
        Ok(RowsCursor { base: ffi::sqlite3_vtab_cursor::default(), path: self.path.clone(), at: None, id: 0, end: 0, key: None })
    }
}

/// An entry the cursor reads: its space's blocks, mapped and held while it reads, and its key in them.
struct At {
    space: KeptSpace,
    rel: String,
    ident: [u8; 32],
    part: i64,
}

#[repr(C)]
struct RowsCursor {
    base: ffi::sqlite3_vtab_cursor,
    path: PathBuf,
    at: Option<At>,
    /// The live row it is at, in id order, and its entry's rows' end: its values are read from the mapping.
    id: u32,
    end: u32,
    key: Option<([u8; 32], String, [u8; 32], i64)>,
}

impl RowsCursor {
    fn with<T>(&self, f: impl FnOnce(&rerail_blocks::kept::Kept<'_>) -> T) -> Option<T> {
        let at = self.at.as_ref()?;
        let view = at.space.view();
        let kept = view.get(&FactKey { rel: &at.rel, ident: &at.ident, part: at.part })?;
        Some(f(&kept))
    }
}

// SAFETY: SQLite's base is first; the cursor owns the mapping it reads.
unsafe impl VTabCursor for RowsCursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        (self.at, self.id, self.end) = (None, 0, 0);
        let (Some(space), Value::Text(rel), Some(ident), Value::Integer(part)) = (space_of(args.get(0)?), args.get::<Value>(1)?, space_of(args.get(2)?), args.get::<Value>(3)?) else {
            return Ok(());
        };
        self.key = Some((space, rel.clone(), ident, part));
        let Some((_, row)) = rows_of(&self.path, Some(space))?.pop() else { return Ok(()) };
        self.at = Some(At { space: KeptSpace::open(&self.path, &space, &row).map_err(error)?, rel, ident, part });
        let at = self.with(|k| {
            if k.rows.arity as usize > COLUMNS {
                return Err(rusqlite::Error::ModuleError(format!("kept_rows: {} has {} columns, past the table's {COLUMNS}", k.key.rel, k.rows.arity)));
            }
            Ok((live_from(k, 0), k.view().total()))
        });
        (self.id, self.end) = at.transpose()?.unwrap_or((0, 0));
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        let id = self.id + 1;
        self.id = self.with(|k| live_from(k, id)).unwrap_or(self.end);
        Ok(())
    }

    fn eof(&self) -> bool {
        self.id >= self.end
    }

    fn column(&self, ctx: &mut Context, column: c_int) -> Result<()> {
        let id = self.id;
        let c = column as usize;
        if column >= SPACE {
            let (space, rel, ident, part) = self.key.as_ref().expect("a filtered cursor's key");
            return match column - SPACE {
                0 => ctx.set_result(&space.as_slice()),
                1 => ctx.set_result(rel),
                2 => ctx.set_result(&ident.as_slice()),
                _ => ctx.set_result(part),
            };
        }
        let value = self
            .with(|k| match c {
                0 => Some(i64::from(id)),
                1 => k.levels.get(id as usize).map(|l| i64::from(*l)),
                _ => (c - 2 < k.rows.arity as usize).then(|| k.view().get(id, c - 2)),
            })
            .flatten();
        ctx.set_result(&value)
    }

    fn rowid(&self) -> Result<i64> {
        Ok(i64::from(self.id))
    }
}
