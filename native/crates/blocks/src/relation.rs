//! A relation: its rows, its kills, and, once frozen, everything reads need, in one descriptor.
//!
//! **Freeze** turns rows written in any order (by many threads, or a GPU's claims) into a relation read in a
//! fixed order:
//!
//! - **Set semantics.** It drops duplicate rows, and in a delta it also drops rows its base still holds live. Both
//!   are found by hashing whole rows: a table of them in scratch for its own rows, and the base's row index for the
//!   base's (decision 0678 step 8). A row the delta killed and pushed again is the base's row brought back.
//! - **One sort column, or none.** The live rows, sorted on the relation's sort column when it has one (ties in
//!   the order they came), else left in that order, moved into it in place: an own row's id is then its place, so
//!   there is no order array, and no dead bits for its own rows (the duplicates are gone, not marked). Rows that
//!   came in order on another column (a join's output, in its probe side's order) are left so, and that column is
//!   the one recorded (`Relation::sort_col`); a delta's rows are sorted on its base's.
//! - **Hash indexes** on the declared keys, and a **row index** on every column (when it has at most `MAX_KEY`):
//!   each key's rows grouped in id order, so a probe's answer comes in a fixed order too.
//! - **Dead bits and statistics.** The dead bits are a delta's kills of its base's rows (none in a full
//!   relation); the statistics are each column's minimum and maximum.
//!
//! **A delta** is a relation whose ids continue after its base's (`base_rows`). Its kills name base rows or its
//! own. `View` reads a base and its delta as one relation: liveness from both, the two merged on the sort column
//! (the base's row first on a tie), and probes of both indexes merged.

use crate::block::Block;
use rerail_delta_core::flat::RelSpan;
use crate::rows::{Rows, RowsW};
use crate::key_hash;
// The relation's format and its reads: the floor's one definition (`rerail_delta_core::relation`).
pub use rerail_delta_core::hash::row_digest;
pub use rerail_delta_core::relation::leading;
pub use rerail_delta_core::relation::{
    ADDED, ADDED_INDEXES, ALIASED, CONTINUES, ColStats, DEAD, DELTA, FAR_BASE, FILTER, FILTER_BITS, FROZEN, HASHED, Index, IndexRoom, Known, MARK_REACH,
    MARK_SAME, MAX_KEY, NO_BLOCK, NODE_BITS, ORDER, ORDER_ROWS, REWRITTEN, Relation, fill_marks, index_rows_bytes, mark_tag,
};
use std::cmp::Ordering;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::*};

pub mod layer_write;

/// A chunk of a relation's block an order is given (`Relation::try_add_order`): its offset, its items, and the bytes
/// its chunk keeps (0: one it does not own, given back by its owner).
#[derive(Clone, Copy, Debug, Default)]
pub struct Given {
    pub at: usize,
    pub len: usize,
    pub kept: usize,
}

/// An order's rows: listed, a chunk of the block (`ORDER`), or the rows `lo..hi` themselves, already in the key's
/// order (`ORDER_ROWS`).
#[derive(Clone, Copy, Debug)]
pub enum OrderRows {
    Listed(Given),
    Range(u32, u32),
}

/// An order given to a relation's directory (`Relation::try_add_order`): its rows, and its fences, tags and prints,
/// each a chunk of the block written already.
#[derive(Clone, Copy, Debug)]
pub struct GivenOrder {
    pub rows: OrderRows,
    pub fences: Given,
    pub tags: Given,
    pub prints: Given,
}

/// What a relation is: its columns' widths, and what freeze builds for its readers.
#[derive(Clone, Debug)]
pub struct Spec {
    pub widths: Vec<u32>,
    /// The one column its rows are sorted on (decision 0678 step 8: zero or one): ties keep the order the rows came
    /// in. `None`: they stay in that order. Rows that come in order on another column stay in it, and the relation
    /// records that column (`Relation::sort_col`); a delta lies in its base's order. Rows are made distinct by their
    /// row index, never by a sort.
    pub sort: Option<u16>,
    /// The keys probes ask by.
    pub indexes: Vec<Vec<u16>>,
    /// The first chunk holds `2^min_log` rows.
    pub min_log: u32,
}

/// A relation's columns as it is made with them, borrowed (a `Spec`'s, or a layer's fixed `layer::LayerSpec`): their
/// widths, its sort column, and the chunk size its rows start at. What making a relation reads of a spec.
#[derive(Clone, Copy, Debug)]
pub struct Cols<'a> {
    pub widths: &'a [u32],
    pub sort: Option<u16>,
    pub min_log: u32,
}

impl Spec {
    pub fn cols(&self) -> Cols<'_> {
        Cols { widths: &self.widths, sort: self.sort, min_log: self.min_log }
    }
}

/// `RelationW::try_create` of `cols`: the descriptor and both row sets' column directories, in one chunk of `block`.
pub fn try_create_cols<'b>(block: &'b Block, cols: Cols<'_>, base: Option<&Relation>) -> Option<&'b Relation> {
    // A chunk as it lies (in a dirty block too: nothing zeroes it as it is handed out), the descriptor's clear state
    // written over it whole, then its fields.
    let bytes = Relation::descriptor_bytes(cols.widths.len());
    let at = block.try_alloc_chunk_any(bytes, 64)?;
    let p: *mut Relation = block.ptr(at);
    let dirs: *mut crate::rows::ColDir = block.ptr(at + Relation::DIRS_AT);
    unsafe {
        block.ptr::<u8>(at).write_bytes(0, bytes);
        (*p).arity = cols.widths.len() as u32;
        (*p).flags.store(if base.is_some() { DELTA } else { 0 }, Relaxed);
        (*p).base_rows = base.map_or(0, |b| {
            // A continuation's delta (its kills, which read no rows through it) counts its base's rows too.
            if b.continues() {
                return b.base_rows + b.rows.len();
            }
            assert_eq!(b.base_rows, 0, "a delta's base is a full relation");
            b.rows.len()
        });
        // A delta lies in its base's order, so a view merges the two on one column.
        (*p).set_sort_col(match base {
            Some(b) => b.sort_col(),
            None => cols.sort.map(usize::from),
        });
        Rows::init_in(&raw mut (*p).rows, &block.region(), dirs, cols.widths, cols.min_log);
        Rows::init_in(&raw mut (*p).killed, &block.region(), dirs.add(cols.widths.len()), &[4], 6);
        (*p).base_block = NO_BLOCK;
        Some(&*p)
    }
}

/// `RelationW::try_continue_far` of `cols`.
pub fn try_continue_far_cols<'b>(block: &'b Block, cols: Cols<'_>, below: Option<(u32, usize, &Relation, u32)>) -> Option<&'b Relation> {
    let rel = try_create_cols(block, cols, None)?;
    let Some((id, at, b, end)) = below else { return Some(rel) };
    if !b.is_frozen() || id == NO_BLOCK || at == 0 || block.holds(std::ptr::from_ref(b)) || end < b.len_all() {
        // SAFETY: just made in `block`; no one has it.
        unsafe { rel.release(block) };
        return None;
    }
    // SAFETY: just made, this caller's alone.
    unsafe {
        let me = std::ptr::from_ref(rel).cast_mut();
        (*me).base_rows = end;
        (*me).base_block = id;
    }
    rel.base_off.store(at as i64, Release);
    rel.flags.fetch_or(DELTA | CONTINUES | FAR_BASE, Release);
    Some(rel)
}

/// A relation's writes that take room from a block, a pin, or the heap: blocks' own, over the floor's `Relation`.
pub trait RelationW {
    /// Makes an empty relation in `block`: a delta of `base` when given. Returns its descriptor.
    fn create<'b>(block: &'b Block, spec: &Spec, base: Option<&Relation>) -> &'b Relation;

    /// `create`, or `None` when the block cannot hold the relation's descriptor and directories (what it took of
    /// the block then stays unused).
    fn try_create<'b>(block: &'b Block, spec: &Spec, base: Option<&Relation>) -> Option<&'b Relation>;

    /// A relation that continues the first `base_rows` rows `base` reads (decision 0384 step 7): its ids below them
    /// are `base`'s, read there, and its own rows come after (a second relation started from rows whose writer
    /// another took, `Rows::shared`). `base` is a full relation, or a continuation itself (a fork of a fork, step 8),
    /// whose own rows it then reads past its base's: ids are read down the chain (`rel_get`). It lies in `base`'s
    /// block, which every reader binding it binds. `None` when the block has no room, or `base` is neither (a delta of
    /// kills), or `base_rows` is not within the rows `base` reads past its own base's.
    fn try_continue<'b>(block: &'b Block, spec: &Spec, base: &Relation, base_rows: u32) -> Option<&'b Relation>;

    /// A relation of `block` that reads a relation of several parts as one, nothing copied (decision 0353 step 6):
    /// rows `0..base_rows` its base's, read down the base's chain (`base`: the base and the block it lies in, this
    /// one or another, `FAR_BASE`; none for a first part, `base_rows` 0), then `rows_of`'s first `len` own rows, where
    /// they lie in `block` (`ALIASED`). `None` when `block` cannot hold its descriptor, or `rows_of` is not `block`'s.
    fn try_link<'b>(block: &'b Block, spec: &Spec, base: Option<(&Block, &Relation)>, base_rows: u32, rows_of: &Relation, len: u32) -> Option<&'b Relation>;

    /// A trace's layer (decision 0686 B2; P§12: "a layer is one `blocks::relation::Relation`"): an empty relation
    /// of `block` that continues `below`, the layer under it, which lies in another block, frozen: `below` is that
    /// block's id, its descriptor's offset there, and the descriptor, read through the pin of that block. Its ids
    /// follow `below`'s (`base_rows`: every id `below` reads), and the reference is by block id and offset
    /// (`base_block`, `base_off`), which a reader resolves through its launch's descriptor table. `None` for the
    /// first layer of a trace (a relation of its own), when `block` has no room, or when `below` is not frozen.
    /// `below`'s `end` is the first id past every id it reads: its own rows' end, wherever they lie (a layer's rows
    /// may lie encoded beside its relation, `layer::OwnRows`).
    fn try_continue_far<'b>(block: &'b Block, spec: &Spec, below: Option<(u32, usize, &Relation, u32)>) -> Option<&'b Relation>;

    /// The relation whose descriptor lies at `at` of `block` (an offset its maker recorded): `None` past the block's
    /// reservation.
    fn in_block(block: &Block, at: usize) -> Option<&Relation>;

    /// Gives a layer frozen just now a sorted order on `cols` (P§11.2's `Order`; decision 0478 step 5b's form),
    /// through the write pin of its block: `ids`, its own ids in the key's order, and beside them `fences` (the
    /// leading value of every `order::FENCE`th) and `prints` (each one's print of its whole key), each written into a
    /// chunk of the block as the order keeps it. Its number in the directory; `None` when the block or the directory
    /// has no room.
    fn add_order(&self, pin: &mut crate::region::BlockPin, cols: &[u16], ids: &[u32], fences: &[i64], prints: &[u8]) -> Option<usize>;

    /// Gives an open relation `n` own rows, to be written (`put_own`), and returns the first. `pin` pins `block`,
    /// the relation's, to write: no other writer of it can be live. False when the block has no room.
    fn claim_own(&self, pin: &mut crate::region::BlockPin, n: u32) -> Option<u32>;

    /// Writes cell `c` of own row `i` (claimed, `claim_own`) of an open relation, through the write pin of its block.
    fn put_own(&self, pin: &mut crate::region::BlockPin, i: u32, c: usize, v: i64);

    /// `get`, a base in another block (`FAR_BASE`) read in `far`, the block it lies in (`rel_get`'s `far`).
    fn get_far(&self, far: Option<&Block>, id: u32, c: usize) -> i64;

    /// A delta of `base` with no rows of its own whose dead bits are `words` (row `id` dead when bit `id` of them is
    /// set): a relation's kills of the rows it shares, read beside them where they lie (decision 0325: the device
    /// skips a dead row by its bit, `rel_dead`). `None` when the block cannot hold it.
    /// `try_kills`, its dead bits the ones `base` keeps (`own_dead`), read where they lie: nothing copied.
    fn try_kills_own<'b>(block: &'b Block, spec: &Spec, base: &Relation) -> Option<&'b Relation>;

    /// Its dead bits are a chunk of their own, so a delta replaced when they change goes back whole
    /// (`release_kills`).
    fn try_kills<'b>(block: &'b Block, spec: &Spec, base: &Relation, words: &[u64]) -> Option<&'b Relation>;

    /// Gives a kills delta (`try_kills`) back to the block's free list: its dead bits' chunk, then itself
    /// (`release`). Returns the bytes given to the OS with them.
    ///
    /// # Safety
    /// It is a delta `try_kills` made in `block`, and no one reads it after (its last holder's).
    unsafe fn release_kills(&self, block: &Block) -> usize;

    /// Gives its chunks back to the block's free list (decision 0342), when the block keeps one. Returns the bytes
    /// given to the OS with them (`Block::free_chunk`).
    ///
    /// # Safety
    /// It lies in `block`, and no one reads or writes it after (its last holder's).
    unsafe fn release(&self, block: &Block) -> usize;

    /// Takes its dead rows' bits, `len` words in the chunk at `at` of `block` (`kept` bytes), written: kept from now on,
    /// given back at its release (decision 0478 step 5d). False when it has some already.
    ///
    /// # Safety
    /// It lies in `block`; the chunk is the caller's to give and nothing writes it after; nothing else writes its
    /// descriptor meanwhile.
    unsafe fn adopt_dead(&self, block: &Block, at: usize, len: usize, kept: usize) -> bool;

    /// Writes its known ranges (decision 0478 step 5c), once: `(lo, hi, cols)` each, in row order. False when they
    /// were written already, a key is past `MAX_KEY`, or `block` has no room.
    ///
    /// # Safety
    /// It lies in `block`, and nothing else writes its descriptor meanwhile (its writer's, at freeze).
    unsafe fn try_set_known(&self, block: &Block, ranges: &[(u32, u32, &[u16])]) -> bool;

    /// The number of this relation's next directory entry, its directory made first where it has none
    /// (`ADDED_INDEXES` entries, a chunk of `block`); `None` when the directory is full or `block` has no room.
    ///
    /// # Safety
    /// As `try_reserve_index`'s: this relation's descriptor is the caller's to change.
    unsafe fn next_entry(&self, block: &Block) -> Option<usize>;

    /// Gives back index `room`, reserved and not marked built (`index_built`), or built and no longer read: its buckets
    /// and rows go back to the block's free list, and its entry to the directory, for the next reservation. In a block
    /// of no free list (whose chunks given back are never taken again) the entry keeps them, for the next index that
    /// fits them (`dead_room`). For an index a batch built that another built meanwhile (both found none, each in its
    /// own batch), and one whose builder failed.
    ///
    /// # Safety
    /// `room` is this relation's, in `block`; nothing reads it now or after (its builder is done), and no other thread
    /// adds an index to this relation meanwhile (its owner's lock).
    unsafe fn give_back_index(&self, block: &Block, room: &IndexRoom);

    /// Gives this relation a sorted order on `cols` (decision 0478 step 5b): `order`'s rows, and its fences, tags
    /// and prints, each a chunk of `block` written already. The relation keeps them from now on and gives them back
    /// when its indexes are forgotten. Its number; `None` when its directory has no room (the chunks are then still
    /// the caller's).
    ///
    /// # Safety
    /// As `try_reserve_index`'s; each chunk lies in `block` and is the caller's to give, written, and nothing writes
    /// it after.
    unsafe fn try_add_order(&self, block: &Block, cols: &[u16], order: GivenOrder) -> Option<usize>;

    /// Gives this relation a node-keyed index on column `col` of its rows `0..n` (decision 0694 step 1), laid out as
    /// a node's id is (`packed::ID_SPLIT`): its document's key (the high 32 bits), then its low 32 bits, a granule's
    /// slot above an index in the granule. Each document is held in the smaller of two forms:
    /// - **bits**: its slots from its least to its greatest, each a slot entry giving its bits, from its least
    ///   index's word to its greatest's. A test is the slot's entry and one bit.
    /// - **lows**: its distinct low halves, sorted. A test is a search of them.
    ///
    /// Either is at most a hashed index's bytes (lows: 4 a distinct value); a document takes bits when they are at
    /// most twice its lows' bytes. Its number; `None` when its directory is full or `block` has no room.
    ///
    /// Its `rows` (`u32`s): the documents' count; each document's key, its form's first field, its second, and where
    /// its form starts; then the forms. A document of bits has its least slot and its slots; their entries, two a
    /// slot, are its words' offset and its first word's number above its words' count (0 for a slot of none). A
    /// document of lows has `LOWS` and its lows' count, then its lows. Its `fences`: the words.
    ///
    /// # Safety
    /// As `try_reserve_index`'s: this relation's descriptor is the caller's to change, its rows `0..n` written.
    unsafe fn try_add_node_bits(&self, block: &Block, col: u16, n: u32) -> Option<usize>;

    /// Gives this relation a membership filter on `cols` of its rows `0..n` (decision 0706 E2): a blocked Bloom filter
    /// in `bloom`'s format, `FILTER_BITS` a row at least (its words a power of two), each row's key placed by
    /// `bloom::key` of its `cols`, in order. A probe whose key it rejects reads no row. Its number; `None` when its
    /// directory is full or `block` has no room.
    ///
    /// # Safety
    /// As `try_reserve_index`'s: this relation's descriptor is the caller's to change, its rows `0..n` written.
    unsafe fn try_add_filter(&self, block: &Block, cols: &[u16], n: u32) -> Option<usize>;

    /// Room for an index on `cols` of rows `0..n` in `block`, which holds this relation: its directory's entry, its
    /// `2^log` buckets (`n`'s power of two at least) and its rows, which a builder fills (`build_index`, or the
    /// device's), then marks built (`index_built`). Its number, its buckets' and rows' offsets in the block, and its
    /// mask; `None` when the block or the directory has no room.
    ///
    /// # Safety
    /// No other thread adds an index to this relation meanwhile (its owner's lock), and its rows `0..n` are written.
    unsafe fn try_reserve_index(&self, block: &Block, cols: &[u16], n: u32) -> Option<IndexRoom>;

    /// `try_reserve_index` of its rows `from..from + n` (`Index::from`): a recursive head's delta read in place.
    ///
    /// # Safety
    /// As `try_reserve_index`'s, its rows `from..from + n` written.
    unsafe fn try_reserve_index_from(&self, block: &Block, cols: &[u16], from: u32, n: u32) -> Option<IndexRoom>;

    /// `try_reserve_index` with room for `keys` distinct keys: `keys`' power of two buckets (at most `n`'s), about
    /// one key a bucket. A key of few values (a kind) is a few buckets, not one a row: its index is about 4 bytes a
    /// row and 4 a key.
    ///
    /// # Safety
    /// As `try_reserve_index`'s.
    unsafe fn try_reserve_index_buckets(&self, block: &Block, cols: &[u16], n: u32, keys: usize) -> Option<IndexRoom>;

    /// `try_reserve_index_from` of room for `n` rows with `buckets` buckets (a power of two), whatever its rows: a live
    /// relation's, whose rows grow past its first room within a program (`grow_index_rows`), its buckets as they are.
    ///
    /// # Safety
    /// As `try_reserve_index`'s, its rows `from..` written as the device claims them.
    unsafe fn try_reserve_index_live(&self, block: &Block, cols: &[u16], from: u32, n: u32, buckets: usize) -> Option<IndexRoom>;

    /// `try_reserve_index_buckets` of `cap` buckets (a power of two).
    ///
    /// # Safety
    /// As `try_reserve_index`'s.
    unsafe fn try_reserve_index_cap(&self, block: &Block, cols: &[u16], n: u32, cap: usize) -> Option<IndexRoom>;

    /// Index `index`'s rows given room for `n` rows (more than it has), its buckets as they are: a live relation's,
    /// whose rows a program of rounds maps as they reach them (decision 0603), between its launches. Its old rows' room
    /// goes back to the block as `forget_indexes` gives it. The new room's offset in the block, or `None` where the
    /// block has no room for it (the index as it was).
    ///
    /// # Safety
    /// The entry is the caller's, built by the device only, and nothing reads or writes it meanwhile.
    unsafe fn grow_index_rows(&self, block: &Block, index: usize, n: u32) -> Option<usize>;

    /// Fills index `room` on the host: each row's bucket by its key's hash, the buckets' starts, then the rows in id
    /// order within each bucket, as the device's builder places them.
    ///
    /// # Safety
    /// `room` is this relation's, reserved and not yet built, and nothing reads it meanwhile.
    unsafe fn build_index(&self, room: &IndexRoom);

    /// `build_index`, each row's key hash given (`hash(i)` for its row `i`: `SEED` mixed with its key's values, as
    /// `key_hash`): for a relation whose key values are read elsewhere (a view's home of no columns, decision 0478
    /// step 5e). Its keys are not compared: no mark says `MARK_SAME` (decision 0671).
    ///
    /// # Safety
    /// As `build_index`'s.
    unsafe fn build_index_hashed(&self, room: &IndexRoom, hash: &dyn Fn(u32) -> u64);

    /// `build_index` of each row's key hash (`hash(i)` of its `i`th row from its first) and whether two rows (by id)
    /// hold one key: each row's bucket, the buckets' starts, then the rows in id order within each bucket, as the
    /// device's builder places them, and their marks (decision 0671).
    ///
    /// # Safety
    /// As `build_index`'s.
    unsafe fn build_index_with(&self, room: &IndexRoom, hash: &dyn Fn(u32) -> u64, same: &dyn Fn(u32, u32) -> bool);

    /// Every index added forgotten, its buckets and rows given back to `block`'s free list (`try_reserve_index`'s
    /// chunks): for a relation whose rows' values change in place (a lattice's fold, decision 0384 step 5), whose
    /// indexes built before no longer hold. The next reader that probes a key builds its index again, in room the
    /// block takes from its free list.
    ///
    /// # Safety
    /// `block` holds this relation, whose indexes were all added (`try_reserve_index`, none of a built relation's);
    /// under its owner's index lock, and no reader probes an index of it meanwhile, or after.
    unsafe fn forget_indexes(&self, block: &Block);
}

