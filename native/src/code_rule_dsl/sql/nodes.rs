//! The `node` virtual table: one row per tape node of every document of the batch, with the
//! columns the lowering joins on (`docs/sql-set-composition-design.md`, "`node` virtual table").
//! A row is read from the document's tree in place (`NodeFacts`): no column is copied per node.
//! A document is keyed by its index in the batch (`file`), whatever it is a document of.

use super::exec::{Batch, BatchSlot, SubFile};
use crate::code_rule_dsl::literal::SubDoc;
use crate::code_rule_dsl::matcher::node_at;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::{AstGrep, Doc};
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::ValueRef;
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, VTab, VTabConfig, VTabConnection, VTabCursor,
    sqlite3_vtab, sqlite3_vtab_cursor,
};
use rusqlite::{Error, Result};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::{CStr, c_int};
use std::hash::{BuildHasher, Hasher};
use std::marker::PhantomData;
use std::sync::{Arc, OnceLock};

/// No node (a NULL column).
pub(super) const NONE: u32 = u32::MAX;

/// A parsed tree's node index from its node id: `literal::SubTree`'s ids carry the tree's base
/// above the index.
pub(super) fn sub_index(node_id: usize) -> u32 {
    (node_id & ((1 << 24) - 1)) as u32
}

/// The hasher of every unification key and list hash: fixed, so equal texts hash alike in every
/// file and batch.
pub(super) fn hasher() -> impl Hasher {
    foldhash::quality::FixedState::with_seed(0x5ef_c0de).build_hasher()
}

/// The hash of a text: a node's `thash`.
pub(super) fn text_hash(text: &str) -> i64 {
    let mut hasher = hasher();
    hasher.write(text.as_bytes());
    hasher.write_u8(0xff);
    hasher.finish() as i64
}

/// One document of the batch as the `node` table reads it: its tree's own records, read in
/// place (a tape's, or a parsed literal's), with no per-node copy. Kind, field, span, size and
/// the structural links are read from the records; the flags and the byte-positioned sibling
/// walks from the document's nodes when a row's column is read; `thash` hashes the node's text
/// when read. What needs a pass over the tree is built on its first use and kept with the batch:
/// the kind postings (`Postings`) and the positions among siblings (`Positions`).
pub(super) struct NodeFacts {
    tree: Tree,
    len: u32,
    postings: OnceLock<Postings>,
    values: parking_lot::Mutex<ValuePostings>,
    positions: OnceLock<Positions>,
}

type ValuePostings = HashMap<u16, HashMap<i64, Arc<[u32]>>>;

/// The tree a document's rows come from.
enum Tree {
    Tape(AstGrep<TapeDoc>),
    /// A parsed literal's (`parse`): read through its nodes, as the matcher reads it.
    Sub(AstGrep<SubDoc>),
}

/// Node ids grouped by kind: each kind's ids ascending.
struct Postings {
    /// Each kind present, ascending, and where its ids start in `ids`.
    kinds: Vec<(u16, u32)>,
    ids: Vec<u32>,
}

/// Each node's position among its parent's children.
struct Positions {
    /// The 1-based index among the parent's named children; 0 for an unnamed node or the root.
    named_ix: Vec<u32>,
    /// By parent: how many of its children are named.
    named_children: Vec<u32>,
    /// One bit per node: the first child of its parent with its field (`FIELD_FIRST`).
    field_first: Vec<u64>,
}

/// The heap bytes the batches' node facts have held (counted when built), and the nodes they
/// cover: their bytes per node (`tests::setup`).
#[cfg(test)]
pub(super) static FACT_BYTES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
pub(super) static FACT_NODES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
fn count_bytes(bytes: usize) {
    FACT_BYTES.fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
}

/// A column a row derives from its node (`NodeFacts::derived`).
fn derived<'r, D: Doc>(
    node: ast_grep_core::Node<'r, D>,
    id: u32,
    column: c_int,
    id_of: impl Fn(&ast_grep_core::Node<'r, D>) -> u32,
) -> Option<i64> {
    let at = |node: Option<ast_grep_core::Node<'r, D>>| node.map(|node| i64::from(id_of(&node)));
    match column {
        COL_NAMED => Some(i64::from(node.is_named())),
        COL_LEAF => Some(i64::from(node.is_named_leaf())),
        COL_NEXT => at(node.next()),
        COL_ANEXT => at(node.next_all().next()),
        COL_APREV => at(node.prev_all().next()),
        COL_WPAR => Some(
            node.parent()
                .map_or(i64::from(id), |parent| i64::from(id_of(&parent))),
        ),
        COL_THASH => Some(text_hash(&node.text())),
        _ => None,
    }
}

