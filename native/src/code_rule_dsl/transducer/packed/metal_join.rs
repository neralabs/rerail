//! A stratum's join plan on the device (decision 0095 §3, §4, step 4): binding tables, the stages that extend and
//! filter them, and relations kept on the device across a chunk's strata.
//!
//! **What runs.** A native join rule (`setfix`'s `JoinRule`) is a pipeline of steps over bindings: an atom read by
//! its bound keys, a negated atom, filters, computed slots, then the head. On the CPU the engine walks it a binding
//! at a time. Here it runs a stage at a time over every binding at once:
//!
//! - A **binding table** (`Table`) holds the rule's slots as columns of 64-bit values, `cap` rows each, column-major.
//!   Its row count is a word on the device (`totals`), written by the stage that made it.
//! - A **stage** takes a table and makes the next one, in two passes: `stage_count` gives each binding its number of
//!   rows out, a scan makes them offsets, and `stage_scatter` writes the rows at their offsets. Outputs are counted,
//!   then scattered, so their order is the bindings' order and never an atomic's. The stage's modes:
//!   - **join**: each binding's candidates in the relation, by its keys (or every row when it has none), the
//!     relation's own column tests, then the fused filters' row program (below); every passing candidate is a row;
//!   - **semi** and **anti**: whether a binding has any such candidate (an atom that binds nothing, a negation);
//!   - **filter**: the row program over the binding's own slots (comparisons, `in`, document tests).
//! - A **map** (`stage_map`) sets slots in place: `x = y`, `d = @doc(n)`.
//!
//! **Filters are the row program.** The fused filters are the CPU's own `BlockProgram` (`setfix::rowcode`), and the
//! device interprets its operations per row (`Cmp`, `And`, `Or`, `Xor`, `AndNot`, `Not`, `Const`): one meaning on
//! both backends, and one pipeline for every rule, so no rule compiles a kernel. The program's columns are a
//! candidate's columns or the binding's slots, each possibly its document (`>> 32`); its parameters are the
//! binding's slots or constants.
//!
//! **Keys.** A relation read by keys is sorted on the device by a hash of its key columns (`rel_hash`, then
//! `metal_sort`'s radix sort of 16, 24 or 32 bits, as its rows need), and a binding finds its run of equal hashes by a
//! bounded binary search; every candidate's keys are compared whole, so a collision costs a comparison, never a row.
//! A relation's upload (`DevRel`) and each of its sorts (`DevIndex`) stay on the device while the engine holds them.
//!
//! **One command buffer.** A `Batch` encodes every stage of every rule it is given into one compute encoder
//! (serial: each dispatch sees the last one's writes), and commits a new command buffer only when the launch budget
//! of one would be exceeded. A table's capacity is fixed when its stage is encoded; a stage whose rows overflow it
//! writes none past it and reports its true count, and the caller runs it again from that stage with the exact size.
//!
//! Every loop has a bound that follows from its input, and threadgroup memory is not used.

use super::Error;
use super::metal_backend::launch::LaunchScope;
use super::metal_sort::{Held, Pass, Scan, SortParts, Submitted, Width, encode_sort, groups, pipeline, queue, scratch};
use metal::{BufferRef, CommandBuffer};

/// No output or step: `Stage::last`, `Stage::passed`.
pub(in crate::code_rule_dsl) const NONE: u32 = u32::MAX;
/// The most keys, column tests, program columns, parameters, binds and operations a stage holds.
pub(in crate::code_rule_dsl) const MAX_KEYS: usize = 8;
pub(in crate::code_rule_dsl) const MAX_TESTS: usize = 8;
pub(in crate::code_rule_dsl) const MAX_COLS: usize = 16;
pub(in crate::code_rule_dsl) const MAX_PARAMS: usize = 16;
pub(in crate::code_rule_dsl) const MAX_BINDS: usize = 16;
pub(in crate::code_rule_dsl) const MAX_OPS: usize = 64;
pub(in crate::code_rule_dsl) const MAX_OUTPUTS: usize = 16;
/// The dispatches a command buffer takes before the batch starts another (the launch budget is 200).
const BUDGET: usize = 150;

/// A stage's mode.
pub(in crate::code_rule_dsl) const JOIN: u32 = 0;
pub(in crate::code_rule_dsl) const SEMI: u32 = 1;
pub(in crate::code_rule_dsl) const ANTI: u32 = 2;
pub(in crate::code_rule_dsl) const FILTER: u32 = 3;

/// A program column's or a bind's source: a candidate's column, or the binding's slot.
pub(in crate::code_rule_dsl) const RIGHT: u32 = 0;
pub(in crate::code_rule_dsl) const LEFT: u32 = 1;
/// A bind's value: a program column, a parameter, or a candidate's column as it is.
pub(in crate::code_rule_dsl) const BIND_COLUMN: u32 = 0;
pub(in crate::code_rule_dsl) const BIND_PARAM: u32 = 1;
pub(in crate::code_rule_dsl) const BIND_RIGHT: u32 = 2;
/// A key's or a parameter's source: a slot of the binding, or a constant.
pub(in crate::code_rule_dsl) const SLOT: u32 = 1;
pub(in crate::code_rule_dsl) const CONST: u32 = 0;
/// An operation's kind (`Stage::ops`, `[kind, a, b, c]`).
pub(in crate::code_rule_dsl) const OP_CMP: u32 = 0;
pub(in crate::code_rule_dsl) const OP_CONST: u32 = 1;
pub(in crate::code_rule_dsl) const OP_NOT: u32 = 2;
pub(in crate::code_rule_dsl) const OP_AND: u32 = 3;
pub(in crate::code_rule_dsl) const OP_OR: u32 = 4;
pub(in crate::code_rule_dsl) const OP_XOR: u32 = 5;
pub(in crate::code_rule_dsl) const OP_ANDNOT: u32 = 6;
/// A comparison's right side is a parameter when this bit is set in its `b`.
pub(in crate::code_rule_dsl) const RHS_PARAM: u32 = 0x8000_0000;

