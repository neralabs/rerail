//! The `nearest` virtual table: a structural primitive over a node set `S`, no DSL operator. A
//! statement names `S` by a stage's rows, `stage = n AND complement = c AND member IN (SELECT -1
//! UNION ALL SELECT file * 4294967296 + node FROM stage_n)` (the `-1` keeps the list non-empty):
//! `S` is those rows, or with `complement = 1` every other node of the file (`plan::NodeSet`).
//! It reads one row per node of a file with, relative to `S`:
//! - `anc`: the node's nearest proper ancestor in `S`, `-1` where none is;
//! - `base`: the node itself where it is in `S`, else `anc`;
//! - `nxt`: the first node in `S` of the chain from the node along structural next siblings (the
//!   node, its next sibling, …), NULL where none is;
//! - `prv`: the same along structural previous siblings;
//! - `last`: its last descendant, as `node`'s (read from the file's structure, not kept).
//!
//! A file's columns are computed in one preorder pass (`anc`, `prv`) and one reverse pass (`nxt`)
//! the first time a statement reads the file for the set, and kept for the batch, with the file's
//! nodes grouped by `anc` (ascending in each group). Lookups are by `id` (one row) or by `anc`
//! with an `id` range (the nodes whose nearest proper `S`-ancestor is `anc`), within one `file`.
//!
//! A descendant walk with kinds lists its candidates in one native pass over its whole input
//! (`descendants`): `srcs IN (…)` passes the input frame's nodes whole, `kinds` the walk's kinds,
//! and optionally `S` (`has` with a rule stop; without it `stopBy: end`) and capture-free shape
//! tests (`shape`, `probe_of`). The statement's first filter reads the nodes once and lists, file
//! by file, each node's descendants of those kinds the walk visits (`anc` equal to the node's
//! base, where `S` is given) and every shape test holds at: from the kind postings or the `anc`
//! group, whichever is the shorter read, testing kinds and shapes on the tape. Each input row's
//! filter (`file`, `src`: its node) then reads its node's list, so the rows come in the input's
//! order, as a join per row would yield them.
//!
//! Keeping a set for the batch is exact because a stage's rows are fixed once the stage ran:
//! `Session::run_batch` runs every stage before any clause that reads it (a stage with one
//! consumer in that clause's run, before its statements after the root frame), a stage reads
//! only stages made before it, and every batch is a fresh `Batch` (the cache is keyed by its
//! identity). So the member list is read once per stage and batch; later statements' lists are
//! not read.

use super::exec::{Batch, BatchSlot, SubFile};
use super::nodes::{NONE, NodeFacts, facts_of, integer};
use super::plan::{NodeSet, Shape};
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::{Value, ValueRef};
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, VTab, VTabConfig, VTabConnection, VTabCursor,
    sqlite3_vtab, sqlite3_vtab_cursor,
};
use rusqlite::{Error, Result};
use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::ffi::{CStr, c_int};
use std::fmt::Write;
use std::marker::PhantomData;
use std::sync::{Arc, Weak};

const COL_FILE: c_int = 0;
const COL_ID: c_int = 1;
const COL_LAST: c_int = 2;
const COL_ANC: c_int = 3;
const COL_BASE: c_int = 4;
const COL_NXT: c_int = 5;
const COL_PRV: c_int = 6;
const COL_STAGE: c_int = 7;
const COL_COMPLEMENT: c_int = 8;
const COL_MEMBER: c_int = 9;
const COL_SRC: c_int = 10;
const COL_SRCS: c_int = 11;
const COL_KINDS: c_int = 12;
const COL_SHAPE: c_int = 13;

const SCHEMA: &CStr = c"CREATE TABLE x(file, id, \"last\", anc, base, nxt, prv, stage HIDDEN, complement HIDDEN, member HIDDEN, src HIDDEN, srcs HIDDEN, kinds HIDDEN, shape HIDDEN)";

/// The constraints naming `set` as the set of a `nearest` table aliased `alias`. A complement
/// passes its stage's rows, which the table leaves out.
fn members(alias: &str, set: NodeSet) -> String {
    format!(
        "{alias}.stage = {} AND {alias}.complement = {} AND {alias}.member IN (SELECT -1 UNION ALL SELECT file * 4294967296 + node FROM {})",
        set.stage,
        i64::from(set.complement),
        super::emit::stage_table(set.stage)
    )
}

/// The SQL naming `set` as the set of a `nearest` table aliased `alias`, in file `file` (an SQL
/// expression): the constraints to put in its join.
pub(super) fn set_of(alias: &str, set: NodeSet, file: &str) -> String {
    format!("{} AND {alias}.file = {file}", members(alias, set))
}

