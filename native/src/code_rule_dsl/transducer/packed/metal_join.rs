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
/// `cat`'s form: b's name alone when a's is empty, else a's, ".", b's.
pub(in crate::code_rule_dsl) const CAT_DOT: u32 = 0;
/// `str_concat`'s form: a's name then b's.
pub(in crate::code_rule_dsl) const CAT_PLAIN: u32 = 1;
/// Mode 9: a text function's value made into the batch's text arena (`Arena`), or a range of its text, bound to
/// `bind_slot[0]`: of X (keys 0, 1, kind `agg[1]`), Y and Z (keys 2 and 4, symbols), numbers (keys 5 and 6), by
/// `agg[0]`. A value with none (the arena full, or `lower` past ASCII) sets the stage's counter word (`TEXTFN_FULL`,
/// `TEXTFN_HOST`).
/// Mode 10: a made text per row by a SIMD group a row (`row_text`): agg[0] & 15 one of `ROW_*`, agg[0] bit 8 a test's
/// wanted verdict; X (keys 0, 1) and Y (keys 2, 3) of kinds agg[1] & 15 and agg[1] >> 4 (`OPND_*`); its value into
/// `bind_slot[0]`.
pub(in crate::code_rule_dsl) const ROWTEXT: u32 = 10;
/// A witness (decision 0277): per binding, its candidates of the stage's relation until one that every probe of the
/// rest of the body passes (`Batch::witness`); the binding out as it is. A probe's key reads a slot (`SLOT`), a
/// constant (`CONST`) or a column of the candidate (`ACOL`, its column in `key_slot`).
pub(in crate::code_rule_dsl) const WITNESS: u32 = 11;
/// A witness probe's key from the candidate's row: its column.
pub(in crate::code_rule_dsl) const ACOL: u32 = 2;
/// The most probes a witness makes per candidate.
pub(in crate::code_rule_dsl) const WITNESS_PROBES: usize = 3;
/// `cat`'s text of X and Y ("." between when X is not empty).
pub(in crate::code_rule_dsl) const ROW_CAT: u32 = 0;
/// `str_concat`'s text of X and Y.
pub(in crate::code_rule_dsl) const ROW_CONCAT: u32 = 1;
/// `replace_all` of a regex (its automata) in X by Y.
pub(in crate::code_rule_dsl) const ROW_REPLACE: u32 = 2;
/// A text test of X by its table.
pub(in crate::code_rule_dsl) const ROW_TEST: u32 = 3;
/// Text function `TEXTFN_*` (agg[0] >> 16) of X: of numbers keys 5 and 6; a literal replacement's pattern key 2's
/// symbol, its replacement key 4's.
pub(in crate::code_rule_dsl) const ROW_MAP: u32 = 4;
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
/// X's text whole: a cat's (operand `OPND_DOT` or `OPND_PLAIN`), made for the head or a text function.
pub(in crate::code_rule_dsl) const TEXTFN_COPY: u32 = 10;
/// `zero_pad(n, w)`: key 5's digits zero-padded to key 6's width (0 to 64), the sign first, never truncated.
pub(in crate::code_rule_dsl) const TEXTFN_ZERO_PAD: u32 = 11;
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
    /// The probed relation's descriptor's offset in its block (a `rerail_blocks` `Relation`, its ids from 0), read
    /// where it lies through the generated reads (decision 0325); 0 for a relation in a buffer of its own,
    /// column-major (a block's header is at 0).
    pub(in crate::code_rule_dsl) r_at: u64,
    /// The offset of a delta of it whose dead bits mark its dead rows (`Relation::try_kills`), which every read of
    /// its rows skips (`row_live`); 0 when none is. Also keeps `ops` at 16 bytes (MSL's `uint4`).
    pub(in crate::code_rule_dsl) r_dead: u64,
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

const _: () = assert!(std::mem::size_of::<Stage>() == 2304 && std::mem::size_of::<Stage>() % 16 == 0);

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

/// The chunk views' kernel (decision 0289 §2): the chunk's structures as `rerail-blocks` generates them, then
/// `view_expand`, which makes each row as `Viewed::value_at` does.
fn chunk_views_source() -> String {
    let mut s = String::from("#include <metal_stdlib>\nusing namespace metal;\n");
    s += rerail_blocks::RELATION_READS;
    s += &crate::code_rule_dsl::facts::vtab::syntax::device_chunk::msl();
    s += CHUNK_VIEWS;
    s
}

const CHUNK_VIEWS: &str = r#"
constant uint CH_NONE = 0xffffffffu;
// A span's items: its own address plus its offset (`rerail_blocks::Span`).
template <typename T> inline device const T* items(device const Span& s) { return (device const T*)((device const char*)&s + s.off); }

inline uint view_start(device const ChunkDoc& d, uint view) {
    return view == 1u ? d.child_row : (view == 2u ? d.field_row : d.node_row);
}
inline long global_of(device const ChunkDoc& d, uint l) {
    uint low = d.preorder != 0u ? l : items<uint>(d.low)[l];
    return long(ulong(d.key) << 32 | ulong(low));
}
// The projection's parent (`Projected::parent`): a node left out of it has the root for parent.
inline uint parent_of(device const ChunkDoc& d, uint l) {
    uint q = items<uint>(d.up)[l];
    return (q == CH_NONE && l != 0u) ? 0u : q;
}
// The kind column (`Dialect::kind`) of local `l`: the canonical code's atom.
inline long kind_of(device const ChunkDoc& d, uint l) {
    uint code = items<uint>(d.kind)[l] % 1024u;
    if (code == 0u) return long(d.tag * 1024u);
    uint c = code < uint(d.canonical.len) ? uint(items<ushort>(d.canonical)[code]) : code;
    return long(d.tag * 1024u + (c == 0xffffu ? 0u : c));
}

// A node's or span's id (`Projected::global`): past the locals with ids by definition, a span's (`granules::span_low`).
inline long loc_global(device const ChunkDoc& d, uint l) {
    uint low;
    if (d.preorder != 0u) {
        low = l;
    } else if (l < uint(d.low.len)) {
        low = items<uint>(d.low)[l];
    } else {
        uint i = l - uint(d.low.len);
        uint slot = (GR_SLOTS - 1u) - min(i >> GR_INDEX_BITS, GR_SPAN_SLOTS - 1u);
        low = slot << GR_INDEX_BITS | (i & ((1u << GR_INDEX_BITS) - 1u));
    }
    return long(ulong(d.key) << 32 | ulong(low));
}
// The lines starting at or before byte `at` (`core_rr::Lines::line`, before its `max(1)`).
inline uint line_of(device const ChunkDoc& d, uint at) {
    device const uint* st = items<uint>(d.lines);
    uint lo = 0u, hi = uint(d.lines.len);
    while (lo < hi) {
        uint m = (lo + hi) / 2u;
        if (st[m] <= at) lo = m + 1u; else hi = m;
    }
    return lo;
}

// The facts' lines starting at or before byte `at` (`Facts::start_line`: `LineTable::line` + 1).
inline uint fline_of(device const ChunkDoc& d, uint at) {
    device const uint* st = items<uint>(d.flines);
    uint lo = 0u, hi = uint(d.flines.len);
    while (lo < hi) {
        uint m = (lo + hi) / 2u;
        if (st[m] <= at) lo = m + 1u; else hi = m;
    }
    return lo;
}

// The document holding node row `row` (of `node`): the last whose rows start at or before it.
inline uint doc_of_node(device const ChunkDoc* docs, uint ndocs, uint row) {
    uint lo = 0u, hi = ndocs;
    while (hi - lo > 1u) {
        uint m = (lo + hi) / 2u;
        if (docs[m].node_row <= row) lo = m; else hi = m;
    }
    return lo;
}

// Row `r`'s value `v` in column `c` of the relation `out` makes (decision 0325 step 5: a relation in its block, its
// rows' descriptor at `rows_at`), through the read written once.
static inline void view_put(device uchar* out, ulong rows_at, uint r, uint c, long v) {
    *(device long*)(out + clif_rows_at((device const uchar*)out, rows_at, r, c)) = v;
}

// p: the root's offset, the view (`device_chunk::VIEW_*`), the rows, the columns, an axis's pairs' offset (else 0),
// the made relation's rows' descriptor's offset (low, high), then each column's index.
kernel void view_expand(device const char* B [[buffer(0)]], device uchar* out [[buffer(1)]], constant uint* p [[buffer(2)]],
                        uint r [[thread_position_in_grid]]) {
    uint n = p[2];
    if (r >= n) return;
    ulong rows_at = ulong(p[5]) | (ulong(p[6]) << 32);
    device const ChunkRoot* root = (device const ChunkRoot*)(B + p[0]);
    device const ChunkDoc* docs = items<ChunkDoc>(root->docs);
    uint view = p[1];
    if (view == 4u) {
        // An axis read whole (`Within`): a pair of node rows, inner << 32 | outer; each column the node's id.
        ulong pair = ((device const ulong*)(B + p[4]))[r];
        for (uint c = 0u; c < p[3]; c++) {
            uint row = p[7 + c] == 0u ? uint(pair >> 32) : uint(pair & 0xffffffffu);
            device const ChunkDoc& d = docs[doc_of_node(docs, root->ndocs, row)];
            view_put(out, rows_at, r, c, global_of(d, row - d.node_row));
        }
        return;
    }
    if (view >= 5u && view <= 8u) {
        // core.rr's places (`View::Loc`): the document whose `Loc` rows hold `r`, then its local's span.
        uint lo = 0u, hi = root->ndocs;
        while (hi - lo > 1u) {
            uint m = (lo + hi) / 2u;
            if (docs[m].loc_row <= r) lo = m; else hi = m;
        }
        device const ChunkDoc& d = docs[lo];
        uint l = r - d.loc_row;
        uint s, e;
        if (l < d.nodes) {
            s = items<uint>(d.start)[l];
            e = items<uint>(d.end)[l];
        } else {
            s = items<uint>(d.spans)[2u * (l - d.nodes)];
            e = items<uint>(d.spans)[2u * (l - d.nodes) + 1u];
        }
        // A guest's places are its file's, found on the host (`ChunkDoc::placed`).
        bool placed = d.placed.len > 0;
        device const uint* pl = items<uint>(d.placed) + 3u * l;
        for (uint c = 0u; c < p[3]; c++) {
            long v;
            if (p[7 + c] == 0u) {
                v = loc_global(d, l);
            } else if (view == 8u) {
                v = long(d.key);
            } else if (placed) {
                v = long(view == 5u ? pl[0] : (view == 6u ? pl[1] : pl[2]));
            } else {
                uint line = max(line_of(d, s), 1u);
                if (view == 5u) v = long(line);
                else if (view == 6u) v = long(e > s ? max(line_of(d, e - 1u), 1u) : line);
                else v = long(s - items<uint>(d.lines)[line - 1u]);
            }
            view_put(out, rows_at, r, c, v);
        }
        return;
    }
    // The last document whose rows start at or before `r` holds it (an empty one starts where the next does).
    uint lo = 0u, hi = root->ndocs;
    while (hi - lo > 1u) {
        uint m = (lo + hi) / 2u;
        if (view_start(docs[m], view) <= r) lo = m; else hi = m;
    }
    device const ChunkDoc& d = docs[lo];
    uint i = r - view_start(d, view);
    uint l = view == 1u ? i + 1u : (view == 2u ? items<uint>(d.fielded)[i] : i);
    uint id = items<uint>(d.core)[l];
    for (uint c = 0u; c < p[3]; c++) {
        uint col = p[7 + c];
        long v;
        if ((view == 0u || view == 3u) && col == 0u) {
            v = long(d.key);
        } else if (view == 3u && col == 1u) {
            v = long(id);
        } else if (view == 0u && col == 2u) {
            v = kind_of(d, l);
        } else if (view == 9u && col == 1u) {
            // `line`'s columns are the node and its start's line, by the facts' lines (`Viewed::value_at`).
            v = long(fline_of(d, items<uint>(d.start)[l]));
        } else if (view == 0u && col == 3u) {
            v = long(items<uint>(d.start)[l]);
        } else if (view == 0u && col == 4u) {
            v = long(items<uint>(d.end)[l]);
        } else if ((view == 1u || view == 2u) && col == 0u) {
            uint q = parent_of(d, l);
            v = q == CH_NONE ? -1 : global_of(d, q);
        } else if (view == 1u && col == 2u) {
            // `child`'s columns are parent, child, index (`Viewed::columns`).
            v = long(items<uint>(d.ord)[l]);
        } else if (view == 2u && col == 1u) {
            v = long(d.tag << 16 | uint(items<ushort>(d.field)[l]));
        } else {
            v = global_of(d, l);
        }
        view_put(out, rows_at, r, c, v);
    }
}

// A kind-set axis's batch solve in the chunk (decision 0289 §2, `Within::solved_in_chunk`), a thread per node row.
// p: the root's offset, the node rows, the members' offset (0: every kind is one), 1 for every ancestor in the set
// (`inside`, containment) or 0 for the nearest. The climb is bounded by the document's nodes.
inline bool axis_member(device const char* B, uint bits_at, device const ChunkDoc& d, uint l) {
    if (bits_at == 0u) return true;
    uint at = d.tag * 1024u + items<uint>(d.kind)[l] % 1024u;
    return ((((device const uint*)(B + bits_at))[at >> 5] >> (at & 31u)) & 1u) != 0u;
}