/// A stage's parameter block, as the kernels read it (`constant Stage&`). The engine fills what the rule says; the
/// batch fills the sizes (`cap_in`, `cap_out`, `in_slot`, `out_slot`, `nr`, `mask`, `counter`).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) struct Stage {
    pub(in crate::code_rule_dsl) key_const: [i64; MAX_KEYS],
    pub(in crate::code_rule_dsl) par_const: [i64; MAX_PARAMS],
    pub(in crate::code_rule_dsl) ops: [[u32; 4]; MAX_OPS],
    pub(in crate::code_rule_dsl) mode: u32,
    pub(in crate::code_rule_dsl) nslots: u32,
    pub(in crate::code_rule_dsl) cap_in: u32,
    pub(in crate::code_rule_dsl) cap_out: u32,
    pub(in crate::code_rule_dsl) in_slot: u32,
    pub(in crate::code_rule_dsl) out_slot: u32,
    pub(in crate::code_rule_dsl) nr: u32,
    pub(in crate::code_rule_dsl) nkeys: u32,
    pub(in crate::code_rule_dsl) mask: u32,
    pub(in crate::code_rule_dsl) ntests: u32,
    pub(in crate::code_rule_dsl) nops: u32,
    pub(in crate::code_rule_dsl) nparams: u32,
    pub(in crate::code_rule_dsl) nbinds: u32,
    pub(in crate::code_rule_dsl) npassed: u32,
    pub(in crate::code_rule_dsl) last: u32,
    pub(in crate::code_rule_dsl) counter: u32,
    pub(in crate::code_rule_dsl) key_kind: [u32; MAX_KEYS],
    pub(in crate::code_rule_dsl) key_slot: [u32; MAX_KEYS],
    pub(in crate::code_rule_dsl) key_rcol: [u32; MAX_KEYS],
    pub(in crate::code_rule_dsl) test_a: [u32; MAX_TESTS],
    pub(in crate::code_rule_dsl) test_b: [u32; MAX_TESTS],
    pub(in crate::code_rule_dsl) col_kind: [u32; MAX_COLS],
    pub(in crate::code_rule_dsl) col_idx: [u32; MAX_COLS],
    pub(in crate::code_rule_dsl) col_doc: [u32; MAX_COLS],
    pub(in crate::code_rule_dsl) par_kind: [u32; MAX_PARAMS],
    pub(in crate::code_rule_dsl) par_slot: [u32; MAX_PARAMS],
    pub(in crate::code_rule_dsl) par_doc: [u32; MAX_PARAMS],
    pub(in crate::code_rule_dsl) bind_slot: [u32; MAX_BINDS],
    pub(in crate::code_rule_dsl) bind_kind: [u32; MAX_BINDS],
    pub(in crate::code_rule_dsl) bind_idx: [u32; MAX_BINDS],
    pub(in crate::code_rule_dsl) passed: [u32; MAX_OUTPUTS],
    pub(in crate::code_rule_dsl) outputs: [u32; MAX_OUTPUTS],
    pub(in crate::code_rule_dsl) noutputs: u32,
    pub(in crate::code_rule_dsl) ncols: u32,
    pub(in crate::code_rule_dsl) pad: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<Stage>() == 2160 && std::mem::size_of::<Stage>() % 16 == 0);

impl Default for Stage {
    fn default() -> Self {
        // SAFETY: every field is an integer, for which all zeros is a value.
        let mut s: Stage = unsafe { std::mem::zeroed() };
        s.last = NONE;
        s.passed = [NONE; MAX_OUTPUTS];
        s
    }
}

impl Stage {
    fn bytes(&self) -> &[u8] {
        // SAFETY: `Stage` is `repr(C)` integers with no padding (asserted above), so its bytes are initialized.
        unsafe { std::slice::from_raw_parts((self as *const Stage).cast::<u8>(), std::mem::size_of::<Stage>()) }
    }
}

const SOURCE: &str = r#"#include <metal_stdlib>
using namespace metal;

struct Stage {
    long key_const[8];
    long par_const[16];
    uint4 ops[64];
    uint mode, nslots, cap_in, cap_out, in_slot, out_slot, nr, nkeys, mask, ntests, nops, nparams, nbinds, npassed, last, counter;
    uint key_kind[8]; uint key_slot[8]; uint key_rcol[8];
    uint test_a[8]; uint test_b[8];
    uint col_kind[16]; uint col_idx[16]; uint col_doc[16];
    uint par_kind[16]; uint par_slot[16]; uint par_doc[16];
    uint bind_slot[16]; uint bind_kind[16]; uint bind_idx[16];
    uint passed[16]; uint outputs[16];
    uint noutputs; uint ncols; uint pad[2];
};

constant uint NONE = 0xffffffffu;

static inline uint hash_keys(thread const long* kv, uint n, uint mask) {
    ulong h = 0x9E3779B97F4A7C15ul;
    for (uint i = 0; i < n && i < 8; i++) { h ^= ulong(kv[i]); h *= 0xff51afd7ed558ccdul; h ^= h >> 33; }
    return uint(h) & mask;
}

static inline long lane_of(long v, uint w) {
    return w == 16 ? long(short(v)) : (w == 32 ? long(int(v)) : v);
}

static inline long colval(constant Stage& s, uint k, device const long* L, uint t, device const long* R, uint r) {
    long v = s.col_kind[k] == 0 ? R[ulong(s.col_idx[k]) * s.nr + r] : L[ulong(s.col_idx[k]) * s.cap_in + t];
    return s.col_doc[k] != 0 ? (v >> 32) : v;
}

static inline long parval(constant Stage& s, uint k, device const long* L, uint t) {
    long v = s.par_kind[k] == 0 ? s.par_const[k] : L[ulong(s.par_slot[k]) * s.cap_in + t];
    return s.par_doc[k] != 0 ? (v >> 32) : v;
}

// The row program's operations for one row: bit x of the result is operation x's value.
static inline ulong eval(constant Stage& s, device const long* L, uint t, device const long* R, uint r) {
    ulong v = 0;
    for (uint x = 0; x < s.nops && x < 64; x++) {
        uint4 o = s.ops[x];
        bool b = false;
        if (o.x == 0) {
            uint w = (o.w >> 8) & 255;
            long a = lane_of(colval(s, o.y, L, t, R, r), w);
            long c = (o.z & 0x80000000u) != 0 ? parval(s, o.z & 0xffffu, L, t) : colval(s, o.z, L, t, R, r);
            c = lane_of(c, w);
            uint op = o.w & 255;
            b = op == 0 ? a == c : (op == 1 ? a != c : (op == 2 ? a < c : a <= c));
        } else if (o.x == 1) {
            b = o.y != 0;
        } else if (o.x == 2) {
            b = ((v >> o.y) & 1) == 0;
        } else if (o.x == 3) {
            b = ((v >> o.y) & (v >> o.z) & 1) != 0;
        } else if (o.x == 4) {
            b = (((v >> o.y) | (v >> o.z)) & 1) != 0;
        } else if (o.x == 5) {
            b = (((v >> o.y) ^ (v >> o.z)) & 1) != 0;
        } else {
            b = ((v >> o.y) & ~(v >> o.z) & 1) != 0;
        }
        v |= ulong(b) << x;
    }
    return v;
}

