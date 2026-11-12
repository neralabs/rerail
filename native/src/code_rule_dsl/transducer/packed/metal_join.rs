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
//! the GPU lane's radix sort (`rerail_accel::gpu::join`) of 16, 24 or 32 bits, as its rows need), and a binding finds its run of equal hashes by a
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
use super::metal_sort::queue;
use metal::{BufferRef, CommandBuffer};
use rerail_accel::gpu::join::{FREE, Held, Pass, Scan, SortParts, Submitted, Width, encode_sort, groups, pipeline, scratch};


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
/// An aggregate over the rows that match each binding's keys (`Stage::agg`): one row out per binding with a value.
pub(in crate::code_rule_dsl) const AGG: u32 = 4;
pub(in crate::code_rule_dsl) const AGG_COUNT: u32 = 0;
pub(in crate::code_rule_dsl) const AGG_SUM: u32 = 1;
pub(in crate::code_rule_dsl) const AGG_MIN: u32 = 2;
pub(in crate::code_rule_dsl) const AGG_MAX: u32 = 3;
/// A text test of a slot (`Stage::agg`: `[slot, want]`): a binding passes when its symbol's verdict bit (the right
/// relation, a bitset of 64 bits a row, by symbol) is `want`.
pub(in crate::code_rule_dsl) const TEXT: u32 = 5;
/// A function of symbols' names, each a slot or a constant (keys 0 and 1), their texts the right relation's (a
/// `DevText`), its value into `bind_slot[0]`: by `agg[0]`, `NAMES_CMP` (`str_cmp`) or `NAMES_LEN` (`strlen`).
pub(in crate::code_rule_dsl) const STRCMP: u32 = 6;
pub(in crate::code_rule_dsl) const NAMES_CMP: u32 = 0;
pub(in crate::code_rule_dsl) const NAMES_LEN: u32 = 1;
/// A literal text test of a pattern and a subject (keys 0 and 1, 2 and 3), each a symbol or a cat's text never made
/// (`OPND_*`, `agg[1]`: the pattern's kind, the subject's << 4): `agg[0]` is the want (bit 0), the test (`TEXT_*`
/// << 1), and `TEXT_SPLIT` when it is a text test (whose pattern a set separator would split).
pub(in crate::code_rule_dsl) const TEXTOP: u32 = 7;
pub(in crate::code_rule_dsl) const TEXT_EQ: u32 = 0;
pub(in crate::code_rule_dsl) const TEXT_PREFIX: u32 = 1;
pub(in crate::code_rule_dsl) const TEXT_SUFFIX: u32 = 2;
pub(in crate::code_rule_dsl) const TEXT_SPLIT: u32 = 1 << 4;
/// An operand: a symbol; `cat`'s text of two; `str_concat`'s.
pub(in crate::code_rule_dsl) const OPND_SYM: u32 = 0;
pub(in crate::code_rule_dsl) const OPND_DOT: u32 = 1;
pub(in crate::code_rule_dsl) const OPND_PLAIN: u32 = 2;
/// A text made of two names, a's and b's (keys 0 and 1) by the form `agg[1]`, into `bind_slot[0]` as a made value
/// (`MADE`): the parts' symbols, not yet a symbol (the device makes none). Its counter word is set when a part is no
/// symbol a made value holds (`MADE_PART`).
pub(in crate::code_rule_dsl) const MAKE: u32 = 8;
/// A made value: this bit, the form at `MADE_FORM`, a's symbol from bit 30, b's from bit 0.
pub(in crate::code_rule_dsl) const MADE: i64 = 1 << 62;
pub(in crate::code_rule_dsl) const MADE_FORM: u32 = 61;
/// One past the greatest part a made value holds.
pub(in crate::code_rule_dsl) const MADE_PART: i64 = 1 << 30;
/// `cat`'s form: b's name alone when a's is empty, else a's, ".", b's.
pub(in crate::code_rule_dsl) const CAT_DOT: u32 = 0;
/// `str_concat`'s form: a's name then b's.
pub(in crate::code_rule_dsl) const CAT_PLAIN: u32 = 1;
/// Mode 9: a text function's value made into the batch's text arena (`Arena`), or a range of its text, bound to
/// `bind_slot[0]`: of X (keys 0, 1, kind `agg[1]`), Y and Z (keys 2 and 4, symbols), numbers (keys 5 and 6), by
/// `agg[0]`. A value with none (the arena full, or `lower` past ASCII) sets the stage's counter word (`TEXTFN_FULL`,
/// `TEXTFN_HOST`).
pub(in crate::code_rule_dsl) const TEXTFN: u32 = 9;
pub(in crate::code_rule_dsl) const TEXTFN_LOWER: u32 = 0;
pub(in crate::code_rule_dsl) const TEXTFN_HEAD: u32 = 1;
pub(in crate::code_rule_dsl) const TEXTFN_DIR: u32 = 2;
pub(in crate::code_rule_dsl) const TEXTFN_SUBSTR: u32 = 3;
pub(in crate::code_rule_dsl) const TEXTFN_SLICE: u32 = 4;
pub(in crate::code_rule_dsl) const TEXTFN_TO_STRING: u32 = 5;
pub(in crate::code_rule_dsl) const TEXTFN_REPLACE: u32 = 6;
pub(in crate::code_rule_dsl) const TEXTFN_SQL_SUBSTR: u32 = 7;
pub(in crate::code_rule_dsl) const TEXTFN_ASCII_LOWER: u32 = 8;
pub(in crate::code_rule_dsl) const TEXTFN_ASCII_UPPER: u32 = 9;
pub(in crate::code_rule_dsl) const TEXTFN_FULL: u32 = 1;
pub(in crate::code_rule_dsl) const TEXTFN_HOST: u32 = 2;
/// An operand that is an arena text (a text function's value).
pub(in crate::code_rule_dsl) const OPND_ARENA: u32 = 3;
/// An arena text's value: the sign bit, its offset from bit 24, its length in the low 24 bits.
pub(in crate::code_rule_dsl) const ARENA_TEXT: i64 = i64::MIN;