// Each node row's rows (its ancestors in the set), into `start`; the extra value past the last row is 0, so the
// scan after leaves the total there.
kernel void axis_count(device const char* B [[buffer(0)]], device uint* start [[buffer(1)]], constant uint* p [[buffer(2)]],
                       uint v [[thread_position_in_grid]]) {
    uint n = p[1];
    if (v > n) return;
    if (v == n) { start[v] = 0u; return; }
    device const ChunkRoot* root = (device const ChunkRoot*)(B + p[0]);
    device const ChunkDoc* docs = items<ChunkDoc>(root->docs);
    device const ChunkDoc& d = docs[doc_of_node(docs, root->ndocs, v)];
    uint count = 0u;
    uint up = parent_of(d, v - d.node_row);
    for (uint k = 0u; k < d.nodes && up != CH_NONE; k++) {
        if (axis_member(B, p[2], d, up)) {
            count++;
            if (p[3] == 0u) break;
        }
        up = parent_of(d, up);
    }
    start[v] = count;
}

// Each node row's rows from its start (the scanned counts): inner << 32 | outer, nearest first. p[4] is the rows
// the pairs' space holds: when the total (`start[n]`) is more, nothing is written, and the host writes again.
kernel void axis_write(device const char* B [[buffer(0)]], device const uint* start [[buffer(1)]], device ulong* pairs [[buffer(2)]],
                       constant uint* p [[buffer(3)]], uint v [[thread_position_in_grid]]) {
    if (v >= p[1] || start[p[1]] > p[4]) return;
    device const ChunkRoot* root = (device const ChunkRoot*)(B + p[0]);
    device const ChunkDoc* docs = items<ChunkDoc>(root->docs);
    device const ChunkDoc& d = docs[doc_of_node(docs, root->ndocs, v)];
    uint at = start[v];
    uint up = parent_of(d, v - d.node_row);
    for (uint k = 0u; k < d.nodes && up != CH_NONE; k++) {
        if (axis_member(B, p[2], d, up)) {
            pairs[at++] = ulong(v) << 32 | ulong(d.node_row + up);
            if (p[3] == 0u) break;
        }
        up = parent_of(d, up);
    }
}
"#;

/// Every join kernel's source: `SOURCE_MAIN` with the relation's reads written once (`rerail_blocks::RELATION_READS`,
/// decision 0325), which a stage reads a relation in its block with.
static SOURCE: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| SOURCE_MAIN.replacen("using namespace metal;\n", &format!("using namespace metal;\n{}", rerail_blocks::RELATION_READS), 1));

const SOURCE_MAIN: &str = r#"#include <metal_stdlib>
using namespace metal;

struct Stage {
    long key_const[8];
    long par_const[16];
    ulong r_at, r_dead;
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

// A value of the probed relation: row `r`'s in column `col`, read in its block through the generated reads (`r_at`,
// decision 0325). Every relation the device reads lies in a block (step 5).
static inline long rget(constant Stage& s, device const long* R, uint r, uint col) {
    return long(clif_rel_get((device const uchar*)R, s.r_at, r, col));
}

static inline long colval(constant Stage& s, uint k, device const long* L, uint t, device const long* R, uint r) {
    long v = s.col_kind[k] == 0 ? rget(s, R, r, s.col_idx[k]) : L[ulong(s.col_idx[k]) * s.cap_in + t];
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
// Whether row `r` of the probed relation is live: not marked by its dead rows' delta (`r_dead`, the generated
// `clif_rel_dead`).
static inline bool row_live(constant Stage& s, device const long* R, uint r) {
    return s.r_dead == 0ul || clif_rel_dead((device const uchar*)R, s.r_dead, r) == 0u;
}

static inline bool matches(constant Stage& s, device const long* R, uint r, thread const long* kv) {
    if (!row_live(s, R, r)) return false;
    for (uint i = 0; i < s.nkeys && i < 8; i++) if (rget(s, R, r, s.key_rcol[i]) != kv[i]) return false;
    for (uint i = 0; i < s.ntests && i < 8; i++) if (rget(s, R, r, s.test_a[i]) != rget(s, R, r, s.test_b[i])) return false;
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
        long x = rget(s, R, r, s.agg[1]);
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
static inline bool textop_one(uint op, Txt x, uint xs, uint lx, Txt y, uint ly) {
    if (op == 0) return lx == ly && txt_same(x, xs, y, 0, lx);
    if (op == 1) return ly >= lx && txt_same(x, xs, y, 0, lx);
    return ly >= lx && txt_same(x, xs, y, ly - lx, lx);
}

// With `TEXT_SPLIT` (agg[0] bit 5), X is a set of patterns (`strings::SET_SEP` between them, empty ones too, as
// `strings::test` splits it): Y passes when any one does. The separators are found a word of eight at a time (an exact
// mask of the bytes equal to 0x1f), each member compared as one pattern.
static inline bool textop_pass(constant Stage& s, device const long* L, uint t, device const long* R, device atomic_uint* A) {
    Txt x = txt_of(s, L, t, R, A, 0, s.agg[1] & 15), y = txt_of(s, L, t, R, A, 2, s.agg[1] >> 4);
    uint lx = txt_len(x), ly = txt_len(y);
    uint op = (s.agg[0] >> 1) & 3;
    bool r = false;
    if ((s.agg[0] & 32) == 0) r = textop_one(op, x, 0, lx, y, ly);
    else {
        const ulong lo7 = 0x7f7f7f7f7f7f7f7ful;
        uint start = 0;
        for (uint at = 0; at < lx; at += 8) {
            ulong z = txt_word(x, at, lx) ^ 0x1f1f1f1f1f1f1f1ful;
            // Bytes past the text read 0 (z's 0x1f): never a separator.
            ulong m = ~(((z & lo7) + lo7) | z | lo7);
            while (m != 0) {
                uint k = clz(m) / 8;
                uint pos = at + k;
                r = r || textop_one(op, x, start, pos - start, y, ly);
                start = pos + 1;
                m &= ~(0x8000000000000000ul >> (8 * k));
            }
        }
        r = r || textop_one(op, x, start, lx - start, y, ly);
    }
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
        if (s.mode == 3 ? !row_live(s, R, r) : !matches(s, R, r, kv)) continue;
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
        else v = kind == 0 ? colval(s, s.bind_idx[b], L, t, R, r) : (kind == 1 ? parval(s, s.bind_idx[b], L, t) : rget(s, R, r, s.bind_idx[b]));
        O[ulong(s.bind_slot[b]) * s.cap_out + at] = v;
    }
}

kernel void stage_scatter(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]], device const uint* perm [[buffer(2)]],
                          device const uint* starts [[buffer(3)]], device const uint* rkeys [[buffer(4)]], device const uint* nruns [[buffer(5)]],
                          device uint* totals [[buffer(6)]], device const uint* offsets [[buffer(7)]], device long* O [[buffer(8)]],
                          device atomic_uint* A [[buffer(9)]], device atomic_uint* counters [[buffer(10)]], constant Stage& s [[buffer(11)]],
                          uint t [[thread_position_in_grid]]) {
    uint n = min(totals[s.in_slot], s.cap_in);
    // The offsets past the rows' are not scanned when the grids are sized on the device (`Scan::size`).
    if (t == 0) totals[s.out_slot] = offsets[n];
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
        if (s.mode == 3 ? !row_live(s, R, r) : !matches(s, R, r, kv)) continue;
        if (!defined(s, L, t, R, r)) continue;
        if (s.last != NONE) {
            ulong v = eval(s, L, t, R, r);
            if (!out_bit(s, v, s.last)) continue;
        }
        write_row(s, L, t, R, r, O, at);
        at++;
    }
}

// A witness's probe (decision 0277): whether relation R has a row of `b`'s keys (a binding's slots, constants or the
// candidate's columns) and column tests; an anti-join's, whether it has none. One loop over the keys' run.
static inline bool witness_probe(constant Stage& b, device const long* L, uint t, uint cap_in, constant Stage& a, device const long* RA, uint ra,
                                 device const long* R, device const uint* perm, device const uint* starts, device const uint* rkeys,
                                 device const uint* nruns) {
    long kv[8];
    for (uint i = 0; i < b.nkeys && i < 8; i++) {
        uint k = b.key_kind[i];
        kv[i] = k == 1 ? L[ulong(b.key_slot[i]) * cap_in + t] : (k == 2 ? rget(a, RA, ra, b.key_slot[i]) : b.key_const[i]);
    }
    uint lo = 0, hi = b.nr;
    if (b.nkeys > 0) {
        uint h = hash_keys(kv, b.nkeys, b.mask);
        uint a = 0, e = b.nr == 0 ? 0 : nruns[0];
        uint runs = e;
        for (uint k = 0; k < 34 && a < e; k++) {
            uint mid = (a + e) / 2;
            if (rkeys[mid] < h) a = mid + 1; else e = mid;
        }
        if (a < runs && rkeys[a] == h) { lo = starts[a]; hi = starts[a + 1]; } else { lo = 0; hi = 0; }
    }
    bool found = false;
    for (uint q = lo; q < hi; q++) {
        uint r = b.nkeys > 0 ? perm[q] : q;
        if (matches(b, R, r, kv)) { found = true; break; }
    }
    return found != (b.mode == 2);
}