impl NodeFacts {
    /// The rows of `root`'s tape.
    pub(super) fn of(root: &AstGrep<TapeDoc>) -> Self {
        let len = root.root().get_doc().transducer_len() as u32;
        Self::over(Tree::Tape(root.clone()), len)
    }

    /// The rows of a parsed literal's tree (`parse`): its nodes are numbered in preorder from
    /// its root, as a tape's are.
    pub(super) fn of_tree(root: &AstGrep<SubDoc>) -> Self {
        let len = root.root().get_doc().tree.len() as u32;
        Self::over(Tree::Sub(root.clone()), len)
    }

    fn over(tree: Tree, len: u32) -> Self {
        #[cfg(test)]
        FACT_NODES.fetch_add(len as usize, std::sync::atomic::Ordering::Relaxed);
        Self {
            tree,
            len,
            postings: OnceLock::new(),
            values: Default::default(),
            positions: OnceLock::new(),
        }
    }

    pub(super) fn len(&self) -> u32 {
        self.len
    }

    /// Node `id` of a parsed tree, as the matcher reads it.
    fn sub_node<'r>(root: &'r AstGrep<SubDoc>, id: u32) -> ast_grep_core::Node<'r, SubDoc> {
        root.adopt(root.root().get_doc().node(id))
    }

    /// A parsed tree's node id as a row id.
    fn sub_id(node: &ast_grep_core::Node<'_, SubDoc>) -> u32 {
        sub_index(node.node_id())
    }

    /// The parent, first child, and structural next and previous siblings of `id` (`NONE` for
    /// none).
    fn links(&self, id: u32) -> [u32; 4] {
        match &self.tree {
            Tree::Tape(root) => root.root().get_doc().transducer_links(id),
            Tree::Sub(root) => {
                let node = Self::sub_node(root, id);
                let id_of = |node: Option<ast_grep_core::Node<'_, SubDoc>>| {
                    node.map_or(NONE, |node| Self::sub_id(&node))
                };
                [
                    id_of(node.parent()),
                    id_of(node.children().next()),
                    id_of(node.next()),
                    id_of(node.prev()),
                ]
            }
        }
    }

    pub(super) fn parent(&self, id: u32) -> u32 {
        self.links(id)[0]
    }

    pub(super) fn first(&self, id: u32) -> u32 {
        self.links(id)[1]
    }

    /// The structural next sibling (`NONE` for none).
    pub(super) fn nsib(&self, id: u32) -> u32 {
        self.links(id)[2]
    }

    /// The structural previous sibling (`NONE` for none).
    pub(super) fn prev(&self, id: u32) -> u32 {
        self.links(id)[3]
    }

    /// The structural next sibling.
    pub(super) fn next_sibling(&self, id: u32) -> Option<u32> {
        let next = self.nsib(id);
        (next != NONE).then_some(next)
    }

    /// The last node of `id`'s subtree.
    pub(super) fn last(&self, id: u32) -> u32 {
        match &self.tree {
            Tree::Tape(root) => root.root().get_doc().transducer_descendants(id).end - 1,
            Tree::Sub(root) => root.root().get_doc().tree.last(id),
        }
    }

    pub(super) fn kind(&self, id: u32) -> u16 {
        match &self.tree {
            Tree::Tape(root) => root.root().get_doc().transducer_kind(id),
            Tree::Sub(root) => Self::sub_node(root, id).kind_id(),
        }
    }

    fn field(&self, id: u32) -> u16 {
        match &self.tree {
            Tree::Tape(root) => root.root().get_doc().transducer_field(id),
            Tree::Sub(root) => root.root().get_doc().tree.field(id),
        }
    }

    pub(super) fn thash(&self, id: u32) -> i64 {
        match &self.tree {
            Tree::Tape(root) => text_hash(root.root().get_doc().transducer_text_str(id)),
            Tree::Sub(root) => text_hash(&Self::sub_node(root, id).text()),
        }
    }

    /// Shared by every requesting clause in this immutable batch. Each requested kind is
    /// hashed once. Composite nodes are included: native leaf unification compares their text.
    pub(super) fn value_nodes(&self, kind: u16, hash: i64) -> Arc<[u32]> {
        let mut values = self.values.lock();
        let postings = values.entry(kind).or_insert_with(|| {
            let mut by_value: HashMap<i64, Vec<u32>> = HashMap::new();
            for at in self.kind_range(kind) {
                let id = self.posted(at);
                by_value.entry(self.thash(id)).or_default().push(id);
            }
            #[cfg(test)]
            count_bytes(
                by_value
                    .values()
                    .map(|ids| ids.len() * size_of::<u32>())
                    .sum(),
            );
            by_value
                .into_iter()
                .map(|(key, ids)| (key, ids.into()))
                .collect()
        });
        postings
            .get(&hash)
            .cloned()
            .unwrap_or_else(|| Arc::from([]))
    }

    pub(super) fn named(&self, id: u32) -> bool {
        match &self.tree {
            Tree::Tape(root) => node_at(root, id).is_named(),
            Tree::Sub(root) => Self::sub_node(root, id).is_named(),
        }
    }

    fn postings_built(&self) -> &Postings {
        self.postings.get_or_init(|| self.postings())
    }

    /// Where the nodes of `kind` are among the postings (`posted`).
    fn kind_range(&self, kind: u16) -> std::ops::Range<usize> {
        let postings = self.postings_built();
        match postings
            .kinds
            .binary_search_by_key(&kind, |&(kind, _)| kind)
        {
            Ok(at) => {
                let start = postings.kinds[at].1 as usize;
                let end = postings
                    .kinds
                    .get(at + 1)
                    .map_or(postings.ids.len(), |&(_, end)| end as usize);
                start..end
            }
            Err(_) => 0..0,
        }
    }

    /// The node at `at` of the postings.
    fn posted(&self, at: usize) -> u32 {
        self.postings_built().ids[at]
    }

    /// Every node keyed by its kind, sorted: the postings.
    fn postings(&self) -> Postings {
        let mut keyed: Vec<u64> = (0..self.len)
            .map(|id| u64::from(self.kind(id)) << 32 | u64::from(id))
            .collect();
        keyed.sort_unstable();
        let mut kinds: Vec<(u16, u32)> = Vec::new();
        let ids: Vec<u32> = keyed
            .iter()
            .enumerate()
            .map(|(at, &key)| {
                let kind = (key >> 32) as u16;
                if kinds.last().is_none_or(|&(last, _)| last != kind) {
                    kinds.push((kind, at as u32));
                }
                key as u32
            })
            .collect();
        #[cfg(test)]
        count_bytes(ids.capacity() * 4 + kinds.capacity() * 8);
        Postings { kinds, ids }
    }

    fn positions(&self) -> &Positions {
        self.positions.get_or_init(|| {
            let size = self.len as usize;
            let mut positions = Positions {
                named_ix: vec![0; size],
                named_children: vec![0; size],
                field_first: vec![0; size.div_ceil(64)],
            };
            let mut seen_fields = Vec::new();
            for id in 0..self.len {
                let mut named = 0;
                seen_fields.clear();
                let mut child = self.first(id);
                while child != NONE {
                    let slot = child as usize;
                    // Mutant 7: anonymous siblings count.
                    if self.named(child) || super::mutant(7) {
                        named += 1;
                        positions.named_ix[slot] = named;
                    }
                    let field = self.field(child);
                    if field != 0 && !seen_fields.contains(&field) {
                        seen_fields.push(field);
                        positions.field_first[slot / 64] |= 1 << (slot % 64);
                    }
                    child = self.nsib(child);
                }
                positions.named_children[id as usize] = named;
            }
            #[cfg(test)]
            count_bytes(size * 8 + size.div_ceil(64) * 8);
            positions
        })
    }

    fn column(&self, id: u32, column: c_int) -> Option<i64> {
        let node = |value: u32| (value != NONE).then_some(i64::from(value));
        let flag = |set: bool| Some(i64::from(set));
        match column {
            COL_ID => Some(i64::from(id)),
            COL_LAST => Some(i64::from(self.last(id))),
            COL_PARENT => node(self.parent(id)),
            COL_KIND => Some(i64::from(self.kind(id))),
            COL_FIELD => Some(i64::from(self.field(id))),
            COL_PREV => node(self.prev(id)),
            COL_NSIB => node(self.nsib(id)),
            COL_NAMED_IX => {
                let index = self.positions().named_ix[id as usize];
                (index != 0).then_some(i64::from(index))
            }
            COL_NAMED_N => {
                let parent = self.parent(id);
                Some(if parent == NONE {
                    0
                } else {
                    i64::from(self.positions().named_children[parent as usize])
                })
            }
            COL_FFIRST => {
                let slot = id as usize;
                flag(self.positions().field_first[slot / 64] >> (slot % 64) & 1 == 1)
            }
            _ => match &self.tree {
                Tree::Tape(root) => {
                    let doc = root.root().get_doc();
                    match column {
                        COL_START => Some(doc.transducer_span(id).start as i64),
                        COL_END => Some(doc.transducer_span(id).end as i64),
                        COL_EXTRA => flag(doc.transducer_marks(id)[0]),
                        COL_ERROR => flag(doc.transducer_marks(id)[1]),
                        COL_MISSING => flag(doc.transducer_marks(id)[2]),
                        COL_THASH => Some(text_hash(doc.transducer_text_str(id))),
                        _ => derived(node_at(root, id), id, column, |node| node.node_id() as u32),
                    }
                }
                Tree::Sub(root) => {
                    let sub = Self::sub_node(root, id);
                    match column {
                        COL_START => Some(sub.range().start as i64),
                        COL_END => Some(sub.range().end as i64),
                        COL_EXTRA => flag(sub.is_extra()),
                        COL_ERROR => flag(sub.is_error()),
                        COL_MISSING => flag(sub.is_missing()),
                        _ => derived(sub, id, column, Self::sub_id),
                    }
                }
            },
        }
    }
}