/// A descendant walk's candidates of `kinds` (not empty) from every row of `input`, as `pk, file,
/// node` rows: `has` with `stopBy: end`, or with the stop set `stop`, listed by one native pass
/// over `input`'s nodes that tests `probe` (`probe_of`) on each candidate too.
pub(super) fn descendants(
    input: &str,
    stop: Option<NodeSet>,
    kinds: &[u16],
    probe: Option<&str>,
) -> String {
    let kinds: Vec<String> = kinds.iter().map(u16::to_string).collect();
    let mut sql = format!(
        "SELECT f.k AS pk, c.file AS file, c.id AS node FROM {input} f CROSS JOIN nearest c \
         ON c.file = f.file AND c.src = f.node \
         AND c.srcs IN (SELECT file * 4294967296 + node FROM {input}) AND c.kinds = '{}'",
        kinds.join(",")
    );
    if let Some(set) = stop {
        write!(sql, " AND {}", members("c", set)).expect("write to a string");
    }
    if let Some(probe) = probe {
        write!(sql, " AND c.shape = '{probe}'").expect("write to a string");
    }
    sql
}

/// `shape` as the tests `descendants` makes on a candidate, each of one node: a kind (`K`), a
/// descendant's (`D`), a child's (`C`) or the first child's in a field (`F<field>`), then `:` and
/// the kinds. An `any` of one such test (one field) is that test of their kinds together: a node
/// (a child, a descendant, a field's first child) has a kind of some branch exactly where it has a
/// kind of their union. An `all` is each of its tests, distinct, joined by `;` (one constraint: a
/// table sees one equality per column). `None` where an `any` mixes tests, which the statement
/// tests instead.
pub(super) fn probe_of(shape: &Shape) -> Option<String> {
    fn test(shape: &Shape) -> Option<(String, BTreeSet<u16>)> {
        let (code, kinds) = match shape {
            Shape::Kinds(kinds) => ("K".to_owned(), kinds),
            Shape::Contains(kinds) => ("D".to_owned(), kinds),
            Shape::Child(kinds) => ("C".to_owned(), kinds),
            Shape::Field(field, kinds) => (format!("F{field}"), kinds),
            Shape::Any(shapes) => {
                let (mut code, mut kinds) = (None, BTreeSet::new());
                for shape in shapes {
                    let (own, more) = test(shape)?;
                    if *code.get_or_insert_with(|| own.clone()) != own {
                        return None;
                    }
                    kinds.extend(more);
                }
                return Some((code?, kinds));
            }
            Shape::All(_) => return None,
        };
        Some((code, kinds.iter().copied().collect()))
    }
    fn add(shape: &Shape, probes: &mut Vec<String>) -> Option<()> {
        if let Shape::All(shapes) = shape {
            return shapes.iter().try_for_each(|shape| add(shape, probes));
        }
        let (code, kinds) = test(shape)?;
        let kinds: Vec<String> = kinds.iter().map(u16::to_string).collect();
        let probe = format!("{code}:{}", kinds.join(","));
        if !probes.contains(&probe) {
            probes.push(probe);
        }
        Some(())
    }
    let mut probes = Vec::new();
    add(shape, &mut probes)?;
    Some(probes.join(";"))
}

/// Kinds written as `descendants` writes them (`1,2`), ascending and distinct; `None` where the
/// text is not such a list.
fn kinds_in(text: &str, kinds: &mut Vec<u16>) -> Option<()> {
    kinds.clear();
    if !text.is_empty() {
        for kind in text.split(',') {
            kinds.push(kind.parse().ok()?);
        }
    }
    kinds.sort_unstable();
    kinds.dedup();
    Some(())
}

/// A shape test `descendants` makes on each candidate (`probe_of`).
enum Probe {
    Kinds(Vec<u16>),
    Contains(Vec<u16>),
    Child(Vec<u16>),
    Field(u16, Vec<u16>),
}

impl Probe {
    fn parse(text: &str) -> Option<Self> {
        let (code, list) = text.split_once(':')?;
        let mut kinds = Vec::new();
        kinds_in(list, &mut kinds)?;
        Some(match code {
            "K" => Self::Kinds(kinds),
            "D" => Self::Contains(kinds),
            "C" => Self::Child(kinds),
            _ => Self::Field(code.strip_prefix('F')?.parse().ok()?, kinds),
        })
    }

    /// Whether it holds at node `id`, as `emit::shape_test` reads it from the `node` table.
    fn holds(&self, facts: &NodeFacts, id: u32) -> bool {
        let of = |kinds: &[u16], node: u32| kinds.binary_search(&facts.kind(node)).is_ok();
        match self {
            Self::Kinds(kinds) => of(kinds, id),
            Self::Contains(kinds) => {
                let (lo, hi) = (id + 1, facts.last(id) + 1);
                if (hi - lo) as usize <= 8 * kinds.len() {
                    (lo..hi).any(|node| of(kinds, node))
                } else {
                    kinds
                        .iter()
                        .any(|&kind| !within(facts.kind_ids(kind), lo, hi).is_empty())
                }
            }
            Self::Child(kinds) => {
                let mut child = facts.first(id);
                while child != NONE {
                    if of(kinds, child) {
                        return true;
                    }
                    child = facts.nsib(child);
                }
                false
            }
            // The first child with the field (`node`'s `ffirst`; no node has field 0 first).
            Self::Field(field, kinds) => {
                let mut child = if *field == 0 { NONE } else { facts.first(id) };
                while child != NONE {
                    if facts.field(child) == *field {
                        return of(kinds, child);
                    }
                    child = facts.nsib(child);
                }
                false
            }
        }
    }
}