// A witness's count (mode 11): 1 for a binding with a candidate of the stage's relation that every probe passes,
// found in the candidates' order, the loop ending at the first; else 0. `stage_scatter` (as a semi-join) writes them.
kernel void witness_count(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]], device const uint* perm [[buffer(2)]],
                          device const uint* starts [[buffer(3)]], device const uint* rkeys [[buffer(4)]], device const uint* nruns [[buffer(5)]],
                          device const uint* totals [[buffer(6)]], device uint* counts [[buffer(7)]], constant Stage* bs [[buffer(8)]],
                          device const long* R0 [[buffer(9)]], device const uint* p0 [[buffer(10)]], device const uint* s0 [[buffer(11)]],
                          device const uint* k0 [[buffer(12)]], device const uint* n0 [[buffer(13)]],
                          device const long* R1 [[buffer(14)]], device const uint* p1 [[buffer(15)]], device const uint* s1 [[buffer(16)]],
                          device const uint* k1 [[buffer(17)]], device const uint* n1 [[buffer(18)]],
                          device const long* R2 [[buffer(19)]], device const uint* p2 [[buffer(20)]], device const uint* s2 [[buffer(21)]],
                          device const uint* k2 [[buffer(22)]], device const uint* n2 [[buffer(23)]],
                          constant Stage& s [[buffer(24)]], uint t [[thread_position_in_grid]]) {
    if (t > s.cap_in) return;
    uint n = min(totals[s.in_slot], s.cap_in);
    if (t >= n) { counts[t] = 0; return; }
    long kv[8];
    uint lo, hi;
    candidates(s, L, t, starts, rkeys, nruns, kv, lo, hi);
    bool keyed = s.nkeys > 0;
    uint nb = min(s.agg[0], 3u);
    uint found = 0;
    for (uint q = lo; q < hi; q++) {
        uint r = keyed ? perm[q] : q;
        if (!matches(s, R, r, kv)) continue;
        bool all = true;
        if (nb > 0) all = witness_probe(bs[0], L, t, s.cap_in, s, R, r, R0, p0, s0, k0, n0);
        if (all && nb > 1) all = witness_probe(bs[1], L, t, s.cap_in, s, R, r, R1, p1, s1, k1, n1);
        if (all && nb > 2) all = witness_probe(bs[2], L, t, s.cap_in, s, R, r, R2, p2, s2, k2, n2);
        if (all) { found = 1; break; }
    }
    counts[t] = found;
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

// The states after each of m (at most 32) bytes of x, lane l's after byte `first + step * l`, from state `st`: the
// bytes' transition functions (rows of table `tab`, S states of C classes) composed by a prefix over the lanes in
// device scratch `f` (two rows of S bytes a lane), Hillis and Steele's: five steps of a row a lane.
static inline uint chunk_states(device const uchar* tab, uint S, uint C, Txt x, int first, int step, uint m, uint st,
                                device uchar* f, uint lane) {
    device const uchar* cls = tab;
    device const uchar* next = tab + 256;
    device uchar* a = f;
    device uchar* b = f + 32 * S;
    bool has = lane < m;
    uint c = has ? uint(cls[txt_byte(x, uint(first + step * int(lane)))]) : 0;
    for (uint q = 0; q < S; q++) a[lane * S + q] = has ? next[q * C + c] : uchar(q);
    simdgroup_barrier(mem_flags::mem_device);
    for (uint d = 1; d < 32; d <<= 1) {
        // The bytes before (lane - d's prefix), then this lane's: applied in order.
        for (uint q = 0; q < S; q++) b[lane * S + q] = lane >= d ? a[lane * S + a[(lane - d) * S + q]] : a[lane * S + q];
        simdgroup_barrier(mem_flags::mem_device);
        device uchar* t = a; a = b; b = t;
    }
    uint r = a[lane * S + st];
    simdgroup_barrier(mem_flags::mem_device);
    return r;
}

// A text's bytes as a text operand.
static inline Txt txt_raw(device const uchar* p, uint n) {
    Txt x;
    x.a = p;
    x.al = n;
    x.b = p;
    x.bl = 0;
    x.dot = 0;
    return x;
}

// The byte of character c of x (n bytes; n when it has fewer), 32 bytes at a time: each chunk's characters' first
// bytes counted by a prefix over the lanes.
static inline uint char_byte(Txt x, uint n, long c, uint lane) {
    uint seen = 0;
    for (uint k = 0; k < n; k += 32) {
        bool first = k + lane < n && (txt_byte(x, k + lane) & 0xc0) != 0x80;
        uint before = simd_prefix_exclusive_sum(first ? 1u : 0u);
        uint hit = simd_min(first && long(seen + before) == c ? lane : 32u);
        if (hit < 32) return k + hit;
        seen += simd_sum(first ? 1u : 0u);
    }
    return n;
}

// The characters [a, b) a substring keeps of nc: `substr(s, i, n)` (f 3, SQLite's substr(s, i + 1, n)), SQLite's
// `substr` (f 7), `slice(s, start, end)` (f 4: negative from the end, clamped).
static inline void substr_range(uint f, long n0, long n1, long nc, thread long& a, thread long& b) {
    if (f != 4) {
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
        a = clamp(n0 < 0 ? nc + n0 : n0, 0l, nc);
        b = clamp(n1 < 0 ? nc + n1 : n1, 0l, nc);
        if (a > b) b = a;
    }
}

// A function's kept range of x [from, to) (`TEXTFN_*` f of numbers n0, n1), by a SIMD group 32 bytes at a time: head
// to the first ".", dir to the last "/" (by a minimum and a maximum over the lanes), substrings by characters counted by
// sums over the lanes, their first bytes found by prefixes; the whole text for the rest. `high`: x has a byte past
// ASCII (`lower`'s, f 0).
static inline void text_range(uint f, Txt x, long n0, long n1, uint lane, thread uint& from, thread uint& to, thread bool& high) {
    uint n = txt_len(x);
    from = 0;
    to = n;
    high = false;
    if (f == 1) {
        for (uint k = 0; k < n; k += 32) {
            uint d = simd_min(k + lane < n && txt_byte(x, k + lane) == uchar('.') ? lane : 32u);
            if (d < 32) { to = k + d; break; }
        }
    } else if (f == 2) {
        int last = -1;
        for (uint k = 0; k < n; k += 32) {
            int d = simd_max(k + lane < n && txt_byte(x, k + lane) == uchar('/') ? int(lane) : -1);
            if (d >= 0) last = int(k) + d;
        }
        to = last >= 0 ? uint(last) : 0;
    } else if (f == 3 || f == 4 || f == 7) {
        long nc = 0, a, b;
        for (uint k = 0; k < n; k += 32) nc += long(simd_sum(k + lane < n && (txt_byte(x, k + lane) & 0xc0) != 0x80 ? 1u : 0u));
        substr_range(f, n0, n1, nc, a, b);
        from = char_byte(x, n, a, lane);
        to = char_byte(x, n, b, lane);
    } else if (f == 0) {
        for (uint k = 0; k < n; k += 32) if (simd_any(k + lane < n && txt_byte(x, k + lane) >= 0x80)) high = true;
    }
}

// `replace_all` of a regex over x (its automata G, `text::table::Replacer`) by z, by a SIMD group 32 bytes at a time:
// each match's end by the forward automaton (the last match before it dies), its start by the reverse one run back
// from the end (the furthest match). The replaced text's length; written to o when `write`. f: the group's scratch
// (64 rows of the automata's states).
static inline uint rx_replace_txt(device const uchar* G, Txt x, Txt z, bool write, device uchar* o, device uchar* f, uint lane) {
    device const uint* h = (device const uint*)G;
    uint fS = h[0], fC = h[1], fdead = h[4], rS = h[5], rC = h[6], rdead = h[9];
    device const uchar* ft = G + h[10];
    device const uchar* rt = G + h[11];
    device const uchar* feoi = ft + 256 + fS * fC;
    device const uchar* facc = feoi + fS;
    device const uchar* reoi = rt + 256 + rS * rC;
    device const uchar* racc = reoi + rS;
    uint n = txt_len(x), lz = txt_len(z);
    uint pos = 0, w = 0;
    while (pos < n) {
        // The match's end: the forward automaton's last match before it dies (or at the end of the text).
        uint st = pos == 0 ? h[2] : h[3], last = NONE;
        bool dead = false;
        for (uint k = pos; k < n && !dead; k += 32) {
            uint m = min(32u, n - k);
            uint q = chunk_states(ft, fS, fC, x, int(k), 1, m, st, f, lane);
            uint d = simd_min(lane < m && q == fdead ? lane : 32u);
            int lm = simd_max(lane < m && lane < d && facc[q] != 0 ? int(lane) : -1);
            if (lm >= 0) last = k + uint(lm);
            if (d < 32) dead = true;
            else st = simd_shuffle(q, ushort(m - 1));
        }
        if (!dead && facc[feoi[st]] != 0) last = n;
        if (last == NONE) break;
        uint e = last;
        // Its start: the reverse automaton's furthest match back from the end, down to the byte before `pos`.
        uint lo = pos > 0 ? pos - 1 : 0, first = NONE;
        uint rs = e == n ? h[7] : h[8];
        bool rd = false;
        for (uint k = e; k > lo && !rd;) {
            uint m = min(32u, k - lo);
            uint q = chunk_states(rt, rS, rC, x, int(k) - 1, -1, m, rs, f, lane);
            uint d = simd_min(lane < m && q == rdead ? lane : 32u);
            int lm = simd_max(lane < m && lane < d && racc[q] != 0 ? int(lane) : -1);
            if (lm >= 0) first = k - uint(lm);
            if (d < 32) rd = true;
            else rs = simd_shuffle(q, ushort(m - 1));
            k -= m;
        }
        if (!rd && pos == 0 && racc[reoi[rs]] != 0) first = 0;
        if (first == NONE || first < pos) break;
        if (write) {
            for (uint i = lane; i < first - pos; i += 32) o[w + i] = txt_byte(x, pos + i);
            for (uint i = lane; i < lz; i += 32) o[w + first - pos + i] = txt_byte(z, i);
        }
        w += first - pos + lz;
        pos = e;
    }
    if (write) for (uint i = lane; i < n - pos; i += 32) o[w + i] = txt_byte(x, pos + i);
    return w + n - pos;
}

// Whether text test table `tab` (`text::table::Table`: S states of C classes, from `start`) accepts x, walked by a
// SIMD group 32 bytes at a time (`chunk_states`), the state after the end of text accepting.
static inline bool txt_verdict(device const uchar* tab, uint S, uint C, uint start, Txt x, device uchar* f, uint lane) {
    device const uchar* eoi = tab + 256 + S * C;
    device const uchar* acc = eoi + S;
    uint n = txt_len(x), st = start;
    for (uint k = 0; k < n; k += 32) {
        uint m = min(32u, n - k);
        uint q = chunk_states(tab, S, C, x, int(k), 1, m, st, f, lane);
        st = simd_shuffle(q, ushort(m - 1));
    }
    return acc[eoi[st]] != 0;
}

// `replace_all` of a regex, once per distinct term (decision 0038's chunked matching), its automata G
// (`text::table::Replacer`): a SIMD group per term (`rx_replace_txt`), the replacement Z (`lz` bytes). Pass 0 counts
// each term's replaced length into `lens`; after their scan (the offsets), pass 1 writes O as a text buffer
// (`DevText`'s: the terms' ids, offsets, bytes). Groups are persistent.
struct ReplaceParams { uint nterms; uint groups; uint lz; uint write; uint states; uint pad0; uint pad1; uint pad2; };

kernel void text_replace(device const long* T [[buffer(0)]], device const uchar* G [[buffer(1)]], device const uchar* Z [[buffer(2)]],
                         device uint* lens [[buffer(3)]], device long* O [[buffer(4)]], device uchar* scratch [[buffer(5)]],
                         constant ReplaceParams& p [[buffer(6)]], uint t [[thread_position_in_grid]], uint lane [[thread_index_in_simdgroup]]) {
    uint sg = t / 32;
    if (t == 0) {
        if (p.write == 0) lens[p.nterms] = 0;
        else {
            O[0] = long(p.nterms);
            ((device uint*)(O + 1 + p.nterms))[p.nterms] = lens[p.nterms];
        }
    }
    if (sg >= p.groups) return;
    device const long* ids = T + 1;
    device const uint* offs = text_offs(T);
    device const uchar* bytes = text_chars(T);
    device uchar* f = scratch + ulong(sg) * 64 * p.states;
    device uint* ooffs = (device uint*)(O + 1 + p.nterms);
    device uchar* obytes = (device uchar*)(ooffs + p.nterms + 1);
    Txt z = txt_raw(Z, p.lz);
    for (uint term = sg; term < p.nterms; term += p.groups) {
        Txt x = txt_raw(bytes + offs[term], offs[term + 1] - offs[term]);
        bool write = p.write != 0;
        uint len = rx_replace_txt(G, x, z, write, obytes + (write ? lens[term] : 0), f, lane);
        if (lane == 0) {
            if (write) {
                O[1 + term] = ids[term];
                ooffs[term] = lens[term];
            } else lens[term] = len;
        }
    }
}

// `replace_all` of literal y (not empty; empty: x itself) in x by z, by a SIMD group: each lane tests whether a match
// starts at its byte (compared a word of eight at a time), the leftmost of a chunk taken, the next looked for past it
// (matches not overlapping). The replaced text's length; written to o when `write`.
static inline uint lit_replace_txt(Txt x, Txt y, Txt z, bool write, device uchar* o, uint lane) {
    uint n = txt_len(x), ly = txt_len(y), lz = txt_len(z), pos = 0, w = 0;
    while (ly > 0 && pos + ly <= n) {
        uint m = NONE;
        for (uint k = pos; k + ly <= n; k += 32) {
            uint at = k + lane;
            uint d = simd_min(at + ly <= n && txt_same(x, at, y, 0, ly) ? lane : 32u);
            if (d < 32) { m = k + d; break; }
        }
        if (m == NONE) break;
        if (write) {
            for (uint i = lane; i < m - pos; i += 32) o[w + i] = txt_byte(x, pos + i);
            for (uint i = lane; i < lz; i += 32) o[w + m - pos + i] = txt_byte(z, i);
        }
        w += m - pos + lz;
        pos = m + ly;
    }
    if (write) for (uint i = lane; i < n - pos; i += 32) o[w + i] = txt_byte(x, pos + i);
    return w + n - pos;
}

constant ulong POW10[20] = {1ul, 10ul, 100ul, 1000ul, 10000ul, 100000ul, 1000000ul, 10000000ul, 100000000ul, 1000000000ul,
    10000000000ul, 100000000000ul, 1000000000000ul, 10000000000000ul, 100000000000000ul, 1000000000000000ul,
    10000000000000000ul, 100000000000000000ul, 1000000000000000000ul, 10000000000000000000ul};

// Mode 10, a made text per row, by a SIMD group a row (decision 0038's chunked matching, per binding): before its
// scatter, `row_text` gives each row t of L its keep bit (counts[t]) and its value (V[t]), by op agg[0] & 15 of X
// (keys 0, 1, kind agg[1] & 15) and Y (keys 2, 3, kind agg[1] >> 4): 0 `cat`'s text of X and Y ("." between when X
// is not empty), 1 `str_concat`'s, 2 `replace_all` of regex G in X by Y, made into the arena (a bump of the group's
// lane 0; full: the stage's counter word's bit 1, the value -1); 3 a text test of X by table G (P: its states,
// classes, start), kept as agg[0] bit 8 wants. P[0]: the groups (persistent), P[1]: the states a group's scratch holds.
kernel void row_text(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]], device atomic_uint* A [[buffer(2)]],
                     device const uint* totals [[buffer(3)]], device atomic_uint* counters [[buffer(4)]], device uint* counts [[buffer(5)]],
                     device long* V [[buffer(6)]], device const uchar* G [[buffer(7)]], device uchar* scratch [[buffer(8)]],
                     device const uint* P [[buffer(9)]], constant Stage& s [[buffer(10)]], uint t [[thread_position_in_grid]],
                     uint lane [[thread_index_in_simdgroup]]) {
    uint n = min(totals[s.in_slot], s.cap_in);
    uint groups = P[0];
    for (uint r = n + t; r <= s.cap_in; r += groups * 32) counts[r] = 0;
    uint sg = t / 32;
    if (sg >= groups) return;
    device uchar* fs = scratch + ulong(sg) * 64 * P[1];
    uint op = s.agg[0] & 15;
    bool want = ((s.agg[0] >> 8) & 1) != 0;
    for (uint row = sg; row < n; row += groups) {
        Txt x = txt_of(s, L, row, R, A, 0, s.agg[1] & 15);
        if (op == 3) {
            bool v = txt_verdict(G, P[2], P[3], P[4], x, fs, lane);
            if (lane == 0) counts[row] = uint(v == want);
            continue;
        }
        Txt y = txt_of(s, L, row, R, A, 2, s.agg[1] >> 4), z = y;
        uint f = (s.agg[0] >> 16) & 255;
        uint lx = txt_len(x), len = 0, dot = 0, from = 0, to = 0, ndig = 0, pad = 0;
        ulong u = 0;
        bool neg = false;
        if (op == 0 || op == 1) {
            dot = op == 0 && lx > 0 ? 1 : 0;
            len = lx + dot + txt_len(y);
        } else if (op == 2) {
            len = rx_replace_txt(G, x, y, false, (device uchar*)A, fs, lane);
        } else if (f == 5 || f == 11) {
            // to_string(n): its decimal digits, a lane each (at most 20). zero_pad(n, w): zeros after the sign up to
            // w columns (0 to 64, as the host's `{:0w$}`).
            long n0 = key_value(s, L, row, 5);
            neg = n0 < 0;
            u = neg ? ulong(-(n0 + 1)) + 1 : ulong(n0);
            ndig = 1;
            while (ndig < 20 && u >= POW10[ndig]) ndig++;
            len = ndig + (neg ? 1 : 0);
            if (f == 11) {
                uint w = uint(clamp(key_value(s, L, row, 6), 0l, 64l));
                pad = w > len ? w - len : 0;
                len += pad;
            }
        } else if (f == 6) {
            // replace_all of a literal pattern (key 2) by key 4's text.
            y = txt_of(s, L, row, R, A, 2, 0);
            z = txt_of(s, L, row, R, A, 4, 0);
            len = lit_replace_txt(x, y, z, false, (device uchar*)A, lane);
        } else {
            bool high;
            text_range(f, x, key_value(s, L, row, 5), key_value(s, L, row, 6), lane, from, to, high);
            if (f == 0 && high) {
                // lower of a name past ASCII: Unicode's mapping is the host's.
                if (lane == 0) {
                    if (s.counter != NONE) atomic_fetch_or_explicit(&counters[s.counter], 2u, memory_order_relaxed);
                    V[row] = -1;
                    counts[row] = 1;
                }
                continue;
            }
            len = to - from;
        }
        uint off = 0;
        if (lane == 0) off = atomic_fetch_add_explicit(A, len, memory_order_relaxed);
        off = simd_shuffle(off, ushort(0));
        uint cap = ((device const uint*)A)[1];
        bool full = ulong(off) + len > cap;
        if (!full) {
            device uchar* o = (device uchar*)A + 16 + off;
            if (op == 2) rx_replace_txt(G, x, y, true, o, fs, lane);
            else if (op <= 1) for (uint i = lane; i < len; i += 32) o[i] = i < lx ? txt_byte(x, i) : (i < lx + dot ? uchar('.') : txt_byte(y, i - lx - dot));
            else if (f == 6) lit_replace_txt(x, y, z, true, o, lane);
            else if (f == 5 || f == 11) {
                uint sign = neg ? 1 : 0;
                for (uint i = lane; i < len; i += 32) {
                    uint d = i - sign - pad;
                    o[i] = neg && i == 0 ? uchar('-') : i < sign + pad ? uchar('0') : uchar('0' + (u / POW10[ndig - 1 - d]) % 10);
                }
            } else {
                for (uint i = lane; i < len; i += 32) {
                    uchar c = txt_byte(x, from + i);
                    if (f == 0 || f == 8) c = (c >= 'A' && c <= 'Z') ? c + 32 : c;
                    else if (f == 9) c = (c >= 'a' && c <= 'z') ? c - 32 : c;
                    o[i] = c;
                }
            }
        }
        if (lane == 0) {
            if (full && s.counter != NONE) atomic_fetch_or_explicit(&counters[s.counter], 1u, memory_order_relaxed);
            V[row] = full ? -1 : arena_value(off, len);
            counts[row] = 1;
        }
    }
}

