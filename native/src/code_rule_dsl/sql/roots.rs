//! `roots(run, file, node)`: a batch's selected roots, read in place. A clause run's files whose
//! roots an anchor index selected hold them as immutable slices already (`ClauseRun::files`); the
//! driver installs those slices for the batch (`BatchSlot::roots`) and the root frames read them
//! here, where every root was once inserted into a B-tree and deleted again.

use super::exec::BatchSlot;
use rusqlite::types::ValueRef;
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, VTab, VTabConfig, VTabConnection, VTabCursor,
    sqlite3_vtab, sqlite3_vtab_cursor,
};
use rusqlite::{Error, Result};
use std::borrow::Cow;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::sync::Arc;

/// A batch's selected roots: per (run id, file), in that order, the file's roots in preorder.
#[derive(Default)]
pub(super) struct Roots {
    files: Vec<RootFile>,
}

struct RootFile {
    run: i64,
    file: i64,
    nodes: Arc<[u32]>,
}

impl Roots {
    /// The roots of `files`: (run id, file, its selected roots) each, a (run id, file) once.
    pub(super) fn of(files: impl IntoIterator<Item = (i64, usize, Arc<[u32]>)>) -> Self {
        let mut files: Vec<RootFile> = files
            .into_iter()
            .filter(|(_, _, nodes)| !nodes.is_empty())
            .map(|(run, file, nodes)| {
                debug_assert!(
                    nodes.windows(2).all(|pair| pair[0] < pair[1]),
                    "selected roots are in preorder"
                );
                RootFile {
                    run,
                    file: file as i64,
                    nodes,
                }
            })
            .collect();
        files.sort_by_key(|file| (file.run, file.file));
        debug_assert!(
            files
                .windows(2)
                .all(|pair| (pair[0].run, pair[0].file) != (pair[1].run, pair[1].file)),
            "a run reads a file once"
        );
        Self { files }
    }
}

const SCHEMA: &CStr = c"CREATE TABLE x(run INTEGER, file INTEGER, node INTEGER)";
/// `idx_num`: the run is constrained to one value.
const RUN: c_int = 1;

#[repr(C)]
pub(super) struct RootTable {
    base: sqlite3_vtab,
    slot: Arc<BatchSlot>,
}

// SAFETY: `sqlite3_vtab` is the first field, as SQLite requires.
unsafe impl<'vtab> VTab<'vtab> for RootTable {
    type Aux = Arc<BatchSlot>;
    type Cursor = RootCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<BatchSlot>>,
        _module: &[u8],
        _database: &[u8],
        _table: &[u8],
        _args: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let slot =
            Arc::clone(aux.ok_or_else(|| Error::ModuleError("roots: no batch slot".into()))?);
        Ok((
            Cow::Borrowed(SCHEMA),
            Self {
                base: sqlite3_vtab::default(),
                slot,
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let run = info.constraints().position(|constraint| {
            constraint.is_usable()
                && constraint.column() == 0
                && constraint.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
        });
        match run {
            Some(index) => {
                // SQLite rechecks the equality: a probe of another type scans every run.
                info.constraint_usage(index).set_argv_index(1);
                info.set_idx_num(RUN);
                info.set_estimated_cost(64.0);
                info.set_estimated_rows(64);
            }
            None => {
                info.set_idx_num(0);
                info.set_estimated_cost(1e12);
                info.set_estimated_rows(i64::MAX);
            }
        }
        Ok(true)
    }

    fn open(&'vtab mut self) -> Result<RootCursor<'vtab>> {
        Ok(RootCursor {
            base: sqlite3_vtab_cursor::default(),
            roots: Arc::default(),
            file: 0,
            end: 0,
            node: 0,
            slot: Arc::clone(&self.slot),
            phantom: PhantomData,
        })
    }
}

#[repr(C)]
pub(super) struct RootCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    /// The batch's roots, held while the cursor reads them.
    roots: Arc<Roots>,
    /// The file entry at, and the end of the run's entries.
    file: usize,
    end: usize,
    /// The root at, within the file entry's.
    node: usize,
    slot: Arc<BatchSlot>,
    phantom: PhantomData<&'vtab RootTable>,
}

// SAFETY: `sqlite3_vtab_cursor` is the first field, as SQLite requires.
unsafe impl VTabCursor for RootCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.roots = self.slot.roots();
        let files = &self.roots.files;
        (self.file, self.end, self.node) = (0, files.len(), 0);
        if idx_num & RUN != 0 {
            let run = match args.iter().next() {
                Some(ValueRef::Integer(run)) => Some(run),
                Some(ValueRef::Real(run)) if run.fract() == 0.0 => Some(run as i64),
                Some(ValueRef::Null) => {
                    self.end = 0;
                    None
                }
                // Text or a blob: every run, which SQLite's recheck narrows.
                _ => None,
            };
            if let Some(run) = run {
                self.file = files.partition_point(|file| file.run < run);
                self.end = files.partition_point(|file| file.run <= run);
            }
        }
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.node += 1;
        if self.node == self.roots.files[self.file].nodes.len() {
            self.file += 1;
            self.node = 0;
        }
        Ok(())
    }

    fn eof(&self) -> bool {
        self.file >= self.end
    }

    fn column(&self, context: &mut Context, column: c_int) -> Result<()> {
        let file = &self.roots.files[self.file];
        context.set_result(&match column {
            0 => file.run,
            1 => file.file,
            _ => i64::from(file.nodes[self.node]),
        })
    }

    fn rowid(&self) -> Result<i64> {
        // Unique within a scan: the file entry's position and the root's.
        Ok(((self.file as i64) << 32) | self.node as i64)
    }
}
