//! `join_run`, the join program (`join_stages.uglykl` with `JOIN_PROGRAM`, decisions 0603, 0674): a chain of its
//! dispatches, a phase's groups a dispatch, a group an item. Written by hand in Rust: the oracle the Rust that rerail-ukl-vec writes from the same source is held to (decision 0567), in results and in
//! the shape of its code.
//!
//! Its shape is the source's, read as a CPU runs it. A group is `JOIN_TG` (256) threads, and an item's work is a
//! function of a thread (`stage_count_one`, `dedup_claim_one`, ...), so the group's work is a loop over its threads,
//! each a call of that function: one function a source function, small, its branches and loops a thread's own. What
//! a SIMD group does together on a GPU is here what one thread does alone: a text's bytes walked by an automaton one
//! after another (`chunk_states`' prefix over the lanes is a GPU's way to walk them), a scan's tile a running sum, a
//! sort's tile a stable count. Barriers order nothing: the group runs in one thread.
//!
//! The reads are written directly in Rust (decisions 0534, 0536; the user's): a view's rows read off its tape where
//! it lies and its projection's numbering (`reads`, over `rerail_reads`), a relation's off its descriptor in its
//! block (`crate::lanes`), the rolling cache's by its host lookups (`rerail_reads::view::cache`). Each is held to
//! the CLIF the JIT and the kernels run by a test.

// Every function here reads and writes the device's buffers through raw pointers, as the source's do.
#![allow(unsafe_op_in_unsafe_fn, clippy::too_many_arguments, clippy::missing_safety_doc)]

use crate::Launch;
use crate::lanes as ln;
use rerail_reads::view::cache;
use rerail_simd::prelude::*;
use rerail_simd::{LaneCount, SimdElement, SupportedLaneCount};
use core::sync::atomic::{AtomicU32, Ordering, Ordering::SeqCst};

const NONE: u32 = u32::MAX;
/// A finisher's phases run itself at most, a dispatch (`JOIN_INLINE`, decision 0674), of at most
/// `JOIN_INLINE_GROUPS` groups each.
const JOIN_INLINE: u32 = 256;
const JOIN_INLINE_GROUPS: u32 = 4;
/// A group's threads (`JOIN_TG`).
const TG: u32 = 256;
/// A text row's SIMD group (`SGW`): its rows and their counts are laid out by it.
const SGW: u32 = 32;
use crate::lanes::KEY_SEED;
/// A scan's tile: `UKL_SCAN_ITEMS` values a thread of the group.
const SCAN_ITEMS: u32 = 4;
/// A sort's tile (`UKL_RADIX_TILE`) and its digits.
const RADIX_TILE: u32 = 1024;
const RADIX_DIGITS: usize = 256;
const VIEW_PLACED: u32 = 16;
const IX_LIVE: u32 = 0x8000_0000;

// What an item runs (`Item::what >> 24`).
const PK_COPY: u32 = 1;
const PK_IX_KEY: u32 = 2;
const PK_RADIX_HIST: u32 = 3;
const PK_RADIX_SCATTER: u32 = 4;
const PK_IX_PLACE: u32 = 5;
const PK_IX_OFFS: u32 = 30;
const PK_IX_MARK: u32 = 31;
const PK_DEDUP_KEY: u32 = 32;
/// A round's new rows placed in its set head's map after its gate (code 7 of `lat_rehash_one`), its items its rows'.
const PK_SET_PLACE: u32 = 33;
const PK_DEDUP_CLEAR: u32 = 6;
const PK_DEDUP_CLAIM: u32 = 7;
const PK_MAP: u32 = 8;
const PK_STAGE_COUNT: u32 = 9;
const PK_SCAN_COUNT: u32 = 10;
const PK_WITNESS_COUNT: u32 = 11;
const PK_ROW_TEXT: u32 = 12;
const PK_DEDUP_MARK: u32 = 13;
const PK_SCAN_SUMS: u32 = 14;
const PK_SCAN_TILES: u32 = 15;
const PK_STAGE_SCATTER: u32 = 16;
const PK_SCAN_SCATTER: u32 = 17;
const PK_ROW_SCATTER: u32 = 18;
const PK_DEDUP_SCATTER: u32 = 19;
const PK_LAT_HCLEAR: u32 = 20;
const PK_LAT_REHASH: u32 = 21;
const PK_LAT_CLEAR: u32 = 22;
const PK_LAT_CLAIM: u32 = 23;
const PK_LAT_BEST: u32 = 24;
const PK_LAT_MERGE: u32 = 25;
const PK_SUM: u32 = 26;
const PK_GATE: u32 = 27;
const PK_ROUND: u32 = 28;
const PK_SIZE: u32 = 29;

// ---- The records, as the source lays them out (its sizes and offsets asserted there, and here). ----

#[repr(C)]
struct Stage {
    key_const: [i64; 8],
    par_const: [i64; 16],
    r_at: u64,
    r_dead: u64,
    ops: [[u32; 4]; 64],
    mode: u32,
    nslots: u32,
    cap_in: u32,
    cap_out: u32,
    in_slot: u32,
    out_slot: u32,
    nr: u32,
    nkeys: u32,
    r_index: u32,
    ntests: u32,
    nops: u32,
    nparams: u32,
    nbinds: u32,
    npassed: u32,
    last: u32,
    counter: u32,
    key_kind: [u32; 8],
    key_slot: [u32; 8],
    key_rcol: [u32; 8],
    test_a: [u32; 8],
    test_b: [u32; 8],
    col_kind: [u32; 16],
    col_idx: [u32; 16],
    col_doc: [u32; 16],
    par_kind: [u32; 16],
    par_slot: [u32; 16],
    par_doc: [u32; 16],
    bind_slot: [u32; 16],
    bind_kind: [u32; 16],
    bind_idx: [u32; 16],
    passed: [u32; 16],
    outputs: [u32; 16],
    noutputs: u32,
    ncols: u32,
    agg: [u32; 2],
    arith: [[u32; 4]; 8],
    v_dir: *const ViewDir,
    r_view: u32,
    v_parent: u32,
    screen: u32,
    screen_at: u32,
    v_set: *const i64,
    v_has: u32,
    v_nset: u32,
    r_from: u32,
    r_len: u32,
}
const _: () = assert!(size_of::<Stage>() == 2352 && core::mem::offset_of!(Stage, ops) == 208);
const _: () = assert!(core::mem::offset_of!(Stage, arith) == 2176 && core::mem::offset_of!(Stage, v_dir) == 2304);

use rerail_reads::views::{NO_ROW, ViewDir, ViewRow};
#[repr(C)]
struct PlacedDir {
    dir: *const ViewDir,
    tail: *const u8,
    at: u64,
    rows: u32,
    pad: u32,
}
const _: () = assert!(size_of::<ViewRow>() == 304 && size_of::<ViewDir>() == 192 && size_of::<PlacedDir>() == 32);
const _: () = assert!(core::mem::offset_of!(ViewRow, source) == 24 && core::mem::offset_of!(ViewRow, homes) == 32 && core::mem::offset_of!(ViewRow, view) == 156);

#[repr(C)]
struct Copy {
    konst: [i64; 16],
    src: [u32; 16],
    n: u32,
    ncols: u32,
    src_stride: u32,
    dst_stride: u32,
    dst_off: u32,
    count_slot: u32,
    src_from: u32,
    pad1: u32,
    off: *const u32,
    src_at: u64,
    dst_rows_at: u64,
    v_dir: *const ViewDir,
    r_view: u32,
    flags: u32,
}
const _: () = assert!(size_of::<Copy>() == 264 && core::mem::offset_of!(Copy, v_dir) == 248);

#[repr(C)]
struct Dedup {
    cap: u32,
    in_slot: u32,
    out_slot: u32,
    ncols: u32,
    mask: u32,
    cols: [u32; 16],
    ocap: u32,
    places: u32,
    /// Its rows less those a relation holds, through its map (`dedup_held`), its rows out placed in it (decision 0567):
    /// 1. 2: the map read only, its rows out placed in it after its head's gate once written (code 7 of
    /// `lat_rehash_one`), so a park may grow it.
    seen: u32,
    hmask: u32,
    /// Its rows made distinct in the head's first order (t58 increment A, `dedup_key_one`): no places.
    sorted: u32,
    r_at: u64,
    r_rows_at: u64,
    h_at: u64,
}
const _: () = assert!(size_of::<Dedup>() == 128);

#[repr(C)]
struct Job {
    l: *const i64,
    r: *const i64,
    totals: *mut u32,
    counters: *mut u32,
    o: *mut i64,
    a: *mut u32,
    d: *const u8,
    f: *const u8,
    w: [*const i64; 3],
    g: *const u8,
    s_: *mut u8,
    vc: *mut u8,
    v: *mut i64,
    h: *mut u32,
    s: *const Stage,
    ws: *const Stage,
    bs: *const Stage,
    dd: *const Dedup,
    p: *const u32,
    cnt: *const u32,
    // Its words (`JobWords`): base, n, kb, eb, l_at, o_at, v_at, h_at.
    at: [u32; 8],
    // The blocks the far bases of the relations it reads lie in (decision 0353 step 6, `far_of`): `rf` the stage's
    // `r`'s (a copy's or an index's, `l`'s), `wf` the witness probes'. Null: the relation's own block.
    rf: *const u8,
    wf: [*const u8; 3],
}
const _: () = assert!(size_of::<Job>() == 240);

#[repr(C)]
struct IxP {
    n: u32,
    ncols: u32,
    at: u64,
    mask: u64,
    cols: [u32; 8],
    dir: *const ViewDir,
    kind: u64,
    from: u32,
    pad: u32,
}
const _: () = assert!(size_of::<IxP>() == 80 && core::mem::offset_of!(IxP, dir) == 56);

#[repr(C)]
#[derive(Clone)]
struct LatP {
    top: i64,
    n: u32,
    had: u32,
    nkeys: u32,
    code: u32,
    after: u32,
    pmask: u32,
    hmask: u32,
    level: u32,
    omask: u32,
    counted: u32,
    r_at: u64,
    r_rows_at: u64,
    h_at: u64,
    h0_at: u64,
}
const _: () = assert!(size_of::<LatP>() == 80);

#[repr(C)]
struct SumPart {
    src: *const u32,
    off: *mut u32,
}
#[repr(C)]
struct SumP {
    m: u32,
    pad: u32,
    total: *mut u32,
    part: [SumPart; 1],
}
#[repr(C)]
struct GateCheck {
    count: *const u32,
    plus: *const u32,
    cap: u32,
    has: u32,
}
#[repr(C)]
struct GateP {
    m: u32,
    pad: [u32; 3],
    check: [GateCheck; 1],
}
#[repr(C)]
struct RoundOp {
    to: *mut u32,
    from: *const u32,
    op: u32,
    n: u32,
}
#[repr(C)]
struct RoundP {
    round: *mut u32,
    nops: u32,
    max: u32,
    next: u32,
    narrow: u32,
    ops: [RoundOp; 1],
}
#[repr(C)]
#[derive(Clone, core::marker::Copy)]
struct Item {
    what: u32,
    group: u32,
    phase: u32,
    seg: u32,
}
#[repr(C)]
struct Seg {
    count: *const u32,
    first: u32,
    items: u32,
    per: u32,
    plus: u32,
}
#[repr(C)]
struct ScanLevel {
    input: *mut u32,
    sums: *mut u32,
    // Its words (`ScanWords`): n, offsets, at, sums_at, summed, pad.
    w: [u32; 6],
}
#[repr(C)]
struct SortPass {
    keys: *const u32,
    values: *const u32,
    hist: *const u32,
    out_keys: *mut u32,
    out_values: *mut u32,
    // Its words (`SortWords`): n, shift, tiles, wide (its keys 64 bits: a sorted dedup's).
    w: [u32; 4],
    pad2: u64,
}
#[repr(C)]
#[derive(Clone, core::marker::Copy)]
struct ProgP {
    phases: u32,
    dispatches: u32,
    end: u32,
    pad: u32,
}
#[repr(C)]
struct SizeOp {
    w: *mut u32,
    c: u64,
    op: u32,
    k: u32,
    x: u32,
    y: u32,
}
const _: () = assert!(size_of::<Item>() == 16 && size_of::<Seg>() == 24 && size_of::<ScanLevel>() == 40);
const _: () = assert!(size_of::<SortPass>() == 64 && size_of::<SizeOp>() == 32);

// ---- Memory: a word's atomics, a column's value. ----

#[inline]
unsafe fn at<'a>(p: *mut u32) -> &'a AtomicU32 {
    AtomicU32::from_ptr(p)
}

/// Column `c`'s value of row `t` in a table of columns `stride` rows apart.
#[inline]
unsafe fn col(l: *const i64, c: u32, stride: u32, t: u32) -> i64 {
    *l.add(c as usize * stride as usize + t as usize)
}

#[inline]
unsafe fn col_put(o: *mut i64, c: u32, stride: u32, t: u32, v: i64) {
    *o.add(c as usize * stride as usize + t as usize) = v;
}

// ---- The reads, written directly in Rust: each a relation's or a cache's at `t + a0`, as the source calls them. ----

#[inline(always)]
fn at_(t: *const u8, a0: u64) -> *const u8 {
    t.wrapping_add(a0 as usize)
}
#[inline(always)]
unsafe fn rows_len(t: *const u8, a0: u64) -> u32 {
    ln::rows_len(at_(t, a0))
}
#[inline(always)]
unsafe fn rows_put(t: *const u8, a0: u64, r: u32, c: u32, v: u64) -> u32 {
    ln::rows_put(at_(t, a0), r, c, v)
}
/// Column `c` of row `r` of the relation at `t + a0`, a base of its chain in another block read at `far`, that
/// block's start (`far_block`).
#[inline(always)]
unsafe fn rel_get(t: *const u8, a0: u64, r: u32, c: u32, far: *const u8) -> u64 {
    ln::one::rel_get(at_(t, a0), r, c, far)
}
#[inline(always)]
unsafe fn rel_dead(t: *const u8, a0: u64, r: u32) -> u32 {
    u32::from(ln::one::rel_dead(at_(t, a0), r))
}
#[inline(always)]
unsafe fn rel_len(t: *const u8, a0: u64) -> u32 {
    ln::rel_len(at_(t, a0))
}
/// Index `index`'s group of key hash `h`: its first place in the index's rows, above its length.
#[inline(always)]
unsafe fn rel_group(t: *const u8, a0: u64, index: u32, h: u64) -> u64 {
    let (start, len) = ln::one::rel_group(at_(t, a0), index, h);
    u64::from(start) << 32 | u64::from(len)
}
#[inline(always)]
unsafe fn rel_group_row(t: *const u8, a0: u64, index: u32, q: u32) -> u32 {
    ln::one::rel_group_row(at_(t, a0), index, q)
}
#[inline(always)]
unsafe fn rel_group_mark(t: *const u8, a0: u64, index: u32, q: u32) -> u32 {
    ln::rel_group_mark(at_(t, a0), index, q)
}
#[inline(always)]
fn key_mix(h: u64, v: i64) -> u64 {
    ln::one::key_mix(h, v as u64)
}
#[inline(always)]
fn key_hash(kv: &[i64]) -> u64 {
    ln::one::key_hash(kv)
}
#[inline(always)]
fn bloom_hash(key: u64) -> u64 {
    ln::one::bloom_hash(key)
}
#[inline(always)]
unsafe fn bloom_may(t: *const u8, a0: u64, h: u64) -> u32 {
    u32::from(ln::one::bloom_may(at_(t, a0), h))
}
#[inline(always)]
unsafe fn atoms_len(t: *const u8, a0: u64, v: u32) -> u32 {
    reads::atoms_len(at_(t, a0), v)
}
#[inline(always)]
unsafe fn atoms_at(t: *const u8, a0: u64, v: u32) -> u64 {
    reads::atoms_str(at_(t, a0), v) as u64
}
/// The `k`th atom whose slot has hash `h`'s high half and whose string is `len` bytes (decision 0666), else NONE.
#[inline(always)]
unsafe fn atoms_find(t: *const u8, a0: u64, h: u64, len: u32, k: u32) -> u32 {
    reads::atoms_find(at_(t, a0), h, len, k)
}
#[inline(always)]
unsafe fn verdict_get(t: *const u8, a0: u64, test: u32, v: u32) -> u32 {
    cache::verdict_get(at_(t, a0), test, v)
}
#[inline(always)]
unsafe fn verdict_put(t: *const u8, a0: u64, test: u32, v: u32, verdict: u32) -> u32 {
    u32::from(cache::verdict_put(at_(t, a0), test, v, verdict))
}
/// The text kept in the cache: its entry, what was found in its low bits.
#[inline(always)]
unsafe fn text_find(t: *const u8, a0: u64, f: u32, sym: u32) -> u64 {
    cache::text_find(at_(t, a0), f, sym)
}
#[inline(always)]
unsafe fn text_open(t: *const u8, a0: u64, f: u32, sym: u32, len: u32) -> u64 {
    cache::text_open(at_(t, a0), f, sym, len)
}
#[inline(always)]
unsafe fn cache_done(t: *const u8, a0: u64, entry: u64) {
    cache::done(at_(t, a0), entry)
}

// ---- Views read in place (decision 0353 step 4d). ----

unsafe fn view_block(d: *const ViewDir) -> *const u8 {
    (d as *const u8).wrapping_sub((*d).at as usize)
}
unsafe fn view_rows(d: *const ViewDir) -> *const ViewRow {
    view_block(d).add((*d).rows as usize) as *const ViewRow
}
/// The first row of view `k` past document `e`'s: the next in the directory's order's (`ViewRow::at`, `row_at`), `end`
/// past the last.
unsafe fn view_rows_end(d: *const ViewDir, e: *const ViewRow, k: usize, end: u32) -> u32 {
    let m = (*e).at + 1;
    if m < (*d).ndocs { (*view_rows(d).add(ViewDir::row_at(d, m) as usize)).rows[k] } else { end }
}

/// Row `e`'s document of directory `d`, as the views' reads take it (`reads::doc`).
#[inline(always)]
unsafe fn doc<'a>(d: *const ViewDir, e: *const ViewRow) -> rerail_reads::view_reads::Doc<'a> {
    reads::doc(d, e.offset_from(view_rows(d)) as u32)
}

unsafe fn vrow_dir(kind: u32, d: *const ViewDir, r: u32, f: &Found<'_>) -> Row {
    let k = kind - 1;
    let lo = ViewDir::doc_of_row(d, k, r, if f.d == d { f.lo } else { NONE });
    let i = r.wrapping_sub((*view_rows(d).add(lo as usize)).rows[k as usize]);
    Row::View(d, lo, k.min(reads::LAST_VIEW), i)
}

/// The document a walk's view reads last found (`Row::get`): its directory, its row there and its reads (one `Doc`,
/// which keeps the node it last selected). A walk's rows come in order, most of them in the document of the row
/// before: such a row neither searches the directory again nor makes the document's reads again, and the row after
/// the one before takes its node by the reads' walk (`Doc::tape_node`), not a select.
struct Found<'a> {
    d: *const ViewDir,
    lo: u32,
    doc: Option<rerail_reads::view_reads::Doc<'a>>,
}

impl<'a> Found<'a> {
    /// None found yet.
    fn new() -> Self {
        Found { d: core::ptr::null(), lo: NONE, doc: None }
    }

    /// Directory `d`'s document `lo`'s reads: the one found, else made (and kept).
    #[inline]
    unsafe fn doc(&mut self, d: *const ViewDir, lo: u32) -> &rerail_reads::view_reads::Doc<'a> {
        if self.d != d || self.lo != lo || self.doc.is_none() {
            *self = Found { d, lo, doc: Some(doc(d, view_rows(d).add(lo as usize))) };
        }
        match &self.doc {
            Some(doc) => doc,
            None => unreachable!("made just above"),
        }
    }
}

/// A row of a view or a relation, found once, so that each of its columns is read from it: a view row's directory,
/// document (its row there), view (`views::VIEWS`) and row in the document, read through the walk's `Found`; or a
/// relation's row (`t + a0`'s row `r`, its far base's block `far`).
#[derive(Clone, Copy)]
enum Row {
    View(*const ViewDir, u32, u32, u32),
    Rel(*const u8, u64, u32, *const u8),
}

impl Row {
    #[inline(always)]
    unsafe fn get(&self, col: u32, f: &mut Found<'_>) -> i64 {
        match *self {
            Row::View(d, lo, k, i) => reads::col(f.doc(d, lo), k, i, col) as i64,
            Row::Rel(t, a0, r, far) => rel_get(t, a0, r, col, far) as i64,
        }
    }
}

unsafe fn vrow(kind: u32, d: *const ViewDir, r: u32, f: &Found<'_>) -> Row {
    if kind & VIEW_PLACED == 0 {
        return vrow_dir(kind, d, r, f);
    }
    let p = d as *const PlacedDir;
    if r < (*p).rows {
        return vrow_dir(kind & !VIEW_PLACED, (*p).dir, r, f);
    }
    // The tail, one relation in its own block: no far base.
    Row::Rel((*p).tail, (*p).at, r - (*p).rows, (*p).tail)
}

/// View `kind`'s row `r`, column `col` (`vrow`'s).
#[inline]
unsafe fn vread(kind: u32, d: *const ViewDir, r: u32, col: u32, f: &mut Found<'_>) -> i64 {
    vrow(kind, d, r, f).get(col, f)
}

// ---- A stage's values (decision 0095). ----

/// The stage's parameters: what every function of a stage takes.
struct St<'a> {
    s: &'a Stage,
    l: *const i64,
    r: *const i64,
    /// The probed relation's far base (decision 0353 step 6): the block it lies in (`far_block`).
    far: *const u8,
    /// The view row last found (`row`) and its number: a binding reads several columns of each row it walks.
    last: core::cell::Cell<(u32, Option<Row>)>,
    /// The document its view reads last found (`Found`). Its reads point into the launch's buffers, which outlive
    /// every stage.
    found: core::cell::RefCell<Found<'static>>,
}

/// The block a relation's far base lies in, as the reads take it (`rel_get`, `lanes::rel_at`): `rf`, else its own.
/// The CPU's reads take addresses, not offsets in a buffer, so not `rel_b`'s offset from `r`.
fn far_block(r: *const u8, rf: *const u8) -> *const u8 {
    if rf.is_null() { r } else { rf }
}

impl<'a> St<'a> {
    /// Stage `s` over the left rows `l` and the relation `r`, whose chain's far base lies in `rf` (none: its own).
    fn new(s: &'a Stage, l: *const i64, r: *const i64, rf: *const u8) -> Self {
        St { s, l, r, far: far_block(r as *const u8, rf), last: core::cell::Cell::new((NONE, None)), found: core::cell::RefCell::new(Found::new()) }
    }
}

