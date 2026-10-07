//! `rerail_files` and `rerail_nodes`: a batch's file versions, and the syntax tree of each, as
//! tables SQL joins with terminal rows.
//!
//! `rerail_nodes` follows the tree from one node along an axis. Its rows come in the order the
//! matcher walks that axis (`pos`), so `ORDER BY pos LIMIT 1` is ast-grep's first related node.

use super::docs::{Derived, Document, batch_state, derived_ordinal, document};
use crate::policy_dsl::transducer::sql_backend::{BatchFile, BatchView, MatchRegistry};
use ast_grep_core::{Doc, Node};
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::ffi;
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
    VTabCursor,
};
use rusqlite::{Connection, Result as SqlResult};
use std::borrow::Cow;
#[cfg(test)]
use std::cell::Cell;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::sync::Arc;

#[cfg(test)]
thread_local! {
    static AXIS_WALK_NODES: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(super) fn reset_axis_walk_nodes() {
    AXIS_WALK_NODES.set(0);
}

#[cfg(test)]
pub(super) fn axis_walk_nodes() -> usize {
    AXIS_WALK_NODES.get()
}

#[cfg(test)]
fn count_axis_walk_nodes(nodes: usize) {
    AXIS_WALK_NODES.set(AXIS_WALK_NODES.get() + nodes);
}

pub(super) fn register(connection: &Connection, registry: &Arc<MatchRegistry>) -> SqlResult<()> {
    const FILES: Module<FilesTable> = Module::eponymous_only_module();
    const NODES: Module<NodesTable> = Module::eponymous_only_module();
    connection.create_module(c"rerail_files", &FILES, Some(Arc::clone(registry)))?;
    connection.create_module(c"rerail_nodes", &NODES, Some(Arc::clone(registry)))
}

fn missing_batch(table: &str) -> rusqlite::Error {
    rusqlite::Error::ModuleError(format!("{table} batch is missing"))
}

fn module_error(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::ModuleError(message.into())
}

// ---------------------------------------------------------------------------------------------
// rerail_files

#[repr(C)]
struct FilesTable {
    base: ffi::sqlite3_vtab,
    registry: Arc<MatchRegistry>,
}

unsafe impl<'vtab> VTab<'vtab> for FilesTable {
    type Aux = Arc<MatchRegistry>;
    type Cursor = FilesCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<MatchRegistry>>,
        _module_name: &[u8],
        _database_name: &[u8],
        _table_name: &[u8],
        _args: &[&[u8]],
    ) -> SqlResult<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let registry = Arc::clone(aux.ok_or_else(|| module_error("rerail_files registry"))?);
        Ok((
            Cow::Borrowed(
                c"CREATE TABLE x(batch HIDDEN,file_index,path,language,module,node_count)",
            ),
            Self {
                base: ffi::sqlite3_vtab::default(),
                registry,
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> SqlResult<bool> {
        let mut batch = None;
        let mut file = None;
        for (index, constraint) in info.constraints().enumerate() {
            if !constraint.is_usable()
                || constraint.operator() != IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                continue;
            }
            match constraint.column() {
                0 => {
                    batch.get_or_insert(index);
                }
                1 => {
                    file.get_or_insert(index);
                }
                _ => {}
            }
        }
        let Some(batch) = batch else {
            info.set_idx_num(0);
            info.set_estimated_cost(f64::MAX);
            return Ok(true);
        };
        let mut usage = info.constraint_usage(batch);
        usage.set_argv_index(1);
        usage.set_omit(true);
        if let Some(file) = file {
            let mut usage = info.constraint_usage(file);
            usage.set_argv_index(2);
            usage.set_omit(true);
            info.set_idx_num(2);
            info.set_estimated_cost(1.0);
            info.set_estimated_rows(1);
        } else {
            info.set_idx_num(1);
            info.set_estimated_cost(100.0);
            info.set_estimated_rows(100);
        }
        Ok(true)
    }

    fn open(&mut self) -> SqlResult<Self::Cursor> {
        Ok(FilesCursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            registry: Arc::clone(&self.registry),
            view: None,
            files: Vec::new(),
            position: 0,
            phantom: PhantomData,
        })
    }
}

#[repr(C)]
struct FilesCursor<'vtab> {
    base: ffi::sqlite3_vtab_cursor,
    registry: Arc<MatchRegistry>,
    view: Option<BatchView>,
    files: Vec<usize>,
    position: usize,
    phantom: PhantomData<&'vtab FilesTable>,
}