/// A program column's or a bind's source: a candidate's column, or the binding's slot.
pub(in crate::code_rule_dsl) const RIGHT: u32 = 0;
pub(in crate::code_rule_dsl) const LEFT: u32 = 1;
/// A bind's value: a program column, a parameter, or a candidate's column as it is.
pub(in crate::code_rule_dsl) const BIND_COLUMN: u32 = 0;
pub(in crate::code_rule_dsl) const BIND_PARAM: u32 = 1;
pub(in crate::code_rule_dsl) const BIND_RIGHT: u32 = 2;
/// A bind's value computed by integer arithmetic (`Stage::arith[bind_idx]`): a row where it has no value (an
/// overflow, a zero divisor) is dropped, as the CPU's `Func` step drops it.
pub(in crate::code_rule_dsl) const BIND_ARITH: u32 = 3;
/// The most arithmetic binds a stage holds.
pub(in crate::code_rule_dsl) const MAX_ARITH: usize = 8;
/// An arithmetic bind's operation (`Stage::arith[j][0]`), and its operands' sources (`[j][3]`: bit 0 the first is a
/// parameter, else a program column; bit 1 the second).
pub(in crate::code_rule_dsl) const ARITH_ADD: u32 = 0;
pub(in crate::code_rule_dsl) const ARITH_SUB: u32 = 1;
pub(in crate::code_rule_dsl) const ARITH_MUL: u32 = 2;
pub(in crate::code_rule_dsl) const ARITH_DIV: u32 = 3;
pub(in crate::code_rule_dsl) const ARITH_MOD: u32 = 4;
pub(in crate::code_rule_dsl) const ARITH_MAX: u32 = 5;
pub(in crate::code_rule_dsl) const ARITH_MIN: u32 = 6;
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
    /// An aggregate stage's fold (mode `AGG`): `[kind | bind << 8, column]` (`AGG_*`; the column `NONE` for none).
    /// Its value's slot is `bind_slot[0]`; its counter word is set when a sum passes 64 bits.
    pub(in crate::code_rule_dsl) agg: [u32; 2],
    /// The arithmetic binds: `[operation, first operand, second operand, sources]` (`ARITH_*`).
    pub(in crate::code_rule_dsl) arith: [[u32; 4]; MAX_ARITH],
}

const _: () = assert!(std::mem::size_of::<Stage>() == 2288 && std::mem::size_of::<Stage>() % 16 == 0);

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
    uint noutputs; uint ncols; uint agg[2];
    uint4 arith[8];
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

// Arithmetic bind j's value for one row, as the CPU's checked arithmetic gives it: false where it has none (an
// overflow, a zero divisor). Wrapping operations on ulong, then the overflow tested, so nothing is undefined.
static inline bool arith(constant Stage& s, uint j, device const long* L, uint t, device const long* R, uint r, thread long& out) {
    uint4 a = s.arith[j];
    long x = (a.w & 1u) != 0 ? parval(s, a.y, L, t) : colval(s, a.y, L, t, R, r);
    long y = (a.w & 2u) != 0 ? parval(s, a.z, L, t) : colval(s, a.z, L, t, R, r);
    ulong ux = as_type<ulong>(x), uy = as_type<ulong>(y);
    bool min_by_neg = x == long(0x8000000000000000ul) && y == -1;
    bool ok = true;
    long v = 0;
    if (a.x == 0u) {
        v = as_type<long>(ux + uy);
        ok = ((x ^ v) & (y ^ v)) >= 0;
    } else if (a.x == 1u) {
        v = as_type<long>(ux - uy);
        ok = ((x ^ y) & (x ^ v)) >= 0;
    } else if (a.x == 2u) {
        v = as_type<long>(ux * uy);
        ok = mulhi(x, y) == (v >> 63);
    } else if (a.x == 3u) {
        ok = y != 0 && !min_by_neg;
        v = ok ? x / y : 0;
    } else if (a.x == 4u) {
        ok = y != 0 && !min_by_neg;
        v = ok ? x % y : 0;
    } else if (a.x == 5u) {
        v = max(x, y);
    } else {
        v = min(x, y);
    }
    out = v;
    return ok;
}