impl RelationW for Relation {
    fn create<'b>(block: &'b Block, spec: &Spec, base: Option<&Relation>) -> &'b Relation {
        Relation::try_create(block, spec, base).unwrap_or_else(|| panic!("block full: a relation of {} columns", spec.widths.len()))
    }

    fn try_create<'b>(block: &'b Block, spec: &Spec, base: Option<&Relation>) -> Option<&'b Relation> {
        // The descriptor and both row sets' column directories, one chunk: given back whole (`release`).
        try_create_cols(block, spec.cols(), base)
    }

    fn try_continue<'b>(block: &'b Block, spec: &Spec, base: &Relation, base_rows: u32) -> Option<&'b Relation> {
        let full = base.base_rows == 0 && !base.is_delta();
        let from = if base.continues() { base.base_rows } else { 0 };
        if !(full || base.continues()) || base_rows < from || base_rows > base.len_all() || !block.holds(std::ptr::from_ref(base)) {
            return None;
        }
        let rel = Relation::try_create(block, spec, None)?;
        // SAFETY: just made, this caller's alone.
        unsafe {
            let me = std::ptr::from_ref(rel).cast_mut();
            (*me).base_rows = base_rows;
        }
        let off = std::ptr::from_ref(base) as i64 - std::ptr::from_ref(rel) as i64;
        rel.base_off.store(off, Release);
        // Its jump (Myers 1983): the base's jump's jump when the base's two jumps are as long, else the base. A base
        // with none (depth 0) jumps to itself.
        let jump = |r: &Relation| -> *const Relation {
            let o = r.jump_off.load(Relaxed);
            if o == 0 { r } else { std::ptr::from_ref(r).cast::<u8>().wrapping_offset(o as isize).cast() }
        };
        // SAFETY: a chain's jumps lie in its block, before it (each made with its base there).
        let (j, depth_of) = (jump(base), |p: *const Relation| unsafe { (*p).depth });
        let jj = unsafe { jump(&*j) };
        let to = if base.depth - depth_of(j) == depth_of(j) - depth_of(jj) && base.depth > 0 { jj } else { std::ptr::from_ref(base) };
        // SAFETY: just made, this caller's alone; `to` is a descriptor of the chain in this block.
        unsafe {
            let me = std::ptr::from_ref(rel).cast_mut();
            (*me).depth = base.depth + 1;
            (*me).jump_rows = (*to).base_rows;
        }
        rel.jump_off.store(to as i64 - std::ptr::from_ref(rel) as i64, Release);
        rel.flags.fetch_or(DELTA | CONTINUES, Release);
        Some(rel)
    }

    fn try_link<'b>(block: &'b Block, spec: &Spec, base: Option<(&Block, &Relation)>, base_rows: u32, rows_of: &Relation, len: u32) -> Option<&'b Relation> {
        if !block.holds(std::ptr::from_ref(rows_of)) || len > rows_of.rows.len() || rows_of.rows.ncols as usize != spec.widths.len() {
            return None;
        }
        let rel = match base {
            None if base_rows == 0 => Relation::try_create(block, spec, None)?,
            None => return None,
            Some((home, b)) if std::ptr::eq(home, block) => Relation::try_continue(block, spec, b, base_rows)?,
            Some((home, b)) => {
                let full = b.base_rows == 0 && !b.is_delta();
                let at = home.offset_of(std::ptr::from_ref(b));
                if !(full || b.continues()) || base_rows > b.len_all() || !home.holds(std::ptr::from_ref(b)) || at == 0 {
                    return None;
                }
                let rel = Relation::try_create(block, spec, None)?;
                // SAFETY: just made, this caller's alone.
                unsafe { (*std::ptr::from_ref(rel).cast_mut()).base_rows = base_rows };
                rel.base_off.store(at as i64, Release);
                rel.flags.fetch_or(DELTA | CONTINUES | FAR_BASE, Release);
                rel
            }
        };
        // Its rows: `rows_of`'s directories, read where they lie; none of its own (its own directories, made with
        // its descriptor, stay unused and go back with it).
        // SAFETY: just made, this caller's alone; `rows_of` is frozen at `len` rows for as long as this lives (its
        // holder keeps it).
        unsafe {
            let me = std::ptr::from_ref(rel).cast_mut();
            let (src, dst) = (&rows_of.rows, &mut (*me).rows);
            dst.min_log = src.min_log;
            dst.ncols = src.ncols;
            dst.nchunks.store(src.nchunks.load(Acquire), Relaxed);
            dst.reserved.store(len, Relaxed);
            dst.len.store(len, Release);
            block.span(&dst.cols, src.cols.as_ptr(), src.cols.len());
        }
        rel.flags.fetch_or(ALIASED | FROZEN, Release);
        Some(rel)
    }

    fn try_continue_far<'b>(block: &'b Block, spec: &Spec, below: Option<(u32, usize, &Relation, u32)>) -> Option<&'b Relation> {
        try_continue_far_cols(block, spec.cols(), below)
    }

    fn in_block(block: &Block, at: usize) -> Option<&Relation> {
        if at == 0 || at + size_of::<Relation>() > block.reserved() || !at.is_multiple_of(align_of::<Relation>()) {
            return None;
        }
        // SAFETY: within the block's reservation and aligned, where its maker put a descriptor.
        Some(unsafe { &*block.ptr::<Relation>(at) })
    }

    fn add_order(&self, pin: &mut crate::region::BlockPin, cols: &[u16], ids: &[u32], fences: &[i64], prints: &[u8]) -> Option<usize> {
        let block: &Block = pin.block();
        assert!(block.holds(std::ptr::from_ref(self)) && pin.writes() && self.is_frozen(), "a frozen layer of a block pinned to write");
        fn chunk<T: Copy>(block: &Block, v: &[T]) -> Option<Given> {
            if v.is_empty() {
                return Some(Given::default());
            }
            let (at, kept) = block.try_alloc_chunk_fit(size_of_val(v), 8)?;
            // SAFETY: a chunk of `v`'s bytes, just taken, in the block the caller's pin holds to write.
            // copy: derived: an order's column, computed by its writer, written once where the relation keeps it
            unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), block.ptr::<T>(at), v.len()) };
            Some(Given { at, len: v.len(), kept })
        }
        let order = GivenOrder { rows: OrderRows::Listed(chunk(block, ids)?), fences: chunk(block, fences)?, tags: Given::default(), prints: chunk(block, prints)? };
        // SAFETY: the pin is the block's one writer, so no one else adds to the directory meanwhile; each chunk is
        // written and lies in the block.
        unsafe { self.try_add_order(block, cols, order) }
    }

    fn claim_own(&self, pin: &mut crate::region::BlockPin, n: u32) -> Option<u32> {
        let block: &Block = pin.block();
        assert!(block.holds(std::ptr::from_ref(self)) && pin.writes() && !self.is_frozen(), "an open relation of a block pinned to write");
        let at = self.rows.len();
        if !self.rows.try_reserve(block, u64::from(at) + u64::from(n)) {
            return None;
        }
        // SAFETY: reserved just now, under the block's one write pin.
        Some(unsafe { self.rows.claim_reserved(n) })
    }

    fn put_own(&self, pin: &mut crate::region::BlockPin, i: u32, c: usize, v: i64) {
        assert!(pin.block().holds(std::ptr::from_ref(self)) && pin.writes() && !self.is_frozen(), "an open relation of a block pinned to write");
        assert!(i < self.rows.len() && c < self.arity as usize, "a claimed row's cell");
        // SAFETY: claimed, and the pin is the block's one writer.
        unsafe { self.rows.put(i, c, v) };
    }

    #[inline]
    fn get_far(&self, far: Option<&Block>, id: u32, c: usize) -> i64 {
        self.get_at(far.map(|b| b.ptr::<u8>(0).cast_const()), id, c)
    }

    fn try_kills_own<'b>(block: &'b Block, spec: &Spec, base: &Relation) -> Option<&'b Relation> {
        let words = base.own_dead();
        assert!(!words.is_empty(), "a relation's own dead rows");
        let rel = Relation::try_create(block, spec, Some(base))?;
        // SAFETY: `words` lie in `block` (`base`'s), which `base` keeps while its delta lives (the delta's holder keeps
        // `base`).
        unsafe { block.span(&rel.dead, words.as_ptr(), words.len()) };
        Some(rel)
    }

    fn try_kills<'b>(block: &'b Block, spec: &Spec, base: &Relation, words: &[u64]) -> Option<&'b Relation> {
        let rel = Relation::try_create(block, spec, Some(base))?;
        let Some(at) = block.try_alloc_chunk(Relation::kills_bytes(words.len()), 64) else {
            // SAFETY: just made in `block`; no one has it.
            unsafe { rel.release(block) };
            return None;
        };
        let w: *mut u64 = block.ptr(at);
        // SAFETY: `w` holds `words.len()` words, just allocated in `block`, as `rel` is.
        unsafe {
            std::ptr::copy_nonoverlapping(words.as_ptr(), w, words.len());
            block.span(&rel.dead, w, words.len());
        }
        Some(rel)
    }

    unsafe fn release_kills(&self, block: &Block) -> usize {
        let dead = self.dead.as_slice();
        let given = block.free_chunk(block.offset_of(dead.as_ptr()), Relation::kills_bytes(dead.len()));
        given + unsafe { self.release(block) }
    }

    unsafe fn release(&self, block: &Block) -> usize {
        // Aliased rows are their own relation's, given back with it.
        let own_rows = if self.flags.load(Acquire) & ALIASED != 0 { 0 } else { unsafe { self.rows.release(block) } };
        let given = own_rows + unsafe { self.killed.release(block) };
        let kept = self.known_kept.load(Acquire) as usize;
        if kept > 0 {
            block.free_chunk_fit(block.offset_of(self.known.as_ptr()), kept);
        }
        let kept = self.own_dead_kept.load(Acquire) as usize;
        if kept > 0 {
            block.free_chunk_fit(block.offset_of(self.own_dead.as_ptr()), kept);
        }
        // Its added indexes, and their directory: chunks of the block's.
        if self.flags.load(Acquire) & ADDED != 0 {
            // SAFETY: the caller's: this is the relation's last use.
            unsafe { self.forget_indexes(block) };
            block.free_chunk_fit_resident(block.offset_of(self.indexes.as_ptr()), block.fit_kept(ADDED_INDEXES * size_of::<Index>()));
        }
        let (at, bytes) = (block.offset_of(std::ptr::from_ref(self)), Relation::descriptor_bytes(self.arity as usize));
        // The last use of `self`: its own memory goes back.
        given + block.free_chunk(at, bytes)
    }

    unsafe fn adopt_dead(&self, block: &Block, at: usize, len: usize, kept: usize) -> bool {
        if self.own_dead_kept.load(Acquire) != 0 || len == 0 || kept == 0 {
            return false;
        }
        // SAFETY: the caller's.
        unsafe {
            let me = std::ptr::from_ref(self).cast_mut();
            block.span(&raw const (*me).own_dead, block.ptr::<u64>(at), len);
        }
        self.own_dead_kept.store(kept as u64, Release);
        true
    }

    unsafe fn try_set_known(&self, block: &Block, ranges: &[(u32, u32, &[u16])]) -> bool {
        if ranges.is_empty() {
            return true;
        }
        if self.known_kept.load(Acquire) != 0 || ranges.iter().any(|(_, _, c)| c.len() > MAX_KEY) {
            return false;
        }
        let Some((at, kept)) = block.try_alloc_chunk_fit(ranges.len() * size_of::<Known>(), 8) else { return false };
        let p = block.ptr::<Known>(at);
        // SAFETY: `p` holds `ranges.len()` entries, this relation's alone; the caller's for its descriptor.
        unsafe {
            for (i, &(lo, hi, cols)) in ranges.iter().enumerate() {
                let mut c = [0u16; MAX_KEY];
                c[..cols.len()].copy_from_slice(cols);
                p.add(i).write(Known { lo, hi, ncols: cols.len() as u32, cols: c });
            }
            let me = std::ptr::from_ref(self).cast_mut();
            block.span(&raw const (*me).known, p, ranges.len());
        }
        self.known_kept.store(kept as u64, Release);
        true
    }

    unsafe fn next_entry(&self, block: &Block) -> Option<usize> {
        if self.indexes.is_empty() {
            let (at, _) = block.try_alloc_chunk_fit_zeroed(ADDED_INDEXES * size_of::<Index>(), 8)?;
            let me = std::ptr::from_ref(self).cast_mut();
            // SAFETY: the caller's.
            unsafe { block.span(&raw const (*me).indexes, block.ptr::<Index>(at), ADDED_INDEXES) };
            self.flags.fetch_or(ADDED, Release);
        }
        let n = (self.nindex.load(Relaxed) as usize).min(self.indexes.len());
        let ix = self.indexes.as_slice();
        // An entry given back is taken first (`give_back_index`) when its room is the block's already; one that keeps
        // its room (a block of no free list) is left for an index that fits it (`dead_room`), and taken last.
        let dead = |i: usize, room: bool| ix[i].state == DEAD && (!ix[i].offs.is_empty()) == room;
        // Its taker marks it live (`state` 0) once its room is had.
        (0..n).find(|&i| dead(i, false)).or((n < ix.len()).then_some(n)).or_else(|| (0..n).find(|&i| dead(i, true)))
    }

    unsafe fn give_back_index(&self, block: &Block, room: &IndexRoom) {
        let ix = std::ptr::from_ref(&self.indexes.as_slice()[room.index]).cast_mut();
        // SAFETY: the caller's; `find_index` skips an entry whose `ncols` is 0.
        unsafe {
            std::ptr::write_volatile(&raw mut (*ix).ncols, 0);
            assert!((*ix).ordered == 0 && (*ix).state != DEAD, "a hashed index given back once");
            (*ix).key = [0; MAX_KEY];
            if !block.has_free_list() {
                (*ix).state = DEAD;
                return;
            }
            give_back_spans(block, self.flags.load(Acquire) & REWRITTEN != 0, ((*ix).offs.as_ptr(), (*ix).offs.len()), ((*ix).rows.as_ptr(), (*ix).rows.len()));
            (*ix).offs.clear();
            (*ix).rows.clear();
            (*ix).marks.clear();
            (*ix).key = [0; MAX_KEY];
            (*ix).state = DEAD;
        }
    }

    unsafe fn try_add_order(&self, block: &Block, cols: &[u16], order: GivenOrder) -> Option<usize> {
        assert!(!cols.is_empty() && cols.len() <= MAX_KEY, "a key of {} columns", cols.len());
        let GivenOrder { rows, fences, tags, prints } = order;
        // SAFETY: the caller's: this relation's descriptor is ours to change.
        unsafe {
            let i = self.next_entry(block)?;
            let ix = std::ptr::from_ref(&self.indexes.as_slice()[i]).cast_mut();
            (*ix).ncols = 0;
            (*ix).from = 0;
            (*ix).state = 0;
            let mut key = [0u16; MAX_KEY];
            key[..cols.len()].copy_from_slice(cols);
            (*ix).key = key;
            fn clear<T>(dst: *mut RelSpan<T>) {
                // SAFETY: the caller's: `dst` is the entry's.
                unsafe { (*dst).clear() }
            }
            clear(&raw mut (*ix).offs);
            let ids = match rows {
                OrderRows::Range(lo, hi) => {
                    (*ix).ordered = ORDER_ROWS;
                    (*ix).mask = (u64::from(lo) << 32) | u64::from(hi);
                    clear(&raw mut (*ix).rows);
                    Given::default()
                }
                OrderRows::Listed(ids) => {
                    (*ix).ordered = ORDER;
                    (*ix).mask = 0;
                    block.span(&raw const (*ix).rows, block.ptr::<u32>(ids.at), ids.len);
                    ids
                }
            };
            fn set<T>(block: &Block, dst: *mut RelSpan<T>, g: Given) {
                if g.len == 0 {
                    clear(dst);
                    return;
                }
                // SAFETY: the caller's: `dst` is the entry's, `g` a chunk of `block`.
                unsafe { block.span(dst, block.ptr::<T>(g.at), g.len) }
            }
            set(block, &raw mut (*ix).fences, fences);
            set(block, &raw mut (*ix).tags, tags);
            set(block, &raw mut (*ix).prints, prints);
            (*ix).held = [ids.kept as u64, fences.kept as u64, tags.kept as u64, prints.kept as u64];
            self.nindex.fetch_max(i as u32 + 1, Release);
            std::sync::atomic::fence(Release);
            std::ptr::write_volatile(&raw mut (*ix).ncols, cols.len() as u32);
            Some(i)
        }
    }

    /// Gives this relation a node-keyed index on column `col` of its rows `0..n` (decision 0694 step 1), laid out as
    /// a node's id is (`packed::ID_SPLIT`): its document's key (the high 32 bits), then its low 32 bits, a granule's
    /// slot above an index in the granule. Each document is held in the smaller of two forms:
    /// - **bits**: its slots from its least to its greatest, each a slot entry giving its bits, from its least
    ///   index's word to its greatest's. A test is the slot's entry and one bit.
    /// - **lows**: its distinct low halves, sorted. A test is a search of them.
    ///
    /// Either is at most a hashed index's bytes (lows: 4 a distinct value); a document takes bits when they are at
    /// most twice its lows' bytes. Its number; `None` when its directory is full or `block` has no room.
    ///
    /// Its `rows` (`u32`s): the documents' count; each document's key, its form's first field, its second, and where
    /// its form starts; then the forms. A document of bits has its least slot and its slots; their entries, two a
    /// slot, are its words' offset and its first word's number above its words' count (0 for a slot of none). A
    /// document of lows has `LOWS` and its lows' count, then its lows. Its `fences`: the words.
    ///
    /// # Safety
    /// As `try_reserve_index`'s: this relation's descriptor is the caller's to change, its rows `0..n` written.
    unsafe fn try_add_node_bits(&self, block: &Block, col: u16, n: u32) -> Option<usize> {
        use crate::packed::ID_SPLIT;
        if n == 0 || col as usize >= self.rows.ncols as usize {
            return None;
        }
        // The distinct values, by document then low half (scratch of the build).
        let mut vals: Vec<u64> = (0..n).map(|id| self.get(id, col as usize) as u64).collect();
        vals.sort_unstable();
        vals.dedup();
        /// A document's values, `vals[a..b]`, and its form: bits (its least slot, its slots, its words) or lows.
        struct Doc {
            hi: u32,
            a: usize,
            b: usize,
            bits: Option<(u32, u32, u32)>,
        }
        let (mut docs, mut a) = (Vec::new(), 0usize);
        while a < vals.len() {
            let hi = (vals[a] >> 32) as u32;
            let b = a + vals[a..].iter().take_while(|&&v| (v >> 32) as u32 == hi).count();
            let slot = |v: u64| (v as u32) >> ID_SPLIT;
            let (s0, s1) = (slot(vals[a]), slot(vals[b - 1]));
            // Each slot's words: its least index's word to its greatest's.
            let mut words = 0u32;
            let mut i = a;
            while i < b {
                let s = slot(vals[i]);
                let j = i + vals[i..b].iter().take_while(|&&v| slot(v) == s).count();
                let x = |v: u64| (v as u32 & ((1 << ID_SPLIT) - 1)) / 64;
                words += x(vals[j - 1]) - x(vals[i]) + 1;
                i = j;
            }
            let bits_bytes = 8 * (s1 - s0 + 1) as usize + 8 * words as usize;
            let bits = (bits_bytes <= 2 * 4 * (b - a)).then_some((s0, s1 - s0 + 1, words));
            docs.push(Doc { hi, a, b, bits });
            a = b;
        }
        const LOWS: u32 = u32::MAX;
        let form: usize = docs.iter().map(|d| d.bits.map_or(d.b - d.a, |(_, slots, _)| 2 * slots as usize)).sum();
        let words: usize = docs.iter().filter_map(|d| d.bits).map(|(_, _, w)| w as usize).sum();
        let dir = 1 + 4 * docs.len() + form;
        let (d_at, d_kept) = block.try_alloc_chunk_fit(4 * dir, 8)?;
        let Some((w_at, w_kept)) = block.try_alloc_chunk_fit(8 * words.max(1), 8) else {
            block.free_chunk_fit(d_at, d_kept);
            return None;
        };
        let (d, w) = (block.ptr::<u32>(d_at), block.ptr::<i64>(w_at));
        // SAFETY: both chunks were just allocated, this index's alone until it is marked built.
        unsafe {
            std::ptr::write_bytes(d, 0, dir);
            std::ptr::write_bytes(w, 0, words.max(1));
            d.write(docs.len() as u32);
            let (mut at, mut off) = ((1 + 4 * docs.len()) as u32, 0u32);
            for (k, doc) in docs.iter().enumerate() {
                let e = d.add(1 + 4 * k);
                e.write(doc.hi);
                e.add(3).write(at);
                let values = &vals[doc.a..doc.b];
                match doc.bits {
                    None => {
                        e.add(1).write(LOWS);
                        e.add(2).write(values.len() as u32);
                        for (i, &v) in values.iter().enumerate() {
                            d.add(at as usize + i).write(v as u32);
                        }
                        at += values.len() as u32;
                    }
                    Some((s0, slots, _)) => {
                        e.add(1).write(s0);
                        e.add(2).write(slots);
                        let mut i = 0;
                        while i < values.len() {
                            let low = values[i] as u32;
                            let s = low >> ID_SPLIT;
                            let j = i + values[i..].iter().take_while(|&&v| (v as u32) >> ID_SPLIT == s).count();
                            let x = |v: u64| v as u32 & ((1 << ID_SPLIT) - 1);
                            let (w0, w1) = (x(values[i]) / 64, x(values[j - 1]) / 64);
                            let t = d.add(at as usize + 2 * (s - s0) as usize);
                            t.write(off);
                            t.add(1).write(w0 << 16 | (w1 - w0 + 1));
                            for &v in &values[i..j] {
                                *w.add((off + x(v) / 64 - w0) as usize) |= 1 << (x(v) % 64);
                            }
                            off += w1 - w0 + 1;
                            i = j;
                        }
                        at += 2 * slots;
                    }
                }
            }
        }
        // SAFETY: the caller's: this relation's descriptor is ours to change.
        unsafe {
            let Some(i) = self.next_entry(block) else {
                block.free_chunk_fit(d_at, d_kept);
                block.free_chunk_fit(w_at, w_kept);
                return None;
            };
            let ix = std::ptr::from_ref(&self.indexes.as_slice()[i]).cast_mut();
            (*ix).ncols = 0;
            (*ix).from = 0;
            (*ix).state = 0;
            let mut key = [0u16; MAX_KEY];
            key[0] = col;
            (*ix).key = key;
            (*ix).ordered = NODE_BITS;
            (*ix).mask = u64::from(n);
            (*ix).offs.clear();
            (*ix).tags.clear();
            (*ix).prints.clear();
            block.span(&raw const (*ix).rows, d, dir);
            block.span(&raw const (*ix).fences, w, words.max(1));
            (*ix).held = [d_kept as u64, w_kept as u64, 0, 0];
            self.nindex.fetch_max(i as u32 + 1, Release);
            std::sync::atomic::fence(Release);
            std::ptr::write_volatile(&raw mut (*ix).ncols, 1);
            Some(i)
        }
    }

    /// Gives this relation a membership filter on `cols` of its rows `0..n` (decision 0706 E2): a blocked Bloom filter
    /// in `bloom`'s format, `FILTER_BITS` a row at least (its words a power of two), each row's key placed by
    /// `bloom::key` of its `cols`, in order. A probe whose key it rejects reads no row. Its number; `None` when its
    /// directory is full or `block` has no room.
    ///
    /// # Safety
    /// As `try_reserve_index`'s: this relation's descriptor is the caller's to change, its rows `0..n` written.
    unsafe fn try_add_filter(&self, block: &Block, cols: &[u16], n: u32) -> Option<usize> {
        if n == 0 || cols.is_empty() || cols.len() > MAX_KEY || cols.iter().any(|&c| c as usize >= self.rows.ncols as usize) {
            return None;
        }
        let words = (n as usize * FILTER_BITS).div_ceil(64).next_power_of_two();
        let (w_at, w_kept) = block.try_alloc_chunk_fit(8 * words, 8)?;
        let w = block.ptr::<u64>(w_at);
        // SAFETY: the chunk was just allocated, this index's alone until it is marked built.
        unsafe {
            std::ptr::write_bytes(w, 0, words);
            let mut key = [0i64; MAX_KEY];
            for id in 0..n {
                for (k, &c) in key.iter_mut().zip(cols) {
                    *k = self.get(id, c as usize);
                }
                let (i, mask) = crate::bloom::place(crate::bloom::key(&key[..cols.len()]), words);
                *w.add(i) |= mask;
            }
        }
        // SAFETY: the caller's: this relation's descriptor is ours to change.
        unsafe {
            let Some(i) = self.next_entry(block) else {
                block.free_chunk_fit(w_at, w_kept);
                return None;
            };
            let ix = std::ptr::from_ref(&self.indexes.as_slice()[i]).cast_mut();
            (*ix).ncols = 0;
            (*ix).from = 0;
            (*ix).state = 0;
            let mut key = [0u16; MAX_KEY];
            key[..cols.len()].copy_from_slice(cols);
            (*ix).key = key;
            (*ix).ordered = FILTER;
            (*ix).mask = u64::from(n);
            (*ix).offs.clear();
            (*ix).tags.clear();
            (*ix).prints.clear();
            block.span(&raw const (*ix).rows, w.cast::<u32>(), 0);
            block.span(&raw const (*ix).fences, w.cast::<i64>(), words);
            (*ix).held = [0, w_kept as u64, 0, 0];
            self.nindex.fetch_max(i as u32 + 1, Release);
            std::sync::atomic::fence(Release);
            std::ptr::write_volatile(&raw mut (*ix).ncols, cols.len() as u32);
            Some(i)
        }
    }

    unsafe fn try_reserve_index(&self, block: &Block, cols: &[u16], n: u32) -> Option<IndexRoom> {
        // SAFETY: the caller's.
        unsafe { self.try_reserve_index_buckets(block, cols, n, n as usize) }
    }

    unsafe fn try_reserve_index_from(&self, block: &Block, cols: &[u16], from: u32, n: u32) -> Option<IndexRoom> {
        // SAFETY: the caller's.
        let mut room = unsafe { self.try_reserve_index_buckets(block, cols, n, n as usize) }?;
        let ix = std::ptr::from_ref(&self.indexes.as_slice()[room.index]).cast_mut();
        // SAFETY: the entry is the caller's, not yet built.
        unsafe { (*ix).from = from };
        room.from = from;
        Some(room)
    }

    unsafe fn try_reserve_index_buckets(&self, block: &Block, cols: &[u16], n: u32, keys: usize) -> Option<IndexRoom> {
        // SAFETY: the caller's.
        unsafe { self.try_reserve_index_cap(block, cols, n, keys.min(n as usize).next_power_of_two().max(1)) }
    }

    unsafe fn try_reserve_index_live(&self, block: &Block, cols: &[u16], from: u32, n: u32, buckets: usize) -> Option<IndexRoom> {
        assert!(buckets.is_power_of_two(), "{buckets} buckets");
        // SAFETY: the caller's.
        let mut room = unsafe { self.try_reserve_index_cap(block, cols, n, buckets) }?;
        let ix = std::ptr::from_ref(&self.indexes.as_slice()[room.index]).cast_mut();
        // SAFETY: the entry is the caller's, not yet built.
        unsafe { (*ix).from = from };
        room.from = from;
        Some(room)
    }

    unsafe fn try_reserve_index_cap(&self, block: &Block, cols: &[u16], n: u32, cap: usize) -> Option<IndexRoom> {
        assert!(!cols.is_empty() && cols.len() <= MAX_KEY, "a key of {} columns", cols.len());
        // SAFETY: the caller's: this relation's descriptor is ours to change.
        unsafe {
            // Chunks the block's free list takes back when the index is forgotten (`forget_indexes`), zeroed as an
            // allocation is.
            let (ob, rb) = (4 * (cap + 1), index_rows_bytes(n as usize));
            // In a block of no free list, an index given back keeps its room for one that fits it (`give_back_index`).
            let reused = if block.has_free_list() || self.indexes.is_empty() { None } else { self.dead_room(cap + 1, n as usize) };
            let i = match reused {
                Some(i) => i,
                None => self.next_entry(block)?,
            };
            let (offs_at, rows_at) = if let Some(d) = reused {
                let d = &self.indexes.as_slice()[d];
                let (o, r) = (block.offset_of(d.offs.as_ptr()), block.offset_of(d.rows.as_ptr()));
                // Its buckets zeroed, as a new chunk's are. Its rows and marks not: each builder writes every one it
                // holds before any reader reads it (`build_index_with`, the device's `ix_place`, `ix_mark`), and a
                // live relation's room is for its bound's rows, most never written.
                block.ptr::<u8>(o).write_bytes(0, ob);
                (o, r)
            } else if self.flags.load(Acquire) & REWRITTEN != 0 {
                let o = block.try_alloc_chunk(ob, 8)?;
                if n == 0 {
                    // No rows, no room for them: a span of none points nowhere (`RelSpan::set_bits`), and gives none back.
                    (o, o)
                } else {
                let Some(r) = block.try_alloc_chunk(rb, 8) else {
                    block.free_chunk(o, ob);
                    return None;
                };
                (o, r)
                }
            } else {
                let (o, _) = block.try_alloc_chunk_fit_zeroed(ob, 8)?;
                if n == 0 {
                    (o, o)
                } else {
                // Its rows and marks unzeroed, as a reused room's (above).
                let Some((r, _)) = block.try_alloc_chunk_fit(rb, 8) else {
                    block.free_chunk_fit(o, block.fit_kept(ob));
                    return None;
                };
                (o, r)
                }
            };
            let (offs, rows): (*mut u32, *mut u32) = (block.ptr(offs_at), block.ptr(rows_at));
            let ix = std::ptr::from_ref(&self.indexes.as_slice()[i]).cast_mut();
            (*ix).ncols = 0;
            (*ix).ordered = 0;
            (*ix).from = 0;
            (*ix).held = [0; 4];
            (*ix).state = 0;
            let mut key = [0u16; MAX_KEY];
            key[..cols.len()].copy_from_slice(cols);
            (*ix).key = key;
            (*ix).mask = (cap - 1) as u64;
            block.span(&raw const (*ix).offs, offs, cap + 1);
            block.span(&raw const (*ix).rows, rows, n as usize);
            // Its marks after its rows, as many (decision 0671: `index_rows_bytes`).
            block.span(&raw const (*ix).marks, rows.wrapping_add(n as usize), n as usize);
            self.nindex.fetch_max(i as u32 + 1, Release);
            let marks = block.offset_of(rows.wrapping_add(n as usize));
            Some(IndexRoom { index: i, offs: block.offset_of(offs), rows: block.offset_of(rows), marks, mask: (cap - 1) as u64, rows_n: n, key: (*ix).key, ncols: cols.len(), from: 0 })
        }
    }

    unsafe fn grow_index_rows(&self, block: &Block, index: usize, n: u32) -> Option<usize> {
        let ix = std::ptr::from_ref(&self.indexes.as_slice()[index]).cast_mut();
        // SAFETY: the caller's: the entry is ours, and no reader runs.
        unsafe {
            let (old, len) = (room_at(block, (*ix).rows.as_ptr()), index_rows_bytes((*ix).rows.len()));
            if let Some(old) = old.filter(|_| (n as usize) <= (*ix).rows.len()) {
                return Some(old);
            }
            let rb = index_rows_bytes(n as usize);
            let rewritten = self.flags.load(Acquire) & REWRITTEN != 0;
            let r = if rewritten { block.try_alloc_chunk(rb, 8)? } else { block.try_alloc_chunk_fit_zeroed(rb, 8)?.0 };
            match old {
                Some(old) if rewritten => block.free_chunk_resident(old, len),
                Some(old) => block.free_chunk_fit_resident(old, block.fit_kept(len)),
                None => {}
            }
            block.span(&raw const (*ix).rows, block.ptr::<u32>(r), n as usize);
            block.span(&raw const (*ix).marks, block.ptr::<u32>(r).wrapping_add(n as usize), n as usize);
            Some(r)
        }
    }

    unsafe fn build_index(&self, room: &IndexRoom) {
        let key = &room.key[..room.ncols];
        // A continuation's values are of every row it reads (its base's first); a delta's its own rows'. By row id.
        let value = |i: u32, c: u16| if self.continues() { self.get(i, c as usize) } else { self.rows.get(i - self.base_rows, c as usize) };
        let first = if self.continues() { 0 } else { self.base_rows };
        let hash = |i: u32| key.iter().fold(crate::SEED, |h, &c| crate::mix(h, value(first + i, c)));
        let same = |a: u32, b: u32| key.iter().all(|&c| value(a, c) == value(b, c));
        // SAFETY: the caller's.
        unsafe { self.build_index_with(room, &hash, &same) }
    }

    unsafe fn build_index_hashed(&self, room: &IndexRoom, hash: &dyn Fn(u32) -> u64) {
        // SAFETY: the caller's.
        unsafe { self.build_index_with(room, hash, &|_, _| false) }
    }

    unsafe fn build_index_with(&self, room: &IndexRoom, hash: &dyn Fn(u32) -> u64, same: &dyn Fn(u32, u32) -> bool) {
        let ix = &self.indexes.as_slice()[room.index];
        let n = room.rows_n as usize;
        // SAFETY: the caller's: the room's spans are ours to write.
        let (offs, rows) = unsafe {
            (std::slice::from_raw_parts_mut(ix.offs.as_slice().as_ptr().cast_mut(), ix.offs.len()),
             std::slice::from_raw_parts_mut(ix.rows.as_slice().as_ptr().cast_mut(), ix.rows.len()))
        };
        offs.fill(0);
        // A continuation's index is of every row it reads (its base's first, ids from 0); a delta's of its own.
        let base = if self.continues() { 0 } else { self.base_rows };
        let bucket: Vec<u32> = (0..n as u32).map(|i| (hash(i) & room.mask) as u32).collect();
        for &b in &bucket {
            offs[b as usize + 1] += 1;
        }
        for b in 0..offs.len() - 1 {
            offs[b + 1] += offs[b];
        }
        let mut at: Vec<u32> = offs[..offs.len() - 1].to_vec();
        for (i, &b) in bucket.iter().enumerate() {
            rows[at[b as usize] as usize] = base + i as u32;
            at[b as usize] += 1;
        }
        // SAFETY: the caller's: the marks are the room's, as many as its rows.
        let marks = unsafe { std::slice::from_raw_parts_mut(ix.marks.as_slice().as_ptr().cast_mut(), ix.marks.len()) };
        let rows = &rows[..];
        fill_marks(offs, &mut marks[..n], &|t| mark_tag(hash(rows[t] - base)), &|t, u| same(rows[t], rows[u]));
    }

    unsafe fn forget_indexes(&self, block: &Block) {
        assert!(self.flags.load(Acquire) & ADDED != 0 || self.indexes.is_empty(), "a built relation's indexes forgotten");
        let n = (self.nindex.load(Acquire) as usize).min(self.indexes.len());
        for ix in &self.indexes.as_slice()[..n] {
            // SAFETY: the caller's; `find_index` skips an entry whose `ncols` is 0.
            unsafe { std::ptr::write_volatile(&raw mut (*std::ptr::from_ref(ix).cast_mut()).ncols, 0) };
            if ix.state == DEAD {
                // Given back already (`give_back_index`): its room is the block's.
                // SAFETY: the caller's.
                unsafe { (*std::ptr::from_ref(ix).cast_mut()).state = 0 };
                continue;
            }
            if ix.ordered == ORDER || ix.ordered == ORDER_ROWS || ix.ordered == NODE_BITS || ix.ordered == FILTER {
                // A sorted order's chunks, each as it was given (`try_add_order`), and a node-keyed index's.
                let spans = [(ix.rows.as_ptr().cast::<u8>(), ix.held[0]), (ix.fences.as_ptr().cast(), ix.held[1]), (ix.tags.as_ptr().cast(), ix.held[2]), (ix.prints.as_ptr().cast(), ix.held[3])];
                for (p, kept) in spans {
                    if kept > 0 {
                        block.free_chunk_fit(block.offset_of(p), kept as usize);
                    }
                }
                continue;
            }
            // Left resident, as an index's room was before it went back (a few bytes a row; the next index takes it).
            give_back_spans(block, self.flags.load(Acquire) & REWRITTEN != 0, (ix.offs.as_ptr(), ix.offs.len()), (ix.rows.as_ptr(), ix.rows.len()));
        }
        self.nindex.store(0, Release);
    }
}