/// The ids of an ascending list within `lo..hi`.
fn within(ids: &[u32], lo: u32, hi: u32) -> &[u32] {
    let from = ids.partition_point(|&id| id < lo);
    let to = from + ids[from..].partition_point(|&id| id < hi);
    &ids[from..to]
}

/// One file's columns for one set.
struct FileNearest {
    anc: Vec<u32>,
    /// A member's rank among the file's members (ascending ids), `NONE` for a non-member.
    rank: Vec<u32>,
    nxt: Vec<u32>,
    prv: Vec<u32>,
    /// The nodes grouped by `anc`, each group ascending: group 0 has no `S`-ancestor, group `r + 1`
    /// has the member of rank `r`; group `g` is `ids[offsets[g]..offsets[g + 1]]`.
    offsets: Vec<u32>,
    ids: Vec<u32>,
}

impl FileNearest {
    /// `listed`: the file's listed nodes, ascending and distinct. The members are them, or
    /// (`complement`) every other node of the file.
    fn of(facts: &NodeFacts, listed: &[u32], complement: bool) -> Self {
        let len = facts.len() as usize;
        let mut member = vec![complement; len];
        for &id in listed {
            if (id as usize) < len {
                member[id as usize] = !complement;
            }
        }
        let mut rank = vec![NONE; len];
        let mut count = 0u32;
        for (id, &is) in member.iter().enumerate() {
            if is {
                rank[id] = count;
                count += 1;
            }
        }
        let is_member = |id: u32| member[id as usize];
        // A parent and a previous sibling come before a node in preorder; a next sibling after.
        let mut anc = vec![NONE; len];
        let mut prv = vec![NONE; len];
        for id in 0..len as u32 {
            let index = id as usize;
            let parent = facts.parent(id);
            if parent != NONE {
                anc[index] = if is_member(parent) {
                    parent
                } else {
                    anc[parent as usize]
                };
            }
            prv[index] = if is_member(id) {
                id
            } else {
                match facts.prev(id) {
                    NONE => NONE,
                    prev => prv[prev as usize],
                }
            };
        }
        let mut nxt = vec![NONE; len];
        for id in (0..len as u32).rev() {
            let index = id as usize;
            nxt[index] = if is_member(id) {
                id
            } else {
                match facts.nsib(id) {
                    NONE => NONE,
                    next => nxt[next as usize],
                }
            };
        }
        let group = |ancestor: u32| match ancestor {
            NONE => 0,
            ancestor => rank[ancestor as usize] as usize + 1,
        };
        let mut offsets = vec![0u32; count as usize + 2];
        for &ancestor in &anc {
            offsets[group(ancestor) + 1] += 1;
        }
        for at in 1..offsets.len() {
            offsets[at] += offsets[at - 1];
        }
        let mut fill = offsets.clone();
        let mut ids = vec![0u32; len];
        for id in 0..len as u32 {
            let slot = &mut fill[group(anc[id as usize])];
            ids[*slot as usize] = id;
            *slot += 1;
        }
        Self {
            anc,
            rank,
            nxt,
            prv,
            offsets,
            ids,
        }
    }

    fn len(&self) -> u32 {
        self.anc.len() as u32
    }

    /// The positions in `ids` of the nodes whose nearest proper `S`-ancestor is `ancestor` (`-1`:
    /// none), with ids in `lo..=hi`.
    fn group(&self, ancestor: i64, lo: u32, hi: u32) -> (usize, usize) {
        let group = if ancestor < 0 {
            0
        } else {
            match u32::try_from(ancestor)
                .ok()
                .and_then(|id| self.rank.get(id as usize).copied())
            {
                Some(rank) if rank != NONE => rank as usize + 1,
                _ => return (0, 0),
            }
        };
        let (start, end) = (
            self.offsets[group] as usize,
            self.offsets[group + 1] as usize,
        );
        let ids = &self.ids[start..end];
        let from = ids.partition_point(|&id| id < lo);
        let to = ids.partition_point(|&id| id <= hi);
        (start + from, start + to.max(from))
    }

    /// `id` where it is a member, else its nearest proper ancestor in `S` (`NONE`: none).
    fn base(&self, id: u32) -> u32 {
        let index = id as usize;
        if self.rank[index] != NONE {
            id
        } else {
            self.anc[index]
        }
    }
}

