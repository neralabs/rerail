//! Tree neighbourhoods read from the tape, as in the spike's `nera_kids`: children by sibling
//! skips (`pre + size`) and ancestors by parent deltas. Only the named node's neighbours are read,
//! so a relation's inner scope never needs every node's decoded parent.
//!
//! The tape is read through `Structure`: P's `file` table (`reader::Reader`, parent deltas), or
//! the content store's v9np tapes (`tapes::StoreTapes`), which have no parent column, so their
//! ancestors are found by descent from the root (decision 0026). Each row carries the node's
//! subtree size, so a caller tests descent by interval without a second read (the spike: carry
//! kind and size out of the walk).
use super::reader::{Column, Reader};
use std::sync::Arc;
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

/// `axis` values: a node's children in order, its ancestors nearest first, its parent alone.
pub(super) const CHILDREN: i64 = 0;
pub(super) const ANCESTORS: i64 = 1;
pub(super) const PARENT: i64 = 2;

/// A tape's structure: each node's subtree size, by preorder.
pub(super) trait Structure: Send {
    /// The subtree size of node `pre` of tape `fid`; an error past the tape's end.
    fn size(&mut self, fid: i64, pre: usize) -> Result<usize>;
    /// `node`'s ancestors, nearest first: by descent from the root, each step skipping siblings.
    fn ancestors(&mut self, fid: i64, node: usize, out: &mut Vec<i64>) -> Result<()> {
        descend(self, fid, node, out)
    }
}

/// `node`'s ancestors, nearest first, by descent from the root through `structure`'s sizes.
pub(super) fn descend<S: Structure + ?Sized>(structure: &mut S, fid: i64, node: usize, out: &mut Vec<i64>) -> Result<()> {
    {
        let end = structure.size(fid, 0)?;
        if node >= end {
            return Err(invalid("tree node outside its tape"));
        }
        let mut at = 0;
        while at != node {
            out.push(at as i64);
            let mut child = at + 1;
            loop {
                if crate::code_rule_dsl::budget::step_budget_exhausted() {
                    return Err(rusqlite::Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_INTERRUPT), None));
                }
                let size = structure.size(fid, child)?;
                if size == 0 {
                    return Err(invalid("empty indexed subtree"));
                }
                if node < child + size {
                    break;
                }
                child += size;
            }
            at = child;
        }
        out.reverse();
        Ok(())
    }
}

/// The structure `nera_tree` reads, shared with its owner.
pub(super) type Shared = Arc<parking_lot::Mutex<dyn Structure>>;

impl Structure for Reader {
    fn size(&mut self, fid: i64, pre: usize) -> Result<usize> {
        let rec = self.rec(fid, pre)?;
        Ok(self.value(fid, Column::Size, pre, rec[2])? as usize)
    }
    fn ancestors(&mut self, fid: i64, node: usize, out: &mut Vec<i64>) -> Result<()> {
        let mut at = node;
        let mut rec = self.rec(fid, node)?;
        loop {
            let delta = self.value(fid, Column::Parent, at, rec[1])? as usize;
            // The root's delta points one past it.
            if delta > at {
                break;
            }
            if delta == 0 {
                return Err(invalid("node is its own parent"));
            }
            at -= delta;
            out.push(at as i64);
            rec = self.rec(fid, at)?;
        }
        Ok(())
    }
}

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
    sizes: Vec<i64>,
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
                c"CREATE TABLE x(pre INTEGER,size INTEGER,fid INTEGER HIDDEN,node INTEGER HIDDEN,axis INTEGER HIDDEN)",
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
                && (2..=4).contains(&constraint.column())
            {
                let slot = (constraint.column() - 2) as usize;
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
            sizes: Vec::new(),
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
        self.sizes.clear();
        self.at = 0;
        let fid = self.fid;
        let node = usize::try_from(self.node).map_err(|_| invalid("negative tree node"))?;
        let mut reader = self.reader.lock();
        match self.axis {
            CHILDREN => {
                let end = node + reader.size(fid, node)?;
                let mut child = node + 1;
                while child < end {
                    if crate::code_rule_dsl::budget::step_budget_exhausted() {
                        return Err(rusqlite::Error::SqliteFailure(
                            ffi::Error::new(ffi::SQLITE_INTERRUPT),
                            None,
                        ));
                    }
                    let size = reader.size(fid, child)?;
                    if size == 0 {
                        return Err(invalid("empty indexed subtree"));
                    }
                    self.nodes.push(child as i64);
                    self.sizes.push(size as i64);
                    child += size;
                }
            }
            ANCESTORS | PARENT => {
                reader.size(fid, node)?;
                reader.ancestors(fid, node, &mut self.nodes)?;
                if self.axis == PARENT {
                    self.nodes.truncate(1);
                }
                for &at in &self.nodes {
                    let size = reader.size(fid, at as usize)?;
                    self.sizes.push(size as i64);
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
            1 => ctx.set_result(&self.sizes[self.at]),
            2 => ctx.set_result(&self.fid),
            3 => ctx.set_result(&self.node),
            _ => ctx.set_result(&self.axis),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.nodes[self.at])
    }
}

#[cfg(test)]
mod tests;