static inline bool out_bit(constant Stage& s, ulong v, uint k) {
    return ((v >> s.outputs[k]) & 1) != 0;
}

// A binding's candidates: [lo, hi) of the sorted order (or of the rows, without keys), and its key values.
static inline void candidates(constant Stage& s, device const long* L, uint t, device const uint* starts, device const uint* rkeys,
                              device const uint* nruns, thread long* kv, thread uint& lo, thread uint& hi) {
    if (s.mode == 3) { lo = 0; hi = 1; return; }
    if (s.nkeys == 0) { lo = 0; hi = s.nr; return; }
    for (uint i = 0; i < s.nkeys && i < 8; i++) kv[i] = s.key_kind[i] == 1 ? L[ulong(s.key_slot[i]) * s.cap_in + t] : s.key_const[i];
    uint h = hash_keys(kv, s.nkeys, s.mask);
    uint a = 0, b = s.nr == 0 ? 0 : nruns[0];
    uint runs = b;
    for (uint k = 0; k < 34 && a < b; k++) {
        uint mid = (a + b) / 2;
        if (rkeys[mid] < h) a = mid + 1; else b = mid;
    }
    if (a < runs && rkeys[a] == h) { lo = starts[a]; hi = starts[a + 1]; } else { lo = 0; hi = 0; }
}

// Whether candidate place q (row r) matches the binding's keys and the relation's column tests.
static inline bool matches(constant Stage& s, device const long* R, uint r, thread const long* kv) {
    for (uint i = 0; i < s.nkeys && i < 8; i++) if (R[ulong(s.key_rcol[i]) * s.nr + r] != kv[i]) return false;
    for (uint i = 0; i < s.ntests && i < 8; i++) if (R[ulong(s.test_a[i]) * s.nr + r] != R[ulong(s.test_b[i]) * s.nr + r]) return false;
    return true;
}

kernel void stage_count(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]], device const uint* perm [[buffer(2)]],
                        device const uint* starts [[buffer(3)]], device const uint* rkeys [[buffer(4)]], device const uint* nruns [[buffer(5)]],
                        device const uint* totals [[buffer(6)]], device atomic_uint* counters [[buffer(7)]], device uint* counts [[buffer(8)]],
                        constant Stage& s [[buffer(9)]], uint t [[thread_position_in_grid]]) {
    if (t > s.cap_in) return;
    uint n = min(totals[s.in_slot], s.cap_in);
    if (t >= n) { counts[t] = 0; return; }
    long kv[8];
    uint lo, hi;
    candidates(s, L, t, starts, rkeys, nruns, kv, lo, hi);
    bool keyed = s.mode != 3 && s.nkeys > 0;
    bool exists = s.mode == 1 || s.mode == 2;
    uint c = 0, p0 = 0;
    uint q_counts[16];
    for (uint m = 0; m < 16; m++) q_counts[m] = 0;
    bool found = false;
    for (uint q = lo; q < hi; q++) {
        uint r = keyed ? perm[q] : q;
        if (s.mode != 3 && !matches(s, R, r, kv)) continue;
        if (exists) { found = true; break; }
        p0++;
        ulong v = s.nops > 0 ? eval(s, L, t, R, r) : 0;
        for (uint m = 0; m < s.npassed && m < 16; m++) q_counts[m] += s.passed[m] == NONE ? 1u : uint(out_bit(s, v, s.passed[m]));
        if (s.last != NONE && !out_bit(s, v, s.last)) continue;
        c++;
    }
    if (exists) c = uint(found != (s.mode == 2));
    counts[t] = c;
    if (!exists && s.counter != NONE) {
        if (p0 > 0) atomic_fetch_add_explicit(&counters[s.counter], p0, memory_order_relaxed);
        for (uint m = 0; m < s.npassed && m < 16; m++) if (q_counts[m] > 0) atomic_fetch_add_explicit(&counters[s.counter + 1 + m], q_counts[m], memory_order_relaxed);
    }
}

static inline void write_row(constant Stage& s, device const long* L, uint t, device const long* R, uint r, device long* O, uint at) {
    for (uint k = 0; k < s.nslots; k++) O[ulong(k) * s.cap_out + at] = L[ulong(k) * s.cap_in + t];
    for (uint b = 0; b < s.nbinds && b < 16; b++) {
        uint kind = s.bind_kind[b];
        long v = kind == 0 ? colval(s, s.bind_idx[b], L, t, R, r) : (kind == 1 ? parval(s, s.bind_idx[b], L, t) : R[ulong(s.bind_idx[b]) * s.nr + r]);
        O[ulong(s.bind_slot[b]) * s.cap_out + at] = v;
    }
}

kernel void stage_scatter(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]], device const uint* perm [[buffer(2)]],
                          device const uint* starts [[buffer(3)]], device const uint* rkeys [[buffer(4)]], device const uint* nruns [[buffer(5)]],
                          device uint* totals [[buffer(6)]], device const uint* offsets [[buffer(7)]], device long* O [[buffer(8)]],
                          constant Stage& s [[buffer(9)]], uint t [[thread_position_in_grid]]) {
    if (t == 0) totals[s.out_slot] = offsets[s.cap_in];
    uint n = min(totals[s.in_slot], s.cap_in);
    if (t >= n) return;
    uint at = offsets[t];
    if (at >= s.cap_out) return;
    long kv[8];
    uint lo, hi;
    if (s.mode == 1 || s.mode == 2) {
        // The count pass decided: a row out is the binding as it is.
        if (offsets[t + 1] > at) write_row(s, L, t, R, 0, O, at);
        return;
    }
    candidates(s, L, t, starts, rkeys, nruns, kv, lo, hi);
    bool keyed = s.mode != 3 && s.nkeys > 0;
    for (uint q = lo; q < hi && at < s.cap_out; q++) {
        uint r = keyed ? perm[q] : q;
        if (s.mode != 3 && !matches(s, R, r, kv)) continue;
        if (s.last != NONE) {
            ulong v = eval(s, L, t, R, r);
            if (!out_bit(s, v, s.last)) continue;
        }
        write_row(s, L, t, R, r, O, at);
        at++;
    }
}