unsafe impl VTabCursor for FilesCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> SqlResult<()> {
        if idx_num == 0 {
            return Err(module_error("rerail_files requires a batch"));
        }
        let view = self
            .registry
            .files(args.get::<i64>(0)?)
            .ok_or_else(|| missing_batch("rerail_files"))?;
        self.files = if idx_num == 2 {
            let file = args.get::<i64>(1)?;
            usize::try_from(file)
                .ok()
                .filter(|&file| file < view.files().len())
                .into_iter()
                .collect()
        } else {
            (0..view.files().len()).collect()
        };
        self.view = Some(view);
        self.position = 0;
        Ok(())
    }

    fn next(&mut self) -> SqlResult<()> {
        self.position += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.position >= self.files.len()
    }

    fn column(&self, context: &mut Context, column: c_int) -> SqlResult<()> {
        let index = self.files[self.position];
        let file = &self.view.as_ref().expect("filtered cursor").files()[index];
        match column {
            0 => context.set_result(&Option::<i64>::None),
            1 => context.set_result(&(index as i64)),
            2 => context.set_result(&&*file.path),
            3 => context.set_result(&&*file.language),
            4 => context.set_result(&file.module.as_deref()),
            5 => context.set_result(&(file.len() as i64)),
            _ => Err(rusqlite::Error::InvalidColumnIndex(column as usize)),
        }
    }

    fn rowid(&self) -> SqlResult<i64> {
        Ok(self.files[self.position] as i64)
    }
}

// ---------------------------------------------------------------------------------------------
// rerail_nodes

/// The walks `rerail_nodes` follows from a node, each in the matcher's order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Axis {
    /// Every node of the file, in preorder (`of` is not used).
    All,
    Itself,
    Parent,
    /// Parent first, up to the root.
    Ancestor,
    /// Every child, named or not, in order.
    Child,
    /// Every descendant in preorder.
    Descendant,
    /// ast-grep's `next`: the next sibling that ends after this node.
    Next,
    /// ast-grep's `next_all`.
    NextAll,
    /// ast-grep's `prev`.
    Prev,
    /// ast-grep's `prev_all`, nearest first.
    PrevAll,
    /// `inside` with `field`: every ancestor, parent first; `via` says whether the walk entered it
    /// through the field (the node it came from is its child in the field).
    FieldAncestor(u16),
    /// `inside` with `field` and `neighbor`: the parent, where the node is its child in the field.
    FieldParent(u16),
    /// `has` with `field` and `neighbor`: the child in the field.
    FieldChild(u16),
    /// `has` with `field` and `end`: the child in the field, then its descendants in preorder.
    FieldDescendant(u16),
    /// `has` with `field` and a stop rule: the child in the field, then its children.
    FieldChildren(u16),
}

impl Axis {
    fn parse(name: &str) -> Option<Self> {
        if let Some((axis, field)) = name.split_once(':') {
            let field = field.parse().ok()?;
            return Some(match axis {
                "field_ancestor" => Self::FieldAncestor(field),
                "field_parent" => Self::FieldParent(field),
                "field_child" => Self::FieldChild(field),
                "field_descendant" => Self::FieldDescendant(field),
                "field_children" => Self::FieldChildren(field),
                _ => return None,
            });
        }
        Some(match name {
            "all" => Self::All,
            "self" => Self::Itself,
            "parent" => Self::Parent,
            "ancestor" => Self::Ancestor,
            "child" => Self::Child,
            "descendant" => Self::Descendant,
            "next" => Self::Next,
            "next_all" => Self::NextAll,
            "prev" => Self::Prev,
            "prev_all" => Self::PrevAll,
            _ => return None,
        })
    }