const COL_FILE: c_int = 0;
const COL_ID: c_int = 1;
const COL_LAST: c_int = 2;
const COL_PARENT: c_int = 3;
const COL_KIND: c_int = 4;
const COL_FIELD: c_int = 5;
const COL_NAMED: c_int = 6;
const COL_LEAF: c_int = 7;
const COL_EXTRA: c_int = 8;
const COL_MISSING: c_int = 9;
const COL_ERROR: c_int = 10;
const COL_NAMED_IX: c_int = 11;
const COL_NAMED_N: c_int = 12;
const COL_PREV: c_int = 13;
const COL_NEXT: c_int = 14;
const COL_START: c_int = 15;
const COL_END: c_int = 16;
const COL_THASH: c_int = 17;
const COL_NSIB: c_int = 18;
const COL_ANEXT: c_int = 19;
const COL_APREV: c_int = 20;
const COL_WPAR: c_int = 21;
const COL_FFIRST: c_int = 22;

const SCHEMA: &CStr = c"CREATE TABLE x(file, id, \"last\", parent, kind, field, named, leaf, extra, missing, error, named_ix, named_n, prev, next, start, \"end\", thash, nsib, anext, aprev, wpar, ffirst)";

/// One constraint `xBestIndex` consumed, in argv order (`idx_str`, one letter pair each).
#[derive(Clone, Copy, PartialEq)]
enum Used {
    /// `file = ?` or `file IN (…)` passed whole.
    File {
        list: bool,
    },
    IdEq,
    IdGt,
    IdGe,
    IdLt,
    IdLe,
    Parent,
    TextHash,
    LastGe,
    LastGt,
    Kind {
        list: bool,
    },
}