// Whether every arithmetic bind of the stage has a value for this row.
static inline bool defined(constant Stage& s, device const long* L, uint t, device const long* R, uint r) {
    for (uint b = 0; b < s.nbinds && b < 16; b++) {
        if (s.bind_kind[b] != 3) continue;
        long v = 0;
        if (!arith(s, s.bind_idx[b], L, t, R, r, v)) return false;
    }
    return true;
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

// An aggregate's value over the binding's matching rows (mode 4, `Stage::agg`): 0 with `v`, 1 when it has none (a min
// or max of no rows, or a tested value that differs), 2 when a sum passes 64 bits.
static inline uint aggregate(constant Stage& s, device const long* L, uint t, device const long* R, device const uint* perm,
                             uint lo, uint hi, thread const long* kv, thread long& v) {
    bool keyed = s.nkeys > 0;
    uint kind = s.agg[0] & 0xff;
    long cnt = 0, sum = 0, mn = 0, mx = 0;
    bool any = false, over = false;
    for (uint q = lo; q < hi; q++) {
        uint r = keyed ? perm[q] : q;
        if (!matches(s, R, r, kv)) continue;
        cnt++;
        if (s.agg[1] == NONE) continue;
        long x = R[ulong(s.agg[1]) * s.nr + r];
        long y = as_type<long>(as_type<ulong>(sum) + as_type<ulong>(x));
        if ((sum < 0) == (x < 0) && (y < 0) != (sum < 0)) over = true;
        sum = y;
        if (!any || x < mn) mn = x;
        if (!any || x > mx) mx = x;
        any = true;
    }
    if (kind == 1 && over) return 2;
    if ((kind == 2 || kind == 3) && !any) return 1;
    v = kind == 0 ? cnt : (kind == 1 ? sum : (kind == 2 ? mn : mx));
    if (((s.agg[0] >> 8) & 1) == 0 && L[ulong(s.bind_slot[0]) * s.cap_in + t] != v) return 1;
    return 0;
}

// A text buffer (`DevText`): its terms' count, their symbols ascending, each one's offset into its bytes, the bytes.
static inline uint text_count(device const long* T) { return uint(T[0]); }
static inline device const uint* text_offs(device const long* T) { return (device const uint*)(T + 1 + T[0]); }
static inline device const uchar* text_chars(device const long* T) { return (device const uchar*)(text_offs(T) + T[0] + 1); }

// Symbol `id`'s text in T (none: the empty text, as a name the store does not hold).
static inline void text_find(device const long* T, long id, thread uint& off, thread uint& len) {
    uint n = text_count(T), a = 0, b = n;
    for (uint k = 0; k < 40 && a < b; k++) { uint m = (a + b) / 2; if (T[1 + m] < id) a = m + 1; else b = m; }
    if (a < n && T[1 + a] == id) { device const uint* o = text_offs(T); off = o[a]; len = o[a + 1] - o[a]; }
    else { off = 0; len = 0; }
}

// Eight bytes of a text from `at`, big-endian, zero past `end`: a window compared at once.
static inline ulong text_word(device const uchar* p, uint at, uint end) {
    ulong w = 0;
    for (uint k = 0; k < 8; k++) w = (w << 8) | ((at + k) < end ? ulong(p[at + k]) : 0ul);
    return w;
}

// The order of two texts by their bytes (-1, 0, 1), as Rust's `str::cmp`: a word of eight bytes at a time over the
// shorter's length, then the lengths.
static inline long text_cmp(device const uchar* p, uint a, uint alen, uint b, uint blen) {
    uint m = min(alen, blen);
    for (uint at = 0; at < m; at += 8) {
        ulong x = text_word(p + a, at, m), y = text_word(p + b, at, m);
        if (x != y) return x < y ? -1 : 1;
    }
    return alen == blen ? 0 : (alen < blen ? -1 : 1);
}

// Whether `len` bytes at p+a are those at p+b: a word of eight at a time.
static inline bool text_same(device const uchar* p, uint a, uint b, uint len) {
    for (uint at = 0; at < len; at += 8) if (text_word(p + a, at, len) != text_word(p + b, at, len)) return false;
    return true;
}

// A key's value: a slot of binding t, or a constant.
static inline long key_value(constant Stage& s, device const long* L, uint t, uint k) {
    return s.key_kind[k] == 1 ? L[ulong(s.key_slot[k]) * s.cap_in + t] : s.key_const[k];
}

// The text arena (`Arena`): its first word the bytes given out (an atomic bump, counting on past the end, so a full
// one says how much was wanted), its second its capacity, its bytes from the 16th.
static inline device const uchar* arena_bytes(device atomic_uint* A) { return (device const uchar*)A + 16; }

// An arena text's value: the sign bit, its offset from bit 24, its length in the low 24 bits.
static inline long arena_value(uint off, uint len) { return long(0x8000000000000000ul | (ulong(off) << 24) | ulong(len)); }

// A text operand: a symbol's name (b's empty), a held cat's text, never made (a's name, "." when `dot`, b's name), or
// an arena text.
struct Txt { device const uchar* a; uint al; device const uchar* b; uint bl; uint dot; };

static inline uint txt_len(Txt x) { return x.al + x.dot + x.bl; }

static inline uchar txt_byte(Txt x, uint i) {
    if (i < x.al) return x.a[i];
    i -= x.al;
    if (i < x.dot) return uchar('.');
    return x.b[i - x.dot];
}

// Eight bytes of x from `at`, big-endian, zero past `end`.
static inline ulong txt_word(Txt x, uint at, uint end) {
    ulong w = 0;
    for (uint k = 0; k < 8; k++) w = (w << 8) | ((at + k) < end ? ulong(txt_byte(x, at + k)) : 0ul);
    return w;
}

// Whether n bytes of x from xs are those of y from ys: a word of eight at a time.
static inline bool txt_same(Txt x, uint xs, Txt y, uint ys, uint n) {
    for (uint at = 0; at < n; at += 8) if (txt_word(x, xs + at, xs + n) != txt_word(y, ys + at, ys + n)) return false;
    return true;
}

// Operand kind 0: key k's symbol; 1: `cat`'s text of keys k and k + 1 (b's alone when a's is empty); 2:
// `str_concat`'s; 3: key k's arena text.
static inline Txt txt_of(constant Stage& s, device const long* L, uint t, device const long* R, device atomic_uint* A, uint k, uint kind) {
    Txt x;
    x.b = text_chars(R);
    x.bl = 0;
    x.dot = 0;
    long v = key_value(s, L, t, k);
    if (kind == 3) {
        // A value with none (-1: the arena was full) reads as empty: the stage runs again.
        ulong off = (ulong(v) >> 24) & ((1ul << 39) - 1);
        uint len = uint(v & 0xffffff);
        bool ok = v != -1 && off + len <= ulong(((device const uint*)A)[1]);
        x.a = arena_bytes(A) + (ok ? off : 0);
        x.al = ok ? len : 0;
        return x;
    }
    uint off;
    text_find(R, v, off, x.al);
    x.a = text_chars(R) + off;
    if (kind != 0) {
        text_find(R, key_value(s, L, t, k + 1), off, x.bl);
        x.b = text_chars(R) + off;
        x.dot = (kind == 1 && x.al > 0) ? 1 : 0;
    }
    return x;
}

// Mode 7: a literal text test of pattern X (keys 0, 1) and subject Y (keys 2, 3), each of kind agg[1] & 15 and
// agg[1] >> 4: by op agg[0] >> 1 & 3, 0 equal, 1 Y begins with X, 2 Y ends with X; as agg[0] & 1 wants.
static inline bool textop_pass(constant Stage& s, device const long* L, uint t, device const long* R, device atomic_uint* A) {
    Txt x = txt_of(s, L, t, R, A, 0, s.agg[1] & 15), y = txt_of(s, L, t, R, A, 2, s.agg[1] >> 4);
    uint lx = txt_len(x), ly = txt_len(y);
    uint op = (s.agg[0] >> 1) & 3;
    bool r;
    if (op == 0) r = lx == ly && txt_same(x, 0, y, 0, lx);
    else if (op == 1) r = ly >= lx && txt_same(x, 0, y, 0, lx);
    else r = ly >= lx && txt_same(x, 0, y, ly - lx, lx);
    return r == ((s.agg[0] & 1) != 0);
}

// The characters of x: its bytes less the UTF-8 continuation bytes (`10xxxxxx`), counted a word at a time.
static inline uint txt_chars(Txt x) {
    uint n = txt_len(x), cont = 0;
    for (uint at = 0; at < n; at += 8) {
        ulong w = txt_word(x, at, n);
        cont += popcount(w & ~(w << 1) & 0x8080808080808080ul);
    }
    return n - cont;
}

// A function of names (mode 6), of operands X (keys 0, 1) and Y (keys 2, 3) as mode 7's (kinds agg[1] & 15 and
// agg[1] >> 4), its value into `bind_slot[0]`: by agg[0], 0 `str_cmp` (their order by bytes, a word of eight at a
// time, then their lengths, as Rust's `str::cmp`), 1 `strlen` of X (its characters).
static inline long strcmp_value(constant Stage& s, device const long* L, uint t, device const long* R, device atomic_uint* A) {
    Txt x = txt_of(s, L, t, R, A, 0, s.agg[1] & 15);
    if (s.agg[0] == 1) return long(txt_chars(x));
    uint lx = txt_len(x);
    Txt y = txt_of(s, L, t, R, A, 2, s.agg[1] >> 4);
    uint ly = txt_len(y), m = min(lx, ly);
    for (uint at = 0; at < m; at += 8) {
        ulong u = txt_word(x, at, m), v = txt_word(y, at, m);
        if (u != v) return u < v ? -1 : 1;
    }
    return lx == ly ? 0 : (lx < ly ? -1 : 1);
}

// The byte offset of character `c` of x (its length past the last).
static inline uint txt_char_at(Txt x, uint c) {
    uint n = txt_len(x), seen = 0;
    for (uint i = 0; i < n; i++) {
        if ((txt_byte(x, i) & 0xc0) != 0x80) {
            if (seen == c) return i;
            seen++;
        }
    }
    return n;
}

// Mode 9, a text function (agg[0]: `TEXTFN_*`) of X (keys 0, 1, kind agg[1] & 15), Y and Z (keys 2 and 4, symbols:
// `replace_all`'s pattern and replacement) and numbers (keys 5 and 6): its text's bytes [from, from + len) of X
// (`head`, `dir`, `substr`, `slice`), or made into the arena (`lower`, `to_string`, `replace_all`). Its value: an
// arena text; -1 and a flag in the stage's counter word (1: the arena is full, its first word then says how much
// was wanted; 2: `lower` of a name past ASCII, which the host's Unicode mapping makes) when it has none.
static inline long textfn_value(constant Stage& s, device const long* L, uint t, device const long* R, device atomic_uint* A, device atomic_uint* counters) {
    Txt x = txt_of(s, L, t, R, A, 0, s.agg[1] & 15);
    uint n = txt_len(x), f = s.agg[0];
    long n0 = key_value(s, L, t, 5), n1 = key_value(s, L, t, 6);
    // A range of X (in bytes) when the function copies one.
    uint from = 0, len = 0, made = 0;
    Txt y, z;
    char digits[24];
    if (f == 8 || f == 9) {
        // SQLite's lower and upper: ASCII alone.
        made = n;
    } else if (f == 0) {
        // lower: ASCII alone (a name past it is the host's: Unicode's mapping).
        for (uint i = 0; i < n; i++) if (txt_byte(x, i) >= 0x80) {
            if (s.counter != NONE) atomic_fetch_or_explicit(&counters[s.counter], 2u, memory_order_relaxed);
            return -1;
        }
        made = n;
    } else if (f == 1) {
        // head: up to the first ".".
        len = n;
        for (uint i = 0; i < n; i++) if (txt_byte(x, i) == uchar('.')) { len = i; break; }
    } else if (f == 2) {
        // dir: up to the last "/" (none: empty).
        for (uint i = n; i > 0; i--) if (txt_byte(x, i - 1) == uchar('/')) { len = i - 1; break; }
    } else if (f == 3 || f == 4 || f == 7) {
        long nc = long(txt_chars(x)), a, b;
        if (f != 4) {
            // substr(s, i, n): SQLite's substr(s, i + 1, n) (f 7: SQLite's own), in characters.
            long p1 = f == 3 ? n0 + 1 : n0, p2 = n1;
            bool neg = p2 < 0;
            if (neg) p2 = -p2;
            if (p1 < 0) {
                p1 += nc;
                if (p1 < 0) { p2 = max(p2 + p1, 0l); p1 = 0; }
            } else if (p1 > 0) p1 -= 1;
            else if (p2 > 0) p2 -= 1;
            if (neg) {
                p1 -= p2;
                if (p1 < 0) { p2 += p1; p1 = 0; }
            }
            a = clamp(p1, 0l, nc);
            b = clamp(p1 + p2, a, nc);
        } else {
            // slice(s, start, end): characters, negative from the end, clamped.
            a = clamp(n0 < 0 ? nc + n0 : n0, 0l, nc);
            b = clamp(n1 < 0 ? nc + n1 : n1, 0l, nc);
            if (a > b) b = a;
        }
        from = txt_char_at(x, uint(a));
        len = txt_char_at(x, uint(b)) - from;
    } else if (f == 5) {
        // to_string(n): its decimal digits.
        ulong u = n0 < 0 ? ulong(-(n0 + 1)) + 1 : ulong(n0);
        uint k = 0;
        do { digits[k++] = char('0' + u % 10); u /= 10; } while (u > 0);
        if (n0 < 0) digits[k++] = '-';
        made = k;
    } else {
        // replace_all(s, p, r) of a literal p (not empty) and r: the leftmost matches, not overlapping.
        y = txt_of(s, L, t, R, A, 2, 0);
        z = txt_of(s, L, t, R, A, 4, 0);
        uint ly = txt_len(y), lz = txt_len(z), hits = 0;
        for (uint i = 0; i + ly <= n && ly > 0;) {
            if (txt_same(x, i, y, 0, ly)) { hits++; i += ly; } else i++;
        }
        made = n - hits * ly + hits * lz;
    }
    if (f >= 1 && f <= 4 || f == 7) made = len;
    device atomic_uint* used = A;
    uint cap = ((device const uint*)A)[1];
    uint off = atomic_fetch_add_explicit(used, made, memory_order_relaxed);
    if (ulong(off) + made > cap) {
        if (s.counter != NONE) atomic_fetch_or_explicit(&counters[s.counter], 1u, memory_order_relaxed);
        return -1;
    }
    device uchar* o = (device uchar*)A + 16 + off;
    if (f == 0 || f == 8) {
        for (uint i = 0; i < n; i++) { uchar c = txt_byte(x, i); o[i] = (c >= 'A' && c <= 'Z') ? c + 32 : c; }
    } else if (f == 9) {
        for (uint i = 0; i < n; i++) { uchar c = txt_byte(x, i); o[i] = (c >= 'a' && c <= 'z') ? c - 32 : c; }
    } else if (f == 5) {
        for (uint i = 0; i < made; i++) o[i] = uchar(digits[made - 1 - i]);
    } else if (f == 6) {
        uint ly = txt_len(y), lz = txt_len(z), w = 0;
        for (uint i = 0; i < n;) {
            if (ly > 0 && i + ly <= n && txt_same(x, i, y, 0, ly)) {
                for (uint k = 0; k < lz; k++) o[w++] = txt_byte(z, k);
                i += ly;
            } else o[w++] = txt_byte(x, i++);
        }
    } else {
        for (uint i = 0; i < len; i++) o[i] = txt_byte(x, from + i);
    }
    return arena_value(off, made);
}

// Mode 8: the made value of a's and b's symbols by form agg[1] (`MADE`); -1 when a part is none a made value holds.
static inline long make_value(constant Stage& s, device const long* L, uint t) {
    long v[2];
    for (uint k = 0; k < 2; k++) v[k] = s.key_kind[k] == 1 ? L[ulong(s.key_slot[k]) * s.cap_in + t] : s.key_const[k];
    const long part = 1l << 30;
    if (v[0] < 0 || v[0] >= part || v[1] < 0 || v[1] >= part) return -1;
    return (1l << 62) | (long(s.agg[1] & 1) << 61) | (v[0] << 30) | v[1];
}

// Whether binding t's symbol in slot agg[0] has its verdict bit (mode 5), as agg[1] wants.
static inline bool text_pass(constant Stage& s, device const long* L, uint t, device const long* R) {
    long v = L[ulong(s.agg[0]) * s.cap_in + t];
    bool bit = v >= 0 && ulong(v >> 6) < ulong(s.nr) && ((R[v >> 6] >> (v & 63)) & 1) != 0;
    return bit == (s.agg[1] != 0);
}

kernel void stage_count(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]], device const uint* perm [[buffer(2)]],
                        device const uint* starts [[buffer(3)]], device const uint* rkeys [[buffer(4)]], device const uint* nruns [[buffer(5)]],
                        device const uint* totals [[buffer(6)]], device atomic_uint* counters [[buffer(7)]], device uint* counts [[buffer(8)]],
                        device atomic_uint* A [[buffer(9)]], constant Stage& s [[buffer(10)]], uint t [[thread_position_in_grid]]) {
    if (t > s.cap_in) return;
    uint n = min(totals[s.in_slot], s.cap_in);
    if (t >= n) { counts[t] = 0; return; }
    long kv[8];
    uint lo, hi;
    if (s.mode == 5) { counts[t] = uint(text_pass(s, L, t, R)); return; }
    if (s.mode == 6) { counts[t] = 1; return; }
    if (s.mode == 7) { counts[t] = uint(textop_pass(s, L, t, R, A)); return; }
    if (s.mode == 9) { counts[t] = 1; return; }
    if (s.mode == 8) {
        if (make_value(s, L, t) < 0 && s.counter != NONE) atomic_store_explicit(&counters[s.counter], 1u, memory_order_relaxed);
        counts[t] = 1;
        return;
    }
    candidates(s, L, t, starts, rkeys, nruns, kv, lo, hi);
    if (s.mode == 4) {
        long v = 0;
        uint st = aggregate(s, L, t, R, perm, lo, hi, kv, v);
        if (st == 2 && s.counter != NONE) atomic_store_explicit(&counters[s.counter], 1u, memory_order_relaxed);
        counts[t] = uint(st == 0);
        return;
    }
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
        bool ok = defined(s, L, t, R, r);
        ulong v = s.nops > 0 ? eval(s, L, t, R, r) : 0;
        for (uint m = 0; m < s.npassed && m < 16; m++) q_counts[m] += s.passed[m] == NONE ? uint(ok) : uint(ok && out_bit(s, v, s.passed[m]));
        if (!ok || (s.last != NONE && !out_bit(s, v, s.last))) continue;
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
        long v = 0;
        if (kind == 3) arith(s, s.bind_idx[b], L, t, R, r, v);
        else v = kind == 0 ? colval(s, s.bind_idx[b], L, t, R, r) : (kind == 1 ? parval(s, s.bind_idx[b], L, t) : R[ulong(s.bind_idx[b]) * s.nr + r]);
        O[ulong(s.bind_slot[b]) * s.cap_out + at] = v;
    }
}