    /// `all`'s rows: the node asked for (`node = ?`) when the file has it and the selection (if
    /// any) holds it; else the selected roots; else the nodes of the wanted kinds from the file's
    /// kind index, in preorder; else every node in preorder. The cursor still filters by kind.
    fn start_all(
        file: &BatchFile,
        node: Option<i64>,
        selected: Option<&[u32]>,
        kinds: Option<&[i64]>,
    ) -> NodeWalk {
        if let Some(node) = node {
            return match u32::try_from(node) {
                Ok(node)
                    if (node as usize) < file.len()
                        && selected.is_none_or(|roots| roots.contains(&node)) =>
                {
                    NodeWalk::One(Some((node, true)))
                }
                _ => NodeWalk::Empty,
            };
        }
        match (selected, kinds) {
            (Some(roots), _) => NodeWalk::Nodes {
                nodes: roots.to_vec(),
                position: 0,
            },
            (None, Some(kinds)) => {
                let mut kinds = kinds
                    .iter()
                    .filter_map(|&kind| u16::try_from(kind).ok())
                    .collect::<Vec<_>>();
                kinds.sort_unstable();
                kinds.dedup();
                NodeWalk::Kinds(kinds.into_iter().map(|kind| (kind, 0)).collect())
            }
            (None, None) => NodeWalk::Range {
                next: 0,
                end: file.len() as u32,
            },
        }
    }

    /// The walk from `of` along every axis but `all` (`start_all`).
    fn start(self, file: &BatchFile, of: u32) -> NodeWalk {
        if of as usize >= file.len() {
            return NodeWalk::Empty;
        }
        let node = file.node(of);
        match self {
            Self::All => unreachable!("`all` starts with `start_all`"),
            Self::Itself => NodeWalk::One(Some((of, true))),
            Self::Parent => NodeWalk::One(node.parent().map(|node| (node.node_id() as u32, true))),
            Self::Ancestor => NodeWalk::Ancestors {
                next: node.parent().map(|node| node.node_id() as u32),
                last: of,
                field: None,
            },
            Self::Child => NodeWalk::Siblings {
                next: node.children().next().map(|node| node.node_id() as u32),
                forward: true,
            },
            Self::Descendant => {
                let range = file.tape().transducer_descendants(of);
                NodeWalk::Range {
                    next: range.start,
                    end: range.end,
                }
            }
            Self::Next => NodeWalk::One(node.next().map(|node| (node.node_id() as u32, true))),
            Self::NextAll => NodeWalk::Siblings {
                next: node.next_all().next().map(|node| node.node_id() as u32),
                forward: true,
            },
            Self::Prev => NodeWalk::One(node.prev().map(|node| (node.node_id() as u32, true))),
            Self::PrevAll => NodeWalk::Siblings {
                next: node.prev_all().next().map(|node| node.node_id() as u32),
                forward: false,
            },
            Self::FieldAncestor(field) => NodeWalk::Ancestors {
                next: node.parent().map(|node| node.node_id() as u32),
                last: of,
                field: Some(field),
            },
            Self::FieldParent(field) => NodeWalk::One(
                node.parent()
                    .filter(|parent| {
                        parent
                            .child_by_field_id(field)
                            .is_some_and(|child| child.node_id() as u32 == of)
                    })
                    .map(|node| (node.node_id() as u32, true)),
            ),
            Self::FieldChild(field) => NodeWalk::One(
                node.child_by_field_id(field)
                    .map(|node| (node.node_id() as u32, true)),
            ),
            Self::FieldDescendant(field) => node
                .child_by_field_id(field)
                .map(|child| {
                    let child = child.node_id() as u32;
                    let range = file.tape().transducer_descendants(child);
                    NodeWalk::Range {
                        next: child,
                        end: range.end,
                    }
                })
                .unwrap_or(NodeWalk::Empty),
            Self::FieldChildren(field) => NodeWalk::FieldChildren {
                child: node
                    .child_by_field_id(field)
                    .map(|node| node.node_id() as u32),
                next: None,
            },
        }
    }