impl St<'_> {
    /// Column `col` of row `r` of what the stage walks: a view's row, found once for its reads of its columns (the
    /// last one found kept); a `has` walk's rows are `child`'s, its value the child's id. Else the relation's row.
    #[inline]
    unsafe fn rget(&self, r: u32, col: u32) -> i64 {
        let s = self.s;
        if s.v_has == 0 && s.r_view == 0 {
            return rel_get(self.r as *const u8, s.r_at, r, col, self.far) as i64;
        }
        let col = if s.v_has != 0 { 1 } else { col };
        let mut f = self.found.borrow_mut();
        if let (q, Some(row)) = self.last.get()
            && q == r
        {
            return row.get(col, &mut f);
        }
        let row = if s.v_has != 0 { vrow(1, s.v_dir, r, &f) } else { vrow(s.r_view, s.v_dir, r, &f) };
        self.last.set((r, Some(row)));
        row.get(col, &mut f)
    }

    unsafe fn slot(&self, k: u32, t: u32) -> i64 {
        col(self.l, k, self.s.cap_in, t)
    }

    unsafe fn colval(&self, k: u32, t: u32, r: u32) -> i64 {
        let (s, k) = (self.s, k as usize);
        let v = if s.col_kind[k] == 0 { self.rget(r, s.col_idx[k]) } else { self.slot(s.col_idx[k], t) };
        if s.col_doc[k] != 0 { v >> 32 } else { v }
    }

    unsafe fn parval(&self, k: u32, t: u32) -> i64 {
        let (s, k) = (self.s, k as usize);
        let v = if s.par_kind[k] == 0 { s.par_const[k] } else { self.slot(s.par_slot[k], t) };
        if s.par_doc[k] != 0 { v >> 32 } else { v }
    }

    /// Arithmetic bind `j`'s value, as the CPU's checked arithmetic gives it: none on an overflow or a zero divisor.
    unsafe fn arith(&self, j: u32, t: u32, r: u32) -> Option<i64> {
        let a = &self.s.arith[j as usize];
        let x = if a[3] & 1 != 0 { self.parval(a[1], t) } else { self.colval(a[1], t, r) };
        let y = if a[3] & 2 != 0 { self.parval(a[2], t) } else { self.colval(a[2], t, r) };
        match a[0] {
            0 => x.checked_add(y),
            1 => x.checked_sub(y),
            2 => x.checked_mul(y),
            3 => x.checked_div(y),
            4 => x.checked_rem(y),
            5 => Some(x.max(y)),
            6 => Some(x.min(y)),
            _ => node_order(self.s.v_dir, x, &mut { NONE }),
        }
    }

    unsafe fn defined(&self, t: u32, r: u32) -> bool {
        let s = self.s;
        (0..s.nbinds.min(16) as usize).all(|b| s.bind_kind[b] != 3 || self.arith(s.bind_idx[b], t, r).is_some())
    }

    /// The row program: bit x of the result is operation x's value.
    unsafe fn eval(&self, t: u32, r: u32) -> u64 {
        let s = self.s;
        let mut v = 0u64;
        for x in 0..s.nops.min(64) {
            let o = &s.ops[x as usize];
            let bit = |k: u32| (v >> (k & 63)) & 1;
            let b = match o[0] {
                0 => {
                    let w = (o[3] >> 8) & 255;
                    let a = lane_of(self.colval(o[1], t, r), w);
                    let c = if o[2] & 0x8000_0000 != 0 { self.parval(o[2] & 0xffff, t) } else { self.colval(o[2], t, r) };
                    let c = lane_of(c, w);
                    match o[3] & 255 {
                        0 => a == c,
                        1 => a != c,
                        2 => a < c,
                        _ => a <= c,
                    }
                }
                1 => o[1] != 0,
                2 => bit(o[1]) == 0,
                3 => bit(o[1]) & bit(o[2]) != 0,
                4 => bit(o[1]) | bit(o[2]) != 0,
                5 => bit(o[1]) ^ bit(o[2]) != 0,
                _ => bit(o[1]) & !bit(o[2]) & 1 != 0,
            };
            v |= (b as u64) << x;
        }
        v
    }

    fn out_bit(&self, v: u64, k: u32) -> bool {
        (v >> (self.s.outputs[k as usize] & 63)) & 1 != 0
    }

    fn key_value(&self, t: u32, k: u32) -> i64 {
        let (s, k) = (self.s, k as usize);
        // SAFETY: a binding's slot, as the source reads it.
        if s.key_kind[k] == 1 { unsafe { self.slot(s.key_slot[k], t) } } else { s.key_const[k] }
    }

    unsafe fn rows(&self) -> u32 {
        let s = self.s;
        if s.r_len == 0 {
            return s.nr;
        }
        let len = rel_len(self.r as *const u8, s.r_at);
        if len > s.r_from { (len - s.r_from).min(s.nr) } else { 0 }
    }

    unsafe fn row_live(&self, r: u32) -> bool {
        self.s.r_dead == 0 || rel_dead(self.r as *const u8, self.s.r_dead, r) == 0
    }

    /// Whether node row `q`'s kind atom is in the stage's set (ascending).
    unsafe fn has_member(&self, q: u32) -> bool {
        let s = self.s;
        // A `has` stage's rows are `node`'s (`rget`): a walk's candidate is the row it holds.
        let k = {
            let mut f = self.found.borrow_mut();
            let row = match self.last.get() {
                (p, Some(row)) if p == q => row,
                _ => vrow(1, s.v_dir, q, &f),
            };
            row.get(2, &mut f)
        };
        let set = core::slice::from_raw_parts(s.v_set, s.v_nset as usize);
        set.binary_search(&k).is_ok()
    }

    /// Whether row `r` has the binding's keys and passes the relation's column tests.
    /// `key`: its keys are known to be `kv` (its index's marks, decision 0671).
    unsafe fn matches(&self, r: u32, kv: &[i64; 8], key: bool) -> bool {
        let s = self.s;
        if s.v_has != 0 {
            return s.ntests == 0 && (s.v_has == 1 || self.has_member(r));
        }
        if !self.row_live(r) {
            return false;
        }
        (key || self.keys_are(r, kv)) && (0..s.ntests.min(8) as usize).all(|i| self.rget(r, s.test_a[i]) == self.rget(r, s.test_b[i]))
    }

    /// Whether a walk's candidates hold the binding's only key by their making: a parent's children (walk 2, a
    /// `v_parent` probe's), whose parent column (`child`'s 0) is the parent's id, checked once by the probe.
    fn parent_known(&self, how: u32) -> bool {
        how == 2 && self.s.nkeys == 1 && self.s.key_rcol[0] == 0
    }

    /// Whether row `r`'s keys are `kv`.
    unsafe fn keys_are(&self, r: u32, kv: &[i64; 8]) -> bool {
        (0..self.s.nkeys.min(8) as usize).all(|i| self.rget(r, self.s.key_rcol[i]) == kv[i])
    }

    /// A parent's children (`by_parent`) or a node's walk of `has` (`by_has`): its document, its numbered node.
    unsafe fn doc_node(&self, key: i64) -> Option<(*const ViewRow, u32)> {
        let d = self.s.v_dir;
        if key < 0 || (*d).ndocs == 0 {
            return None;
        }
        let mut f = self.found.borrow_mut();
        let a = ViewDir::doc_of_key(d, ((key as u64) >> 32) as u32, if f.d == d { f.lo } else { NONE });
        if a == NONE {
            return None;
        }
        let e = view_rows(d).add(a as usize);
        let l = f.doc(d, a).number(key as u32);
        (l != NONE).then_some((e, l))
    }

    /// The `node` or `child` view probed by its id (`Stage::v_parent` 2, `Batch::by_own`): the id's document and
    /// numbered node, its id checked (as a parent's is, `by_parent`), and its one row there, kept with its node as the
    /// stage's last (`rget`), so its other keys and columns read it with no search; as the host's probe by a node's
    /// own id (`Batch::node_hint`). None where no node has the id (a `child` row: the root has none).
    unsafe fn by_own(&self, kv: &[i64; 8]) -> Walk {
        let s = self.s;
        let none = Walk { lo: 0, hi: 0, how: 0, tag: 0 };
        let Some(i) = (0..s.nkeys.min(8) as usize).find(|&i| s.key_rcol[i] == 1) else { return none };
        let Some((e, l)) = self.doc_node(kv[i]) else { return none };
        let d = s.v_dir;
        let a = e.offset_from(view_rows(d)) as u32;
        let k = s.r_view - 1;
        if self.found.borrow_mut().doc(d, a).node_id(l) != kv[i] || (k == 1 && l == 0) {
            return none;
        }
        let local = l - k;
        let r = (*e).rows[k as usize].wrapping_add(local);
        self.last.set((r, Some(Row::View(d, a, k, local))));
        Walk { lo: r, hi: r.wrapping_add(1), how: 0, tag: 0 }
    }

    /// The candidates of keys `kv`: [lo, hi) and how they are walked (0 rows, 1 an index's entries, 2 a parent's
    /// children by their next siblings, 3 a node's numbered ancestors).
    unsafe fn probe(&self, kv: &[i64; 8]) -> Walk {
        let s = self.s;
        if s.nkeys == 0 {
            return Walk { lo: s.r_from, hi: s.r_from.wrapping_add(self.rows()), how: 0, tag: 0 };
        }
        let none = Walk { lo: 0, hi: 0, how: 0, tag: 0 };
        let d = s.v_dir;
        if s.v_has != 0 {
            let Some((e, l)) = self.doc_node(kv[0]) else { return none };
            let first = (*e).rows[0];
            if s.v_has == 1 {
                if !self.has_member(first.wrapping_add(l)) {
                    return none;
                }
                let u = doc(d, e).up(l);
                return if u == NONE { none } else { Walk { lo: first.wrapping_add(u), hi: NONE, how: 3, tag: 0 } };
            }
            return Walk { lo: first.wrapping_add(l).wrapping_add(1), hi: first.wrapping_add(doc(d, e).end(l)), how: 0, tag: 0 };
        }
        if s.v_parent == 2 {
            return self.by_own(kv);
        }
        if s.v_parent != 0 {
            let Some((e, l)) = self.doc_node(kv[0]) else { return none };
            let first = (*e).rows[1];
            if l == 0 && doc(d, e).entry.parented == 0 {
                let hi = view_rows_end(d, e, 1, s.nr);
                return Walk { lo: first, hi, how: 0, tag: 0 };
            }
            // Its children's parent column is its id (`parent_known`): the key is checked here once, not at each child.
            if doc(d, e).node_id(l) != kv[0] {
                return none;
            }
            let c = doc(d, e).child_first(l);
            return if c == NONE { none } else { Walk { lo: first.wrapping_add(c), hi: NONE, how: 2, tag: 0 } };
        }
        let h = key_hash(&kv[..s.nkeys.min(8) as usize]);
        let g = rel_group(self.r as *const u8, s.r_at, s.r_index, h);
        let lo = (g >> 32) as u32;
        Walk { lo, hi: lo.wrapping_add(g as u32), how: 1, tag: mark_tag(h) }
    }

    /// A binding's candidates and its keys' values; none where the probed relation's screen refuses the key.
    unsafe fn candidates(&self, t: u32, f: *const u8, kv: &mut [i64; 8]) -> Walk {
        let s = self.s;
        if s.mode == 3 {
            return Walk { lo: 0, hi: 1, how: 0, tag: 0 };
        }
        for i in 0..s.nkeys.min(8) {
            kv[i as usize] = self.key_value(t, i);
        }
        if s.screen != 0 && s.screen <= s.nkeys && s.screen <= 8 {
            let h = bloom_hash(kv[s.screen as usize - 1] as u64);
            if bloom_may(f, s.screen_at as u64, h) == 0 {
                return Walk { lo: 0, hi: 0, how: 0, tag: 0 };
            }
        }
        self.probe(kv)
    }

    /// Candidate `q`'s row.
    unsafe fn cand_row(&self, how: u32, q: u32) -> u32 {
        if how == 1 { rel_group_row(self.r as *const u8, self.s.r_at, self.s.r_index, q) } else { q }
    }

    /// A walk of the tape (`how` 2: a parent's children from `child` row `q`; 3: a node's numbered ancestors from
    /// `node` row `q`), each candidate's row given to `f` until it returns false: its document found once (`Found`),
    /// each step from the tape node in hand (the next sibling, the parent), as the host's walks take them
    /// (`each_child_node`, `Within::containers_on`), and each candidate's row kept as the stage's last (`row`) with
    /// its node known to its document's reads (`Doc::know`), so its columns' reads select none.
    unsafe fn walk_tape(&self, how: u32, q: u32, mut f: impl FnMut(u32) -> bool) {
        let d = self.s.v_dir;
        let k = if how == 2 { 1 } else { 0 };
        let a = {
            let fd = self.found.borrow();
            ViewDir::doc_of_row(d, k, q, if fd.d == d { fd.lo } else { NONE })
        };
        let first = (*view_rows(d).add(a as usize)).rows[k as usize];
        let mut i = q.wrapping_sub(first);
        let (mut x, parent, end) = {
            let mut fd = self.found.borrow_mut();
            let dd = fd.doc(d, a);
            // A `child` row's node is numbered node `i + 1`, a `node` row's node `i`.
            let x = dd.tape_node(i.wrapping_add(k));
            // A child's siblings end with its parent's subtree; one with no tape parent (a narrowed projection's) has
            // none (`clif_view_child_next`).
            let parent = if how == 2 { dd.tape.parent(x) } else { 0 };
            let end = if how == 2 && parent != NO_ROW { dd.tape.end_of(parent) } else { 0 };
            (x, parent, end)
        };
        loop {
            let r = first.wrapping_add(i);
            self.found.borrow_mut().doc(d, a).know(i.wrapping_add(k), x, NONE);
            self.last.set((r, Some(Row::View(d, a, k, i))));
            if !f(r) {
                return;
            }
            let mut fd = self.found.borrow_mut();
            let dd = fd.doc(d, a);
            // The next numbered sibling within the parent, or numbered strict ancestor.
            let mut c = match how {
                2 if parent == NO_ROW => NO_ROW,
                2 => dd.tape.next_in(x, end),
                _ => dd.tape.parent(x),
            };
            while c != NO_ROW && !dd.is(0, c) {
                c = if how == 2 { dd.tape.next_in(c, end) } else { dd.tape.parent(c) };
            }
            if c == NO_ROW {
                return;
            }
            x = c;
            i = dd.rank(c).wrapping_sub(k);
        }
    }

    /// The candidates of keys `kv`, in their walk's order: each one's row, place, and whether its keys are known to
    /// be `kv`. An index group's (decision 0671): only the places whose mark's tag is the key's, each one's key known
    /// by the walk's previous place of the tag where its mark says `MARK_SAME` (that place is the walk's last of the
    /// tag: it walks its places in order), else compared.
    unsafe fn each(&self, w: Walk, kv: &[i64; 8], mut f: impl FnMut(u32, u32, bool) -> bool) {
        let mut q = w.lo;
        let known = self.parent_known(w.how);
        if (w.how == 2 || w.how == 3) && q != NONE {
            self.walk_tape(w.how, q, |r| f(r, r, known));
            return;
        }
        // The walk's last place of the tag: none (0), its key `kv` (1), another (2).
        let mut last = 0u32;
        while q < w.hi {
            let mut key = false;
            if w.how == 1 {
                let m = rel_group_mark(self.r as *const u8, self.s.r_at, self.s.r_index, q);
                // Another key's tag: not read.
                if (m ^ w.tag) & !MARK_SAME != 0 {
                    q = q.wrapping_add(1);
                    continue;
                }
                key = if last != 0 && m & MARK_SAME != 0 { last == 1 } else { self.keys_are(self.cand_row(1, q), kv) };
                last = if key { 1 } else { 2 };
                if !key {
                    q = q.wrapping_add(1);
                    continue;
                }
            }
            if !f(self.cand_row(w.how, q), q, key || known) {
                return;
            }
            q = q.wrapping_add(1);
        }
    }

    /// An aggregate over the binding's matching rows: its value, else 1 (none), or 2 (a sum past 64 bits).
    unsafe fn aggregate(&self, t: u32, w: Walk, kv: &[i64; 8]) -> Result<i64, u32> {
        let s = self.s;
        let kind = s.agg[0] & 0xff;
        let (mut cnt, mut sum, mut mn, mut mx, mut any, mut over) = (0i64, 0i64, 0i64, 0i64, false, false);
        self.each(w, kv, |r, _, key| {
            if self.matches(r, kv, key) {
                cnt += 1;
                if s.agg[1] != NONE {
                    let x = self.rget(r, s.agg[1]);
                    let (y, o) = sum.overflowing_add(x);
                    over |= o;
                    sum = y;
                    mn = if any { mn.min(x) } else { x };
                    mx = if any { mx.max(x) } else { x };
                    any = true;
                }
            }
            true
        });
        if kind == 1 && over {
            return Err(2);
        }
        if (kind == 2 || kind == 3) && !any {
            return Err(1);
        }
        let v = match kind {
            0 => cnt,
            1 => sum,
            2 => mn,
            _ => mx,
        };
        if (s.agg[0] >> 8) & 1 == 0 && self.slot(s.bind_slot[0], t) != v {
            return Err(1);
        }
        Ok(v)
    }

    /// Binding `t`'s row `r` out at `at` of `o`: its slots, then its binds.
    unsafe fn write_row(&self, t: u32, r: u32, o: *mut i64, at: u32) {
        let s = self.s;
        for k in 0..s.nslots {
            col_put(o, k, s.cap_out, at, self.slot(k, t));
        }
        self.write_binds(t, r, o, at);
    }

    /// Binding `t`'s row `r`'s binds out at `at` of `o`.
    unsafe fn write_binds(&self, t: u32, r: u32, o: *mut i64, at: u32) {
        let s = self.s;
        for b in 0..s.nbinds.min(16) as usize {
            let i = s.bind_idx[b];
            let v = match s.bind_kind[b] {
                3 => self.arith(i, t, r).unwrap_or(0),
                0 => self.colval(i, t, r),
                1 => self.parval(i, t),
                _ => self.rget(r, i),
            };
            col_put(o, s.bind_slot[b], s.cap_out, at, v);
        }
    }

    /// Text operand `k` of kind `kind` (0 a symbol, 1 `cat`'s, 2 `str_concat`'s, 3 an arena text, 4 a node's text).
    unsafe fn txt_of(&self, t: u32, d: *const u8, a: *const u32, k: u32, kind: u32, unheld: &mut bool) -> Txt {
        let v = self.key_value(t, k);
        if kind == 4 {
            // A node's text through its document's source (`node_txt`, `rr_node_text_any`'s: a line's span too); one
            // no read here finds reads as empty and sets `unheld` (the stratum then runs on the host).
            return node_txt(self.s, v, true).unwrap_or_else(|_| {
                *unheld = true;
                Txt::raw(core::ptr::NonNull::<u8>::dangling().as_ptr(), 0)
            });
        }
        if kind == 3 {
            let off = ((v as u64) >> 24) & ((1u64 << 39) - 1);
            let len = (v & 0xff_ffff) as u32;
            let ok = v != -1 && off + len as u64 <= *a.add(1) as u64;
            return Txt::raw(arena_bytes(a).add(if ok { off as usize } else { 0 }), if ok { len } else { 0 });
        }
        let (pa, al) = sym_text(d, v, unheld);
        let mut x = Txt { a: pa, al, b: d, bl: 0, dot: 0 };
        if kind != 0 {
            let (pb, bl) = sym_text(d, self.key_value(t, k + 1), unheld);
            x.b = pb;
            x.bl = bl;
            x.dot = (kind == 1 && al > 0) as u32;
        }
        x
    }
}

/// A join's binding's finds (`join_stages.uglykl`'s): its candidates' first place and a bit a candidate, where they
/// are at most `FINDS_MAX` consecutive places; else `FINDS_WALK`.
const FINDS_MAX: u32 = 32;
const FINDS_WALK: i64 = NONE as i64;

#[derive(Clone, core::marker::Copy)]
struct Walk {
    lo: u32,
    hi: u32,
    how: u32,
    /// An index group's (`how` 1): its key's tag (decision 0671), which its places' marks are read against.
    tag: u32,
}

/// A mark's `MARK_SAME` (decision 0671, `rerail_blocks::relation::MARK_SAME`).
const MARK_SAME: u32 = 1;

/// A key's tag from its hash (`rerail_blocks::relation::mark_tag`).
fn mark_tag(h: u64) -> u32 {
    (h >> 32) as u32 & !MARK_SAME
}

fn lane_of(v: i64, w: u32) -> i64 {
    match w {
        16 => v as i16 as i64,
        32 => v as i32 as i64,
        _ => v,
    }
}

// ---- Texts: their bytes 16 a step. ----

/// `N` lanes at `p`.
#[inline(always)]
unsafe fn ld<T: SimdElement, const N: usize>(p: *const T) -> Simd<T, N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Simd::from_slice(core::slice::from_raw_parts(p, N))
}

/// `v`'s `N` lanes to `p`.
#[inline(always)]
unsafe fn st<T: SimdElement, const N: usize>(p: *mut T, v: Simd<T, N>)
where
    LaneCount<N>: SupportedLaneCount,
{
    v.copy_to_slice(core::slice::from_raw_parts_mut(p, N))
}

/// The first of `n` bytes at `p` that is `c`.
unsafe fn find_byte(p: *const u8, n: u32, c: u8) -> Option<u32> {
    let needle = u8x16::splat(c);
    let mut i = 0;
    while i + 16 <= n {
        let m = ld::<u8, 16>(p.add(i as usize)).simd_eq(needle).to_bitmask();
        if m != 0 {
            return Some(i + m.trailing_zeros());
        }
        i += 16;
    }
    (i..n).find(|&k| *p.add(k as usize) == c)
}

/// The last of `n` bytes at `p` that is `c`.
unsafe fn rfind_byte(p: *const u8, n: u32, c: u8) -> Option<u32> {
    let needle = u8x16::splat(c);
    let mut end = n;
    while end >= 16 {
        let m = ld::<u8, 16>(p.add(end as usize - 16)).simd_eq(needle).to_bitmask();
        if m != 0 {
            return Some(end - 16 + 63 - m.leading_zeros());
        }
        end -= 16;
    }
    (0..end).rev().find(|&k| *p.add(k as usize) == c)
}

/// How many of `n` bytes at `p` begin a character: no UTF-8 continuation byte (0x80-0xbf, below -64 as an i8).
unsafe fn count_chars(p: *const u8, n: u32) -> u32 {
    let floor = i8x16::splat(-64);
    let (mut c, mut i) = (0u32, 0u32);
    while i + 16 <= n {
        c += ld::<i8, 16>(p.add(i as usize).cast()).simd_ge(floor).to_bitmask().count_ones();
        i += 16;
    }
    c + (i..n).filter(|&k| *p.add(k as usize) & 0xc0 != 0x80).count() as u32
}

/// Whether one of `n` bytes at `p` is past ASCII.
unsafe fn any_high(p: *const u8, n: u32) -> bool {
    let mut i = 0;
    while i + 16 <= n {
        if ld::<u8, 16>(p.add(i as usize)).simd_ge(u8x16::splat(0x80)).any() {
            return true;
        }
        i += 16;
    }
    (i..n).any(|k| *p.add(k as usize) >= 0x80)
}

/// The first of `n` places where the bytes at `p` and `q` differ.
unsafe fn first_diff(p: *const u8, q: *const u8, n: u32) -> Option<u32> {
    let mut i = 0;
    while i + 16 <= n {
        let m = ld::<u8, 16>(p.add(i as usize)).simd_ne(ld(q.add(i as usize))).to_bitmask();
        if m != 0 {
            return Some(i + m.trailing_zeros());
        }
        i += 16;
    }
    (i..n).find(|&k| *p.add(k as usize) != *q.add(k as usize))
}