kernel void stage_scatter(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]], device const uint* perm [[buffer(2)]],
                          device const uint* starts [[buffer(3)]], device const uint* rkeys [[buffer(4)]], device const uint* nruns [[buffer(5)]],
                          device uint* totals [[buffer(6)]], device const uint* offsets [[buffer(7)]], device long* O [[buffer(8)]],
                          device atomic_uint* A [[buffer(9)]], device atomic_uint* counters [[buffer(10)]], constant Stage& s [[buffer(11)]],
                          uint t [[thread_position_in_grid]]) {
    if (t == 0) totals[s.out_slot] = offsets[s.cap_in];
    uint n = min(totals[s.in_slot], s.cap_in);
    if (t >= n) return;
    uint at = offsets[t];
    if (at >= s.cap_out) return;
    long kv[8];
    uint lo, hi;
    if (s.mode == 5 || s.mode == 7) {
        if (offsets[t + 1] > at) write_row(s, L, t, R, 0, O, at);
        return;
    }
    if (s.mode == 6) {
        if (offsets[t + 1] <= at) return;
        write_row(s, L, t, R, 0, O, at);
        O[ulong(s.bind_slot[0]) * s.cap_out + at] = strcmp_value(s, L, t, R, A);
        return;
    }
    if (s.mode == 9) {
        if (offsets[t + 1] <= at) return;
        write_row(s, L, t, R, 0, O, at);
        O[ulong(s.bind_slot[0]) * s.cap_out + at] = textfn_value(s, L, t, R, A, counters);
        return;
    }
    if (s.mode == 8) {
        if (offsets[t + 1] <= at) return;
        write_row(s, L, t, R, 0, O, at);
        O[ulong(s.bind_slot[0]) * s.cap_out + at] = make_value(s, L, t);
        return;
    }
    if (s.mode == 4) {
        if (offsets[t + 1] <= at) return;
        candidates(s, L, t, starts, rkeys, nruns, kv, lo, hi);
        long v = 0;
        aggregate(s, L, t, R, perm, lo, hi, kv, v);
        write_row(s, L, t, R, 0, O, at);
        if ((s.agg[0] >> 8) & 1) O[ulong(s.bind_slot[0]) * s.cap_out + at] = v;
        return;
    }
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
        if (!defined(s, L, t, R, r)) continue;
        if (s.last != NONE) {
            ulong v = eval(s, L, t, R, r);
            if (!out_bit(s, v, s.last)) continue;
        }
        write_row(s, L, t, R, r, O, at);
        at++;
    }
}