// Slots set in place, in order: a later one may read an earlier one.
kernel void stage_map(device long* B [[buffer(0)]], device const uint* totals [[buffer(1)]], constant Stage& s [[buffer(2)]],
                      uint t [[thread_position_in_grid]]) {
    uint n = min(totals[s.in_slot], s.cap_in);
    if (t >= n) return;
    for (uint b = 0; b < s.nbinds && b < 16; b++) {
        uint k = s.bind_idx[b];
        long v;
        if (s.bind_kind[b] == 1) {
            v = parval(s, k, B, t);
        } else {
            v = B[ulong(s.col_idx[k]) * s.cap_in + t];
            if (s.col_doc[k] != 0) v = v >> 32;
        }
        B[ulong(s.bind_slot[b]) * s.cap_in + t] = v;
    }
}

// A join from one binding (a rule's first table): a thread per candidate row, not one thread walking them all.
kernel void scan_count(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]], device atomic_uint* counters [[buffer(2)]],
                       device uint* counts [[buffer(3)]], device const uint* totals [[buffer(4)]], constant Stage& s [[buffer(5)]],
                       uint r [[thread_position_in_grid]]) {
    if (r > s.nr) return;
    if (r == s.nr || totals[s.in_slot] == 0) { counts[r] = 0; return; }
    long kv[8];
    for (uint i = 0; i < s.nkeys && i < 8; i++) kv[i] = s.key_kind[i] == 1 ? L[ulong(s.key_slot[i]) * s.cap_in] : s.key_const[i];
    uint c = 0;
    if (matches(s, R, r, kv)) {
        ulong v = s.nops > 0 ? eval(s, L, 0, R, r) : 0;
        if (s.counter != NONE) {
            atomic_fetch_add_explicit(&counters[s.counter], 1u, memory_order_relaxed);
            for (uint m = 0; m < s.npassed && m < 16; m++) {
                if (s.passed[m] == NONE || out_bit(s, v, s.passed[m])) atomic_fetch_add_explicit(&counters[s.counter + 1 + m], 1u, memory_order_relaxed);
            }
        }
        c = uint(s.last == NONE || out_bit(s, v, s.last));
    }
    counts[r] = c;
}

kernel void scan_scatter(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]], device uint* totals [[buffer(2)]],
                         device const uint* offsets [[buffer(3)]], device long* O [[buffer(4)]], constant Stage& s [[buffer(5)]],
                         uint r [[thread_position_in_grid]]) {
    if (r == 0) totals[s.out_slot] = offsets[s.nr];
    if (r >= s.nr) return;
    uint at = offsets[r];
    if (offsets[r + 1] > at && at < s.cap_out) write_row(s, L, 0, R, r, O, at);
}

// A rule's head rows made distinct: each row claims a place in an open-addressed table by its columns' hash (the
// least row index of rows equal to it wins), and a row is kept when it holds its place. The kept rows keep their order.
struct Dedup { uint cap; uint in_slot; uint out_slot; uint ncols; uint mask; uint cols[16]; };

static inline bool same_row(constant Dedup& d, device const long* T, uint a, uint b) {
    for (uint c = 0; c < d.ncols && c < 16; c++) if (T[ulong(d.cols[c]) * d.cap + a] != T[ulong(d.cols[c]) * d.cap + b]) return false;
    return true;
}

static inline uint row_hash(constant Dedup& d, device const long* T, uint x) {
    ulong h = 0x9E3779B97F4A7C15ul;
    for (uint c = 0; c < d.ncols && c < 16; c++) { h ^= ulong(T[ulong(d.cols[c]) * d.cap + x]); h *= 0xff51afd7ed558ccdul; h ^= h >> 33; }
    return uint(h) & d.mask;
}

kernel void dedup_clear(device uint* P [[buffer(0)]], constant Dedup& d [[buffer(1)]], uint i [[thread_position_in_grid]]) {
    if (i <= d.mask) P[i] = 0xffffffffu;
}

kernel void dedup_claim(device const long* T [[buffer(0)]], device const uint* totals [[buffer(1)]], device atomic_uint* P [[buffer(2)]],
                        constant Dedup& d [[buffer(3)]], uint x [[thread_position_in_grid]]) {
    uint n = min(totals[d.in_slot], d.cap);
    if (x >= n) return;
    uint at = row_hash(d, T, x);
    // Bounded: a place at a time, and a weak exchange's spurious failure tries the place again.
    for (uint k = 0; k < 2 * (d.mask + 1); k++) {
        uint seen = 0xffffffffu;
        if (atomic_compare_exchange_weak_explicit(&P[at], &seen, x, memory_order_relaxed, memory_order_relaxed)) return;
        if (seen == 0xffffffffu) continue;
        if (same_row(d, T, seen, x)) { atomic_fetch_min_explicit(&P[at], x, memory_order_relaxed); return; }
        at = (at + 1) & d.mask;
    }
}

kernel void dedup_mark(device const long* T [[buffer(0)]], device const uint* totals [[buffer(1)]], device const uint* P [[buffer(2)]],
                       device uint* counts [[buffer(3)]], constant Dedup& d [[buffer(4)]], uint x [[thread_position_in_grid]]) {
    if (x > d.cap) return;
    uint n = min(totals[d.in_slot], d.cap);
    if (x >= n) { counts[x] = 0; return; }
    uint at = row_hash(d, T, x);
    uint keep = 0;
    for (uint k = 0; k <= d.mask; k++) {
        uint y = P[at];
        if (y == 0xffffffffu) break;
        if (y == x) { keep = 1; break; }
        if (same_row(d, T, y, x)) break;
        at = (at + 1) & d.mask;
    }
    counts[x] = keep;
}