/// `n` bytes from `p` to `o`, their ASCII letters lowered, or raised (`upper`): a letter's case is its bit 0x20.
unsafe fn case_bytes(p: *const u8, n: u32, o: *mut u8, upper: bool) {
    let first = u8x16::splat(if upper { b'a' } else { b'A' });
    let (span, bit) = (u8x16::splat(26), u8x16::splat(0x20));
    let mut i = 0;
    while i + 16 <= n {
        let v = ld::<u8, 16>(p.add(i as usize));
        let letter = (v - first).simd_lt(span);
        st(o.add(i as usize), letter.select(v ^ bit, v));
        i += 16;
    }
    for k in i as usize..n as usize {
        let c = *p.add(k);
        *o.add(k) = if upper { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
    }
}

static DOT: u8 = b'.';

/// A text operand: a's bytes, "." when `dot`, b's bytes. Read in place, a part at a time.
#[derive(Clone, core::marker::Copy)]
struct Txt {
    a: *const u8,
    al: u32,
    b: *const u8,
    bl: u32,
    dot: u32,
}

impl Txt {
    fn raw(p: *const u8, n: u32) -> Txt {
        Txt { a: p, al: n, b: p, bl: 0, dot: 0 }
    }
    fn len(&self) -> u32 {
        self.al + self.dot + self.bl
    }
    /// Its parts: each its first byte's place in the text, its address and its bytes.
    fn parts(&self) -> [(u32, *const u8, u32); 3] {
        [(0, self.a, self.al), (self.al, &DOT, self.dot), (self.al + self.dot, self.b, self.bl)]
    }
    /// Byte `i`'s address, and how many bytes its part has from it.
    unsafe fn run(&self, i: u32) -> (*const u8, u32) {
        if i < self.al {
            return (self.a.add(i as usize), self.al - i);
        }
        let i = i - self.al;
        if i < self.dot {
            return (&DOT, 1);
        }
        let i = i - self.dot;
        (self.b.add(i as usize), self.bl - i)
    }
    unsafe fn byte(&self, i: u32) -> u8 {
        *self.run(i).0
    }
    /// Its bytes from `from`, in order (an automaton's walk that stops at a dead state).
    unsafe fn bytes(&self, from: u32, to: u32) -> impl Iterator<Item = u8> + '_ {
        (from..to).map(move |i| self.byte(i))
    }
    /// The first place of its `n` bytes from `xs` that differs from `y`'s from `ys`, a run of both at a time.
    unsafe fn diff(&self, xs: u32, y: &Txt, ys: u32, n: u32) -> Option<u32> {
        let mut k = 0;
        while k < n {
            let ((p, pn), (q, qn)) = (self.run(xs + k), y.run(ys + k));
            let m = pn.min(qn).min(n - k);
            if let Some(d) = first_diff(p, q, m) {
                return Some(k + d);
            }
            k += m;
        }
        None
    }
    /// Whether its `n` bytes from `xs` are `y`'s from `ys`.
    unsafe fn same(&self, xs: u32, y: &Txt, ys: u32, n: u32) -> bool {
        self.diff(xs, y, ys, n).is_none()
    }
    /// Its order against `y`: by their bytes, then their lengths.
    unsafe fn cmp(&self, y: &Txt) -> core::cmp::Ordering {
        let n = self.len().min(y.len());
        match self.diff(0, y, 0, n) {
            Some(d) => self.byte(d).cmp(&y.byte(d)),
            None => self.len().cmp(&y.len()),
        }
    }
    /// Its characters: its bytes less the UTF-8 continuation bytes.
    unsafe fn chars(&self) -> u32 {
        self.parts().iter().map(|&(_, p, n)| count_chars(p, n)).sum()
    }
    /// The first `c` from `from`.
    unsafe fn find(&self, from: u32, c: u8) -> Option<u32> {
        self.parts().iter().find_map(|&(at, p, n)| {
            let skip = from.saturating_sub(at);
            if skip >= n { None } else { find_byte(p.add(skip as usize), n - skip, c).map(|i| at + skip + i) }
        })
    }
    /// The last `c`.
    unsafe fn rfind(&self, c: u8) -> Option<u32> {
        self.parts().iter().rev().find_map(|&(at, p, n)| rfind_byte(p, n, c).map(|i| at + i))
    }
    unsafe fn any_high(&self) -> bool {
        self.parts().iter().any(|&(_, p, n)| any_high(p, n))
    }
    /// The byte character `c` begins at (its length where it has fewer): 16 bytes' characters counted a step.
    unsafe fn char_byte(&self, c: i64) -> u32 {
        let mut seen = 0i64;
        for (at, p, n) in self.parts() {
            let mut i = 0;
            while i + 16 <= n && seen + count_chars(p.add(i as usize), 16) as i64 <= c {
                seen += count_chars(p.add(i as usize), 16) as i64;
                i += 16;
            }
            for k in i..n {
                if *p.add(k as usize) & 0xc0 != 0x80 {
                    if seen == c {
                        return at + k;
                    }
                    seen += 1;
                }
            }
        }
        self.len()
    }
    /// Its bytes [from, to) written at `o`, each part's as one copy, or mapped by case (`case`: 1 lower, 2 upper).
    unsafe fn write(&self, from: u32, to: u32, o: *mut u8, case: u32) {
        let mut k = from;
        while k < to {
            let (p, pn) = self.run(k);
            let m = pn.min(to - k);
            let at = o.add((k - from) as usize);
            match case {
                0 => core::ptr::copy_nonoverlapping(p, at, m as usize),
                c => case_bytes(p, m, at, c == 2),
            }
            k += m;
        }
    }
    /// The first place from `from` that `y` (not empty) starts at: its first byte found 16 a step, then the rest.
    unsafe fn find_txt(&self, from: u32, y: &Txt) -> Option<u32> {
        let (n, ly) = (self.len(), y.len());
        let first = y.byte(0);
        let mut k = from;
        while k + ly <= n {
            let m = self.find(k, first)?;
            if m + ly > n {
                return None;
            }
            if self.same(m, y, 0, ly) {
                return Some(m);
            }
            k = m + 1;
        }
        None
    }
}

unsafe fn arena_bytes(a: *const u32) -> *const u8 {
    (a as *const u8).add(16)
}

fn arena_value(off: u32, len: u32) -> i64 {
    (0x8000_0000_0000_0000u64 | ((off as u64) << 24) | len as u64) as i64
}

/// Symbol `v`'s bytes through the atoms directory `d`; a value no atom of it is reads as empty, `unheld` set.
unsafe fn sym_text(d: *const u8, v: i64, unheld: &mut bool) -> (*const u8, u32) {
    let n = if (0..=u32::MAX as i64).contains(&v) { atoms_len(d, 0, v as u32) } else { NONE };
    if n == NONE {
        *unheld = true;
        return (d, 0);
    }
    (atoms_at(d, 0, v as u32) as *const u8, n)
}

/// Node `v`'s preorder key (`@rr_node_order`, `ARITH_NODE_ORDER`): its document by key in the stage's node view `d`
/// (`memo`: the document found last, tried first), its numbered node's order there; `None` where the view numbers no
/// node `v`, as the host's read (`LongTexts::order_any`) has none.
#[inline]
pub(super) unsafe fn node_order(d: *const ViewDir, v: i64, memo: &mut u32) -> Option<i64> {
    if d.is_null() || v < 0 {
        return None;
    }
    let m = ViewDir::doc_of_key(d, ((v as u64) >> 32) as u32, *memo);
    if m == rerail_reads::views::NO_DOC {
        return None;
    }
    *memo = m;
    reads::node_order(d, m, v as u32).map(|k| k as i64)
}

/// Why a node's text is none (`node_txt`).
enum NoText {
    /// A node the directory does not hold (another batch's, a comment span, a match's span, no node): the stratum runs
    /// on the host (`TEXTFN_UNHELD`).
    Unheld,
    /// A line's span read by the long-text twin (`rr_node_text`), which has no row of a line: no row.
    Line,
}

/// Node `key`'s text, read where its document's source lies (`join_stages`' `node_txt`, decision 0287 §2): its document
/// by the directory's search on its key (`dir_doc_of_key`), its numbered node (`number`), its bytes' range in its own
/// source (`text_span`), read at the source's home. The stage's `node` view binds the directory (`Stage::v_dir`). With
/// `lines` (`rr_node_text_any`'s), a document's `source` span or a `src_line` span (decision 0353 row 7) is its line's
/// bytes there (`line_span`), less its terminator where it has one (`line_terminated`, `lines::length_ending_at`).
unsafe fn node_txt(s: &Stage, key: i64, lines: bool) -> Result<Txt, NoText> {
    let d = s.v_dir;
    if key < 0 || d.is_null() || (*d).ndocs == 0 {
        return Err(NoText::Unheld);
    }
    let m = ViewDir::doc_of_key(d, ((key as u64) >> 32) as u32, NONE);
    if m == NONE {
        return Err(NoText::Unheld);
    }
    match reads::node_span(d, m, key as u64 as u32) {
        None => Err(NoText::Unheld),
        Some((_, _, true)) if !lines => Err(NoText::Line),
        Some((p, n, _)) => Ok(Txt::raw(p, n)),
    }
}

/// Mode 7's test of pattern `x`'s bytes [xs, xs + lx) against subject `y`: by op 0 equal, 1 a prefix, 2 a suffix.
unsafe fn textop_one(op: u32, x: &Txt, xs: u32, lx: u32, y: &Txt, ly: u32) -> bool {
    let fits = if op == 0 { lx == ly } else { ly >= lx };
    fits && x.same(xs, y, if op >= 2 { ly - lx } else { 0 }, lx)
}

/// Mode 7: X a pattern, or a set of them (`TEXT_SPLIT`: 0x1f between them, found 16 bytes a step), Y passing when one
/// does.
unsafe fn textop_pass(st: &St, t: u32, d: *const u8, a: *const u32, unheld: &mut bool, host: &mut bool) -> bool {
    let s = st.s;
    let x = st.txt_of(t, d, a, 0, s.agg[1] & 15, unheld);
    let y = st.txt_of(t, d, a, 2, s.agg[1] >> 4, unheld);
    let (lx, ly, op) = (x.len(), y.len(), (s.agg[0] >> 1) & 3);
    let split = s.agg[0] & 32 != 0;
    let mut r = false;
    let mut start = 0;
    loop {
        let sep = if split { x.find(start, 0x1f) } else { None };
        if sep.is_some() && s.agg[0] & 64 != 0 {
            *host = true;
        }
        let end = sep.unwrap_or(lx);
        r |= textop_one(op, &x, start, end - start, &y, ly);
        match sep {
            Some(i) => start = i + 1,
            None => break,
        }
    }
    r == (s.agg[0] & 1 != 0)
}

/// Mode 6: `str_cmp` of X and Y (by their bytes, then their lengths), or `strlen` of X (its characters).
unsafe fn strcmp_value(st: &St, t: u32, d: *const u8, a: *const u32, unheld: &mut bool) -> i64 {
    let s = st.s;
    let x = st.txt_of(t, d, a, 0, s.agg[1] & 15, unheld);
    if s.agg[0] == 1 {
        return x.chars() as i64;
    }
    let y = st.txt_of(t, d, a, 2, s.agg[1] >> 4, unheld);
    if s.agg[0] == 2 {
        // `instr`: where y first is in x, in characters from 1 (as `str::find`); 0 where it is not.
        let (lx, ly) = (x.len(), y.len());
        let mut at = 0;
        while at + ly <= lx {
            if x.same(at, &y, 0, ly) {
                return (0..at).filter(|&i| x.byte(i) & 0xc0 != 0x80).count() as i64 + 1;
            }
            at += 1;
        }
        return 0;
    }
    x.cmp(&y) as i64
}

/// Whether a trim takes byte `c`: a space (`spaces`), else one of `y`'s bytes.
unsafe fn trim_has(y: &Txt, spaces: bool, c: u8) -> bool {
    if spaces {
        return c == b' ';
    }
    (0..y.len()).any(|i| y.byte(i) == c)
}

/// An automaton's table (`text::table::Table`): its 256 byte classes, then its transitions, `s` states of `c` classes,
/// then its states after the end and their verdicts. Its classes of 64 bytes are four lookups of 64 bytes each.
struct Automaton {
    tab: *const u8,
    classes: [u8x64; 4],
    s: u32,
    c: u32,
    start: u32,
}

impl Automaton {
    unsafe fn new(tab: *const u8, s: u32, c: u32, start: u32) -> Self {
        let classes = core::array::from_fn(|k| ld(tab.add(64 * k)));
        Automaton { tab, classes, s, c, start }
    }

    /// Bytes `v`'s classes: an index past a table's 64 gives 0 there, so the four lookups' or is the class.
    #[inline(always)]
    fn classes_of(&self, v: u8x64) -> u8x64 {
        let sixty_four = u8x64::splat(64);
        let mut r = self.classes[0].swizzle_dyn(v);
        let mut i = v;
        for k in 1..4 {
            i -= sixty_four;
            r |= self.classes[k].swizzle_dyn(i);
        }
        r
    }

    #[inline(always)]
    unsafe fn step(&self, q: u32, class: u8) -> u32 {
        *self.tab.add(256 + (q * self.c + class as u32) as usize) as u32
    }

    /// The state after `n` bytes at `p` from `q`: 64 bytes' classes at once, then the walk through them.
    unsafe fn walk(&self, mut q: u32, p: *const u8, n: u32) -> u32 {
        let mut i = 0;
        while i + 64 <= n {
            for k in self.classes_of(ld(p.add(i as usize))).to_array() {
                q = self.step(q, k);
            }
            i += 64;
        }
        for k in i..n {
            q = self.step(q, *self.tab.add(*p.add(k as usize) as usize));
        }
        q
    }

    /// Whether it accepts `x`: its state after the end of the text.
    unsafe fn verdict(&self, x: &Txt) -> bool {
        let q = x.parts().iter().fold(self.start, |q, &(_, p, n)| self.walk(q, p, n));
        let eoi = self.tab.add(256 + (self.s * self.c) as usize);
        *eoi.add(self.s as usize + *eoi.add(q as usize) as usize) != 0
    }
}

/// A replacer's automaton, stepped a byte at a time (its walks stop at a dead state).
struct Dfa {
    tab: *const u8,
    c: u32,
}
impl Dfa {
    unsafe fn step(&self, q: u32, b: u8) -> u32 {
        let class = *self.tab.add(b as usize) as u32;
        *self.tab.add(256 + (q * self.c + class) as usize) as u32
    }
}

/// `replace_all` of regex `g` in `x` by `z` (`text::table::Replacer`): each match's end by the forward automaton (the
/// last match before it dies), its start by the reverse one run back from it (the furthest match), matches delayed a
/// byte. The replaced text's length; written to `o` where given, the unmatched bytes and `z` a part's copy at a time.
/// foldhash's folded multiply: the 128-bit product's halves, xored.
fn fold_mul(x: u64, y: u64) -> u64 {
    let p = x as u128 * y as u128;
    (p as u64) ^ ((p >> 64) as u64)
}

/// `n` (at most 8) bytes from `p` as a little-endian word.
unsafe fn bytes_le(p: *const u8, n: u32) -> u64 {
    (0..n).fold(0, |w, k| w | (*p.add(k as usize) as u64) << (8 * k))
}

/// The store's hash of a string (`content_store::atoms::hash_str`: foldhash 0.2's quality hasher, fixed state, seed
/// 0x5EED71, a `str`'s bytes then 0xff), the one the atoms blocks' tables from hash to id are keyed by (decision 0666):
/// `store_hash`.
pub(crate) unsafe fn store_hash(mut v: *const u8, len: u32) -> u64 {
    const S: [u64; 6] = [0xc0ac29b7c97c50dd, 0x3f84d5b5b5470917, 0x9216d5d98979fb1b, 0xd1310ba698dfb5ac, 0x2ffd72dbd01adfb7, 0xb8e1afed6a267e96];
    let at = |v: *const u8, k: u32| bytes_le(v.add(k as usize), 8);
    // The per-hasher seed: folded_multiply(0x5EED71, ARBITRARY4) ^ ARBITRARY3, rotated by the length.
    let mut acc = (fold_mul(0x5EED71, 0x452821e638d01377) ^ 0x082efa98ec4e6c89).rotate_right(len & 63);
    if len <= 16 {
        let (mut s0, mut s1) = (acc, S[1]);
        if len >= 8 {
            s0 ^= bytes_le(v, 8);
            s1 ^= bytes_le(v.add(len as usize - 8), 8);
        } else if len >= 4 {
            s0 ^= bytes_le(v, 4);
            s1 ^= bytes_le(v.add(len as usize - 4), 4);
        } else if len > 0 {
            s0 ^= *v as u64;
            s1 ^= (*v.add(len as usize - 1) as u64) << 8 | *v.add(len as usize / 2) as u64;
        }
        acc = fold_mul(s0, s1);
    } else {
        let (mut s0, mut s1) = (acc, acc.wrapping_add(S[1]));
        let mut n = len;
        if n > 128 {
            let (mut s2, mut s3) = (acc.wrapping_add(S[2]), acc.wrapping_add(S[3]));
            if n > 256 {
                let (mut s4, mut s5) = (acc.wrapping_add(S[4]), acc.wrapping_add(S[5]));
                loop {
                    s0 = fold_mul(at(v, 0) ^ s0, at(v, 48) ^ S[0]);
                    s1 = fold_mul(at(v, 8) ^ s1, at(v, 56) ^ S[0]);
                    s2 = fold_mul(at(v, 16) ^ s2, at(v, 64) ^ S[0]);
                    s3 = fold_mul(at(v, 24) ^ s3, at(v, 72) ^ S[0]);
                    s4 = fold_mul(at(v, 32) ^ s4, at(v, 80) ^ S[0]);
                    s5 = fold_mul(at(v, 40) ^ s5, at(v, 88) ^ S[0]);
                    v = v.add(96);
                    n -= 96;
                    if n <= 256 {
                        break;
                    }
                }
                s0 ^= s4;
                s1 ^= s5;
            }
            loop {
                s0 = fold_mul(at(v, 0) ^ s0, at(v, 32) ^ S[0]);
                s1 = fold_mul(at(v, 8) ^ s1, at(v, 40) ^ S[0]);
                s2 = fold_mul(at(v, 16) ^ s2, at(v, 48) ^ S[0]);
                s3 = fold_mul(at(v, 24) ^ s3, at(v, 56) ^ S[0]);
                v = v.add(64);
                n -= 64;
                if n <= 128 {
                    break;
                }
            }
            s0 ^= s2;
            s1 ^= s3;
        }
        s0 = fold_mul(at(v, 0) ^ s0, at(v, n - 16) ^ S[0]);
        s1 = fold_mul(at(v, 8) ^ s1, at(v, n - 8) ^ S[0]);
        if n >= 32 {
            s0 = fold_mul(at(v, 16) ^ s0, at(v, n - 32) ^ S[0]);
            s1 = fold_mul(at(v, 24) ^ s1, at(v, n - 24) ^ S[0]);
            if n >= 64 {
                s0 = fold_mul(at(v, 32) ^ s0, at(v, n - 48) ^ S[0]);
                s1 = fold_mul(at(v, 40) ^ s1, at(v, n - 40) ^ S[0]);
                if n >= 96 {
                    s0 = fold_mul(at(v, 48) ^ s0, at(v, n - 64) ^ S[0]);
                    s1 = fold_mul(at(v, 56) ^ s1, at(v, n - 56) ^ S[0]);
                }
            }
        }
        acc = s0 ^ s1;
    }
    // A `str`'s 0xff (`write_u8`: the sponge), then the quality hasher's last fold.
    fold_mul(fold_mul(0xff ^ acc, S[0]), 0x243f6a8885a308d3)
}

/// The atom whose string is the made text at `off` in arena `a`, `len` bytes: its id, found by the store's hash in the
/// tables of atoms directory `d`'s runs (`atoms_find`, decision 0666), each candidate's bytes compared; -1 where no
/// atom is (`atom_of_text`).
unsafe fn atom_of_text(a: *const u32, off: u32, len: u32, d: *const u8) -> i64 {
    if len == 0 {
        return 0;
    }
    let mine = (a as *const u8).add(16 + off as usize);
    let h = store_hash(mine, len);
    for k in 0.. {
        let id = atoms_find(d, 0, h, len, k);
        if id == NONE {
            return -1;
        }
        let theirs = atoms_at(d, 0, id) as *const u8;
        if core::slice::from_raw_parts(theirs, len as usize) == core::slice::from_raw_parts(mine, len as usize) {
            return id as i64;
        }
    }
    -1
}

/// A regex group's closure from state `sid` at `pos` into a thread list (`rx_group_closure`): each state once, the
/// group's two slots a thread, its entries in priority order; a capture state saves its slot, restored after.
#[allow(clippy::too_many_arguments)]
unsafe fn rx_group_closure(w: *const u32, list: &mut [u32], count: &mut u32, stack: &mut [u32], sid: u32, pos: u32, n: u32, c: [u32; 2]) {
    let wd = |i: u32| *w.add(i as usize);
    let st = w.add(wd(4) as usize);
    let s_count = wd(0) as usize;
    let (dense, rest) = list.split_at_mut(s_count);
    let (sparse, slots) = rest.split_at_mut(s_count);
    let mut cur = c;
    let cap = wd(3);
    stack[0] = sid;
    stack[1] = 0;
    let mut sp = 1u32;
    // Bounded: each state is entered once a closure, and pushes its entries, `w[3]` at most.
    let mut turn = 0;
    while turn < 2 * cap + 2 && sp > 0 {
        turn += 1;
        sp -= 1;
        let (a, old) = (stack[2 * sp as usize], stack[2 * sp as usize + 1]);
        if a & 0x8000_0000 != 0 {
            cur[(a & 1) as usize] = old;
            continue;
        }
        let i = sparse[a as usize];
        if i < *count && dense[i as usize] == a {
            continue;
        }
        sparse[a as usize] = *count;
        dense[*count as usize] = a;
        slots[2 * *count as usize] = cur[0];
        slots[2 * *count as usize + 1] = cur[1];
        *count += 1;
        let s = st.add(4 * a as usize);
        let sd = |k: usize| *s.add(k);
        let mut push = |x: u32, v: u32, sp: &mut u32| {
            stack[2 * *sp as usize] = x;
            stack[2 * *sp as usize + 1] = v;
            *sp += 1;
        };
        match sd(0) {
            3 => {
                let mut k = sd(1);
                while k > 0 && sp < cap {
                    push(wd(sd(2) + k - 1), 0, &mut sp);
                    k -= 1;
                }
            }
            4 if sp + 2 <= cap => {
                if sd(1) < 2 {
                    push(0x8000_0000 | sd(1), cur[sd(1) as usize], &mut sp);
                    cur[sd(1) as usize] = pos;
                }
                push(sd(2), 0, &mut sp);
            }
            5 if ((sd(1) == 0 && pos == 0) || (sd(1) == 1 && pos == n)) && sp < cap => push(sd(2), 0, &mut sp),
            _ => {}
        }
    }
}

/// A regex's capture group (`TEXTFN_RX_GROUP`, decision 0688): program `g` (`text::table::GroupProgram`, the regex's
/// Thompson NFA) walked over `x` by a Pike VM, as `regex_automata`'s `PikeVM` walks it: its threads in priority order,
/// each state once a position, the group's two slots a thread; a match cuts the threads after it, and the last match
/// found is the leftmost-first one (`rx_group_range`). `scratch`: two thread lists and the closure's stack, grown here.
/// The group's range of `x`, where the regex matched and the group took part.
unsafe fn rx_group_range(g: *const u8, x: &Txt, scratch: &mut Vec<u32>) -> Option<(u32, u32)> {
    let w = g as *const u32;
    let wd = |i: u32| *w.add(i as usize);
    let (s_count, n) = (wd(0) as usize, x.len());
    let st = w.add(wd(4) as usize);
    scratch.clear();
    scratch.resize(8 * s_count + 2 * wd(3) as usize + 2, 0);
    let (lists, stack) = scratch.split_at_mut(8 * s_count);
    let (mut l0, mut l1) = lists.split_at_mut(4 * s_count);
    let mut c0 = 0u32;
    let (mut got0, mut got1) = (NONE, NONE);
    rx_group_closure(w, l0, &mut c0, stack, wd(2), 0, n, [NONE, NONE]);
    let mut pos = 0;
    while pos <= n && c0 > 0 {
        let mut c1 = 0u32;
        let b = if pos < n { x.byte(pos) as u32 } else { 0 };
        for i in 0..c0 as usize {
            let s = st.add(4 * l0[i] as usize);
            let sd = |k: usize| *s.add(k);
            let (t0, t1) = (l0[2 * s_count + 2 * i], l0[2 * s_count + 2 * i + 1]);
            if sd(0) == 6 {
                got0 = t0;
                got1 = t1;
                break;
            }
            if pos == n {
                continue;
            }
            let mut next = NONE;
            if sd(0) == 1 && sd(1) <= b && b <= sd(2) {
                next = sd(3);
            } else if sd(0) == 2 {
                for k in 0..sd(1) {
                    let r = w.add((sd(2) + 3 * k) as usize);
                    if *r <= b && b <= *r.add(1) {
                        next = *r.add(2);
                        break;
                    }
                }
            }
            if next != NONE {
                rx_group_closure(w, l1, &mut c1, stack, next, pos + 1, n, [t0, t1]);
            }
        }
        core::mem::swap(&mut l0, &mut l1);
        core::mem::swap(&mut c0, &mut c1);
        pos += 1;
    }
    (got0 != NONE && got1 != NONE).then_some((got0, got1))
}