/// Where `p` lies in `block`, or none: a span of no items points nowhere (`RelSpan::set_bits`), and holds no room.
fn room_at<T>(block: &Block, p: *const T) -> Option<usize> {
    block.holds(p).then(|| block.offset_of(p))
}

/// A hashed index's buckets and rows given back to `block`, each that holds room (an index of no rows has no rows
/// room), resident: the next index takes it.
fn give_back_spans(block: &Block, rewritten: bool, offs: (*const u32, usize), rows: (*const u32, usize)) {
    let spans = [(room_at(block, offs.0), 4 * offs.1), (room_at(block, rows.0), index_rows_bytes(rows.1))];
    for (at, bytes) in spans {
        match at {
            Some(at) if rewritten => block.free_chunk_resident(at, bytes),
            Some(at) => block.free_chunk_fit_resident(at, block.fit_kept(bytes)),
            None => {}
        }
    }
}

/// A writer of one open relation: claims rows, writes them, kills rows. `Send`: one per thread or morsel.
pub struct RelW<'b> {
    pub block: &'b Block,
    pub rel: &'b Relation,
    pub lock: &'b Mutex<()>,
}

impl<'b> RelW<'b> {
    /// One row. Returns its id.
    pub fn push(&self, vals: &[i64]) -> u32 {
        debug_assert_eq!(vals.len(), self.rel.arity as usize);
        assert!(!self.rel.is_frozen(), "push into a frozen relation");
        let s = self.rel.rows.claim(self.block, self.lock, 1);
        for (c, &v) in vals.iter().enumerate() {
            unsafe { self.rel.rows.put(s, c, v) };
        }
        s + self.rel.base_rows
    }

