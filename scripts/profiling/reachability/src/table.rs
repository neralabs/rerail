use crate::graph::{Shared, SharedGraph, failure, lookup};
use fallible_iterator::FallibleIterator;
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
    VTabCursor,
};
use rusqlite::{Connection, Result, ffi};
use std::borrow::Cow;
use std::ffi::{CStr, c_int};
use std::sync::Arc;

pub fn register(db: &Connection, registry: Shared) -> Result<()> {
    const MODULE: Module<ReachTable> = Module::eponymous_only_module();
    db.create_module(c"reachability", &MODULE, Some(registry))
}

#[repr(C)]
struct ReachTable {
    base: ffi::sqlite3_vtab,
    registry: Shared,
}

// SAFETY: repr(C) places SQLite's required base at offset zero. Cursors own their data.
unsafe impl<'a> VTab<'a> for ReachTable {
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
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(root INTEGER NOT NULL,node INTEGER NOT NULL,graph INTEGER HIDDEN NOT NULL,PRIMARY KEY(graph,root,node)) WITHOUT ROWID"),
            Self {
                base: ffi::sqlite3_vtab::default(),
                registry: aux.cloned().ok_or_else(|| failure("missing registry"))?,
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut chosen = [None; 3];
        for (i, constraint) in info.constraints().enumerate() {
            if constraint.is_usable()
                && constraint.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
                && let Ok(column @ 0..=2) = usize::try_from(constraint.column())
            {
                // Prefer a consumer's equality binding over the shared request
                // set. The other constraint remains SQLite's responsibility.
                if chosen[column].is_none()
                    || (column != 2
                        && info.is_in_constraint(chosen[column].unwrap())?
                        && !info.is_in_constraint(i)?)
                {
                    chosen[column] = Some(i);
                }
            }
        }
        // Never silently enumerate an unconstrained all-pairs closure.
        if chosen[2].is_none() || (chosen[0].is_none() && chosen[1].is_none()) {
            return Ok(false);
        }
        let mut flags = 0;
        let mut argument = 1;
        // Stable filter order: graph, root, node.
        for column in [2, 0, 1] {
            if let Some(i) = chosen[column] {
                flags |= 1 << column;
                if column != 2 && info.is_in_constraint(i)? {
                    info.set_in_constraint(i, true)?;
                    flags |= 1 << (column + 3);
                }
                let mut usage = info.constraint_usage(i);
                usage.set_argv_index(argument);
                usage.set_omit(true);
                argument += 1;
            }
        }
        // A bound target plus a batch of roots is an ancestor scan followed by
        // root membership. Do not traverse every root just to test one target.
        if flags & 1 == 0 || (flags & 8 != 0 && flags & 2 != 0) {
            flags |= 32;
        }
        let single_root = flags & 1 != 0 && flags & 8 == 0;
        let single_node = flags & 2 != 0 && flags & 16 == 0;
        let (rows, cost) = if single_root && single_node {
            (1, 10.0)
        } else if single_root || single_node {
            (100, 100.0)
        } else {
            (1_000_000, 1_000_000.0)
        };
        info.set_idx_num(flags);
        info.set_estimated_rows(rows);
        info.set_estimated_cost(cost);
        Ok(true)
    }

    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            registry: Arc::clone(&self.registry),
            graph: None,
            handle: 0,
            roots: vec![],
            root_index: 0,
            values: Arc::new(vec![]),
            position: 0,
            reverse: false,
            targets: None,
            row: None,
        })
    }
}

#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    registry: Shared,
    graph: Option<SharedGraph>,
    handle: i64,
    roots: Vec<i64>,
    root_index: usize,
    values: Arc<Vec<i64>>,
    position: usize,
    reverse: bool,
    targets: Option<Vec<i64>>,
    row: Option<(i64, i64)>,
}

fn arguments(args: &Filters<'_>, index: usize, batch: bool) -> Result<Vec<i64>> {
    let mut values = if batch {
        let mut input = args.in_values(index)?;
        let mut values = vec![];
        while let Some(value) = input.next()? {
            if value != rusqlite::types::ValueRef::Null {
                values.push(value.as_i64()?);
            }
        }
        values
    } else {
        args.get::<Option<i64>>(index)?.into_iter().collect()
    };
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

impl Cursor {
    fn advance(&mut self) -> Result<()> {
        self.row = None;
        let graph = self
            .graph
            .as_ref()
            .ok_or_else(|| failure("cursor has no graph"))?;
        let mut graph = graph.lock().map_err(|_| failure("graph poisoned"))?;
        loop {
            if self.position < self.values.len() {
                graph.charge()?;
                let node = self.values[self.position];
                self.position += 1;
                if self
                    .targets
                    .as_ref()
                    .is_some_and(|targets| targets.binary_search(&node).is_err())
                {
                    continue;
                }
                let root = self.roots[self.root_index - 1];
                self.row = Some(if self.reverse {
                    (node, root)
                } else {
                    (root, node)
                });
                graph.emitted += 1;
                return Ok(());
            }
            if self.root_index == self.roots.len() {
                return Ok(());
            }
            self.values = graph.reachable(self.roots[self.root_index], self.reverse)?;
            self.root_index += 1;
            self.position = 0;
        }
    }
}

// SAFETY: repr(C) places the cursor base first. No borrowed SQLite value escapes filter.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, flags: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.handle = args.get(0)?;
        self.graph = Some(lookup(&self.registry, self.handle)?);
        self.reverse = flags & 32 != 0;
        let root_values = if flags & 1 != 0 {
            Some(arguments(args, 1, flags & 8 != 0)?)
        } else {
            None
        };
        let node_values = if flags & 2 != 0 {
            Some(arguments(
                args,
                if root_values.is_some() { 2 } else { 1 },
                flags & 16 != 0,
            )?)
        } else {
            None
        };
        if flags & 24 == 24 {
            self.reverse =
                node_values.as_ref().unwrap().len() < root_values.as_ref().unwrap().len();
        }
        (self.roots, self.targets) = if self.reverse {
            (node_values.unwrap(), root_values)
        } else {
            (root_values.unwrap(), node_values)
        };
        self.root_index = 0;
        self.values = Arc::new(vec![]);
        self.position = 0;
        {
            let mut graph = self
                .graph
                .as_ref()
                .unwrap()
                .lock()
                .map_err(|_| failure("graph poisoned"))?;
            graph.filters += 1;
            if flags & 24 != 0 {
                graph.batch_filters += 1;
            }
            graph.requested_roots += self.roots.len() as u64;
        }
        self.advance()
    }
    fn next(&mut self) -> Result<()> {
        self.advance()
    }
    fn eof(&self) -> bool {
        self.row.is_none()
    }
    fn column(&self, ctx: &mut Context, column: c_int) -> Result<()> {
        let (root, node) = self.row.ok_or_else(|| failure("cursor exhausted"))?;
        ctx.set_result(&match column {
            0 => root,
            1 => node,
            _ => self.handle,
        })
    }
    fn rowid(&self) -> Result<i64> {
        Err(failure("reachability uses a composite key, not a rowid"))
    }
}