impl Used {
    fn code(self) -> &'static str {
        match self {
            Self::File { list: false } => "f=",
            Self::File { list: true } => "f*",
            Self::IdEq => "i=",
            Self::IdGt => "i>",
            Self::IdGe => "i}",
            Self::IdLt => "i<",
            Self::IdLe => "i{",
            Self::Parent => "p=",
            Self::TextHash => "h=",
            Self::LastGe => "l}",
            Self::LastGt => "l>",
            Self::Kind { list: false } => "k=",
            Self::Kind { list: true } => "k*",
        }
    }

    fn parse(code: &[u8]) -> Option<Self> {
        Some(match code {
            b"f=" => Self::File { list: false },
            b"f*" => Self::File { list: true },
            b"i=" => Self::IdEq,
            b"i>" => Self::IdGt,
            b"i}" => Self::IdGe,
            b"i<" => Self::IdLt,
            b"i{" => Self::IdLe,
            b"p=" => Self::Parent,
            b"h=" => Self::TextHash,
            b"l}" => Self::LastGe,
            b"l>" => Self::LastGt,
            b"k=" => Self::Kind { list: false },
            b"k*" => Self::Kind { list: true },
            _ => return None,
        })
    }
}

#[repr(C)]
pub(super) struct NodeTable {
    base: sqlite3_vtab,
    slot: Arc<BatchSlot>,
}