    /// Rows, flat: one claim for all of them.
    pub fn extend(&self, flat: &[i64]) {
        let a = self.rel.arity as usize;
        let n = flat.len() / a;
        if n == 0 {
            return;
        }
        let s = self.rel.rows.claim(self.block, self.lock, n as u32);
        for (r, row) in flat.chunks_exact(a).enumerate() {
            for (c, &v) in row.iter().enumerate() {
                unsafe { self.rel.rows.put(s + r as u32, c, v) };
            }
        }
    }

    /// Kills a row by id: a base row (in a delta) or one of its own.
    pub fn kill(&self, id: u32) {
        let s = self.rel.killed.claim(self.block, self.lock, 1);
        unsafe { self.rel.killed.put(s, 0, id as i64) };
    }

    /// Adds `vals` unless this relation already holds it: one writer only (a fixpoint's own relation). Returns
    /// whether it was new. The set test is an open-addressed table in the block, grown by doubling.
    pub fn insert_unique(&self, vals: &[i64]) -> bool {
        let rel = self.rel;
        let n = rel.rows.len() as u64;
        if (n + 1) * 2 > rel.pending.len() as u64 {
            self.grow_pending(((n + 1) * 4).next_power_of_two().max(64) as usize);
        }
        let slots = unsafe { rel.pending.as_mut_slice() };
        let mask = slots.len() - 1;
        let mut s = key_hash(vals) as usize & mask;
        loop {
            let x = slots[s];
            if x == 0 {
                let id = self.push(vals);
                slots[s] = id - rel.base_rows + 1;
                return true;
            }
            let id = x - 1;
            if (0..vals.len()).all(|c| rel.rows.get(id, c) == vals[c]) {
                return false;
            }
            s = (s + 1) & mask;
        }
    }