// The texts of made values (mode 8's), one a thread: V's n values, each one's bytes into its `width` bytes of `out`
// (its length into `lens`), from the parts' names in T. The host stores them as symbols; it makes no text.
kernel void text_make(device const long* V [[buffer(0)]], device const long* T [[buffer(1)]], device uint* lens [[buffer(2)]],
                      device uchar* out [[buffer(3)]], constant uint2& p [[buffer(4)]], uint t [[thread_position_in_grid]]) {
    if (t >= p.x) return;
    long v = V[t];
    uint form = uint((v >> 61) & 1);
    long a = (v >> 30) & ((1l << 30) - 1), b = v & ((1l << 30) - 1);
    uint ao, al, bo, bl;
    text_find(T, a, ao, al);
    text_find(T, b, bo, bl);
    device const uchar* c = text_chars(T);
    device uchar* o = out + ulong(t) * p.y;
    bool dot = form == 0 && al > 0;
    uint n = al + (dot ? 1 : 0) + bl;
    if (n > p.y) { lens[t] = 0xffffffffu; return; }
    uint at = 0;
    for (uint k = 0; k < al; k++) o[at++] = c[ao + k];
    if (dot) o[at++] = uchar('.');
    for (uint k = 0; k < bl; k++) o[at++] = c[bo + k];
    lens[t] = n;
}