kernel void dedup_scatter(device const long* T [[buffer(0)]], device uint* totals [[buffer(1)]], device const uint* offsets [[buffer(2)]],
                          device long* O [[buffer(3)]], constant Dedup& d [[buffer(4)]], uint x [[thread_position_in_grid]]) {
    if (x == 0) totals[d.out_slot] = offsets[d.cap];
    if (x >= d.cap) return;
    uint at = offsets[x];
    if (offsets[x + 1] == at) return;
    for (uint c = 0; c < d.ncols && c < 16; c++) O[ulong(c) * d.cap + at] = T[ulong(d.cols[c]) * d.cap + x];
}

// A relation's rows' hash of its key columns, for its sort.
kernel void rel_hash(device const long* R [[buffer(0)]], device uint* out [[buffer(1)]], constant uint* p [[buffer(2)]],
                     uint t [[thread_position_in_grid]]) {
    uint nr = p[0];
    if (t >= nr) return;
    long kv[8];
    for (uint i = 0; i < p[1] && i < 8; i++) kv[i] = R[ulong(p[3 + i]) * nr + t];
    out[t] = hash_keys(kv, p[1], p[2]);
}
"#;

/// Whether this machine has the Metal device the stages run on.
pub(in crate::code_rule_dsl) fn available() -> bool {
    super::metal_sort::device().is_ok()
}

/// The hash the device sorts and probes by, on the host (a check of the kernels').
pub(in crate::code_rule_dsl) fn hash_keys(kv: &[i64], mask: u32) -> u32 {
    let mut h: u64 = 0x9E37_79B9_7F4A_7C15;
    for &v in kv.iter().take(MAX_KEYS) {
        h ^= v as u64;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
    }
    (h as u32) & mask
}

/// A relation's columns on the device: `rows` 64-bit values each, column-major.
pub(in crate::code_rule_dsl) struct DevRel {
    buffer: Held,
    pub(in crate::code_rule_dsl) rows: usize,
    pub(in crate::code_rule_dsl) cols: usize,
}

impl DevRel {
    /// A relation of `cols` columns and `rows` rows, its values written by `fill` straight into the device's buffer
    /// (column `c` at `[c * rows .. (c + 1) * rows]`).
    pub(in crate::code_rule_dsl) fn upload(cols: usize, rows: usize, fill: impl FnOnce(&mut [i64])) -> Result<DevRel, Error> {
        let buffer = Held::new((8 * cols * rows).max(8))?;
        // SAFETY: the buffer holds at least `cols * rows` values, and nothing else refers to it yet.
        let values = unsafe { std::slice::from_raw_parts_mut(buffer.buffer.contents().cast::<i64>(), cols * rows) };
        fill(values);
        Ok(DevRel { buffer, rows, cols })
    }

    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        8 * self.cols * self.rows
    }
}

/// A relation sorted on the device by the hash of key columns: the sorted order (`perm`), the runs of equal hashes
/// and their count, and the hash's mask.
pub(in crate::code_rule_dsl) struct DevIndex {
    parts: SortParts,
    pub(in crate::code_rule_dsl) mask: u32,
    pub(in crate::code_rule_dsl) rows: usize,
}

impl DevIndex {
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        4 * self.rows + 8 * (self.rows + 1)
    }
}

/// The hash bits for a relation of `rows` rows: about 2^8 times its rows, so few runs mix keys, in whole bytes (each
/// byte one pass of the sort).
fn hash_bits(rows: usize) -> u32 {
    let need = usize::BITS - rows.max(1).leading_zeros() + 8;
    need.clamp(16, 32).div_ceil(8) * 8
}

/// A binding table on the device: `slots` columns of `cap` 64-bit values, column-major.
pub(in crate::code_rule_dsl) struct Table {
    buffer: Held,
    pub(in crate::code_rule_dsl) cap: usize,
    pub(in crate::code_rule_dsl) slots: usize,
}

impl Table {
    pub(in crate::code_rule_dsl) fn new(slots: usize, cap: usize) -> Result<Table, Error> {
        Ok(Table { buffer: Held::new((8 * slots.max(1) * cap.max(1)).max(8))?, cap: cap.max(1), slots })
    }

    /// The table's one row `values` (a rule's first table: its preset slots).
    pub(in crate::code_rule_dsl) fn one(slots: usize, values: &[i64]) -> Result<Table, Error> {
        let t = Table::new(slots, 1)?;
        // SAFETY: the buffer holds at least `slots.max(1)` values, and nothing else refers to it yet.
        let out = unsafe { std::slice::from_raw_parts_mut(t.buffer.buffer.contents().cast::<i64>(), slots.max(1)) };
        out.fill(0);
        out[..values.len()].copy_from_slice(values);
        Ok(t)
    }

    /// Slot `slot`'s first `n` values, once the command buffer that wrote them completed.
    pub(in crate::code_rule_dsl) fn column(&self, slot: usize, n: usize) -> &[i64] {
        // SAFETY: the table holds `cap` values per slot and `n <= cap`; the writer completed.
        unsafe { std::slice::from_raw_parts(self.buffer.buffer.contents().cast::<i64>().add(slot * self.cap), n.min(self.cap)) }
    }

    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        8 * self.slots.max(1) * self.cap
    }
}

/// Words the host writes before a submission and reads after it: a rule's row counts per table (`totals`) and its
/// fused steps' counts (`counters`).
pub(in crate::code_rule_dsl) struct Words {
    buffer: Held,
    pub(in crate::code_rule_dsl) len: usize,
}

impl Words {
    pub(in crate::code_rule_dsl) fn zeroed(len: usize) -> Result<Words, Error> {
        let buffer = Held::new((4 * len).max(4))?;
        let w = Words { buffer, len };
        w.slice_mut().fill(0);
        Ok(w)
    }

    fn slice_mut(&self) -> &mut [u32] {
        // SAFETY: `len` words; the host writes them only while no command buffer that reads them runs.
        unsafe { std::slice::from_raw_parts_mut(self.buffer.buffer.contents().cast::<u32>(), self.len) }
    }

    pub(in crate::code_rule_dsl) fn set(&self, i: usize, v: u32) {
        self.slice_mut()[i] = v;
    }

    /// Word `i`, once the command buffers that write it completed.
    pub(in crate::code_rule_dsl) fn get(&self, i: usize) -> u32 {
        self.slice_mut()[i]
    }
}