    fn grow_pending(&self, cap: usize) {
        let rel = self.rel;
        let p: *mut u32 = self.block.alloc_t(cap);
        let slots = unsafe { std::slice::from_raw_parts_mut(p, cap) };
        let mut row = vec![0i64; rel.arity as usize];
        for i in 0..rel.rows.len() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = rel.rows.get(i, c);
            }
            let mut s = key_hash(&row) as usize & (cap - 1);
            while slots[s] != 0 {
                s = (s + 1) & (cap - 1);
            }
            slots[s] = i + 1;
        }
        rel.pend_used.fetch_add((cap * 4) as u64, Relaxed);
        unsafe { self.block.span(&rel.pending, p, cap) };
    }
}

/// What `freeze` did, for measurement.
#[derive(Default, Debug, Clone, Copy)]
pub struct FreezeStats {
    pub rows: u64,
    pub live: u64,
    pub dropped: u64,
    pub bytes: u64,
}

/// Freezes an open relation: set semantics, the canonical order, indexes, dead bits and statistics (see the
/// module's comment). `base` is the delta's base. After this, the relation only changes by a new delta. Its
/// temporaries come from this thread's scratch (`scratch::with`).
pub fn freeze(block: &Block, rel: &Relation, spec: &Spec, base: Option<&Relation>) -> FreezeStats {
    crate::scratch::with(|s| freeze_in(block, rel, spec, base, s))
}

/// Bits a value range needs.
fn bits_for(range: u64) -> u32 {
    64 - range.leading_zeros()
}

/// The column the rows `ids` (read by `val(column, row)`) already lie in order on: `want` when they lie in its order,
/// else, when `others`, the first column they lie in order on (a join's output usually lies in its probe side's
/// order) whose values are not all one (an order that says nothing); `None` when there is none. Each column's check
/// stops at its first row out of order.
pub(crate) fn in_order_on(val: impl Fn(usize, usize) -> i64, a: usize, want: Option<usize>, others: bool, ids: &[u32]) -> Option<usize> {
    let sorted = |c: usize| ids.windows(2).all(|w| val(c, w[0] as usize) <= val(c, w[1] as usize));
    if let Some(c) = want.filter(|&c| sorted(c)) {
        return Some(c);
    }
    if !others {
        return None;
    }
    let varies = |c: usize| ids.first().zip(ids.last()).is_some_and(|(&f, &l)| val(c, f as usize) != val(c, l as usize));
    (0..a).filter(|&c| Some(c) != want).find(|&c| varies(c) && sorted(c))
}

/// Arranges the rows `ids` (read by `val(column, row)`) for a relation that asks to be sorted on `want` (none: as
/// they came) and drops duplicates, without a sort on every column (decision 0678 step 8). The rows are kept in the
/// order they came when they lie in order on `want`, or, when `others`, on another column (`in_order_on`: the user,
/// 2026-11-25, "If relations arrive already sorted on a different column (common after joins), then we should keep it
/// sorted on the different column"); else they are radix sorted on `want` (its range `lo..=hi`), stably, so ties
/// keep their order. Then each row equal to one before it, found in a table of the rows' hashes in `scratch`, is
/// dropped. The rows left are moved to the front of `ids`, in order. Returns their count, and the column they lie
/// sorted on.
pub(crate) fn distinct_sorted(scratch: &crate::scratch::Scratch, val: impl Fn(usize, usize) -> i64, a: usize, want: Option<(usize, i64, i64)>, others: bool, ids: &mut [u32]) -> (usize, Option<usize>) {
    let m = ids.len();
    let on = in_order_on(&val, a, want.map(|w| w.0), others, ids);
    if let Some((c, lo, hi)) = want.filter(|_| on.is_none()) {
        let bits = bits_for((hi as i128 - lo as i128) as u64);
        if m > 1 && bits > 0 {
            let keys: &mut [u64] = scratch.take(m);
            for (k, &i) in keys.iter_mut().zip(ids.iter()) {
                *k = (val(c, i as usize) as i128 - lo as i128) as u64;
            }
            let (tk, ti) = (scratch.take::<u64>(m), scratch.take::<u32>(m));
            crate::sort::radix_pairs(keys, ids, bits, tk, ti);
        }
    }
    let cap = (2 * m).next_power_of_two().max(2);
    let mask = cap - 1;
    let slots: &mut [u32] = scratch.take(cap);
    slots.fill(u32::MAX);
    let mut left = 0;
    for p in 0..m {
        let i = ids[p] as usize;
        let mut h = crate::SEED;
        for c in 0..a {
            h = crate::mix(h, val(c, i));
        }
        let mut at = h as usize & mask;
        let seen = loop {
            match slots[at] {
                u32::MAX => {
                    slots[at] = i as u32;
                    break false;
                }
                j if (0..a).all(|c| val(c, j as usize) == val(c, i)) => break true,
                _ => at = (at + 1) & mask,
            }
        };
        if !seen {
            ids[left] = ids[p];
            left += 1;
        }
    }
    (left, on.or(want.map(|w| w.0)))
}

/// How many distinct values `hs` holds, by a table of them in `scratch`: no sort.
fn distinct_hashes(scratch: &crate::scratch::Scratch, hs: &[u64]) -> usize {
    let cap = (2 * hs.len()).next_power_of_two().max(2);
    let mask = cap - 1;
    let slots: &mut [u64] = scratch.take(cap);
    let used: &mut [u64] = scratch.zeroed(cap.div_ceil(64));
    let mut d = 0;
    for &h in hs {
        let mut at = (h ^ h >> 29) as usize & mask;
        loop {
            if used[at >> 6] >> (at & 63) & 1 == 0 {
                used[at >> 6] |= 1 << (at & 63);
                slots[at] = h;
                d += 1;
                break;
            }
            if slots[at] == h {
                break;
            }
            at = (at + 1) & mask;
        }
    }
    d
}

/// Builds the indexes on `keys` of a relation frozen just now, its live rows `order` (read by `val(column, row)`,
/// written back in that order, so row `j` of it is id `br + j`), each column's range `lo..=hi`: each key's rows grouped,
/// in id order within a group, a hashed index's buckets from the number of distinct keys; a key of one column, the
/// one the rows lie sorted on (`sort`), is that order (`ordered`, no buckets). The directory holds `room` entries
/// (`keys` first; the rest left for orders added later, `try_add_order`). Freeze's (`freeze_in`) and a trace's layer's
/// (`layer_write::index`, one of these a call): one builder.
#[allow(clippy::too_many_arguments)]
fn build_indexes(block: &Block, keys: &[Vec<u16>], room: usize, every: &[u16], sort: Option<usize>, val: &dyn Fn(usize, usize) -> i64, order: &[u32], lo: &[i64], hi: &[i64], br: usize, scratch: &crate::scratch::Scratch) -> *mut Index {
    let ixp: *mut crate::relation::Index = block.alloc_t(room.max(keys.len()).max(1));
    for (x, icols) in keys.iter().enumerate() {
        // SAFETY: entry `x` of the directory just allocated.
        build_one_index(block, unsafe { &mut *ixp.add(x) }, icols, every, sort, val, order, lo, hi, br, scratch);
    }
    ixp
}

/// Builds one index of `build_indexes` into its directory entry `ix`: a trace's layer builds each as an item of its
/// own (`layer_write::index`).
#[allow(clippy::too_many_arguments)]
fn build_one_index(block: &Block, ix: &mut Index, icols: &[u16], every: &[u16], sort: Option<usize>, val: &dyn Fn(usize, usize) -> i64, order: &[u32], lo: &[i64], hi: &[i64], br: usize, scratch: &crate::scratch::Scratch) {
    let live = order.len();
    {
        assert!(icols.len() <= MAX_KEY);
        ix.ncols = icols.len() as u32;
        for (j, &c) in icols.iter().enumerate() {
            ix.key[j] = c;
        }
        if icols.len() == 1 && sort == Some(icols[0] as usize) {
            ix.ordered = 1;
            return;
        }
        let kv = |j: usize| -> [i64; MAX_KEY] {
            let mut k = [0i64; MAX_KEY];
            for (t, &c) in icols.iter().enumerate() {
                k[t] = val(c as usize, order[j] as usize);
            }
            k
        };
        let nk = icols.len();
        // Distinct keys: a bitmap over the key's packed range when it is at most 2^24, else the hashes sorted.
        let ibits: [u32; MAX_KEY] = core::array::from_fn(|t| icols.get(t).map_or(0, |&c| bits_for((hi[c as usize] as i128 - lo[c as usize] as i128) as u64)));
        let span: u32 = ibits.iter().sum();
        let distinct = if live == 0 {
            0
        } else if icols == every {
            live
        } else if span <= 24 {
            let words: &mut [u64] = scratch.zeroed((1usize << span).div_ceil(64));
            let mut d = 0usize;
            for j in 0..live {
                let k = kv(j);
                let mut p = 0u64;
                for (t, &c) in icols.iter().enumerate() {
                    p = (p << ibits[t]) | (k[t] - lo[c as usize]) as u64;
                }
                let (w, b) = ((p >> 6) as usize, p & 63);
                d += (words[w] >> b & 1 == 0) as usize;
                words[w] |= 1 << b;
            }
            d
        } else {
            let hs: &mut [u64] = scratch.take(live);
            for (j, h) in hs.iter_mut().enumerate() {
                *h = key_hash(&kv(j)[..nk]);
            }
            distinct_hashes(scratch, hs)
        };
        let cap = distinct.next_power_of_two().max(1);
        let mask = (cap - 1) as u64;
        // One counting pass: each row's bucket, the buckets' sizes, then the rows placed in order.
        let bucket: &mut [u32] = scratch.take(live);
        let tags: &mut [u32] = scratch.take(live);
        // Exact claims: the buckets' counts start at zero, written so; every row and mark is written below. The rows,
        // then their marks (decision 0671).
        let bp: *mut u32 = block.try_claim_clear_t(cap + 1).expect("room in the block for an index's buckets");
        let rp: *mut u32 = block.try_claim_t(2 * live.max(1)).expect("room in the block for an index's rows");
        let offs = unsafe { std::slice::from_raw_parts_mut(bp, cap + 1) };
        for (j, b) in bucket.iter_mut().enumerate() {
            let h = key_hash(&kv(j)[..nk]);
            *b = (h & mask) as u32;
            tags[j] = mark_tag(h);
            offs[*b as usize + 1] += 1;
        }
        for b in 0..cap {
            offs[b + 1] += offs[b];
        }
        let at: &mut [u32] = scratch.take(cap);
        at.copy_from_slice(&offs[..cap]); // copy: not block data (each bucket's write cursor, in scratch)
        let rows_out = unsafe { std::slice::from_raw_parts_mut(rp, live.max(1)) };
        for (j, &b) in bucket.iter().enumerate() {
            rows_out[at[b as usize] as usize] = (br + j) as u32;
            at[b as usize] += 1;
        }
        // SAFETY: the marks' half of the rows' claim, `live` of them, written here alone.
        let marks = unsafe { std::slice::from_raw_parts_mut(rp.add(live), live) };
        let rows_out = &rows_out[..live];
        let j = |t: usize| rows_out[t] as usize - br;
        fill_marks(offs, marks, &|t| tags[j(t)], &|t, u| kv(j(t))[..nk] == kv(j(u))[..nk]);
        ix.mask = mask;
        unsafe {
            block.span(&raw const ix.offs, bp, cap + 1);
            block.span(&raw const ix.rows, rp, live);
            block.span(&raw const ix.marks, rp.add(live), live);
        }
    }
}