    /// The walk from `node` in a parsed literal's document, each node by `ordinal`, with whether
    /// the walk entered it (`via`).
    fn walk_node<'r, D: Doc>(
        self,
        node: Node<'r, D>,
        ordinal: impl Fn(&Node<'r, D>) -> u32,
    ) -> Vec<(u32, bool)> {
        let all = |nodes: &mut dyn Iterator<Item = Node<'r, D>>| {
            nodes.map(|node| (ordinal(&node), true)).collect()
        };
        match self {
            Self::All => all(&mut node.root().root().dfs()),
            Self::Itself => vec![(ordinal(&node), true)],
            Self::Parent => all(&mut node.parent().into_iter()),
            Self::Ancestor => all(&mut node.ancestors()),
            Self::Child => all(&mut node.children()),
            Self::Descendant => all(&mut node.dfs().skip(1)),
            Self::Next => all(&mut node.next().into_iter()),
            Self::NextAll => all(&mut node.next_all()),
            Self::Prev => all(&mut node.prev().into_iter()),
            Self::PrevAll => all(&mut node.prev_all()),
            Self::FieldAncestor(field) => {
                let mut last = node.node_id();
                node.ancestors()
                    .map(|candidate| {
                        let expected = std::mem::replace(&mut last, candidate.node_id());
                        let entered = candidate
                            .child_by_field_id(field)
                            .is_some_and(|child| child.node_id() == expected);
                        (ordinal(&candidate), entered)
                    })
                    .collect()
            }
            Self::FieldParent(field) => node
                .parent()
                .filter(|parent| {
                    parent
                        .child_by_field_id(field)
                        .is_some_and(|child| child.node_id() == node.node_id())
                })
                .map(|parent| vec![(ordinal(&parent), true)])
                .unwrap_or_default(),
            Self::FieldChild(field) => all(&mut node.child_by_field_id(field).into_iter()),
            Self::FieldDescendant(field) => match node.child_by_field_id(field) {
                Some(child) => all(&mut child.dfs()),
                None => Vec::new(),
            },
            Self::FieldChildren(field) => match node.child_by_field_id(field) {
                Some(child) => all(&mut std::iter::once(child.clone()).chain(child.children())),
                None => Vec::new(),
            },
        }
    }
}

enum NodeWalk {
    Empty,
    Nodes {
        nodes: Vec<u32>,
        position: usize,
    },
    One(Option<(u32, bool)>),
    /// The nodes of some kinds in preorder: a merge of each kind's `BatchFile::nodes_of_kind`,
    /// with each kind's position in its list.
    Kinds(Vec<(u16, usize)>),
    Range {
        next: u32,
        end: u32,
    },
    Ancestors {
        next: Option<u32>,
        last: u32,
        field: Option<u16>,
    },
    Siblings {
        next: Option<u32>,
        forward: bool,
    },
    FieldChildren {
        child: Option<u32>,
        next: Option<u32>,
    },
    /// A walk over a parsed literal's document, collected whole (`Axis::walk_node`).
    Collected(std::vec::IntoIter<(u32, bool)>),
}

impl NodeWalk {
    /// The next node. Only walks from a node read the tree: a text-only file has none, so every
    /// walk `start_all` begins over it is empty.
    fn next(&mut self, file: &BatchFile) -> Option<(u32, bool)> {
        let node = match self {
            Self::Empty => None,
            Self::Collected(nodes) => nodes.next(),
            Self::Nodes { nodes, position } => {
                let node = nodes.get(*position).copied()?;
                *position += 1;
                Some((node, true))
            }
            Self::One(node) => node.take(),
            Self::Kinds(kinds) => {
                let (node, position) = kinds
                    .iter_mut()
                    .filter_map(|(kind, position)| {
                        let node = *file.nodes_of_kind(*kind).get(*position)?;
                        Some((node, position))
                    })
                    .min_by_key(|&(node, _)| node)?;
                *position += 1;
                Some((node, true))
            }
            Self::Range { next, end } if *next < *end => {
                let node = *next;
                *next += 1;
                Some((node, true))
            }
            Self::Range { .. } => None,
            Self::Ancestors { next, last, field } => {
                let node = next.take()?;
                let entered = field.is_none_or(|field| {
                    file.node(node)
                        .child_by_field_id(field)
                        .is_some_and(|child| child.node_id() as u32 == *last)
                });
                *last = node;
                *next = file.tape().transducer_parent(node);
                Some((node, entered))
            }
            Self::Siblings { next, forward } => {
                let node = next.take()?;
                let tape = file.tape();
                *next = if *forward {
                    tape.transducer_next_sibling(node, false)
                } else {
                    tape.transducer_prev_sibling(node)
                };
                Some((node, true))
            }
            Self::FieldChildren { child, next } => match child.take() {
                Some(node) => {
                    *next = file
                        .node(node)
                        .children()
                        .next()
                        .map(|child| child.node_id() as u32);
                    Some((node, true))
                }
                None => {
                    let node = next.take()?;
                    *next = file.tape().transducer_next_sibling(node, false);
                    Some((node, true))
                }
            },
        };
        #[cfg(test)]
        if node.is_some() {
            count_axis_walk_nodes(1);
        }
        node
    }
}

