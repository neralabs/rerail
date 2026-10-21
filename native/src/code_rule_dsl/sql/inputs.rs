//! Selected roots are already immutable Rust slices. Expose them directly to SQLite rather
//! than copying every (run, file, node) membership through a separate INSERT into a B-tree.

mod files;
use super::exec::ClauseRun;
use parking_lot::RwLock;
use rusqlite::types::ValueRef;
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
    VTabCursor, sqlite3_vtab, sqlite3_vtab_cursor,
};
use rusqlite::{Connection, Error, Result};
use std::borrow::Cow;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct RootInputs {
    current: RwLock<Arc<RootSet>>,
    runs: Arc<files::RunInputs>,
}

#[derive(Default)]
struct RootSet {
    files: Vec<RootFile>,
}

struct RootFile {
    run: i64,
    file: i64,
    nodes: Arc<[u32]>,
    base: i64,
}

fn ordered(nodes: &Arc<[u32]>) -> Arc<[u32]> {
    if nodes.windows(2).all(|pair| pair[0] < pair[1]) {
        Arc::clone(nodes)
    } else {
        let mut owned = nodes.to_vec();
        owned.sort_unstable();
        owned.dedup();
        owned.into()
    }
}

impl RootInputs {
    pub(super) fn register(self: &Arc<Self>, connection: &Connection) -> Result<()> {
        const MODULE: Module<'static, RootTable> = Module::eponymous_only_module();
        connection.create_module(c"roots", &MODULE, Some(Arc::clone(self)))?;
        self.runs.register(connection)
    }

    pub(super) fn install(&self, runs: &[ClauseRun]) -> usize {
        let mut files = Vec::new();
        for run in runs {
            for (file, nodes) in &run.files {
                if let Some(nodes) = nodes.as_ref().filter(|nodes| !nodes.is_empty()) {
                    files.push(RootFile {
                        run: run.statement as i64,
                        file: *file as i64,
                        nodes: ordered(nodes),
                        base: 0,
                    });
                }
            }
        }
        files.sort_unstable_by_key(|file| (file.run, file.file));
        let mut unique: Vec<RootFile> = Vec::with_capacity(files.len());
        for file in files {
            if let Some(previous) = unique
                .last_mut()
                .filter(|previous| (previous.run, previous.file) == (file.run, file.file))
            {
                // Preserve INSERT OR IGNORE semantics for repeated selected-file entries too.
                let mut nodes = previous.nodes.to_vec();
                nodes.extend_from_slice(&file.nodes);
                nodes.sort_unstable();
                nodes.dedup();
                previous.nodes = nodes.into();
            } else {
                unique.push(file);
            }
        }
        let mut rows = 0;
        for file in &mut unique {
            file.base = rows as i64;
            rows += file.nodes.len();
        }
        *self.current.write() = Arc::new(RootSet { files: unique });
        rows
    }

    pub(super) fn install_runs(&self, rows: Vec<(i64, i64)>) {
        self.runs.install(rows);
    }

    pub(super) fn clear(&self) {
        self.runs.install(Vec::new());
        *self.current.write() = Arc::default();
    }
}

#[repr(C)]
struct RootTable {
    base: sqlite3_vtab,
    inputs: Arc<RootInputs>,
}

// SAFETY: the SQLite base is first and every cursor owns an immutable input snapshot.
unsafe impl<'vtab> VTab<'vtab> for RootTable {
    type Aux = Arc<RootInputs>;
    type Cursor = RootCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Self::Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let inputs =
            Arc::clone(aux.ok_or_else(|| Error::ModuleError("roots: no input slot".into()))?);
        Ok((Cow::Borrowed(c"CREATE TABLE x(run INTEGER NOT NULL, file INTEGER NOT NULL, node INTEGER NOT NULL)"),
            Self { base:sqlite3_vtab::default(), inputs }))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut equality = [None; 3];
        for (index, constraint) in info.constraints().enumerate() {
            if constraint.is_usable()
                && constraint.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
                && (0..3).contains(&constraint.column())
            {
                equality[constraint.column() as usize].get_or_insert(index);
            }
        }
        let mut mask = 0;
        let mut argument = 1;
        for (column, index) in equality.into_iter().enumerate() {
            if let Some(index) = index {
                info.constraint_usage(index).set_argv_index(argument);
                // SQLite rechecks affinity, collation and equality. Text probes can scan safely.
                mask |= 1 << column;
                argument += 1;
            }
        }
        info.set_idx_num(mask);
        let cost = match mask {
            // Text probes can require an unconstrained scan before SQLite rechecks affinity,
            // so do not advertise a one-row scan even when all three columns are constrained.
            7 => 1.0,
            3 | 5 => 32.0,
            1 => 256.0,
            _ => 1_000_000.0,
        };
        info.set_estimated_cost(cost);
        info.set_estimated_rows(cost as i64);
        let order: Vec<_> = info.order_bys().collect();
        if order.len() <= 3
            && order.iter().enumerate().all(|(column, order)| {
                !order.is_order_by_desc() && order.column() == column as c_int
            })
        {
            info.set_order_by_consumed(true);
        }
        Ok(true)
    }

    fn open(&'vtab mut self) -> Result<Self::Cursor> {
        Ok(RootCursor {
            base: sqlite3_vtab_cursor::default(),
            inputs: Arc::clone(&self.inputs),
            data: Arc::default(),
            file: 0,
            end: 0,
            node: 0,
            node_end: 0,
            file_eq: None,
            node_eq: None,
            phantom: PhantomData,
        })
    }
}