unsafe fn rx_replace_txt(g: *const u8, x: &Txt, z: &Txt, o: Option<*mut u8>) -> u32 {
    let h = g as *const u32;
    let hw = |i: usize| *h.add(i);
    let (fs, fc, fdead, rs, rc, rdead) = (hw(0), hw(1), hw(4), hw(5), hw(6), hw(9));
    let (ft, rt) = (g.add(hw(10) as usize), g.add(hw(11) as usize));
    let (feoi, reoi) = (ft.add((256 + fs * fc) as usize), rt.add((256 + rs * rc) as usize));
    let (facc, racc) = (feoi.add(fs as usize), reoi.add(rs as usize));
    let (fwd, rev) = (Dfa { tab: ft, c: fc }, Dfa { tab: rt, c: rc });
    let (n, lz) = (x.len(), z.len());
    let (mut pos, mut w) = (0u32, 0u32);
    while pos < n {
        let mut st = if pos == 0 { hw(2) } else { hw(3) };
        let (mut last, mut dead) = (NONE, false);
        for (k, b) in (pos..n).zip(x.bytes(pos, n)) {
            let q = fwd.step(st, b);
            if q == fdead {
                dead = true;
                break;
            }
            if *facc.add(q as usize) != 0 {
                last = k;
            }
            st = q;
        }
        if !dead && *facc.add(*feoi.add(st as usize) as usize) != 0 {
            last = n;
        }
        if last == NONE {
            break;
        }
        let e = last;
        let lo = pos.saturating_sub(1);
        let mut rq = if e == n { hw(7) } else { hw(8) };
        let (mut first, mut rd) = (NONE, false);
        for k in (lo..e).rev() {
            let q = rev.step(rq, x.byte(k));
            if q == rdead {
                rd = true;
                break;
            }
            if *racc.add(q as usize) != 0 {
                first = k + 1;
            }
            rq = q;
        }
        if !rd && pos == 0 && *racc.add(*reoi.add(rq as usize) as usize) != 0 {
            first = 0;
        }
        if first == NONE || first < pos {
            break;
        }
        if let Some(o) = o {
            x.write(pos, first, o.add(w as usize), 0);
            z.write(0, lz, o.add((w + first - pos) as usize), 0);
        }
        w += first - pos + lz;
        pos = e;
    }
    if let Some(o) = o {
        x.write(pos, n, o.add(w as usize), 0);
    }
    w + n - pos
}

/// `replace_all` of literal `y` (empty: none) in `x` by `z`, matches not overlapping, each found 16 bytes a step.
unsafe fn lit_replace_txt(x: &Txt, y: &Txt, z: &Txt, o: Option<*mut u8>) -> u32 {
    let (n, ly, lz) = (x.len(), y.len(), z.len());
    let (mut pos, mut w) = (0u32, 0u32);
    while let Some(m) = if ly > 0 { x.find_txt(pos, y) } else { None } {
        if let Some(o) = o {
            x.write(pos, m, o.add(w as usize), 0);
            z.write(0, lz, o.add((w + m - pos) as usize), 0);
        }
        w += m - pos + lz;
        pos = m + ly;
    }
    if let Some(o) = o {
        x.write(pos, n, o.add(w as usize), 0);
    }
    w + n - pos
}

fn pow10_of(k: u32) -> u64 {
    10u64.pow(k.min(19))
}

/// The characters [a, b) a substring keeps of `nc`: `substr` (f 3), SQLite's (f 7), `slice` (f 4). Its arithmetic
/// wraps, as the device's.
fn substr_range(f: u32, n0: i64, n1: i64, nc: i64) -> (i64, i64) {
    if f == 4 {
        let a = (if n0 < 0 { nc.wrapping_add(n0) } else { n0 }).clamp(0, nc);
        let b = (if n1 < 0 { nc.wrapping_add(n1) } else { n1 }).clamp(0, nc);
        return (a, b.max(a));
    }
    let (mut p1, mut p2) = (if f == 3 { n0.wrapping_add(1) } else { n0 }, n1);
    let neg = p2 < 0;
    if neg {
        p2 = p2.wrapping_neg();
    }
    if p1 < 0 {
        p1 = p1.wrapping_add(nc);
        if p1 < 0 {
            p2 = p2.wrapping_add(p1).max(0);
            p1 = 0;
        }
    } else if p1 > 0 {
        p1 -= 1;
    } else if p2 > 0 {
        p2 -= 1;
    }
    if neg {
        p1 = p1.wrapping_sub(p2);
        if p1 < 0 {
            p2 = p2.wrapping_add(p1);
            p1 = 0;
        }
    }
    let a = p1.clamp(0, nc);
    (a, p1.wrapping_add(p2).clamp(a, nc))
}

/// A function's kept range of `x` [from, to), and whether `x` has a byte past ASCII (`lower`'s, f 0).
unsafe fn text_range(f: u32, x: &Txt, n0: i64, n1: i64) -> (u32, u32, bool) {
    let n = x.len();
    match f {
        1 => (0, x.find(0, b'.').unwrap_or(n), false),
        2 => (0, x.rfind(b'/').unwrap_or(0), false),
        3 | 4 | 7 => {
            let (a, b) = substr_range(f, n0, n1, x.chars() as i64);
            (x.char_byte(a), x.char_byte(b), false)
        }
        0 => (0, n, x.any_high()),
        _ => (0, n, false),
    }
}

// ---- The stages' functions, a thread a binding (or a candidate row). ----

/// The buffers an item's job reaches (`Job`, its words placed in the arena where its size script placed them).
pub mod classes;
mod dedup_lanes;
pub mod reads;
mod stage_lanes;

struct Bufs {
    l: *const i64,
    o: *mut i64,
    v: *mut i64,
    h: *mut u32,
    la: *mut u32,
    lt: *mut u32,
    c: *mut u32,
    kb: u32,
    eb: u32,
}

unsafe fn stage_count_one(st: &St, j: &Job, b: &Bufs, t: u32, acc: &mut [u32; 17]) {
    let s = st.s;
    let counts = b.c;
    if t > s.cap_in {
        return;
    }
    let n = (*j.totals.add(s.in_slot as usize)).min(s.cap_in);
    if t >= n {
        *counts.add(t as usize) = 0;
        return;
    }
    if s.mode == 6 {
        *counts.add(t as usize) = 1;
        return;
    }
    if s.mode == 7 {
        let (mut unheld, mut host) = (false, false);
        *counts.add(t as usize) = textop_pass(st, t, j.d, j.a, &mut unheld, &mut host) as u32;
        if s.counter != NONE && (unheld || host) {
            at(j.counters.add(s.counter as usize)).fetch_or(if unheld { 4 } else { 0 } | if host { 2 } else { 0 }, SeqCst);
        }
        return;
    }
    let mut kv = [0i64; 8];
    let w = st.candidates(t, j.f, &mut kv);
    if s.mode == 4 {
        let got = st.aggregate(t, w, &kv);
        if got == Err(2) && s.counter != NONE {
            at(j.counters.add(s.counter as usize)).store(1, SeqCst);
        }
        *counts.add(t as usize) = got.is_ok() as u32;
        return;
    }
    let exists = s.mode == 1 || s.mode == 2;
    let (mut c, mut p0, mut found) = (0u32, 0u32, false);
    let mut q_counts = [0u32; 16];
    let passed = s.npassed.min(16) as usize;
    // A join's finds (`V`): a bit a candidate of at most `FINDS_MAX` consecutive places, set where it is a row out.
    let kept = s.mode == 0 && s.v_has == 0 && s.v_parent == 0 && w.how <= 1 && w.hi.wrapping_sub(w.lo) <= FINDS_MAX;
    let mut finds = 0u32;
    st.each(w, &kv, |r, q, key| {
        if if s.mode == 3 { !st.row_live(r) } else { !st.matches(r, &kv, key) } {
            return true;
        }
        if exists {
            found = true;
            return false;
        }
        p0 += 1;
        let ok = st.defined(t, r);
        let v = if s.nops > 0 { st.eval(t, r) } else { 0 };
        for (q, &bit) in q_counts.iter_mut().zip(&s.passed).take(passed) {
            *q += (ok && (bit == NONE || st.out_bit(v, bit))) as u32;
        }
        if ok && (s.last == NONE || st.out_bit(v, s.last)) {
            if kept {
                finds |= 1 << (q.wrapping_sub(w.lo) & 31);
            }
            c += 1;
        }
        true
    });
    if exists {
        c = (found != (s.mode == 2)) as u32;
    } else if s.mode == 0 {
        *b.v.add(t as usize) = if kept { (u64::from(w.lo) | u64::from(finds) << 32) as i64 } else { FINDS_WALK };
    }
    *counts.add(t as usize) = c;
    if !exists {
        acc[0] += p0;
        for (a, &q) in acc[1..].iter_mut().zip(&q_counts).take(passed) {
            *a += q;
        }
    }
}

unsafe fn stage_scatter_one(st: &St, j: &Job, b: &Bufs, t: u32) {
    let s = st.s;
    let (offsets, o) = (b.c, b.o);
    let o0 = *offsets;
    let n = (*j.totals.add(s.in_slot as usize)).min(s.cap_in);
    if t == 0 {
        *j.totals.add(s.out_slot as usize) = (*offsets.add(n as usize)).wrapping_sub(o0);
    }
    if t >= n {
        return;
    }
    let mut a = (*offsets.add(t as usize)).wrapping_sub(o0);
    if a >= s.cap_out {
        return;
    }
    let kept = (*offsets.add(t as usize + 1)).wrapping_sub(o0) > a;
    match s.mode {
        7 | 1 | 2 => {
            if kept {
                st.write_row(t, 0, o, a);
            }
        }
        6 => {
            if kept {
                st.write_row(t, 0, o, a);
                let mut unheld = false;
                col_put(o, s.bind_slot[0], s.cap_out, a, strcmp_value(st, t, j.d, j.a, &mut unheld));
                if unheld && s.counter != NONE {
                    at(j.counters.add(s.counter as usize)).fetch_or(4, SeqCst);
                }
            }
        }
        4 => {
            if kept {
                let mut kv = [0i64; 8];
                let w = st.candidates(t, j.f, &mut kv);
                let v = st.aggregate(t, w, &kv);
                st.write_row(t, 0, o, a);
                if (s.agg[0] >> 8) & 1 != 0 {
                    // A kept row's aggregate is found: the count pass kept it for that.
                    col_put(o, s.bind_slot[0], s.cap_out, a, v.unwrap_or(0));
                }
            }
        }
        0 if *b.v.add(t as usize) as u32 != NONE => {
            // Its finds (`stage_count_one`'s): its rows out, each found once, none walked again.
            let found = *b.v.add(t as usize) as u64;
            let (first, mut bits) = (found as u32, (found >> 32) as u32);
            while bits != 0 && a < s.cap_out {
                let q = first + bits.trailing_zeros();
                st.write_row(t, if s.nkeys == 0 { q } else { st.cand_row(1, q) }, o, a);
                bits &= bits - 1;
                a += 1;
            }
        }
        _ => {
            let mut kv = [0i64; 8];
            let w = st.candidates(t, j.f, &mut kv);
            st.each(w, &kv, |r, _, key| {
                if a >= s.cap_out {
                    return false;
                }
                if if s.mode == 3 { !st.row_live(r) } else { !st.matches(r, &kv, key) } {
                    return true;
                }
                if !st.defined(t, r) || (s.last != NONE && !st.out_bit(st.eval(t, r), s.last)) {
                    return true;
                }
                st.write_row(t, r, o, a);
                a += 1;
                true
            });
        }
    }
}

/// A witness's probe: whether relation `r` has a row of `b`'s keys (an anti-join's: none).
unsafe fn witness_probe(b: &Stage, l: *const i64, t: u32, cap_in: u32, a: &St, ra: u32, r: *const i64, rf: *const u8) -> bool {
    let mut kv = [0i64; 8];
    for (i, k) in kv.iter_mut().enumerate().take(b.nkeys.min(8) as usize) {
        *k = match b.key_kind[i] {
            1 => col(l, b.key_slot[i], cap_in, t),
            2 => a.rget(ra, b.key_slot[i]),
            _ => b.key_const[i],
        };
    }
    let bst = St::new(b, l, r, rf);
    let w = bst.probe(&kv);
    let mut found = false;
    bst.each(w, &kv, |row, _, key| {
        found = bst.matches(row, &kv, key);
        !found
    });
    found != (b.mode == 2)
}

unsafe fn witness_count_one(st: &St, j: &Job, b: &Bufs, t: u32) {
    let s = st.s;
    if t > s.cap_in {
        return;
    }
    let n = (*j.totals.add(s.in_slot as usize)).min(s.cap_in);
    if t >= n {
        *b.c.add(t as usize) = 0;
        return;
    }
    let mut kv = [0i64; 8];
    let w = st.candidates(t, j.f, &mut kv);
    let nb = s.agg[0].min(3) as usize;
    let mut found = 0;
    st.each(w, &kv, |r, _, key| {
        if !st.matches(r, &kv, key) {
            return true;
        }
        let all = (0..nb).all(|k| witness_probe(&*j.bs.add(k), st.l, t, s.cap_in, st, r, j.w[k], j.wf[k]));
        if all {
            found = 1;
        }
        !all
    });
    *b.c.add(t as usize) = found;
}

/// Row `t` of a copy that reads a view or a relation, or writes a relation: a read and a write each value.
unsafe fn rows_copy_one(src: *const u8, far: *const u8, dst: *mut u8, c: &Copy, n: u32, t: u32, f: &mut Found<'_>) {
    if t >= n {
        return;
    }
    let base = if c.flags & 1 != 0 { *c.off } else { 0 };
    let grown = if c.flags & 2 != 0 { rows_len(dst, c.dst_rows_at) } else { 0 };
    let a = c.dst_off.wrapping_add(base).wrapping_add(grown).wrapping_add(t);
    let row = if c.r_view != 0 { Some(vrow(c.r_view, c.v_dir, t, f)) } else { None };
    for k in 0..c.ncols.min(16) as usize {
        let from = c.src[k];
        let v = if from == NONE {
            c.konst[k]
        } else if let Some(row) = row {
            row.get(from, f)
        } else if c.src_at == 0 {
            col(src as *const i64, from, c.src_stride, t)
        } else {
            rel_get(src, c.src_at, c.src_from.wrapping_add(t), from, far) as i64
        };
        if c.dst_rows_at == 0 {
            col_put(dst as *mut i64, k as u32, c.dst_stride, a, v);
        } else {
            rows_put(dst, c.dst_rows_at, a, k as u32, v as u64);
        }
    }
}

/// A join from one binding's (a rule's first table's) candidates, a thread each: where its stage probes an index by
/// keys, the places of its keys' group (`St::probe`), from `lo`; else the relation's rows from `r_from`. `n` of them;
/// `tag` the keys' (decision 0671). A scan of every row of the relation checked each row's keys: `node(_, n, K)` read
/// every node of the batch for its kind, where the host reads that kind's nodes alone (`Batch::by_kind`).
#[derive(Clone, Copy)]
struct ScanCands {
    lo: u32,
    n: u32,
    tag: u32,
    grouped: bool,
}

impl St<'_> {
    /// The scan's keys (slot 0's or constants) and its candidates (`ScanCands`).
    unsafe fn scan_cands(&self, j: &Job) -> ([i64; 8], ScanCands) {
        let s = self.s;
        let mut kv = [0i64; 8];
        for (i, k) in kv.iter_mut().enumerate().take(s.nkeys.min(8) as usize) {
            *k = if s.key_kind[i] == 1 { col(self.l, s.key_slot[i], s.cap_in, 0) } else { s.key_const[i] };
        }
        let rows = if *j.totals.add(s.in_slot as usize) == 0 { 0 } else { self.rows() };
        if s.nkeys == 0 || s.v_parent == 1 || s.v_has != 0 || rows == 0 {
            return (kv, ScanCands { lo: s.r_from, n: rows, tag: 0, grouped: false });
        }
        // Its keys' index group, or a node's own row (`by_own`).
        let w = self.probe(&kv);
        (kv, ScanCands { lo: w.lo, n: w.hi.wrapping_sub(w.lo).min(rows), tag: w.tag, grouped: w.how == 1 })
    }

    /// Thread `r`'s candidate of `c`: its row; `None` past them, or at a place of another key's tag (as a probe's walk
    /// passes it, `St::each`).
    #[inline]
    unsafe fn scan_cand(&self, c: &ScanCands, r: u32) -> Option<u32> {
        if r >= c.n {
            return None;
        }
        if !c.grouped {
            return Some(c.lo.wrapping_add(r));
        }
        let s = self.s;
        let q = c.lo.wrapping_add(r);
        let m = rel_group_mark(self.r as *const u8, s.r_at, s.r_index, q);
        if (m ^ c.tag) & !MARK_SAME != 0 {
            return None;
        }
        Some(rel_group_row(self.r as *const u8, s.r_at, s.r_index, q))
    }
}

unsafe fn scan_count_one(st: &St, j: &Job, b: &Bufs, kv: &[i64; 8], cands: &ScanCands, r: u32) {
    let s = st.s;
    let counts = b.c;
    if r > s.nr {
        return;
    }
    let Some(id) = st.scan_cand(cands, r) else {
        *counts.add(r as usize) = 0;
        return;
    };
    let mut c = 0;
    if st.matches(id, kv, false) {
        let v = if s.nops > 0 { st.eval(0, id) } else { 0 };
        if s.counter != NONE {
            at(j.counters.add(s.counter as usize)).fetch_add(1, SeqCst);
            for m in 0..s.npassed.min(16) as usize {
                if s.passed[m] == NONE || st.out_bit(v, s.passed[m]) {
                    at(j.counters.add(s.counter as usize + 1 + m)).fetch_add(1, SeqCst);
                }
            }
        }
        c = (s.last == NONE || st.out_bit(v, s.last)) as u32;
    }
    *counts.add(r as usize) = c;
}

// ---- A head's distinct rows. ----

unsafe fn same_row(d: &Dedup, t: *const i64, a: u32, b: u32) -> bool {
    d.cols[..d.ncols.min(16) as usize].iter().all(|&c| col(t, c, d.cap, a) == col(t, c, d.cap, b))
}

/// The places a dedup uses (`dedup_mask`): twice its rows' count, a power of two (16 at least), within its own.
unsafe fn dedup_mask(totals: *const u32, d: &Dedup) -> u32 {
    let two = 2 * (*totals.add(d.in_slot as usize)).min(d.cap) as u64;
    let p = if two <= 16 { 16 } else { two.next_power_of_two() };
    (p - 1).min(d.mask as u64) as u32
}

unsafe fn ix_rows(bb: *const u8, p: &IxP) -> u32 {
    if p.ncols & IX_LIVE == 0 {
        return p.n;
    }
    let len = rel_len(bb, p.at);
    if len > p.from { (len - p.from).min(p.n) } else { 0 }
}

/// The index's row `t` (from its first, `from`), found once for its key columns; a relation's read with its far
/// base's block `far`.
#[inline]
unsafe fn ix_row(bb: *const u8, far: *const u8, p: &IxP, t: u32, f: &Found<'_>) -> Row {
    if p.kind != 0 { vrow(p.kind as u32, p.dir, t, f) } else { Row::Rel(bb, p.at, p.from.wrapping_add(t), far) }
}

/// Row `t`'s key hash (`key_hash`).
unsafe fn ix_hash64(bb: *const u8, far: *const u8, p: &IxP, t: u32, f: &mut Found<'_>) -> u64 {
    let row = ix_row(bb, far, p, t, f);
    let mut h = KEY_SEED;
    for i in 0..(p.ncols & !IX_LIVE).min(8) as usize {
        h = key_mix(h, row.get(p.cols[i], f));
    }
    h
}

/// `MARK_REACH` (decision 0671): the places before one a builder looks back over for its tag.
const MARK_REACH: u32 = 8;

/// `join_stages`' `ix_mark_one` of places `t0..t1`: each one's key's tag, and `MARK_SAME` where its bucket's previous
/// place of the tag, among the `MARK_REACH` before it, holds its key. Its marks after its rows' room (`n`). Before its
/// rows are placed (`ix_place_range`): a place's row is its sorted place's (`places`), its tag its key's pass's, at
/// the row's own place in the rows' room (`ix_key_range`), so no tag reads a row again.
unsafe fn ix_mark_range(bb: *const u8, rf: *const u8, keys: *const u32, places: *const u32, offs: *const u32, rows: *mut u32, p: &IxP, kb: u32, eb: u32, t0: u32, t1: u32) {
    let t1 = t1.min(ix_rows(bb, p));
    let far = far_block(bb, rf);
    let row = |u: u32| (*places.add((eb + u) as usize)).wrapping_sub(eb);
    let tag = |r: u32| *rows.add(r as usize);
    // The places' rows, the sorted level's, are in key order: a document's rows apart. A pair compared reads each
    // through its own found document (`Found`). A place's key, once read, is kept for the places after it that look
    // back to it (`Keys`): its tag's previous place is most often the one just before, whose key that place read as
    // its own, so each row's key is read once, not twice (the compares' reads were most of the marks' cost).
    let (mut fu, mut ft) = (Found::new(), Found::new());
    let nk = (p.ncols & !IX_LIVE).min(8) as usize;
    let mut kept = Keys::new();
    for t in t0..t1 {
        let lo = *offs.add((*keys.add((eb + t) as usize)).wrapping_sub(kb) as usize);
        let rt = row(t);
        let tg = tag(rt);
        let mut same = false;
        for u in (lo.max(t.saturating_sub(MARK_REACH))..t).rev() {
            let ru = row(u);
            if tag(ru) != tg {
                continue;
            }
            let a = *kept.of(u, || ix_key(bb, far, p, ru, nk, &mut fu));
            let b = kept.of(t, || ix_key(bb, far, p, rt, nk, &mut ft));
            same = a[..nk] == b[..nk];
            break;
        }
        *rows.add((p.n + t) as usize) = tg | u32::from(same);
    }
}

/// Row `r`'s key, its first `nk` values (`ix_row`'s columns `p.cols`).
#[inline(always)]
unsafe fn ix_key(bb: *const u8, far: *const u8, p: &IxP, r: u32, nk: usize, f: &mut Found<'_>) -> [i64; 8] {
    let row = ix_row(bb, far, p, r, f);
    let mut key = [0i64; 8];
    for (i, k) in key.iter_mut().enumerate().take(nk) {
        *k = row.get(p.cols[i], f);
    }
    key
}

/// The keys of the places a mark's look-back reaches (`MARK_REACH` before it, and itself), read once each: place `t`
/// in slot `t % KEYS_KEPT`.
struct Keys {
    place: [u32; KEYS_KEPT],
    key: [[i64; 8]; KEYS_KEPT],
}

/// Slots for a place and the `MARK_REACH` before it.
const KEYS_KEPT: usize = 16;
const _: () = assert!(KEYS_KEPT > MARK_REACH as usize);

impl Keys {
    fn new() -> Self {
        Keys { place: [NONE; KEYS_KEPT], key: [[0; 8]; KEYS_KEPT] }
    }

    /// Place `t`'s key: kept, else `read` and kept.
    #[inline(always)]
    fn of(&mut self, t: u32, read: impl FnOnce() -> [i64; 8]) -> &[i64; 8] {
        let s = t as usize % KEYS_KEPT;
        if self.place[s] != t {
            self.key[s] = read();
            self.place[s] = t;
        }
        &self.key[s]
    }
}

// ---- A lattice relation's rounds (decision 0124). ----

unsafe fn lat_r(r: *const u8, p: &LatP, row: u32, c: u32) -> i64 {
    // The device's own relation, one part in its block: no far base.
    rel_get(r, p.r_at, row, c, r) as i64
}

unsafe fn lat_put(r: *const u8, p: &LatP, row: u32, c: u32, v: i64) {
    rows_put(r, p.r_rows_at, row, c, v as u64);
}

unsafe fn lat_key(u: *const i64, p: &LatP, x: u32) -> [i64; 8] {
    let mut key = [0i64; 8];
    for c in 0..p.nkeys.min(8) {
        key[c as usize] = col(u, c, p.n, x);
    }
    key
}

unsafe fn lat_hash(key: &[i64; 8], nk: u32, mask: u32) -> u32 {
    (key_hash(&key[..nk.min(8) as usize]) as u32) & mask
}

unsafe fn lat_same(u: *const i64, stride: u32, a: u32, b: u32, nk: u32) -> bool {
    (0..nk.min(8)).all(|c| col(u, c, stride, a) == col(u, c, stride, b))
}

/// Its rows: `n`, or as many as an earlier phase's word says (`counted`).
unsafe fn lat_rows(p: &LatP, cnt: *const u32) -> u32 {
    if p.counted & 1 != 0 { (*cnt).min(p.n) } else { p.n }
}