// Mode 10's rows out: those `row_text` kept, each its value (V's) in `bind_slot[0]` but for a test's.
kernel void row_scatter(device const long* L [[buffer(0)]], device const long* V [[buffer(1)]], device uint* totals [[buffer(2)]],
                        device const uint* offsets [[buffer(3)]], device long* O [[buffer(4)]], constant Stage& s [[buffer(5)]],
                        uint t [[thread_position_in_grid]]) {
    uint n = min(totals[s.in_slot], s.cap_in);
    if (t == 0) totals[s.out_slot] = offsets[n];
    if (t >= n) return;
    uint at = offsets[t];
    if (offsets[t + 1] <= at || at >= s.cap_out) return;
    for (uint k = 0; k < s.nslots; k++) O[ulong(k) * s.cap_out + at] = L[ulong(k) * s.cap_in + t];
    if ((s.agg[0] & 15) != 3) O[ulong(s.bind_slot[0]) * s.cap_out + at] = V[t];
}

// A text function of constant arguments (`TEXTFN_*`: f), once per distinct term (decision 0038's chunked matching):
// a SIMD group per term, 32 bytes at a time. The kept range of its text (head: to the first "."; dir: to the last
// "/"; substr, sql_substr, slice: characters counted by sums over the lanes, their first bytes found by prefixes), or
// its bytes mapped (lower, upper). `lower` of a term past ASCII sets `flag` (Unicode's mapping is the host's). Pass 0
// counts each term's length into `lens`; after their scan, pass 1 writes O as a text buffer (`DevText`'s).
struct MapParams { uint nterms; uint groups; uint f; uint write; long n0; long n1; };

kernel void text_map(device const long* T [[buffer(0)]], device uint* lens [[buffer(1)]], device long* O [[buffer(2)]],
                     device atomic_uint* flag [[buffer(3)]], constant MapParams& p [[buffer(4)]], uint t [[thread_position_in_grid]],
                     uint lane [[thread_index_in_simdgroup]]) {
    uint sg = t / 32;
    if (t == 0) {
        if (p.write == 0) lens[p.nterms] = 0;
        else {
            O[0] = long(p.nterms);
            ((device uint*)(O + 1 + p.nterms))[p.nterms] = lens[p.nterms];
        }
    }
    if (sg >= p.groups) return;
    device const long* ids = T + 1;
    device const uint* offs = text_offs(T);
    device const uchar* bytes = text_chars(T);
    device uint* ooffs = (device uint*)(O + 1 + p.nterms);
    device uchar* obytes = (device uchar*)(ooffs + p.nterms + 1);
    uint f = p.f;
    for (uint term = sg; term < p.nterms; term += p.groups) {
        device const uchar* x = bytes + offs[term];
        uint n = offs[term + 1] - offs[term];
        uint from, to;
        bool high;
        text_range(f, txt_raw(x, n), p.n0, p.n1, lane, from, to, high);
        if (f == 0 && high && lane == 0 && p.write == 0) atomic_fetch_or_explicit(flag, 1u, memory_order_relaxed);
        uint len = to - from;
        if (p.write == 0) {
            if (lane == 0) lens[term] = len;
            continue;
        }
        device uchar* o = obytes + lens[term];
        for (uint i = lane; i < len; i += 32) {
            uchar c = x[from + i];
            if (f == 0 || f == 8) c = (c >= 'A' && c <= 'Z') ? c + 32 : c;
            else if (f == 9) c = (c >= 'a' && c <= 'z') ? c - 32 : c;
            o[i] = c;
        }
        if (lane == 0) {
            O[1 + term] = ids[term];
            ooffs[term] = lens[term];
        }
    }
}

// Rows copied (decision 0325 step 5): `n` rows of `ncols` columns (as many as slot `count_slot` of `totals` says, at
// most `n`; all `n` when it is NONE), column k of the destination from source column `src[k]` (or the constant
// `konst[k]` where it is NONE), into rows `dst_off..`. Each side is a binding table (column-major, its stride; its
// offset 0) or a relation in a block: read through the reads written once (`clif_rel_get`, the relation's descriptor
// at `src_at`), written through them (`clif_rows_at`, its rows' descriptor at `dst_rows_at`).
struct Copy {
    long konst[16];
    uint src[16];
    uint n, ncols, src_stride, dst_stride, dst_off, count_slot, pad0, pad1;
    ulong src_at, dst_rows_at;
};