/// A descendant walk's rows (`descendants`), listed by the statement's first filter for every
/// node of its input and read by each filter after it: a cursor lives for one run of its
/// statement, in which the input's `IN` list stays the same.
#[derive(Default)]
struct Walked {
    /// Whether the rows are listed.
    listed: bool,
    /// The input's nodes as `file << 32 | node`, ascending and distinct.
    inputs: Vec<u64>,
    /// The rows of `inputs[i]` are `ids[starts[i]..starts[i + 1]]`, ascending.
    starts: Vec<u32>,
    ids: Vec<u32>,
    /// The candidates' kinds, ascending and distinct.
    kinds: Vec<u16>,
    probes: Vec<Probe>,
    /// One input node's candidates before the shape tests.
    scratch: Vec<u32>,
    /// The current filter's rows: positions in `ids`.
    next: usize,
    end: usize,
}

impl Walked {
    /// Lists the rows of the input's nodes `inputs[nodes]`: one file's, whose stop set's columns
    /// are `stop` (`None`: `stopBy: end`).
    fn visit(
        &mut self,
        facts: &NodeFacts,
        stop: Option<&FileNearest>,
        nodes: std::ops::Range<usize>,
    ) {
        let Self {
            inputs,
            starts,
            ids,
            kinds,
            probes,
            scratch,
            ..
        } = self;
        let len = facts.len();
        let of = |node: u32| kinds.binary_search(&facts.kind(node)).is_ok();
        // A range at most this long is read whole rather than through the postings (as `node`).
        let short = 8 * kinds.len();
        for &key in &inputs[nodes] {
            starts.push(ids.len() as u32);
            let from = (key & 0xffff_ffff) as u32;
            if from >= len {
                continue;
            }
            let (lo, hi) = (from + 1, facts.last(from) + 1);
            if lo >= hi {
                continue;
            }
            scratch.clear();
            match stop {
                None if (hi - lo) as usize <= short => {
                    scratch.extend((lo..hi).filter(|&id| of(id)))
                }
                None => {
                    for &kind in kinds.iter() {
                        scratch.extend_from_slice(within(facts.kind_ids(kind), lo, hi));
                    }
                    if kinds.len() > 1 {
                        scratch.sort_unstable();
                    }
                }
                // The nodes below `from` whose nearest proper ancestor in `S` is `from`'s base:
                // its `anc` group's, or the kinds' postings', whichever is shorter.
                Some(stop) => {
                    let base = stop.base(from);
                    let ancestor = if base == NONE { -1 } else { i64::from(base) };
                    let (start, end) = stop.group(ancestor, lo, hi - 1);
                    let group = &stop.ids[start..end];
                    let posted = || {
                        kinds
                            .iter()
                            .map(|&kind| within(facts.kind_ids(kind), lo, hi).len())
                            .sum::<usize>()
                    };
                    if group.len() <= short || group.len() <= posted() {
                        scratch.extend(group.iter().copied().filter(|&id| of(id)));
                    } else {
                        for &kind in kinds.iter() {
                            scratch.extend(
                                within(facts.kind_ids(kind), lo, hi)
                                    .iter()
                                    .copied()
                                    .filter(|&id| stop.anc[id as usize] == base),
                            );
                        }
                        if kinds.len() > 1 {
                            scratch.sort_unstable();
                        }
                    }
                }
            }
            ids.extend(
                scratch
                    .iter()
                    .filter(|&&id| probes.iter().all(|probe| probe.holds(facts, id))),
            );
        }
    }
}

/// The sets read in the current batch, by stage.
#[derive(Default)]
struct Cache {
    batch: Weak<Batch>,
    stages: HashMap<i64, StageSet>,
}

struct StageSet {
    /// Each file's rows of the stage, ascending and distinct.
    listed: HashMap<u32, Vec<u32>>,
    /// Each file's columns, for the stage's rows and for their complement.
    files: HashMap<(u32, bool), Arc<FileNearest>>,
}

#[repr(C)]
pub(super) struct NearestTable {
    base: sqlite3_vtab,
    slot: Arc<BatchSlot>,
    cache: Arc<parking_lot::Mutex<Cache>>,
}

/// One constraint `xBestIndex` consumed, in argv order (`idx_str`, one letter pair each, as
/// `node`'s).
#[derive(Clone, Copy)]
enum Used {
    Stage,
    Complement,
    Member,
    File,
    IdEq,
    IdGt,
    IdGe,
    IdLt,
    IdLe,
    Anc,
    /// `src = ?` (an input row's node) and `srcs IN (…)` passed whole (the input's nodes): a
    /// descendant walk (`descendants`).
    Src,
    Srcs,
    Kinds,
    Shape,
}

