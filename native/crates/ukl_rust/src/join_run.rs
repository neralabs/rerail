//! `join_run`, the join program (`join_stages.uglykl` with `JOIN_PROGRAM`, decision 0603), written by hand in Rust:
//! the oracle the Rust that rerail-ukl-vec writes from the same source is held to (decision 0567), in results and in
//! the shape of its code.
//!
//! Its shape is the source's, read as a CPU runs it. A group is `JOIN_TG` (256) threads, and an item's work is a
//! function of a thread (`stage_count_one`, `dedup_claim_one`, ...), so the group's work is a loop over its threads,
//! each a call of that function: one function a source function, small, its branches and loops a thread's own. What
//! a SIMD group does together on a GPU is here what one thread does alone: a text's bytes walked by an automaton one
//! after another (`chunk_states`' prefix over the lanes is a GPU's way to walk them), a scan's tile a running sum, a
//! sort's tile a stable count. Barriers order nothing: the group runs in one thread.
//!
//! The reads are the reads written once in CLIF (decision 0310), the CPU's compiled code, called through the launch's
//! table (`Launch::reads`, `rerail_clif::reads::READS`' order), never written again here.

// Every function here reads and writes the device's buffers through raw pointers, as the source's do.
#![allow(unsafe_op_in_unsafe_fn, clippy::too_many_arguments, clippy::missing_safety_doc)]

use crate::Launch;
use core::sync::atomic::{AtomicU32, Ordering::SeqCst};

const NONE: u32 = u32::MAX;
/// A group's threads (`JOIN_TG`).
const TG: u32 = 256;
/// A text row's SIMD group (`SGW`): its rows and their counts are laid out by it.
const SGW: u32 = 32;
const KEY_SEED: u64 = 0x243F_6A88_85A3_08D3;
const TURNS: u32 = 100_000_000;
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

#[repr(C)]
struct ViewRow {
    entry: u64,
    tape: u64,
    lines: u64,
    homes: [u32; 2],
    rows: [u32; 12],
    view: [u32; 32],
}
#[repr(C)]
struct ViewHome {
    host: u64,
    gpu: *const u8,
}
#[repr(C)]
struct ViewDir {
    at: u64,
    ndocs: u32,
    nhomes: u32,
    rows: u64,
    homes: u64,
    cache: u64,
}
#[repr(C)]
struct PlacedDir {
    dir: *const ViewDir,
    tail: *const u8,
    at: u64,
    rows: u32,
    pad: u32,
}
const _: () = assert!(size_of::<ViewRow>() == 208 && size_of::<ViewDir>() == 40 && size_of::<PlacedDir>() == 32);

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
}
const _: () = assert!(size_of::<Dedup>() == 92);

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
}
const _: () = assert!(size_of::<Job>() == 208);

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
    pad: u32,
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
    // Its words (`ScanWords`): n, offsets, at, sums_at.
    w: [u32; 4],
}
#[repr(C)]
struct SortPass {
    keys: *const u32,
    values: *const u32,
    hist: *const u32,
    out_keys: *mut u32,
    out_values: *mut u32,
    // Its words (`SortWords`): n, shift, tiles, pad.
    w: [u32; 4],
    pad2: u64,
}
#[repr(C)]
struct ProgP {
    phases: u32,
    capacity: u32,
    seeds: u32,
    budget: u32,
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
const _: () = assert!(size_of::<Item>() == 16 && size_of::<Seg>() == 24 && size_of::<ScanLevel>() == 32);
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

// ---- The reads written once in CLIF, compiled: each by its place in `READS`. ----

#[derive(Clone, core::marker::Copy)]
struct Reads(*const *const u8);

/// A read of a relation (its block `t`, its descriptor at `a0`): the compiled code takes their sum.
macro_rules! rel_read {
    ($name:ident, $i:expr, ($($a:ident: $t:ty),*) -> $r:ty) => {
        #[inline]
        unsafe fn $name(self, t: *const u8, a0: u64, $($a: $t),*) -> $r {
            let f: unsafe extern "C" fn(u64, $($t),*) -> $r = core::mem::transmute(*self.0.add($i));
            f((t as u64).wrapping_add(a0), $($a),*)
        }
    };
}

/// The views' reads, as `join_stages`' `vread_dir` calls them: the node, child, field, core, line, loc_line, loc_end,
/// loc_col, loc_docof, tape, depth and order views.
const VIEW_READS: [usize; 12] = [51, 52, 53, 54, 55, 56, 57, 58, 59, 76, 77, 78];

impl Reads {
    rel_read!(rows_len, 27, () -> u32);
    rel_read!(rows_put, 28, (r: u32, c: u32, v: u64) -> u32);
    rel_read!(rel_get, 29, (r: u32, c: u32) -> u64);
    rel_read!(rel_dead, 30, (r: u32) -> u32);
    rel_read!(rel_len, 31, () -> u32);
    rel_read!(rel_group, 32, (index: u32, h: u64) -> u64);
    rel_read!(rel_group_row, 33, (index: u32, q: u32) -> u32);
    rel_read!(rel_key_mix, 34, (h: u64, v: u64) -> u64);
    rel_read!(bloom_hash, 35, (key: u64) -> u64);
    rel_read!(bloom_may, 37, (h: u64) -> u32);
    rel_read!(atoms_len, 65, (v: u32) -> u32);
    // Its result an address (`reads::addresses`): the symbol's bytes.
    rel_read!(atoms_at, 66, (v: u32) -> u64);
    rel_read!(verdict_get, 67, (test: u32, v: u32) -> u32);
    rel_read!(verdict_put, 68, (test: u32, v: u32, verdict: u32) -> u32);
    // Their results an address, its low two bits what was found.
    rel_read!(text_find, 69, (f: u32, sym: u32) -> u64);
    rel_read!(text_open, 70, (f: u32, sym: u32, len: u32) -> u64);
    // Its parameter an address: the entry.
    rel_read!(cache_done, 71, (entry: u64) -> u32);
    rel_read!(dir_doc_of_row, 79, (k: u32, r: u32, memo: u32) -> u32);
    rel_read!(dir_doc_of_key, 80, (key: u32, memo: u32) -> u32);

    /// View read `i` (`READS`' place) of document entry `e` of directory `d`: its row `r`, column `col`.
    #[inline]
    unsafe fn view(self, i: usize, d: *const ViewDir, e: *const ViewRow, r: u32, col: u32) -> u64 {
        let f: unsafe extern "C" fn(u64, u64, u64, u64, u32, u32) -> u64 = core::mem::transmute(*self.0.add(i));
        let (b, t, l) = view_parts(d, e);
        f((b as u64).wrapping_add((*e).entry), t as u64, (*e).view.as_ptr() as u64, l as u64, r, col)
    }

    /// A view's walk read `i` (`child_first`, `child_next`, `number`, `up`, `end`) of node `x`.
    #[inline]
    unsafe fn walk(self, i: usize, d: *const ViewDir, e: *const ViewRow, x: u32) -> u32 {
        let f: unsafe extern "C" fn(u64, u64, u64, u64, u32) -> u32 = core::mem::transmute(*self.0.add(i));
        let (b, t, l) = view_parts(d, e);
        f((b as u64).wrapping_add((*e).entry), t as u64, (*e).view.as_ptr() as u64, l as u64, x)
    }

    #[inline]
    unsafe fn key_mix(self, h: u64, v: i64) -> u64 {
        self.rel_key_mix(core::ptr::null(), 0, h, v as u64)
    }