kernel void rows_copy(device const uchar* S [[buffer(0)]], device uchar* D [[buffer(1)]], device const uint* totals [[buffer(2)]],
                      constant Copy& c [[buffer(3)]], uint t [[thread_position_in_grid]]) {
    uint n = c.count_slot == NONE ? c.n : min(totals[c.count_slot], c.n);
    if (t >= n) return;
    for (uint k = 0; k < c.ncols && k < 16; k++) {
        uint from = c.src[k];
        long v = from == NONE ? c.konst[k]
               : c.src_at == 0ul ? ((device const long*)S)[ulong(from) * c.src_stride + t]
               : long(clif_rel_get(S, c.src_at, t, from));
        if (c.dst_rows_at == 0ul) ((device long*)D)[ulong(k) * c.dst_stride + c.dst_off + t] = v;
        else *(device long*)(D + clif_rows_at((device const uchar*)D, c.dst_rows_at, c.dst_off + t, k)) = v;
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
    // The relation's descriptor's offset in its block (`p[3]`, `p[4]`: low, high).
    ulong at = ulong(p[3]) | (ulong(p[4]) << 32);
    long kv[8];
    for (uint i = 0; i < p[1] && i < 8; i++) {
        uint c = p[5 + i];
        kv[i] = long(clif_rel_get((device const uchar*)R, at, t, c));
    }
    out[t] = hash_keys(kv, p[1], p[2]);
}

// A lattice relation's rounds on the device (decision 0124): a round's rows `U` (a table of `k` slots: the key's
// `nkeys` columns, then the value) folded by key into the held relation `R` (its `k` columns and a change count,
// stride `had`). `P` groups `U`'s rows by key (each place the least row of its key), `best` the key's chosen row
// (min and max select one of their values), `flags` a flat key's conflict. A held key's row is updated in place; a
// new key's row goes to `N` (as `R`'s columns), found next by `H` (key to `R`'s row, kept from round to round). Each
// changed key's row (key and value) goes to `D`, the next round's delta: `counters[0]` of them, `counters[1]` new.
// `level`: the round, written to a changed or new key's level column (`nkeys + 2`) where the relation keeps levels
// (decision 0311); 0: no level column.
struct LatP { long top; uint n; uint had; uint nkeys; uint code; uint after; uint pmask; uint hmask; uint level; ulong r_at, r_rows_at; };

// The held relation `R`'s row `row` in column `c`, and the place to write it: `R` is a relation in its block
// (decision 0325 step 5), its descriptor at `r_at`, its rows' at `r_rows_at`.
static inline long lat_r(device const uchar* R, constant LatP& p, uint row, uint c) {
    return long(clif_rel_get(R, p.r_at, row, c));
}
static inline device long* lat_rp(device uchar* R, constant LatP& p, uint row, uint c) {
    return (device long*)(R + clif_rows_at((device const uchar*)R, p.r_rows_at, row, c));
}

static inline uint lat_hash(thread const long* key, uint nk, uint mask) {
    ulong h = 0x9E3779B97F4A7C15ul;
    for (uint c = 0; c < nk && c < 8; c++) { h ^= ulong(key[c]); h *= 0xff51afd7ed558ccdul; h ^= h >> 33; }
    return uint(h) & mask;
}

static inline bool lat_same(device const long* T, uint stride, uint a, uint b, uint nk) {
    for (uint c = 0; c < nk && c < 8; c++) if (T[ulong(c) * stride + a] != T[ulong(c) * stride + b]) return false;
    return true;
}

kernel void lat_clear(device uint* P [[buffer(0)]], device uint* best [[buffer(1)]], device uint* flags [[buffer(2)]],
                      constant LatP& p [[buffer(3)]], uint i [[thread_position_in_grid]]) {
    if (i > p.pmask) return;
    P[i] = 0xffffffffu;
    best[i] = 0xffffffffu;
    flags[i] = 0;
}

kernel void lat_hclear(device uint* H [[buffer(0)]], constant LatP& p [[buffer(1)]], uint i [[thread_position_in_grid]]) {
    if (i <= p.hmask) H[i] = 0xffffffffu;
}

// `R`'s row `t` into `H` (a table grown: every held row placed again).
kernel void lat_rehash(device const uchar* R [[buffer(0)]], device atomic_uint* H [[buffer(1)]], constant LatP& p [[buffer(2)]],
                       uint t [[thread_position_in_grid]]) {
    if (t >= p.had) return;
    long key[8];
    for (uint c = 0; c < p.nkeys && c < 8; c++) key[c] = lat_r(R, p, t, c);
    uint at = lat_hash(key, p.nkeys, p.hmask);
    // Bounded: a place at a time, and a weak exchange's spurious failure tries the place again.
    for (uint k = 0; k < 2 * (p.hmask + 1); k++) {
        uint seen = 0xffffffffu;
        if (atomic_compare_exchange_weak_explicit(&H[at], &seen, t, memory_order_relaxed, memory_order_relaxed)) return;
        if (seen == 0xffffffffu) continue;
        at = (at + 1) & p.hmask;
    }
}

kernel void lat_claim(device const long* U [[buffer(0)]], device atomic_uint* P [[buffer(1)]], constant LatP& p [[buffer(2)]],
                      uint x [[thread_position_in_grid]]) {
    if (x >= p.n) return;
    long key[8];
    for (uint c = 0; c < p.nkeys && c < 8; c++) key[c] = U[ulong(c) * p.n + x];
    uint at = lat_hash(key, p.nkeys, p.pmask);
    for (uint k = 0; k < 2 * (p.pmask + 1); k++) {
        uint seen = 0xffffffffu;
        if (atomic_compare_exchange_weak_explicit(&P[at], &seen, x, memory_order_relaxed, memory_order_relaxed)) return;
        if (seen == 0xffffffffu) continue;
        if (lat_same(U, p.n, seen, x, p.nkeys)) { atomic_fetch_min_explicit(&P[at], x, memory_order_relaxed); return; }
        at = (at + 1) & p.pmask;
    }
}

// The place of row `x`'s key in `P` (claimed: it is there).
static inline uint lat_place(device const long* U, device const uint* P, constant LatP& p, uint x) {
    long key[8];
    for (uint c = 0; c < p.nkeys && c < 8; c++) key[c] = U[ulong(c) * p.n + x];
    uint at = lat_hash(key, p.nkeys, p.pmask);
    for (uint k = 0; k <= p.pmask; k++) {
        uint y = P[at];
        if (y == 0xffffffffu) return 0xffffffffu;
        if (lat_same(U, p.n, y, x, p.nkeys)) return at;
        at = (at + 1) & p.pmask;
    }
    return 0xffffffffu;
}

// Code 3 (`shortlex_min`, decision 0307): whether arena text `a` is before `b`, by their characters, then their
// bytes (as `ShortlexMin`: `(chars, bytes)`), a word of eight at a time.
static inline bool lat_text_less(device atomic_uint* A, long a, long b) {
    ulong ua = ulong(a), ub = ulong(b);
    Txt x = txt_raw(arena_bytes(A) + uint((ua & 0x7ffffffffffffffful) >> 24), uint(ua & 0xfffffful));
    Txt y = txt_raw(arena_bytes(A) + uint((ub & 0x7ffffffffffffffful) >> 24), uint(ub & 0xfffffful));
    uint cx = txt_chars(x), cy = txt_chars(y);
    if (cx != cy) return cx < cy;
    uint lx = txt_len(x), ly = txt_len(y), m = min(lx, ly);
    for (uint at = 0; at < m; at += 8) {
        ulong u = txt_word(x, at, m), v = txt_word(y, at, m);
        if (u != v) return u < v;
    }
    return lx < ly;
}

// Whether value `a` (of row `ra`) is chosen over `b` (of `rb`): the top absorbs; min the least, max the greatest,
// shortlex the least text (code 3, over the arena `A`); a tie to the first row.
static inline bool lat_better(constant LatP& p, device atomic_uint* A, long a, uint ra, long b, uint rb) {
    if (b == p.top) return false;
    if (a == p.top) return true;
    if (a == b) return ra < rb;
    if (p.code == 3) {
        if (lat_text_less(A, a, b)) return true;
        return !lat_text_less(A, b, a) && ra < rb;
    }
    return p.code == 0 ? a < b : a > b;
}

kernel void lat_best(device const long* U [[buffer(0)]], device const uint* P [[buffer(1)]], device atomic_uint* best [[buffer(2)]],
                     device atomic_uint* flags [[buffer(3)]], device atomic_uint* A [[buffer(4)]], constant LatP& p [[buffer(5)]],
                     uint x [[thread_position_in_grid]]) {
    if (x >= p.n) return;
    uint s = lat_place(U, P, p, x);
    if (s == 0xffffffffu) return;
    long v = U[ulong(p.nkeys) * p.n + x];
    if (p.code == 2) {
        if (v != U[ulong(p.nkeys) * p.n + P[s]]) atomic_fetch_or_explicit(&flags[s], 1u, memory_order_relaxed);
        return;
    }
    uint cur = atomic_load_explicit(&best[s], memory_order_relaxed);
    for (uint k = 0; k < 4 * p.n + 8; k++) {
        if (cur != 0xffffffffu && !lat_better(p, A, v, x, U[ulong(p.nkeys) * p.n + cur], cur)) return;
        if (atomic_compare_exchange_weak_explicit(&best[s], &cur, x, memory_order_relaxed, memory_order_relaxed)) return;
    }
}

// The join of two present values (`code`: 0 min, 1 max, 2 flat, 3 shortlex over the arena `A`, a tie to `a`), the
// top absorbing.
static inline long lat_lub(constant LatP& p, device atomic_uint* A, long a, long b) {
    if (a == p.top || b == p.top) return p.top;
    if (p.code == 3) return lat_text_less(A, b, a) ? b : a;
    if (p.code == 0) return min(a, b);
    if (p.code == 1) return max(a, b);
    return a == b ? a : p.top;
}

// A key changed past `after` times: min's or max's value moving on is its top (a finite chain); flat's and
// shortlex's are kept (finite lattices: `widen` keeps the route, decision 0124).
static inline long lat_widen(constant LatP& p, long old, long now) {
    if (p.code == 0 && now < old) return p.top;
    if (p.code == 1 && now > old) return p.top;
    return now;
}

kernel void lat_merge(device const long* U [[buffer(0)]], device const uint* P [[buffer(1)]], device const uint* best [[buffer(2)]],
                      device const uint* flags [[buffer(3)]], device uchar* R [[buffer(4)]], device atomic_uint* H [[buffer(5)]],
                      device long* N [[buffer(6)]], device long* D [[buffer(7)]], device atomic_uint* counters [[buffer(8)]],
                      device atomic_uint* A [[buffer(9)]], constant LatP& p [[buffer(10)]], uint s [[thread_position_in_grid]]) {
    if (s > p.pmask) return;
    uint rep = P[s];
    if (rep == 0xffffffffu) return;
    uint r = (p.code == 2 || best[s] == 0xffffffffu) ? rep : best[s];
    long v = U[ulong(p.nkeys) * p.n + r];
    if (p.code == 2 && flags[s] != 0) v = p.top;
    long key[8];
    for (uint c = 0; c < p.nkeys && c < 8; c++) key[c] = U[ulong(c) * p.n + r];
    uint k = p.nkeys + 1;
    uint at = lat_hash(key, p.nkeys, p.hmask);
    uint held = 0xffffffffu;
    for (uint i = 0; i <= p.hmask; i++) {
        uint h = atomic_load_explicit(&H[at], memory_order_relaxed);
        if (h == 0xffffffffu) break;
        bool same = h < p.had;
        for (uint c = 0; same && c < p.nkeys && c < 8; c++) same = lat_r(R, p, h, c) == key[c];
        if (same) { held = h; break; }
        at = (at + 1) & p.hmask;
    }
    if (held != 0xffffffffu) {
        long o = lat_r(R, p, held, p.nkeys);
        long m = lat_lub(p, A, o, v);
        if (m == o) return;
        long changes = lat_r(R, p, held, k);
        if (p.after != 0xffffffffu && changes >= long(p.after)) {
            m = lat_widen(p, o, m);
            if (m == o) return;
        }
        *lat_rp(R, p, held, p.nkeys) = m;
        *lat_rp(R, p, held, k) = changes + 1;
        if (p.level != 0) *lat_rp(R, p, held, k + 1) = long(p.level);
        v = m;
    } else {
        uint a = atomic_fetch_add_explicit(&counters[1], 1u, memory_order_relaxed);
        for (uint c = 0; c < p.nkeys && c < 8; c++) N[ulong(c) * p.n + a] = key[c];
        N[ulong(p.nkeys) * p.n + a] = v;
        N[ulong(k) * p.n + a] = 1;
        if (p.level != 0) N[ulong(k + 1) * p.n + a] = long(p.level);
        uint id = p.had + a;
        uint place = lat_hash(key, p.nkeys, p.hmask);
        for (uint i = 0; i < 2 * (p.hmask + 1); i++) {
            uint seen = 0xffffffffu;
            if (atomic_compare_exchange_weak_explicit(&H[place], &seen, id, memory_order_relaxed, memory_order_relaxed)) break;
            if (seen == 0xffffffffu) continue;
            place = (place + 1) & p.hmask;
        }
    }
    uint d = atomic_fetch_add_explicit(&counters[0], 1u, memory_order_relaxed);
    for (uint c = 0; c < p.nkeys && c < 8; c++) D[ulong(c) * p.n + d] = key[c];
    D[ulong(p.nkeys) * p.n + d] = v;
}
"#;

/// Whether this machine has the Metal device the stages run on.
pub fn available() -> bool {
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

/// A kind-set axis's batch solve on the device, written into `chunk` (decision 0289 §2, `facts::nearest::ChunkAxis`):
/// each node's rows counted by a climb of its ancestors, the counts scanned into the starts, then the pairs written,
/// in one submission. The pairs' space is sized before the count: a nearest axis has at most a row per node, so its
/// bound is exact; containment's is `INSIDE_ROWS` a node, and the rare batch past it writes again once its total is
/// read. The space past the total is given back to the block. The starts and the pairs are page-aligned in the
/// block, each bound as a buffer of its own with no copy; the block reserves address space, so the bound's pages
/// the device never writes are never memory.
pub fn axis_in_chunk(chunk: &crate::code_rule_dsl::facts::vtab::syntax::device_chunk::Chunk, bits: Option<&[u32]>, inside: bool)
                     -> Result<crate::code_rule_dsl::facts::nearest::InChunk, String> {
    axis_in_chunk_bounded(chunk, bits, inside, INSIDE_ROWS)
}

/// Containment's rows a node that the pairs' space is sized for before the count.
const INSIDE_ROWS: usize = 8;

/// `axis_in_chunk`, with containment's space sized for `inside_rows` a node (a test makes it 0, to write again).
pub(in crate::code_rule_dsl) fn axis_in_chunk_bounded(chunk: &crate::code_rule_dsl::facts::vtab::syntax::device_chunk::Chunk,
                                                      bits: Option<&[u32]>, inside: bool, inside_rows: usize)
                                                      -> Result<crate::code_rule_dsl::facts::nearest::InChunk, String> {
    use crate::code_rule_dsl::facts::vtab::syntax::device_chunk::ChunkRoot;
    const PAGE: usize = 16384;
    let b = &chunk.block;
    // SAFETY: the chunk's root, which `device_chunk::build` wrote.
    let n = unsafe { (*b.ptr::<ChunkRoot>(chunk.root)).nodes } as usize;
    let full = || "the chunk block cannot hold the axis".to_string();
    let bits_at = match bits {
        Some(words) => {
            let p: *mut u32 = b.try_alloc_t(words.len()).ok_or_else(full)?;
            // SAFETY: just allocated for `words.len()` values.
            unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), p, words.len()) };
            b.offset_of(p)
        }
        None => 0,
    };
    let start_bytes = (4 * (n + 1)).next_multiple_of(PAGE);
    let start_at = b.try_alloc(start_bytes, PAGE).ok_or_else(full)?;
    let cap = if inside { inside_rows * n } else { n }.min(u32::MAX as usize);
    let cap_bytes = (8 * cap).max(8).next_multiple_of(PAGE);
    let pairs_at = b.try_alloc(cap_bytes, PAGE).ok_or_else(full)?;
    let device = rerail_accel::gpu::join::device()?;
    let whole = b.gpu_buffer(device);
    let part = |at: usize, bytes: usize| {
        device.new_buffer_with_bytes_no_copy(b.ptr::<u8>(at) as *const std::ffi::c_void, bytes as u64, metal::MTLResourceOptions::StorageModeShared, None)
    };
    let starts = part(start_at, start_bytes);
    let pairs = part(pairs_at, cap_bytes);
    let count = pipeline("chunk_views", "axis_count", chunk_views_source)?;
    let write = pipeline("chunk_views", "axis_write", chunk_views_source)?;
    let at = |o: usize| u32::try_from(o).map_err(|_| "the axis lies past 4 GB of its chunk".to_string());
    let p = [at(chunk.root)?, n as u32, at(bits_at)?, u32::from(inside), cap as u32];
    let scan = Scan::owned(n + 1)?;
    let mut batch = Batch::new().map_err(|e| format!("{e:?}"))?;
    let pass = batch.pass(17).map_err(|e| format!("{e:?}"))?;
    pass.run(&count, &[&whole, &starts], &p, &[], groups(n + 1));
    scan.encode(pass, &starts)?;
    if n > 0 {
        pass.run(&write, &[&whole, &starts, &pairs], &p, &[], groups(n));
    }
    batch.finish().map_err(|e| format!("{e:?}"))?;
    // SAFETY: the scan completed: its last value is the total.
    let len = unsafe { *b.ptr::<u32>(start_at + 4 * n) } as usize;
    if len <= cap {
        b.shrink_last(pairs_at, cap_bytes, (8 * len).max(8).next_multiple_of(PAGE));
        return Ok(crate::code_rule_dsl::facts::nearest::InChunk { start: start_at, pairs: pairs_at, len });
    }
    // Past the bound: the write was skipped, and is made again into space of the total's size.
    let _span = crate::code_rule_dsl::logic::profile::span(|| format!("containment past {inside_rows} rows a node: written again"));
    b.shrink_last(pairs_at, cap_bytes, 0);
    let pairs_bytes = (8 * len).next_multiple_of(PAGE);
    let pairs_at = b.try_alloc(pairs_bytes, PAGE).ok_or_else(full)?;
    let pairs = part(pairs_at, pairs_bytes);
    let whole = b.gpu_buffer(device);
    let p = [p[0], p[1], p[2], p[3], len as u32];
    let mut batch = Batch::new().map_err(|e| format!("{e:?}"))?;
    let pass = batch.pass(1).map_err(|e| format!("{e:?}"))?;
    pass.run(&write, &[&whole, &starts, &pairs], &p, &[], groups(n));
    batch.finish().map_err(|e| format!("{e:?}"))?;
    Ok(crate::code_rule_dsl::facts::nearest::InChunk { start: start_at, pairs: pairs_at, len })
}