/// `freeze`, its temporaries from `scratch`. How, for `n` rows of `a` columns:
///
/// - each column is read once, a chunk at a time, into `n` values;
/// - unless the rows lie in order on the sort column, or (a full relation's) on another, the sort column, when there
///   is one, is radix sorted with the row ids, stably (`distinct_sorted`);
/// - a duplicate is a row equal to one before it in a table of the rows' hashes; a row the base holds is found in
///   the base's row index;
/// - each column is written back in order, a chunk at a time;
/// - an index's buckets are filled by one counting pass over the rows in order, its bucket count from the
///   number of distinct keys (a bitmap of the key's range, when that is small, else a table of their hashes).
pub fn freeze_in(block: &Block, rel: &Relation, spec: &Spec, base: Option<&Relation>, scratch: &crate::scratch::Scratch) -> FreezeStats {
    assert!(!rel.is_frozen());
    let a = rel.arity as usize;
    // Its rows are moved by value: a column under key runs is written in its runs' order by its writer, never sorted.
    assert!((0..a).all(|c| !rel.rows.col(c).has_runs()), "a relation of key runs frozen");
    let n = rel.rows.len() as usize;
    let before = block.used();
    let br = rel.base_rows as usize;

    // Dead bits: its kills, over the base's rows and its own.
    let total = br + n;
    let dead: &mut [u64] = scratch.zeroed(total.div_ceil(64).max(1));
    let nkilled = rel.killed.len();
    let mut kills_base = false;
    for k in 0..nkilled {
        let id = rel.killed.get(k, 0) as usize;
        dead[id >> 6] |= 1 << (id & 63);
        kills_base |= id < br;
    }
    let is_dead = |dead: &[u64], id: usize| dead[id >> 6] >> (id & 63) & 1 == 1;

    // Each column read once, and its range over the rows.
    let cols: &mut [i64] = scratch.take(n * a);
    let mut lo = vec![i64::MAX; a];
    let mut hi = vec![i64::MIN; a];
    for c in 0..a {
        let col = &mut cols[c * n..(c + 1) * n];
        rel.rows.read_col(c, n as u32, col);
        for &v in col.iter() {
            lo[c] = lo[c].min(v);
            hi[c] = hi[c].max(v);
        }
    }
    let val = |c: usize, i: usize| cols[c * n + i];

    // The rows left once duplicates go, and the column they lie sorted on: a full relation's the spec's, or one its
    // rows came in order on; a delta's its base's, so a view merges the two on one column.
    let want = match base {
        Some(b) => b.sort_col(),
        None => spec.sort.map(|c| c as usize),
    };
    let ids: &mut [u32] = scratch.take(n);
    let mut m = 0;
    for i in 0..n {
        if nkilled == 0 || !is_dead(dead, br + i) {
            ids[m] = i as u32;
            m += 1;
        }
    }
    let (m, sort) = distinct_sorted(scratch, val, a, want.map(|c| (c, lo[c], hi[c])), base.is_none(), &mut ids[..m]);
    rel.set_sort_col(sort);
    let ids = &ids[..m];

    // Set semantics: a row once, and in a delta, not a row its base holds. The base row equal to own row `i`, if
    // the base holds one: one probe of the base's row index, else of its sort column's range, else a scan (a base
    // of more than `MAX_KEY` columns and no sort column).
    let every: Vec<u16> = (0..a as u16).collect();
    let base_ix = base.and_then(|b| b.find_index(&every, b.rows.len()));
    let base_row = |i: usize| -> Option<u32> {
        let b = base?;
        let same = |id: u32| (0..a).all(|c| b.rows.get(id, c) == val(c, i));
        if let Some(x) = base_ix {
            let mut key = [0i64; MAX_KEY];
            for (c, k) in key.iter_mut().enumerate().take(a) {
                *k = val(c, i);
            }
            return b.group(x, key_hash(&key[..a])).iter().copied().find(|&id| same(id));
        }
        let (lo, hi) = match b.sort_col() {
            None => (0, b.rows.len()),
            Some(c) => {
                let lo = b.rows.lower_bound(c, 0, b.rows.len(), val(c, i));
                (lo, b.rows.upper_from(c, lo, b.rows.len(), val(c, i)))
            }
        };
        (lo..hi).find(|&id| same(id))
    };
    // An own row its base holds is dropped. When the delta killed that base row, the row pushed again brings it
    // back (the user, 2026-11-24: resurrected rather than inserted again): its kill is cancelled, so its id stays
    // what it was and the change is the least one. A base row dead in the base itself is not brought back: a full
    // relation's rows are all live, so none is.
    let order: &mut [u32] = scratch.take(m);
    let back: &mut [u32] = scratch.take(m);
    let (mut live, mut nback) = (0, 0);
    for &id in ids {
        let i = id as usize;
        if let Some(b) = base_row(i) {
            let based = base.expect("a base row's base");
            if !based.dead_bit(b) && is_dead(dead, b as usize) {
                back[nback] = b;
                nback += 1;
            }
            continue;
        }
        order[live] = i as u32;
        live += 1;
    }
    for &b in &back[..nback] {
        dead[b as usize >> 6] &= !(1u64 << (b & 63));
    }
    // The change: each base row killed that was live in the base and stays killed, ascending, once.
    let changed: &mut [u32] = scratch.take(nkilled as usize);
    let mut nchanged = 0;
    if let Some(b) = base {
        for k in 0..nkilled {
            let id = rel.killed.get(k, 0) as usize;
            if id < br && !b.dead_bit(id as u32) && is_dead(dead, id) {
                changed[nchanged] = id as u32;
                nchanged += 1;
            }
        }
    }
    let changed = &mut changed[..nchanged];
    changed.sort_unstable();
    let changed = {
        let mut u = 0;
        for j in 0..changed.len() {
            if j == 0 || changed[j] != changed[j - 1] {
                changed[u] = changed[j];
                u += 1;
            }
        }
        &changed[..u]
    };
    let order = &order[..live];
    let dropped = (m - live) as u64;

    // Sorted in place: each column written back in its order, so an own row's id is its place in that
    // order. No order array, and no dead bits for its own rows (the dropped ones are gone). The statistics are
    // each column's range over the live rows.
    let moved: &mut [i64] = scratch.take(live);
    let sp: *mut ColStats = block.alloc_t(a);
    for c in 0..a {
        let (mut l, mut h) = (i64::MAX, i64::MIN);
        for (d, &i) in moved.iter_mut().zip(order) {
            let v = val(c, i as usize);
            *d = v;
            l = l.min(v);
            h = h.max(v);
        }
        rel.rows.write_col(c, moved);
        unsafe { *sp.add(c) = ColStats { min: l, max: h } };
    }
    rel.rows.len.store(live as u32, Release);

    // Indexes: each key's rows grouped, in id order within a group; then the row index, on every column, which a
    // delta's freeze finds its base's rows in (when the relation has at most `MAX_KEY` columns and no declared index
    // is on them all). Only a full relation is a delta's base, so only a full relation has one. A relation with no
    // declared index is given its row index as an added one, after it is frozen (below), so its directory keeps its
    // room for the indexes its readers add (`try_reserve_index`).
    let row_index = base.is_none() && a > 0 && a <= MAX_KEY && !spec.indexes.contains(&every);
    let mut keys: Vec<Vec<u16>> = spec.indexes.clone();
    if row_index && !keys.is_empty() {
        keys.push(every.clone());
    }
    let ixp = build_indexes(block, &keys, keys.len().max(1), &every, sort, &val, order, &lo, &hi, br, scratch);

    // Dead bits: only the base's rows a delta killed.
    unsafe {
        if kills_base {
            let base_words = br.div_ceil(64);
            // The last word's bits past the base are own ids, which moved: none of them is dead.
            let tail = br % 64;
            if tail != 0 {
                dead[base_words - 1] &= (1u64 << tail) - 1;
            }
            let words: *mut u64 = block.alloc_t(base_words);
            std::ptr::copy_nonoverlapping(dead.as_ptr(), words, base_words);
            block.span(&rel.dead, words, base_words);
        }
        if !changed.is_empty() {
            let cp: *mut u32 = block.alloc_t(changed.len());
            // copy: derived: the change, computed here from the kills, written once where it is kept
            std::ptr::copy_nonoverlapping(changed.as_ptr(), cp, changed.len());
            block.span(&rel.changed, cp, changed.len());
        }
        block.span(&rel.indexes, ixp, keys.len());
        block.span(&rel.stats, sp, a);
    }
    rel.nindex.store(keys.len() as u32, Relaxed);
    rel.live.store(live as u32, Relaxed);
    rel.flags.fetch_or(FROZEN, Release);
    if row_index && keys.is_empty() && live > 0 {
        // SAFETY: frozen just now, by its only writer; its rows `0..live` are written.
        unsafe {
            let room = rel.try_reserve_index(block, &every, live as u32).expect("block full: a row index");
            rel.build_index(&room);
            rel.index_built(&room);
        }
    }
    FreezeStats { rows: n as u64, live: live as u64, dropped, bytes: (block.used() - before) as u64 }
}

/// The own rows of a frozen relation whose columns `cols` equal `key`, by its indexes: a range of its sorted rows when
/// `cols` starts with the column they lie sorted on (`leading`, checked against the rest of the key), else a hashed
/// index's bucket on exactly `cols` (checked against the key), else every own row checked; with how many rows it read.
pub fn own_matching<'r>(rel: &'r Relation, cols: &'r [u16], key: &'r [i64]) -> (impl Iterator<Item = u32> + 'r, usize) {
    let same = move |id: u32| cols.iter().zip(key).all(|(&c, &k)| rel.own(id, c as usize) == k);
    let lead = rel.sort_col().and_then(|s| cols.iter().position(|&c| c as usize == s));
    let (ids, read): (Ids<'r>, usize) = if let Some(p) = lead {
        let r = leading(rel, &cols[p..p + 1], &key[p..p + 1]);
        let n = r.len();
        (Ids::Range(rel.base_rows + r.start..rel.base_rows + r.end), n)
    } else if let Some(x) = rel.find_index(cols, rel.rows.len()) {
        let g = rel.group(x, key_hash(key));
        (Ids::Slice(g.iter()), g.len())
    } else {
        (Ids::Range(rel.ids()), rel.rows.len() as usize)
    };
    (ids.filter(move |&id| same(id)), read)
}

/// A relation's state as readers see it (P§12's `Version`; the rebuild plan's B2: "its View generalized from a base and
/// one delta to n layers"), read where its rows lie. Three forms, one definition:
///
/// - a full relation alone (`full`): its own rows, less its dead bits;
/// - a delta and its base in hand (`new`): ids `..base_rows` the base's, the rest the delta's; a base row is live
///   unless either marks it dead;
/// - a trace's state (`chain`): a layer and every layer below it, each a frozen continuation of the one below by block
///   id and offset (`Relation::far`), reached through a launch's descriptor table. Each layer's own ids are
///   `base_rows ..` its rows' end, and a row is live when no layer above its own, up to the state's, names it in its
///   kills (`Relation::changed`). Restricted to a partition (`States::restrict`), only that partition's rows: each
///   layer's range of them and its kills of them, by the layer's `parts` and `gone` (`layer::Layer`).
///
/// The empty state (`top` none) has no rows. A probe and the ordered scan come in order on the sort column (a chain's,
/// in the trace's canonical order). Its reads can be counted (`counted`): every row and liveness word they touch.
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub base: Option<&'a Relation>,
    /// Its top relation: `None` for the empty state.
    pub top: Option<&'a Relation>,
    pub(crate) arity: u32,
    /// A trace's state: the launch's descriptor table its layers are reached through, and the top layer's block.
    /// A trace's state (`View::chain`): the floor's, its layers reached through the launch's descriptor table, its
    /// restriction and its reads' count.
    pub(crate) chain: Option<rerail_delta_core::trace::State<'a>>,
}

impl<'a> View<'a> {
    /// A delta and its base (or a full relation alone, `base` none).
    pub fn new(base: Option<&'a Relation>, top: &'a Relation) -> View<'a> {
        View { base, top: Some(top), arity: top.arity, chain: None }
    }

    pub fn full(r: &'a Relation) -> View<'a> {
        View::new(None, r)
    }

    /// Its top relation: a relation's view's, not the empty state's.
    #[inline]
    pub fn rel(&self) -> &'a Relation {
        self.top.expect("a relation's view, not the empty state")
    }

    /// The same state, its reads counted into `reads`.
    pub fn counted(self, reads: &'a AtomicU64) -> View<'a> {
        View { chain: self.chain.map(|s| s.counted(reads)), ..self }
    }


    /// Whether it is a trace's state (`chain`).
    #[inline]
    pub fn is_chain(&self) -> bool {
        self.chain.is_some()
    }

    /// A trace's state's top layer's block: `None` for the empty state and for a relation's view.
    pub fn head(&self) -> Option<crate::gens::Id> {
        self.chain.and_then(|s| s.head())
    }

    /// The partition it is restricted to.
    pub fn part(&self) -> Option<i64> {
        self.chain.and_then(|s| s.part())
    }

    #[inline]
    pub fn arity(&self) -> usize {
        self.arity as usize
    }

    /// Every id, live or not: `0 .. total`.
    #[inline]
    pub fn total(&self) -> u32 {
        if self.is_chain() {
            // A trace's state: its newest layer's end, wherever its rows lie (`layer::OwnRows`).
            return self.chain_end();
        }
        self.top.map_or(0, |t| t.base_rows + t.rows.len())
    }

    #[inline]
    pub fn get(&self, id: u32, c: usize) -> i64 {
        if self.is_chain() {
            return self.chain_get(id, c);
        }
        let t = self.rel();
        if id < t.base_rows { self.base.expect("a delta's base").rows.get(id, c) } else { t.own(id, c) }
    }

    #[inline]
    pub fn live(&self, id: u32) -> bool {
        if self.is_chain() {
            return self.chain_live(id);
        }
        let Some(t) = self.top else { return false };
        if t.dead_bit(id) {
            return false;
        }
        if id < t.base_rows {
            return !self.base.expect("a delta's base").dead_bit(id);
        }
        // An open relation's own rows are live; a frozen one's dead bits cover its dropped duplicates.
        true
    }

    pub fn row(&self, id: u32) -> Vec<i64> {
        (0..self.arity()).map(|c| self.get(id, c)).collect()
    }

    /// Two rows by the sort column: equal when it has none (a merge then takes the base's row first).
    fn cmp(&self, x: u32, y: u32) -> Ordering {
        match self.rel().sort_col() {
            None => Ordering::Equal,
            Some(c) => self.get(x, c).cmp(&self.get(y, c)),
        }
    }

    /// Live ids in order: the base's (less what the delta killed) merged with the delta's on the sort column, the
    /// base's first on a tie; with no sort column, the base's, then the delta's.
    pub fn ordered(&self) -> Vec<u32> {
        self.ordered_iter().collect()
    }

    /// `ordered`, read in place: no allocation. A relation and its one base (a chain's is `bistate::Read::scan`).
    pub fn ordered_iter(&self) -> impl Iterator<Item = u32> + '_ {
        assert!(!self.is_chain(), "an ordered read of a relation and its one base (a chain's is `bistate::Read::scan`)");
        assert!(self.rel().is_frozen(), "an ordered read needs a frozen relation");
        let own = self.rel().base_rows..self.rel().base_rows + self.rel().rows.len();
        let based = 0..self.base.map_or(0, |b| b.rows.len());
        Merge::new(self, based.filter(|&id| self.live(id)), own)
    }

    pub fn live_len(&self) -> usize {
        if self.is_chain() || self.top.is_none() {
            return (0..self.total()).filter(|&id| self.live(id)).count();
        }
        let own = self.rel().rows.len() as usize;
        match self.base {
            None => own,
            Some(b) => own + (0..b.rows.len()).filter(|&id| self.live(id)).count(),
        }
    }

    /// The live rows whose index-`i` columns equal `key`, in order (`ordered`'s).
    pub fn probe(&self, i: usize, key: &[i64]) -> Vec<u32> {
        self.probe_iter(i, key).collect()
    }

    /// `probe`, read in place: both groups filtered and merged as they are walked, no allocation.
    pub fn probe_iter<'k>(&'k self, i: usize, key: &'k [i64]) -> Probe<'k> {
        assert!(!self.is_chain(), "a probe by index number reads a relation and its one base (a chain's is `bistate::Read::matching`)");
        let ix = self.rel().index_at(i);
        let cols = &ix.key[..ix.ncols as usize];
        let (own, based, exact) = if ix.ordered == 1 {
            let own = leading(self.rel(), cols, key);
            let based = self.base.map_or(0..0, |b| leading(b, cols, key));
            (Ids::Range(self.rel().base_rows + own.start..self.rel().base_rows + own.end), Ids::Range(based), true)
        } else {
            let h = key_hash(key);
            let based: &[u32] = self.base.map_or(&[], |b| b.group(i, h));
            (Ids::Slice(self.rel().group(i, h).iter()), Ids::Slice(based.iter()), false)
        };
        let own0 = self.rel().rows.reader(cols[0] as usize);
        let base0 = self.base.map_or(own0, |b| b.rows.reader(cols[0] as usize));
        Probe { v: *self, top: self.rel(), key, cols, own0, base0, exact, own, based, ha: None, hb: None, primed: false }
    }

    /// A digest of the live rows: the same for the same set, whatever wrote it, in any order (a sum of each row's
    /// hash, so no order is needed; `Packed::digest` is the same).
    pub fn digest(&self) -> u64 {
        let (mut sum, mut n) = (0u64, 0u64);
        for id in (0..self.total()).filter(|&id| self.live(id)) {
            sum = sum.wrapping_add(row_digest((0..self.arity()).map(|c| self.get(id, c))));
            n += 1;
        }
        crate::mix(sum, n as i64)
    }
}

/// A probe's answer, in order (`View::ordered`'s): a frozen relation's own rows (always live: its dead bits name only its
/// base's rows), and when it is a delta, its base's live rows, merged. A leading key's ranges are exact; a bucket's
/// rows are checked against the key.
pub struct Probe<'k> {
    v: View<'k>,
    top: &'k Relation,
    /// The key, and its first column resolved once: the delta's (or the full relation's) own, and its base's. A
    /// key of more columns checks the rest by `cols` (`rest_match`).
    key: &'k [i64],
    cols: &'k [u16],
    own0: crate::rows::ColRead,
    base0: crate::rows::ColRead,
    exact: bool,
    own: Ids<'k>,
    based: Ids<'k>,
    ha: Option<u32>,
    hb: Option<u32>,
    primed: bool,
}

/// A key's columns past its first, checked on `rows` at `row`.
#[cold]
#[inline(never)]
fn rest_match(rows: &Rows, cols: &[u16], key: &[i64], row: u32) -> bool {
    cols.iter().zip(key).skip(1).all(|(&c, &k)| rows.get(row, c as usize) == k)
}

impl Probe<'_> {
    #[inline(always)]
    fn next_own(&mut self) -> Option<u32> {
        let br = self.top.base_rows;
        loop {
            let id = self.own.next()?;
            if self.exact {
                return Some(id);
            }
            let row = id - br;
            if self.own0.get(row) == self.key[0] && (self.cols.len() == 1 || rest_match(&self.top.rows, self.cols, self.key, row)) {
                return Some(id);
            }
        }
    }

    #[inline]
    fn next_based(&mut self) -> Option<u32> {
        let b = self.v.base?;
        loop {
            let id = self.based.next()?;
            if self.top.dead_bit(id) {
                continue;
            }
            if self.exact || (self.base0.get(id) == self.key[0] && (self.cols.len() == 1 || rest_match(&b.rows, self.cols, self.key, id))) {
                return Some(id);
            }
        }
    }
}