// A text test's verdicts, once per distinct term (decision 0038's chunked matching): a SIMD group per term walks it
// 32 bytes at a time, each lane one byte's transition function (a row of the table), the group composing them (a
// reduction of compositions, its scratch in device memory: 32 rows of `states` bytes a group); the chunk's function
// takes the state on. A term whose final state, after the end of text, accepts sets its symbol's bit. The groups are
// persistent: `groups` of them over the terms.
struct TextParams { uint nterms; uint states; uint classes; uint start; uint groups; uint pad0; uint pad1; uint pad2; };

kernel void text_verdicts(device const long* T [[buffer(0)]], device const uchar* table [[buffer(1)]], device atomic_uint* bits [[buffer(2)]],
                          device uchar* scratch [[buffer(3)]], constant TextParams& p [[buffer(4)]], uint t [[thread_position_in_grid]],
                          uint lane [[thread_index_in_simdgroup]]) {
    device const long* ids = T + 1;
    device const uint* offs = text_offs(T);
    device const uchar* bytes = text_chars(T);
    uint sg = t / 32;
    if (sg >= p.groups) return;
    uint S = p.states, C = p.classes;
    device const uchar* cls = table;
    device const uchar* next = table + 256;
    device const uchar* eoi = next + S * C;
    device const uchar* acc = eoi + S;
    device uchar* f = scratch + ulong(sg) * 32 * S;
    for (uint term = sg; term < p.nterms; term += p.groups) {
        uint lo = offs[term], hi = offs[term + 1];
        uint state = p.start;
        for (uint base = lo; base < hi; base += 32) {
            uint at = base + lane;
            bool has = at < hi;
            uint c = has ? uint(cls[bytes[at]]) : 0;
            for (uint q = 0; q < S; q++) f[lane * S + q] = has ? next[q * C + c] : uchar(q);
            simdgroup_barrier(mem_flags::mem_device);
            for (uint d = 1; d < 32; d <<= 1) {
                uint tasks = (32 / (2 * d)) * S;
                // The earlier block's function, then the later's, into the earlier's row; then copied to the later's.
                for (uint k = lane; k < tasks; k += 32) {
                    uint right = (k / S) * 2 * d + 2 * d - 1, left = right - d, q = k % S;
                    f[left * S + q] = f[right * S + f[left * S + q]];
                }
                simdgroup_barrier(mem_flags::mem_device);
                for (uint k = lane; k < tasks; k += 32) {
                    uint right = (k / S) * 2 * d + 2 * d - 1, left = right - d, q = k % S;
                    f[right * S + q] = f[left * S + q];
                }
                simdgroup_barrier(mem_flags::mem_device);
            }
            state = f[31 * S + state];
            simdgroup_barrier(mem_flags::mem_device);
        }
        state = eoi[state];
        if (lane == 0 && acc[state] != 0) {
            long id = ids[term];
            if (id >= 0) atomic_fetch_or_explicit(&bits[ulong(id) >> 5], 1u << uint(id & 31), memory_order_relaxed);
        }
    }
}

// Rows copied between column-major buffers (a binding table, a relation): `n` rows of `ncols` columns, column k of
// the destination from source column `src[k]` (or the constant `konst[k]` where it is NONE), at `dst_off`.
struct Copy {
    long konst[16];
    uint src[16];
    uint n, ncols, src_stride, dst_stride, dst_off, pad0, pad1, pad2;
};

kernel void rows_copy(device const long* S [[buffer(0)]], device long* D [[buffer(1)]], constant Copy& c [[buffer(2)]],
                      uint t [[thread_position_in_grid]]) {
    if (t >= c.n) return;
    for (uint k = 0; k < c.ncols && k < 16; k++) {
        long v = c.src[k] == NONE ? c.konst[k] : S[ulong(c.src[k]) * c.src_stride + t];
        D[ulong(k) * c.dst_stride + c.dst_off + t] = v;
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
    rerail_accel::gpu::join::device().is_ok()
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

    /// A relation of `cols` columns and `rows` rows that the device fills (`Batch::copy_rows`).
    pub(in crate::code_rule_dsl) fn empty(cols: usize, rows: usize) -> Result<DevRel, Error> {
        Ok(DevRel { buffer: Held::new((8 * cols * rows).max(8))?, rows, cols })
    }

    /// Column `c`'s values, once the command buffer that wrote them completed.
    pub(in crate::code_rule_dsl) fn column(&self, c: usize) -> &[i64] {
        // SAFETY: the buffer holds `rows` values per column; the writer completed.
        unsafe { std::slice::from_raw_parts(self.buffer.buffer.contents().cast::<i64>().add(c * self.rows), self.rows) }
    }
}

/// Distinct symbols and their names' bytes on the device, in one buffer: their count, the symbols ascending (a
/// symbol's text is found by search), each one's offset into the bytes, the bytes. The terms a text test's verdicts
/// are made for, and the texts `str_cmp` orders.
pub(in crate::code_rule_dsl) struct DevText {
    packed: std::rc::Rc<DevRel>,
    pub(in crate::code_rule_dsl) terms: usize,
    /// One past the greatest symbol: the verdicts' bits.
    pub(in crate::code_rule_dsl) span: usize,
    /// The longest name's bytes.
    pub(in crate::code_rule_dsl) longest: usize,
    /// Whether a name holds the set separator (`strings::SET_SEP`): a text test would split it.
    pub(in crate::code_rule_dsl) sep: bool,
}

impl DevText {
    /// The distinct symbols `ids` (in any order), each named by `name` (copied to the device: the host evaluates
    /// nothing).
    pub(in crate::code_rule_dsl) fn upload(ids: &[i64], mut name: impl FnMut(i64, &mut Vec<u8>)) -> Result<DevText, Error> {
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let n = ids.len();
        let mut chars = Vec::new();
        let mut offs = Vec::with_capacity(n + 1);
        offs.push(0u32);
        for &id in &ids {
            name(id, &mut chars);
            offs.push(u32::try_from(chars.len()).map_err(|_| Error::BackendCapacity)?);
        }
        let mut out: Vec<u8> = Vec::with_capacity(8 * (n + 1) + 4 * (n + 1) + chars.len() + 1);
        out.extend_from_slice(&(n as i64).to_le_bytes());
        for id in &ids {
            out.extend_from_slice(&id.to_le_bytes());
        }
        for o in &offs {
            out.extend_from_slice(&o.to_le_bytes());
        }
        out.extend_from_slice(&chars);
        out.push(0);
        let span = ids.last().map_or(0, |&m| m.max(0) as usize + 1);
        let rows = out.len().div_ceil(8);
        let held = Held::of(out.into_iter()).map_err(|_| Error::BackendCapacity)?;
        let longest = offs.windows(2).map(|w| (w[1] - w[0]) as usize).max().unwrap_or(0);
        let sep = chars.contains(&0x1f);
        Ok(DevText { packed: std::rc::Rc::new(DevRel { buffer: held, rows, cols: 1 }), terms: n, span, longest, sep })
    }

    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        self.packed.buffer.buffer.length() as usize
    }

    /// The text as a stage's right relation (`STRCMP` reads it).
    pub(in crate::code_rule_dsl) fn rel(&self) -> std::rc::Rc<DevRel> {
        self.packed.clone()
    }
}