/// A relation's columns on the device: `rows` 64-bit values each, column-major.
pub(in crate::code_rule_dsl) struct DevRel {
    buffer: Held,
    pub(in crate::code_rule_dsl) rows: usize,
    pub(in crate::code_rule_dsl) cols: usize,
    /// Its relation's descriptor's offset in the block `buffer` binds (decision 0325): every relation the device
    /// reads or writes lies in a block, read through `rel_get` and written through `rows_at`. 0 for no relation:
    /// words a raw mode reads (a text's names, a test's verdict bits).
    pub(in crate::code_rule_dsl) at: u64,
    /// Its dead rows' delta's offset in that block (`Stage::r_dead`); 0 when it has none.
    pub(in crate::code_rule_dsl) dead: u64,
    /// The relation, when it is the device's own (`empty`, `upload`): what the host reads it by once the device
    /// wrote it.
    part: Option<crate::code_rule_dsl::facts::rows::Part>,
}

impl DevRel {
    /// A relation of the device's own, of `cols` columns and `rows` rows, in a block of the device's
    /// (`Part::for_device`, decision 0325 step 5), every row the device's to write.
    pub(in crate::code_rule_dsl) fn empty(cols: usize, rows: usize) -> Result<DevRel, Error> {
        let part = crate::code_rule_dsl::facts::rows::Part::for_device(cols, rows).map_err(|_| Error::BackendCapacity)?;
        let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
        let buffer = Held::kept(part.facts().block.gpu_buffer(device), Box::new(part.facts().clone()));
        debug_assert_eq!(part.rows_at(), part.rel_at() + REL_ROWS as usize);
        Ok(DevRel { buffer, rows, cols, at: part.rel_at() as u64, dead: 0, part: Some(part) })
    }

    /// A relation of `cols` columns and `rows` rows, its values given by `fill` (column `c` at
    /// `[c * rows .. (c + 1) * rows]`) and written by the host into a relation of the device's own.
    pub(in crate::code_rule_dsl) fn upload(cols: usize, rows: usize, fill: impl FnOnce(&mut [i64])) -> Result<DevRel, Error> {
        let mut values = vec![0i64; cols * rows];
        fill(&mut values);
        let dev = DevRel::empty(cols, rows)?;
        let rel = dev.part.as_ref().expect("its own").rel();
        for c in 0..cols {
            for (r, &v) in values[c * rows..(c + 1) * rows].iter().enumerate() {
                // SAFETY: row `r` is claimed (`for_device`), and nothing else writes the relation yet.
                unsafe { rel.rows.put(r as u32, c, v) };
            }
        }
        Ok(dev)
    }

    /// `words` 64-bit words a raw mode reads (not a relation), given by `fill`.
    pub(in crate::code_rule_dsl) fn words(words: usize, fill: impl FnOnce(&mut [i64])) -> Result<DevRel, Error> {
        let buffer = Held::new(8 * words.max(1))?;
        // SAFETY: the buffer holds at least `words` values, and nothing else refers to it yet.
        fill(unsafe { std::slice::from_raw_parts_mut(buffer.buffer.contents().cast::<i64>(), words) });
        Ok(DevRel::raw(buffer, words))
    }

    /// The words of `buffer`, `words` of them (not a relation).
    fn raw(buffer: Held, words: usize) -> DevRel {
        DevRel { buffer, rows: words, cols: 1, at: 0, dead: 0, part: None }
    }

    /// A relation the device reads where it lies (decision 0325): the relation of `rows` rows and `cols` columns
    /// whose descriptor is `at` bytes into the block `buffer` binds. Nothing is copied.
    pub(in crate::code_rule_dsl) fn in_block(buffer: Held, at: usize, dead: usize, rows: usize, cols: usize) -> DevRel {
        assert!(at > 0, "a descriptor past the block's header");
        DevRel { buffer, rows, cols, at: at as u64, dead: dead as u64, part: None }
    }

    /// Its rows' descriptor's offset in its block: what a writer's `rows_at` takes.
    pub(in crate::code_rule_dsl) fn rows_at(&self) -> u64 {
        assert!(self.at != 0, "words, not a relation");
        self.at + REL_ROWS
    }