impl Iterator for Probe<'_> {
    type Item = u32;

    #[inline(always)]
    fn next(&mut self) -> Option<u32> {
        if self.v.base.is_none() {
            return self.next_own();
        }
        if !self.primed {
            self.primed = true;
            self.ha = self.next_based();
            self.hb = self.next_own();
        }
        let take_a = match (self.ha, self.hb) {
            (Some(x), Some(y)) => self.v.cmp(x, y) != Ordering::Greater,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => return None,
        };
        if take_a {
            let x = self.ha;
            self.ha = self.next_based();
            x
        } else {
            let y = self.hb;
            self.hb = self.next_own();
            y
        }
    }
}

/// Ids from a hash bucket, or a range of sorted rows.
enum Ids<'a> {
    Slice(std::slice::Iter<'a, u32>),
    Range(std::ops::Range<u32>),
}

impl Iterator for Ids<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        match self {
            Ids::Slice(s) => s.next().copied(),
            Ids::Range(r) => r.next(),
        }
    }
}



/// Two id streams, each in order on the sort column, merged into one.
struct Merge<'v, A: Iterator<Item = u32>, B: Iterator<Item = u32>> {
    v: &'v View<'v>,
    a: std::iter::Peekable<A>,
    b: std::iter::Peekable<B>,
}

impl<'v, A: Iterator<Item = u32>, B: Iterator<Item = u32>> Merge<'v, A, B> {
    fn new(v: &'v View<'v>, a: A, b: B) -> Self {
        Merge { v, a: a.peekable(), b: b.peekable() }
    }
}

impl<A: Iterator<Item = u32>, B: Iterator<Item = u32>> Iterator for Merge<'_, A, B> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        match (self.a.peek(), self.b.peek()) {
            (Some(&x), Some(&y)) => {
                if self.v.cmp(x, y) != Ordering::Greater { self.a.next() } else { self.b.next() }
            }
            (Some(_), None) => self.a.next(),
            _ => self.b.next(),
        }
    }
}


#[cfg(test)]
mod tests {
    /// A relation released in a free-listed block (decision 0342) gives its chunks back: the next relation of its
    /// shape takes them, so the block grows by the descriptors alone, and reads its own rows.
    #[test]
    fn a_released_relations_chunks_are_taken_by_the_next() {
        let block = Block::anon(kind::FACTS, 64 << 20).with_free_list();
        let spec = Spec { widths: vec![8, 8, 8], sort: None, indexes: Vec::new(), min_log: crate::rows::MIN_LOG };
        let fill = |r: &Relation, base: i64| {
            assert!(r.rows.try_reserve(&block, 50_000));
            unsafe {
                r.rows.claim_reserved(50_000);
                for i in 0..50_000u32 {
                    for c in 0..3 {
                        r.rows.put(i, c, base + i as i64 * 3 + c as i64);
                    }
                }
            }
        };
        let a = Relation::try_create(&block, &spec, None).unwrap();
        fill(a, 0);
        let after_one = block.used();
        unsafe { a.release(&block) };
        assert!(block.free_bytes() >= 50_000 * 24, "its chunks are free: {}", block.free_bytes());
        let b = Relation::try_create(&block, &spec, None).unwrap();
        fill(b, 1_000_000);
        assert_eq!(block.used(), after_one, "the next took its chunks and its descriptor: nothing new from the front");
        assert_eq!(b.rows.get(49_999, 2), 1_000_000 + 49_999 * 3 + 2);
    }

    use super::*;
    use crate::block::kind;
    use crate::ids::ColumnIndex;
    use crate::table::Table;
    use std::collections::BTreeSet;

    /// Decision 0671: a place's mark is its key's tag, with `MARK_SAME` where its bucket's previous place of the tag
    /// (among the `MARK_REACH` before it) holds its key; another key of the tag there, or none, leaves it clear.
    #[test]
    fn a_places_mark_says_whether_its_previous_of_its_tag_holds_its_key() {
        // One bucket of 13 places: keys by place, tags by key (keys 1 and 2 share tag 6).
        let keys = [1, 1, 2, 2, 1, 3, 3, 4, 5, 6, 7, 8, 3];
        let tag = |k: i32| if k <= 2 { 6u32 } else { 10 * k as u32 };
        let offs = [0u32, keys.len() as u32];
        let mut marks = [0u32; 13];
        fill_marks(&offs, &mut marks, &|t| tag(keys[t]), &|t, u| keys[t] == keys[u]);
        let same: Vec<u32> = marks.iter().map(|m| m & MARK_SAME).collect();
        // A place's previous of its tag: the same key (places 1, 3, 6), another key of the tag (2, 4), none (0, 5, the
        // single keys); place 12's is place 6, 6 places back.
        assert_eq!(same, [0, 1, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 1]);
        assert!(marks.iter().zip(keys).all(|(m, k)| m & !MARK_SAME == tag(k)), "each place's tag");
    }

    /// A built index's marks: each place's tag its key's hash's, and each `MARK_SAME` place's key its previous place of
    /// its tag's.
    #[test]
    fn a_built_indexs_marks_are_its_keys() {
        let block = Block::anon(kind::FACTS, 16 << 20).with_free_list();
        let spec = Spec { widths: vec![8, 8], sort: None, indexes: Vec::new(), min_log: crate::rows::MIN_LOG };
        let r = Relation::try_create(&block, &spec, None).unwrap();
        let n = 5_000u32;
        assert!(r.rows.try_reserve(&block, u64::from(n)));
        unsafe {
            r.rows.claim_reserved(n);
            for i in 0..n {
                r.rows.put(i, 0, i64::from(i * 7 % 997));
                r.rows.put(i, 1, i64::from(i));
            }
        }
        // 1024 buckets for 997 keys, about one key a bucket (as an index is sized), some two or more.
        let room = unsafe { r.try_reserve_index_buckets(&block, &[0], n, 1024) }.unwrap();
        unsafe {
            r.build_index(&room);
            r.index_built(&room);
        }
        let ix = r.index_at(room.index);
        let (offs, rows, marks) = (ix.offs.as_slice(), ix.rows.as_slice(), ix.marks.as_slice());
        assert_eq!(marks.len(), rows.len());
        let key = |t: usize| r.rows.get(rows[t], 0);
        let mut known = 0;
        for b in 0..offs.len() - 1 {
            for t in offs[b] as usize..offs[b + 1] as usize {
                assert_eq!(marks[t] & !MARK_SAME, mark_tag(key_hash(&[key(t)])), "place {t}'s tag");
                if marks[t] & MARK_SAME != 0 {
                    let lo = (offs[b] as usize).max(t.saturating_sub(MARK_REACH));
                    let prev = (lo..t).rev().find(|&u| marks[u] & !MARK_SAME == marks[t] & !MARK_SAME).unwrap();
                    assert_eq!(key(prev), key(t), "place {t}: its previous of its tag's key");
                    known += 1;
                }
            }
        }
        assert!(known > n as usize / 2, "most places' keys known by the place before of their tag: {known}");
    }

    /// Decision 0384 step 7: a continuation reads its ids below `base_rows` in its base (in the same block) and its
    /// own after, by the read written once (`rel_get`); its index is of every id; a kills delta of it marks any; its
    /// base's rows written past `base_rows` (the base's own writer going on) are not its.
    #[test]
    fn a_continuation_reads_its_bases_rows_where_they_lie() {
        let block = Block::anon(kind::FACTS, 16 << 20).with_free_list();
        let spec = Spec { widths: vec![8, 8], sort: None, indexes: Vec::new(), min_log: crate::rows::MIN_LOG };
        let put = |r: &Relation, from: u32, n: u32, tag: i64| {
            assert!(r.rows.try_reserve(&block, u64::from(r.rows.len() + n)));
            let at = r.rows.len();
            unsafe {
                r.rows.claim_reserved(n);
                for i in 0..n {
                    r.rows.put(at + i, 0, i64::from((from + i) % 10));
                    r.rows.put(at + i, 1, tag + i64::from(from + i));
                }
            }
        };
        let base = Relation::try_create(&block, &spec, None).unwrap();
        put(base, 0, 100, 1000);
        let fork = Relation::try_continue(&block, &spec, base, 100).expect("room in the base's block");
        assert!(fork.continues() && std::ptr::eq(fork.base().unwrap(), base));
        put(fork, 100, 50, 2000);
        // The base's writer goes on past the fork's start: rows the fork does not read.
        put(base, 100, 30, 9000);
        assert_eq!(fork.len_all(), 150);
        for id in 0..150 {
            assert_eq!(fork.get(id, 1), if id < 100 { 1000 } else { 2000 } + i64::from(id), "row {id}");
        }
        // An index of every id it reads.
        let n = fork.len_all();
        let room = unsafe { fork.try_reserve_index(&block, &[0], n) }.expect("room");
        unsafe {
            fork.build_index(&room);
            fork.index_built(&room);
        }
        let i = fork.find_index(&[0], n).expect("built");
        let key = |v: i64| [v].iter().fold(crate::SEED, |h, &x| crate::mix(h, x));
        let mut sevens: Vec<u32> = fork.group(i, key(7)).iter().copied().filter(|&id| fork.get(id, 0) == 7).collect();
        sevens.sort_unstable();
        assert_eq!(sevens, (0..150).filter(|id| id % 10 == 7).collect::<Vec<u32>>(), "its base's rows and its own");
        // Kills of any of its ids.
        let kills = Relation::try_kills(&block, &spec, fork, &[1 << 3, 0, 1 << 4]).expect("a delta");
        assert!(kills.dead_bit(3) && kills.dead_bit(132) && !kills.dead_bit(4));
        // A continuation of a continuation (a fork of a fork, step 8): ids read down the chain. Below its base's base
        // rows, or past what its base reads, it is refused; a kills delta is no base.
        assert!(Relation::try_continue(&block, &spec, fork, 10).is_none());
        assert!(Relation::try_continue(&block, &spec, fork, 151).is_none());
        assert!(Relation::try_continue(&block, &spec, kills, 0).is_none());
        let fork2 = Relation::try_continue(&block, &spec, fork, 120).expect("a continuation of a continuation");
        put(fork2, 120, 40, 3000);
        // The first fork's writer goes on past 120: rows the second does not read.
        put(fork, 150, 5, 8000);
        assert_eq!(fork2.len_all(), 160);
        for id in 0..160 {
            let want = if id < 100 { 1000 } else if id < 120 { 2000 } else { 3000 } + i64::from(id);
            assert_eq!(fork2.get(id, 1), want, "row {id} down the chain");
        }
        let n = fork2.len_all();
        let room = unsafe { fork2.try_reserve_index(&block, &[0], n) }.expect("room");
        unsafe {
            fork2.build_index(&room);
            fork2.index_built(&room);
        }
        let i = fork2.find_index(&[0], n).expect("built");
        let mut threes: Vec<u32> = fork2.group(i, key(3)).iter().copied().filter(|&id| fork2.get(id, 0) == 3).collect();
        threes.sort_unstable();
        assert_eq!(threes, (0..160).filter(|id| id % 10 == 3).collect::<Vec<u32>>(), "every id down the chain");
        // Kept, a continuation is packed with every row it reads (decision 0456 K3: `kept_facts::columns`, tested
        // there as `a_continuation_is_kept_with_every_row_it_reads`).
    }

    /// Decision 0384 step 5: an added index's buckets and rows are chunks of a free-listed block, which forgetting it
    /// (`forget_indexes`, a relation whose rows changed in place) gives back, and the next index takes again: a
    /// relation probed after every change of its rows grows its block by nothing. Released, the relation gives back
    /// its indexes and their directory too.
    #[test]
    fn a_forgotten_index_gives_its_room_back() {
        let block = Block::anon(kind::FACTS, 16 << 20).with_free_list();
        let spec = Spec { widths: vec![8, 8], sort: None, indexes: Vec::new(), min_log: crate::rows::MIN_LOG };
        let rel = Relation::try_create(&block, &spec, None).unwrap();
        rel.rewritten();
        let n = 20_000u32;
        assert!(rel.rows.try_reserve(&block, u64::from(n)));
        unsafe {
            rel.rows.claim_reserved(n);
            for i in 0..n {
                rel.rows.put(i, 0, i64::from(i % 97));
                rel.rows.put(i, 1, i64::from(i));
            }
        }
        let index = |r: &Relation| unsafe {
            let room = r.try_reserve_index(&block, &[0], n).expect("room");
            r.build_index(&room);
            r.index_built(&room);
        };
        index(rel);
        assert!(rel.find_index(&[0], n).is_some());
        let used = block.used();
        let free = block.free_bytes();
        unsafe { rel.forget_indexes(&block) };
        assert!(rel.find_index(&[0], n).is_none(), "forgotten");
        assert!(block.free_bytes() >= free + 4 * n as usize, "its buckets and rows back on the free list: {} from {free}", block.free_bytes());
        for _ in 0..20 {
            index(rel);
            unsafe { rel.forget_indexes(&block) };
        }
        index(rel);
        assert_eq!(block.used(), used, "each index again in the room the last gave back");
        let i = rel.find_index(&[0], n).expect("built again");
        let group = rel.group(i, [5i64].iter().fold(crate::SEED, |h, &v| crate::mix(h, v)));
        assert_eq!(group.iter().filter(|&&r| rel.rows.get(r, 0) == 5).count(), (0..n).filter(|i| i % 97 == 5).count(), "a key's rows");
        let free = block.free_bytes();
        unsafe { rel.release(&block) };
        assert!(block.free_bytes() >= free + 4 * n as usize + 16 * n as usize, "rows, index and directory given back");
    }

    /// An index of an empty relation holds buckets and no rows room: its rows span points nowhere (`RelSpan::set_bits`,
    /// a span of no items). Forgotten, given back and released, it gives back its buckets and reads no address from
    /// the empty span (`offset_of` past the block, which panicked), with and without a rewritten relation's chunks; one
    /// made again takes the same room.
    #[test]
    fn an_index_of_no_rows_holds_no_rows_room_and_goes_back_cleanly() {
        for rewritten in [false, true] {
            let block = Block::anon(kind::FACTS, 16 << 20).with_free_list();
            let spec = Spec { widths: vec![8, 8], sort: None, indexes: Vec::new(), min_log: crate::rows::MIN_LOG };
            let rel = Relation::try_create(&block, &spec, None).unwrap();
            if rewritten {
                rel.rewritten();
            }
            let index = |r: &Relation| unsafe {
                let room = r.try_reserve_index(&block, &[0], 0).expect("room");
                r.build_index(&room);
                r.index_built(&room);
            };
            index(rel);
            let i = rel.find_index(&[0], 0).expect("built");
            assert!(rel.group(i, crate::mix(crate::SEED, 5)).is_empty(), "no rows in any group");
            let used = block.used();
            unsafe { rel.forget_indexes(&block) };
            index(rel);
            assert_eq!(block.used(), used, "{rewritten}: made again in the buckets the last gave back");
            let room = unsafe { rel.try_reserve_index(&block, &[1], 0) }.expect("room");
            unsafe { rel.give_back_index(&block, &room) };
            unsafe { rel.release(&block) };
        }
    }

    /// Rows as a sorted set (each once), to compare sets.
    fn as_set(rows: &[Vec<i64>]) -> Vec<Vec<i64>> {
        let set: BTreeSet<Vec<i64>> = rows.iter().cloned().collect();
        assert_eq!(set.len(), rows.len(), "a row twice");
        let mut by: Vec<Vec<i64>> = set.into_iter().collect();
        by.sort_by_key(|r| (r[1], r[0], r[2]));
        by
    }

