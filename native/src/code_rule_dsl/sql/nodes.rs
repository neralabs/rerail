//! The `node` virtual table: one row per tape node of every file of the batch, with the columns
//! the lowering joins on (`docs/sql-set-composition-design.md`, "`node` virtual table"). The facts
//! behind the columns are computed once per file when the batch is built (`NodeFacts::of`).

use super::exec::{Batch, BatchSlot, SubFile};
use crate::code_rule_dsl::literal::SubDoc;
use crate::code_rule_dsl::matcher::node_at;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::{AstGrep, Doc};
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::{Value, ValueRef};
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
use std::sync::Arc;

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

const NAMED: u8 = 1;
const LEAF: u8 = 2;
const EXTRA: u8 = 4;
const MISSING: u8 = 8;
const ERROR: u8 = 16;
/// The first child of its parent with its field.
const FIELD_FIRST: u8 = 32;

/// Every column of one file's nodes, by node id.
pub(super) struct NodeFacts {
    len: u32,
    last: Vec<u32>,
    parent: Vec<u32>,
    kind: Vec<u16>,
    field: Vec<u16>,
    flags: Vec<u8>,
    named_ix: Vec<u32>,
    named_n: Vec<u32>,
    /// `SgNode::prev` (the structural previous sibling).
    prev: Vec<u32>,
    /// `SgNode::next`: the next sibling ending after this node ends (zero-width skip).
    next: Vec<u32>,
    /// The structural next sibling.
    nsib: Vec<u32>,
    /// Where `next_all`/`prev_all` start (`TapeNode::walk_anchor`'s neighbours).
    anext: Vec<u32>,
    aprev: Vec<u32>,
    /// The parent of the sibling walks: the parent, or the root itself for the root.
    wpar: Vec<u32>,
    first: Vec<u32>,
    start: Vec<u32>,
    end: Vec<u32>,
    shash: Vec<i64>,
    thash: Vec<i64>,
    /// Node ids by kind, ascending.
    postings: HashMap<u16, Vec<u32>>,
}

impl NodeFacts {
    /// The facts of `root`'s tape, computed in one pass over its nodes (the unification keys
    /// bottom-up, from the last node back).
    pub(super) fn of(root: &AstGrep<TapeDoc>) -> Self {
        let tape = root.root().get_doc();
        Self::build(
            tape.transducer_len() as u32,
            |id| node_at(root, id),
            |node| node.node_id() as u32,
            |id| tape.transducer_descendants(id).end - 1,
            |id| tape.transducer_field(id),
        )
    }

    /// The facts of a parsed literal's tree (`parse`): its nodes are numbered in preorder from
    /// its root, as a tape's are.
    pub(super) fn of_tree(root: &AstGrep<SubDoc>) -> Self {
        let tree = &root.root().get_doc().tree;
        Self::build(
            tree.len() as u32,
            |id| root.adopt(root.root().get_doc().node(id)),
            |node| sub_index(node.node_id()),
            |id| tree.last(id),
            |id| tree.field(id),
        )
    }