/// Code 4 (a set's map, `Dedup::seen`): row `s` of the relation's first `n` placed instead, each row a key of its own.
/// Code 7 (a set's map its dedup only read, `Dedup::seen` 2): row `s` of a round's new rows written past the
/// relation's rows (`Batch::append_live`), as many as the word at `h0_at` of `ob` holds (at most `n`), placed as code
/// 4's, after its head's gate: a park before it grew the map with every row it held.
/// Code 6 (a set's held bitmap, t58 increment A): row `s`'s bit set, at its index under the key past `p` (`held_index`).
unsafe fn lat_rehash_one(r: *const u8, ob: *const u8, hb: *mut u8, p: &LatP, s: u32) {
    if p.code == 6 {
        if s >= p.n {
            return;
        }
        let k = &*((p as *const LatP).add(1) as *const HeadKeyP);
        let x = held_index(k, |c| lat_r(r, p, s, c) as u64);
        at((hb.add(p.h_at as usize) as *mut u32).add((x >> 5) as usize)).fetch_or(1 << (x & 31), Ordering::Relaxed);
        return;
    }
    if p.code == 4 || p.code == 7 {
        let row = if p.code == 7 {
            if s >= (*(ob.add(p.h0_at as usize) as *const u32)).min(p.n) {
                return;
            }
            rows_len(r, p.r_rows_at) + s
        } else if s >= p.n {
            return;
        } else {
            s
        };
        let h = hb.add(p.h_at as usize) as *mut u32;
        let mut key = [0i64; 8];
        for c in 0..p.nkeys.min(8) {
            key[c as usize] = lat_r(r, p, row, c);
        }
        let mut a = lat_hash(&key, p.nkeys, p.hmask);
        for _ in 0..=p.hmask {
            if at(h.add(3 * a as usize)).compare_exchange(NONE, row, SeqCst, SeqCst).is_ok() {
                return;
            }
            a = (a + 1) & p.hmask;
        }
        return;
    }
    // A program's tail's growth of a map that did not grow (`omask` all ones, `Batch::lattice_tail`): nothing.
    if s > p.omask || p.omask == NONE {
        return;
    }
    let old = ob.add(p.h0_at as usize) as *const u32;
    let t = *old.add(3 * s as usize);
    if t == NONE {
        return;
    }
    let h = hb.add(p.h_at as usize) as *mut u32;
    let mut key = [0i64; 8];
    for c in 0..p.nkeys.min(8) {
        key[c as usize] = lat_r(r, p, t, c);
    }
    let mut a = lat_hash(&key, p.nkeys, p.hmask);
    for _ in 0..=p.hmask {
        if at(h.add(3 * a as usize)).compare_exchange(NONE, t, SeqCst, SeqCst).is_ok() {
            at(h.add(3 * a as usize + 1)).store(*old.add(3 * s as usize + 1), SeqCst);
            at(h.add(3 * a as usize + 2)).store(*old.add(3 * s as usize + 2), SeqCst);
            return;
        }
        a = (a + 1) & p.hmask;
    }
}

unsafe fn lat_claim_one(u: *const i64, pl: *mut u32, cnt: *const u32, p: &LatP, x: u32) {
    if x >= lat_rows(p, cnt) {
        return;
    }
    let mut a = lat_hash(&lat_key(u, p, x), p.nkeys, p.pmask);
    for _ in 0..=p.pmask {
        let place = at(pl.add(a as usize));
        match place.compare_exchange(NONE, x, SeqCst, SeqCst) {
            Ok(_) => return,
            Err(seen) if lat_same(u, p.n, seen, x, p.nkeys) => {
                place.fetch_min(x, SeqCst);
                return;
            }
            Err(_) => a = (a + 1) & p.pmask,
        }
    }
}

/// The place of row `x`'s key in `pl` (claimed: it is there).
unsafe fn lat_place(u: *const i64, pl: *const u32, p: &LatP, x: u32) -> u32 {
    let mut a = lat_hash(&lat_key(u, p, x), p.nkeys, p.pmask);
    for _ in 0..=p.pmask {
        let y = *pl.add(a as usize);
        if y == NONE {
            return NONE;
        }
        if lat_same(u, p.n, y, x, p.nkeys) {
            return a;
        }
        a = (a + 1) & p.pmask;
    }
    NONE
}

/// Code 3 (`shortlex_min`): whether arena text `a` is before `b`, by their characters, then their bytes.
unsafe fn lat_text_less(arena: *const u32, a: i64, b: i64) -> bool {
    let text = |v: i64| {
        let v = v as u64;
        Txt::raw(arena_bytes(arena).add(((v & 0x7fff_ffff_ffff_ffff) >> 24) as u32 as usize), (v & 0xff_ffff) as u32)
    };
    let (x, y) = (text(a), text(b));
    let (cx, cy) = (x.chars(), y.chars());
    if cx != cy {
        return cx < cy;
    }
    x.cmp(&y).is_lt()
}

unsafe fn lat_better(p: &LatP, arena: *const u32, a: i64, ra: u32, b: i64, rb: u32) -> bool {
    if b == p.top {
        return false;
    }
    if a == p.top {
        return true;
    }
    if a == b {
        return ra < rb;
    }
    if p.code == 3 {
        return lat_text_less(arena, a, b) || (!lat_text_less(arena, b, a) && ra < rb);
    }
    if p.code == 0 { a < b } else { a > b }
}

unsafe fn lat_best_one(u: *const i64, pl: *const u32, best: *mut u32, flags: *mut u32, arena: *const u32, cnt: *const u32, p: &LatP, x: u32) {
    if x >= lat_rows(p, cnt) {
        return;
    }
    let s = lat_place(u, pl, p, x);
    if s == NONE {
        return;
    }
    let v = col(u, p.nkeys, p.n, x);
    if p.code == 2 {
        if v != col(u, p.nkeys, p.n, *pl.add(s as usize)) {
            at(flags.add(s as usize)).fetch_or(1, SeqCst);
        }
        return;
    }
    let place = at(best.add(s as usize));
    let mut cur = place.load(SeqCst);
    for _ in 0..4 * p.n + 8 {
        if cur != NONE && !lat_better(p, arena, v, x, col(u, p.nkeys, p.n, cur), cur) {
            return;
        }
        match place.compare_exchange(cur, x, SeqCst, SeqCst) {
            Ok(_) => return,
            Err(seen) => cur = seen,
        }
    }
}

unsafe fn lat_lub(p: &LatP, arena: *const u32, a: i64, b: i64) -> i64 {
    if a == p.top || b == p.top {
        return p.top;
    }
    match p.code {
        3 => {
            if lat_text_less(arena, b, a) { b } else { a }
        }
        0 => a.min(b),
        1 => a.max(b),
        _ => {
            if a == b { a } else { p.top }
        }
    }
}

fn lat_widen(p: &LatP, old: i64, now: i64) -> i64 {
    if (p.code == 0 && now < old) || (p.code == 1 && now > old) { p.top } else { now }
}

unsafe fn lat_merge_one(u: *const i64, pl: *const u32, best: *const u32, flags: *const u32, r: *mut u8, hb: *mut u8, d: *mut i64, counters: *mut u32, arena: *const u32, round: *const u32, pp: &LatP, s: u32) {
    let mut p = pp.clone();
    if p.counted & 2 != 0 {
        p.had = rows_len(r, p.r_rows_at);
    }
    if p.counted & 4 != 0 {
        p.level = *round;
    }
    let h = hb.add(p.h_at as usize) as *mut u32;
    if s > p.pmask {
        return;
    }
    let rep = *pl.add(s as usize);
    if rep == NONE {
        return;
    }
    let b = *best.add(s as usize);
    let row = if p.code == 2 || b == NONE { rep } else { b };
    let mut v = col(u, p.nkeys, p.n, row);
    if p.code == 2 && *flags.add(s as usize) != 0 {
        v = p.top;
    }
    let key = lat_key(u, &p, row);
    let nk = p.nkeys.min(8);
    let mut a = lat_hash(&key, p.nkeys, p.hmask);
    let mut held = NONE;
    for _ in 0..=p.hmask {
        let hrow = at(h.add(3 * a as usize)).load(SeqCst);
        if hrow == NONE {
            break;
        }
        if hrow < p.had && (0..nk).all(|c| lat_r(r, &p, hrow, c) == key[c as usize]) {
            held = hrow;
            break;
        }
        a = (a + 1) & p.hmask;
    }
    if held != NONE {
        let o = lat_r(r, &p, held, p.nkeys);
        let mut m = lat_lub(&p, arena, o, v);
        if m == o {
            return;
        }
        let changes = at(h.add(3 * a as usize + 1)).load(SeqCst);
        if p.after != NONE && changes >= p.after {
            m = lat_widen(&p, o, m);
            if m == o {
                return;
            }
        }
        lat_put(r, &p, held, p.nkeys, m);
        at(h.add(3 * a as usize + 1)).store(changes + 1, SeqCst);
        if p.level != 0 {
            at(h.add(3 * a as usize + 2)).store(p.level, SeqCst);
        }
        v = m;
    } else {
        let id = p.had.wrapping_add(at(counters.add(1)).fetch_add(1, SeqCst));
        for c in 0..nk {
            lat_put(r, &p, id, c, key[c as usize]);
        }
        lat_put(r, &p, id, p.nkeys, v);
        let mut place = lat_hash(&key, p.nkeys, p.hmask);
        for _ in 0..=p.hmask {
            if at(h.add(3 * place as usize)).compare_exchange(NONE, id, SeqCst, SeqCst).is_ok() {
                at(h.add(3 * place as usize + 1)).store(1, SeqCst);
                at(h.add(3 * place as usize + 2)).store(p.level, SeqCst);
                break;
            }
            place = (place + 1) & p.hmask;
        }
    }
    let k = at(counters).fetch_add(1, SeqCst);
    for c in 0..nk {
        col_put(d, c, p.n, k, key[c as usize]);
    }
    col_put(d, p.nkeys, p.n, k, v);
}

// ---- A group's threads as a range [t0, t1): what each thread does alike, a vector of them at a time. ----

/// Words [from, to) of `p` set to `v`, 16 a store.
unsafe fn fill_u32(p: *mut u32, from: u32, to: u32, v: u32) {
    let (mut i, to) = (from as usize, to as usize);
    let x = u32x16::splat(v);
    while i + 16 <= to {
        st(p.add(i), x);
        i += 16;
    }
    while i < to {
        *p.add(i) = v;
        i += 1;
    }
}

/// `n` values set to `v`, 4 a store.
unsafe fn fill_i64(dst: *mut i64, n: u32, v: i64) {
    let x = i64x4::splat(v);
    let mut i = 0;
    while i + 4 <= n as usize {
        st(dst.add(i), x);
        i += 4;
    }
    for i in i..n as usize {
        *dst.add(i) = v;
    }
}

/// `n` values from `src` to `dst`: one copy, or each shifted to its document (`doc`: its high word), 4 a step.
unsafe fn copy_col(src: *const i64, dst: *mut i64, n: u32, doc: bool) {
    if !doc {
        core::ptr::copy(src, dst, n as usize);
        return;
    }
    let mut i = 0;
    while i + 4 <= n as usize {
        st(dst.add(i), ld::<i64, 4>(src.add(i)) >> i64x4::splat(32));
        i += 4;
    }
    for i in i..n as usize {
        *dst.add(i) = *src.add(i) >> 32;
    }
}

/// Row `t`'s place in column `c` of a table of columns `stride` rows apart.
#[inline(always)]
fn colp(l: *const i64, c: u32, stride: u32, t: u32) -> *const i64 {
    l.wrapping_add(c as usize * stride as usize + t as usize)
}

#[inline(always)]
fn colp_mut(o: *mut i64, c: u32, stride: u32, t: u32) -> *mut i64 {
    o.wrapping_add(c as usize * stride as usize + t as usize)
}

/// The kept rows of [t0, t1) by their scanned counts (`offsets[t + 1] - offsets[t]`, 0 or 1): each run of rows whose
/// places out follow one another, as its first row, its first place and its rows. Four rows' counts a compare: a
/// vector of none skipped, a vector of ones a run's four more rows.
unsafe fn kept_runs(offsets: *const u32, o0: u32, t0: u32, t1: u32, mut f: impl FnMut(u32, u32, u32)) {
    let (mut start, mut first, mut len) = (0u32, 0u32, 0u32);
    let mut t = t0;
    while t < t1 {
        if t + 4 <= t1 {
            let lo = ld::<u32, 4>(offsets.add(t as usize));
            let d = ld::<u32, 4>(offsets.add(t as usize + 1)) - lo;
            let (mx, mn) = (d.reduce_max(), d.reduce_min());
            if mx == 0 {
                if len > 0 {
                    f(start, first, len);
                    len = 0;
                }
                t += 4;
                continue;
            }
            if mx == 1 && mn == 1 {
                let a = lo[0].wrapping_sub(o0);
                if len == 0 || a != first.wrapping_add(len) {
                    if len > 0 {
                        f(start, first, len);
                    }
                    (start, first, len) = (t, a, 0);
                }
                len += 4;
                t += 4;
                continue;
            }
        }
        let a = (*offsets.add(t as usize)).wrapping_sub(o0);
        let d = (*offsets.add(t as usize + 1)).wrapping_sub(*offsets.add(t as usize));
        if d == 0 {
            if len > 0 {
                f(start, first, len);
                len = 0;
            }
        } else if len > 0 && a == first.wrapping_add(len) {
            len += 1;
        } else {
            if len > 0 {
                f(start, first, len);
            }
            (start, first, len) = (t, a, 1);
        }
        t += 1;
    }
    if len > 0 {
        f(start, first, len);
    }
}

/// A run's rows below `cap` places out.
fn below(first: u32, len: u32, cap: u32) -> u32 {
    if first >= cap { 0 } else { len.min(cap - first) }
}

unsafe fn stage_count_range(st: &St, j: &Job, b: &Bufs, t0: u32, t1: u32) {
    let s = st.s;
    let t1 = t1.min(s.cap_in.saturating_add(1));
    if t0 >= t1 {
        return;
    }
    let n = (*j.totals.add(s.in_slot as usize)).min(s.cap_in);
    fill_u32(b.c, t0.max(n), t1, 0);
    let live = t1.min(n);
    if s.mode == 6 {
        fill_u32(b.c, t0, live, 1);
        return;
    }
    // The stage's counters, the range's added once.
    let mut acc = [0u32; 17];
    if stage_lanes::runs_at_lanes(s) {
        stage_lanes::stage_count_lanes::<{ stage_lanes::LANES }>(st, j, b.c, b.v, t0.min(live), live, &mut acc);
    } else {
        for t in t0..live {
            stage_count_one(st, j, b, t, &mut acc);
        }
    }
    if s.counter != NONE {
        for (m, &a) in acc.iter().enumerate().filter(|(_, a)| **a > 0) {
            at(j.counters.add(s.counter as usize + m)).fetch_add(a, SeqCst);
        }
    }
}

unsafe fn witness_count_range(st: &St, j: &Job, b: &Bufs, t0: u32, t1: u32) {
    let s = st.s;
    let t1 = t1.min(s.cap_in.saturating_add(1));
    let n = (*j.totals.add(s.in_slot as usize)).min(s.cap_in);
    fill_u32(b.c, t0.max(n), t1, 0);
    for t in t0..t1.min(n) {
        witness_count_one(st, j, b, t);
    }
}