const NODES_BATCH: c_int = 0;
const NODES_AXIS: c_int = 1;
const NODES_OF: c_int = 2;
const NODES_FILE: c_int = 3;
const NODES_NODE: c_int = 4;
const NODES_POS: c_int = 5;
const NODES_KIND: c_int = 6;
const NODES_SELECTED: c_int = 12;
const NODES_VIA: c_int = 13;
/// `idx_num` bit: the kind constraint is an `IN` list passed whole.
const KIND_LIST: c_int = 1 << 8;

#[repr(C)]
struct NodesTable {
    base: ffi::sqlite3_vtab,
    registry: Arc<MatchRegistry>,
}

unsafe impl<'vtab> VTab<'vtab> for NodesTable {
    type Aux = Arc<MatchRegistry>;
    type Cursor = NodesCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<MatchRegistry>>,
        _module_name: &[u8],
        _database_name: &[u8],
        _table_name: &[u8],
        _args: &[&[u8]],
    ) -> SqlResult<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let registry = Arc::clone(aux.ok_or_else(|| module_error("rerail_nodes registry"))?);
        Ok((
            Cow::Borrowed(
                c"CREATE TABLE x(batch HIDDEN,axis HIDDEN,of HIDDEN,file_index,node,pos,kind,field,named,start_byte,end_byte,parent,selected HIDDEN,via)",
            ),
            Self {
                base: ffi::sqlite3_vtab::default(),
                registry,
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> SqlResult<bool> {
        // Slots: batch, file, axis, of, node, kind, selected.
        let mut slots: [Option<usize>; 7] = [None; 7];
        for (index, constraint) in info.constraints().enumerate() {
            if !constraint.is_usable()
                || constraint.operator() != IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                continue;
            }
            let slot = match constraint.column() {
                NODES_BATCH => 0,
                NODES_FILE => 1,
                NODES_AXIS => 2,
                NODES_OF => 3,
                NODES_NODE => 4,
                NODES_KIND => 5,
                NODES_SELECTED => 6,
                _ => continue,
            };
            slots[slot].get_or_insert(index);
        }
        // A kind `IN` list arrives whole (`KIND_LIST`); otherwise SQLite passes one kind per call.
        let kind_list = match slots[5] {
            Some(kind) => info.is_in_constraint(kind)? && info.set_in_constraint(kind, true)?,
            None => false,
        };
        if slots[0].is_none() || slots[1].is_none() {
            info.set_idx_num(0);
            info.set_estimated_cost(f64::MAX);
            info.set_estimated_rows(i64::MAX);
            return Ok(true);
        }
        let mut idx_num = 0;
        let mut argv = 0;
        for (slot, constraint) in slots.iter().enumerate() {
            if let Some(constraint) = constraint {
                argv += 1;
                idx_num |= 1 << slot;
                let mut usage = info.constraint_usage(*constraint);
                usage.set_argv_index(argv);
                usage.set_omit(true);
            }
        }
        if kind_list {
            idx_num |= KIND_LIST;
        }
        let ordered = info.num_of_order_by() == 1
            && info
                .order_bys()
                .next()
                .is_some_and(|order| order.column() == NODES_POS && !order.is_order_by_desc());
        if ordered {
            info.set_order_by_consumed(true);
        }
        let (cost, rows) = if idx_num & (1 << 4) != 0 {
            (1.0, 1)
        } else if idx_num & (1 << 3) != 0 {
            (10.0, 10)
        } else if idx_num & (1 << 5) != 0 {
            (1_000.0, 1_000)
        } else {
            (10_000.0, 10_000)
        };
        info.set_idx_num(idx_num);
        info.set_estimated_cost(cost);
        info.set_estimated_rows(rows);
        Ok(true)
    }

    fn open(&mut self) -> SqlResult<Self::Cursor> {
        Ok(NodesCursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            registry: Arc::clone(&self.registry),
            view: None,
            doc: None,
            file: 0,
            of: None,
            walk: NodeWalk::Empty,
            current: None,
            wanted: None,
            kinds: None,
            position: 0,
            phantom: PhantomData,
        })
    }
}