    unsafe fn key_hash(self, kv: &[i64]) -> u64 {
        kv.iter().fold(KEY_SEED, |h, &v| self.key_mix(h, v))
    }
}
const CHILD_FIRST: usize = 62;
const CHILD_NEXT: usize = 63;
const NUMBER: usize = 64;
const UP: usize = 74;
const END: usize = 75;

// ---- Views read in place (decision 0353 step 4d). ----

unsafe fn view_block(d: *const ViewDir) -> *const u8 {
    (d as *const u8).wrapping_sub((*d).at as usize)
}
unsafe fn view_rows(d: *const ViewDir) -> *const ViewRow {
    view_block(d).add((*d).rows as usize) as *const ViewRow
}
unsafe fn view_home(d: *const ViewDir, h: u32, at: u64) -> *const u8 {
    let homes = view_block(d).add((*d).homes as usize) as *const ViewHome;
    (*homes.add(h as usize)).gpu.wrapping_add(at as usize)
}
/// The directory's block, document `e`'s tape and its lines.
unsafe fn view_parts(d: *const ViewDir, e: *const ViewRow) -> (*const u8, *const u8, *const u8) {
    (view_block(d), view_home(d, (*e).homes[0], (*e).tape), view_home(d, (*e).homes[1], (*e).lines))
}

unsafe fn vread_dir(rd: Reads, kind: u32, d: *const ViewDir, r: u32, col: u32, memo: &mut u32) -> i64 {
    let k = kind - 1;
    let lo = rd.dir_doc_of_row(view_block(d), (*d).at, k, r, *memo);
    *memo = lo;
    let e = view_rows(d).add(lo as usize);
    let i = r.wrapping_sub((*e).rows[k as usize]);
    rd.view(VIEW_READS[(k as usize).min(11)], d, e, i, col) as i64
}

unsafe fn vread(rd: Reads, kind: u32, d: *const ViewDir, r: u32, col: u32, memo: &mut u32) -> i64 {
    if kind & VIEW_PLACED == 0 {
        return vread_dir(rd, kind, d, r, col, memo);
    }
    let p = d as *const PlacedDir;
    if r < (*p).rows {
        return vread_dir(rd, kind & !VIEW_PLACED, (*p).dir, r, col, memo);
    }
    rd.rel_get((*p).tail, (*p).at, r - (*p).rows, col) as i64
}

// ---- A stage's values (decision 0095). ----

/// The stage's parameters and the reads: what every function of a stage takes.
#[derive(Clone, core::marker::Copy)]
struct St<'a> {
    s: &'a Stage,
    rd: Reads,
    l: *const i64,
    r: *const i64,
}