    /// Whether `rows` are in order on column `c`.
    fn sorted_on(rows: &[Vec<i64>], c: usize) -> bool {
        rows.windows(2).all(|w| w[0][c] <= w[1][c])
    }

    fn rows_of(v: &View) -> Vec<Vec<i64>> {
        v.ordered_iter().map(|id| v.row(id)).collect()
    }

    /// An index added after the relation was made (`try_reserve_index`, `build_index`): found by its key once built
    /// (not before), and each key's group, read by the read written once (`group`), holds exactly that key's rows
    /// in id order, for a key of one column and one of eight. A full directory refuses.
    #[test]
    fn an_added_index_groups_each_keys_rows_in_id_order() {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let lock = Mutex::new(());
        let spec = Spec { widths: vec![8; 9], sort: None, indexes: Vec::new(), min_log: 4 };
        let rel = Relation::create(&block, &spec, None);
        let rows: Vec<[i64; 9]> = (0..2_000i64).map(|i| std::array::from_fn(|c| if c == 0 { i % 37 } else { (i * 7 + c as i64) % 5 - 2 })).collect();
        RelW { block: &block, rel, lock: &lock }.extend(&rows.iter().flatten().copied().collect::<Vec<_>>());
        let n = rel.rows.len();
        for cols in [vec![0u16], (1..9).collect::<Vec<u16>>()] {
            assert_eq!(rel.find_index(&cols, n), None);
            // SAFETY: one thread; the rows are written.
            let room = unsafe { rel.try_reserve_index(&block, &cols, n) }.expect("room");
            assert_eq!(rel.find_index(&cols, n), None, "not found before it is built");
            unsafe {
                rel.build_index(&room);
                rel.index_built(&room);
            }
            let i = rel.find_index(&cols, n).expect("found once built");
            for probe in [0usize, 1, 5, 36, 1999] {
                let key: Vec<i64> = cols.iter().map(|&c| rows[probe][c as usize]).collect();
                let want: Vec<u32> = (0..n).filter(|&r| cols.iter().zip(&key).all(|(&c, &v)| rows[r as usize][c as usize] == v)).collect();
                let got: Vec<u32> = rel.group(i, crate::key_hash(&key)).iter().copied().filter(|&r| cols.iter().zip(&key).all(|(&c, &v)| rel.rows.get(r, c as usize) == v)).collect();
                assert_eq!(got, want, "key {key:?} on {cols:?}");
            }
        }
        for c in 2..ADDED_INDEXES as u16 {
            assert!(unsafe { rel.try_reserve_index(&block, &[c], n) }.is_some());
        }
        assert!(unsafe { rel.try_reserve_index(&block, &[0, 1], n) }.is_none(), "the directory is full");
    }

    /// An index of rows `from..` (a delta read in place) is found by its first row as well as its count, never for
    /// the relation's first rows; an index given back (`give_back_index`) is found no more, its room and its entry
    /// taken by the next reservation, and the directory's count does not shrink.
    #[test]
    fn an_index_is_of_its_rows_from_and_one_given_back_is_taken_again() {
        let block = Block::anon(kind::FACTS, 16 << 20).with_free_list();
        let lock = Mutex::new(());
        let spec = Spec { widths: vec![8; 2], sort: None, indexes: Vec::new(), min_log: 4 };
        let rel = Relation::create(&block, &spec, None);
        let rows: Vec<i64> = (0..2_000i64).flat_map(|i| [i % 37, i]).collect();
        RelW { block: &block, rel, lock: &lock }.extend(&rows);
        // SAFETY: one thread; the rows are written. Built as the device builds them: rows `from + t`.
        let full = unsafe { rel.try_reserve_index(&block, &[0], 1_000) }.expect("room");
        unsafe { rel.index_built(&full) };
        let delta = unsafe { rel.try_reserve_index_from(&block, &[0], 1_000, 1_000) }.expect("room");
        unsafe { rel.index_built(&delta) };
        assert_eq!(rel.find_index(&[0], 1_000), Some(full.index), "the first rows' index");
        assert_eq!(rel.find_index_from(&[0], 1_000, 1_000), Some(delta.index), "the delta's, not the first rows'");
        assert_eq!(rel.find_index_from(&[0], 500, 1_000), None);
        let (count, used) = (rel.nindex.load(Relaxed), block.used());
        // SAFETY: one thread; no reader holds it.
        unsafe { rel.give_back_index(&block, &full) };
        assert_eq!(rel.find_index(&[0], 1_000), None, "given back");
        assert_eq!(rel.find_index_from(&[0], 1_000, 1_000), Some(delta.index), "the other kept");
        let again = unsafe { rel.try_reserve_index(&block, &[1], 1_000) }.expect("room");
        assert_eq!(again.index, full.index, "the entry given back is taken");
        assert_eq!(block.used(), used, "and its room");
        assert_eq!(rel.nindex.load(Relaxed), count, "the directory's count kept");
        unsafe { rel.index_built(&again) };
        assert_eq!(rel.find_index(&[1], 1_000), Some(again.index));
        // Forgotten, every entry, the dead one too, goes; none is given back twice.
        unsafe { rel.give_back_index(&block, &again) };
        unsafe { rel.forget_indexes(&block) };
        assert_eq!(rel.find_index_from(&[0], 1_000, 1_000), None);

        // A block of no free list (a relation alone in its block): the entry keeps its room, and an index that fits
        // takes it, the block's bytes as they were; one that does not fit takes new room.
        let block = Block::anon(kind::FACTS, 16 << 20);
        let rel = Relation::create(&block, &spec, None);
        RelW { block: &block, rel, lock: &lock }.extend(&rows);
        let twice = unsafe { rel.try_reserve_index(&block, &[0, 1], 1_000) }.expect("room");
        unsafe { rel.index_built(&twice) };
        let used = block.used();
        unsafe { rel.give_back_index(&block, &twice) };
        assert_eq!(rel.find_index(&[0, 1], 1_000), None, "given back");
        let other = unsafe { rel.try_reserve_index(&block, &[0], 900) }.expect("room");
        assert_eq!((other.index, other.offs, other.rows), (twice.index, twice.offs, twice.rows), "its room taken again");
        assert_eq!(block.used(), used);
        unsafe {
            rel.build_index(&other);
            rel.index_built(&other);
        }
        let i = rel.find_index(&[0], 900).expect("built");
        let got: Vec<u32> = rel.group(i, crate::key_hash(&[5])).iter().copied().filter(|&r| rel.rows.get(r, 0) == 5).collect();
        assert_eq!(got, (0..900).filter(|r| r % 37 == 5).collect::<Vec<u32>>(), "read right from the room taken again");
        let bigger = unsafe { rel.try_reserve_index(&block, &[1], 2_000) }.expect("room");
        assert_ne!(bigger.index, twice.index);
        assert!(block.used() > used);
    }

    /// Freeze keeps a set sorted on its one column, a delta's set being its base's less its kills plus its own rows;
    /// the probes answer as a scan does, in the same order. Small values and values too wide to pack together.
    #[test]
    fn freeze_and_probes_agree_with_a_set() {
        for wide in [false, true] {
            let spec = Spec { widths: vec![8, 4, 4], sort: Some(1), indexes: vec![vec![1], vec![2]], min_log: 4 };
            let big = if wide { 1i64 << 50 } else { 1 };
            let mut x = 7u64;
            let mut next = |m: u64| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x % m) as i64
            };
            let rows: Vec<[i64; 3]> = (0..3_000).map(|_| [next(40) * big, next(30), next(500)]).collect();
            let block = Block::anon(kind::FACTS, 64 << 20);
            let lock = Mutex::new(());
            let rel = Relation::create(&block, &spec, None);
            RelW { block: &block, rel, lock: &lock }.extend(&rows.iter().flatten().copied().collect::<Vec<_>>());
            freeze(&block, rel, &spec, None);
            let set: BTreeSet<(i64, i64, i64)> = rows.iter().map(|r| (r[1], r[0], r[2])).collect();
            let v = View::full(rel);
            let want: Vec<Vec<i64>> = set.iter().map(|&(b, a, c)| vec![a, b, c]).collect();
            assert_eq!(as_set(&rows_of(&v)), want, "wide {wide}: the set, each row once");
            assert!(sorted_on(&rows_of(&v), 1), "wide {wide}: sorted on its column");

            // A delta: kills of base rows and of its own, repeats of base rows, new rows.
            let dblock = Block::anon(kind::FACTS, 16 << 20);
            let d = Relation::create(&dblock, &spec, Some(rel));
            let w = RelW { block: &dblock, rel: d, lock: &lock };
            let mut now = set.clone();
            for id in (0..rel.rows.len()).step_by(97) {
                w.kill(id);
                let r = v.row(id);
                now.remove(&(r[1], r[0], r[2]));
            }
            for id in (5..rel.rows.len()).step_by(211) {
                let r = v.row(id);
                w.push(&r);
                now.insert((r[1], r[0], r[2]));
            }
            for i in 0..300 {
                let r = [next(50) * big, 30 + next(5), i];
                let own = w.push(&r);
                if i % 7 == 0 {
                    w.kill(own);
                } else {
                    now.insert((r[1], r[0], r[2]));
                }
            }
            freeze(&dblock, d, &spec, Some(rel));
            let dv = View::new(Some(rel), d);
            let want: Vec<Vec<i64>> = now.iter().map(|&(b, a, c)| vec![a, b, c]).collect();
            assert_eq!(as_set(&rows_of(&dv)), want, "wide {wide}: the delta's set");
            assert!(sorted_on(&rows_of(&dv), 1), "wide {wide}: the delta's rows merged on its column");
            assert_eq!(dv.live_len(), now.len());
            let order: Vec<u32> = dv.ordered();
            for (ix, col) in [(0usize, 1usize), (1, 2)] {
                for k in [0i64, 3, 17, 31, 33, 250, 499, 1_000] {
                    let got: Vec<u32> = dv.probe_iter(ix, &[k]).collect();
                    let scan: Vec<u32> = order.iter().copied().filter(|&id| dv.get(id, col) == k).collect();
                    assert_eq!(got, scan, "wide {wide}: probe of column {col} for {k}, in the ordered read's order");
                }
            }
        }
    }

    /// Decision 0678 step 8. With no sort column, freeze keeps the rows in the order they came, each once. A delta
    /// finds the base row a pushed row equals through the base's row index, whether the base has declared indexes
    /// (it is appended to them) or none (it is added, and the directory keeps room for more); a row the delta killed
    /// and pushed again is brought back. A base wider than `MAX_KEY` has no row index and is searched instead.
    #[test]
    fn rows_are_sorted_on_one_column_or_none_and_found_by_their_row_index() {
        let lock = Mutex::new(());
        // No sort: arrival order, duplicates gone.
        let spec = Spec { widths: vec![8, 8], sort: None, indexes: Vec::new(), min_log: 4 };
        let block = Block::anon(kind::FACTS, 16 << 20);
        let rel = Relation::create(&block, &spec, None);
        let came = [[5i64, 1], [3, 9], [5, 1], [1, 2], [3, 9], [9, 0]];
        RelW { block: &block, rel, lock: &lock }.extend(&came.iter().flatten().copied().collect::<Vec<_>>());
        freeze(&block, rel, &spec, None);
        assert_eq!(rows_of(&View::full(rel)), vec![vec![5, 1], vec![3, 9], vec![1, 2], vec![9, 0]], "in the order they came, each once");
        assert!(rel.find_index(&[0, 1], 4).is_some(), "a row index, added");
        assert!(unsafe { rel.try_reserve_index(&block, &[1], 4) }.is_some(), "room for an index a reader adds");
        assert_eq!(rel.sort_col(), None, "in order on no column");

        // Rows that come in order on a column other than the one asked for (a join's output, in its probe side's
        // order) stay in it, and it is the column recorded; with none asked for, likewise.
        for want in [Some(0), None] {
            let spec = Spec { widths: vec![8, 8], sort: want, indexes: vec![vec![0], vec![1]], min_log: 4 };
            let block = Block::anon(kind::FACTS, 16 << 20);
            let rel = Relation::create(&block, &spec, None);
            let came = [[5i64, 1], [3, 2], [5, 2], [3, 2], [1, 7]];
            RelW { block: &block, rel, lock: &lock }.extend(&came.iter().flatten().copied().collect::<Vec<_>>());
            freeze(&block, rel, &spec, None);
            let v = View::full(rel);
            assert_eq!(rows_of(&v), vec![vec![5, 1], vec![3, 2], vec![5, 2], vec![1, 7]], "{want:?}: as they came, each once");
            assert_eq!((rel.sort_col(), v.sort()), (Some(1), Some(ColumnIndex::new(1))), "{want:?}: column 1 recorded");
            assert_eq!(v.probe(1, &[2]), vec![1, 2], "{want:?}: column 1's index is its order");
            assert_eq!(v.probe(0, &[5]), vec![0, 2], "{want:?}: column 0's is hashed");
            // A delta lies in its base's order, whatever order its own rows came in.
            let dblock = Block::anon(kind::FACTS, 16 << 20);
            let d = Relation::create(&dblock, &spec, Some(rel));
            let dw = RelW { block: &dblock, rel: d, lock: &lock };
            dw.push(&[0, 9]);
            dw.push(&[9, 0]);
            dw.push(&[8, 3]);
            freeze(&dblock, d, &spec, Some(rel));
            let dv = View::new(Some(rel), d);
            assert_eq!(d.sort_col(), Some(1));
            assert_eq!(rows_of(&dv), vec![vec![9, 0], vec![5, 1], vec![3, 2], vec![5, 2], vec![8, 3], vec![1, 7], vec![0, 9]], "{want:?}: merged on column 1");
        }

        for (indexes, widths) in [(Vec::new(), vec![8u32, 8]), (vec![vec![0u16]], vec![8, 8]), (Vec::new(), vec![8; 9])] {
            let a = widths.len();
            let spec = Spec { widths, sort: Some(0), indexes, min_log: 4 };
            let block = Block::anon(kind::FACTS, 16 << 20);
            let rel = Relation::create(&block, &spec, None);
            // In order on no column: sorted on column 0.
            let row = |i: i64| (0..a as i64).map(|c| if c == 0 { i % 7 } else { (i * 37 % 200) * 10 + c }).collect::<Vec<i64>>();
            let w = RelW { block: &block, rel, lock: &lock };
            for i in 0..200 {
                w.push(&row(i));
            }
            freeze(&block, rel, &spec, None);
            let every: Vec<u16> = (0..a as u16).collect();
            assert_eq!(rel.find_index(&every, 200).is_some(), a <= MAX_KEY, "{a} columns: a row index when it fits");
            let v = View::full(rel);
            let id_of = |i: i64| (0..200).find(|&id| v.row(id) == row(i)).expect("a row");
            let dblock = Block::anon(kind::FACTS, 16 << 20);
            let d = Relation::create(&dblock, &spec, Some(rel));
            let dw = RelW { block: &dblock, rel: d, lock: &lock };
            let (gone, back, again) = (id_of(3), id_of(40), id_of(41));
            dw.kill(gone);
            dw.kill(back);
            dw.push(&row(40));
            dw.push(&row(41));
            dw.push(&row(500));
            freeze(&dblock, d, &spec, Some(rel));
            let dv = View::new(Some(rel), d);
            assert!(!dv.live(gone) && dv.live(back) && dv.live(again), "{a} columns: the kill of a row pushed again is cancelled");
            assert_eq!(d.rows.len(), 1, "{a} columns: only the new row is the delta's own");
            assert_eq!(d.changed.as_slice(), &[gone], "{a} columns: the change is the one kill that stands");
            assert!(sorted_on(&rows_of(&dv), 0));
        }
    }
}