/// A text test's automaton on the device (`text::table`).
pub(in crate::code_rule_dsl) struct DevTable {
    table: Held,
    states: u32,
    classes: u32,
    start: u32,
}

impl DevTable {
    pub(in crate::code_rule_dsl) fn upload(t: &crate::code_rule_dsl::text::table::Table) -> Result<DevTable, Error> {
        Ok(DevTable { table: Held::of(t.bytes.iter().copied()).map_err(|_| Error::BackendCapacity)?, states: t.states, classes: t.classes, start: t.start })
    }
}

/// Rows a copy reads or writes: a binding table's (its stride its capacity) or a relation's (its rows).
pub(in crate::code_rule_dsl) enum Rows<'a> {
    Table(&'a Table),
    Rel(&'a DevRel),
}

impl Rows<'_> {
    fn of(&self) -> (&BufferRef, usize) {
        match self {
            Rows::Table(t) => (&t.buffer.buffer, t.cap),
            Rows::Rel(r) => (&r.buffer.buffer, r.rows.max(1)),
        }
    }
}

/// A destination column of a copy: a source column, or a constant.
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) enum Col {
    From(usize),
    Const(i64),
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CopyParams {
    konst: [i64; 16],
    src: [u32; 16],
    n: u32,
    ncols: u32,
    src_stride: u32,
    dst_stride: u32,
    dst_off: u32,
    pad: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<CopyParams>() == 224);

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
/// The device buffers' budget left in the process (other lanes hold theirs: the syntax side's tapes most).
pub(in crate::code_rule_dsl) fn device_bytes_left() -> usize {
    super::metal_backend::METAL_BUFFER_LIMIT.saturating_sub(super::metal_backend::reserved())
}

/// Gives this thread's pooled free buffers back to the budget (after a refusal for capacity).
pub(in crate::code_rule_dsl) fn give_back() {
    rerail_accel::gpu::shared::drop_free();
}

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

/// A batch's text arena: the texts its text functions make (`TEXTFN`), shared by every job in the batch. Its first
/// word is an atomic bump of the bytes given out, which counts on past the end, so a full arena says how much its
/// jobs wanted (they then run again in one that big, or on the host); its second, its capacity.
pub(in crate::code_rule_dsl) struct Arena {
    buffer: Held,
    pub(in crate::code_rule_dsl) cap: usize,
}

/// A batch's arena, to start with: most strata make few texts.
pub(in crate::code_rule_dsl) const ARENA_START: usize = 64 << 10;

impl Arena {
    pub(in crate::code_rule_dsl) fn new(cap: usize) -> Result<Arena, Error> {
        let cap = cap.clamp(16, u32::MAX as usize / 2);
        let buffer = Held::new(16 + cap)?;
        // SAFETY: 16 bytes of header, written before any command buffer reads them.
        let head = unsafe { std::slice::from_raw_parts_mut(buffer.buffer.contents().cast::<u32>(), 4) };
        head.copy_from_slice(&[0, cap as u32, 0, 0]);
        Ok(Arena { buffer, cap })
    }

    /// The bytes its jobs asked for (past `cap` when it filled), once they completed.
    pub(in crate::code_rule_dsl) fn wanted(&self) -> usize {
        // SAFETY: the header's first word, read once the command buffers that write it completed.
        unsafe { *self.buffer.buffer.contents().cast::<u32>() as usize }
    }

    /// The text of arena value `v` (`ARENA_TEXT`'s), once the command buffers that made it completed.
    pub(in crate::code_rule_dsl) fn text(&self, v: i64) -> Option<&[u8]> {
        if v & ARENA_TEXT == 0 || v == -1 {
            return None;
        }
        let (off, len) = (((v as u64) >> 24 & ((1 << 39) - 1)) as usize, (v & 0xff_ffff) as usize);
        if off + len > self.cap {
            return None;
        }
        // SAFETY: within the arena's `cap` bytes past its header.
        Some(unsafe { std::slice::from_raw_parts(self.buffer.buffer.contents().cast::<u8>().add(16 + off), len) })
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
    open: Option<CommandBuffer>,
    pass: Option<Pass<'static>>,
    committed: Vec<Submitted>,
    done: Done,
    dummy: Held,
    /// Buffers the encoded work reads that nothing else holds: kept until it completes (a pooled buffer is written
    /// by the host when handed out again, and Metal orders no host write after a GPU read).
    keep: Vec<Held>,
    /// The texts its jobs' text functions make, made the first time a job asks.
    arena: Option<std::rc::Rc<Arena>>,
    arena_cap: usize,
}

impl Batch {
    pub(in crate::code_rule_dsl) fn new() -> Result<Batch, Error> {
        rerail_accel::gpu::join::device()?;
        Batch::with_arena(ARENA_START)
    }

    /// A batch whose text arena holds `cap` bytes.
    pub(in crate::code_rule_dsl) fn with_arena(cap: usize) -> Result<Batch, Error> {
        rerail_accel::gpu::join::device()?;
        Ok(Batch { open: None, pass: None, committed: Vec::new(), done: Done::default(), dummy: Held::new(64)?, keep: Vec::new(), arena: None, arena_cap: cap })
    }

    /// Its text arena, shared by its jobs.
    pub(in crate::code_rule_dsl) fn arena(&mut self) -> Result<std::rc::Rc<Arena>, Error> {
        if self.arena.is_none() {
            self.arena = Some(std::rc::Rc::new(Arena::new(self.arena_cap)?));
        }
        Ok(self.arena.clone().expect("the arena"))
    }

    /// The encoder of the open command buffer, with room for `need` more dispatches (a new command buffer when not).
    fn pass(&mut self, need: usize) -> Result<&Pass<'static>, Error> {
        if self.pass.as_ref().is_some_and(|p| p.dispatches() + need > BUDGET) {
            self.submit_open()?;
        }
        if self.pass.is_none() {
            let command = queue()?.new_command_buffer().to_owned();
            // SAFETY: the command buffer is owned by `self.open` and outlives the pass, which `commit` ends and drops
            // before the command buffer (the field order of the take there).
            let r: &'static metal::CommandBufferRef = unsafe { &*(command.as_ref() as *const metal::CommandBufferRef) };
            self.open = Some(command);
            self.pass = Some(Pass::new(r));
        }
        Ok(self.pass.as_ref().expect("an open pass"))
    }

    /// Ends the open pass and commits its command buffer (`Submitted::commit`, checked against the launch budget).
    fn submit_open(&mut self) -> Result<(), Error> {
        let Some(pass) = self.pass.take() else { return Ok(()) };
        self.done.dispatches += pass.dispatches() as u32;
        pass.end();
        let command = self.open.take().expect("an open command buffer");
        let s = Submitted::commit(&command)?;
        self.committed.push(s);
        self.done.submissions += 1;
        Ok(())
    }

    /// Commits what is open and waits for every command buffer.
    pub(in crate::code_rule_dsl) fn finish(&mut self) -> Result<Done, Error> {
        self.submit_open()?;
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
            let counts = scratch(FREE, 4 * n)?;
            let scan = Scan::new(n)?;
            let pass = self.pass(12)?;
            let lbuf: &BufferRef = &left.buffer.buffer;
            pass.run_bytes(&scount, &[lbuf, &rbuf, &counters.buffer.buffer, &counts, &totals.buffer.buffer], stage.bytes(), groups(n));
            scan.encode(pass, &counts)?;
            pass.run_bytes(&sscatter, &[lbuf, &rbuf, &totals.buffer.buffer, &counts, &out.buffer.buffer], stage.bytes(), groups(n));
            return Ok(());
        }
        let n = left.cap + 1;
        let counts = scratch(FREE, 4 * n)?;
        let scan = Scan::new(n)?;
        let arena: metal::Buffer = if stage.mode == TEXTFN || matches!(stage.mode, STRCMP | TEXTOP) && (stage.agg[1] & 15 == OPND_ARENA || stage.agg[1] >> 4 == OPND_ARENA) {
            self.arena()?.buffer.buffer.to_owned()
        } else {
            self.dummy.buffer.to_owned()
        };
        let pass = self.pass(12)?;
        let lbuf: &BufferRef = &left.buffer.buffer;
        pass.run_bytes(&count, &[lbuf, &rbuf, &perm, &starts, &rkeys, &nruns, &totals.buffer.buffer, &counters.buffer.buffer, &counts, &arena], stage.bytes(), groups(n));
        scan.encode(pass, &counts)?;
        pass.run_bytes(&scatter, &[lbuf, &rbuf, &perm, &starts, &rkeys, &nruns, &totals.buffer.buffer, &counts, &out.buffer.buffer, &arena, &counters.buffer.buffer], stage.bytes(), groups(n));
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
        let counts = scratch(FREE, 4 * n)?;
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

    /// `n` rows of `src` into `dst` at row `dst_off`, column k of `dst` as `cols[k]` says (a column of `src`, or a
    /// constant): a head's rows into its union, a union's new rows onto its relation.
    pub(in crate::code_rule_dsl) fn copy_rows(&mut self, src: Rows<'_>, n: usize, cols: &[Col], dst: Rows<'_>, dst_off: usize) -> Result<(), Error> {
        if n == 0 {
            return Ok(());
        }
        assert!(cols.len() <= 16);
        let copy = pipeline("join", "rows_copy", || SOURCE.to_owned())?;
        let (s, src_stride) = src.of();
        let (d, dst_stride) = dst.of();
        assert!(dst_off + n <= dst_stride, "a copy of {n} rows at {dst_off} into a destination of {dst_stride}");
        let mut p = CopyParams { konst: [0; 16], src: [NONE; 16], n: n as u32, ncols: cols.len() as u32, src_stride: src_stride as u32, dst_stride: dst_stride as u32, dst_off: dst_off as u32, pad: [0; 3] };
        for (k, c) in cols.iter().enumerate() {
            match *c {
                Col::From(c) => p.src[k] = c as u32,
                Col::Const(v) => p.konst[k] = v,
            }
        }
        // SAFETY: `CopyParams` is `repr(C)` integers with no padding (its size asserted above).
        let bytes = unsafe { std::slice::from_raw_parts((&p as *const CopyParams).cast::<u8>(), std::mem::size_of::<CopyParams>()) };
        let (s, d) = (s.to_owned(), d.to_owned());
        let pass = self.pass(1)?;
        pass.run_bytes(&copy, &[&s, &d], bytes, groups(n));
        Ok(())
    }

    /// Keeps `table` until the batch's work completes (its verdicts read it).
    pub(in crate::code_rule_dsl) fn hold_table(&mut self, table: DevTable) {
        self.keep.push(table.table);
    }

    /// The verdicts of `table` for `text`'s terms, as bits by symbol into `bits` (zeroed, at least `text.span` bits).
    pub(in crate::code_rule_dsl) fn verdicts(&mut self, text: &DevText, table: &DevTable, bits: &DevRel) -> Result<(), Error> {
        if text.terms == 0 {
            return Ok(());
        }
        assert!(bits.rows * 64 >= text.span);
        let kernel = pipeline("join", "text_verdicts", || SOURCE.to_owned())?;
        let groups = text.terms.min(1024);
        let scratch = Held::new(groups * 32 * table.states as usize)?;
        let p: [u32; 8] = [text.terms as u32, table.states, table.classes, table.start, groups as u32, 0, 0, 0];
        // SAFETY: plain integers.
        let bytes = unsafe { std::slice::from_raw_parts(p.as_ptr().cast::<u8>(), 32) };
        let pass = self.pass(1)?;
        pass.run_bytes(&kernel, &[&text.packed.buffer.buffer, &table.table.buffer, &bits.buffer.buffer, &scratch.buffer], bytes, (groups * 32).div_ceil(256));
        self.keep.push(scratch);
        Ok(())
    }

    /// The texts of made values `vals` (`MAKE`'s), their parts' names in `text`: each one's length into `lens` (one
    /// a row) and its bytes into `width` bytes of `out`.
    pub(in crate::code_rule_dsl) fn make_texts(&mut self, vals: &DevRel, text: &DevText, width: usize, lens: &DevRel, out: &DevRel) -> Result<(), Error> {
        if vals.rows == 0 {
            return Ok(());
        }
        assert!(lens.rows * 2 >= vals.rows && out.rows * 8 >= vals.rows * width);
        let kernel = pipeline("join", "text_make", || SOURCE.to_owned())?;
        let p: [u32; 2] = [vals.rows as u32, width as u32];
        // SAFETY: plain integers.
        let bytes = unsafe { std::slice::from_raw_parts(p.as_ptr().cast::<u8>(), 8) };
        let pass = self.pass(1)?;
        pass.run_bytes(&kernel, &[&vals.buffer.buffer, &text.packed.buffer.buffer, &lens.buffer.buffer, &out.buffer.buffer], bytes, vals.rows.div_ceil(256));
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
        if rerail_accel::gpu::join::device().is_err() {
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
        if rerail_accel::gpu::join::device().is_err() {
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