/// The document a cursor walks.
enum Walked {
    File(usize),
    Derived(Arc<Derived>),
}

#[repr(C)]
struct NodesCursor<'vtab> {
    base: ffi::sqlite3_vtab_cursor,
    registry: Arc<MatchRegistry>,
    view: Option<BatchView>,
    doc: Option<Walked>,
    /// The document's index, as statements name it.
    file: i64,
    of: Option<i64>,
    /// The walk state; SQLite asks for one row at a time.
    walk: NodeWalk,
    current: Option<(u32, bool)>,
    wanted: Option<i64>,
    kinds: Option<Vec<i64>>,
    position: usize,
    phantom: PhantomData<&'vtab NodesTable>,
}

unsafe impl VTabCursor for NodesCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> SqlResult<()> {
        if idx_num & 3 != 3 {
            return Err(module_error("rerail_nodes requires a batch and a file"));
        }
        let view = self
            .registry
            .files(args.get::<i64>(0)?)
            .ok_or_else(|| missing_batch("rerail_nodes"))?;
        let mut argv = 1;
        let file = args.get::<i64>(argv)?;
        argv += 1;
        let axis_name = if idx_num & (1 << 2) != 0 {
            let axis = args.get::<String>(argv)?;
            argv += 1;
            Some(axis)
        } else {
            None
        };
        let of = if idx_num & (1 << 3) != 0 {
            let of = args.get::<Option<i64>>(argv)?;
            argv += 1;
            Some(of)
        } else {
            None
        };
        let node = if idx_num & (1 << 4) != 0 {
            let node = args.get::<Option<i64>>(argv)?;
            argv += 1;
            Some(node)
        } else {
            None
        };
        let kinds = if idx_num & (1 << 5) != 0 {
            let mut kinds = Vec::new();
            if idx_num & KIND_LIST != 0 {
                let mut values = args.in_values(argv)?;
                while let Some(value) = values.next()? {
                    kinds.push(value.as_i64()?);
                }
            } else {
                kinds.push(args.get::<i64>(argv)?);
            }
            argv += 1;
            Some(kinds)
        } else {
            None
        };
        // `selected = 1`: every node only among the roots the batch's anchor index selected for
        // the running statement (`BatchState::roots`); without a selection, every node.
        let selected = if idx_num & (1 << 6) != 0 && args.get::<i64>(argv)? == 1 {
            batch_state(&view).and_then(|state| state.selected_roots())
        } else {
            None
        };
        let axis = match axis_name.as_deref() {
            Some(name) => {
                Axis::parse(name).ok_or_else(|| module_error(format!("unknown axis {name}")))?
            }
            None if of.is_some() => Axis::Itself,
            None => Axis::All,
        };
        self.position = 0;
        self.file = file;
        self.of = of.flatten();
        self.doc = None;
        self.wanted = node.flatten();
        self.walk = match document(&view, batch_state(&view), file) {
            // `node = NULL` holds for no node.
            _ if node == Some(None) => NodeWalk::Empty,
            Some(Document::File(target)) => {
                self.doc = Some(Walked::File(file as usize));
                match (axis, of) {
                    (Axis::All, _) => {
                        Axis::start_all(target, self.wanted, selected.as_deref(), kinds.as_deref())
                    }
                    (_, Some(Some(of))) => {
                        u32::try_from(of).map_or(NodeWalk::Empty, |of| axis.start(target, of))
                    }
                    // `of` is NULL (a relation from no node) or missing.
                    _ => NodeWalk::Empty,
                }
            }
            // A parsed literal's tree is small: its walk is collected whole.
            Some(Document::Derived(derived)) => {
                let from = match (axis, of) {
                    (Axis::All, _) => Some(0),
                    (_, Some(Some(of))) => u32::try_from(of).ok(),
                    _ => None,
                };
                let walked = match from {
                    Some(from) if (from as usize) < derived.len() => {
                        axis.walk_node(derived.node(from), |node| derived_ordinal(node.node_id()))
                    }
                    _ => Vec::new(),
                };
                self.doc = Some(Walked::Derived(derived));
                NodeWalk::Collected(walked.into_iter())
            }
            None => NodeWalk::Empty,
        };
        self.kinds = kinds;
        self.view = Some(view);
        self.advance();
        Ok(())
    }

    fn next(&mut self) -> SqlResult<()> {
        self.position += 1;
        self.advance();
        Ok(())
    }

    fn eof(&self) -> bool {
        self.current.is_none()
    }

    fn column(&self, context: &mut Context, column: c_int) -> SqlResult<()> {
        let (node, via) = self.current.expect("non-EOF cursor");
        match column {
            NODES_BATCH | NODES_AXIS | NODES_SELECTED => {
                return context.set_result(&Option::<i64>::None);
            }
            NODES_OF => return context.set_result(&self.of),
            NODES_FILE => return context.set_result(&self.file),
            NODES_NODE => return context.set_result(&i64::from(node)),
            NODES_POS => return context.set_result(&(self.position as i64)),
            NODES_VIA => return context.set_result(&i64::from(via)),
            _ => {}
        }
        match self.doc.as_ref().expect("filtered cursor") {
            Walked::File(file) => {
                let tape = self.view.as_ref().expect("filtered cursor").files()[*file].tape();
                match column {
                    6 => context.set_result(&i64::from(tape.transducer_kind(node))),
                    7 => context.set_result(&i64::from(tape.transducer_field(node))),
                    8 => context.set_result(&i64::from(tape.transducer_flags(node) & 1)),
                    9 => context.set_result(&(tape.transducer_span(node).start as i64)),
                    10 => context.set_result(&(tape.transducer_span(node).end as i64)),
                    11 => context.set_result(&tape.transducer_parent(node).map(i64::from)),
                    _ => Err(rusqlite::Error::InvalidColumnIndex(column as usize)),
                }
            }
            Walked::Derived(derived) => {
                let found = derived.node(node);
                match column {
                    6 => context.set_result(&i64::from(found.kind_id())),
                    7 => context.set_result(&0_i64),
                    8 => context.set_result(&i64::from(found.is_named())),
                    9 => context.set_result(&(found.range().start as i64)),
                    10 => context.set_result(&(found.range().end as i64)),
                    11 => context.set_result(
                        &found
                            .parent()
                            .map(|parent| i64::from(derived_ordinal(parent.node_id()))),
                    ),
                    _ => Err(rusqlite::Error::InvalidColumnIndex(column as usize)),
                }
            }
        }
    }

    fn rowid(&self) -> SqlResult<i64> {
        Ok(i64::from(self.current.expect("non-EOF cursor").0))
    }
}

impl NodesCursor<'_> {
    fn advance(&mut self) {
        self.current = None;
        match &self.doc {
            None => {}
            Some(Walked::File(doc)) => {
                let file = &self.view.as_ref().expect("filtered cursor").files()[*doc];
                while let Some(candidate) = self.walk.next(file) {
                    if self
                        .wanted
                        .is_some_and(|node| i64::from(candidate.0) != node)
                        || self.kinds.as_ref().is_some_and(|kinds| {
                            !kinds.contains(&i64::from(file.tape().transducer_kind(candidate.0)))
                        })
                    {
                        continue;
                    }
                    self.current = Some(candidate);
                    break;
                }
            }
            Some(Walked::Derived(derived)) => {
                let NodeWalk::Collected(nodes) = &mut self.walk else {
                    return;
                };
                for candidate in nodes.by_ref() {
                    if self
                        .wanted
                        .is_some_and(|node| i64::from(candidate.0) != node)
                        || self.kinds.as_ref().is_some_and(|kinds| {
                            !kinds.contains(&i64::from(derived.node(candidate.0).kind_id()))
                        })
                    {
                        continue;
                    }
                    self.current = Some(candidate);
                    break;
                }
            }
        }
    }
}