impl Used {
    fn code(self) -> &'static str {
        match self {
            Self::Stage => "s=",
            Self::Complement => "c=",
            Self::Member => "m*",
            Self::File => "f=",
            Self::IdEq => "i=",
            Self::IdGt => "i>",
            Self::IdGe => "i}",
            Self::IdLt => "i<",
            Self::IdLe => "i{",
            Self::Anc => "a=",
            Self::Src => "b=",
            Self::Srcs => "n*",
            Self::Kinds => "k=",
            Self::Shape => "h=",
        }
    }

    fn parse(code: &[u8]) -> Option<Self> {
        Some(match code {
            b"s=" => Self::Stage,
            b"c=" => Self::Complement,
            b"m*" => Self::Member,
            b"f=" => Self::File,
            b"i=" => Self::IdEq,
            b"i>" => Self::IdGt,
            b"i}" => Self::IdGe,
            b"i<" => Self::IdLt,
            b"i{" => Self::IdLe,
            b"a=" => Self::Anc,
            b"b=" => Self::Src,
            b"n*" => Self::Srcs,
            b"k=" => Self::Kinds,
            b"h=" => Self::Shape,
            _ => return None,
        })
    }
}

unsafe impl<'vtab> VTab<'vtab> for NearestTable {
    type Aux = Arc<BatchSlot>;
    type Cursor = NearestCursor<'vtab>;

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
            Arc::clone(aux.ok_or_else(|| Error::ModuleError("nearest: no batch slot".into()))?);
        Ok((
            Cow::Borrowed(SCHEMA),
            Self {
                base: sqlite3_vtab::default(),
                slot,
                cache: Arc::default(),
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        use IndexConstraintOp as Op;
        let (mut stage, mut complement, mut member, mut file) = (None, None, None, None);
        let (mut id_eq, mut anc) = (None, None);
        let (mut src, mut srcs, mut kinds, mut shape) = (None, None, None, None);
        let (mut lo, mut hi) = (None, None);
        for (index, constraint) in info.constraints().enumerate() {
            if !constraint.is_usable() {
                continue;
            }
            match (constraint.column(), constraint.operator()) {
                (COL_STAGE, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    stage.get_or_insert(index);
                }
                (COL_COMPLEMENT, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    complement.get_or_insert(index);
                }
                (COL_MEMBER, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    member.get_or_insert(index);
                }
                (COL_FILE, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    file.get_or_insert(index);
                }
                (COL_ID, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    id_eq.get_or_insert(index);
                }
                (COL_ID, Op::SQLITE_INDEX_CONSTRAINT_GT) => {
                    lo.get_or_insert((index, Used::IdGt));
                }
                (COL_ID, Op::SQLITE_INDEX_CONSTRAINT_GE) => {
                    lo.get_or_insert((index, Used::IdGe));
                }
                (COL_ID, Op::SQLITE_INDEX_CONSTRAINT_LT) => {
                    hi.get_or_insert((index, Used::IdLt));
                }
                (COL_ID, Op::SQLITE_INDEX_CONSTRAINT_LE) => {
                    hi.get_or_insert((index, Used::IdLe));
                }
                (COL_ANC, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    anc.get_or_insert(index);
                }
                (COL_SRC, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    src.get_or_insert(index);
                }
                (COL_SRCS, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    srcs.get_or_insert(index);
                }
                (COL_KINDS, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    kinds.get_or_insert(index);
                }
                (COL_SHAPE, Op::SQLITE_INDEX_CONSTRAINT_EQ) => {
                    shape.get_or_insert(index);
                }
                _ => {}
            }
        }
        let reject = |info: &mut IndexInfo| {
            info.set_estimated_cost(f64::MAX);
            info.set_estimated_rows(i64::MAX);
            info.set_idx_str("");
            Ok(true)
        };
        // A set must come whole in one `xFilter` (an `IN` list), with its stage and whether it is
        // the stage's complement.
        let whole = match member {
            Some(index) => info.is_in_constraint(index)? && info.set_in_constraint(index, true)?,
            None => false,
        };
        let set = match (stage, complement, member, whole) {
            (Some(stage), Some(complement), Some(member), true) => Some([
                (stage, Used::Stage),
                (complement, Used::Complement),
                (member, Used::Member),
            ]),
            _ => None,
        };
        // A descendant walk: the input's nodes whole, one row's node and file, and the kinds; a
        // set where it has a stop.
        if let (Some(srcs), Some(kinds)) = (srcs, kinds) {
            let nodes = info.is_in_constraint(srcs)? && info.set_in_constraint(srcs, true)?;
            let stopped = stage.is_some() || complement.is_some() || member.is_some();
            let (Some(file), Some(src), true, false) = (file, src, nodes, stopped && set.is_none())
            else {
                return reject(info);
            };
            let mut used = vec![
                (file, Used::File),
                (src, Used::Src),
                (srcs, Used::Srcs),
                (kinds, Used::Kinds),
            ];
            used.extend(set.into_iter().flatten());
            used.extend(shape.map(|shape| (shape, Used::Shape)));
            return Ok(Self::consume(info, &used, 10.0));
        }
        // A lookup in one file.
        let (Some(set), Some(file)) = (set, file) else {
            return reject(info);
        };
        let mut used = set.to_vec();
        used.push((file, Used::File));
        let cost = if let Some(index) = id_eq {
            used.push((index, Used::IdEq));
            1.0
        } else {
            let cost = if let Some(index) = anc {
                used.push((index, Used::Anc));
                10.0
            } else {
                1_000_000.0
            };
            used.extend(lo);
            used.extend(hi);
            cost
        };
        Ok(Self::consume(info, &used, cost))
    }

    fn open(&'vtab mut self) -> Result<NearestCursor<'vtab>> {
        Ok(NearestCursor {
            base: sqlite3_vtab_cursor::default(),
            slot: Arc::clone(&self.slot),
            cache: Arc::clone(&self.cache),
            file: 0,
            stage: 0,
            columns: None,
            batch: None,
            sub: None,
            source: Source::Done,
            current: None,
            walking: false,
            walked: Walked::default(),
            src: 0,
            phantom: PhantomData,
        })
    }
}

impl NearestTable {
    /// Passes the constraints `used`, in argv order, and costs the plan at `cost`.
    fn consume(info: &mut IndexInfo, used: &[(usize, Used)], cost: f64) -> bool {
        let mut codes = String::new();
        for (argv, (index, use_)) in used.iter().enumerate() {
            let mut usage = info.constraint_usage(*index);
            usage.set_argv_index(argv as c_int + 1);
            usage.set_omit(true);
            codes.push_str(use_.code());
        }
        info.set_idx_str(&codes);
        info.set_estimated_cost(cost);
        info.set_estimated_rows(cost as i64);
        true
    }
}

/// The ids a scan yields, ascending.
enum Source {
    /// Positions in the file's `ids` (one `anc` group).
    Group {
        next: usize,
        end: usize,
    },
    Range {
        next: u32,
        end: u32,
    },
    Done,
}

#[repr(C)]
pub(super) struct NearestCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    slot: Arc<BatchSlot>,
    cache: Arc<parking_lot::Mutex<Cache>>,
    file: u32,
    stage: i64,
    /// `file`'s columns for the set; `None` in a walk without one.
    columns: Option<Arc<FileNearest>>,
    /// The batch and parsed tree `file` is read from, for `last`.
    batch: Option<Arc<Batch>>,
    sub: Option<Arc<SubFile>>,
    source: Source,
    current: Option<u32>,
    /// Whether the filter is a descendant walk's (`Used::Src`), its state, and the current row's
    /// input node.
    walking: bool,
    walked: Walked,
    src: u32,
    phantom: PhantomData<&'vtab NearestTable>,
}

impl Cache {
    /// Empties the cache where it holds another batch's sets.
    fn reset(&mut self, batch: &Arc<Batch>) {
        let current = Arc::downgrade(batch);
        if !self.batch.ptr_eq(&current) {
            self.batch = current;
            self.stages.clear();
        }
    }

    /// Reads `stage`'s rows from the `IN` list at `argv`, unless they were read for the batch.
    fn read(&mut self, stage: i64, args: &Filters<'_>, argv: usize) -> Result<()> {
        if let std::collections::hash_map::Entry::Vacant(slot) = self.stages.entry(stage) {
            let mut listed: HashMap<u32, Vec<u32>> = HashMap::new();
            let mut items = args.in_values(argv)?;
            while let Some(item) = items.next()? {
                if let Some(key) = integer(item).filter(|&key| key >= 0) {
                    listed
                        .entry((key >> 32) as u32)
                        .or_default()
                        .push((key & 0xffff_ffff) as u32);
                }
            }
            for nodes in listed.values_mut() {
                nodes.sort_unstable();
                nodes.dedup();
            }
            slot.insert(StageSet {
                listed,
                files: HashMap::new(),
            });
        }
        Ok(())
    }

    /// Document `file`'s columns for `stage`'s set (read by `read`), or its complement.
    fn columns(
        &mut self,
        batch: &Batch,
        sub: &Option<Arc<SubFile>>,
        stage: i64,
        file: u32,
        complement: bool,
    ) -> Arc<FileNearest> {
        // A statement's filters all read the batch it runs on, so the set its filter read stays.
        let set = self.stages.get_mut(&stage).expect("read by the filter");
        match set.files.get(&(file, complement)) {
            Some(columns) => Arc::clone(columns),
            None => {
                let facts = facts_of(batch, sub, file);
                let listed = set.listed.get(&file).map_or(&[][..], Vec::as_slice);
                let columns = Arc::new(FileNearest::of(facts, listed, complement));
                set.files.insert((file, complement), Arc::clone(&columns));
                columns
            }
        }
    }
}

impl NearestCursor<'_> {
    fn advance(&mut self) {
        if self.walking {
            let walked = &mut self.walked;
            self.current = (walked.next < walked.end).then(|| {
                walked.next += 1;
                walked.ids[walked.next - 1]
            });
            return;
        }
        let columns = self.columns.as_ref();
        self.current = match &mut self.source {
            Source::Group { next, end } if *next < *end => {
                *next += 1;
                columns.map(|columns| columns.ids[*next - 1])
            }
            Source::Range { next, end } if *next < *end => {
                *next += 1;
                Some(*next - 1)
            }
            _ => None,
        };
    }

    /// A descendant walk's filter for one input row (`file`, `src`): its node's rows, listed for
    /// the whole input by the statement's first filter (`list`).
    fn walk(&mut self, batch: Arc<Batch>, codes: &str, args: &Filters<'_>) -> Result<()> {
        if !self.walked.listed {
            self.list(&batch, codes, args)?;
        }
        self.walking = true;
        self.columns = None;
        let value = |argv: usize| -> Result<Option<u32>> {
            Ok(integer(ValueRef::from(&args.get::<Value>(argv)?))
                .and_then(|value| u32::try_from(value).ok()))
        };
        let walked = &mut self.walked;
        (walked.next, walked.end) = (0, 0);
        if let (Some(file), Some(src)) = (value(0)?, value(1)?) {
            let key = u64::from(file) << 32 | u64::from(src);
            if let Ok(at) = walked.inputs.binary_search(&key) {
                walked.next = walked.starts[at] as usize;
                walked.end = walked.starts[at + 1] as usize;
            }
            self.file = file;
            self.src = src;
            self.sub = batch.subfile(file as usize);
        }
        self.batch = Some(batch);
        self.advance();
        Ok(())
    }

    /// Lists every input node's rows: the input's nodes, kinds, set and shape, in `codes`' order
    /// after the row's `file` and `src`.
    fn list(&mut self, batch: &Arc<Batch>, codes: &str, args: &Filters<'_>) -> Result<()> {
        let bad = || Error::ModuleError(format!("nearest: bad walk {codes}"));
        let text = |argv: usize| -> Result<Option<String>> {
            Ok(match args.get::<Value>(argv)? {
                Value::Text(text) => Some(text),
                _ => None,
            })
        };
        let value = |argv: usize| -> Result<Option<i64>> {
            Ok(integer(ValueRef::from(&args.get::<Value>(argv)?)))
        };
        let walked = &mut self.walked;
        walked.inputs.clear();
        walked.starts.clear();
        walked.ids.clear();
        walked.probes.clear();
        let (mut stage, mut complement, mut member) = (None, false, None);
        for (argv, code) in codes.as_bytes().chunks(2).enumerate() {
            match Used::parse(code).ok_or_else(bad)? {
                Used::File | Used::Src => {}
                Used::Srcs => {
                    let mut items = args.in_values(argv)?;
                    while let Some(item) = items.next()? {
                        if let Some(key) = integer(item).filter(|&key| key >= 0) {
                            walked.inputs.push(key as u64);
                        }
                    }
                }
                Used::Kinds => {
                    text(argv)?
                        .and_then(|kinds| kinds_in(&kinds, &mut walked.kinds))
                        .ok_or_else(bad)?;
                }
                Used::Shape => {
                    let probes = text(argv)?.ok_or_else(bad)?;
                    for probe in probes.split(';').filter(|probe| !probe.is_empty()) {
                        walked.probes.push(Probe::parse(probe).ok_or_else(bad)?);
                    }
                }
                Used::Stage => stage = Some(value(argv)?.ok_or_else(bad)?),
                Used::Complement => complement = value(argv)?.ok_or_else(bad)? != 0,
                Used::Member => member = Some(argv),
                _ => return Err(bad()),
            }
        }
        let stop = match (stage, member) {
            (Some(stage), Some(member)) => {
                let mut cache = self.cache.lock();
                cache.reset(batch);
                cache.read(stage, args, member)?;
                self.stage = stage;
                Some((stage, complement))
            }
            (None, None) => None,
            _ => return Err(bad()),
        };
        walked.inputs.sort_unstable();
        walked.inputs.dedup();
        let mut at = 0;
        while let Some(&key) = walked.inputs.get(at) {
            let file = (key >> 32) as u32;
            let end = at + walked.inputs[at..].partition_point(|&key| (key >> 32) as u32 == file);
            if (file as usize) < batch.documents() {
                let sub = batch.subfile(file as usize);
                let columns = stop.map(|(stage, complement)| {
                    self.cache
                        .lock()
                        .columns(batch, &sub, stage, file, complement)
                });
                walked.visit(facts_of(batch, &sub, file), columns.as_deref(), at..end);
            } else {
                let listed = walked.ids.len() as u32;
                walked.starts.extend((at..end).map(|_| listed));
            }
            at = end;
        }
        walked.starts.push(walked.ids.len() as u32);
        walked.listed = true;
        Ok(())
    }
}

unsafe impl VTabCursor for NearestCursor<'_> {
    fn filter(&mut self, _: c_int, idx_str: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let batch = self
            .slot
            .current()
            .ok_or_else(|| Error::ModuleError("nearest: no batch installed".into()))?;
        let codes = idx_str.unwrap_or("");
        if codes.starts_with("f=b=n*") {
            return self.walk(batch, codes, args);
        }
        self.walking = false;
        if !codes.starts_with("s=c=m*f=") {
            return Err(Error::ModuleError(
                "nearest requires a stage, whether it is complemented, its rows and a file".into(),
            ));
        }
        let value = |argv: usize| -> Result<Option<i64>> {
            Ok(integer(ValueRef::from(&args.get::<Value>(argv)?)))
        };
        let stage = value(0)?
            .ok_or_else(|| Error::ModuleError("nearest: stage must be an integer".into()))?;
        let complement = value(1)?
            .ok_or_else(|| Error::ModuleError("nearest: complement must be an integer".into()))?
            != 0;
        let mut cache = self.cache.lock();
        cache.reset(&batch);
        cache.read(stage, args, 2)?;
        self.stage = stage;
        self.columns = None;
        self.source = Source::Done;
        self.current = None;
        let Some(file) = value(3)?
            .and_then(|file| u32::try_from(file).ok())
            .filter(|&file| (file as usize) < batch.documents())
        else {
            return Ok(());
        };
        let sub = batch.subfile(file as usize);
        let columns = cache.columns(&batch, &sub, stage, file, complement);
        drop(cache);
        let len = columns.len();
        let (mut lo, mut hi) = (0i64, i64::from(len) - 1);
        let mut id_eq = None;
        let mut ancestor = None;
        for (argv, code) in codes.as_bytes().chunks(2).enumerate().skip(4) {
            let used = Used::parse(code)
                .ok_or_else(|| Error::ModuleError(format!("nearest: bad plan {codes}")))?;
            let Some(value) = value(argv)? else {
                // NULL equals and orders against nothing.
                self.file = file;
                return Ok(());
            };
            match used {
                Used::IdEq => id_eq = Some(value),
                Used::IdGt => lo = lo.max(value.saturating_add(1)),
                Used::IdGe => lo = lo.max(value),
                Used::IdLt => hi = hi.min(value.saturating_sub(1)),
                Used::IdLe => hi = hi.min(value),
                Used::Anc => ancestor = Some(value),
                Used::Stage
                | Used::Complement
                | Used::Member
                | Used::File
                | Used::Src
                | Used::Srcs
                | Used::Kinds
                | Used::Shape => {
                    unreachable!("the first four, or a walk's")
                }
            }
        }
        if let Some(id) = id_eq {
            lo = lo.max(id);
            hi = hi.min(id);
        }
        self.file = file;
        if lo <= hi {
            let (lo, hi) = (lo.max(0) as u32, hi.min(i64::from(u32::MAX - 1)) as u32);
            self.source = match ancestor {
                Some(ancestor) => {
                    let (next, end) = columns.group(ancestor, lo, hi);
                    Source::Group { next, end }
                }
                None => Source::Range {
                    next: lo,
                    end: hi + 1,
                },
            };
        }
        self.columns = Some(columns);
        self.batch = Some(batch);
        self.sub = sub;
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
        let id = self.current.expect("a row");
        let columns = self.columns.as_deref();
        let index = id as usize;
        let node = |value: u32| (value != NONE).then_some(i64::from(value));
        let or_none = |value: u32| node(value).unwrap_or(-1);
        match column {
            COL_FILE => context.set_result(&i64::from(self.file)),
            COL_ID => context.set_result(&i64::from(id)),
            COL_LAST => {
                let batch = self.batch.as_ref().expect("a batch");
                let last = facts_of(batch, &self.sub, self.file).last(id);
                context.set_result(&i64::from(last))
            }
            COL_ANC => context.set_result(&columns.map(|columns| or_none(columns.anc[index]))),
            COL_BASE => context.set_result(&columns.map(|columns| {
                if columns.rank[index] != NONE {
                    i64::from(id)
                } else {
                    or_none(columns.anc[index])
                }
            })),
            COL_NXT => context.set_result(&columns.and_then(|columns| node(columns.nxt[index]))),
            COL_PRV => context.set_result(&columns.and_then(|columns| node(columns.prv[index]))),
            COL_STAGE => context.set_result(&self.stage),
            COL_SRC => context.set_result(&self.walking.then_some(i64::from(self.src))),
            _ => context.set_result(&None::<i64>),
        }
    }

    fn rowid(&self) -> Result<i64> {
        let id = self.current.expect("a row");
        Ok((i64::from(self.file) << 32) | i64::from(id))
    }
}