#[repr(C)]
struct RootCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    inputs: Arc<RootInputs>,
    data: Arc<RootSet>,
    file: usize,
    end: usize,
    node: usize,
    node_end: usize,
    file_eq: Option<i64>,
    node_eq: Option<i64>,
    phantom: PhantomData<&'vtab RootTable>,
}

impl RootCursor<'_> {
    fn seek_file(&mut self) {
        while self.file < self.end {
            let file = &self.data.files[self.file];
            if self.file_eq.is_none_or(|key| key == file.file) {
                let (start, end) = match self.node_eq {
                    None => (0, file.nodes.len()),
                    Some(key) => {
                        let start = file.nodes.partition_point(|&node| i64::from(node) < key);
                        (
                            start,
                            start
                                + usize::from(
                                    file.nodes
                                        .get(start)
                                        .is_some_and(|&node| i64::from(node) == key),
                                ),
                        )
                    }
                };
                if start < end {
                    self.node = start;
                    self.node_end = end;
                    return;
                }
            }
            self.file += 1;
        }
    }
}

// SAFETY: the SQLite cursor base is first; columns refer to a retained immutable snapshot.
unsafe impl VTabCursor for RootCursor<'_> {
    fn filter(&mut self, mask: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.data = Arc::clone(&self.inputs.current.read());
        self.file = 0;
        self.end = self.data.files.len();
        self.file_eq = None;
        self.node_eq = None;
        let mut constraints = [None; 3];
        let mut arguments = args.iter();
        for (column, constraint) in constraints.iter_mut().enumerate() {
            if mask & (1 << column) == 0 {
                continue;
            }
            let value = arguments
                .next()
                .ok_or_else(|| Error::ModuleError("roots: missing equality probe".into()))?;
            match value {
                ValueRef::Null => self.end = 0,
                ValueRef::Integer(value) => *constraint = Some(value),
                ValueRef::Real(value) if value.fract() == 0.0 => *constraint = Some(value as i64),
                // Numeric text follows the declared INTEGER affinity. Leave it to SQLite rather
                // than narrowing incorrectly. Other unsupported probes are also safely rechecked.
                _ => {}
            }
        }
        if self.end == 0 {
            return Ok(());
        }
        if let Some(run) = constraints[0] {
            self.file = self.data.files.partition_point(|file| file.run < run);
            self.end = self.data.files.partition_point(|file| file.run <= run);
            if let Some(file) = constraints[1] {
                let files = &self.data.files[self.file..self.end];
                let start = files.partition_point(|entry| entry.file < file);
                let end = files.partition_point(|entry| entry.file <= file);
                self.end = self.file + end;
                self.file += start;
            }
        }
        self.file_eq = constraints[1];
        self.node_eq = constraints[2];
        self.seek_file();
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.node += 1;
        if self.node == self.node_end {
            self.file += 1;
            self.seek_file();
        }
        Ok(())
    }

    fn eof(&self) -> bool {
        self.file >= self.end
    }

    fn column(&self, context: &mut Context, column: c_int) -> Result<()> {
        let file = &self.data.files[self.file];
        context.set_result(&match column {
            0 => file.run,
            1 => file.file,
            _ => i64::from(file.nodes[self.node]),
        })
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.data.files[self.file].base + self.node as i64 + 1)
    }
}

#[cfg(test)]
mod tests;