impl St<'_> {
    unsafe fn rget(&self, r: u32, col: u32) -> i64 {
        let s = self.s;
        let mut memo = NONE;
        if s.v_has != 0 {
            return vread(self.rd, 1, s.v_dir, r, 1, &mut memo);
        }
        if s.r_view != 0 {
            return vread(self.rd, s.r_view, s.v_dir, r, col, &mut memo);
        }
        self.rd.rel_get(self.r as *const u8, s.r_at, r, col) as i64
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
            _ => Some(x.min(y)),
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
        let len = self.rd.rel_len(self.r as *const u8, s.r_at);
        if len > s.r_from { (len - s.r_from).min(s.nr) } else { 0 }
    }

    unsafe fn row_live(&self, r: u32) -> bool {
        self.s.r_dead == 0 || self.rd.rel_dead(self.r as *const u8, self.s.r_dead, r) == 0
    }

    /// Whether node row `q`'s kind atom is in the stage's set (ascending).
    unsafe fn has_member(&self, q: u32) -> bool {
        let s = self.s;
        let mut memo = NONE;
        let k = vread(self.rd, 1, s.v_dir, q, 2, &mut memo);
        let set = core::slice::from_raw_parts(s.v_set, s.v_nset as usize);
        set.binary_search(&k).is_ok()
    }

    /// Whether row `r` has the binding's keys and passes the relation's column tests.
    unsafe fn matches(&self, r: u32, kv: &[i64; 8]) -> bool {
        let s = self.s;
        if s.v_has != 0 {
            return s.ntests == 0 && (s.v_has == 1 || self.has_member(r));
        }
        if !self.row_live(r) {
            return false;
        }
        (0..s.nkeys.min(8) as usize).all(|i| self.rget(r, s.key_rcol[i]) == kv[i])
            && (0..s.ntests.min(8) as usize).all(|i| self.rget(r, s.test_a[i]) == self.rget(r, s.test_b[i]))
    }

    /// A parent's children (`by_parent`) or a node's walk of `has` (`by_has`): its document, its numbered node.
    unsafe fn doc_node(&self, key: i64) -> Option<(*const ViewRow, u32)> {
        let d = self.s.v_dir;
        if key < 0 || (*d).ndocs == 0 {
            return None;
        }
        let a = self.rd.dir_doc_of_key(view_block(d), (*d).at, ((key as u64) >> 32) as u32, NONE);
        if a == NONE {
            return None;
        }
        let e = view_rows(d).add(a as usize);
        let l = self.rd.walk(NUMBER, d, e, key as u32);
        (l != NONE).then_some((e, l))
    }

    /// The candidates of keys `kv`: [lo, hi) and how they are walked (0 rows, 1 an index's entries, 2 a parent's
    /// children by their next siblings, 3 a node's numbered ancestors).
    unsafe fn probe(&self, kv: &[i64; 8]) -> Walk {
        let s = self.s;
        if s.nkeys == 0 {
            return Walk { lo: s.r_from, hi: s.r_from.wrapping_add(self.rows()), how: 0 };
        }
        let none = Walk { lo: 0, hi: 0, how: 0 };
        let d = s.v_dir;
        if s.v_has != 0 {
            let Some((e, l)) = self.doc_node(kv[0]) else { return none };
            let first = (*e).rows[0];
            if s.v_has == 1 {
                if !self.has_member(first.wrapping_add(l)) {
                    return none;
                }
                let u = self.rd.walk(UP, d, e, l);
                return if u == NONE { none } else { Walk { lo: first.wrapping_add(u), hi: NONE, how: 3 } };
            }
            return Walk { lo: first.wrapping_add(l).wrapping_add(1), hi: first.wrapping_add(self.rd.walk(END, d, e, l)), how: 0 };
        }
        if s.v_parent != 0 {
            let Some((e, l)) = self.doc_node(kv[0]) else { return none };
            let first = (*e).rows[1];
            if l == 0 {
                let a = e.offset_from(view_rows(d)) as u32;
                let hi = if a + 1 < (*d).ndocs { (*view_rows(d).add(a as usize + 1)).rows[1] } else { s.nr };
                return Walk { lo: first, hi, how: 0 };
            }
            let c = self.rd.walk(CHILD_FIRST, d, e, l);
            return if c == NONE { none } else { Walk { lo: first.wrapping_add(c), hi: NONE, how: 2 } };
        }
        let g = self.rd.rel_group(self.r as *const u8, s.r_at, s.r_index, self.rd.key_hash(&kv[..s.nkeys.min(8) as usize]));
        let lo = (g >> 32) as u32;
        Walk { lo, hi: lo.wrapping_add(g as u32), how: 1 }
    }

    /// A binding's candidates and its keys' values; none where the probed relation's screen refuses the key.
    unsafe fn candidates(&self, t: u32, f: *const u8, kv: &mut [i64; 8]) -> Walk {
        let s = self.s;
        if s.mode == 3 {
            return Walk { lo: 0, hi: 1, how: 0 };
        }
        for i in 0..s.nkeys.min(8) {
            kv[i as usize] = self.key_value(t, i);
        }
        if s.screen != 0 && s.screen <= s.nkeys && s.screen <= 8 {
            let h = self.rd.bloom_hash(f, 0, kv[s.screen as usize - 1] as u64);
            if self.rd.bloom_may(f, s.screen_at as u64, h) == 0 {
                return Walk { lo: 0, hi: 0, how: 0 };
            }
        }
        self.probe(kv)
    }

    /// Candidate `q`'s row.
    unsafe fn cand_row(&self, how: u32, q: u32) -> u32 {
        if how == 1 { self.rd.rel_group_row(self.r as *const u8, self.s.r_at, self.s.r_index, q) } else { q }
    }

    /// The candidate after `q`.
    unsafe fn cand_next(&self, how: u32, q: u32) -> u32 {
        let d = self.s.v_dir;
        match how {
            2 => {
                let e = view_rows(d).add(self.rd.dir_doc_of_row(view_block(d), (*d).at, 1, q, NONE) as usize);
                let n = self.rd.walk(CHILD_NEXT, d, e, q.wrapping_sub((*e).rows[1]));
                if n == NONE { NONE } else { (*e).rows[1].wrapping_add(n) }
            }
            3 => {
                let e = view_rows(d).add(self.rd.dir_doc_of_row(view_block(d), (*d).at, 0, q, NONE) as usize);
                let u = self.rd.walk(UP, d, e, q.wrapping_sub((*e).rows[0]));
                if u == NONE { NONE } else { (*e).rows[0].wrapping_add(u) }
            }
            _ => q.wrapping_add(1),
        }
    }

    /// The candidates' rows, in their walk's order.
    unsafe fn each(&self, w: Walk, mut f: impl FnMut(u32) -> bool) {
        let mut q = w.lo;
        while q < w.hi {
            if !f(self.cand_row(w.how, q)) {
                return;
            }
            q = self.cand_next(w.how, q);
        }
    }

    /// An aggregate over the binding's matching rows: its value, else 1 (none), or 2 (a sum past 64 bits).
    unsafe fn aggregate(&self, t: u32, w: Walk, kv: &[i64; 8]) -> Result<i64, u32> {
        let s = self.s;
        let kind = s.agg[0] & 0xff;
        let (mut cnt, mut sum, mut mn, mut mx, mut any, mut over) = (0i64, 0i64, 0i64, 0i64, false, false);
        self.each(w, |r| {
            if self.matches(r, kv) {
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

    /// Text operand `k` of kind `kind` (0 a symbol, 1 `cat`'s, 2 `str_concat`'s, 3 an arena text).
    unsafe fn txt_of(&self, t: u32, d: *const u8, a: *const u32, k: u32, kind: u32, unheld: &mut bool) -> Txt {
        let v = self.key_value(t, k);
        if kind == 3 {
            let off = ((v as u64) >> 24) & ((1u64 << 39) - 1);
            let len = (v & 0xff_ffff) as u32;
            let ok = v != -1 && off + len as u64 <= *a.add(1) as u64;
            return Txt::raw(arena_bytes(a).add(if ok { off as usize } else { 0 }), if ok { len } else { 0 });
        }
        let (pa, al) = sym_text(self.rd, d, v, unheld);
        let mut x = Txt { a: pa, al, b: d, bl: 0, dot: 0 };
        if kind != 0 {
            let (pb, bl) = sym_text(self.rd, d, self.key_value(t, k + 1), unheld);
            x.b = pb;
            x.bl = bl;
            x.dot = (kind == 1 && al > 0) as u32;
        }
        x
    }
}

#[derive(Clone, core::marker::Copy)]
struct Walk {
    lo: u32,
    hi: u32,
    how: u32,
}

fn lane_of(v: i64, w: u32) -> i64 {
    match w {
        16 => v as i16 as i64,
        32 => v as i32 as i64,
        _ => v,
    }
}

// ---- Texts. ----

/// A text operand: a's bytes, "." when `dot`, b's bytes.
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
    unsafe fn byte(&self, i: u32) -> u8 {
        if i < self.al {
            return *self.a.add(i as usize);
        }
        let i = i - self.al;
        if i < self.dot { b'.' } else { *self.b.add((i - self.dot) as usize) }
    }
    /// Its bytes from `from`, in order.
    unsafe fn bytes(&self, from: u32, to: u32) -> impl Iterator<Item = u8> + '_ {
        (from..to).map(move |i| self.byte(i))
    }
    /// Whether its `n` bytes from `xs` are `y`'s from `ys`.
    unsafe fn same(&self, xs: u32, y: &Txt, ys: u32, n: u32) -> bool {
        (0..n).all(|k| self.byte(xs + k) == y.byte(ys + k))
    }
    /// Its characters: its bytes less the UTF-8 continuation bytes.
    unsafe fn chars(&self) -> u32 {
        self.bytes(0, self.len()).filter(|&c| c & 0xc0 != 0x80).count() as u32
    }
}

unsafe fn arena_bytes(a: *const u32) -> *const u8 {
    (a as *const u8).add(16)
}

fn arena_value(off: u32, len: u32) -> i64 {
    (0x8000_0000_0000_0000u64 | ((off as u64) << 24) | len as u64) as i64
}

/// Symbol `v`'s bytes through the atoms directory `d`; a value no atom of it is reads as empty, `unheld` set.
unsafe fn sym_text(rd: Reads, d: *const u8, v: i64, unheld: &mut bool) -> (*const u8, u32) {
    let n = if (0..=u32::MAX as i64).contains(&v) { rd.atoms_len(d, 0, v as u32) } else { NONE };
    if n == NONE {
        *unheld = true;
        return (d, 0);
    }
    (rd.atoms_at(d, 0, v as u32) as *const u8, n)
}

/// Mode 7's test of pattern `x`'s bytes [xs, xs + lx) against subject `y`: by op 0 equal, 1 a prefix, 2 a suffix.
unsafe fn textop_one(op: u32, x: &Txt, xs: u32, lx: u32, y: &Txt, ly: u32) -> bool {
    let fits = if op == 0 { lx == ly } else { ly >= lx };
    fits && x.same(xs, y, if op >= 2 { ly - lx } else { 0 }, lx)
}

/// Mode 7: X a pattern, or a set of them (`TEXT_SPLIT`: 0x1f between them), Y passing when one does.
unsafe fn textop_pass(st: &St, t: u32, d: *const u8, a: *const u32, unheld: &mut bool, host: &mut bool) -> bool {
    let s = st.s;
    let x = st.txt_of(t, d, a, 0, s.agg[1] & 15, unheld);
    let y = st.txt_of(t, d, a, 2, s.agg[1] >> 4, unheld);
    let (lx, ly, op) = (x.len(), y.len(), (s.agg[0] >> 1) & 3);
    let split = s.agg[0] & 32 != 0;
    let mut r = false;
    let mut start = 0;
    for i in 0..=lx {
        let sep = split && i < lx && x.byte(i) == 0x1f;
        if sep && s.agg[0] & 64 != 0 {
            *host = true;
        }
        if sep || i == lx {
            r |= textop_one(op, &x, start, i - start, &y, ly);
            start = i + 1;
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
    match x.bytes(0, x.len()).cmp(y.bytes(0, y.len())) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// An automaton's table (`text::table::Table`): its byte classes, then its transitions, `s` states of `c` classes.
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

/// Whether table `tab` (`s` states, `c` classes, from `start`) accepts `x`: its state after the end of the text.
unsafe fn txt_verdict(tab: *const u8, s: u32, c: u32, start: u32, x: &Txt) -> bool {
    let a = Dfa { tab, c };
    let q = x.bytes(0, x.len()).fold(start, |q, b| a.step(q, b));
    let eoi = tab.add(256 + (s * c) as usize);
    *eoi.add(s as usize + *eoi.add(q as usize) as usize) != 0
}

/// `replace_all` of regex `g` in `x` by `z` (`text::table::Replacer`): each match's end by the forward automaton (the
/// last match before it dies), its start by the reverse one run back from it (the furthest match), matches delayed a
/// byte. The replaced text's length; written to `o` where given.
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
        for k in pos..n {
            let q = fwd.step(st, x.byte(k));
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
            put_bytes(o.add(w as usize), x.bytes(pos, first));
            put_bytes(o.add((w + first - pos) as usize), z.bytes(0, lz));
        }
        w += first - pos + lz;
        pos = e;
    }
    if let Some(o) = o {
        put_bytes(o.add(w as usize), x.bytes(pos, n));
    }
    w + n - pos
}

/// `replace_all` of literal `y` (empty: none) in `x` by `z`, matches not overlapping.
unsafe fn lit_replace_txt(x: &Txt, y: &Txt, z: &Txt, o: Option<*mut u8>) -> u32 {
    let (n, ly, lz) = (x.len(), y.len(), z.len());
    let (mut pos, mut w) = (0u32, 0u32);
    while ly > 0 && pos + ly <= n {
        let Some(m) = (pos..=n - ly).find(|&k| x.same(k, y, 0, ly)) else { break };
        if let Some(o) = o {
            put_bytes(o.add(w as usize), x.bytes(pos, m));
            put_bytes(o.add((w + m - pos) as usize), z.bytes(0, lz));
        }
        w += m - pos + lz;
        pos = m + ly;
    }
    if let Some(o) = o {
        put_bytes(o.add(w as usize), x.bytes(pos, n));
    }
    w + n - pos
}

unsafe fn put_bytes(o: *mut u8, bytes: impl Iterator<Item = u8>) {
    for (i, b) in bytes.enumerate() {
        *o.add(i) = b;
    }
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

/// The byte of character `c` of `x` (its first `n` bytes; `n` when it has fewer).
unsafe fn char_byte(x: &Txt, n: u32, c: i64) -> u32 {
    let mut seen = 0i64;
    for i in 0..n {
        if x.byte(i) & 0xc0 != 0x80 {
            if seen == c {
                return i;
            }
            seen += 1;
        }
    }
    n
}

/// A function's kept range of `x` [from, to), and whether `x` has a byte past ASCII (`lower`'s, f 0).
unsafe fn text_range(f: u32, x: &Txt, n0: i64, n1: i64) -> (u32, u32, bool) {
    let n = x.len();
    match f {
        1 => (0, (0..n).find(|&i| x.byte(i) == b'.').unwrap_or(n), false),
        2 => (0, (0..n).rev().find(|&i| x.byte(i) == b'/').unwrap_or(0), false),
        3 | 4 | 7 => {
            let (a, b) = substr_range(f, n0, n1, x.chars() as i64);
            (char_byte(x, n, a), char_byte(x, n, b), false)
        }
        0 => (0, n, x.bytes(0, n).any(|c| c >= 0x80)),
        _ => (0, n, false),
    }
}

// ---- The stages' functions, a thread a binding (or a candidate row). ----

/// The buffers an item's job reaches (`Job`, its words placed in the arena where its size script placed them).
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

unsafe fn stage_count_one(st: &St, j: &Job, b: &Bufs, t: u32) {
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
    st.each(w, |r| {
        if if s.mode == 3 { !st.row_live(r) } else { !st.matches(r, &kv) } {
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
            c += 1;
        }
        true
    });
    if exists {
        c = (found != (s.mode == 2)) as u32;
    }
    *counts.add(t as usize) = c;
    if !exists && s.counter != NONE {
        if p0 > 0 {
            at(j.counters.add(s.counter as usize)).fetch_add(p0, SeqCst);
        }
        for (m, &q) in q_counts.iter().enumerate().take(passed) {
            if q > 0 {
                at(j.counters.add(s.counter as usize + 1 + m)).fetch_add(q, SeqCst);
            }
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
        _ => {
            let mut kv = [0i64; 8];
            let w = st.candidates(t, j.f, &mut kv);
            st.each(w, |r| {
                if a >= s.cap_out {
                    return false;
                }
                if if s.mode == 3 { !st.row_live(r) } else { !st.matches(r, &kv) } {
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
unsafe fn witness_probe(b: &Stage, rd: Reads, l: *const i64, t: u32, cap_in: u32, a: &St, ra: u32, r: *const i64) -> bool {
    let mut kv = [0i64; 8];
    for (i, k) in kv.iter_mut().enumerate().take(b.nkeys.min(8) as usize) {
        *k = match b.key_kind[i] {
            1 => col(l, b.key_slot[i], cap_in, t),
            2 => a.rget(ra, b.key_slot[i]),
            _ => b.key_const[i],
        };
    }
    let bst = St { s: b, rd, l, r };
    let w = bst.probe(&kv);
    let mut found = false;
    bst.each(w, |row| {
        found = bst.matches(row, &kv);
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
    st.each(w, |r| {
        if !st.matches(r, &kv) {
            return true;
        }
        let all = (0..nb).all(|k| witness_probe(&*j.bs.add(k), st.rd, st.l, t, s.cap_in, st, r, j.w[k]));
        if all {
            found = 1;
        }
        !all
    });
    *b.c.add(t as usize) = found;
}

unsafe fn row_scatter_one(st: &St, j: &Job, b: &Bufs, t: u32) {
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
    let a = (*offsets.add(t as usize)).wrapping_sub(o0);
    if (*offsets.add(t as usize + 1)).wrapping_sub(o0) <= a || a >= s.cap_out {
        return;
    }
    for k in 0..s.nslots {
        col_put(o, k, s.cap_out, a, st.slot(k, t));
    }
    let op = s.agg[0] & 15;
    if op != 3 && op != 5 {
        col_put(o, s.bind_slot[0], s.cap_out, a, *b.v.add(t as usize));
    }
}

unsafe fn rows_copy_one(rd: Reads, src: *const u8, dst: *mut u8, totals: *const u32, c: &Copy, t: u32) {
    let n = if c.count_slot == NONE { c.n } else { (*totals.add(c.count_slot as usize)).min(c.n) };
    if t >= n {
        return;
    }
    let base = if c.flags & 1 != 0 { *c.off } else { 0 };
    let grown = if c.flags & 2 != 0 { rd.rows_len(dst, c.dst_rows_at) } else { 0 };
    let a = c.dst_off.wrapping_add(base).wrapping_add(grown).wrapping_add(t);
    let mut memo = NONE;
    for k in 0..c.ncols.min(16) as usize {
        let from = c.src[k];
        let v = if from == NONE {
            c.konst[k]
        } else if c.r_view != 0 {
            vread(rd, c.r_view, c.v_dir, t, from, &mut memo)
        } else if c.src_at == 0 {
            col(src as *const i64, from, c.src_stride, t)
        } else {
            rd.rel_get(src, c.src_at, c.src_from.wrapping_add(t), from) as i64
        };
        if c.dst_rows_at == 0 {
            col_put(dst as *mut i64, k as u32, c.dst_stride, a, v);
        } else {
            rd.rows_put(dst, c.dst_rows_at, a, k as u32, v as u64);
        }
    }
}

unsafe fn stage_map_one(s: &Stage, bb: *mut i64, totals: *const u32, t: u32) {
    let n = (*totals.add(s.in_slot as usize)).min(s.cap_in);
    if t >= n {
        return;
    }
    let st = St { s, rd: Reads(core::ptr::null()), l: bb, r: core::ptr::null() };
    for b in 0..s.nbinds.min(16) as usize {
        let k = s.bind_idx[b] as usize;
        let v = if s.bind_kind[b] == 1 {
            st.parval(k as u32, t)
        } else {
            let v = col(bb, s.col_idx[k], s.cap_in, t);
            if s.col_doc[k] != 0 { v >> 32 } else { v }
        };
        col_put(bb, s.bind_slot[b], s.cap_in, t, v);
    }
}

unsafe fn scan_count_one(st: &St, j: &Job, b: &Bufs, r: u32) {
    let s = st.s;
    let counts = b.c;
    if r > s.nr {
        return;
    }
    if r >= st.rows() || *j.totals.add(s.in_slot as usize) == 0 {
        *counts.add(r as usize) = 0;
        return;
    }
    let mut kv = [0i64; 8];
    for (i, k) in kv.iter_mut().enumerate().take(s.nkeys.min(8) as usize) {
        *k = if s.key_kind[i] == 1 { col(st.l, s.key_slot[i], s.cap_in, 0) } else { s.key_const[i] };
    }
    let id = s.r_from.wrapping_add(r);
    let mut c = 0;
    if st.matches(id, &kv) {
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

unsafe fn scan_scatter_one(st: &St, j: &Job, b: &Bufs, r: u32) {
    let s = st.s;
    let offsets = b.c;
    let o0 = *offsets;
    let rows = st.rows();
    if r == 0 {
        *j.totals.add(s.out_slot as usize) = (*offsets.add(rows as usize)).wrapping_sub(o0);
    }
    if r >= rows {
        return;
    }
    let a = (*offsets.add(r as usize)).wrapping_sub(o0);
    if (*offsets.add(r as usize + 1)).wrapping_sub(o0) > a && a < s.cap_out {
        st.write_row(0, s.r_from.wrapping_add(r), b.o, a);
    }
}

// ---- A head's distinct rows. ----

unsafe fn same_row(d: &Dedup, t: *const i64, a: u32, b: u32) -> bool {
    d.cols[..d.ncols.min(16) as usize].iter().all(|&c| col(t, c, d.cap, a) == col(t, c, d.cap, b))
}

unsafe fn row_hash(rd: Reads, d: &Dedup, t: *const i64, x: u32) -> u32 {
    let h = d.cols[..d.ncols.min(16) as usize].iter().fold(KEY_SEED, |h, &c| rd.key_mix(h, col(t, c, d.cap, x)));
    (h as u32) & d.mask
}

unsafe fn dedup_claim_one(rd: Reads, t: *const i64, totals: *const u32, p: *mut u32, d: &Dedup, x: u32) {
    if x >= (*totals.add(d.in_slot as usize)).min(d.cap) {
        return;
    }
    let mut a = row_hash(rd, d, t, x);
    for _ in 0..=d.mask {
        let place = at(p.add(a as usize));
        match place.compare_exchange(NONE, x, SeqCst, SeqCst) {
            Ok(_) => return,
            Err(seen) if same_row(d, t, seen, x) => {
                place.fetch_min(x, SeqCst);
                return;
            }
            Err(_) => a = (a + 1) & d.mask,
        }
    }
}

unsafe fn dedup_mark_one(rd: Reads, t: *const i64, totals: *const u32, p: *const u32, counts: *mut u32, d: &Dedup, x: u32) {
    if x > d.cap {
        return;
    }
    if x >= (*totals.add(d.in_slot as usize)).min(d.cap) {
        *counts.add(x as usize) = 0;
        return;
    }
    let mut a = row_hash(rd, d, t, x);
    let mut keep = 0;
    for _ in 0..=d.mask {
        let y = *p.add(a as usize);
        if y == NONE {
            break;
        }
        if y == x {
            keep = 1;
            break;
        }
        if same_row(d, t, y, x) {
            break;
        }
        a = (a + 1) & d.mask;
    }
    *counts.add(x as usize) = keep;
}

unsafe fn dedup_scatter_one(t: *const i64, totals: *mut u32, offsets: *const u32, o: *mut i64, d: &Dedup, x: u32) {
    let o0 = *offsets;
    let n = (*totals.add(d.in_slot as usize)).min(d.cap);
    if x == 0 {
        *totals.add(d.out_slot as usize) = (*offsets.add(n as usize)).wrapping_sub(o0);
    }
    if x >= n {
        return;
    }
    let a = (*offsets.add(x as usize)).wrapping_sub(o0);
    if (*offsets.add(x as usize + 1)).wrapping_sub(o0) == a || a >= d.ocap {
        return;
    }
    for c in 0..d.ncols.min(16) {
        col_put(o, c, d.ocap, a, col(t, d.cols[c as usize], d.cap, x));
    }
}

// ---- Indexes built on the device. ----

unsafe fn ix_rows(rd: Reads, bb: *const u8, p: &IxP) -> u32 {
    if p.ncols & IX_LIVE == 0 {
        return p.n;
    }
    let len = rd.rel_len(bb, p.at);
    if len > p.from { (len - p.from).min(p.n) } else { 0 }
}

unsafe fn ix_key_one(rd: Reads, bb: *const u8, out: *mut u32, places: *mut u32, p: &IxP, kb: u32, eb: u32, t: u32) {
    if t >= ix_rows(rd, bb, p) {
        return;
    }
    *places.add((eb + t) as usize) = eb + t;
    let mut memo = NONE;
    let mut h = KEY_SEED;
    for i in 0..(p.ncols & !IX_LIVE).min(8) as usize {
        let v = if p.kind != 0 { vread(rd, p.kind as u32, p.dir, t, p.cols[i], &mut memo) } else { rd.rel_get(bb, p.at, p.from.wrapping_add(t), p.cols[i]) as i64 };
        h = rd.key_mix(h, v);
    }
    *out.add((eb + t) as usize) = kb.wrapping_add((h & p.mask) as u32);
}

unsafe fn ix_place_one(rd: Reads, bb: *const u8, keys: *const u32, places: *const u32, offs: *mut u32, rows: *mut u32, p: &IxP, kb: u32, eb: u32, t: u32) {
    let n = ix_rows(rd, bb, p);
    let cap = (p.mask as u32).wrapping_add(1);
    if n == 0 && t == 0 {
        for x in 0..=cap {
            *offs.add(x as usize) = 0;
        }
    }
    if t >= n {
        return;
    }
    *rows.add(t as usize) = p.from.wrapping_add(*places.add((eb + t) as usize)).wrapping_sub(eb);
    let b = (*keys.add((eb + t) as usize)).wrapping_sub(kb);
    let from = if t == 0 { 0 } else { (*keys.add((eb + t - 1) as usize)).wrapping_sub(kb).wrapping_add(1) };
    for x in from..=b {
        *offs.add(x as usize) = t;
    }
    if t == n - 1 {
        for x in b + 1..=cap {
            *offs.add(x as usize) = n;
        }
    }
}

// ---- A lattice relation's rounds (decision 0124). ----

unsafe fn lat_r(rd: Reads, r: *const u8, p: &LatP, row: u32, c: u32) -> i64 {
    rd.rel_get(r, p.r_at, row, c) as i64
}

unsafe fn lat_put(rd: Reads, r: *const u8, p: &LatP, row: u32, c: u32, v: i64) {
    rd.rows_put(r, p.r_rows_at, row, c, v as u64);
}

unsafe fn lat_key(u: *const i64, p: &LatP, x: u32) -> [i64; 8] {
    let mut key = [0i64; 8];
    for c in 0..p.nkeys.min(8) {
        key[c as usize] = col(u, c, p.n, x);
    }
    key
}

unsafe fn lat_hash(rd: Reads, key: &[i64; 8], nk: u32, mask: u32) -> u32 {
    (rd.key_hash(&key[..nk.min(8) as usize]) as u32) & mask
}

unsafe fn lat_same(u: *const i64, stride: u32, a: u32, b: u32, nk: u32) -> bool {
    (0..nk.min(8)).all(|c| col(u, c, stride, a) == col(u, c, stride, b))
}

/// Its rows: `n`, or as many as an earlier phase's word says (`counted`).
unsafe fn lat_rows(p: &LatP, cnt: *const u32) -> u32 {
    if p.counted & 1 != 0 { (*cnt).min(p.n) } else { p.n }
}

unsafe fn lat_clear_one(pl: *mut u32, best: *mut u32, flags: *mut u32, p: &LatP, i: u32) {
    if i > p.pmask {
        return;
    }
    *pl.add(i as usize) = NONE;
    *best.add(i as usize) = NONE;
    *flags.add(i as usize) = 0;
}

unsafe fn lat_hclear_one(hb: *mut u8, p: &LatP, i: u32) {
    if i > p.hmask {
        return;
    }
    let h = hb.add(p.h_at as usize) as *mut [u32; 3];
    *h.add(i as usize) = [NONE, 0, 0];
}

unsafe fn lat_rehash_one(rd: Reads, r: *const u8, ob: *const u8, hb: *mut u8, p: &LatP, s: u32) {
    if s > p.omask {
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
        key[c as usize] = lat_r(rd, r, p, t, c);
    }
    let mut a = lat_hash(rd, &key, p.nkeys, p.hmask);
    for _ in 0..=p.hmask {
        if at(h.add(3 * a as usize)).compare_exchange(NONE, t, SeqCst, SeqCst).is_ok() {
            at(h.add(3 * a as usize + 1)).store(*old.add(3 * s as usize + 1), SeqCst);
            at(h.add(3 * a as usize + 2)).store(*old.add(3 * s as usize + 2), SeqCst);
            return;
        }
        a = (a + 1) & p.hmask;
    }
}

unsafe fn lat_claim_one(rd: Reads, u: *const i64, pl: *mut u32, cnt: *const u32, p: &LatP, x: u32) {
    if x >= lat_rows(p, cnt) {
        return;
    }
    let mut a = lat_hash(rd, &lat_key(u, p, x), p.nkeys, p.pmask);
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
unsafe fn lat_place(rd: Reads, u: *const i64, pl: *const u32, p: &LatP, x: u32) -> u32 {
    let mut a = lat_hash(rd, &lat_key(u, p, x), p.nkeys, p.pmask);
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
    x.bytes(0, x.len()).lt(y.bytes(0, y.len()))
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

unsafe fn lat_best_one(rd: Reads, u: *const i64, pl: *const u32, best: *mut u32, flags: *mut u32, arena: *const u32, cnt: *const u32, p: &LatP, x: u32) {
    if x >= lat_rows(p, cnt) {
        return;
    }
    let s = lat_place(rd, u, pl, p, x);
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

unsafe fn lat_merge_one(rd: Reads, u: *const i64, pl: *const u32, best: *const u32, flags: *const u32, r: *mut u8, hb: *mut u8, d: *mut i64, counters: *mut u32, arena: *const u32, round: *const u32, pp: &LatP, s: u32) {
    let mut p = pp.clone();
    if p.counted & 2 != 0 {
        p.had = rd.rows_len(r, p.r_rows_at);
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
    let mut a = lat_hash(rd, &key, p.nkeys, p.hmask);
    let mut held = NONE;
    for _ in 0..=p.hmask {
        let hrow = at(h.add(3 * a as usize)).load(SeqCst);
        if hrow == NONE {
            break;
        }
        if hrow < p.had && (0..nk).all(|c| lat_r(rd, r, &p, hrow, c) == key[c as usize]) {
            held = hrow;
            break;
        }
        a = (a + 1) & p.hmask;
    }
    if held != NONE {
        let o = lat_r(rd, r, &p, held, p.nkeys);
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
        lat_put(rd, r, &p, held, p.nkeys, m);
        at(h.add(3 * a as usize + 1)).store(changes + 1, SeqCst);
        if p.level != 0 {
            at(h.add(3 * a as usize + 2)).store(p.level, SeqCst);
        }
        v = m;
    } else {
        let id = p.had.wrapping_add(at(counters.add(1)).fetch_add(1, SeqCst));
        for c in 0..nk {
            lat_put(rd, r, &p, id, c, key[c as usize]);
        }
        lat_put(rd, r, &p, id, p.nkeys, v);
        let mut place = lat_hash(rd, &key, p.nkeys, p.hmask);
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
    let ctl6 = at(ctl.add(6));
    let ctl7 = at(ctl.add(7));
    if !any {
        ctl6.fetch_or(16, SeqCst);
        ctl7.store(NONE, SeqCst);
    } else if cur.wrapping_add(1) > (*p).max {
        ctl6.fetch_or(4, SeqCst);
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

// ---- Scans and sorts: a tile at a time. ----

/// Tile `tile`'s sum of `n` values into `sums[tile]`.
unsafe fn scan_tile_sum(input: *const u32, sums: *mut u32, n: u32, tile: u32) {
    let (first, end) = scan_tile_range(n, tile);
    *sums.add(tile as usize) = (first..end).fold(0u32, |s, i| s.wrapping_add(*input.add(i as usize)));
}

/// Tile `tile` scanned in place from its offset; the last tile writes the total at `n`.
unsafe fn scan_tile(io: *mut u32, offsets: Option<*const u32>, n: u32, tile: u32) {
    let (first, end) = scan_tile_range(n, tile);
    let base = offsets.map_or(0, |o| *o.add(tile as usize));
    let mut run = base;
    for i in first..end {
        let v = *io.add(i as usize);
        *io.add(i as usize) = run;
        run = run.wrapping_add(v);
    }
    let tile_values = TG * SCAN_ITEMS;
    let last = if n == 0 { 0 } else { (n - 1) / tile_values };
    if tile == last {
        *io.add(n as usize) = run;
    }
}

fn scan_tile_range(n: u32, tile: u32) -> (u32, u32) {
    let size = TG * SCAN_ITEMS;
    let first = (tile as u64 * size as u64).min(n as u64) as u32;
    (first, (first as u64 + size as u64).min(n as u64) as u32)
}

/// Tile `tile`'s digit histogram, digit-major (`hist[d * tiles + tile]`).
unsafe fn radix_hist_tile(keys: *const u32, hist: *mut u32, n: u32, shift: u32, tiles: u32, tile: u32) {
    let mut h = [0u32; RADIX_DIGITS];
    let first = tile.saturating_mul(RADIX_TILE);
    for r in first..first.saturating_add(RADIX_TILE).min(n) {
        h[((*keys.add(r as usize) >> shift) & 255) as usize] += 1;
    }
    for (d, &c) in h.iter().enumerate() {
        *hist.add(d * tiles as usize + tile as usize) = c;
    }
}

/// Tile `tile`'s rows at their places: each digit's from its scanned offset, in the rows' order (a stable sort).
unsafe fn radix_scatter_tile(sp: &SortPass, n: u32, shift: u32, tiles: u32, tile: u32) {
    let mut next = [0u32; RADIX_DIGITS];
    for (d, x) in next.iter_mut().enumerate() {
        *x = *sp.hist.add(d * tiles as usize + tile as usize);
    }
    let first = tile.saturating_mul(RADIX_TILE);
    for r in first..first.saturating_add(RADIX_TILE).min(n) {
        let key = *sp.keys.add(r as usize);
        let d = ((key >> shift) & 255) as usize;
        let a = next[d] as usize;
        next[d] += 1;
        *sp.out_keys.add(a) = key;
        *sp.out_values.add(a) = *sp.values.add(r as usize);
    }
}

// ---- Text rows: a SIMD group's on a GPU, a thread's here. ----

/// Mode 10's rows of SIMD group `sg` (`row_text_one`): each row's keep bit and made text (`counts`, `V`).
unsafe fn row_text(st: &St, j: &Job, b: &Bufs, ctl: *mut u32, sg: u32) {
    let s = st.s;
    let (counts, vv, p) = (b.c, b.v, j.p);
    let pw = |i: usize| *p.add(i);
    let n = (*j.totals.add(s.in_slot as usize)).min(s.cap_in);
    let groups = pw(0);
    // The counts past its rows, its share of them a SIMD group's lanes'.
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
    let (a, d, g, vc) = (j.a, j.d, j.g, j.vc);
    let op = s.agg[0] & 15;
    let want = (s.agg[0] >> 8) & 1 != 0;
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
                    got = st.rd.verdict_get(vc, 0, test, v as u32);
                    if got == 0 {
                        let id = v as u32;
                        let len = st.rd.atoms_len(d, 0, id);
                        got = if len == NONE {
                            3
                        } else {
                            let x = Txt::raw(st.rd.atoms_at(d, 0, id) as *const u8, len);
                            let verdict = if txt_verdict(g, pw(2), pw(3), pw(4), &x) { 2 } else { 1 };
                            st.rd.verdict_put(vc, 0, test, id, verdict);
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
    let mut row = sg;
    while row < n {
        let r = row;
        row += groups;
        let mut unheld = false;
        let x = st.txt_of(r, d, a, 0, s.agg[1] & 15, &mut unheld);
        if unheld {
            counter(4);
            *vv.add(r as usize) = -1;
            *counts.add(r as usize) = 0;
            continue;
        }
        if op == 3 {
            *counts.add(r as usize) = (txt_verdict(g, pw(2), pw(3), pw(4), &x) == want) as u32;
            continue;
        }
        // A text kept by (functions P[5], symbol key 7) in the rolling cache: copied into the arena.
        let ksym = st.key_value(r, 7);
        let keeps = s.agg[0] & (1 << 9) != 0 && (0..=u32::MAX as i64).contains(&ksym);
        let ksym = ksym as u32;
        if keeps {
            let e = st.rd.text_find(vc, 0, pw(5), ksym);
            if e & 3 == 0 {
                let e = e as *const u8;
                let len = (e.add(12) as *const u32).read_unaligned().wrapping_sub(16);
                let (off, full) = arena_claim(a, len);
                if !full {
                    core::ptr::copy_nonoverlapping(e.add(16), (a as *mut u8).add(16 + off as usize), len as usize);
                }
                made(ctl, vv, counts, r, off, len, full);
                continue;
            }
        }
        let y = st.txt_of(r, d, a, 2, s.agg[1] >> 4, &mut unheld);
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
        let (off, full) = arena_claim(a, len);
        let o = (a as *mut u8).add(16 + off as usize);
        if !full {
            match make {
                Make::Cat(dot, y) => put_bytes(o, x.bytes(0, lx).chain((0..dot).map(|_| b'.')).chain(y.bytes(0, y.len()))),
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
                Make::Range(from) => put_bytes(
                    o,
                    x.bytes(from, from + len).map(|c| match f {
                        0 | 8 => c.to_ascii_lowercase(),
                        9 => c.to_ascii_uppercase(),
                        _ => c,
                    }),
                ),
            }
        }
        if keeps && !full && !unheld && len <= 4096 {
            // Made: kept for the next reader, its entry completed after its bytes.
            let e = st.rd.text_open(vc, 0, pw(5), ksym, len);
            if e & 3 == 1 {
                let e = (e & !3) as *mut u8;
                core::ptr::copy_nonoverlapping(o, e.add(16), len as usize);
                st.rd.cache_done(vc, 0, e as u64);
            }
        }
        if unheld {
            counter(4);
        }
        made(ctl, vv, counts, r, off, len, full);
    }
}

/// `len` bytes of the text arena: their offset, and whether they are past its room.
unsafe fn arena_claim(a: *mut u32, len: u32) -> (u32, bool) {
    let off = at(a).fetch_add(len, SeqCst);
    (off, off as u64 + len as u64 > at(a.add(1)).load(SeqCst) as u64)
}

/// A made text's row: its value (none where the arena was full: the phase runs again) and its keep bit.
unsafe fn made(ctl: *mut u32, vv: *mut i64, counts: *mut u32, r: u32, off: u32, len: u32, full: bool) {
    if full {
        at(ctl.add(6)).fetch_or(64, SeqCst);
    }
    *vv.add(r as usize) = if full { -1 } else { arena_value(off, len) };
    *counts.add(r as usize) = 1;
}

// ---- The program: a work queue of items, a group an item. ----

struct Prog {
    jobs: *mut Job,
    items: *const Item,
    phases: *const u32,
    segs: *const Seg,
    scans: *mut ScanLevel,
    sorts: *const SortPass,
    script: *const SizeOp,
    ctl: *mut u32,
    slots: *mut u32,
    counts: *mut u32,
    a: ProgP,
    rd: Reads,
}

impl Prog {
    unsafe fn ctl(&self, i: usize) -> &AtomicU32 {
        at(self.ctl.add(i))
    }

    unsafe fn seg_items(&self, g: *const Seg) -> u32 {
        let g = &*g;
        if g.per == 0 {
            return g.items;
        }
        let need = (*g.count as u64 + g.plus as u64).div_ceil(g.per as u64);
        need.min(g.items as u64) as u32
    }

    unsafe fn push(&self, item: u32) -> bool {
        self.ctl(2).fetch_add(1, SeqCst);
        let end = self.a.capacity;
        for _ in 0..=self.a.capacity {
            let tail = self.ctl(1).fetch_add(1, SeqCst);
            if tail >= end {
                break;
            }
            if at(self.slots.add(tail as usize)).swap(item + 1, SeqCst) != NONE {
                return true;
            }
        }
        self.ctl(3).store(1, SeqCst);
        self.ctl(2).fetch_sub(1, SeqCst);
        false
    }

    unsafe fn take(&self) -> Option<u32> {
        let mut head = self.ctl(0).load(SeqCst);
        let mut claimed = false;
        for _ in 0..TURNS {
            let tail = self.ctl(1).load(SeqCst).min(self.a.capacity);
            if head >= tail {
                return None;
            }
            match self.ctl(0).compare_exchange(head, head + 1, SeqCst, SeqCst) {
                Ok(_) => {
                    claimed = true;
                    break;
                }
                Err(seen) => head = seen,
            }
        }
        if !claimed {
            return None;
        }
        match at(self.slots.add(head as usize)).swap(NONE, SeqCst) {
            0 => None,
            v => Some(v - 1),
        }
    }

    unsafe fn push_or_flag(&self, item: u32) {
        if !self.push(item) {
            self.ctl(6).fetch_or(1, SeqCst);
        }
    }

    unsafe fn phase(&self, q: u32) -> [u32; 4] {
        core::array::from_fn(|k| *self.phases.add(4 * q as usize + k))
    }

    unsafe fn phase_reset(&self, q: u32) -> u32 {
        let [_, _, s0, ns] = self.phase(q);
        let m = (0..ns).fold(0u32, |m, g| m.wrapping_add(self.seg_items(self.segs.add((s0 + g) as usize))));
        at(self.ctl.add(10 + self.a.phases as usize + q as usize)).store(if ns == 0 { 0 } else { m.max(1) }, SeqCst);
        at(self.ctl.add(10 + q as usize)).store(0, SeqCst);
        (ns != 0 && m == 0) as u32
    }

    unsafe fn phase_push(&self, next: u32, none: u32) {
        if next == 0 {
            return;
        }
        let [first, count, s0, ns] = self.phase(next - 1);
        if ns == 0 {
            for i in 0..count {
                self.push_or_flag(first + i);
            }
        }
        for g in 0..ns {
            let seg = self.segs.add((s0 + g) as usize);
            let m = if none != 0 && g == 0 { 1 } else { self.seg_items(seg) };
            for i in 0..m {
                self.push_or_flag((*seg).first + i);
            }
        }
    }

    unsafe fn phase_texts(&self, p: u32) {
        let [first, count, s0, ns] = self.phase(p);
        for pass in 0..2 {
            let mut m = 0u32;
            for g in 0..ns.max(1) {
                let (a, n) = if ns == 0 { (first, count) } else { let seg = self.segs.add((s0 + g) as usize); ((*seg).first, self.seg_items(seg)) };
                for i in 0..n {
                    if (*self.items.add((a + i) as usize)).what >> 24 != PK_ROW_TEXT {
                        continue;
                    }
                    m += 1;
                    if pass == 1 {
                        self.push_or_flag(a + i);
                    }
                }
            }
            if pass == 0 {
                at(self.ctl.add(10 + self.a.phases as usize + p as usize)).store(m.max(1), SeqCst);
                at(self.ctl.add(10 + p as usize)).store(0, SeqCst);
            }
        }
    }

    /// Item `it`'s work, every thread of the group's: whether it is whole (a size script that parked is not).
    unsafe fn item(&self, it: Item) -> bool {
        let (kind, x, g) = (it.what >> 24, it.what & 0xff_ffff, it.group);
        if kind == PK_SIZE {
            return size_one(self.script.add(x as usize), g);
        }
        // A segment's item strides its job's rows: `t`, `t + step`, ..., below its count's.
        let (mut step, mut lim) = (0u32, 0u32);
        if it.seg != 0 {
            let seg = self.segs.add(it.seg as usize - 1);
            if (*seg).per != 0 {
                step = self.seg_items(seg) * TG;
                lim = (*(*seg).count).wrapping_add((*seg).plus);
            }
        }
        if kind == PK_SCAN_SUMS || kind == PK_SCAN_TILES {
            let lv = &*self.scans.add(x as usize);
            let n = lv.w[0];
            let input = lv.input.add(lv.w[2] as usize);
            let sums = lv.sums.add(lv.w[3] as usize);
            let tiles = n.div_ceil(TG * SCAN_ITEMS);
            let tstep = step / TG;
            let mut tile = g;
            loop {
                if kind == PK_SCAN_SUMS {
                    scan_tile_sum(input, sums, n, tile);
                } else {
                    scan_tile(input, (lv.w[1] != 0).then_some(sums as *const u32), n, tile);
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
            let (n, shift, tiles) = (sp.w[0], sp.w[1], sp.w[2]);
            if kind == PK_RADIX_HIST {
                radix_hist_tile(sp.keys, sp.hist as *mut u32, n, shift, tiles, g);
            } else {
                radix_scatter_tile(sp, n, shift, tiles, g);
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
            // A SIMD group a row: the group's SIMD groups, each its rows.
            let st = St { s: &*j.s, rd: self.rd, l: bufs.l, r: j.r };
            for sg in g * (TG / SGW)..(g + 1) * (TG / SGW) {
                row_text(&st, j, &bufs, self.ctl, sg);
            }
            return true;
        }
        for lid in 0..TG {
            let mut t = g.wrapping_mul(TG).wrapping_add(lid);
            loop {
                self.job(kind, j, &bufs, t);
                let (next, over) = t.overflowing_add(step);
                if step == 0 || next >= lim || over {
                    break;
                }
                t = next;
            }
        }
        true
    }

    /// Thread `t`'s part of job `j`, of kind `kind`.
    unsafe fn job(&self, kind: u32, j: &Job, b: &Bufs, t: u32) {
        let rd = self.rd;
        let stage = |s: *const Stage| St { s: &*s, rd, l: b.l, r: j.r };
        let lat = || &*(j.p as *const LatP);
        match kind {
            PK_COPY => rows_copy_one(rd, b.l as *const u8, b.o as *mut u8, j.totals, &*(j.p as *const Copy), t),
            PK_IX_KEY => ix_key_one(rd, b.l as *const u8, b.h, j.s_ as *mut u32, &*(j.p as *const IxP), b.kb, b.eb, t),
            PK_IX_PLACE => ix_place_one(rd, b.l as *const u8, b.v as *const u32, j.vc as *const u32, j.a, j.counters, &*(j.p as *const IxP), b.kb, b.eb, t),
            PK_DEDUP_CLEAR => {
                if t <= (*j.dd).mask {
                    *b.h.add(t as usize) = NONE;
                }
            }
            PK_DEDUP_CLAIM => dedup_claim_one(rd, b.l, j.totals, b.h, &*j.dd, t),
            PK_MAP => stage_map_one(&*j.s, b.o, j.totals, t),
            PK_STAGE_COUNT => stage_count_one(&stage(j.s), j, b, t),
            PK_SCAN_COUNT => scan_count_one(&stage(j.s), j, b, t),
            PK_WITNESS_COUNT => witness_count_one(&stage(j.s), j, b, t),
            PK_DEDUP_MARK => dedup_mark_one(rd, b.l, j.totals, b.h, b.c, &*j.dd, t),
            PK_STAGE_SCATTER => stage_scatter_one(&stage(j.ws), j, b, t),
            PK_SCAN_SCATTER => scan_scatter_one(&stage(j.s), j, b, t),
            PK_ROW_SCATTER => row_scatter_one(&stage(j.s), j, b, t),
            PK_DEDUP_SCATTER => dedup_scatter_one(b.l, j.totals, b.c, b.o, &*j.dd, t),
            PK_LAT_HCLEAR => lat_hclear_one(j.vc, lat(), t),
            PK_LAT_REHASH => lat_rehash_one(rd, j.s_, j.g, j.vc, lat(), t),
            PK_LAT_CLEAR => lat_clear_one(b.la, b.lt, b.h, lat(), t),
            PK_LAT_CLAIM => lat_claim_one(rd, b.l, b.la, j.cnt, lat(), t),
            PK_LAT_BEST => lat_best_one(rd, b.l, b.la, b.lt, b.h, b.v as *const u32, j.cnt, lat(), t),
            PK_LAT_MERGE => lat_merge_one(rd, b.l, b.la, b.lt, b.h, j.s_, j.vc, b.o, j.counters, b.v as *const u32, j.g as *const u32, lat(), t),
            PK_SUM if t == 0 => sum_one(j.p as *const SumP),
            PK_GATE => gate_one(j.p as *const GateP, self.ctl, t),
            PK_ROUND if t == 0 => round_one(j.p as *const RoundP, self.ctl),
            _ => {}
        }
    }

    /// A group's turns: items taken and run until none is left or the launch's budget is spent.
    unsafe fn run(&self) {
        let (mut next, mut none) = (0u32, 0u32);
        for turn in 0..TURNS {
            if turn == 0 {
                // A phase parked as its texts found the arena full (bit 30), or after a gate's room (bit 29).
                let pp = self.ctl(9).load(SeqCst);
                if pp >> 30 == 1 && self.ctl(9).compare_exchange(pp, 0, SeqCst, SeqCst).is_ok() {
                    self.phase_texts(pp & 0x3fff_ffff);
                }
                if pp >> 29 == 1 && self.ctl(9).compare_exchange(pp, 0, SeqCst, SeqCst).is_ok() {
                    let p = pp & 0x1fff_ffff;
                    let jump = self.ctl(7).load(SeqCst);
                    if jump != 0 {
                        self.ctl(7).store(0, SeqCst);
                    }
                    next = if jump != 0 {
                        if jump == NONE { 0 } else { jump }
                    } else if p + 1 < self.a.phases {
                        p + 2
                    } else {
                        0
                    };
                    if next != 0 {
                        none = self.phase_reset(next - 1);
                    }
                }
                self.phase_push(next, none);
            }
            // A ticket of the launch's budget, then a seed while any are left, else a continuation.
            let (mut got, mut item) = (0u32, 0u32);
            if self.ctl(8).fetch_add(1, SeqCst) < self.a.budget {
                if self.ctl(5).load(SeqCst) < self.a.seeds {
                    item = self.ctl(5).fetch_add(1, SeqCst);
                    got = (item < self.a.seeds) as u32;
                    item = item.wrapping_add(*self.phases);
                }
                if got == 0
                    && let Some(i) = self.take()
                {
                    item = i;
                    got = 2;
                }
            }
            if got == 0 {
                self.ctl(8).fetch_sub(1, SeqCst);
                return;
            }
            let it = *self.items.add(item as usize);
            let whole = self.item(it);
            (next, none) = (0, 0);
            if !whole {
                self.ctl(9).store(item + 1, SeqCst);
                self.ctl(6).fetch_or(32, SeqCst);
            } else {
                let p = it.phase;
                let finished = at(self.ctl.add(10 + p as usize)).fetch_add(1, SeqCst) + 1;
                let target = at(self.ctl.add(10 + self.a.phases as usize + p as usize)).load(SeqCst);
                let all = finished == if target != 0 { target } else { *self.phases.add(4 * p as usize + 1) };
                let flags = if all { self.ctl(6).load(SeqCst) } else { 0 };
                if flags & 128 != 0 {
                    self.ctl(6).fetch_and(!128, SeqCst);
                }
                if flags & 128 != 0 && flags & 2 == 0 {
                    // A gate's room mapped past: the program parks after the gate's phase.
                    self.ctl(9).store(0xC000_0000 | p, SeqCst);
                    self.ctl(6).fetch_or(32, SeqCst);
                } else if all && flags & 64 != 0 {
                    // Its texts found the arena full: the phase parks, and runs again once the host maps more.
                    self.ctl(6).fetch_and(!64, SeqCst);
                    self.ctl(9).store(0x8000_0000 | p, SeqCst);
                    self.ctl(6).fetch_or(32, SeqCst);
                } else {
                    let jump = if all { self.ctl(7).load(SeqCst) } else { 0 };
                    if jump != 0 {
                        self.ctl(7).store(0, SeqCst);
                    }
                    next = if jump != 0 {
                        if jump == NONE { 0 } else { jump }
                    } else if all && p + 1 < self.a.phases && self.ctl(6).load(SeqCst) & 2 == 0 {
                        p + 2
                    } else {
                        0
                    };
                    if next != 0 {
                        none = self.phase_reset(next - 1);
                    }
                }
            }
            self.phase_push(next, none);
            if got == 2 {
                self.ctl(2).fetch_sub(1, SeqCst);
            }
        }
    }
}

/// The join program's group (`join_run`): its `JOIN_TG` threads, run as one.
///
/// # Safety
///
/// `argv` holds the kernel's arguments (each the address of its bytes: a buffer's address, the `ProgP`'s bytes), its
/// buffers as the host made them; `l.reads` the compiled reads (`rerail_clif::jit::Reads::table`).
pub unsafe fn join_run(argv: *const *const u8, l: &Launch) {
    let threads = l.block[0] * l.block[1] * l.block[2];
    assert!(threads == TG, "join_run: a group of {TG} threads, not {threads}");
    assert!(!l.reads.is_null(), "join_run: the compiled reads (Launch::reads)");
    let buf = |i: usize| (*argv.add(i) as *const *mut u8).read_unaligned();
    let a = (*argv.add(10) as *const ProgP).read_unaligned();
    let prog = Prog {
        jobs: buf(0) as *mut Job,
        items: buf(1) as *const Item,
        phases: buf(2) as *const u32,
        segs: buf(3) as *const Seg,
        scans: buf(4) as *mut ScanLevel,
        sorts: buf(5) as *const SortPass,
        script: buf(6) as *const SizeOp,
        ctl: buf(7) as *mut u32,
        slots: buf(8) as *mut u32,
        counts: buf(9) as *mut u32,
        a,
        rd: Reads(l.reads),
    };
    prog.run();
}

#[cfg(test)]
mod tests {
    /// Each read the program calls is at its place in `READS`, the table's order.
    #[test]
    fn the_reads_are_at_their_places() {
        let places = [
            ("rows_len", 27),
            ("rows_put", 28),
            ("rel_get", 29),
            ("rel_dead", 30),
            ("rel_len", 31),
            ("rel_group", 32),
            ("rel_group_row", 33),
            ("rel_key_mix", 34),
            ("bloom_hash", 35),
            ("bloom_may", 37),
            ("view_node", super::VIEW_READS[0]),
            ("view_child", super::VIEW_READS[1]),
            ("view_field", super::VIEW_READS[2]),
            ("view_core", super::VIEW_READS[3]),
            ("view_line", super::VIEW_READS[4]),
            ("view_loc_line", super::VIEW_READS[5]),
            ("view_loc_end", super::VIEW_READS[6]),
            ("view_loc_col", super::VIEW_READS[7]),
            ("view_loc_docof", super::VIEW_READS[8]),
            ("view_tape", super::VIEW_READS[9]),
            ("view_depth", super::VIEW_READS[10]),
            ("view_order", super::VIEW_READS[11]),
            ("view_child_first", super::CHILD_FIRST),
            ("view_child_next", super::CHILD_NEXT),
            ("view_number", super::NUMBER),
            ("atoms_len", 65),
            ("atoms_at", 66),
            ("verdict_get", 67),
            ("verdict_put", 68),
            ("text_find", 69),
            ("text_open", 70),
            ("cache_done", 71),
            ("view_up", super::UP),
            ("view_end", super::END),
            ("dir_doc_of_row", 79),
            ("dir_doc_of_key", 80),
        ];
        for (name, at) in places {
            assert_eq!(rerail_clif::reads::READS[at].name, name, "READS[{at}]");
        }
    }
}
