//! Tree neighbourhoods read from the tape, as in the spike's `nera_kids`: children by sibling
//! skips (`pre + size`) and ancestors by parent deltas. Only the named node's neighbours are read,
//! so a relation's inner scope never needs every node's decoded parent.
use super::reader::{Column, Shared};
use rusqlite::{
    Connection, Result, ffi,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor,
    },
};
use std::{
    borrow::Cow,
    ffi::{CStr, c_int},
};

/// `axis` values.
pub(super) const CHILDREN: i64 = 0;
pub(super) const ANCESTORS: i64 = 1;

#[repr(C)]
struct Table {
    base: ffi::sqlite3_vtab,
    reader: Shared,
}
#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    reader: Shared,
    nodes: Vec<i64>,
    at: usize,
    fid: i64,
    node: i64,
    axis: i64,
}
pub(super) fn install(db: &Connection, reader: Shared) -> Result<()> {
    const MODULE: Module<Table> = Module::eponymous_only_module();
    db.create_module(c"nera_tree", &MODULE, Some(reader))
}
fn invalid(message: &str) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.into())
}
// SAFETY: SQLite's base is first. The reader belongs to the owning connection.
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
        Ok((
            Cow::Borrowed(
                c"CREATE TABLE x(pre INTEGER,fid INTEGER HIDDEN,node INTEGER HIDDEN,axis INTEGER HIDDEN)",
            ),
            Self {
                base: ffi::sqlite3_vtab::default(),
                reader: aux.expect("reader supplied").clone(),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut found = [false; 3];
        for (constraint, mut usage) in info.constraints_and_usages() {
            if constraint.is_usable()
                && constraint.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
                && (1..=3).contains(&constraint.column())
            {
                let slot = (constraint.column() - 1) as usize;
                if !found[slot] {
                    found[slot] = true;
                    usage.set_argv_index(slot as c_int + 1);
                    usage.set_omit(true);
                }
            }
        }
        info.set_estimated_cost(10.0);
        info.set_estimated_rows(8);
        Ok(found.into_iter().all(|v| v))
    }
    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            reader: self.reader.clone(),
            nodes: Vec::new(),
            at: 0,
            fid: 0,
            node: 0,
            axis: 0,
        })
    }
}
impl Cursor {
    fn walk(&mut self) -> Result<()> {
        self.nodes.clear();
        self.at = 0;
        let fid = self.fid;
        let node = usize::try_from(self.node).map_err(|_| invalid("negative tree node"))?;
        let mut reader = self.reader.lock();
        let rec = reader.rec(fid, node)?;
        match self.axis {
            CHILDREN => {
                let end = node + reader.value(fid, Column::Size, node, rec[2])? as usize;
                let mut child = node + 1;
                while child < end {
                    #[cfg(feature = "code-rule-profile")]
                    {
                        reader.counters.child_steps += 1;
                    }
                    if crate::code_rule_dsl::budget::step_budget_exhausted() {
                        return Err(rusqlite::Error::SqliteFailure(
                            ffi::Error::new(ffi::SQLITE_INTERRUPT),
                            None,
                        ));
                    }
                    self.nodes.push(child as i64);
                    let rec = reader.rec(fid, child)?;
                    let size = reader.value(fid, Column::Size, child, rec[2])? as usize;
                    if size == 0 {
                        return Err(invalid("empty indexed subtree"));
                    }
                    child += size;
                }
            }
            ANCESTORS => {
                let mut at = node;
                let mut rec = rec;
                loop {
                    let delta = reader.value(fid, Column::Parent, at, rec[1])? as usize;
                    // The root's delta points one past it.
                    if delta > at {
                        break;
                    }
                    if delta == 0 {
                        return Err(invalid("node is its own parent"));
                    }
                    at -= delta;
                    self.nodes.push(at as i64);
                    rec = reader.rec(fid, at)?;
                }
            }
            _ => return Err(invalid("unknown tree axis")),
        }
        Ok(())
    }
}
// SAFETY: the cursor owns its coordinates and only reads through the shared connection guard.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.fid = args.get(0)?;
        self.node = args.get(1)?;
        self.axis = args.get(2)?;
        self.walk()
    }
    fn next(&mut self) -> Result<()> {
        self.at += 1;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.at >= self.nodes.len()
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        match col {
            0 => ctx.set_result(&self.nodes[self.at]),
            1 => ctx.set_result(&self.fid),
            2 => ctx.set_result(&self.node),
            _ => ctx.set_result(&self.axis),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.nodes[self.at])
    }
}