/// What a batch did: its command buffers, dispatches, the GPU's time on them, and the host's CPU encoding them.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::code_rule_dsl) struct Done {
    pub(in crate::code_rule_dsl) submissions: u32,
    pub(in crate::code_rule_dsl) dispatches: u32,
    pub(in crate::code_rule_dsl) gpu_seconds: f64,
}

/// Work encoded into one command buffer at a time, committed when the launch budget of one is near and at `finish`;
/// every command buffer runs on this thread's queue, in order.
pub(in crate::code_rule_dsl) struct Batch {
    open: Option<(LaunchScope, CommandBuffer)>,
    pass: Option<Pass<'static>>,
    committed: Vec<Submitted>,
    done: Done,
    dummy: Held,
    /// Buffers the encoded work reads that nothing else holds: kept until it completes (a pooled buffer is written
    /// by the host when handed out again, and Metal orders no host write after a GPU read).
    keep: Vec<Held>,
}

impl Batch {
    pub(in crate::code_rule_dsl) fn new() -> Result<Batch, Error> {
        super::metal_sort::device()?;
        Ok(Batch { open: None, pass: None, committed: Vec::new(), done: Done::default(), dummy: Held::new(64)?, keep: Vec::new() })
    }

    /// The encoder of the open command buffer, with room for `need` more dispatches (a new command buffer when not).
    fn pass(&mut self, need: usize) -> Result<&Pass<'static>, Error> {
        if self.pass.as_ref().is_some_and(|p| p.dispatches() + need > BUDGET) {
            self.commit()?;
        }
        if self.pass.is_none() {
            let scope = LaunchScope::enter();
            let command = queue()?.new_command_buffer().to_owned();
            // SAFETY: the command buffer is owned by `self.open` and outlives the pass, which `commit` ends and drops
            // before the command buffer (the field order of the take there).
            let r: &'static metal::CommandBufferRef = unsafe { &*(command.as_ref() as *const metal::CommandBufferRef) };
            self.open = Some((scope, command));
            self.pass = Some(Pass::new(r));
        }
        Ok(self.pass.as_ref().expect("an open pass"))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let Some(pass) = self.pass.take() else { return Ok(()) };
        self.done.dispatches += pass.dispatches() as u32;
        pass.end();
        let (scope, command) = self.open.take().expect("an open command buffer");
        let s = Submitted::commit(&command)?;
        drop(scope);
        self.committed.push(s);
        self.done.submissions += 1;
        Ok(())
    }

    /// Commits what is open and waits for every command buffer.
    pub(in crate::code_rule_dsl) fn finish(&mut self) -> Result<Done, Error> {
        self.commit()?;
        for s in self.committed.drain(..) {
            self.done.gpu_seconds += s.wait();
        }
        self.keep.clear();
        Ok(std::mem::take(&mut self.done))
    }

    /// `rel` sorted by the hash of its columns `cols`, encoded into the batch (to be read by the stages after it).
    pub(in crate::code_rule_dsl) fn index(&mut self, rel: &DevRel, cols: &[usize]) -> Result<DevIndex, Error> {
        let n = rel.rows;
        let bits = hash_bits(n);
        let mask = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
        let hash = pipeline("join", "rel_hash", || SOURCE.to_owned())?;
        // A sort is at most 4 passes of three dispatches and two scans' levels, and a few more.
        let pass = self.pass(48)?;
        let words = Held::new((4 * n).max(4))?;
        let mut p = vec![n as u32, cols.len() as u32, mask];
        p.extend(cols.iter().map(|&c| c as u32));
        if n > 0 {
            pass.run(&hash, &[&rel.buffer.buffer, &words.buffer], &p, &[], groups(n));
        }
        let parts = encode_sort(pass, &words.buffer, Width::W32, bits, n)?;
        self.keep.push(words);
        Ok(DevIndex { parts, mask, rows: n })
    }

    /// A stage from `left` into `out`, over `right` (its relation and, read by keys, its index): count, scan,
    /// scatter. `stage`'s sizes are set here.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn stage(&mut self, mut stage: Stage, left: &Table, right: Option<(&DevRel, Option<&DevIndex>)>, in_slot: usize, out_slot: usize,
                                          totals: &Words, counters: &Words, counter: Option<usize>, out: &Table) -> Result<(), Error> {
        let count = pipeline("join", "stage_count", || SOURCE.to_owned())?;
        let scatter = pipeline("join", "stage_scatter", || SOURCE.to_owned())?;
        stage.cap_in = left.cap as u32;
        stage.cap_out = out.cap as u32;
        stage.in_slot = in_slot as u32;
        stage.out_slot = out_slot as u32;
        stage.counter = counter.map_or(NONE, |c| c as u32);
        stage.nslots = left.slots as u32;
        let (rbuf, perm, starts, rkeys, nruns) = match right {
            Some((rel, index)) => {
                stage.nr = rel.rows as u32;
                match index {
                    Some(ix) => {
                        stage.mask = ix.mask;
                        (&*rel.buffer.buffer, &*ix.parts.perm.buffer, &*ix.parts.starts.buffer, &*ix.parts.rkeys.buffer, &*ix.parts.nruns.buffer)
                    }
                    None => (&*rel.buffer.buffer, &*self.dummy.buffer, &*self.dummy.buffer, &*self.dummy.buffer, &*self.dummy.buffer),
                }
            }
            None => (&*self.dummy.buffer, &*self.dummy.buffer, &*self.dummy.buffer, &*self.dummy.buffer, &*self.dummy.buffer),
        };
        // Retained by the command buffer, which holds every buffer it binds until it completes.
        let (rbuf, perm, starts, rkeys, nruns): (metal::Buffer, metal::Buffer, metal::Buffer, metal::Buffer, metal::Buffer) =
            (rbuf.to_owned(), perm.to_owned(), starts.to_owned(), rkeys.to_owned(), nruns.to_owned());
        if left.cap == 1 && stage.mode == JOIN && right.is_some() {
            // From one binding: a thread per candidate row (`scan_count`), its rows in the relation's order.
            let scount = pipeline("join", "scan_count", || SOURCE.to_owned())?;
            let sscatter = pipeline("join", "scan_scatter", || SOURCE.to_owned())?;
            let n = stage.nr as usize + 1;
            let counts = scratch(super::metal_sort::FREE, 4 * n)?;
            let scan = Scan::new(n)?;
            let pass = self.pass(12)?;
            let lbuf: &BufferRef = &left.buffer.buffer;
            pass.run_bytes(&scount, &[lbuf, &rbuf, &counters.buffer.buffer, &counts, &totals.buffer.buffer], stage.bytes(), groups(n));
            scan.encode(pass, &counts)?;
            pass.run_bytes(&sscatter, &[lbuf, &rbuf, &totals.buffer.buffer, &counts, &out.buffer.buffer], stage.bytes(), groups(n));
            return Ok(());
        }
        let n = left.cap + 1;
        let counts = scratch(super::metal_sort::FREE, 4 * n)?;
        let scan = Scan::new(n)?;
        let pass = self.pass(12)?;
        let lbuf: &BufferRef = &left.buffer.buffer;
        pass.run_bytes(&count, &[lbuf, &rbuf, &perm, &starts, &rkeys, &nruns, &totals.buffer.buffer, &counters.buffer.buffer, &counts], stage.bytes(), groups(n));
        scan.encode(pass, &counts)?;
        pass.run_bytes(&scatter, &[lbuf, &rbuf, &perm, &starts, &rkeys, &nruns, &totals.buffer.buffer, &counts, &out.buffer.buffer], stage.bytes(), groups(n));
        Ok(())
    }

    /// `table`'s distinct rows over slots `cols` (at most 16) into `out` (a column per slot of `cols`, in that order,
    /// the same capacity), their count into `totals[out_slot]`; the first of equal rows is kept, and the order kept.
    pub(in crate::code_rule_dsl) fn dedup(&mut self, table: &Table, cols: &[usize], in_slot: usize, out_slot: usize, totals: &Words, out: &Table) -> Result<(), Error> {
        let clear = pipeline("join", "dedup_clear", || SOURCE.to_owned())?;
        let claim = pipeline("join", "dedup_claim", || SOURCE.to_owned())?;
        let mark = pipeline("join", "dedup_mark", || SOURCE.to_owned())?;
        let scatter = pipeline("join", "dedup_scatter", || SOURCE.to_owned())?;
        assert!(cols.len() <= 16 && out.cap == table.cap);
        let places = (2 * table.cap).next_power_of_two().max(16);
        let mut d = [0u32; 21];
        d[0] = table.cap as u32;
        d[1] = in_slot as u32;
        d[2] = out_slot as u32;
        d[3] = cols.len() as u32;
        d[4] = (places - 1) as u32;
        for (i, &c) in cols.iter().enumerate() {
            d[5 + i] = c as u32;
        }
        let bytes: &[u8] = unsafe { std::slice::from_raw_parts(d.as_ptr().cast::<u8>(), std::mem::size_of_val(&d)) };
        let p = Held::new(4 * places)?;
        let n = table.cap + 1;
        let counts = scratch(super::metal_sort::FREE, 4 * n)?;
        let scan = Scan::new(n)?;
        let pass = self.pass(12)?;
        let t: &BufferRef = &table.buffer.buffer;
        pass.run_bytes(&clear, &[&p.buffer], bytes, groups(places));
        pass.run_bytes(&claim, &[t, &totals.buffer.buffer, &p.buffer], bytes, groups(table.cap));
        pass.run_bytes(&mark, &[t, &totals.buffer.buffer, &p.buffer, &counts], bytes, groups(n));
        scan.encode(pass, &counts)?;
        pass.run_bytes(&scatter, &[t, &totals.buffer.buffer, &counts, &out.buffer.buffer], bytes, groups(n));
        self.keep.push(p);
        Ok(())
    }

    /// Sets slots of `table` in place (`stage`'s binds: program columns of the binding's slots, or parameters).
    pub(in crate::code_rule_dsl) fn map(&mut self, mut stage: Stage, table: &Table, in_slot: usize, totals: &Words) -> Result<(), Error> {
        let map = pipeline("join", "stage_map", || SOURCE.to_owned())?;
        stage.cap_in = table.cap as u32;
        stage.cap_out = table.cap as u32;
        stage.in_slot = in_slot as u32;
        stage.nslots = table.slots as u32;
        let pass = self.pass(1)?;
        pass.run_bytes(&map, &[&table.buffer.buffer, &totals.buffer.buffer], stage.bytes(), groups(table.cap));
        Ok(())
    }
}

