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
//! Keeping a set for the batch is exact because a stage's rows are fixed once the stage ran:
//! `Session::run_batch` runs every stage before any clause that reads it (a stage with one
//! consumer in that clause's run, before its statements after the root frame), a stage reads
//! only stages made before it, and every batch is a fresh `Batch` (the cache is keyed by its
//! identity). So the member list is read once per stage and batch; later statements' lists are
//! not read.

use super::exec::{Batch, BatchSlot, SubFile};
use super::nodes::{NONE, NodeFacts, facts_of, integer};
use super::plan::NodeSet;
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

const SCHEMA: &CStr = c"CREATE TABLE x(file, id, \"last\", anc, base, nxt, prv, stage HIDDEN, complement HIDDEN, member HIDDEN)";

/// The SQL naming `set` as the set of a `nearest` table aliased `alias`, in file `file` (an SQL
/// expression): the constraints to put in its join. A complement passes its stage's rows, which
/// the table leaves out.
pub(super) fn set_of(alias: &str, set: NodeSet, file: &str) -> String {
    format!(
        "{alias}.stage = {} AND {alias}.complement = {} AND {alias}.member IN (SELECT -1 UNION ALL SELECT file * 4294967296 + node FROM {}) AND {alias}.file = {file}",
        set.stage,
        i64::from(set.complement),
        super::emit::stage_table(set.stage)
    )
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
                _ => {}
            }
        }
        // The set must come whole in one `xFilter` (an `IN` list), with its stage, whether it is
        // the stage's complement, and one file.
        let whole = match member {
            Some(index) => info.is_in_constraint(index)? && info.set_in_constraint(index, true)?,
            None => false,
        };
        let (Some(stage), Some(complement), Some(member), Some(file), true) =
            (stage, complement, member, file, whole)
        else {
            info.set_estimated_cost(f64::MAX);
            info.set_estimated_rows(i64::MAX);
            info.set_idx_str("");
            return Ok(true);
        };
        let mut used = vec![
            (stage, Used::Stage),
            (complement, Used::Complement),
            (member, Used::Member),
            (file, Used::File),
        ];
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
        Ok(true)
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
            phantom: PhantomData,
        })
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
    columns: Option<Arc<FileNearest>>,
    /// The batch and parsed tree `file` is read from, for `last`.
    batch: Option<Arc<Batch>>,
    sub: Option<Arc<SubFile>>,
    source: Source,
    current: Option<u32>,
    phantom: PhantomData<&'vtab NearestTable>,
}

impl NearestCursor<'_> {
    fn advance(&mut self) {
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
}

unsafe impl VTabCursor for NearestCursor<'_> {
    fn filter(&mut self, _: c_int, idx_str: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let batch = self
            .slot
            .current()
            .ok_or_else(|| Error::ModuleError("nearest: no batch installed".into()))?;
        let codes = idx_str.unwrap_or("");
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
        let current = Arc::downgrade(&batch);
        if !cache.batch.ptr_eq(&current) {
            cache.batch = current;
            cache.stages.clear();
        }
        if let std::collections::hash_map::Entry::Vacant(slot) = cache.stages.entry(stage) {
            let mut listed: HashMap<u32, Vec<u32>> = HashMap::new();
            let mut items = args.in_values(2)?;
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
        let set = cache.stages.get_mut(&stage).expect("read above");
        let sub = batch.subfile(file as usize);
        let columns = match set.files.get(&(file, complement)) {
            Some(columns) => Arc::clone(columns),
            None => {
                let facts = facts_of(&batch, &sub, file);
                let listed = set.listed.get(&file).map_or(&[][..], Vec::as_slice);
                let columns = Arc::new(FileNearest::of(facts, listed, complement));
                set.files.insert((file, complement), Arc::clone(&columns));
                columns
            }
        };
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
                Used::Stage | Used::Complement | Used::Member | Used::File => {
                    unreachable!("the first four")
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
        let columns = self.columns.as_ref().expect("a file");
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
            COL_ANC => context.set_result(&or_none(columns.anc[index])),
            COL_BASE => context.set_result(&if columns.rank[index] != NONE {
                i64::from(id)
            } else {
                or_none(columns.anc[index])
            }),
            COL_NXT => context.set_result(&node(columns.nxt[index])),
            COL_PRV => context.set_result(&node(columns.prv[index])),
            COL_STAGE => context.set_result(&self.stage),
            _ => context.set_result(&None::<i64>),
        }
    }

    fn rowid(&self) -> Result<i64> {
        let id = self.current.expect("a row");
        Ok((i64::from(self.file) << 32) | i64::from(id))
    }
}