unsafe impl<'vtab> VTab<'vtab> for NodeTable {
    type Aux = Arc<BatchSlot>;
    type Cursor = NodeCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<BatchSlot>>,
        _module: &[u8],
        _database: &[u8],
        _table: &[u8],
        _args: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let slot = Arc::clone(aux.ok_or_else(|| Error::ModuleError("node: no batch slot".into()))?);
        Ok((
            Cow::Borrowed(SCHEMA),
            Self {
                base: sqlite3_vtab::default(),
                slot,
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        use IndexConstraintOp as Op;
        let mut file = None;
        let mut id_eq = None;
        let mut id_lo = None;
        let mut id_hi = None;
        let mut parent = None;
        let mut last = None;
        let mut kind = None;
        let mut text_hash = None;
        for (index, constraint) in info.constraints().enumerate() {
            if !constraint.is_usable() {
                continue;
            }
            let op = constraint.operator();
            match (constraint.column(), op) {
                (COL_FILE, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    file.get_or_insert(index);
                }
                (COL_ID, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    id_eq.get_or_insert(index);
                }
                (COL_ID, Op::SQLITE_INDEX_CONSTRAINT_GT) => {
                    id_lo.get_or_insert((index, Used::IdGt));
                }
                (COL_ID, Op::SQLITE_INDEX_CONSTRAINT_GE) => {
                    id_lo.get_or_insert((index, Used::IdGe));
                }
                (COL_ID, Op::SQLITE_INDEX_CONSTRAINT_LT) => {
                    id_hi.get_or_insert((index, Used::IdLt));
                }
                (COL_ID, Op::SQLITE_INDEX_CONSTRAINT_LE) => {
                    id_hi.get_or_insert((index, Used::IdLe));
                }
                (COL_THASH, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    text_hash.get_or_insert(index);
                }
                (COL_PARENT, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    parent.get_or_insert(index);
                }
                (COL_LAST, Op::SQLITE_INDEX_CONSTRAINT_GE) => {
                    last.get_or_insert((index, Used::LastGe));
                }
                (COL_LAST, Op::SQLITE_INDEX_CONSTRAINT_GT) => {
                    last.get_or_insert((index, Used::LastGt));
                }
                (COL_KIND, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    kind.get_or_insert(index);
                }
                _ => {}
            }
        }
        let mut used: Vec<(usize, Used)> = Vec::new();
        if let Some(index) = file {
            let list = info.is_in_constraint(index)? && info.set_in_constraint(index, true)?;
            used.push((index, Used::File { list }));
        }
        // The access path: the most selective constraint; the others filter what it yields. A
        // kind constraint is always taken: alone it reads the kind's postings, with an id range
        // the postings within it, and otherwise it filters what the path yields.
        let cost: f64;
        if let Some(index) = id_eq {
            used.push((index, Used::IdEq));
            cost = 1.0;
        } else if let Some(index) = parent {
            // Children ascend along the chain: an id range starts it later and ends it earlier.
            used.push((index, Used::Parent));
            used.extend(id_lo);
            used.extend(id_hi);
            cost = 10.0;
        } else if let (Some(hi), Some(last)) = (id_hi, last) {
            used.push((hi.0, hi.1));
            used.push((last.0, last.1));
            cost = 10.0;
        } else if id_lo.is_some() || id_hi.is_some() {
            // A range bounded on both sides (a subtree's) yields fewer ids than one bounded on one
            // side, which runs to an end of the file: the planner must prefer it, or it bounds the
            // range on one side and filters the rest of the file row by row.
            cost = if id_lo.is_some() && id_hi.is_some() {
                20.0
            } else if kind.is_some() {
                50.0
            } else {
                100.0
            };
            used.extend(id_lo);
            used.extend(id_hi);
        } else if kind.is_some() {
            cost = 1_000.0;
        } else {
            cost = 100_000.0;
        }
        if let Some(index) = kind {
            let list = info.is_in_constraint(index)? && info.set_in_constraint(index, true)?;
            used.push((index, Used::Kind { list }));
        }
        let cost = if let Some(index) = text_hash {
            used.push((index, Used::TextHash));
            if kind.is_some() { cost.min(8.0) } else { cost }
        } else {
            cost
        };
        let cost = if file.is_some() { cost } else { cost * 1_000.0 };
        let mut codes = String::new();
        for (argv, (index, use_)) in used.iter().enumerate() {
            let mut usage = info.constraint_usage(*index);
            usage.set_argv_index(argv as c_int + 1);
            usage.set_omit(true);
            codes.push_str(use_.code());
        }
        let single_file = matches!(used.first(), Some((_, Used::File { list: false })));
        let ordered = {
            let mut orders = info.order_bys();
            match info.num_of_order_by() {
                1 => {
                    single_file
                        && orders
                            .next()
                            .is_some_and(|o| o.column() == COL_ID && !o.is_order_by_desc())
                }
                2 => {
                    let a = orders.next();
                    let b = orders.next();
                    a.is_some_and(|o| o.column() == COL_FILE && !o.is_order_by_desc())
                        && b.is_some_and(|o| o.column() == COL_ID && !o.is_order_by_desc())
                }
                _ => false,
            }
        };
        if ordered {
            info.set_order_by_consumed(true);
        }
        info.set_idx_str(&codes);
        info.set_estimated_cost(cost);
        info.set_estimated_rows(cost as i64);
        Ok(true)
    }

    fn open(&'vtab mut self) -> Result<NodeCursor<'vtab>> {
        Ok(NodeCursor {
            base: sqlite3_vtab_cursor::default(),
            slot: Arc::clone(&self.slot),
            batch: None,
            files: Vec::new(),
            file_pos: 0,
            filter: Filter::default(),
            kinds: Vec::new(),
            source: Source::Done,
            ids: Vec::new(),
            current: None,
            sub: None,
            phantom: PhantomData,
        })
    }
}

/// What the constraints left to check on every row, exactly (they were all omitted).
#[derive(Clone, Copy, Default)]
struct Filter {
    id_lo: Option<u32>,
    id_hi: Option<u32>,
    parent: Option<u32>,
    last_lo: Option<u32>,
    /// The node's kind is one of the cursor's `kinds`.
    kinds: bool,
    text_hash: Option<i64>,
    /// No row can satisfy the constraints (a NULL or out-of-range value).
    empty: bool,
}

/// The ids one file's scan yields, ascending, before `Filter`.
enum Source {
    Range {
        next: u32,
        end: u32,
    },
    Chain(u32),
    /// The cursor's `ids`, from `pos`.
    Listed {
        pos: usize,
    },
    /// The file's postings (`NodeFacts::posted`) `next..end`.
    Posted {
        next: usize,
        end: usize,
    },
    /// Immutable value postings shared by every lookup in this batch.
    Values {
        ids: Arc<[u32]>,
        next: usize,
        end: usize,
    },
    Done,
}

/// A cursor answers each `xFilter` in time proportional to what it yields: its buffers (`files`,
/// `kinds`, `ids`) are reused across the filters of a statement, and the batch is read from the
/// slot once per statement (a batch outlives every statement run on it).
#[repr(C)]
pub(super) struct NodeCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    slot: Arc<BatchSlot>,
    batch: Option<Arc<Batch>>,
    files: Vec<u32>,
    file_pos: usize,
    filter: Filter,
    /// The kinds a `Filter::kinds` row is one of, ascending.
    kinds: Vec<u16>,
    source: Source,
    /// `Source::Listed`'s ids.
    ids: Vec<u32>,
    current: Option<(u32, u32)>,
    /// The parsed tree being scanned, where the file is one (`exec::SubFile`).
    sub: Option<Arc<SubFile>>,
    phantom: PhantomData<&'vtab NodeTable>,
}

/// An integer constraint value; `None` for NULL or a non-integer (which no column equals).
pub(super) fn integer(value: ValueRef<'_>) -> Option<i64> {
    match value {
        ValueRef::Integer(value) => Some(value),
        ValueRef::Real(value) if value.fract() == 0.0 => Some(value as i64),
        _ => None,
    }
}

/// The first of `0..len` where `before` turns false (it holds on a prefix).
fn partition(len: usize, before: impl Fn(usize) -> bool) -> usize {
    let (mut low, mut high) = (0, len);
    while low < high {
        let middle = low + (high - low) / 2;
        if before(middle) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

/// The facts of document `file` of `batch`: a file's, or `sub`'s where it is a parsed tree.
pub(super) fn facts_of<'a>(
    batch: &'a Batch,
    sub: &'a Option<Arc<SubFile>>,
    file: u32,
) -> &'a NodeFacts {
    match (batch.files.get(file as usize), sub) {
        (Some(entry), _) => &entry.facts,
        (None, Some(sub)) => &sub.facts,
        (None, None) => unreachable!("a document of the batch"),
    }
}

impl NodeCursor<'_> {
    fn start_file(&mut self) {
        self.source = Source::Done;
        let Some(&file) = self.files.get(self.file_pos) else {
            return;
        };
        let batch = self.batch.as_ref().expect("filtered");
        self.sub = batch.subfile(file as usize);
        let facts = facts_of(batch, &self.sub, file);
        let filter = self.filter;
        let len = facts.len();
        let lo = filter.id_lo.unwrap_or(0);
        let hi = filter.id_hi.map_or(len, |hi| hi.saturating_add(1).min(len));
        self.source = if lo >= hi {
            Source::Done
        } else if let Some(hash) = filter.text_hash.filter(|_| filter.kinds) {
            if let [kind] = self.kinds[..] {
                let ids = facts.value_nodes(kind, hash);
                let next = ids.partition_point(|&id| id < lo);
                let end = ids.partition_point(|&id| id < hi);
                self.source = Source::Values { ids, next, end };
                return;
            }
            self.ids.clear();
            for &kind in &self.kinds {
                let ids = facts.value_nodes(kind, hash);
                let from = ids.partition_point(|&id| id < lo);
                let to = ids.partition_point(|&id| id < hi);
                self.ids.extend_from_slice(&ids[from..to]);
            }
            self.ids.sort_unstable();
            self.ids.dedup();
            Source::Listed { pos: 0 }
        } else if let Some(parent) = filter.parent {
            // From the range's first id where it is a child, else the first child.
            let first = match filter.id_lo {
                _ if parent >= len => NONE,
                Some(lo) if lo < len && facts.parent(lo) == parent => lo,
                _ => facts.first(parent),
            };
            if first == NONE {
                Source::Done
            } else {
                Source::Chain(first)
            }
        } else if let (Some(last), Some(id_hi)) = (filter.last_lo, filter.id_hi)
            && id_hi <= last
        {
            // `id <= hi <= x` and `last >= x`: the ancestors-or-self of `x`.
            self.ids.clear();
            let mut at = last;
            while at < len {
                self.ids.push(at);
                at = facts.parent(at);
            }
            self.ids.reverse();
            Source::Listed { pos: 0 }
        } else if filter.kinds && (hi - lo) as usize > 8 * self.kinds.len() {
            // The kinds' postings within `lo..hi`, unless the range is the shorter read.
            let within = |kind: u16| {
                let range = facts.kind_range(kind);
                let from =
                    range.start + partition(range.len(), |at| facts.posted(range.start + at) < lo);
                let to =
                    range.start + partition(range.len(), |at| facts.posted(range.start + at) < hi);
                from..to
            };
            if let [kind] = self.kinds[..] {
                let range = within(kind);
                Source::Posted {
                    next: range.start,
                    end: range.end,
                }
            } else {
                self.ids.clear();
                for &kind in &self.kinds {
                    self.ids.extend(within(kind).map(|at| facts.posted(at)));
                }
                self.ids.sort_unstable();
                Source::Listed { pos: 0 }
            }
        } else {
            Source::Range { next: lo, end: hi }
        };
    }

    fn next_id(&mut self) -> Option<u32> {
        let batch = self.batch.as_ref().expect("filtered");
        let file = self.files[self.file_pos];
        let facts = facts_of(batch, &self.sub, file);
        match &mut self.source {
            Source::Range { next, end } => (*next < *end).then(|| {
                *next += 1;
                *next - 1
            }),
            Source::Chain(next) => {
                let id = *next;
                if id == NONE || self.filter.id_hi.is_some_and(|hi| id > hi) {
                    return None;
                }
                *next = facts.nsib(id);
                Some(id)
            }
            Source::Listed { pos } => {
                let id = self.ids.get(*pos).copied();
                *pos += 1;
                id
            }
            Source::Posted { next, end } => (*next < *end).then(|| {
                *next += 1;
                facts.posted(*next - 1)
            }),
            Source::Values { ids, next, end } => (*next < *end).then(|| {
                *next += 1;
                ids[*next - 1]
            }),
            Source::Done => None,
        }
    }

    fn passes(&self, id: u32) -> bool {
        let batch = self.batch.as_ref().expect("filtered");
        let file = self.files[self.file_pos];
        let facts = facts_of(batch, &self.sub, file);
        let filter = &self.filter;
        id < facts.len()
            && filter.id_lo.is_none_or(|lo| id >= lo)
            && filter.id_hi.is_none_or(|hi| id <= hi)
            && filter
                .parent
                .is_none_or(|parent| facts.parent(id) == parent)
            && filter.last_lo.is_none_or(|last| facts.last(id) >= last)
            && (!filter.kinds || self.kinds.binary_search(&facts.kind(id)).is_ok())
            && filter.text_hash.is_none_or(|hash| facts.thash(id) == hash)
    }

    fn advance(&mut self) {
        self.current = None;
        if self.filter.empty {
            return;
        }
        while self.file_pos < self.files.len() {
            while let Some(id) = self.next_id() {
                if self.passes(id) {
                    self.current = Some((self.files[self.file_pos], id));
                    return;
                }
            }
            self.file_pos += 1;
            self.start_file();
        }
    }
}

unsafe impl VTabCursor for NodeCursor<'_> {
    fn filter(&mut self, _: c_int, idx_str: Option<&str>, args: &Filters<'_>) -> Result<()> {
        if self.batch.is_none() {
            self.batch = Some(
                self.slot
                    .current()
                    .ok_or_else(|| Error::ModuleError("node: no batch installed".into()))?,
            );
        }
        let batch = self.batch.as_deref().expect("installed above");
        let codes = idx_str.unwrap_or("");
        let mut filter = Filter::default();
        let mut constrained_files = false;
        // Without a file constraint the batch's files; with one, its parsed trees too.
        let count = batch.files.len() as i64;
        let document = |file: i64| -> Option<u32> {
            ((0..count).contains(&file) || (0..batch.documents() as i64).contains(&file))
                .then_some(file as u32)
        };
        let clamp = |value: i64| u32::try_from(value.max(0)).unwrap_or(u32::MAX - 1);
        let narrow = |filter: &mut Filter, lo: Option<i64>, hi: Option<i64>| {
            if let Some(lo) = lo {
                let lo = clamp(lo);
                filter.id_lo = Some(filter.id_lo.map_or(lo, |old| old.max(lo)));
            }
            if let Some(hi) = hi {
                if hi < 0 {
                    filter.empty = true;
                } else {
                    let hi = clamp(hi);
                    filter.id_hi = Some(filter.id_hi.map_or(hi, |old| old.min(hi)));
                }
            }
        };
        for ((argv, code), value) in codes.as_bytes().chunks(2).enumerate().zip(args.iter()) {
            let used = Used::parse(code)
                .ok_or_else(|| Error::ModuleError(format!("node: bad plan {codes}")))?;
            match used {
                Used::File { list } => {
                    constrained_files = true;
                    self.files.clear();
                    if list {
                        let mut items = args.in_values(argv)?;
                        while let Some(item) = items.next()? {
                            self.files.extend(integer(item).and_then(document));
                        }
                        self.files.sort_unstable();
                        self.files.dedup();
                    } else {
                        self.files.extend(integer(value).and_then(document));
                    }
                }
                Used::Kind { list } => {
                    filter.kinds = true;
                    self.kinds.clear();
                    let kind = |value: i64| u16::try_from(value).ok();
                    if list {
                        let mut items = args.in_values(argv)?;
                        while let Some(item) = items.next()? {
                            self.kinds.extend(integer(item).and_then(kind));
                        }
                        self.kinds.sort_unstable();
                        self.kinds.dedup();
                    } else {
                        self.kinds.extend(integer(value).and_then(kind));
                    }
                }
                other => {
                    let Some(value) = integer(value) else {
                        filter.empty = true;
                        continue;
                    };
                    match other {
                        Used::TextHash => filter.text_hash = Some(value),
                        Used::IdEq => narrow(&mut filter, Some(value), Some(value)),
                        Used::IdGt => narrow(&mut filter, Some(value.saturating_add(1)), None),
                        Used::IdGe => narrow(&mut filter, Some(value), None),
                        Used::IdLt => narrow(&mut filter, None, Some(value.saturating_sub(1))),
                        Used::IdLe => narrow(&mut filter, None, Some(value)),
                        Used::Parent => {
                            if value < 0 {
                                filter.empty = true;
                            } else {
                                filter.parent = Some(clamp(value));
                            }
                        }
                        Used::LastGe | Used::LastGt => {
                            let value = if matches!(other, Used::LastGt) {
                                value.saturating_add(1)
                            } else {
                                value
                            };
                            let value = clamp(value);
                            filter.last_lo =
                                Some(filter.last_lo.map_or(value, |old| old.max(value)));
                        }
                        _ => unreachable!("handled above"),
                    }
                }
            }
        }
        if !constrained_files {
            self.files.clear();
            self.files.extend(0..count as u32);
        }
        self.filter = filter;
        self.file_pos = 0;
        self.start_file();
        self.advance();
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.advance();
        Ok(())
    }

    fn eof(&self) -> bool {
        self.current.is_none()
    }

    fn column(&self, context: &mut Context, column: c_int) -> Result<()> {
        let (file, id) = self.current.expect("a row");
        if column == COL_FILE {
            return context.set_result(&i64::from(file));
        }
        let batch = self.batch.as_ref().expect("filtered");
        context.set_result(&facts_of(batch, &self.sub, file).column(id, column))
    }

    fn rowid(&self) -> Result<i64> {
        let (file, id) = self.current.expect("a row");
        Ok((i64::from(file) << 32) | i64::from(id))
    }
}