    /// The device memory it holds of its own: its relation's values, or its words; none for one read in another's
    /// block.
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        match (&self.part, self.at) {
            (Some(_), _) | (None, 0) => 8 * self.cols * self.rows,
            _ => 0,
        }
    }

    /// The buffer it is bound by: its block's, or its words'.
    pub(in crate::code_rule_dsl) fn held(&self) -> &Held {
        &self.buffer
    }

    /// Column `c`'s values, once the command buffer that wrote them completed: a relation of the device's own, read
    /// where it lies.
    pub(in crate::code_rule_dsl) fn column(&self, c: usize) -> Vec<i64> {
        let rel = self.part.as_ref().expect("a relation of the device's own").rel();
        (0..self.rows as u32).map(|r| rel.rows.get(r, c)).collect()
    }

    /// Words `0..n` (`words`, or a text's), once the command buffer that wrote them completed.
    pub(in crate::code_rule_dsl) fn word_slice(&self, n: usize) -> &[i64] {
        assert_eq!(self.at, 0, "a relation's values are read by column");
        // SAFETY: the buffer holds at least `n` words; the writer completed.
        unsafe { std::slice::from_raw_parts(self.buffer.buffer.contents().cast::<i64>(), n.min(self.rows)) }
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
    /// The names' bytes, all told.
    pub(in crate::code_rule_dsl) chars: usize,
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
        let nchars = chars.len();
        Ok(DevText { packed: std::rc::Rc::new(DevRel::raw(held, rows)), terms: n, span, longest, sep, chars: nchars })
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
    /// Its buffer, its rows (a table's capacity, a relation's rows), and its descriptors' offsets: the relation's
    /// and its rows' (0 for a table, column-major).
    fn of(&self) -> (&BufferRef, usize, u64, u64) {
        match self {
            Rows::Table(t) => (&t.buffer.buffer, t.cap, 0, 0),
            Rows::Rel(r) => {
                assert!(r.at != 0, "a copy of words, not a relation");
                (&r.buffer.buffer, r.rows, r.at, r.rows_at())
            }
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
    count_slot: u32,
    pad: [u32; 2],
    src_at: u64,
    dst_rows_at: u64,
}

const _: () = assert!(std::mem::size_of::<CopyParams>() == 240);

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

/// A relation's rows' descriptor's offset from its own (`rerail_blocks`' layout).
const REL_ROWS: u64 = std::mem::offset_of!(rerail_blocks::relation::Relation, rows) as u64;

/// A binding table on the device: `slots` columns of `cap` 64-bit values, column-major.
/// The device buffers' budget left in the process (other lanes hold theirs: the syntax side's tapes most).
/// What the device joins may still lease of the device's budget: their own leases count (`relational_bytes_left`).
pub(in crate::code_rule_dsl) fn device_bytes_left() -> usize {
    super::metal_backend::relational_bytes_left()
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

/// A stage whose table's space is past this many rows has its grids sized on the device (`Scan::size`): below it,
/// the threads past the rows are fewer than the sizing's own dispatch costs. `RERAIL_DEVICE_SIZED_MIN` sets it
/// (0: every stage's, to test them), `RERAIL_DEVICE_SIZED=off` turns the sizing off.
fn sized_min() -> usize {
    static MIN: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *MIN.get_or_init(|| match std::env::var("RERAIL_DEVICE_SIZED") {
        Ok(v) if v == "off" => usize::MAX,
        _ => std::env::var("RERAIL_DEVICE_SIZED_MIN").ok().and_then(|v| v.parse().ok()).unwrap_or(4096),
    })
}

/// Bytes the device reads (a regex's automaton for `TEXTFN_REGEX`).
pub(in crate::code_rule_dsl) struct DevBytes {
    held: Held,
}

impl DevBytes {
    pub(in crate::code_rule_dsl) fn upload(bytes: &[u8]) -> Result<DevBytes, Error> {
        Ok(DevBytes { held: Held::of(bytes.iter().copied()).map_err(|_| Error::BackendCapacity)? })
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

    /// `bytes` put into the arena by the host, between batches (no command buffer writing it is pending): their
    /// value, or none when it has no room.
    pub(in crate::code_rule_dsl) fn put(&self, bytes: &[u8]) -> Option<i64> {
        let off = self.wanted();
        if off + bytes.len() > self.cap || bytes.len() > 0xff_ffff {
            return None;
        }
        // SAFETY: the header's first word and `bytes.len()` bytes within `cap` past the header; no command buffer is
        // using the arena (the caller's between batches).
        unsafe {
            let base = self.buffer.buffer.contents().cast::<u8>();
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), base.add(16 + off), bytes.len());
            *base.cast::<u32>() = (off + bytes.len()) as u32;
        }
        Some(ARENA_TEXT | ((off as i64) << 24) | bytes.len() as i64)
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
    /// Stages whose grids were sized on the device (`Scan::size`).
    pub(in crate::code_rule_dsl) sized: u32,
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
    /// Text buffers its passes read, kept until it is done (a chain's intermediate texts are dropped by their
    /// maker once the next pass is encoded: their buffers must not be reused while the GPU reads them).
    read: Vec<std::rc::Rc<DevRel>>,
    /// What its passes write that their maker gave up before it was done (a stratum refused after some of its rules
    /// were encoded): kept until it is, so no buffer it writes goes back to the pool while the GPU may write it.
    held: Vec<Box<dyn std::any::Any>>,
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
        Ok(Batch { open: None, pass: None, committed: Vec::new(), done: Done::default(), dummy: Held::new(64)?, keep: Vec::new(), read: Vec::new(), held: Vec::new(), arena: None, arena_cap: cap })
    }

    /// Keeps `what` (encoded work's buffers, given up by its maker) until the batch is done.
    pub(in crate::code_rule_dsl) fn hold(&mut self, what: Box<dyn std::any::Any>) {
        self.held.push(what);
    }

    /// A batch whose text arena is `arena`, kept across batches: a recursive component's rounds (decision 0307),
    /// whose texts one round makes the next reads.
    pub(in crate::code_rule_dsl) fn with_shared_arena(arena: std::rc::Rc<Arena>) -> Result<Batch, Error> {
        let mut b = Batch::with_arena(arena.cap)?;
        b.arena = Some(arena);
        Ok(b)
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

    /// Commits what is open without waiting: the work encoded so far runs before any command buffer committed after
    /// it on the one queue (a batch that reads what this one's work writes).
    pub(in crate::code_rule_dsl) fn commit_open(&mut self) -> Result<(), Error> {
        self.submit_open()
    }

    /// Commits what is open and waits for every command buffer.
    pub(in crate::code_rule_dsl) fn finish(&mut self) -> Result<Done, Error> {
        self.submit_open()?;
        for s in self.committed.drain(..) {
            self.done.gpu_seconds += s.wait();
        }
        self.keep.clear();
        self.read.clear();
        self.held.clear();
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
        let mut p = vec![n as u32, cols.len() as u32, mask, rel.at as u32, (rel.at >> 32) as u32];
        p.extend(cols.iter().map(|&c| c as u32));
        if n > 0 {
            pass.run(&hash, &[&rel.buffer.buffer, &words.buffer], &p, &[], groups(n));
        }
        let parts = encode_sort(pass, &words.buffer, Width::W32, bits, n)?;
        self.keep.push(words);
        Ok(DevIndex { parts, mask, rows: n })
    }

    /// The columns `cols` (its source's) of a view made from a chunk block on the device (decision 0289 §2), into
    /// `out`, a relation of `out.rows` rows whose columns are `cols` in order: what `Viewed::row_at` made on the host.
    /// `pairs`: an axis's pairs' offset in the block (`device_chunk::VIEW_AXIS`), else 0.
    pub(in crate::code_rule_dsl) fn expand_view(&mut self, block: &metal::Buffer, root: usize, view: u32, pairs: usize, cols: &[usize], out: &DevRel) -> Result<(), Error> {
        let expand = pipeline("chunk_views", "view_expand", chunk_views_source)?;
        let pass = self.pass(1)?;
        let at = |o: usize| u32::try_from(o).map_err(|_| Error::BackendCapacity);
        let rows_at = out.rows_at();
        let mut p = vec![at(root)?, view, out.rows as u32, cols.len() as u32, at(pairs)?, rows_at as u32, (rows_at >> 32) as u32];
        p.extend(cols.iter().map(|&c| c as u32));
        if out.rows > 0 {
            pass.run(&expand, &[block, &out.buffer.buffer], &p, &[], groups(out.rows));
        }
        Ok(())
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
                stage.r_at = rel.at;
                stage.r_dead = rel.dead;
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
        let arena: metal::Buffer = if matches!(stage.mode, STRCMP | TEXTOP) && (stage.agg[1] & 15 == OPND_ARENA || stage.agg[1] >> 4 == OPND_ARENA) {
            self.arena()?.buffer.buffer.to_owned()
        } else {
            self.dummy.buffer.to_owned()
        };
        let pass = self.pass(13)?;
        let lbuf: &BufferRef = &left.buffer.buffer;
        let count_bufs: [&BufferRef; 10] = [lbuf, &rbuf, &perm, &starts, &rkeys, &nruns, &totals.buffer.buffer, &counters.buffer.buffer, &counts, &arena];
        let scatter_bufs: [&BufferRef; 11] = [lbuf, &rbuf, &perm, &starts, &rkeys, &nruns, &totals.buffer.buffer, &counts, &out.buffer.buffer, &arena, &counters.buffer.buffer];
        if n > sized_min() {
            // The table's space is the host's guess: the grids are sized on the device, from the rows the stage
            // before made (decision 0113's GPU-sized work).
            let sizes = scan.size(pass, &totals.buffer.buffer, in_slot, left.cap)?;
            pass.run_bytes_indirect(&count, &count_bufs, stage.bytes(), &sizes.buffer, 4 * 32);
            scan.encode_sized(pass, &counts, &sizes)?;
            pass.run_bytes_indirect(&scatter, &scatter_bufs, stage.bytes(), &sizes.buffer, 4 * 32);
            self.done.sized += 1;
            self.keep.push(sizes);
            return Ok(());
        }
        pass.run_bytes(&count, &count_bufs, stage.bytes(), groups(n));
        scan.encode(pass, &counts)?;
        pass.run_bytes(&scatter, &scatter_bufs, stage.bytes(), groups(n));
        Ok(())
    }

    /// A witness stage (`WITNESS`, decision 0277) from `left` into `out`: per binding, the candidates of `right` in
    /// order until one that each of `rest`'s probes (semi- or anti-joins, at most `WITNESS_PROBES`) passes; then the
    /// bindings that have one, as a semi-join writes them. `stage`'s sizes, and the probes', are set here.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn witness(&mut self, mut stage: Stage, left: &Table, right: (&DevRel, Option<&DevIndex>), rest: &[(Stage, &DevRel, Option<&DevIndex>)],
                                            in_slot: usize, out_slot: usize, totals: &Words, out: &Table) -> Result<(), Error> {
        assert!(rest.len() <= WITNESS_PROBES, "a witness of more probes than it takes");
        let count = pipeline("join", "witness_count", || SOURCE.to_owned())?;
        let scatter = pipeline("join", "stage_scatter", || SOURCE.to_owned())?;
        stage.cap_in = left.cap as u32;
        stage.cap_out = out.cap as u32;
        stage.in_slot = in_slot as u32;
        stage.out_slot = out_slot as u32;
        stage.counter = NONE;
        stage.nslots = left.slots as u32;
        stage.nr = right.0.rows as u32;
        stage.r_at = right.0.at;
        stage.r_dead = right.0.dead;
        stage.agg[0] = rest.len() as u32;
        let dummy: metal::Buffer = self.dummy.buffer.to_owned();
        let parts = |rel: &DevRel, ix: Option<&DevIndex>| -> [metal::Buffer; 5] {
            match ix {
                Some(ix) => [rel.buffer.buffer.to_owned(), ix.parts.perm.buffer.to_owned(), ix.parts.starts.buffer.to_owned(), ix.parts.rkeys.buffer.to_owned(), ix.parts.nruns.buffer.to_owned()],
                None => [rel.buffer.buffer.to_owned(), dummy.clone(), dummy.clone(), dummy.clone(), dummy.clone()],
            }
        };
        if let Some(ix) = right.1 {
            stage.mask = ix.mask;
        }
        let a = parts(right.0, right.1);
        let mut probes = [Stage::default(); WITNESS_PROBES];
        let mut bufs: Vec<[metal::Buffer; 5]> = Vec::new();
        for (k, (b, rel, ix)) in rest.iter().enumerate() {
            let mut b = *b;
            b.nr = rel.rows as u32;
            b.r_at = rel.at;
            b.r_dead = rel.dead;
            b.cap_in = left.cap as u32;
            if let Some(ix) = ix {
                b.mask = ix.mask;
            }
            probes[k] = b;
            bufs.push(parts(rel, *ix));
        }
        while bufs.len() < WITNESS_PROBES {
            bufs.push([dummy.clone(), dummy.clone(), dummy.clone(), dummy.clone(), dummy.clone()]);
        }
        let held = Held::of(probes.iter().flat_map(|p| p.bytes().iter().copied()).collect::<Vec<u8>>().into_iter()).map_err(|_| Error::BackendCapacity)?;
        let n = left.cap + 1;
        let counts = scratch(FREE, 4 * n)?;
        let scan = Scan::new(n)?;
        let pass = self.pass(3)?;
        let lbuf: &BufferRef = &left.buffer.buffer;
        let mut count_bufs: Vec<&BufferRef> = vec![lbuf, &a[0], &a[1], &a[2], &a[3], &a[4], &totals.buffer.buffer, &counts, &held.buffer];
        for b in &bufs {
            count_bufs.extend(b.iter().map(|x| &**x));
        }
        pass.run_bytes(&count, &count_bufs, stage.bytes(), groups(n));
        scan.encode(pass, &counts)?;
        // The bindings with a witness out as they are: a semi-join's scatter (mode 1 writes the binding alone).
        let mut write = stage;
        write.mode = SEMI;
        write.nbinds = 0;
        let scatter_bufs: [&BufferRef; 11] = [lbuf, &a[0], &a[1], &a[2], &a[3], &a[4], &totals.buffer.buffer, &counts, &out.buffer.buffer, &dummy, &dummy];
        pass.run_bytes(&scatter, &scatter_bufs, write.bytes(), groups(n));
        self.keep.push(held);
        Ok(())
    }

    /// A mode 10 stage (`ROWTEXT`) from `left` into `out`: `row_text` (a SIMD group a row, over the names `names`,
    /// with automaton `auto`: a replacement's automata, or a test's table and its states, classes and start) gives
    /// each row its keep bit and value, then a scan and `row_scatter`.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn row_stage(&mut self, mut stage: Stage, left: &Table, names: &DevRel, auto: Option<(&DevBytes, [u32; 4])>, in_slot: usize, out_slot: usize,
                                              totals: &Words, counters: &Words, counter: Option<usize>, out: &Table) -> Result<(), Error> {
        let text = pipeline("join", "row_text", || SOURCE.to_owned())?;
        let scatter = pipeline("join", "row_scatter", || SOURCE.to_owned())?;
        stage.cap_in = left.cap as u32;
        stage.cap_out = out.cap as u32;
        stage.in_slot = in_slot as u32;
        stage.out_slot = out_slot as u32;
        stage.counter = counter.map_or(NONE, |c| c as u32);
        stage.nslots = left.slots as u32;
        let n = left.cap + 1;
        let ngroups = left.cap.clamp(1, 1024);
        // [scratch states, states, classes, start]: a replacement's scratch holds its larger automaton's states.
        let [kept, states, classes, start] = auto.map_or([1, 0, 0, 0], |a| a.1);
        let work = Held::new(ngroups * 64 * kept.max(1) as usize)?;
        let p = Held::of([ngroups as u32, kept, states, classes, start, 0, 0, 0].into_iter()).map_err(|_| Error::BackendCapacity)?;
        let counts = scratch(FREE, 4 * n)?;
        let values = Held::new(8 * n)?;
        let scan = Scan::new(n)?;
        let arena = self.arena()?.buffer.buffer.to_owned();
        let g: metal::Buffer = auto.map_or_else(|| self.dummy.buffer.to_owned(), |a| a.0.held.buffer.to_owned());
        let pass = self.pass(4)?;
        let lbuf: &BufferRef = &left.buffer.buffer;
        pass.run_bytes(&text, &[lbuf, &names.buffer.buffer, &arena, &totals.buffer.buffer, &counters.buffer.buffer, &counts, &values.buffer, &g, &work.buffer, &p.buffer],
                       stage.bytes(), (ngroups * 32).div_ceil(256));
        scan.encode(pass, &counts)?;
        pass.run_bytes(&scatter, &[lbuf, &values.buffer, &totals.buffer.buffer, &counts, &out.buffer.buffer], stage.bytes(), groups(n));
        self.keep.extend([work, p, values]);
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
        self.copy(src, n, None, cols, dst, dst_off)
    }

    /// A head's rows written in place (decision 0325 step 4): `src`'s first rows (as many as slot `count` of `totals`
    /// holds, at most `cap`; `cap` of them with no slot) into rows `0..` of `dst`, a relation in its block whose
    /// chunks hold `cap` rows. Each column of the head is a column of `src` or a constant (`cols`). The batch keeps
    /// `dst`'s block until its work completes.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn place_rows(&mut self, src: Rows<'_>, cap: usize, count: Option<usize>, totals: &Words, cols: &[Col], dst: DevRel) -> Result<(), Error> {
        self.copy(src, cap, count.map(|c| (c, totals)), cols, Rows::Rel(&dst), 0)?;
        self.keep.push(dst.buffer);
        Ok(())
    }

    /// `n` rows of `src` (as many as slot `count.0` of `count.1` holds, at most `n`), each column of `dst` a column of
    /// `src` or a constant (`cols`), into `dst`'s rows `dst_off..`: `rows_copy`, either side a table or a relation in
    /// its block.
    #[allow(clippy::too_many_arguments)]
    fn copy(&mut self, src: Rows<'_>, n: usize, count: Option<(usize, &Words)>, cols: &[Col], dst: Rows<'_>, dst_off: usize) -> Result<(), Error> {
        if n == 0 {
            return Ok(());
        }
        assert!(cols.len() <= 16);
        let copy = pipeline("join", "rows_copy", || SOURCE.to_owned())?;
        let (s, src_rows, src_at, _) = src.of();
        let (d, dst_rows, _, dst_rows_at) = dst.of();
        assert!(n <= src_rows.max(1), "a copy of {n} rows of a source of {src_rows}");
        assert!(dst_off + n <= dst_rows, "a copy of {n} rows at {dst_off} into a destination of {dst_rows}");
        let mut p = CopyParams {
            konst: [0; 16], src: [NONE; 16], n: n as u32, ncols: cols.len() as u32, src_stride: src_rows as u32, dst_stride: dst_rows as u32,
            dst_off: dst_off as u32, count_slot: count.map_or(NONE, |c| c.0 as u32), pad: [0; 2], src_at, dst_rows_at,
        };
        for (k, c) in cols.iter().enumerate() {
            match *c {
                Col::From(c) => p.src[k] = c as u32,
                Col::Const(v) => p.konst[k] = v,
            }
        }
        // SAFETY: `CopyParams` is `repr(C)` integers with no padding (its size asserted above).
        let bytes = unsafe { std::slice::from_raw_parts((&p as *const CopyParams).cast::<u8>(), std::mem::size_of::<CopyParams>()) };
        let (s, d) = (s.to_owned(), d.to_owned());
        let totals = count.map_or_else(|| self.dummy.buffer.to_owned(), |c| c.1.buffer.buffer.to_owned());
        let pass = self.pass(1)?;
        pass.run_bytes(&copy, &[&s, &d, &totals], bytes, groups(n));
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

    /// `replace_all` of a regex (its automata `table`, `text::table::Replacer`) by `with`, once per term of `text`: a
    /// text buffer of the same terms, their replaced texts (`text_replace`, two passes and a scan between).
    pub(in crate::code_rule_dsl) fn replace(&mut self, text: &DevText, table: &DevBytes, states: u32, with: &[u8]) -> Result<DevText, Error> {
        let n = text.terms;
        // A match of at least a byte becomes `with`: no text grows past its length times `with`'s.
        let room = text.chars.saturating_mul(with.len().max(1));
        let out = std::rc::Rc::new(DevRel::raw(Held::new((8 * (1 + n) + 4 * (n + 1) + room + 1).div_ceil(8) * 8)?, (8 * (1 + n) + 4 * (n + 1) + room + 1).div_ceil(8)));
        let kernel = pipeline("join", "text_replace", || SOURCE.to_owned())?;
        let groups = n.clamp(1, 1024);
        let scratch = Held::new(groups * 64 * states as usize)?;
        let lens = Held::new(4 * (n + 1))?;
        let z = Held::of(with.iter().copied().chain([0u8]).collect::<Vec<u8>>().into_iter()).map_err(|_| Error::BackendCapacity)?;
        let scan = Scan::new(n + 1)?;
        for write in [0u32, 1] {
            let p: [u32; 8] = [n as u32, groups as u32, with.len() as u32, write, states, 0, 0, 0];
            // SAFETY: plain integers.
            let bytes = unsafe { std::slice::from_raw_parts(p.as_ptr().cast::<u8>(), 32) };
            let pass = self.pass(8)?;
            pass.run_bytes(&kernel, &[&text.packed.buffer.buffer, &table.held.buffer, &z.buffer, &lens.buffer, &out.buffer.buffer, &scratch.buffer], bytes, (groups * 32).div_ceil(256));
            if write == 0 {
                scan.encode(pass, &lens.buffer)?;
            }
        }
        self.keep.extend([scratch, lens, z]);
        self.read.push(text.packed.clone());
        Ok(DevText { packed: out, terms: n, span: text.span, longest: room, sep: true, chars: room })
    }

    /// Text function `f` (`TEXTFN_*`) of constant numbers `n0`, `n1`, once per term of `text` (`text_map`): a text
    /// buffer of the same terms, their texts made. `flag` is set when `lower` met a term past ASCII.
    pub(in crate::code_rule_dsl) fn text_map(&mut self, text: &DevText, f: u32, n0: i64, n1: i64, flag: &Words) -> Result<DevText, Error> {
        let n = text.terms;
        // No function here makes a text longer.
        let room = text.chars;
        let out = std::rc::Rc::new(DevRel::raw(Held::new((8 * (1 + n) + 4 * (n + 1) + room + 1).div_ceil(8) * 8)?, (8 * (1 + n) + 4 * (n + 1) + room + 1).div_ceil(8)));
        let kernel = pipeline("join", "text_map", || SOURCE.to_owned())?;
        let groups = n.clamp(1, 1024);
        let lens = Held::new(4 * (n + 1))?;
        let scan = Scan::new(n + 1)?;
        for write in [0u32, 1] {
            let mut p = [0u8; 32];
            for (k, w) in [n as u32, groups as u32, f, write].iter().enumerate() {
                p[4 * k..4 * k + 4].copy_from_slice(&w.to_le_bytes());
            }
            p[16..24].copy_from_slice(&n0.to_le_bytes());
            p[24..32].copy_from_slice(&n1.to_le_bytes());
            let pass = self.pass(8)?;
            pass.run_bytes(&kernel, &[&text.packed.buffer.buffer, &lens.buffer, &out.buffer.buffer, &flag.buffer.buffer], &p, (groups * 32).div_ceil(256));
            if write == 0 {
                scan.encode(pass, &lens.buffer)?;
            }
        }
        self.keep.push(lens);
        self.read.push(text.packed.clone());
        Ok(DevText { packed: out, terms: n, span: text.span, longest: text.longest, sep: true, chars: room })
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
    /// Work still in flight holds its buffers: wait for it before they return to the pool. Work encoded and never
    /// committed is dropped uncommitted (nothing reads it, and nothing ran it): committing it here ran the launch
    /// guard, whose thread-local a batch dropped in a thread's teardown (a wave left open) can no longer read.
    fn drop(&mut self) {
        drop(self.pass.take());
        drop(self.open.take());
        for s in self.committed.drain(..) {
            s.wait();
        }
    }
}

#[cfg(test)]
pub mod tests {
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

    /// Decision 0325 step 3: relations read where they lie in their blocks, with dead rows marked by a delta's dead
    /// bits (`Rows::in_block_dead`), as a maintained relation's kills of the rows it shares. A scan, a keyed join
    /// (through the device's own index of the relation) and an anti-join skip the dead rows: the nested loops'
    /// rows over the live ones.
    #[test]
    fn stages_skip_the_dead_rows_of_relations_in_their_blocks() {
        use crate::code_rule_dsl::facts::rows::Rows;
        let Ok(device) = rerail_accel::gpu::join::device() else { return };
        let (na, nb, nc) = (2000, 3000, 500);
        let (a0, a1) = (rows(na, 21, 3000), rows(na, 22, 200));
        let (b0, b1) = (rows(nb, 23, 200), rows(nb, 24, 3000));
        let c0 = rows(nc, 25, 3000);
        // Dead: every third row of `a`, every fifth of `b`, every other of `c`.
        let dead = |n: usize, every: usize| {
            let mut w = vec![0u64; n.div_ceil(64)];
            (0..n).step_by(every).for_each(|i| w[i >> 6] |= 1 << (i & 63));
            w
        };
        let is_dead = |w: &[u64], i: usize| w[i >> 6] >> (i & 63) & 1 == 1;
        let (da, db, dc) = (dead(na, 3), dead(nb, 5), dead(nc, 2));
        let placed = |cols: &[&Vec<i64>], d: &[u64]| {
            let (n, k) = (cols[0].len(), cols.len());
            let mut r = Rows::new(k);
            r.extend_from_slice(&(0..n).flat_map(|i| cols.iter().map(move |c| c[i])).collect::<Vec<_>>());
            let b = r.in_block_dead(d).expect("the dead rows' delta");
            assert!(b.dead > 0);
            let held = rerail_accel::gpu::join::Held::kept(b.facts.block.gpu_buffer(device), Box::new(b.facts.clone()));
            (DevRel::in_block(held, b.at, b.dead, b.rows as usize, k), r)
        };
        let ((ra, _ka), (rb, _kb), (rc, _kc)) = (placed(&[&a0, &a1], &da), placed(&[&b0, &b1], &db), placed(&[&c0], &dc));
        let mut batch = Batch::new().unwrap();
        let ib = batch.index(&rb, &[0]).unwrap();
        let ic = batch.index(&rc, &[0]).unwrap();
        let slots = 3;
        let totals = Words::zeroed(4).unwrap();
        totals.set(0, 1);
        let counters = Words::zeroed(4).unwrap();
        let t0 = Table::one(slots, &[]).unwrap();
        let t1 = Table::new(slots, na).unwrap();
        let mut scan = Stage { mode: JOIN, nbinds: 2, ..Stage::default() };
        scan.bind_slot[..2].copy_from_slice(&[0, 1]);
        scan.bind_kind[..2].copy_from_slice(&[BIND_RIGHT, BIND_RIGHT]);
        scan.bind_idx[..2].copy_from_slice(&[0, 1]);
        batch.stage(scan, &t0, Some((&ra, None)), 0, 1, &totals, &counters, None, &t1).unwrap();
        batch.finish().unwrap();
        let live_a: Vec<usize> = (0..na).filter(|&i| !is_dead(&da, i)).collect();
        assert_eq!(totals.get(1) as usize, live_a.len(), "the scan's rows are the live ones");
        let mut want2: Vec<[i64; 3]> = Vec::new();
        for &i in &live_a {
            for j in (0..nb).filter(|&j| !is_dead(&db, j)) {
                if b0[j] == a1[i] {
                    want2.push([a0[i], a1[i], b1[j]]);
                }
            }
        }
        let t2 = Table::new(slots, want2.len().max(1)).unwrap();
        let mut join = Stage { mode: JOIN, nkeys: 1, nbinds: 1, ..Stage::default() };
        join.key_kind[0] = SLOT;
        join.key_slot[0] = 1;
        join.key_rcol[0] = 0;
        join.bind_slot[0] = 2;
        join.bind_kind[0] = BIND_RIGHT;
        join.bind_idx[0] = 1;
        batch.stage(join, &t1, Some((&rb, Some(&ib))), 1, 2, &totals, &counters, None, &t2).unwrap();
        let t3 = Table::new(slots, want2.len().max(1)).unwrap();
        let mut anti = Stage { mode: ANTI, nkeys: 1, ..Stage::default() };
        anti.key_kind[0] = SLOT;
        anti.key_slot[0] = 2;
        batch.stage(anti, &t2, Some((&rc, Some(&ic))), 2, 3, &totals, &counters, None, &t3).unwrap();
        batch.finish().unwrap();
        let got = |t: &Table, n: usize| (0..n).map(|r| [t.column(0, n)[r], t.column(1, n)[r], t.column(2, n)[r]]).collect::<Vec<_>>();
        assert_eq!(got(&t2, totals.get(2) as usize), want2, "the join's rows: live rows of both");
        let live_c: std::collections::HashSet<i64> = (0..nc).filter(|&i| !is_dead(&dc, i)).map(|i| c0[i]).collect();
        let want3: Vec<[i64; 3]> = want2.iter().copied().filter(|r| !live_c.contains(&r[2])).collect();
        assert_eq!(got(&t3, totals.get(3) as usize), want3, "a dead row of `c` does not hold a binding back");
        let all_c: std::collections::HashSet<i64> = c0.iter().copied().collect();
        assert!(want2.iter().filter(|r| !all_c.contains(&r[2])).count() < want3.len(), "the test reaches a dead row of `c`");
        assert!(want2.len() > 500, "{}", want2.len());
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

/// A lattice relation's key table on the device (`Batch::lattice_fold`): each held key's row, kept from round to
/// round, grown (every held row placed again) when the rows it may hold pass half its places.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct LatKeys {
    table: Option<Held>,
    places: usize,
}

/// A lattice the device folds by (`Batch::lattice_fold`): 0 min, 1 max, 2 flat, 3 shortlex (its values arena texts,
/// decision 0307); its top; `widen`'s count.
#[derive(Clone, Copy, Debug)]
pub(in crate::code_rule_dsl) struct LatCode {
    pub(in crate::code_rule_dsl) code: u32,
    pub(in crate::code_rule_dsl) top: i64,
    pub(in crate::code_rule_dsl) after: Option<u32>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LatParams { top: i64, n: u32, had: u32, nkeys: u32, code: u32, after: u32, pmask: u32, hmask: u32, level: u32, r_at: u64, r_rows_at: u64 }

const _: () = assert!(std::mem::size_of::<LatParams>() == 56);

impl Batch {
    /// A lattice relation's round (decision 0124): the `n` rows of `u` (its key's `nkeys` columns, then the value)
    /// folded by key into `total` (the relation's `nkeys + 2` columns, its last a key's changes; `had` rows): a held
    /// key's row updated in place, a new key's row into `new_keys` (as `total`'s columns), each changed key's row (key
    /// and value) into `delta`. `counters`: `[changed, new]`. `level`: where the relation keeps levels (decision
    /// 0311), the round, written to a changed or new key's row in a column after its changes (`total` and `new_keys`
    /// then have one more).
    #[allow(clippy::too_many_arguments)]
    /// Code 3 compares its values' texts in `arena`, which holds every value the fold reads.
    pub(in crate::code_rule_dsl) fn lattice_fold(&mut self, u: &Table, n: usize, total: Option<&DevRel>, nkeys: usize, lat: LatCode, keys: &mut LatKeys,
                                                 counters: &Words, new_keys: &Table, delta: &Table, arena: Option<&Arena>, level: Option<u32>) -> Result<(), Error> {
        assert!(lat.code != 3 || arena.is_some(), "a shortlex fold reads its values' texts");
        assert!(nkeys + 1 <= MAX_KEYS && u.cap == n && new_keys.cap == n && delta.cap == n);
        let had = total.map_or(0, |t| t.rows);
        let pplaces = (2 * n).next_power_of_two().max(16);
        let want = (2 * (had + n)).next_power_of_two().max(16);
        let mut p = LatParams { top: lat.top, n: n as u32, had: had as u32, nkeys: nkeys as u32, code: lat.code, after: lat.after.unwrap_or(NONE), pmask: (pplaces - 1) as u32, hmask: 0, level: level.unwrap_or(0),
                                 r_at: total.map_or(0, |t| t.at), r_rows_at: total.map_or(0, |t| t.rows_at()) };
        let r: metal::Buffer = total.map_or_else(|| self.dummy.buffer.to_owned(), |t| t.buffer.buffer.to_owned());
        if keys.places < want {
            // Grown: every held row placed again.
            let table = Held::new(4 * want)?;
            p.hmask = (want - 1) as u32;
            let bytes = lat_bytes(&p);
            let hclear = pipeline("join", "lat_hclear", || SOURCE.to_owned())?;
            let rehash = pipeline("join", "lat_rehash", || SOURCE.to_owned())?;
            let pass = self.pass(2)?;
            pass.run_bytes(&hclear, &[&table.buffer], bytes, groups(want));
            if had > 0 {
                pass.run_bytes(&rehash, &[&r, &table.buffer], bytes, groups(had));
            }
            if let Some(old) = keys.table.replace(table) {
                self.keep.push(old);
            }
            keys.places = want;
        }
        p.hmask = (keys.places - 1) as u32;
        let bytes = lat_bytes(&p);
        let clear = pipeline("join", "lat_clear", || SOURCE.to_owned())?;
        let claim = pipeline("join", "lat_claim", || SOURCE.to_owned())?;
        let best = pipeline("join", "lat_best", || SOURCE.to_owned())?;
        let merge = pipeline("join", "lat_merge", || SOURCE.to_owned())?;
        let places = Held::new(4 * pplaces)?;
        let chosen = Held::new(4 * pplaces)?;
        let flags = Held::new(4 * pplaces)?;
        let h: metal::Buffer = keys.table.as_ref().expect("a key table").buffer.to_owned();
        let a: metal::Buffer = arena.map_or_else(|| self.dummy.buffer.to_owned(), |a| a.buffer.buffer.to_owned());
        let pass = self.pass(4)?;
        let ub: &BufferRef = &u.buffer.buffer;
        pass.run_bytes(&clear, &[&places.buffer, &chosen.buffer, &flags.buffer], bytes, groups(pplaces));
        pass.run_bytes(&claim, &[ub, &places.buffer], bytes, groups(n));
        pass.run_bytes(&best, &[ub, &places.buffer, &chosen.buffer, &flags.buffer, &a], bytes, groups(n));
        pass.run_bytes(&merge, &[ub, &places.buffer, &chosen.buffer, &flags.buffer, &r, &h, &new_keys.buffer.buffer, &delta.buffer.buffer, &counters.buffer.buffer, &a], bytes, groups(pplaces));
        self.keep.extend([places, chosen, flags]);
        Ok(())
    }
}

fn lat_bytes(p: &LatParams) -> &[u8] {
    // SAFETY: `LatParams` is `repr(C)`: an i64, eight u32s, two u64s, no padding (its size asserted).
    unsafe { std::slice::from_raw_parts((p as *const LatParams).cast::<u8>(), std::mem::size_of::<LatParams>()) }
}