impl Drop for Batch {
    /// Work still in flight holds its buffers: wait for it before they return to the pool.
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(n: usize, seed: u64, modulus: i64) -> Vec<i64> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((x >> 33) as i64).rem_euclid(modulus) + (3 << 32)
            })
            .collect()
    }

    /// A two-stage plan on the device against the same plan by nested loops: a scan of `a` (slots 0, 1), a join of
    /// `b` on slot 1 binding slot 2, keeping rows whose slot 2 is less than slot 0 (a row program over a candidate's
    /// column and the binding's slot); then an anti-join with `c` on slot 2, and a semi-join with `b` on slot 0.
    #[test]
    fn stages_on_the_device_are_the_nested_loops_rows() {
        if super::super::metal_sort::device().is_err() {
            return;
        }
        let (na, nb, nc) = (3000, 5000, 700);
        let (a0, a1) = (rows(na, 1, 4000), rows(na, 2, 300));
        let (b0, b1) = (rows(nb, 3, 300), rows(nb, 4, 4000));
        let c0 = rows(nc, 5, 4000);
        let up = |cols: &[&Vec<i64>]| {
            let n = cols[0].len();
            DevRel::upload(cols.len(), n, |v| for (k, c) in cols.iter().enumerate() { v[k * n..(k + 1) * n].copy_from_slice(c) }).unwrap()
        };
        let (ra, rb, rc) = (up(&[&a0, &a1]), up(&[&b0, &b1]), up(&[&c0]));
        let mut batch = Batch::new().unwrap();
        let ib = batch.index(&rb, &[0]).unwrap();
        let ic = batch.index(&rc, &[0]).unwrap();
        let ib1 = batch.index(&rb, &[1]).unwrap();
        let slots = 3;
        let totals = Words::zeroed(5).unwrap();
        totals.set(0, 1);
        let counters = Words::zeroed(8).unwrap();
        let t0 = Table::one(slots, &[]).unwrap();
        let t1 = Table::new(slots, na).unwrap();
        let mut scan = Stage { mode: JOIN, nbinds: 2, ..Stage::default() };
        scan.bind_slot[..2].copy_from_slice(&[0, 1]);
        scan.bind_kind[..2].copy_from_slice(&[BIND_RIGHT, BIND_RIGHT]);
        scan.bind_idx[..2].copy_from_slice(&[0, 1]);
        batch.stage(scan, &t0, Some((&ra, None)), 0, 1, &totals, &counters, None, &t1).unwrap();
        // Too small on purpose: the stage reports its true count.
        let t2 = Table::new(slots, 1000).unwrap();
        let mut join = Stage { mode: JOIN, nkeys: 1, nbinds: 1, nops: 1, ncols: 1, nparams: 1, noutputs: 1, last: 0, npassed: 1, ..Stage::default() };
        join.key_kind[0] = SLOT;
        join.key_slot[0] = 1;
        join.key_rcol[0] = 0;
        join.bind_slot[0] = 2;
        join.bind_kind[0] = BIND_RIGHT;
        join.bind_idx[0] = 1;
        join.col_kind[0] = RIGHT;
        join.col_idx[0] = 1;
        join.par_kind[0] = SLOT;
        join.par_slot[0] = 0;
        join.ops[0] = [OP_CMP, 0, RHS_PARAM, 2 | (64 << 8)];
        join.outputs[0] = 0;
        join.passed[0] = 0;
        batch.stage(join, &t1, Some((&rb, Some(&ib))), 1, 2, &totals, &counters, Some(0), &t2).unwrap();
        batch.finish().unwrap();
        // The nested loops.
        let mut want2: Vec<[i64; 3]> = Vec::new();
        let mut p0 = 0;
        for i in 0..na {
            for j in 0..nb {
                if b0[j] == a1[i] {
                    p0 += 1;
                    if b1[j] < a0[i] {
                        want2.push([a0[i], a1[i], b1[j]]);
                    }
                }
            }
        }
        assert_eq!(totals.get(1) as usize, na);
        assert_eq!(totals.get(2) as usize, want2.len());
        assert_eq!((counters.get(0), counters.get(1)), (p0, want2.len() as u32));
        assert!(want2.len() > 1000, "the test overflows the first table: {}", want2.len());
        // Again with room for every row; then the anti-join and the semi-join.
        let t2 = Table::new(slots, want2.len()).unwrap();
        batch.stage(join, &t1, Some((&rb, Some(&ib))), 1, 2, &totals, &counters, None, &t2).unwrap();
        let t3 = Table::new(slots, want2.len()).unwrap();
        let mut anti = Stage { mode: ANTI, nkeys: 1, ..Stage::default() };
        anti.key_kind[0] = SLOT;
        anti.key_slot[0] = 2;
        batch.stage(anti, &t2, Some((&rc, Some(&ic))), 2, 3, &totals, &counters, None, &t3).unwrap();
        let t4 = Table::new(slots, want2.len()).unwrap();
        let mut semi = Stage { mode: SEMI, nkeys: 1, ..Stage::default() };
        semi.key_kind[0] = SLOT;
        semi.key_slot[0] = 0;
        semi.key_rcol[0] = 1;
        batch.stage(semi, &t3, Some((&rb, Some(&ib1))), 3, 4, &totals, &counters, None, &t4).unwrap();
        batch.finish().unwrap();
        let got = |t: &Table, n: usize| (0..n).map(|r| [t.column(0, n)[r], t.column(1, n)[r], t.column(2, n)[r]]).collect::<Vec<_>>();
        assert_eq!(got(&t2, totals.get(2) as usize), want2, "the join's rows, in the bindings' order");
        let cset: std::collections::HashSet<i64> = c0.iter().copied().collect();
        let bset: std::collections::HashSet<i64> = b1.iter().copied().collect();
        let want3: Vec<[i64; 3]> = want2.iter().copied().filter(|r| !cset.contains(&r[2])).collect();
        let want4: Vec<[i64; 3]> = want3.iter().copied().filter(|r| bset.contains(&r[0])).collect();
        assert_eq!(got(&t3, totals.get(3) as usize), want3);
        assert_eq!(got(&t4, totals.get(4) as usize), want4);
        assert!(!want4.is_empty() && want4.len() < want3.len() && want3.len() < want2.len());
    }

    /// A table's rows over two of its three slots made distinct on the device: the first of equal rows, in order.
    #[test]
    fn head_rows_made_distinct_keep_the_first_of_each_in_order() {
        if super::super::metal_sort::device().is_err() {
            return;
        }
        let (cap, n) = (5000, 4321);
        let a = rows(cap, 7, 40);
        let b = rows(cap, 11, 30);
        let c = rows(cap, 13, 1000);
        let t = Table::new(3, cap).unwrap();
        // SAFETY: the table holds 3 * cap values and nothing else refers to it yet.
        let v = unsafe { std::slice::from_raw_parts_mut(t.buffer.buffer.contents().cast::<i64>(), 3 * cap) };
        v[..cap].copy_from_slice(&c);
        v[cap..2 * cap].copy_from_slice(&a);
        v[2 * cap..].copy_from_slice(&b);
        let totals = Words::zeroed(2).unwrap();
        totals.set(0, n as u32);
        let out = Table::new(2, cap).unwrap();
        let mut batch = Batch::new().unwrap();
        batch.dedup(&t, &[2, 1], 0, 1, &totals, &out).unwrap();
        batch.finish().unwrap();
        let mut seen = std::collections::HashSet::new();
        let want: Vec<(i64, i64)> = (0..n).map(|x| (b[x], a[x])).filter(|r| seen.insert(*r)).collect();
        let got_n = totals.get(1) as usize;
        let got: Vec<(i64, i64)> = (0..got_n).map(|x| (out.column(0, got_n)[x], out.column(1, got_n)[x])).collect();
        assert!(want.len() < n && want.len() > 100);
        assert_eq!(got, want);
    }

    #[test]
    fn hash_bits_grow_with_the_rows_in_whole_bytes() {
        assert_eq!(hash_bits(0), 16);
        assert_eq!(hash_bits(200), 16);
        assert_eq!(hash_bits(70_000), 32);
        assert_eq!(hash_bits(1000), 24);
    }
}