unsafe fn scan_count_range(st: &St, j: &Job, b: &Bufs, t0: u32, t1: u32) {
    let s = st.s;
    let t1 = t1.min(s.nr.saturating_add(1));
    if t0 >= t1 {
        return;
    }
    let (kv, cands) = st.scan_cands(j);
    fill_u32(b.c, t0.max(cands.n), t1, 0);
    for r in t0..t1.min(cands.n) {
        scan_count_one(st, j, b, &kv, &cands, r);
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn dedup_mark_range(t: *const i64, totals: *const u32, p: *const u32, counts: *mut u32, r: *const u8, hb: *const u8, d: &Dedup, x0: u32, x1: u32) {
    let x1 = x1.min(d.cap.saturating_add(1));
    let n = (*totals.add(d.in_slot as usize)).min(d.cap);
    fill_u32(counts, x0.max(n), x1, 0);
    dedup_lanes::dedup_mark_lanes::<{ stage_lanes::LANES }>(t, p, counts, r, hb, d, x0.min(n), x1.min(n));
}

unsafe fn stage_scatter_range(st: &St, j: &Job, b: &Bufs, t0: u32, t1: u32) {
    let s = st.s;
    if s.mode == 0 && stage_lanes::runs_at_lanes(s) {
        let n = (*j.totals.add(s.in_slot as usize)).min(s.cap_in);
        if t0 == 0 && t1 > 0 {
            *j.totals.add(s.out_slot as usize) = (*b.c.add(n as usize)).wrapping_sub(*b.c);
        }
        stage_lanes::stage_scatter_lanes::<{ stage_lanes::LANES }>(st, j, b.c, b.o, b.v, t0.min(n), t1.min(n));
        return;
    }
    if !matches!(s.mode, 1 | 2 | 7) {
        for t in t0..t1 {
            stage_scatter_one(st, j, b, t);
        }
        return;
    }
    // A binding kept as it is: its slots a run's copy of each column, its binds a row at a time.
    let (offsets, o) = (b.c, b.o);
    let o0 = *offsets;
    let n = (*j.totals.add(s.in_slot as usize)).min(s.cap_in);
    if t0 == 0 && t1 > 0 {
        *j.totals.add(s.out_slot as usize) = (*offsets.add(n as usize)).wrapping_sub(o0);
    }
    kept_runs(offsets, o0, t0, t1.min(n), |t, a, len| {
        let len = below(a, len, s.cap_out);
        for k in 0..s.nslots {
            copy_col(colp(st.l, k, s.cap_in, t), colp_mut(o, k, s.cap_out, a), len, false);
        }
        for i in 0..len {
            st.write_binds(t + i, 0, o, a + i);
        }
    });
}

unsafe fn row_scatter_range(st: &St, j: &Job, b: &Bufs, t0: u32, t1: u32) {
    let s = st.s;
    let (offsets, o) = (b.c, b.o);
    let o0 = *offsets;
    let n = (*j.totals.add(s.in_slot as usize)).min(s.cap_in);
    if t0 == 0 && t1 > 0 {
        *j.totals.add(s.out_slot as usize) = (*offsets.add(n as usize)).wrapping_sub(o0);
    }
    let op = s.agg[0] & 15;
    kept_runs(offsets, o0, t0, t1.min(n), |t, a, len| {
        let len = below(a, len, s.cap_out);
        for k in 0..s.nslots {
            copy_col(colp(st.l, k, s.cap_in, t), colp_mut(o, k, s.cap_out, a), len, false);
        }
        if op != 3 && op != 5 {
            copy_col(b.v.add(t as usize), colp_mut(o, s.bind_slot[0], s.cap_out, a), len, false);
        }
    });
}

unsafe fn scan_scatter_range(st: &St, j: &Job, b: &Bufs, r0: u32, r1: u32) {
    let s = st.s;
    let offsets = b.c;
    let o0 = *offsets;
    let rows = st.rows();
    if r0 == 0 && r1 > 0 {
        *j.totals.add(s.out_slot as usize) = (*offsets.add(rows as usize)).wrapping_sub(o0);
    }
    // Past its candidates every count is none (`scan_count_range`): nothing kept there to walk.
    let (_, cands) = st.scan_cands(j);
    kept_runs(offsets, o0, r0, r1.min(rows).min(cands.n), |r, a, len| {
        for i in 0..below(a, len, s.cap_out) {
            let id = st.scan_cand(&cands, r + i).expect("a kept candidate is one of the scan's");
            st.write_row(0, id, b.o, a + i);
        }
    });
}

/// A sorted dedup's key (`HeadKeyP`, t58 increment A): the head's first order packed to its columns' bounds.
#[repr(C)]
struct HeadKeyP {
    /// Reserved, 0: where a tag's flag was (no order carries a tag, decision 0187 P1), kept for the layout.
    _reserved: u32,
    nparts: u32,
    rank_shift: u32,
    rank: u32,
    /// Column (of the dedup's `cols`), bits, least's low and high words.
    part: [[u32; 4]; 16],
    /// The level's wide sort's keys (its size script's), and whether its passes are odd: its sorted keys are then in
    /// the region's second pair (`WideAt`).
    total: u32,
    odd: u32,
    pad: [u32; 2],
}
const _: () = assert!(size_of::<HeadKeyP>() == 288);

/// The level's wide sort's region in the arena (t58 increment A), from its first word (`h_at`, `SortWords::wide`, its
/// size script's claim) and its keys: keys A and B (64 bits each), then its tiles' histograms. A key holds its whole
/// row (`head_key`), so the sort moves keys alone, and the kept rows are decoded from them (`head_row`).
#[derive(Clone, Copy)]
struct WideAt {
    keys: [*mut u64; 2],
    hist: *mut u32,
}

/// # Safety
/// `base` is the region's first word, and it holds `n` keys twice, and the histograms past them.
unsafe fn wide_at(base: *mut u32, n: u32) -> WideAt {
    let n = n as usize;
    let k = base as *mut u64;
    WideAt { keys: [k, k.add(n)], hist: base.add(4 * n) }
}

/// Row `x`'s key (`head_key`).
#[inline(always)]
unsafe fn head_key(t: *const i64, d: &Dedup, p: &HeadKeyP, x: u32) -> u64 {
    let val = |c: u32| *colp(t, d.cols[c as usize], d.cap, x) as u64;
    let mut k = 0u64;
    for part in &p.part[..p.nparts.min(16) as usize] {
        let w = part[1];
        if w == 0 {
            continue;
        }
        let v = val(part[0]).wrapping_sub(u64::from(part[2]) | u64::from(part[3]) << 32);
        k = if w >= 64 { v } else { k << w | v };
    }
    if p.rank_shift >= 64 { k } else { k | u64::from(p.rank) << p.rank_shift }
}

/// A row's index among every row its key's bounds allow (a held bitmap's, `HeldBits`): its parts less their least,
/// end to end, no tag; `val(c)` its value in column `c` of the key's.
#[inline(always)]
unsafe fn held_index(p: &HeadKeyP, val: impl Fn(u32) -> u64) -> u64 {
    let mut x = 0u64;
    for part in &p.part[..p.nparts.min(16) as usize] {
        let w = part[1];
        if w == 0 || w >= 64 {
            continue;
        }
        x = x << w | val(part[0]).wrapping_sub(u64::from(part[2]) | u64::from(part[3]) << 32);
    }
    x
}

/// A sorted dedup's held bitmap (`Dedup::seen`, its index bits in `hmask`, its first word at `h_at` of `vc`), and its
/// keys' index mask: a key's low bits are its index (its tag and rank above them, `head_key`).
#[inline(always)]
unsafe fn held_bits(d: &Dedup, vc: *mut u8) -> Option<(*mut u32, u64)> {
    (d.seen != 0).then(|| (vc.add(d.h_at as usize) as *mut u32, (1u64 << d.hmask) - 1))
}

/// Key `k`'s row (`head_key`'s, read back): each part from the key's low bits up, its least added, into its column of
/// `o`'s row `at`. Every column of the dedup's is a part (`HeadKey`: the order is on every column).
#[inline(always)]
unsafe fn head_row(mut k: u64, p: &HeadKeyP, o: *mut i64, ocap: u32, at: u32) {
    for part in p.part[..p.nparts.min(16) as usize].iter().rev() {
        let w = part[1];
        let v = if w >= 64 { k } else { k & ((1u64 << w) - 1) };
        k = if w >= 64 { 0 } else { k >> w };
        *colp_mut(o, part[0], ocap, at) = v.wrapping_add(u64::from(part[2]) | u64::from(part[3]) << 32) as i64;
    }
}

/// Rows `x0..x1`' keys at `eb` of the level's wide sort (`dedup_key_one`).
unsafe fn dedup_key_range(t: *const i64, totals: *const u32, keys: *mut u64, d: &Dedup, p: &HeadKeyP, eb: u32, x0: u32, x1: u32) {
    let n = (*totals.add(d.in_slot as usize)).min(d.cap);
    for x in x0..x1.min(n) {
        *keys.add((eb + x) as usize) = head_key(t, d, p, x);
    }
}

/// Sorted keys `t0..t1`: each kept where it is not the one before it, and its held bit (where it has a bitmap) is
/// clear (`dedup_mark_sorted_one`).
unsafe fn dedup_mark_sorted_range(totals: *const u32, keys: *const u64, counts: *mut u32, d: &Dedup, vc: *mut u8, eb: u32, t0: u32, t1: u32) {
    let t1 = t1.min(d.cap.saturating_add(1));
    let n = (*totals.add(d.in_slot as usize)).min(d.cap);
    fill_u32(counts, t0.max(n), t1, 0);
    let k = keys.add(eb as usize);
    match held_bits(d, vc) {
        None => {
            for s in t0..t1.min(n) {
                *counts.add(s as usize) = u32::from(s == 0 || *k.add(s as usize) != *k.add(s as usize - 1));
            }
        }
        Some((bits, mask)) => {
            for s in t0..t1.min(n) {
                let key = *k.add(s as usize);
                let x = key & mask;
                let held = (*bits.add((x >> 5) as usize) >> (x & 31)) & 1;
                *counts.add(s as usize) = u32::from((s == 0 || key != *k.add(s as usize - 1)) && held == 0);
            }
        }
    }
}

/// Sorted keys `t0..t1`' rows where they were kept, decoded from them, in their order, and each one's held bit set
/// (`dedup_scatter_sorted_one`).
#[allow(clippy::too_many_arguments)]
unsafe fn dedup_scatter_sorted_range(totals: *mut u32, offsets: *const u32, o: *mut i64, d: &Dedup, vc: *mut u8, p: &HeadKeyP, keys: *const u64, eb: u32, t0: u32, t1: u32) {
    let bits = held_bits(d, vc);
    let o0 = *offsets;
    let n = (*totals.add(d.in_slot as usize)).min(d.cap);
    if t0 == 0 && t1 > 0 {
        *totals.add(d.out_slot as usize) = (*offsets.add(n as usize)).wrapping_sub(o0);
    }
    let k = keys.add(eb as usize);
    for s in t0..t1.min(n) {
        let at = (*offsets.add(s as usize)).wrapping_sub(o0);
        if (*offsets.add(s as usize + 1)).wrapping_sub(o0) == at || at >= d.ocap {
            continue;
        }
        let key = *k.add(s as usize);
        head_row(key, p, o, d.ocap, at);
        if let Some((bits, mask)) = bits {
            let x = key & mask;
            (*(bits.add((x >> 5) as usize) as *const AtomicU32)).fetch_or(1 << (x & 31), Ordering::Relaxed);
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn dedup_scatter_range(t: *const i64, totals: *mut u32, offsets: *const u32, o: *mut i64, r: *const u8, hb: *mut u8, d: &Dedup, x0: u32, x1: u32) {
    let o0 = *offsets;
    // The rows they become (`Dedup::seen`): the relation's rows as the device holds them, and their places among the
    // new ones (`Batch::append_live` writes them there).
    let had = if d.seen == 1 { rows_len(r, d.r_rows_at) } else { 0 };
    let n = (*totals.add(d.in_slot as usize)).min(d.cap);
    if x0 == 0 && x1 > 0 {
        *totals.add(d.out_slot as usize) = (*offsets.add(n as usize)).wrapping_sub(o0);
    }
    kept_runs(offsets, o0, x0, x1.min(n), |x, a, len| {
        let len = below(a, len, d.ocap);
        for c in 0..d.ncols.min(16) {
            copy_col(colp(t, d.cols[c as usize], d.cap, x), colp_mut(o, c, d.ocap, a), len, false);
        }
        if d.seen == 1 {
            dedup_lanes::dedup_place_lanes::<{ stage_lanes::LANES }>(t, hb, d, x, len, had + a);
        }
    });
}

/// A stage's map of its bindings' columns: each bind's column over the range, a document's high words 2 a step.
unsafe fn stage_map_range(s: &Stage, bb: *mut i64, totals: *const u32, t0: u32, t1: u32) {
    let t1 = t1.min((*totals.add(s.in_slot as usize)).min(s.cap_in));
    if t0 >= t1 {
        return;
    }
    let cnt = t1 - t0;
    for b in 0..s.nbinds.min(16) as usize {
        let k = s.bind_idx[b] as usize;
        let dst = colp_mut(bb, s.bind_slot[b], s.cap_in, t0);
        if s.bind_kind[b] == 1 {
            if s.par_kind[k] == 0 {
                let v = s.par_const[k];
                fill_i64(dst, cnt, if s.par_doc[k] != 0 { v >> 32 } else { v });
            } else {
                copy_col(colp(bb, s.par_slot[k], s.cap_in, t0), dst, cnt, s.par_doc[k] != 0);
            }
        } else {
            copy_col(colp(bb, s.col_idx[k], s.cap_in, t0), dst, cnt, s.col_doc[k] != 0);
        }
    }
}

/// A copy of rows: columns to columns a column's copy at a time; through a view or a relation's reads a row at a
/// time.
unsafe fn rows_copy_range(src: *const u8, rf: *const u8, dst: *mut u8, totals: *const u32, c: &Copy, t0: u32, t1: u32) {
    let n = if c.count_slot == NONE { c.n } else { (*totals.add(c.count_slot as usize)).min(c.n) };
    let t1 = t1.min(n);
    if t0 >= t1 {
        return;
    }
    if c.r_view != 0 || c.src_at != 0 || c.dst_rows_at != 0 {
        let mut f = Found::new();
        for t in t0..t1 {
            rows_copy_one(src, far_block(src, rf), dst, c, n, t, &mut f);
        }
        return;
    }
    let base = if c.flags & 1 != 0 { *c.off } else { 0 };
    let grown = if c.flags & 2 != 0 { rows_len(dst, c.dst_rows_at) } else { 0 };
    let a = c.dst_off.wrapping_add(base).wrapping_add(grown).wrapping_add(t0);
    for k in 0..c.ncols.min(16) {
        let d = colp_mut(dst as *mut i64, k, c.dst_stride, a);
        match c.src[k as usize] {
            NONE => fill_i64(d, t1 - t0, c.konst[k as usize]),
            from => copy_col(colp(src as *const i64, from, c.src_stride, t0), d, t1 - t0, false),
        }
    }
}

/// `join_stages`' `ix_key_one` of rows `t0..t1`: each one's key, its place, and its tag at its own place in the index's
/// rows' room (`rows`), which its marks read (`ix_mark_range`) before its rows are placed there.
unsafe fn ix_key_range(bb: *const u8, rf: *const u8, out: *mut u32, places: *mut u32, rows: *mut u32, p: &IxP, kb: u32, eb: u32, t0: u32, t1: u32) {
    let t1 = t1.min(ix_rows(bb, p));
    if t0 >= t1 {
        return;
    }
    // Each row its own place: eb + t, 4 a store.
    let mut v = u32x4::splat(eb.wrapping_add(t0)) + u32x4::from_array([0, 1, 2, 3]);
    let mut t = t0;
    while t + 4 <= t1 {
        st(places.add((eb + t) as usize), v);
        v += u32x4::splat(4);
        t += 4;
    }
    for t in t..t1 {
        *places.add((eb + t) as usize) = eb + t;
    }
    if p.kind == 0 {
        dedup_lanes::ix_keys_lanes::<{ stage_lanes::LANES }>(bb, far_block(bb, rf), out, rows, p, kb, eb, t0, t1);
        return;
    }
    dedup_lanes::ix_keys_view_lanes::<{ stage_lanes::LANES }>(bb, far_block(bb, rf), out, rows, p, kb, eb, t0, t1);
}

/// The index's rows written into its relation's block, from the level's keys sorted (stably) with their places: its
/// sorted place `t` is the level's `eb + t`, its row's id the place it came from less `eb`, past the relation's first
/// row `from` (so a bucket's rows are in id order, as the host's builder places them). After its marks, which read
/// the rows' tags where its keys' pass left them (`ix_mark_range`).
unsafe fn ix_place_range(bb: *const u8, places: *const u32, rows: *mut u32, p: &IxP, eb: u32, t0: u32, t1: u32) {
    let t1 = t1.min(ix_rows(bb, p));
    if t0 >= t1 {
        return;
    }
    // A row's relation row: its place, from `from`, 4 a step.
    let d = u32x4::splat(p.from.wrapping_sub(eb));
    let mut t = t0;
    while t + 4 <= t1 {
        st(rows.add(t as usize), ld::<u32, 4>(places.add((eb + t) as usize)) + d);
        t += 4;
    }
    for t in t..t1 {
        *rows.add(t as usize) = p.from.wrapping_add(*places.add((eb + t) as usize)).wrapping_sub(eb);
    }
}

/// The index's buckets `x0..x1`' first rows (decision 0647), of its `cap + 1` (the last, its rows'): each the first
/// sorted place whose bucket is `x` or past it. One search of the sorted keys finds `x0`'s; the range's buckets are
/// consecutive and the keys sorted by them, so each next bucket's is found walking on from the last. A search for
/// every bucket was a binary search's scattered reads of all the keys a bucket, the join program's largest cost on
/// the CPU device.
unsafe fn ix_offs_range(bb: *const u8, keys: *const u32, offs: *mut u32, p: &IxP, kb: u32, eb: u32, x0: u32, x1: u32) {
    let cap = (p.mask as u32).wrapping_add(1);
    let n = ix_rows(bb, p);
    let sorted = core::slice::from_raw_parts(keys.add(eb as usize), n as usize);
    let end = x1.min(cap.saturating_add(1));
    if x0 >= end {
        return;
    }
    let mut at = sorted.partition_point(|&k| k.wrapping_sub(kb) < x0);
    for x in x0..end {
        while at < sorted.len() && sorted[at].wrapping_sub(kb) < x {
            at += 1;
        }
        *offs.add(x as usize) = at as u32;
    }
}

unsafe fn lat_clear_range(pl: *mut u32, best: *mut u32, flags: *mut u32, p: &LatP, i0: u32, i1: u32) {
    let i1 = i1.min(p.pmask.saturating_add(1));
    fill_u32(pl, i0, i1, NONE);
    fill_u32(best, i0, i1, NONE);
    fill_u32(flags, i0, i1, 0);
}

/// A lattice's key table cleared: each entry (row, changes, level) none, 0, 0, four entries three stores. Code 6: a
/// held bitmap's words, 0.
unsafe fn lat_hclear_range(hb: *mut u8, p: &LatP, i0: u32, i1: u32) {
    // A program's tail's growth of a map that did not grow (`omask` all ones, `Batch::lattice_tail`): nothing.
    if p.omask == NONE {
        return;
    }
    let i1 = i1.min(p.hmask.saturating_add(1));
    let h = hb.add(p.h_at as usize) as *mut u32;
    if p.code == 6 {
        fill_u32(h, i0, i1, 0);
        return;
    }
    let entry = [u32x4::from_array([NONE, 0, 0, NONE]), u32x4::from_array([0, 0, NONE, 0]), u32x4::from_array([0, NONE, 0, 0])];
    let mut i = i0;
    while i + 4 <= i1 {
        let e = h.add(3 * i as usize);
        st(e, entry[0]);
        st(e.add(4), entry[1]);
        st(e.add(8), entry[2]);
        i += 4;
    }
    for i in i..i1 {
        *(h.add(3 * i as usize) as *mut [u32; 3]) = [NONE, 0, 0];
    }
}

// ---- The program's own steps: sums, gates, rounds, sizes. ----

unsafe fn sum_one(p: *const SumP) {
    let parts = core::slice::from_raw_parts((*p).part.as_ptr(), (*p).m as usize);
    let mut a = 0u32;
    for part in parts {
        *part.off = a;
        a = a.wrapping_add(*part.src);
    }
    *(*p).total = a;
}

unsafe fn gate_one(p: *const GateP, ctl: *mut u32, t: u32) {
    if t >= (*p).m {
        return;
    }
    let c = &*(*p).check.as_ptr().add(t as usize);
    let n = *c.count as u64 + if c.has & 1 != 0 { *c.plus as u64 } else { 0 };
    if n > c.cap as u64 {
        at(ctl.add(6)).fetch_or(if c.has & 2 != 0 { 128 } else { 2 }, SeqCst);
    }
}

unsafe fn round_one(p: *const RoundP, ctl: *mut u32) {
    let ops = core::slice::from_raw_parts((*p).ops.as_ptr(), (*p).nops as usize);
    let any = ops.iter().any(|o| o.op == 4 && *o.from != 0);
    // The round's work: the rows its stages made (op 6).
    let work: u64 = ops.iter().filter(|o| o.op == 6).map(|o| *o.from as u64).sum();
    let cur = *(*p).round;
    for o in ops {
        match o.op {
            0 => *o.to = (*o.to).wrapping_add(*o.from),
            1 => *o.to = *o.from,
            2 => core::ptr::write_bytes(o.to, 0, o.n as usize),
            3 if cur < o.n => *o.to.add(cur as usize) = *o.from,
            5 => {
                for k in 0..o.n as usize {
                    *o.to.add(k) = (*o.to.add(k)).wrapping_add(*o.from.add(k));
                }
            }
            _ => {}
        }
    }
    *(*p).round = cur.wrapping_add(1);
    // No new rows: the program ends; past its rounds: it stops; less work than `narrow`: the CPU goes on from its
    // rows (decision 0657). Each ends the walk (the jump's end); else the round after, from `next` (`ctl[7]`).
    let (ctl6, ctl7) = (at(ctl.add(6)), at(ctl.add(7)));
    if !any {
        ctl6.fetch_or(16, SeqCst);
        ctl7.store(NONE, SeqCst);
    } else if cur.wrapping_add(1) > (*p).max {
        ctl6.fetch_or(4, SeqCst);
        ctl7.store(NONE, SeqCst);
    } else if work < (*p).narrow as u64 {
        ctl6.fetch_or(8, SeqCst);
        ctl7.store(NONE, SeqCst);
    } else {
        ctl7.store((*p).next + 1, SeqCst);
    }
}

/// A level's size script (`Item::group` ops): false where it parks, its claims past its arena's room.
unsafe fn size_one(ops: *const SizeOp, n: u32) -> bool {
    let (mut full, mut mark, mut want) = (false, 0u32, 0u64);
    let mut top: *mut u32 = core::ptr::null_mut();
    let mut r = [0u64; 16];
    for i in 0..n as usize {
        let o = &*ops.add(i);
        let (k, x, y) = ((o.k & 15) as usize, (o.x & 15) as usize, (o.y & 15) as usize);
        match o.op {
            0 => r[k] = (*o.w as u64).wrapping_add(o.c),
            2 => r[k] = o.c,
            3 => r[k] = r[x].wrapping_add(r[y]).wrapping_add(o.c),
            4 => r[k] = r[x].saturating_sub(r[y]),
            5 => r[k] = r[x].wrapping_mul(o.c),
            6 => r[k] = (r[x].wrapping_add(o.c).wrapping_sub(1)) / o.c,
            7 => r[k] = r[x].min(r[y]),
            8 => r[k] = r[x].max(o.c),
            9 => {
                let got = r[x].wrapping_add(15) / 16;
                let got = got.min(NONE as u64) as u32;
                if full {
                    want += got as u64;
                    r[k] = o.c;
                    continue;
                }
                let old = at(o.w).fetch_add(got, SeqCst);
                if top.is_null() {
                    top = o.w;
                    mark = old;
                }
                if old as u64 + got as u64 > at(o.w.add(1)).load(SeqCst) as u64 {
                    full = true;
                    want = old as u64 + got as u64;
                    at(o.w).store(mark, SeqCst);
                }
                r[k] = if full { o.c } else { o.c.wrapping_add(16 * old as u64) };
            }
            11 => *o.w = r[k] as u32,
            12 => r[k] = *o.w.add(r[x] as usize) as u64,
            13 => {
                let v = r[x].max(o.c);
                let mut q = 1u64;
                for _ in 0..40 {
                    if q >= v {
                        break;
                    }
                    q <<= 1;
                }
                r[k] = q;
            }
            _ => {}
        }
    }
    if full {
        at(top.add(2)).store(want.min(NONE as u64) as u32, SeqCst);
    }
    !full
}

// ---- Scans and sorts: a tile at a time, its values 4 a register. ----

/// The sum of `n` words at `p`, 16 a step.
unsafe fn sum_u32(p: *const u32, n: u32) -> u32 {
    let mut acc = u32x16::splat(0);
    let mut i = 0;
    while i + 16 <= n {
        acc += ld(p.add(i as usize));
        i += 16;
    }
    let total = acc.reduce_sum();
    (i..n).fold(total, |s, k| s.wrapping_add(*p.add(k as usize)))
}

/// `n` words at `p` replaced by their running sum before each, from `base`; the sum after them. A register's four
/// sums by two shifted adds, its last carried into the next register.
unsafe fn scan_u32(p: *mut u32, n: u32, base: u32) -> u32 {
    let mut carry = u32x4::splat(base);
    let mut i = 0;
    while i + 4 <= n {
        let v = ld::<u32, 4>(p.add(i as usize));
        let one = v + v.shift_elements_right::<1>(0);
        let all = one + one.shift_elements_right::<2>(0);
        st(p.add(i as usize), carry + (all - v));
        carry = u32x4::splat((carry + all)[3]);
        i += 4;
    }
    let mut run = carry[0];
    for k in i..n {
        let v = *p.add(k as usize);
        *p.add(k as usize) = run;
        run = run.wrapping_add(v);
    }
    run
}

/// Tile `tile`'s sum of `n` values into `sums[tile]`.
unsafe fn scan_tile_sum(input: *const u32, sums: *mut u32, n: u32, tile: u32) {
    let (first, end) = scan_tile_range(n, tile);
    *sums.add(tile as usize) = sum_u32(input.add(first as usize), end - first);
}

/// Tile `tile` scanned in place from `base`; the last tile writes the total at `n`.
unsafe fn scan_tile_from(io: *mut u32, base: u32, n: u32, tile: u32) {
    let (first, end) = scan_tile_range(n, tile);
    let run = scan_u32(io.add(first as usize), end - first, base);
    let last = if n == 0 { 0 } else { (n - 1) / (TG * SCAN_ITEMS) };
    if tile == last {
        *io.add(n as usize) = run;
    }
}

fn scan_tile_range(n: u32, tile: u32) -> (u32, u32) {
    let size = TG * SCAN_ITEMS;
    let first = (tile as u64 * size as u64).min(n as u64) as u32;
    (first, (first as u64 + size as u64).min(n as u64) as u32)
}

/// A tile's rows [first, end) and each one's digit, four keys a shift and a mask.
unsafe fn digits(keys: *const u32, first: u32, end: u32, shift: u32, mut f: impl FnMut(u32, usize)) {
    let (sh, mask) = (u32x4::splat(shift), u32x4::splat(255));
    let mut r = first;
    while r + 4 <= end {
        let d = ((ld::<u32, 4>(keys.add(r as usize)) >> sh) & mask).to_array();
        for (k, &x) in d.iter().enumerate() {
            f(r + k as u32, x as usize);
        }
        r += 4;
    }
    for r in r..end {
        f(r, ((*keys.add(r as usize) >> shift) & 255) as usize);
    }
}

/// Tile `tile`'s digit histogram, digit-major (`hist[d * tiles + tile]`): four histograms, a row's by its place of
/// four (no two adds in a row wait on one count), summed 4 digits a step.
unsafe fn radix_hist_tile(keys: *const u32, hist: *mut u32, n: u32, shift: u32, tiles: u32, tile: u32) {
    let mut h = [[0u32; RADIX_DIGITS]; 4];
    let first = tile.saturating_mul(RADIX_TILE);
    digits(keys, first, first.saturating_add(RADIX_TILE).min(n), shift, |r, d| h[(r & 3) as usize][d] += 1);
    for d in (0..RADIX_DIGITS).step_by(4) {
        let at = |k: usize| u32x4::from_slice(&h[k][d..d + 4]);
        let sum = ((at(0) + at(1)) + (at(2) + at(3))).to_array();
        for (k, &c) in sum.iter().enumerate() {
            *hist.add((d + k) * tiles as usize + tile as usize) = c;
        }
    }
}

/// The wide sort's (64-bit keys, `SortWords::wide`) tile histogram, as `radix_hist_tile`'s.
unsafe fn radix_hist_tile64(keys: *const u64, hist: *mut u32, n: u32, shift: u32, tiles: u32, tile: u32) {
    // Four tables, a key each in turn: consecutive keys of one digit do not wait on one counter.
    let mut h = [[0u32; RADIX_DIGITS]; 4];
    let first = tile.saturating_mul(RADIX_TILE);
    let end = first.saturating_add(RADIX_TILE).min(n);
    let dg = |r: u32| ((*keys.add(r as usize) >> shift) & 255) as usize;
    let mut r = first;
    while r + 4 <= end {
        h[0][dg(r)] += 1;
        h[1][dg(r + 1)] += 1;
        h[2][dg(r + 2)] += 1;
        h[3][dg(r + 3)] += 1;
        r += 4;
    }
    while r < end {
        h[0][dg(r)] += 1;
        r += 1;
    }
    let [h0, h1, h2, h3] = &h;
    for (d, (((a, b), c), e)) in h0.iter().zip(h1).zip(h2).zip(h3).enumerate() {
        *hist.add(d * tiles as usize + tile as usize) = a + b + c + e;
    }
}

/// The wide sort's tile's rows at their places, as `radix_scatter_tile`'s.
unsafe fn radix_scatter_tile64(wa: WideAt, from: usize, n: u32, shift: u32, tiles: u32, tile: u32) {
    let (keys, out_keys) = (wa.keys[from] as *const u64, wa.keys[1 - from]);
    let mut next = [0u32; RADIX_DIGITS];
    for (d, x) in next.iter_mut().enumerate() {
        *x = *wa.hist.add(d * tiles as usize + tile as usize);
    }
    let first = tile.saturating_mul(RADIX_TILE);
    for r in first..first.saturating_add(RADIX_TILE).min(n) {
        let k = *keys.add(r as usize);
        let d = ((k >> shift) & 255) as usize;
        let a = next[d] as usize;
        next[d] += 1;
        *out_keys.add(a) = k;
    }
}

/// Tile `tile`'s rows at their places: each digit's from its scanned offset, in the rows' order (a stable sort).
unsafe fn radix_scatter_tile(sp: &SortPass, n: u32, shift: u32, tiles: u32, tile: u32) {
    let mut next = [0u32; RADIX_DIGITS];
    for (d, x) in next.iter_mut().enumerate() {
        *x = *sp.hist.add(d * tiles as usize + tile as usize);
    }
    let first = tile.saturating_mul(RADIX_TILE);
    digits(sp.keys, first, first.saturating_add(RADIX_TILE).min(n), shift, |r, d| {
        let a = next[d] as usize;
        next[d] += 1;
        *sp.out_keys.add(a) = *sp.keys.add(r as usize);
        *sp.out_values.add(a) = *sp.values.add(r as usize);
    });
}

// ---- Text rows: a SIMD group's on a GPU, a thread's here. ----

/// Mode 10's rows of SIMD group `sg` (`row_text_one`): each row's keep bit and made text (`counts`, `V`).
unsafe fn row_text(st: &St, j: &Job, b: &Bufs, ctl: *mut u32, sg: u32) {
    let s = st.s;
    let (counts, vv, p) = (b.c, b.v, j.p);
    let pw = |i: usize| *p.add(i);
    // P[6]: the arena keeps its texts one value a content, in table `b.h` (`canon_text`, decision 0659).
    let (canon, table) = (pw(6) != 0, b.h);
    let n = (*j.totals.add(s.in_slot as usize)).min(s.cap_in);
    let groups = pw(0);
    // The count past its rows (the scan's last, its total's): its counts are its rows' and one (`level_sizes`).
    if sg == 0 {
        *counts.add(n as usize) = 0;
    }
    if sg >= groups {
        return;
    }
    let (a, d, g, vc) = (j.a, j.d, j.g, j.vc);
    let op = s.agg[0] & 15;
    let want = (s.agg[0] >> 8) & 1 != 0;
    // A test's automaton, its byte classes in registers for every row of the group's.
    let dfa = (op == 3 || op == 5).then(|| Automaton::new(g, pw(2), pw(3), pw(4)));
    let counter = |bits: u32| {
        if s.counter != NONE {
            at(j.counters.add(s.counter as usize)).fetch_or(bits, SeqCst);
        }
    };
    if op == 5 {
        // A symbol's test, its verdict kept in the rolling cache: looked up, else walked and kept.
        let test = pw(5);
        let mut base = sg * SGW;
        while base < n {
            let mut unheld = false;
            for row in base..(base + SGW).min(n) {
                let v = st.key_value(row, 0);
                let mut got = 1;
                if (0..=u32::MAX as i64).contains(&v) {
                    got = verdict_get(vc, 0, test, v as u32);
                    if got == 0 {
                        let id = v as u32;
                        let len = atoms_len(d, 0, id);
                        got = if len == NONE {
                            3
                        } else {
                            let x = Txt::raw(atoms_at(d, 0, id) as *const u8, len);
                            let verdict = if dfa.as_ref().is_some_and(|a| a.verdict(&x)) { 2 } else { 1 };
                            verdict_put(vc, 0, test, id, verdict);
                            verdict
                        };
                    }
                }
                *counts.add(row as usize) = ((got == 2) == want) as u32;
                unheld |= got == 3;
            }
            if unheld {
                counter(4);
            }
            base += groups * SGW;
        }
        return;
    }
    // The regex group's thread lists (`rx_group_range`), grown to its program's states.
    let mut rx_scratch = Vec::new();
    // A node's text (`TEXTFN_NODE_TEXT_ANY`, `TEXTFN_NODE_TEXT`): X is key 5's node's, read where its source lies.
    let fnk = (s.agg[0] >> 16) & 255;
    let node_text = op == 4 && (fnk == 15 || fnk == 16);
    let mut row = sg;
    while row < n {
        let r = row;
        row += groups;
        let mut unheld = false;
        let x = if node_text {
            match node_txt(s, st.key_value(r, 5), fnk == 15) {
                Ok(x) => x,
                Err(NoText::Unheld) => {
                    unheld = true;
                    Txt::raw(d, 0)
                }
                Err(NoText::Line) => {
                    *vv.add(r as usize) = -1;
                    *counts.add(r as usize) = 0;
                    continue;
                }
            }
        } else {
            st.txt_of(r, d, a, 0, s.agg[1] & 15, &mut unheld)
        };
        if unheld {
            counter(4);
            *vv.add(r as usize) = -1;
            *counts.add(r as usize) = 0;
            continue;
        }
        if op == 3 {
            *counts.add(r as usize) = (dfa.as_ref().is_some_and(|a| a.verdict(&x)) == want) as u32;
            continue;
        }
        // A text kept by (functions P[5], symbol key 7) in the rolling cache: copied into the arena.
        let ksym = st.key_value(r, 7);
        let keeps = s.agg[0] & (1 << 9) != 0 && (0..=u32::MAX as i64).contains(&ksym);
        // `ROW_FIND` (bit 10): the made text's value is the atom it is, found in the atoms directory (`atom_of_text`,
        // decision 0666), or no row where none is.
        let finds = s.agg[0] & (1 << 10) != 0;
        // `ROW_FIND_NONE` (bit 11): a text no atom is keeps its row, its value -1 (a negated atom's key: no row has it).
        let keep_none = s.agg[0] & (1 << 11) != 0;
        let ksym = ksym as u32;
        if keeps {
            let e = text_find(vc, 0, pw(5), ksym);
            if e & 3 == 0 {
                let e = e as *const u8;
                let len = (e.add(12) as *const u32).read_unaligned().wrapping_sub(16);
                let (off, full) = arena_claim(a, len, canon);
                if !full {
                    core::ptr::copy_nonoverlapping(e.add(16), (a as *mut u8).add(16 + off as usize), len as usize);
                }
                let value = if !full && finds { atom_of_text(a, off, len, d) } else { text_value(a, table, ctl, canon, off, len, full) };
                made(ctl, vv, counts, r, value, full, keep_none);
                continue;
            }
        }
        let y = if node_text { x } else { st.txt_of(r, d, a, 2, (s.agg[1] >> 4) & 15, &mut unheld) };
        let f = (s.agg[0] >> 16) & 255;
        let lx = x.len();
        // How the text is made, and its length.
        enum Make {
            Cat(u32, Txt),
            Rx(Txt),
            Digits { neg: bool, u: u64, ndig: u32, pad: u32 },
            Lit(Txt, Txt),
            Range(u32),
        }
        let (make, len) = if op <= 1 {
            let dot = (op == 0 && lx > 0) as u32;
            (Make::Cat(dot, y), lx + dot + y.len())
        } else if op == 2 {
            (Make::Rx(y), rx_replace_txt(g, &x, &y, None))
        } else if f == 5 || f == 11 {
            let n0 = st.key_value(r, 5);
            let neg = n0 < 0;
            let u = n0.unsigned_abs();
            let ndig = (1..20).find(|&k| u < pow10_of(k)).unwrap_or(20);
            let mut len = ndig + neg as u32;
            let mut pad = 0;
            if f == 11 {
                let w = st.key_value(r, 6).clamp(0, 64) as u32;
                pad = w.saturating_sub(len);
                len += pad;
            }
            (Make::Digits { neg, u, ndig, pad }, len)
        } else if f == 12 {
            // str_between(x, y, z): x less prefix y (keys 2, 3) and suffix z (keys 4, 5), each of its kind (agg[1] >> 4
            // and >> 8: a symbol, a held cat's parts, an arena text); no row where x has not both.
            let y = st.txt_of(r, d, a, 2, (s.agg[1] >> 4) & 15, &mut unheld);
            let z = st.txt_of(r, d, a, 4, (s.agg[1] >> 8) & 15, &mut unheld);
            let (ly, lz) = (y.len(), z.len());
            if lx < ly + lz || !x.same(0, &y, 0, ly) || !x.same(lx - lz, &z, 0, lz) {
                *vv.add(r as usize) = -1;
                *counts.add(r as usize) = 0;
                continue;
            }
            (Make::Range(ly), lx - lz - ly)
        } else if f == 13 {
            // pkg_up(x, k): x to its k-th '.' from its end; empty where it has fewer.
            let k = st.key_value(r, 5);
            let mut cut = lx;
            if k > 0 {
                let mut seen = 0i64;
                cut = 0;
                for i in (1..=lx).rev() {
                    if x.byte(i - 1) == b'.' {
                        seen += 1;
                        if seen == k {
                            cut = i - 1;
                            break;
                        }
                    }
                }
            }
            (Make::Range(0), cut)
        } else if f == 14 {
            // rx_group(x, re, name): the group's range of x by its program (`rx_group_range`); no row where the regex
            // does not match x or the group took no part, as on the CPU.
            let Some((from, to)) = rx_group_range(g, &x, &mut rx_scratch) else {
                *vv.add(r as usize) = -1;
                *counts.add(r as usize) = 0;
                continue;
            };
            (Make::Range(from), to - from)
        } else if f == 15 || f == 16 {
            // A node's text whole; `rr_node_text`'s only past `text`'s inline bound (`core_rr::TEXT_INLINE`, 256 bytes):
            // no row for a shorter one, as the host's `long_text` has none.
            if f == 16 && lx <= 256 {
                *vv.add(r as usize) = -1;
                *counts.add(r as usize) = 0;
                continue;
            }
            (Make::Range(0), lx)
        } else if (17..=21).contains(&f) {
            // The trims: spaces (17 both ends, 18 its start) or the characters of y (19 both ends, 20 its start, 21 its
            // end), a byte at a time; a set past ASCII is the host's (its characters, not its bytes).
            if f >= 19 && (0..y.len()).any(|i| y.byte(i) >= 0x80) {
                counter(2);
                *vv.add(r as usize) = -1;
                *counts.add(r as usize) = 1;
                continue;
            }
            let (mut a0, mut b0) = (0, lx);
            if f != 21 {
                while a0 < b0 && trim_has(&y, f < 19, x.byte(a0)) {
                    a0 += 1;
                }
            }
            if matches!(f, 17 | 19 | 21) {
                while b0 > a0 && trim_has(&y, f < 19, x.byte(b0 - 1)) {
                    b0 -= 1;
                }
            }
            (Make::Range(a0), b0 - a0)
        } else if f == 6 {
            let y = st.txt_of(r, d, a, 2, 0, &mut unheld);
            let z = st.txt_of(r, d, a, 4, 0, &mut unheld);
            let len = lit_replace_txt(&x, &y, &z, None);
            (Make::Lit(y, z), len)
        } else {
            let (from, to, high) = text_range(f, &x, st.key_value(r, 5), st.key_value(r, 6));
            if f == 0 && high {
                // `lower` past ASCII: Unicode's mapping is the host's.
                counter(2);
                *vv.add(r as usize) = -1;
                *counts.add(r as usize) = 1;
                continue;
            }
            (Make::Range(from), to.wrapping_sub(from))
        };
        let (off, full) = arena_claim(a, len, canon);
        let o = (a as *mut u8).add(16 + off as usize);
        if !full {
            match make {
                Make::Cat(dot, y) => {
                    x.write(0, lx, o, 0);
                    if dot != 0 {
                        *o.add(lx as usize) = b'.';
                    }
                    y.write(0, y.len(), o.add((lx + dot) as usize), 0);
                }
                Make::Rx(y) => {
                    rx_replace_txt(g, &x, &y, Some(o));
                }
                Make::Lit(y, z) => {
                    lit_replace_txt(&x, &y, &z, Some(o));
                }
                Make::Digits { neg, u, ndig, pad } => {
                    let sign = neg as u32;
                    for i in 0..len {
                        let c = if neg && i == 0 {
                            b'-'
                        } else if i < sign + pad {
                            b'0'
                        } else {
                            b'0' + ((u / pow10_of(ndig - 1 - (i - sign - pad))) % 10) as u8
                        };
                        *o.add(i as usize) = c;
                    }
                }
                Make::Range(from) => x.write(
                    from,
                    from + len,
                    o,
                    match f {
                        0 | 8 => 1,
                        9 => 2,
                        _ => 0,
                    },
                ),
            }
        }
        if keeps && !full && !unheld && len <= 4096 {
            // Made: kept for the next reader, its entry completed after its bytes.
            let e = text_open(vc, 0, pw(5), ksym, len);
            if e & 3 == 1 {
                let e = (e & !3) as *mut u8;
                core::ptr::copy_nonoverlapping(o, e.add(16), len as usize);
                cache_done(vc, 0, e as u64);
            }
        }
        let value = if !full && finds { atom_of_text(a, off, len, d) } else { text_value(a, table, ctl, canon, off, len, full) };
        if unheld {
            counter(4);
        }
        made(ctl, vv, counts, r, value, full, keep_none);
    }
}

/// A string join's order of rows `a` and `b` (`strjoin_cmp`): by its `by` columns (`col_idx[1..ncols]`), then x
/// (`col_idx[0]`), each a number by value or, where `agg[1]` has its bit, a symbol by its name's bytes (as the host's
/// `Sortable`). A symbol not held: `unheld`.
unsafe fn strjoin_cmp(st: &St, d: *const u8, a: u32, b: u32, unheld: &mut bool) -> core::cmp::Ordering {
    use core::cmp::Ordering::*;
    let s = st.s;
    let nc = s.ncols;
    for i in 1..=nc {
        let c = if i < nc { i } else { 0 };
        let (va, vb) = (st.rget(a, s.col_idx[c as usize]), st.rget(b, s.col_idx[c as usize]));
        if va == vb {
            continue;
        }
        if (s.agg[1] >> c) & 1 == 0 {
            return va.cmp(&vb);
        }
        let ((ta, la), (tb, lb)) = (sym_text(d, va, unheld), sym_text(d, vb, unheld));
        let o = core::slice::from_raw_parts(ta, la as usize).cmp(core::slice::from_raw_parts(tb, lb as usize));
        return if o == Equal { va.cmp(&vb) } else { o };
    }
    Equal
}

/// A string join's rows of SIMD group `sg` (`ROW_STRJOIN`, decision 0661, `row_strjoin`): each binding's matching
/// rows (its candidates, as an aggregate's), their distinct (by..., x) ordered by `strjoin_cmp`, their x's names joined
/// by the separator's (`par_const[0]`) into the arena; "" for none. More matches than a SIMD group's lanes: the
/// counter's bit 2, and the stratum runs on the host.
unsafe fn row_strjoin(st: &St, j: &Job, b: &Bufs, ctl: *mut u32, sg: u32) {
    let s = st.s;
    let (counts, vv, p, a, d) = (b.c, b.v, j.p, j.a, j.d);
    let pw = |i: usize| *p.add(i);
    let n = (*j.totals.add(s.in_slot as usize)).min(s.cap_in);
    let groups = pw(0);
    let (canon, table) = (pw(6) != 0, b.h);
    for lane in 0..SGW {
        let mut r = n.wrapping_add(sg * SGW + lane);
        while r <= s.cap_in {
            *counts.add(r as usize) = 0;
            r = r.wrapping_add(groups * SGW);
        }
    }
    if sg >= groups {
        return;
    }
    let mut unheld = false;
    let (sep, lsep) = sym_text(d, s.par_const[0], &mut unheld);
    let mut row = sg;
    while row < n {
        let r = row;
        row += groups;
        let mut kv = [0i64; 8];
        let w = st.candidates(r, j.f, &mut kv);
        // The matches, a lane's each on a GPU: past its lanes, `over`.
        let mut m = [0u32; SGW as usize];
        let (mut k, mut over) = (0usize, false);
        st.each(w, &kv, |q, _, key| {
            if !st.matches(q, &kv, key) {
                return true;
            }
            if k == SGW as usize {
                over = true;
                return false;
            }
            m[k] = q;
            k += 1;
            true
        });
        let m = &mut m[..k];
        let mut cu = false;
        m.sort_unstable_by(|&a, &b| strjoin_cmp(st, d, a, b, &mut cu));
        // The kept names (equal rows once), each its bytes; whether each is held.
        let mut names: [(*const u8, u32); SGW as usize] = [(d, 0); SGW as usize];
        let (mut kept, mut held, mut first_held) = (0usize, !cu, !cu);
        for i in 0..m.len() {
            if i > 0 && strjoin_cmp(st, d, m[i - 1], m[i], &mut cu) == core::cmp::Ordering::Equal {
                continue;
            }
            let mut u = false;
            names[kept] = sym_text(d, st.rget(m[i], s.col_idx[0]), &mut u);
            held &= !u;
            if kept == 0 {
                first_held &= !u;
            }
            kept += 1;
        }
        if over || !held || unheld {
            // Its counter's bit as the group's first lane finds it: 2 where its own name and the separator are held.
            if s.counter != NONE {
                at(j.counters.add(s.counter as usize)).fetch_or(if first_held && !unheld { 2 } else { 4 }, SeqCst);
            }
            *vv.add(r as usize) = -1;
            *counts.add(r as usize) = 1;
            continue;
        }
        let names = &names[..kept];
        let total = names.iter().map(|&(_, l)| l).sum::<u32>() + if kept > 0 { lsep * (kept as u32 - 1) } else { 0 };
        let (off, full) = arena_claim(a, total, canon);
        if !full {
            let mut o = (a as *mut u8).add(16 + off as usize);
            for (i, &(x, l)) in names.iter().enumerate() {
                core::ptr::copy_nonoverlapping(x, o, l as usize);
                o = o.add(l as usize);
                if i + 1 < kept {
                    core::ptr::copy_nonoverlapping(sep, o, lsep as usize);
                    o = o.add(lsep as usize);
                }
            }
        }
        made(ctl, vv, counts, r, text_value(a, table, ctl, canon, off, total, full), full, false);
    }
}

/// A text's room of `len` bytes in the text arena: its offset, and whether it is past the arena's room. In a canonical
/// arena (`canon`) its length before its bytes, four bytes little-endian.
unsafe fn arena_claim(a: *mut u32, len: u32, canon: bool) -> (u32, bool) {
    let room = len + if canon { 4 } else { 0 };
    let off = at(a).fetch_add(room, SeqCst);
    let full = off as u64 + room as u64 > at(a.add(1)).load(SeqCst) as u64;
    if canon && !full {
        core::ptr::copy_nonoverlapping(len.to_le_bytes().as_ptr(), (a as *mut u8).add(16 + off as usize), 4);
    }
    (off + if canon { 4 } else { 0 }, full)
}

/// A canonical arena's probes a text at most before its table is called full (`Arena::grow` places it again).
const CANON_PROBES: u32 = 64;

/// The made text at `off` of `len` bytes, kept one value a content (`Canon`, decision 0659): the value of the one in
/// table `t` (header [places - 1, texts, full, 0], places after) whose bytes are its, else its own, put there. A table
/// with no place for it in `CANON_PROBES`: its full word, the program's flag 64 and -1, and its phase parks until the
/// host grows it.
unsafe fn canon_text(a: *mut u32, t: *mut u32, off: u32, len: u32, ctl: *mut u32) -> i64 {
    let mine = core::slice::from_raw_parts(arena_bytes(a).add(off as usize), len as usize);
    let h = mine.iter().fold(0x811c_9dc5u32, |h, &b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    let mask = *t;
    let mut p = 0;
    while p < CANON_PROBES && p <= mask {
        let i = h.wrapping_add(p) & mask;
        p += 1;
        let e = match at(t.add(4 + i as usize)).compare_exchange(0, off + 1, SeqCst, SeqCst) {
            Ok(_) => {
                at(t.add(1)).fetch_add(1, SeqCst);
                return arena_value(off, len);
            }
            Err(e) => e,
        };
        let theirs = arena_bytes(a).add(e as usize - 1);
        let elen = u32::from_le_bytes(core::ptr::read_unaligned(theirs.sub(4) as *const [u8; 4]));
        if elen == len && core::slice::from_raw_parts(theirs, len as usize) == mine {
            return arena_value(e - 1, len);
        }
    }
    at(t.add(2)).fetch_or(1, SeqCst);
    at(ctl.add(6)).fetch_or(64, SeqCst);
    -1
}

/// A made text's value: none where the arena was full (the phase runs again), its canonical one in a canonical
/// arena (`canon_text`, table `t`), else its own.
unsafe fn text_value(a: *mut u32, t: *mut u32, ctl: *mut u32, canon: bool, off: u32, len: u32, full: bool) -> i64 {
    if full {
        -1
    } else if canon {
        canon_text(a, t, off, len, ctl)
    } else {
        arena_value(off, len)
    }
}

/// A made text's row: its value (`text_value`) and its keep bit: none for a text no atom is (`ROW_FIND`), as the CPU
/// drops it at the function (`PROBE_OUT`), unless `keep_none` (`ROW_FIND_NONE`).
unsafe fn made(ctl: *mut u32, vv: *mut i64, counts: *mut u32, r: u32, value: i64, full: bool, keep_none: bool) {
    if full {
        at(ctl.add(6)).fetch_or(64, SeqCst);
    }
    *vv.add(r as usize) = value;
    *counts.add(r as usize) = (full || keep_none || value != -1) as u32;
}

// ---- The program: a chain of dispatches, a phase a dispatch, a group an item (decision 0674). ----

/// A phase's groups at most (`PHASE_GROUPS`, decision 0637): a segment's share of them in a phase of several
/// (`seg_cap`), past which a group strides its job's rows.
const PHASE_GROUPS: u32 = 1 << 12;
/// A segment whose count of none starts no group (decision 0647): its `per`'s top bit.
const SEG_EMPTY: u32 = 1 << 31;

fn seg_cap(ns: u32) -> u32 {
    PHASE_GROUPS / ns.max(1)
}

/// A segment's groups (decisions 0603 step 5, 0637): as many as its count on the device needs, `ceil((*count + plus)
/// / per)`, at most `cap`; all its items where `per` is 0 (a job of no count).
unsafe fn seg_groups(g: *const Seg, cap: u32) -> u32 {
    let g = &*g;
    let per = g.per & !SEG_EMPTY;
    if per == 0 {
        return g.items;
    }
    let need = (*g.count as u64 + g.plus as u64).div_ceil(per as u64);
    need.min(cap as u64) as u32
}

struct Prog {
    jobs: *mut Job,
    items: *const Item,
    phases: *const u32,
    segs: *const Seg,
    scans: *mut ScanLevel,
    sorts: *const SortPass,
    script: *const SizeOp,
    ctl: *mut u32,
    counts: *mut u32,
    a: ProgP,
    /// Its items' time by kind (`join_run_timed`, the join lab's), none for the kernel's own run.
    clock: *mut ItemClock,
    /// The classes its stages' jobs run by where their forms are theirs (`join_run_classed`), none: the program's work.
    classes: *const classes::Classes,
}

/// A replay's items by kind (`join_run_timed`, the join lab's profile): each kind's items run and their nanoseconds.
/// Indexed by the item's kind (`Item::what`'s top byte, `PK_*`), named by `item_kind_name`.
#[derive(Clone, Debug)]
pub struct ItemClock {
    pub nanos: [u64; 64],
    pub items: [u64; 64],
}

impl Default for ItemClock {
    fn default() -> ItemClock {
        ItemClock { nanos: [0; 64], items: [0; 64] }
    }
}

/// An item kind's name (`PK_*`, lower case), `?` for one the program has not.
pub fn item_kind_name(kind: usize) -> &'static str {
    match kind as u32 {
        PK_COPY => "copy",
        PK_IX_KEY => "ix_key",
        PK_RADIX_HIST => "radix_hist",
        PK_RADIX_SCATTER => "radix_scatter",
        PK_IX_PLACE => "ix_place",
        PK_IX_OFFS => "ix_offs",
        PK_IX_MARK => "ix_mark",
        PK_DEDUP_KEY => "dedup_key",
        PK_SET_PLACE => "set_place",
        PK_DEDUP_CLEAR => "dedup_clear",
        PK_DEDUP_CLAIM => "dedup_claim",
        PK_MAP => "map",
        PK_STAGE_COUNT => "stage_count",
        PK_SCAN_COUNT => "scan_count",
        PK_WITNESS_COUNT => "witness_count",
        PK_ROW_TEXT => "row_text",
        PK_DEDUP_MARK => "dedup_mark",
        PK_SCAN_SUMS => "scan_sums",
        PK_SCAN_TILES => "scan_tiles",
        PK_STAGE_SCATTER => "stage_scatter",
        PK_SCAN_SCATTER => "scan_scatter",
        PK_ROW_SCATTER => "row_scatter",
        PK_DEDUP_SCATTER => "dedup_scatter",
        PK_LAT_HCLEAR => "lat_hclear",
        PK_LAT_REHASH => "lat_rehash",
        PK_LAT_CLEAR => "lat_clear",
        PK_LAT_CLAIM => "lat_claim",
        PK_LAT_BEST => "lat_best",
        PK_LAT_MERGE => "lat_merge",
        PK_SUM => "sum",
        PK_GATE => "gate",
        PK_ROUND => "round",
        PK_SIZE => "size",
        _ => "?",
    }
}

impl Prog {
    /// `item`, its time added to its kind's where the run keeps a clock (`join_run_timed`).
    unsafe fn timed(&self, it: Item) -> bool {
        if self.clock.is_null() {
            return self.item(it);
        }
        let t = std::time::Instant::now();
        let whole = self.item(it);
        let c = &mut *self.clock;
        let k = (it.what >> 24) as usize & 63;
        c.nanos[k] += t.elapsed().as_nanos() as u64;
        c.items[k] += 1;
        whole
    }

    unsafe fn ctl(&self, i: usize) -> &AtomicU32 {
        at(self.ctl.add(i))
    }

    unsafe fn phase(&self, q: u32) -> [u32; 4] {
        core::array::from_fn(|k| *self.phases.add(4 * q as usize + k))
    }

    /// Phase `q` about to run (`phase_reset`): its count emptied, its groups as its segments' counts need them, at
    /// least one unless every segment's `per` has `SEG_EMPTY`. Whether none needs any where one still runs (1).
    unsafe fn phase_reset(&self, q: u32) -> u32 {
        let [_, _, s0, ns] = self.phase(q);
        let cap = seg_cap(ns);
        let m = (0..ns).fold(0u32, |m, g| m.wrapping_add(seg_groups(self.segs.add((s0 + g) as usize), cap)));
        let empty = ns != 0 && (0..ns).all(|g| (*self.segs.add((s0 + g) as usize)).per & SEG_EMPTY != 0);
        let target = if ns == 0 {
            0
        } else if empty {
            m
        } else {
            m.max(1)
        };
        self.ctl(10 + (self.a.phases + q) as usize).store(target, SeqCst);
        self.ctl(10 + q as usize).store(0, SeqCst);
        (ns != 0 && m == 0 && !empty) as u32
    }

    /// Whether phase `q`'s groups each run at most one stride of their rows (`phase_light`): no segment's groups past
    /// its cap.
    unsafe fn phase_light(&self, q: u32) -> bool {
        let [_, _, s0, ns] = self.phase(q);
        let cap = seg_cap(ns);
        (0..ns).all(|s| {
            let g = &*self.segs.add((s0 + s) as usize);
            let per = g.per & !SEG_EMPTY;
            per == 0 || (*g.count as u64 + g.plus as u64).div_ceil(per as u64) <= cap as u64
        })
    }

    /// Phase `q`'s groups, a group an item, once reset: its items, or its segments' (its `target`).
    unsafe fn phase_groups(&self, q: u32) -> u32 {
        let [_, count, _, ns] = self.phase(q);
        if ns == 0 { count } else { self.ctl(10 + (self.a.phases + q) as usize).load(SeqCst) }
    }

    /// A segment's items that run a text's rows again (`seg_text_items`): those its groups reach, at most its items.
    unsafe fn seg_text_items(&self, seg: *const Seg, cap: u32) -> u32 {
        seg_groups(seg, cap).min((*seg).items)
    }

    /// The item group `g` of phase `p` runs, and the group it runs as (`phase_item`, decision 0637): a phase of no
    /// segments, its `g`-th item; else its segments' groups in order, a segment's `g`-th item (past its `items`, its
    /// first, as group `g`); the first segment's first alone where `none`; with `texts`, its `g`-th text item, as
    /// itself. `NONE` past them.
    unsafe fn phase_item(&self, p: u32, mut g: u32, none: u32, texts: bool) -> (u32, u32) {
        let [first, count, s0, ns] = self.phase(p);
        let cap = seg_cap(ns);
        let own = |i: u32| (i, (*self.items.add(i as usize)).group);
        if !texts && ns == 0 {
            return if g < count { own(first + g) } else { (NONE, 0) };
        }
        if !texts && none != 0 {
            return if g == 0 { own((*self.segs.add(s0 as usize)).first) } else { (NONE, 0) };
        }
        for s in 0..ns.max(1) {
            let at_ = if ns == 0 { first } else { (*self.segs.add((s0 + s) as usize)).first };
            if !texts {
                let sg = self.segs.add((s0 + s) as usize);
                let n = seg_groups(sg, cap);
                if g < n {
                    let group = if (*sg).per == 0 { (*self.items.add((at_ + g) as usize)).group } else { g };
                    return (at_ + if g < (*sg).items { g } else { 0 }, group);
                }
                g -= n;
                continue;
            }
            let n = if ns == 0 { count } else { self.seg_text_items(self.segs.add((s0 + s) as usize), cap) };
            for i in 0..n {
                if (*self.items.add((at_ + i) as usize)).what >> 24 != PK_ROW_TEXT {
                    continue;
                }
                if g == 0 {
                    return own(at_ + i);
                }
                g -= 1;
            }
        }
        (NONE, 0)
    }

    /// Phase `p`'s text items to run again (`phase_texts`): its count emptied, its items to count them; their number.
    unsafe fn phase_texts(&self, p: u32) -> u32 {
        let [first, count, s0, ns] = self.phase(p);
        let cap = seg_cap(ns);
        let mut m = 0u32;
        for g in 0..ns.max(1) {
            let (a, n) = if ns == 0 {
                (first, count)
            } else {
                let seg = self.segs.add((s0 + g) as usize);
                ((*seg).first, self.seg_text_items(seg, cap))
            };
            for i in 0..n {
                m += u32::from((*self.items.add((a + i) as usize)).what >> 24 == PK_ROW_TEXT);
            }
        }
        self.ctl(10 + (self.a.phases + p) as usize).store(m.max(1), SeqCst);
        self.ctl(10 + p as usize).store(0, SeqCst);
        m
    }

    /// The phase after phase `p` (`phase_after`, the phase and one, 0 none): a round's end's jump, else the next below
    /// the walk's end (`ProgP::end`; inside the program's tail, below its phases); none where a gate stopped the program.
    unsafe fn phase_after(&self, p: u32) -> u32 {
        let jump = self.ctl(7).load(SeqCst);
        if jump != 0 {
            self.ctl(7).store(0, SeqCst);
        }
        let stop = if p < self.a.end { self.a.end } else { self.a.phases };
        if jump != 0 {
            if jump == NONE { 0 } else { jump }
        } else if p + 1 < stop && self.ctl(6).load(SeqCst) & 2 == 0 {
            p + 2
        } else {
            0
        }
    }

    /// Phase `next - 1` reset and made the one running (`phase_enter`); its groups. A phase whose segments' counts
    /// start no group is passed over as run, to the one after it.
    unsafe fn phase_enter(&self, mut next: u32) -> u32 {
        for _ in 0..=self.a.phases {
            if next == 0 {
                break;
            }
            let q = next - 1;
            let none = self.phase_reset(q);
            self.ctl(0).store(q, SeqCst);
            self.ctl(3).store(none, SeqCst);
            let n = self.phase_groups(q);
            if n != 0 {
                return n;
            }
            // Passed over as run: its count its items', as a phase run whole has it.
            self.ctl(10 + q as usize).store(self.phase(q)[1], SeqCst);
            next = self.phase_after(q);
        }
        0
    }

    /// The next dispatch's groups, `n` (`ukl_chain_set`), the finisher's of dispatch `d`.
    unsafe fn chain_set(&self, d: u32, n: u32) {
        self.ctl(4).store(n, SeqCst);
        let slot = 10 + 2 * self.a.phases as usize + 4 * d as usize;
        self.ctl(slot).store(n, SeqCst);
        self.ctl(slot + 1).store(1, SeqCst);
        self.ctl(slot + 2).store(1, SeqCst);
        self.ctl(1).store(d, SeqCst);
    }

    /// Item `it`'s work, every thread of the group's: whether it is whole (a size script that parked is not).
    unsafe fn item(&self, it: Item) -> bool {
        let (kind, x, g) = (it.what >> 24, it.what & 0xff_ffff, it.group);
        if kind == PK_SIZE {
            return size_one(self.script.add(x as usize), g);
        }
        // A segment's item strides its job's rows past its phase's cap (`seg_cap`): `t`, `t + step`, ..., below its
        // count's, `step` the segment's groups handed out.
        let (mut step, mut lim) = (0u32, 0u32);
        if it.seg != 0 {
            let seg = self.segs.add(it.seg as usize - 1);
            if (*seg).per != 0 {
                step = seg_groups(seg, seg_cap(self.phase(it.phase)[3])) * TG;
                lim = (*(*seg).count).wrapping_add((*seg).plus);
            }
        }
        if kind == PK_SCAN_SUMS || kind == PK_SCAN_TILES {
            let lv = &*self.scans.add(x as usize);
            let w = |k: usize| core::ptr::addr_of!(lv.w).cast::<u32>().add(k).read_volatile();
            let n = w(0);
            let input = lv.input.add(w(2) as usize);
            let sums = lv.sums.add(w(3) as usize);
            let tiles = n.div_ceil(TG * SCAN_ITEMS);
            let tstep = step / TG;
            // A sized scan's first level whose next level is empty (decision 0647): its tiles' sums not scanned, each
            // tile adding the sums before it itself.
            let own = kind == PK_SCAN_TILES && w(1) & 2 != 0 && core::ptr::addr_of!((*self.scans.add(x as usize + 1)).w).cast::<u32>().read_volatile() == 0;
            let mut tile = g;
            loop {
                if kind == PK_SCAN_SUMS {
                    scan_tile_sum(input, sums, n, tile);
                } else if own {
                    let before = (0..tile as usize).fold(0u32, |a, i| a.wrapping_add(*sums.add(i)));
                    scan_tile_from(input, before, n, tile);
                } else {
                    scan_tile_from(input, if w(1) & 1 != 0 { *sums.add(tile as usize) } else { 0 }, n, tile);
                }
                if tstep == 0 || tile + tstep >= tiles {
                    break;
                }
                tile += tstep;
            }
            return true;
        }
        if kind == PK_RADIX_HIST || kind == PK_RADIX_SCATTER {
            let sp = &*self.sorts.add(x as usize);
            let w = |k: usize| core::ptr::addr_of!(sp.w).cast::<u32>().add(k).read_volatile();
            let (n, shift, tiles) = (w(0), sp.w[1], w(2));
            // The wide sort's (t58 increment A): its region the arena's, from its size script's word; its tiles as many
            // as its keys claimed. Else its keys' 32 bits. Its tiles, either way: this item's, then each `step` on (its
            // segment's groups) while below its tiles', as a scan's.
            let wide = (w(3) != 0).then(|| wide_at(self.counts.add(w(3) as usize - 1), n));
            let from = ((shift / 8) % 2) as usize;
            let tstep = step / TG;
            let mut tile = g;
            // The wide sort's (t58 increment A): its region the arena's, from its size script's word; its tiles as many
            // as its keys claimed, an item each stride.
            if w(3) != 0 {
                let wa = wide_at(self.counts.add(w(3) as usize - 1), n);
                let from = ((shift / 8) % 2) as usize;
                while tile < tiles {
                    if kind == PK_RADIX_HIST {
                        radix_hist_tile64(wa.keys[from], wa.hist, n, shift, tiles, tile);
                    } else {
                        radix_scatter_tile64(wa, from, n, shift, tiles, tile);
                    }
                    if tstep == 0 || tile + tstep >= tiles {
                        break;
                    }
                    tile += tstep;
                }
                return true;
            }
            loop {
                match wide {
                    Some(_) if tile >= tiles => break,
                    Some(wa) if kind == PK_RADIX_HIST => radix_hist_tile64(wa.keys[from], wa.hist, n, shift, tiles, tile),
                    Some(wa) => radix_scatter_tile64(wa, from, n, shift, tiles, tile),
                    None if kind == PK_RADIX_HIST => radix_hist_tile(sp.keys, sp.hist as *mut u32, n, shift, tiles, tile),
                    None => radix_scatter_tile(sp, n, shift, tiles, tile),
                }
                if tstep == 0 || tile + tstep >= tiles {
                    break;
                }
                tile += tstep;
            }
            return true;
        }
        let j = &*self.jobs.add(x as usize);
        let jw = |k: usize| core::ptr::addr_of!(j.at).cast::<u32>().add(k).read_volatile();
        let (l_at, o_at, v_at, h_at) = (jw(4), jw(5), jw(6), jw(7));
        let arena = self.counts as *mut i64;
        let pick = |at: u32, own: *mut i64| if at != 0 { arena.add(at as usize - 1) } else { own };
        let mut bufs = Bufs {
            l: pick(l_at, j.l as *mut i64),
            o: pick(o_at, j.o),
            v: pick(v_at, j.v),
            h: if h_at != 0 { self.counts.add(h_at as usize - 1) } else { j.h },
            la: j.a,
            lt: j.totals,
            c: self.counts.add(jw(0) as usize),
            kb: jw(2),
            eb: jw(3),
        };
        if h_at != 0 && (PK_LAT_CLEAR..=PK_LAT_MERGE).contains(&kind) {
            let places = (*(j.p as *const LatP)).pmask as usize + 1;
            bufs.la = bufs.h;
            bufs.lt = bufs.h.add(places);
            bufs.h = bufs.h.add(2 * places);
        }
        if kind == PK_ROW_TEXT {
            // A SIMD group a row: the group's SIMD groups, each its rows; a string join's (`ROW_STRJOIN`) or a text's.
            let st = St::new(&*j.s, bufs.l, j.r, j.rf);
            let strjoin = (*j.s).agg[0] & 15 == 6;
            for sg in g * (TG / SGW)..(g + 1) * (TG / SGW) {
                if strjoin {
                    row_strjoin(&st, j, &bufs, self.ctl, sg);
                } else {
                    row_text(&st, j, &bufs, self.ctl, sg);
                }
            }
            return true;
        }
        // The group's threads, a range: all of them, then (a segment's) each stride's, below its limit each. A
        // segment's rows past its limit are past the room its size script placed (its counts' `count + plus`): the
        // next job's, written there by its last group in whatever order the groups run (a scan's zeros over the next
        // job's counts, its rows lost).
        let mut t0 = g.wrapping_mul(TG);
        let mut t1 = t0.saturating_add(TG);
        if step != 0 {
            t1 = t1.min(lim);
            if t0 >= t1 {
                return true;
            }
        }
        loop {
            self.range(kind, j, &bufs, t0, t1);
            match t0.checked_add(step) {
                Some(next) if step != 0 && next < lim => (t0, t1) = (next, next.saturating_add(TG).min(lim)),
                _ => break,
            }
        }
        true
    }


    /// Threads [t0, t1) of job `j`, of kind `kind`: a range's vector work, or each thread's own.
    unsafe fn range(&self, kind: u32, j: &Job, b: &Bufs, t0: u32, t1: u32) {
        if !self.classes.is_null() && (*self.classes).range(kind, j, b, t0, t1) {
            return;
        }
        let stage = |s: *const Stage| St::new(&*s, b.l, j.r, j.rf);
        let lat = || &*(j.p as *const LatP);
        match kind {
            PK_COPY => rows_copy_range(b.l as *const u8, j.rf, b.o as *mut u8, j.totals, &*(j.p as *const Copy), t0, t1),
            PK_IX_KEY => ix_key_range(b.l as *const u8, j.rf, b.h, j.s_ as *mut u32, j.counters, &*(j.p as *const IxP), b.kb, b.eb, t0, t1),
            PK_IX_PLACE => ix_place_range(b.l as *const u8, j.vc as *const u32, j.counters, &*(j.p as *const IxP), b.eb, t0, t1),
            PK_IX_OFFS => ix_offs_range(b.l as *const u8, b.v as *const u32, j.a, &*(j.p as *const IxP), b.kb, b.eb, t0, t1),
            PK_IX_MARK => ix_mark_range(b.l as *const u8, j.rf, b.v as *const u32, j.vc as *const u32, j.a, j.counters, &*(j.p as *const IxP), b.kb, b.eb, t0, t1),
            // Threads [t0, t1) clear places 4 t0 to 4 t1, within those the rows use (`dedup_mask`).
            PK_DEDUP_CLEAR => {
                let end = dedup_mask(j.totals, &*j.dd) as u64 + 1;
                let (a, z) = ((4 * t0 as u64).min(end) as u32, (4 * t1 as u64).min(end) as u32);
                fill_u32(b.h, a, z, NONE)
            }
            PK_MAP => stage_map_range(&*j.s, b.o, j.totals, t0, t1),
            PK_DEDUP_CLAIM => {
                let d = &*j.dd;
                let n = (*j.totals.add(d.in_slot as usize)).min(d.cap);
                dedup_lanes::dedup_claim_lanes::<{ stage_lanes::LANES }>(b.l, b.h, d, dedup_mask(j.totals, d), t0.min(n), t1.min(n))
            }
            PK_STAGE_COUNT => stage_count_range(&stage(j.s), j, b, t0, t1),
            PK_SCAN_COUNT => scan_count_range(&stage(j.s), j, b, t0, t1),
            PK_WITNESS_COUNT => witness_count_range(&stage(j.s), j, b, t0, t1),
            // A sorted dedup's (t58 increment A): `h` its keys in, `w[0]` its places in; `w[1]`, `w[2]` its keys and
            // places sorted.
            PK_DEDUP_KEY => {
                let p = &*(j.p as *const HeadKeyP);
                dedup_key_range(b.l, j.totals, wide_at(b.h, p.total).keys[0], &*j.dd, p, b.eb, t0, t1)
            }
            PK_DEDUP_MARK if (*j.dd).sorted != 0 => {
                let p = &*(j.p as *const HeadKeyP);
                dedup_mark_sorted_range(j.totals, wide_at(b.h, p.total).keys[p.odd as usize], b.c, &*j.dd, j.vc, b.eb, t0, t1)
            }
            PK_DEDUP_MARK => dedup_mark_range(b.l, j.totals, b.h, b.c, j.s_, j.vc, &*j.dd, t0, t1),
            PK_STAGE_SCATTER => stage_scatter_range(&stage(j.ws), j, b, t0, t1),
            PK_SCAN_SCATTER => scan_scatter_range(&stage(j.s), j, b, t0, t1),
            PK_ROW_SCATTER => row_scatter_range(&stage(j.s), j, b, t0, t1),
            PK_DEDUP_SCATTER if (*j.dd).sorted != 0 => {
                let p = &*(j.p as *const HeadKeyP);
                dedup_scatter_sorted_range(j.totals, b.c, b.o, &*j.dd, j.vc, p, wide_at(b.h, p.total).keys[p.odd as usize], b.eb, t0, t1)
            }
            PK_DEDUP_SCATTER => dedup_scatter_range(b.l, j.totals, b.c, b.o, j.s_, j.vc, &*j.dd, t0, t1),
            PK_LAT_HCLEAR => lat_hclear_range(j.vc, lat(), t0, t1),
            // A program's tail's growth of a map that did not grow (`omask` all ones, `Batch::lattice_tail`): no
            // thread has a place to move, so the range is skipped whole rather than thread by thread.
            PK_LAT_REHASH if lat().omask == NONE => {}
            PK_LAT_CLEAR => lat_clear_range(b.la, b.lt, b.h, lat(), t0, t1),
            PK_SUM if t0 == 0 && t1 > 0 => sum_one(j.p as *const SumP),
            PK_ROUND if t0 == 0 && t1 > 0 => round_one(j.p as *const RoundP, self.ctl),
            PK_SUM | PK_ROUND => {}
            _ => {
                for t in t0..t1 {
                    self.job(kind, j, b, t);
                }
            }
        }
    }

    /// Thread `t`'s part of job `j`, of a kind whose threads each go their own way (a hash table's probe, a claim's
    /// retries).
    unsafe fn job(&self, kind: u32, j: &Job, b: &Bufs, t: u32) {
        let lat = || &*(j.p as *const LatP);
        match kind {
            PK_LAT_REHASH | PK_SET_PLACE => lat_rehash_one(j.s_, j.g, j.vc, lat(), t),
            PK_LAT_CLAIM => lat_claim_one(b.l, b.la, j.cnt, lat(), t),
            PK_LAT_BEST => lat_best_one(b.l, b.la, b.lt, b.h, b.v as *const u32, j.cnt, lat(), t),
            PK_LAT_MERGE => lat_merge_one(b.l, b.la, b.lt, b.h, j.s_, j.vc, b.o, j.counters, b.v as *const u32, j.g as *const u32, lat(), t),
            PK_GATE => gate_one(j.p as *const GateP, self.ctl, t),
            _ => {}
        }
    }

    /// Group `g` of the chain's dispatch now (`join_run`): the start (dispatch 0), else its phase's item, and the
    /// next dispatch's groups where it finishes the phase.
    unsafe fn group(&self, g: u32) {
        let (d, p, mode, back) = (self.ctl(1).load(SeqCst), self.ctl(0).load(SeqCst), self.ctl(3).load(SeqCst), self.ctl(2).load(SeqCst));
        if d == 0 {
            let pp = self.ctl(9).load(SeqCst);
            let n = if self.ctl(5).load(SeqCst) == 0 {
                self.ctl(5).store(1, SeqCst);
                self.phase_enter(1)
            } else if pp >> 30 == 1 {
                self.ctl(9).store(0, SeqCst);
                let q = pp & 0x3fff_ffff;
                let n = self.phase_texts(q);
                self.ctl(0).store(q, SeqCst);
                self.ctl(3).store(2, SeqCst);
                n
            } else if pp >> 29 == 1 {
                self.ctl(9).store(0, SeqCst);
                self.phase_enter(self.phase_after(pp & 0x1fff_ffff))
            } else if back != 0 {
                1
            } else {
                self.ctl(4).load(SeqCst)
            };
            self.chain_set(1, n);
            return;
        }
        let (mut p, mut back) = (p, back);
        let (mut item, mut group) = if back != 0 {
            (back - 1, (*self.items.add(back as usize - 1)).group)
        } else {
            self.phase_item(p, g, mode & 1, mode & 2 != 0)
        };
        // Its item, then whether it finishes its phase; a finisher whose next phase is few and light runs it itself,
        // each of its groups in turn (`gi` of its `gn`; 0: the dispatch's own item), all of them once begun.
        let (mut gi, mut gn, mut ph) = (0u32, 0u32, 0u32);
        for _ in 0..JOIN_INLINE * JOIN_INLINE_GROUPS {
            let whole = item == NONE || {
                let mut it = *self.items.add(item as usize);
                it.group = group;
                self.timed(it)
            };
            let q = self.finish(p, item, whole, back, d, ph + 1 < JOIN_INLINE);
            let (next, groups) = if q != 0 {
                (0, self.phase_groups(q - 1))
            } else if whole && gi + 1 < gn {
                (gi + 1, gn)
            } else {
                return;
            };
            if next == 0 {
                ph += 1;
                p = q - 1;
            }
            (back, gi, gn) = (0, next, groups);
            (item, group) = self.phase_item(p, gi, self.ctl(3).load(SeqCst) & 1, false);
        }
    }

    /// The group's item `item` of phase `p` counted (`prog_finish`): where it finishes its phase, the next reset and
    /// its groups, the next dispatch's, or the phase and one where they are few and light and `inline_ok` (this group
    /// runs each of its groups); 0 where the group is done with the phase.
    unsafe fn finish(&self, p: u32, item: u32, whole: bool, back: u32, d: u32, inline_ok: bool) -> u32 {
        if back != 0 {
            self.ctl(2).store(0, SeqCst);
        }
        if !whole {
            self.ctl(9).store(item + 1, SeqCst);
            self.ctl(6).fetch_or(32, SeqCst);
            return 0;
        }
        let want = self.ctl(10 + (self.a.phases + p) as usize).load(SeqCst);
        let want = if want != 0 { want } else { self.phase(p)[1] };
        if self.ctl(10 + p as usize).fetch_add(1, SeqCst) + 1 != want {
            return 0;
        }
        self.ctl(8).fetch_add(1, SeqCst);
        let flags = self.ctl(6).load(SeqCst);
        if flags & 128 != 0 {
            self.ctl(6).fetch_and(!128, SeqCst);
        }
        let n = if flags & 128 != 0 && flags & 2 == 0 {
            // A gate's room mapped past: the program parks after the gate's phase.
            self.ctl(9).store(0xC000_0000 | p, SeqCst);
            self.ctl(6).fetch_or(32, SeqCst);
            0
        } else if flags & 64 != 0 {
            // Its texts found the arena full: the phase parks, and runs its texts again once the host maps more.
            self.ctl(6).fetch_and(!64, SeqCst);
            self.ctl(9).store(0x8000_0000 | p, SeqCst);
            self.ctl(6).fetch_or(32, SeqCst);
            0
        } else {
            self.phase_enter(self.phase_after(p))
        };
        if inline_ok && (n == 1 || (n != 0 && n <= JOIN_INLINE_GROUPS && self.phase_light(self.ctl(0).load(SeqCst)))) {
            return self.ctl(0).load(SeqCst) + 1;
        }
        self.chain_set(d + 1, n);
        0
    }
}

/// The join program's group (`join_run`): its `JOIN_TG` threads, run as one.
///
/// # Safety
///
/// `argv` holds the kernel's arguments (each the address of its bytes: a buffer's address, the `ProgP`'s bytes), its
/// buffers as the host made them.
pub unsafe fn join_run(argv: *const *const u8, l: &Launch) {
    join_run_timed(argv, l, core::ptr::null_mut());
}

/// `join_run`, each item's time added to its kind's in `clock` (the join lab's profile; none: `join_run`).
///
/// # Safety
///
/// As `join_run`'s; `clock` is null or an `ItemClock` of the caller's, which only this group's run writes.
pub unsafe fn join_run_timed(argv: *const *const u8, l: &Launch, clock: *mut ItemClock) {
    join_run_classed(argv, l, clock, None);
}

/// `join_run_timed`, each job of a stage whose form is one of `classes`' run by its class's parts
/// (`classes::Classes`), the rest by the program's work. None: the classes its host gave the launch (`Launch::host`,
/// the CPU device's: the address of its runs' shapes' classes), or none.
///
/// # Safety
///
/// As `join_run_timed`'s.
pub unsafe fn join_run_classed(argv: *const *const u8, l: &Launch, clock: *mut ItemClock, classes: Option<&classes::Classes>) {
    let threads = l.block[0] * l.block[1] * l.block[2];
    assert!(threads == TG, "join_run: a group of {TG} threads, not {threads}");
    let buf = |i: usize| (*argv.add(i) as *const *mut u8).read_unaligned();
    let a = (*argv.add(9) as *const ProgP).read_unaligned();
    let prog = Prog {
        jobs: buf(0) as *mut Job,
        items: buf(1) as *const Item,
        phases: buf(2) as *const u32,
        segs: buf(3) as *const Seg,
        scans: buf(4) as *mut ScanLevel,
        sorts: buf(5) as *const SortPass,
        script: buf(6) as *const SizeOp,
        ctl: buf(7) as *mut u32,
        counts: buf(8) as *mut u32,
        a,
        clock,
        // SAFETY: a host's word is the address of classes its launch holds until its groups are done.
        classes: classes.map_or(l.host as *const classes::Classes, |c| c as *const classes::Classes),
    };
    // Its groups run one after another on one thread (the CPU device's), each as the kernel's group.
    prog.group(l.group[0]);
}