    fn build<'r, D: Doc + 'r>(
        len: u32,
        node_at: impl Fn(u32) -> ast_grep_core::Node<'r, D>,
        id_of_node: impl Fn(&ast_grep_core::Node<'r, D>) -> u32,
        last_of: impl Fn(u32) -> u32,
        field_of: impl Fn(u32) -> u16,
    ) -> Self {
        let size = len as usize;
        let mut facts = Self {
            len,
            last: vec![0; size],
            parent: vec![NONE; size],
            kind: vec![0; size],
            field: vec![0; size],
            flags: vec![0; size],
            named_ix: vec![0; size],
            named_n: vec![0; size],
            prev: vec![NONE; size],
            next: vec![NONE; size],
            nsib: vec![NONE; size],
            anext: vec![NONE; size],
            aprev: vec![NONE; size],
            wpar: vec![NONE; size],
            first: vec![NONE; size],
            start: vec![0; size],
            end: vec![0; size],
            shash: vec![0; size],
            thash: vec![0; size],
            postings: HashMap::new(),
        };
        let id_of =
            |node: Option<ast_grep_core::Node<'r, D>>| node.map_or(NONE, |node| id_of_node(&node));
        for id in 0..len {
            let node = node_at(id);
            let index = id as usize;
            let range = node.range();
            facts.last[index] = last_of(id);
            facts.kind[index] = node.kind_id();
            facts.field[index] = field_of(id);
            facts.start[index] = range.start as u32;
            facts.end[index] = range.end as u32;
            let mut flags = 0;
            for (set, flag) in [
                (node.is_named(), NAMED),
                (node.is_named_leaf(), LEAF),
                (node.is_extra(), EXTRA),
                (node.is_missing(), MISSING),
                (node.is_error(), ERROR),
            ] {
                if set {
                    flags |= flag;
                }
            }
            facts.flags[index] |= flags;
            facts.next[index] = id_of(node.next());
            facts.anext[index] = id_of(node.next_all().next());
            facts.aprev[index] = id_of(node.prev_all().next());
            facts.wpar[index] = node.parent().map_or(id, |parent| id_of_node(&parent));
            facts.thash[index] = text_hash(&node.text());
            facts.postings.entry(node.kind_id()).or_default().push(id);
            let mut named = 0;
            let mut seen_fields = Vec::new();
            let mut previous = NONE;
            for child in node.children() {
                let child_id = id_of_node(&child);
                let slot = child_id as usize;
                facts.parent[slot] = id;
                facts.prev[slot] = previous;
                if previous != NONE {
                    facts.nsib[previous as usize] = child_id;
                } else {
                    facts.first[index] = child_id;
                }
                previous = child_id;
                if child.is_named() {
                    named += 1;
                    facts.named_ix[slot] = named;
                }
                let field = field_of(child_id);
                if field != 0 && !seen_fields.contains(&field) {
                    seen_fields.push(field);
                    facts.flags[slot] |= FIELD_FIRST;
                }
            }
            let mut child = facts.first[index];
            while child != NONE {
                facts.named_n[child as usize] = named;
                child = facts.nsib[child as usize];
            }
        }
        // Unification keys: a node with no named child is keyed by its text; any other by its
        // kind and its children's keys (`does_node_match_exactly`).
        for id in (0..len).rev() {
            let index = id as usize;
            facts.shash[index] = if facts.flags[index] & LEAF != 0 {
                let mut hasher = hasher();
                hasher.write_u8(1);
                hasher.write_i64(facts.thash[index]);
                hasher.finish() as i64
            } else {
                let mut hasher = hasher();
                hasher.write_u8(2);
                hasher.write_u16(facts.kind[index]);
                let mut child = facts.first[index];
                while child != NONE {
                    hasher.write_i64(facts.shash[child as usize]);
                    child = facts.nsib[child as usize];
                }
                hasher.finish() as i64
            };
        }
        facts
    }

    pub(super) fn len(&self) -> u32 {
        self.len
    }

    /// The structural next sibling.
    pub(super) fn next_sibling(&self, id: u32) -> Option<u32> {
        let next = self.nsib[id as usize];
        (next != NONE).then_some(next)
    }

    pub(super) fn thash(&self, id: u32) -> i64 {
        self.thash[id as usize]
    }

    pub(super) fn named(&self, id: u32) -> bool {
        self.flags[id as usize] & NAMED != 0
    }

    /// The nodes of `kind`, ascending.
    pub(super) fn of_kind(&self, kind: u16) -> &[u32] {
        self.postings.get(&kind).map_or(&[], Vec::as_slice)
    }

    fn column(&self, id: u32, column: c_int) -> Option<i64> {
        let index = id as usize;
        let node = |value: u32| (value != NONE).then_some(i64::from(value));
        let flag = |flag: u8| Some(i64::from(self.flags[index] & flag != 0));
        match column {
            COL_ID => Some(i64::from(id)),
            COL_LAST => Some(i64::from(self.last[index])),
            COL_PARENT => node(self.parent[index]),
            COL_KIND => Some(i64::from(self.kind[index])),
            COL_FIELD => Some(i64::from(self.field[index])),
            COL_NAMED => flag(NAMED),
            COL_LEAF => flag(LEAF),
            COL_EXTRA => flag(EXTRA),
            COL_MISSING => flag(MISSING),
            COL_ERROR => flag(ERROR),
            COL_NAMED_IX => (self.named_ix[index] != 0).then_some(i64::from(self.named_ix[index])),
            COL_NAMED_N => Some(i64::from(self.named_n[index])),
            COL_PREV => node(self.prev[index]),
            COL_NEXT => node(self.next[index]),
            COL_START => Some(i64::from(self.start[index])),
            COL_END => Some(i64::from(self.end[index])),
            COL_SHASH => Some(self.shash[index]),
            COL_THASH => Some(self.thash[index]),
            COL_NSIB => node(self.nsib[index]),
            COL_ANEXT => node(self.anext[index]),
            COL_APREV => node(self.aprev[index]),
            COL_WPAR => node(self.wpar[index]),
            COL_FFIRST => flag(FIELD_FIRST),
            _ => None,
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
const COL_SHASH: c_int = 17;
const COL_THASH: c_int = 18;
const COL_NSIB: c_int = 19;
const COL_ANEXT: c_int = 20;
const COL_APREV: c_int = 21;
const COL_WPAR: c_int = 22;
const COL_FFIRST: c_int = 23;

const SCHEMA: &CStr = c"CREATE TABLE x(file, id, \"last\", parent, kind, field, named, leaf, extra, missing, error, named_ix, named_n, prev, next, start, \"end\", shash, thash, nsib, anext, aprev, wpar, ffirst)";

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
            Self::LastGe => "l}",
            Self::LastGt => "l>",
            Self::Kind { list: false } => "k=",
            Self::Kind { list: true } => "k*",
        }
    }

    fn parse(code: &str) -> Option<Self> {
        Some(match code {
            "f=" => Self::File { list: false },
            "f*" => Self::File { list: true },
            "i=" => Self::IdEq,
            "i>" => Self::IdGt,
            "i}" => Self::IdGe,
            "i<" => Self::IdLt,
            "i{" => Self::IdLe,
            "p=" => Self::Parent,
            "l}" => Self::LastGe,
            "l>" => Self::LastGt,
            "k=" => Self::Kind { list: false },
            "k*" => Self::Kind { list: true },
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
        // The access path: the most selective constraint; the others filter what it yields.
        let cost: f64;
        if let Some(index) = id_eq {
            used.push((index, Used::IdEq));
            cost = 1.0;
        } else if let Some(index) = parent {
            used.push((index, Used::Parent));
            cost = 10.0;
        } else if let (Some(hi), Some(last)) = (id_hi, last) {
            used.push((hi.0, hi.1));
            used.push((last.0, last.1));
            cost = 10.0;
        } else if id_lo.is_some() || id_hi.is_some() {
            used.extend(id_lo);
            used.extend(id_hi);
            cost = 100.0;
        } else if let Some(index) = kind {
            let list = info.is_in_constraint(index)? && info.set_in_constraint(index, true)?;
            used.push((index, Used::Kind { list }));
            cost = 1_000.0;
        } else {
            cost = 100_000.0;
        }
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
            source: Source::Done,
            current: None,
            sub: None,
            phantom: PhantomData,
        })
    }
}

/// What the constraints left to check on every row, exactly (they were all omitted).
#[derive(Default)]
struct Filter {
    id_lo: Option<u32>,
    id_hi: Option<u32>,
    parent: Option<u32>,
    last_lo: Option<u32>,
    kinds: Option<Vec<u16>>,
    /// No row can satisfy the constraints (a NULL or out-of-range value).
    empty: bool,
}

/// The ids one file's scan yields, ascending, before `Filter`.
enum Source {
    Range { next: u32, end: u32 },
    Chain(u32),
    List { ids: Vec<u32>, pos: usize },
    Done,
}

#[repr(C)]
pub(super) struct NodeCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    slot: Arc<BatchSlot>,
    batch: Option<Arc<Batch>>,
    files: Vec<u32>,
    file_pos: usize,
    filter: Filter,
    source: Source,
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

/// The facts of document `file` of `batch`: a file's, or `sub`'s where it is a parsed tree.
fn facts_of<'a>(batch: &'a Batch, sub: &'a Option<Arc<SubFile>>, file: u32) -> &'a NodeFacts {
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
        let filter = &self.filter;
        let len = facts.len();
        let lo = filter.id_lo.unwrap_or(0);
        let hi = filter.id_hi.map_or(len, |hi| hi.saturating_add(1).min(len));
        self.source = if let Some(parent) = filter.parent {
            if parent < len {
                let first = facts.first[parent as usize];
                if first == NONE {
                    Source::Done
                } else {
                    Source::Chain(first)
                }
            } else {
                Source::Done
            }
        } else if let (Some(last), Some(id_hi)) = (filter.last_lo, filter.id_hi)
            && id_hi <= last
        {
            // `id <= hi <= x` and `last >= x`: the ancestors-or-self of `x`.
            let mut ids = Vec::new();
            let mut at = last;
            while at < len {
                ids.push(at);
                at = facts.parent[at as usize];
            }
            ids.reverse();
            Source::List { ids, pos: 0 }
        } else if let Some(kinds) = &filter.kinds
            && filter.id_lo.is_none()
            && filter.id_hi.is_none()
        {
            let mut ids: Vec<u32> = kinds
                .iter()
                .flat_map(|&kind| facts.of_kind(kind).iter().copied())
                .collect();
            if kinds.len() > 1 {
                ids.sort_unstable();
            }
            Source::List { ids, pos: 0 }
        } else if lo < hi {
            Source::Range { next: lo, end: hi }
        } else {
            Source::Done
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
                if id == NONE {
                    return None;
                }
                *next = facts.nsib[id as usize];
                Some(id)
            }
            Source::List { ids, pos } => {
                let id = ids.get(*pos).copied();
                *pos += 1;
                id
            }
            Source::Done => None,
        }
    }

    fn passes(&self, id: u32) -> bool {
        let batch = self.batch.as_ref().expect("filtered");
        let file = self.files[self.file_pos];
        let facts = facts_of(batch, &self.sub, file);
        let filter = &self.filter;
        let index = id as usize;
        id < facts.len()
            && filter.id_lo.is_none_or(|lo| id >= lo)
            && filter.id_hi.is_none_or(|hi| id <= hi)
            && filter
                .parent
                .is_none_or(|parent| facts.parent[index] == parent)
            && filter.last_lo.is_none_or(|last| facts.last[index] >= last)
            && filter
                .kinds
                .as_ref()
                .is_none_or(|kinds| kinds.contains(&facts.kind[index]))
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
        let batch = self
            .slot
            .current()
            .ok_or_else(|| Error::ModuleError("node: no batch installed".into()))?;
        let codes = idx_str.unwrap_or("");
        let mut filter = Filter::default();
        let mut files: Option<Vec<u32>> = None;
        // Without a file constraint the batch's files; with one, its parsed trees too.
        let count = batch.files.len() as i64;
        let documents = batch.documents() as i64;
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
        for (argv, code) in codes.as_bytes().chunks(2).enumerate() {
            let used = std::str::from_utf8(code)
                .ok()
                .and_then(Used::parse)
                .ok_or_else(|| Error::ModuleError(format!("node: bad plan {codes}")))?;
            let value = || -> Result<Option<i64>> {
                Ok(integer(ValueRef::from(&args.get::<Value>(argv)?)))
            };
            let list = || -> Result<Vec<i64>> {
                let mut values = Vec::new();
                let mut items = args.in_values(argv)?;
                while let Some(item) = items.next()? {
                    values.extend(integer(item));
                }
                Ok(values)
            };
            match used {
                Used::File { list: is_list } => {
                    let mut wanted: Vec<u32> = if is_list {
                        list()?
                    } else {
                        value()?.into_iter().collect()
                    }
                    .into_iter()
                    .filter(|file| (0..documents).contains(file))
                    .map(|file| file as u32)
                    .collect();
                    wanted.sort_unstable();
                    wanted.dedup();
                    files = Some(wanted);
                }
                Used::Kind { list: is_list } => {
                    let mut kinds: Vec<u16> = if is_list {
                        list()?
                    } else {
                        value()?.into_iter().collect()
                    }
                    .into_iter()
                    .filter_map(|kind| u16::try_from(kind).ok())
                    .collect();
                    kinds.sort_unstable();
                    kinds.dedup();
                    filter.kinds = Some(kinds);
                }
                other => {
                    let Some(value) = value()? else {
                        filter.empty = true;
                        continue;
                    };
                    match other {
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
        self.files = files.unwrap_or_else(|| (0..count as u32).collect());
        self.filter = filter;
        self.batch = Some(batch);
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
