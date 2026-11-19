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
use rerail_accel::gpu::join::{FREE, Held, Pass, Scan, Submitted, Width, encode_sort, groups, pipeline, scratch};


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
    /// The probed relation's index read by its keys (`rel_group`, decision 0325 step 5): its number in the
    /// relation's directory.
    pub(in crate::code_rule_dsl) r_index: u32,
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
    /// A probed view read in place (decision 0353 step 4d): its batch's directory's GPU address
    /// (`view_reads::ViewDir`), and the view's number (1 + `device_chunk::VIEW_*`; 0: a relation).
    pub(in crate::code_rule_dsl) v_dir: u64,
    pub(in crate::code_rule_dsl) r_view: u32,
    /// 1 when the probed view is `child` probed by its parent, off the tape (`DevIndex::by_parent`).
    pub(in crate::code_rule_dsl) v_parent: u32,
}

const _: () = assert!(std::mem::size_of::<Stage>() == 2320 && std::mem::size_of::<Stage>() % 16 == 0);

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
// rows' descriptor at `rows_at`), through the writer written once: a row past its chunks is counted, not written
// (decision 0427).
static inline void view_put(device uchar* out, ulong rows_at, uint r, uint c, long v) {
    clif_rows_put((device const uchar*)out, rows_at, r, c, ulong(v));
}

// p: the root's offset, the view (`device_chunk::VIEW_*`), the rows, the columns, 0 (an axis's pairs' place until
// decision 0353 step 5),
// the made relation's rows' descriptor's offset (low, high), then each column's index.
kernel void view_expand(device const char* B [[buffer(0)]], device uchar* out [[buffer(1)]], constant uint* p [[buffer(2)]],
                        uint r [[thread_position_in_grid]]) {
    uint n = p[2];
    if (r >= n) return;
    ulong rows_at = ulong(p[5]) | (ulong(p[6]) << 32);
    device const ChunkRoot* root = (device const ChunkRoot*)(B + p[0]);
    device const ChunkDoc* docs = items<ChunkDoc>(root->docs);
    uint view = p[1];
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

// Each node row's rows from its start (the scanned counts), nearest first: the ids of `n` and `a`, written once into
// the axis's relation (decision 0353 step 5) through the writer written once, bounded by its chunks. p[4] is the
// rows the relation holds, the total (`start[n]`) read before: a total past it writes nothing. p[5], p[6]: the
// relation's rows' descriptor's offset in `out`'s block (low, high).
kernel void axis_write(device const char* B [[buffer(0)]], device const uint* start [[buffer(1)]], device uchar* out [[buffer(2)]],
                       constant uint* p [[buffer(3)]], uint v [[thread_position_in_grid]]) {
    if (v >= p[1] || start[p[1]] > p[4]) return;
    ulong rows_at = ulong(p[5]) | (ulong(p[6]) << 32);
    device const ChunkRoot* root = (device const ChunkRoot*)(B + p[0]);
    device const ChunkDoc* docs = items<ChunkDoc>(root->docs);
    device const ChunkDoc& d = docs[doc_of_node(docs, root->ndocs, v)];
    uint at = start[v];
    uint l = v - d.node_row;
    long n_id = global_of(d, l);
    uint up = parent_of(d, l);
    for (uint k = 0u; k < d.nodes && up != CH_NONE; k++) {
        if (axis_member(B, p[2], d, up)) {
            view_put(out, rows_at, at, 0u, n_id);
            view_put(out, rows_at, at, 1u, global_of(d, up));
            at++;
            if (p[3] == 0u) break;
        }
        up = parent_of(d, up);
    }
}
"#;

/// Every join kernel's source: `SOURCE_MAIN` with the relation's reads written once (`rerail_blocks::RELATION_READS`,
/// decision 0325), which a stage reads a relation in its block with.
/// And the views' reads (decision 0353 step 4d), which a stage reads a view in place with: the tape's, the
/// numbering's and the views' own, each document's view read where the batch's directory holds it (`device`).
static SOURCE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let reads = format!(
        "using namespace metal;\n#define CLIF_VIEW_SPACE device\n{}{}{}{}{VREAD}",
        rerail_blocks::RELATION_READS,
        // The reads that read a 16z tape too, through its batch's cache (decision 0353 step 4b).
        include_str!("metal_tape_reads_z.metal"),
        include_str!("../../facts/vtab/syntax/id_reads.metal"),
        include_str!("../../facts/vtab/syntax/view_reads.metal"),
    );
    SOURCE_MAIN.replacen("using namespace metal;\n", &reads, 1)
});

/// A view's row `r`'s value in column `col` (decision 0353 step 4d): the document whose rows hold it (the last whose
/// first row in the view is at or before it: an empty one starts where the next does), then the view's read written
/// once, over the document's entry, tape and view where they lie. `kind`: 1 + `device_chunk::VIEW_*`.
const VREAD: &str = r#"
__attribute__((noinline)) static long vread(uint kind, device const ViewDir* d, uint r, uint col) {
    device const ViewRow* rows = d->rows;
    uint k = kind - 1u;
    uint lo = 0u, hi = d->ndocs;
    while (hi - lo > 1u) {
        uint m = (lo + hi) / 2u;
        if (rows[m].rows[k] <= r) lo = m; else hi = m;
    }
    device const ViewRow& e = rows[lo];
    uint i = r - e.rows[k];
    device const uint* v = e.view;
    switch (k) {
        case 0u: return long(clif_view_node(d->chunk, e.entry, e.tape, v, e.lines, i, col));
        case 1u: return long(clif_view_child(d->chunk, e.entry, e.tape, v, e.lines, i, col));
        case 2u: return long(clif_view_field(d->chunk, e.entry, e.tape, v, e.lines, i, col));
        case 3u: return long(clif_view_core(d->chunk, e.entry, e.tape, v, e.lines, i, col));
        case 4u: return long(clif_view_line(d->chunk, e.entry, e.tape, v, e.lines, i, col));
        case 5u: return long(clif_view_loc_line(d->chunk, e.entry, e.tape, v, e.lines, i, col));
        case 6u: return long(clif_view_loc_end(d->chunk, e.entry, e.tape, v, e.lines, i, col));
        case 7u: return long(clif_view_loc_col(d->chunk, e.entry, e.tape, v, e.lines, i, col));
        default: return long(clif_view_loc_docof(d->chunk, e.entry, e.tape, v, e.lines, i, col));
    }
}
"#;

const SOURCE_MAIN: &str = r#"#include <metal_stdlib>
using namespace metal;

struct Stage {
    long key_const[8];
    long par_const[16];
    ulong r_at, r_dead;
    uint4 ops[64];
    uint mode, nslots, cap_in, cap_out, in_slot, out_slot, nr, nkeys, r_index, ntests, nops, nparams, nbinds, npassed, last, counter;
    uint key_kind[8]; uint key_slot[8]; uint key_rcol[8];
    uint test_a[8]; uint test_b[8];
    uint col_kind[16]; uint col_idx[16]; uint col_doc[16];
    uint par_kind[16]; uint par_slot[16]; uint par_doc[16];
    uint bind_slot[16]; uint bind_kind[16]; uint bind_idx[16];
    uint passed[16]; uint outputs[16];
    uint noutputs; uint ncols; uint agg[2];
    uint4 arith[8];
    device const ViewDir* v_dir; uint r_view; uint v_parent;
};

constant uint NONE = 0xffffffffu;

// A key's hash, one value at a time: `rerail_blocks::key_hash`, the read written once (`clif_rel_key_mix`, decision
// 0325 step 5), which reads nothing through its descriptor. Every hash of a key on the device is it: a relation's
// index, a probe of it, the heads' set (`dedup`), a lattice's key table.
constant ulong KEY_SEED = 0x243F6A8885A308D3ul;
static inline ulong key_mix(ulong h, long v) {
    return clif_rel_key_mix((device const uchar*)nullptr, 0ul, h, ulong(v));
}
static inline ulong key_hash(thread const long* kv, uint n) {
    ulong h = KEY_SEED;
    for (uint i = 0; i < n && i < 8; i++) h = key_mix(h, kv[i]);
    return h;
}

static inline long lane_of(long v, uint w) {
    return w == 16 ? long(short(v)) : (w == 32 ? long(int(v)) : v);
}

// A value of the probed relation: row `r`'s in column `col`, read in its block through the generated reads (`r_at`,
// decision 0325). Every relation the device reads lies in a block (step 5).
static inline long rget(constant Stage& s, device const long* R, uint r, uint col) {
    if (s.r_view != 0u) return vread(s.r_view, s.v_dir, r, col);
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

// Entry `q` of the probed relation's index `r_index` (its rows grouped by key, in its block): a row id.
static inline uint group_row(constant Stage& s, device const long* R, uint q) {
    return clif_rel_group_row((device const uchar*)R, s.r_at, s.r_index, q);
}

// The group of index `index` of the relation at `at` for the keys `kv`: [lo, hi) of its entries (every row whose key
// falls in the key's bucket; `matches` tests the key itself).
static inline void group_of(device const long* R, ulong at, uint index, thread const long* kv, uint n, thread uint& lo, thread uint& hi) {
    ulong g = clif_rel_group((device const uchar*)R, at, index, key_hash(kv, n));
    lo = uint(g >> 32);
    hi = lo + uint(g & 0xfffffffful);
}

// The `child` view probed by its parent (decision 0353 step 4c, `Stage::v_parent`): the parent's children read off
// the tape, its first child and each next sibling (`clif_view_child_first`, `clif_view_child_next`), with no index of
// the view. A walk of a binding's candidates (`how`): 0 the rows [lo, hi), 1 an index group's entries [lo, hi), 2 a
// parent's children from row lo, each step the next sibling's row (`NONE` ends it).
static inline device const ViewDoc* vdoc(device const ViewDir* d, uint m) {
    return (device const ViewDoc*)(d->chunk + d->rows[m].entry);
}

// A parent `key`'s children: its document by bisection of the directory on its key (the batch's documents are in
// key order), its numbered node (`clif_view_number`), then its first child. The root's children are also the nodes
// under unnumbered ones (a narrowed projection): its probe walks its document's rows, which `matches` tests.
static inline void by_parent(constant Stage& s, long key, thread uint& lo, thread uint& hi, thread uint& how) {
    device const ViewDir* d = s.v_dir;
    lo = 0u; hi = 0u; how = 0u;
    if (key < 0 || d->ndocs == 0u) return;
    uint k = uint(ulong(key) >> 32);
    uint a = 0u, b = d->ndocs;
    while (b - a > 1u) {
        uint m = (a + b) / 2u;
        if (vdoc(d, m)->key <= k) a = m; else b = m;
    }
    if (vdoc(d, a)->key != k) return;
    device const ViewRow& e = d->rows[a];
    uint l = clif_view_number(d->chunk, e.entry, e.tape, e.view, e.lines, uint(ulong(key) & 0xfffffffful));
    if (l == NONE) return;
    uint first = e.rows[1];
    if (l == 0u) { lo = first; hi = a + 1u < d->ndocs ? d->rows[a + 1u].rows[1] : s.nr; return; }
    uint c = clif_view_child_first(d->chunk, e.entry, e.tape, e.view, e.lines, l);
    if (c == NONE) return;
    lo = first + c; hi = NONE; how = 2u;
}

// The next sibling's row of `child` row `r`, or `NONE`: its document as `vread` finds it, then the read.
static inline uint child_next(constant Stage& s, uint r) {
    device const ViewDir* d = s.v_dir;
    device const ViewRow* rows = d->rows;
    uint lo = 0u, hi = d->ndocs;
    while (hi - lo > 1u) {
        uint m = (lo + hi) / 2u;
        if (rows[m].rows[1] <= r) lo = m; else hi = m;
    }
    device const ViewRow& e = rows[lo];
    uint n = clif_view_child_next(d->chunk, e.entry, e.tape, e.view, e.lines, r - e.rows[1]);
    return n == NONE ? NONE : e.rows[1] + n;
}

// A walk's candidate `q`: its row, and the next candidate.
static inline uint cand_row(constant Stage& s, device const long* R, uint how, uint q) {
    return how == 1u ? group_row(s, R, q) : q;
}
static inline uint cand_next(constant Stage& s, uint how, uint q) {
    return how == 2u ? child_next(s, q) : q + 1u;
}

// The candidates of keys `kv` in relation R (`s`'s): its index group, a parent's children, or every row.
static inline void probe(constant Stage& s, device const long* R, thread const long* kv, thread uint& lo, thread uint& hi, thread uint& how) {
    if (s.nkeys == 0) { lo = 0; hi = s.nr; how = 0u; return; }
    if (s.v_parent != 0u) { by_parent(s, kv[0], lo, hi, how); return; }
    how = 1u;
    group_of(R, s.r_at, s.r_index, kv, s.nkeys, lo, hi);
}

// A binding's candidates (`probe`), and its key values.
static inline void candidates(constant Stage& s, device const long* L, uint t, device const long* R, thread long* kv, thread uint& lo, thread uint& hi, thread uint& how) {
    how = 0u;
    if (s.mode == 3) { lo = 0; hi = 1; return; }
    for (uint i = 0; i < s.nkeys && i < 8; i++) kv[i] = s.key_kind[i] == 1 ? L[ulong(s.key_slot[i]) * s.cap_in + t] : s.key_const[i];
    probe(s, R, kv, lo, hi, how);
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
static inline uint aggregate(constant Stage& s, device const long* L, uint t, device const long* R,
                             uint lo, uint hi, uint how, thread const long* kv, thread long& v) {
    uint kind = s.agg[0] & 0xff;
    long cnt = 0, sum = 0, mn = 0, mx = 0;
    bool any = false, over = false;
    for (uint q = lo; q < hi; q = cand_next(s, how, q)) {
        uint r = cand_row(s, R, how, q);
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

kernel void stage_count(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]],
                        device const uint* totals [[buffer(2)]], device atomic_uint* counters [[buffer(3)]], device uint* counts [[buffer(4)]],
                        device atomic_uint* A [[buffer(5)]], constant Stage& s [[buffer(6)]], uint t [[thread_position_in_grid]]) {
    if (t > s.cap_in) return;
    uint n = min(totals[s.in_slot], s.cap_in);
    if (t >= n) { counts[t] = 0; return; }
    long kv[8];
    uint lo, hi, how;
    if (s.mode == 5) { counts[t] = uint(text_pass(s, L, t, R)); return; }
    if (s.mode == 6) { counts[t] = 1; return; }
    if (s.mode == 7) { counts[t] = uint(textop_pass(s, L, t, R, A)); return; }
    candidates(s, L, t, R, kv, lo, hi, how);
    if (s.mode == 4) {
        long v = 0;
        uint st = aggregate(s, L, t, R, lo, hi, how, kv, v);
        if (st == 2 && s.counter != NONE) atomic_store_explicit(&counters[s.counter], 1u, memory_order_relaxed);
        counts[t] = uint(st == 0);
        return;
    }
    bool exists = s.mode == 1 || s.mode == 2;
    uint c = 0, p0 = 0;
    uint q_counts[16];
    for (uint m = 0; m < 16; m++) q_counts[m] = 0;
    bool found = false;
    for (uint q = lo; q < hi; q = cand_next(s, how, q)) {
        uint r = cand_row(s, R, how, q);
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

kernel void stage_scatter(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]],
                          device uint* totals [[buffer(2)]], device const uint* offsets [[buffer(3)]], device long* O [[buffer(4)]],
                          device atomic_uint* A [[buffer(5)]], device atomic_uint* counters [[buffer(6)]], constant Stage& s [[buffer(7)]],
                          uint t [[thread_position_in_grid]]) {
    uint n = min(totals[s.in_slot], s.cap_in);
    // The offsets past the rows' are not scanned when the grids are sized on the device (`Scan::size`).
    if (t == 0) totals[s.out_slot] = offsets[n];
    if (t >= n) return;
    uint at = offsets[t];
    if (at >= s.cap_out) return;
    long kv[8];
    uint lo, hi, how;
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
        candidates(s, L, t, R, kv, lo, hi, how);
        long v = 0;
        aggregate(s, L, t, R, lo, hi, how, kv, v);
        write_row(s, L, t, R, 0, O, at);
        if ((s.agg[0] >> 8) & 1) O[ulong(s.bind_slot[0]) * s.cap_out + at] = v;
        return;
    }
    if (s.mode == 1 || s.mode == 2) {
        // The count pass decided: a row out is the binding as it is.
        if (offsets[t + 1] > at) write_row(s, L, t, R, 0, O, at);
        return;
    }
    candidates(s, L, t, R, kv, lo, hi, how);
    for (uint q = lo; q < hi && at < s.cap_out; q = cand_next(s, how, q)) {
        uint r = cand_row(s, R, how, q);
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
                                 device const long* R) {
    long kv[8];
    for (uint i = 0; i < b.nkeys && i < 8; i++) {
        uint k = b.key_kind[i];
        kv[i] = k == 1 ? L[ulong(b.key_slot[i]) * cap_in + t] : (k == 2 ? rget(a, RA, ra, b.key_slot[i]) : b.key_const[i]);
    }
    uint lo, hi, how;
    probe(b, R, kv, lo, hi, how);
    bool found = false;
    for (uint q = lo; q < hi; q = cand_next(b, how, q)) {
        uint r = cand_row(b, R, how, q);
        if (matches(b, R, r, kv)) { found = true; break; }
    }
    return found != (b.mode == 2);
}

// A witness's count (mode 11): 1 for a binding with a candidate of the stage's relation that every probe passes,
// found in the candidates' order, the loop ending at the first; else 0. `stage_scatter` (as a semi-join) writes them.
kernel void witness_count(device const long* L [[buffer(0)]], device const long* R [[buffer(1)]],
                          device const uint* totals [[buffer(2)]], device uint* counts [[buffer(3)]], constant Stage* bs [[buffer(4)]],
                          device const long* R0 [[buffer(5)]], device const long* R1 [[buffer(6)]], device const long* R2 [[buffer(7)]],
                          constant Stage& s [[buffer(8)]], uint t [[thread_position_in_grid]]) {
    if (t > s.cap_in) return;
    uint n = min(totals[s.in_slot], s.cap_in);
    if (t >= n) { counts[t] = 0; return; }
    long kv[8];
    uint lo, hi, how;
    candidates(s, L, t, R, kv, lo, hi, how);
    uint nb = min(s.agg[0], 3u);
    uint found = 0;
    for (uint q = lo; q < hi; q = cand_next(s, how, q)) {
        uint r = cand_row(s, R, how, q);
        if (!matches(s, R, r, kv)) continue;
        bool all = true;
        if (nb > 0) all = witness_probe(bs[0], L, t, s.cap_in, s, R, r, R0);
        if (all && nb > 1) all = witness_probe(bs[1], L, t, s.cap_in, s, R, r, R1);
        if (all && nb > 2) all = witness_probe(bs[2], L, t, s.cap_in, s, R, r, R2);
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
    device const ViewDir* v_dir; uint r_view; uint v_pad;
};

kernel void rows_copy(device const uchar* S [[buffer(0)]], device uchar* D [[buffer(1)]], device const uint* totals [[buffer(2)]],
                      constant Copy& c [[buffer(3)]], uint t [[thread_position_in_grid]]) {
    uint n = c.count_slot == NONE ? c.n : min(totals[c.count_slot], c.n);
    if (t >= n) return;
    for (uint k = 0; k < c.ncols && k < 16; k++) {
        uint from = c.src[k];
        long v = from == NONE ? c.konst[k]
               : c.r_view != 0u ? vread(c.r_view, c.v_dir, t, from)
               : c.src_at == 0ul ? ((device const long*)S)[ulong(from) * c.src_stride + t]
               : long(clif_rel_get(S, c.src_at, t, from));
        if (c.dst_rows_at == 0ul) ((device long*)D)[ulong(k) * c.dst_stride + c.dst_off + t] = v;
        else clif_rows_put((device const uchar*)D, c.dst_rows_at, c.dst_off + t, k, ulong(v));
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
    ulong h = KEY_SEED;
    for (uint c = 0; c < d.ncols && c < 16; c++) h = key_mix(h, T[ulong(d.cols[c]) * d.cap + x]);
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
// A relation's index built on the device (decision 0325 step 5): each row's bucket, its key's hash (`key_hash`, as
// the host's builder and every probe hash it) masked. p: the rows, the key's columns, the relation's descriptor's
// offset in its block (low, high), the mask (low, high), each key column.
// A view read in place (decision 0353 step 4d): its directory and number (`Stage::v_dir`, `r_view`; 0: a relation).
struct ViewArg { device const ViewDir* dir; ulong kind; };
kernel void ix_bucket(device const uchar* B [[buffer(0)]], device uint* out [[buffer(1)]], constant uint* p [[buffer(2)]],
                      constant ViewArg& va [[buffer(3)]], uint t [[thread_position_in_grid]]) {
    if (t >= p[0]) return;
    ulong at = ulong(p[2]) | (ulong(p[3]) << 32);
    ulong mask = ulong(p[4]) | (ulong(p[5]) << 32);
    ulong h = KEY_SEED;
    for (uint i = 0; i < p[1] && i < 8; i++) h = key_mix(h, va.kind != 0ul ? vread(uint(va.kind), va.dir, t, p[6 + i]) : long(clif_rel_get(B, at, t, p[6 + i])));
    out[t] = uint(h & mask);
}

// The index's rows and buckets written into its relation's block, from the rows sorted (stably) by bucket: entry `t`
// is the sort's row `t` (so a bucket's rows are in id order, as the host's builder places them), and run `t` (a
// bucket that has rows) sets its bucket's start and the empty buckets' before it; the last run the rest. p: the
// rows, the buckets, the buckets' offset in the block (low, high), the rows' offset (low, high).
kernel void ix_place(device uchar* B [[buffer(0)]], device const uint* perm [[buffer(1)]], device const uint* starts [[buffer(2)]],
                     device const uint* rkeys [[buffer(3)]], device const uint* nruns [[buffer(4)]], constant uint* p [[buffer(5)]],
                     uint t [[thread_position_in_grid]]) {
    uint n = p[0], cap = p[1];
    device uint* offs = (device uint*)(B + (ulong(p[2]) | (ulong(p[3]) << 32)));
    device uint* rows = (device uint*)(B + (ulong(p[4]) | (ulong(p[5]) << 32)));
    if (t < n) rows[t] = perm[t];
    uint runs = nruns[0];
    if (t >= runs) return;
    uint b = rkeys[t];
    uint from = t == 0 ? 0u : rkeys[t - 1] + 1u;
    for (uint x = from; x <= b; x++) offs[x] = starts[t];
    if (t == runs - 1) for (uint x = b + 1; x <= cap; x++) offs[x] = n;
}

// A lattice relation's rounds on the device (decision 0124): a round's rows `U` (a table of `k` slots: the key's
// `nkeys` columns, then the value) folded by key into the lattice's relation `R` itself (its `k` columns, `had` rows
// held: one live row a key), through its key map `H` (decision 0384 step 5, `setfix::keys`), which lies in a block at
// `h_at`: a slot a key, three words: the key's row (EMPTY: none), its changes, its round. `P` groups `U`'s rows by
// key (each place the least row of its key), `best` the key's chosen row (min and max select one of their values),
// `flags` a flat key's conflict. A held key's row is updated where it lies; a new key's row is written past `R`'s
// rows (at `had` on, room the host reserved), and its slot put in `H`. Each changed key's row (key and value) goes to
// `D`, the next round's delta: `counters[0]` of them, `counters[1]` new. `level`: the round, written to a changed or
// new key's slot where the relation keeps levels (decision 0311); 0: none.
struct LatP { long top; uint n; uint had; uint nkeys; uint code; uint after; uint pmask; uint hmask; uint level; uint omask; uint pad;
              ulong r_at, r_rows_at, h_at, h0_at; };

// The relation `R`'s row `row` in column `c`, and the place to write it: `R` is a relation in its block
// (decision 0325 step 5), its descriptor at `r_at`, its rows' at `r_rows_at`; written through the writer written
// once, bounded by its chunks (decision 0427).
static inline long lat_r(device const uchar* R, constant LatP& p, uint row, uint c) {
    return long(clif_rel_get(R, p.r_at, row, c));
}
static inline void lat_put(device uchar* R, constant LatP& p, uint row, uint c, long v) {
    clif_rows_put((device const uchar*)R, p.r_rows_at, row, c, ulong(v));
}

static inline uint lat_hash(thread const long* key, uint nk, uint mask) {
    return uint(key_hash(key, nk)) & mask;
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

// The key map's slot `i` emptied (a map new, or grown).
kernel void lat_hclear(device uchar* HB [[buffer(0)]], constant LatP& p [[buffer(1)]], uint i [[thread_position_in_grid]]) {
    if (i > p.hmask) return;
    device uint* H = (device uint*)(HB + p.h_at);
    H[3 * i] = 0xffffffffu;
    H[3 * i + 1] = 0;
    H[3 * i + 2] = 0;
}

// The old map's slot `s` (at `h0_at` in `OB`, `omask + 1` slots) into the new one (grown: every key placed again, its
// changes and round with it); its key read in `R`'s row.
kernel void lat_rehash(device const uchar* R [[buffer(0)]], device const uchar* OB [[buffer(1)]], device uchar* HB [[buffer(2)]],
                       constant LatP& p [[buffer(3)]], uint s [[thread_position_in_grid]]) {
    if (s > p.omask) return;
    device const uint* O = (device const uint*)(OB + p.h0_at);
    uint t = O[3 * s];
    if (t == 0xffffffffu) return;
    device atomic_uint* H = (device atomic_uint*)(HB + p.h_at);
    long key[8];
    for (uint c = 0; c < p.nkeys && c < 8; c++) key[c] = lat_r(R, p, t, c);
    uint at = lat_hash(key, p.nkeys, p.hmask);
    // Bounded: a place at a time, and a weak exchange's spurious failure tries the place again.
    for (uint k = 0; k < 2 * (p.hmask + 1); k++) {
        uint seen = 0xffffffffu;
        if (atomic_compare_exchange_weak_explicit(&H[3 * at], &seen, t, memory_order_relaxed, memory_order_relaxed)) {
            atomic_store_explicit(&H[3 * at + 1], O[3 * s + 1], memory_order_relaxed);
            atomic_store_explicit(&H[3 * at + 2], O[3 * s + 2], memory_order_relaxed);
            return;
        }
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
                      device const uint* flags [[buffer(3)]], device uchar* R [[buffer(4)]], device uchar* HB [[buffer(5)]],
                      device long* D [[buffer(6)]], device atomic_uint* counters [[buffer(7)]],
                      device atomic_uint* A [[buffer(8)]], constant LatP& p [[buffer(9)]], uint s [[thread_position_in_grid]]) {
    device atomic_uint* H = (device atomic_uint*)(HB + p.h_at);
    if (s > p.pmask) return;
    uint rep = P[s];
    if (rep == 0xffffffffu) return;
    uint r = (p.code == 2 || best[s] == 0xffffffffu) ? rep : best[s];
    long v = U[ulong(p.nkeys) * p.n + r];
    if (p.code == 2 && flags[s] != 0) v = p.top;
    long key[8];
    for (uint c = 0; c < p.nkeys && c < 8; c++) key[c] = U[ulong(c) * p.n + r];
    uint at = lat_hash(key, p.nkeys, p.hmask);
    uint held = 0xffffffffu;
    for (uint i = 0; i <= p.hmask; i++) {
        uint h = atomic_load_explicit(&H[3 * at], memory_order_relaxed);
        if (h == 0xffffffffu) break;
        // A row past `had` is a new key's this round (another key's: each key is one place of `P`).
        bool same = h < p.had;
        for (uint c = 0; same && c < p.nkeys && c < 8; c++) same = lat_r(R, p, h, c) == key[c];
        if (same) { held = h; break; }
        at = (at + 1) & p.hmask;
    }
    if (held != 0xffffffffu) {
        long o = lat_r(R, p, held, p.nkeys);
        long m = lat_lub(p, A, o, v);
        if (m == o) return;
        uint changes = atomic_load_explicit(&H[3 * at + 1], memory_order_relaxed);
        if (p.after != 0xffffffffu && changes >= p.after) {
            m = lat_widen(p, o, m);
            if (m == o) return;
        }
        lat_put(R, p, held, p.nkeys, m);
        atomic_store_explicit(&H[3 * at + 1], changes + 1, memory_order_relaxed);
        if (p.level != 0) atomic_store_explicit(&H[3 * at + 2], p.level, memory_order_relaxed);
        v = m;
    } else {
        uint a = atomic_fetch_add_explicit(&counters[1], 1u, memory_order_relaxed);
        uint id = p.had + a;
        for (uint c = 0; c < p.nkeys && c < 8; c++) lat_put(R, p, id, c, key[c]);
        lat_put(R, p, id, p.nkeys, v);
        uint place = lat_hash(key, p.nkeys, p.hmask);
        for (uint i = 0; i < 2 * (p.hmask + 1); i++) {
            uint seen = 0xffffffffu;
            if (atomic_compare_exchange_weak_explicit(&H[3 * place], &seen, id, memory_order_relaxed, memory_order_relaxed)) {
                atomic_store_explicit(&H[3 * place + 1], 1u, memory_order_relaxed);
                atomic_store_explicit(&H[3 * place + 2], p.level, memory_order_relaxed);
                break;
            }
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

/// A kind-set axis's batch solve on the device (decision 0289 §2, `facts::nearest::ChunkAxis`), from `chunk`: each
/// node's rows counted by a climb of its ancestors and the counts scanned into the starts (in the chunk, page-aligned,
/// bound as a buffer of their own with no copy); then, the total read, the rows written, each the ids of `n` and `a`,
/// into a relation of exactly that many rows (decision 0353 step 5). It is a derived fact like a rule's head: it lies
/// in a run's block, not leased from the device's budget, which a room sized before the count (containment's up to
/// several rows a node) filled.
pub fn axis_in_chunk(chunk: &crate::code_rule_dsl::facts::vtab::syntax::device_chunk::Chunk, bits: Option<&[u32]>, inside: bool)
                     -> Result<crate::code_rule_dsl::facts::nearest::InChunk, String> {
    use crate::code_rule_dsl::facts::rows::Part;
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
            // copy: not block data: the set's membership bits, made here for the kernel.
            unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), p, words.len()) };
            b.offset_of(p)
        }
        None => 0,
    };
    let start_bytes = (4 * (n + 1)).next_multiple_of(PAGE);
    let start_at = b.try_alloc(start_bytes, PAGE).ok_or_else(full)?;
    let device = rerail_accel::gpu::join::device()?;
    let whole = b.gpu_buffer(device);
    let starts = device.new_buffer_with_bytes_no_copy(b.ptr::<u8>(start_at) as *const std::ffi::c_void, start_bytes as u64, metal::MTLResourceOptions::StorageModeShared, None);
    let count = pipeline("chunk_views", "axis_count", chunk_views_source)?;
    let write = pipeline("chunk_views", "axis_write", chunk_views_source)?;
    let at = |o: usize| u32::try_from(o).map_err(|_| "the axis lies past 4 GB of its chunk".to_string());
    let mut p = [at(chunk.root)?, n as u32, at(bits_at)?, u32::from(inside), 0, 0, 0];
    let scan = Scan::owned(n + 1)?;
    let mut batch = Batch::new().map_err(|e| format!("{e:?}"))?;
    let pass = batch.pass(16).map_err(|e| format!("{e:?}"))?;
    pass.run(&count, &[&whole, &starts], &p, &[], groups(n + 1));
    scan.encode(pass, &starts)?;
    batch.finish().map_err(|e| format!("{e:?}"))?;
    // SAFETY: the scan completed: its last value is the total.
    let len = unsafe { *b.ptr::<u32>(start_at + 4 * n) } as usize;
    // Alone in its block, with room for an index by each column: strata probe an axis by `n` and by `a`.
    let rows = Part::with_index_room(crate::code_rule_dsl::facts::rows::Life::Run, 2, len, 2);
    if len > 0 {
        let needed = rows.rel().rows.needed.load(std::sync::atomic::Ordering::Relaxed);
        let rows_at = rows.rows_at() as u64;
        (p[4], p[5], p[6]) = (len as u32, rows_at as u32, (rows_at >> 32) as u32);
        let out = rows.facts().block.gpu_buffer(device);
        let mut batch = Batch::new().map_err(|e| format!("{e:?}"))?;
        let pass = batch.pass(1).map_err(|e| format!("{e:?}"))?;
        pass.run(&write, &[&whole, &starts, &out], &p, &[], groups(n));
        batch.finish().map_err(|e| format!("{e:?}"))?;
        // Bounded by the relation's chunks (decision 0427): what the writer refused fails the solve, which the host
        // then makes.
        let now = rows.rel().rows.needed.load(std::sync::atomic::Ordering::Relaxed);
        if now != needed {
            return Err(format!("the axis's writer refused {} rows past its relation's chunks (decision 0427)", now.wrapping_sub(needed)));
        }
    }
    // SAFETY: the write completed, every row below the total written.
    unsafe { rows.written(len as u32) };
    Ok(crate::code_rule_dsl::facts::nearest::InChunk { start: start_at, rows })
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
    /// The block its relation lies in (none for words): where its indexes go (decision 0325 step 5).
    home: Option<std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>>,
    /// A view read in place (decision 0353 step 4d): its rows are read through the views' reads, and its relation
    /// (of no columns) holds its indexes alone.
    pub(in crate::code_rule_dsl) view: Option<crate::code_rule_dsl::facts::vtab::ViewInPlace>,
    /// Its descriptor's arity and base rows when it was made (Q253): a descriptor found otherwise later was
    /// overwritten, not misread.
    made: (u32, u32),
}

impl DevRel {
    /// `self`, its descriptor's arity and base rows noted as they are now (`made`).
    fn noted(mut self) -> DevRel {
        if self.home.is_some() {
            let r = self.relation();
            self.made = (r.arity, r.base_rows);
        }
        self
    }

    /// A relation of the device's own, of `cols` columns and `rows` rows, in a block of the device's
    /// (`Part::for_device`, decision 0325 step 5), every row the device's to write.
    pub(in crate::code_rule_dsl) fn empty(cols: usize, rows: usize) -> Result<DevRel, Error> {
        let part = crate::code_rule_dsl::facts::rows::Part::for_device(cols, rows).map_err(|_| Error::BackendCapacity)?;
        let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
        let buffer = Held::kept(part.facts().block.gpu_buffer(device), Box::new(part.facts().clone()));
        debug_assert_eq!(part.rows_at(), part.rel_at() + REL_ROWS as usize);
        let home = Some(part.facts().clone());
        Ok(DevRel { buffer, rows, cols, at: part.rel_at() as u64, dead: 0, part: Some(part), home, view: None, made: (0, 0) }.noted())
    }

    /// A view of `cols` columns and `rows` rows read in place (decision 0353 step 4d): a relation of no columns of
    /// the device's own, which its indexes go in, and the view's directory, which its rows are read through.
    pub(in crate::code_rule_dsl) fn view(cols: usize, rows: usize, view: crate::code_rule_dsl::facts::vtab::ViewInPlace) -> Result<DevRel, Error> {
        let mut dev = DevRel::empty(0, rows)?;
        dev.cols = cols;
        dev.view = Some(view);
        Ok(dev)
    }

    /// Makes the buffers a view's reads reach by address resident in `pass` (none for a relation).
    pub(in crate::code_rule_dsl) fn resident_in(&self, pass: &Pass<'_>) {
        if let Some(v) = &self.view {
            let buffers: Vec<&BufferRef> = v.buffers.iter().map(|b| &**b).collect();
            pass.use_read(&buffers);
            if let Some(cache) = &v.cache {
                pass.use_read_write(cache);
            }
        }
    }

    /// A relation of `cols` columns and `rows` rows, its values given by `fill` (column `c` at
    /// `[c * rows .. (c + 1) * rows]`) and written by the host into a relation of the device's own.
// copy (0353 step 6): an upload of rows from the host. It goes with its last caller (decision 0353).
    pub(in crate::code_rule_dsl) fn upload(cols: usize, rows: usize, fill: impl FnOnce(&mut [i64])) -> Result<DevRel, Error> {
        // copy (0353 step 6): its site (row 5).
        static UPLOAD: rerail_prof::copies::Site = rerail_prof::copies::Site::new(5, "6", "rows uploaded from the host (DevRel::upload)");
        UPLOAD.time(|| (Self::upload_in(cols, rows, fill), 8 * cols * rows))
    }

    fn upload_in(cols: usize, rows: usize, fill: impl FnOnce(&mut [i64])) -> Result<DevRel, Error> {
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
        DevRel { buffer, rows: words, cols: 1, at: 0, dead: 0, part: None, home: None, view: None, made: (0, 0) }
    }

    /// Whether it is a relation read where it lies (`in_block`): it holds the relation's block while it lives.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn read_in_place(&self) -> bool {
        self.part.is_none() && self.view.is_none() && self.home.is_some()
    }

    /// A relation the device reads where it lies (decision 0325): `b`'s, of `cols` columns, bound by its block's own
    /// device buffer (which it keeps). Nothing is copied.
    pub(in crate::code_rule_dsl) fn in_block(b: &crate::code_rule_dsl::facts::rows::InBlock, cols: usize) -> Result<DevRel, Error> {
        assert!(b.at > 0, "a descriptor past the block's header");
        let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
        // The block, the relation and its pages: not given back while the device reads them (decisions 0342, Q76).
        let buffer = Held::kept(b.facts.block.gpu_buffer(device), Box::new(b.clone()));
        Ok(DevRel { buffer, rows: b.rows as usize, cols, at: b.at as u64, dead: b.dead as u64, part: None, home: Some(b.facts.clone()), view: None, made: (0, 0) }.noted())
    }

    /// Its relation's descriptor.
    fn relation(&self) -> &rerail_blocks::relation::Relation {
        let home = self.home.as_ref().expect("a relation, not words");
        // SAFETY: `at` is a relation's descriptor in `home`'s block, which `home` keeps.
        unsafe { &*home.block.ptr::<rerail_blocks::relation::Relation>(self.at as usize) }
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
            _ if self.view.is_some() => 0,
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
    // copy (0353 step 6): symbols' names copied for the device (row 5). They go: the kernels read the strings block.
    pub(in crate::code_rule_dsl) fn upload(ids: &[i64], name: impl FnMut(i64, &mut Vec<u8>)) -> Result<DevText, Error> {
        static NAMES: rerail_prof::copies::Site = rerail_prof::copies::Site::new(5, "6", "symbols' names copied for the device (DevText::upload)");
        let copying = NAMES.start();
        let text = Self::upload_in(ids, name);
        copying.done(text.as_ref().map_or(0, DevText::bytes));
        text
    }

    fn upload_in(ids: &[i64], mut name: impl FnMut(i64, &mut Vec<u8>)) -> Result<DevText, Error> {
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
    table: DevBytes,
    states: u32,
    classes: u32,
    start: u32,
}

impl DevTable {
    /// Test table `t` where the automata block holds it (decision 0450 step 2c): nothing uploaded.
    pub(in crate::code_rule_dsl) fn of(t: &crate::code_rule_dsl::text::table::Table) -> Result<DevTable, Error> {
        Ok(DevTable { table: DevBytes::of(&t.automaton)?, states: t.states, classes: t.classes, start: t.start })
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
    /// A source view read in place (`Stage::v_dir`, `r_view`; 0: none).
    v_dir: u64,
    r_view: u32,
    v_pad: u32,
}

const _: () = assert!(std::mem::size_of::<CopyParams>() == 256);

/// A relation's index on key columns, the relation's own (decision 0325 step 5): its number in the relation's
/// directory, read by `rel_group`. A relation whose block had no room for it was copied once into a block that has
/// (`moved`), which the probes then read.
pub(in crate::code_rule_dsl) struct DevIndex {
    pub(in crate::code_rule_dsl) index: u32,
    pub(in crate::code_rule_dsl) rows: usize,
    moved: Option<std::rc::Rc<DevRel>>,
    /// Whether this call built it (a sort made), or found it the relation's already.
    pub(in crate::code_rule_dsl) built: bool,
    /// No index: the `child` view probed by its parent reads the parent's children off the tape (`by_parent`).
    by_parent: bool,
    /// Its block's buffer, taken once the index has its room: what its builder and its probes bind. A block with no
    /// slab of its own is bound by a wrap of its used extent (`Block::gpu_buffer`), which a buffer taken before the
    /// index was added (its relation's) does not reach.
    buffer: Held,
}

impl DevIndex {
    /// The device memory it holds of its own: a moved relation's (the index lies in its relation's block).
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        self.moved.as_ref().map_or(0, |m| m.bytes())
    }

    /// The relation its probes read: `rel`, or the copy it was built on.
    pub(in crate::code_rule_dsl) fn of<'a>(&'a self, rel: &'a DevRel) -> &'a DevRel {
        self.moved.as_deref().unwrap_or(rel)
    }

    /// The buffer its probes bind: its block's, which reaches the index.
    fn bound(&self) -> &BufferRef {
        &self.buffer.buffer
    }
}

/// The buffer of `home`'s block as it is now (reaching what it holds now), the block kept with it.
fn bound(home: &std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>) -> Result<Held, Error> {
    let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
    Ok(Held::kept(home.block.gpu_buffer(device), Box::new(home.clone())))
}

/// A stage's probed relation `rel` (and its index, read by keys) into its fields.
fn bind_right(stage: &mut Stage, rel: &DevRel, index: Option<&DevIndex>) {
    stage.nr = rel.rows as u32;
    stage.r_at = rel.at;
    stage.r_dead = rel.dead;
    // A relation reads none: a stage bound before to a view reads its new relation, not the view.
    (stage.v_dir, stage.r_view) = rel.view.as_ref().map_or((0, 0), |v| (v.dir, v.kind));
    stage.v_parent = 0;
    if let Some(ix) = index {
        stage.r_index = ix.index;
        stage.v_parent = u32::from(ix.by_parent);
    }
}

/// An index whose builder runs in a batch: marked built (`Relation::index_built`) once the batch's work completes.
struct Building {
    home: std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>,
    rel: *const rerail_blocks::relation::Relation,
    room: rerail_blocks::relation::IndexRoom,
    /// The relation's descriptor as it was once the index was reserved (Q253): checked before the index is marked
    /// built, so a descriptor something overwrote meanwhile names what changed instead of being followed.
    before: Vec<u8>,
}

impl Building {
    /// The descriptor's bytes now.
    fn descriptor(rel: *const rerail_blocks::relation::Relation) -> Vec<u8> {
        // SAFETY: a relation's descriptor in a block its `home` keeps; plain words.
        unsafe { std::slice::from_raw_parts(rel.cast::<u8>(), std::mem::size_of::<rerail_blocks::relation::Relation>()) }.to_vec()
    }

    /// Panics, naming the words that changed, when the descriptor's arity, base rows or index directory are not as
    /// they were, or its index count went down (nothing else changes them while an index builds).
    fn check(&self, origin: &str) {
        use rerail_blocks::relation::Relation;
        use std::mem::{offset_of, size_of};
        let now = Building::descriptor(self.rel);
        let fixed = [(offset_of!(Relation, arity), 4), (offset_of!(Relation, base_rows), 4), (offset_of!(Relation, indexes), size_of::<rerail_blocks::Span<rerail_blocks::relation::Index>>())];
        // An index count only grows: another index reserved meanwhile adds one.
        let count = |b: &[u8]| u32::from_le_bytes(b[offset_of!(Relation, nindex)..][..4].try_into().unwrap());
        if fixed.iter().all(|&(at, n)| now[at..at + n] == self.before[at..at + n]) && count(&now) >= count(&self.before) {
            return;
        }
        let words = |b: &[u8]| b.as_chunks::<8>().0.iter().map(|w| format!("{:016x}", u64::from_le_bytes(*w))).collect::<Vec<_>>().join(" ");
        let changed: Vec<usize> = (0..now.len() / 8).filter(|&k| now[8 * k..8 * k + 8] != self.before[8 * k..8 * k + 8]).map(|k| 8 * k).collect();
        let h = self.home.block.header();
        panic!(
            "a relation's descriptor overwritten while its index was built (Q253): {origin}, at {} of a block of kind {} ({} used of {}), index {} (buckets at {}, rows at {}, {} rows); bytes changed at {changed:?}\n  before {}\n  now    {}",
            self.home.block.offset_of(self.rel), h.kind, self.home.block.used(), self.home.block.reserved(), self.room.index, self.room.offs, self.room.rows, self.room.rows_n, words(&self.before), words(&now)
        );
    }
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

    /// A table of the rows `rows` (row-major, `slots` values a row): a test's round of rows.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn of_rows(slots: usize, rows: &[i64]) -> Result<Table, Error> {
        let n = rows.len() / slots;
        let t = Table::new(slots, n)?;
        // SAFETY: the buffer holds `slots * cap` values, and nothing else refers to it yet.
        let out = unsafe { std::slice::from_raw_parts_mut(t.buffer.buffer.contents().cast::<i64>(), slots * t.cap) };
        for (i, row) in rows.chunks(slots).enumerate() {
            for (c, &v) in row.iter().enumerate() {
                out[c * t.cap + i] = v;
            }
        }
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

/// An automaton the device reads (a test's table, a regex's replacement automata) where the automata block holds it
/// (decision 0450 step 2c): the block's no-copy wrap, bound at the automaton's offset. Nothing is uploaded, and no
/// thread keeps it: a command buffer that binds it keeps the wrap.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct DevBytes {
    buffer: metal::Buffer,
    at: u64,
}

impl DevBytes {
    pub(in crate::code_rule_dsl) fn of(a: &crate::code_rule_dsl::text::automata::Automaton) -> Result<DevBytes, Error> {
        let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
        Ok(DevBytes { buffer: crate::code_rule_dsl::text::automata::block().gpu_buffer(device), at: a.at as u64 })
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
    /// The indexes its work builds, marked built when it is done.
    building: Vec<Building>,
    /// The relations its work writes in their blocks, each with its rows' `needed` before (decision 0427): a row
    /// its kernels could not write, past the chunks its relation holds, fails the batch, named.
    written: Vec<Written>,
}

/// A relation a batch's kernels write (`Batch::written`).
struct Written {
    home: std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>,
    at: u64,
    rows: usize,
    needed: u32,
    what: &'static str,
}

impl Batch {
    pub(in crate::code_rule_dsl) fn new() -> Result<Batch, Error> {
        rerail_accel::gpu::join::device()?;
        Batch::with_arena(ARENA_START)
    }

    /// A batch whose text arena holds `cap` bytes.
    pub(in crate::code_rule_dsl) fn with_arena(cap: usize) -> Result<Batch, Error> {
        rerail_accel::gpu::join::device()?;
        Ok(Batch { open: None, pass: None, committed: Vec::new(), done: Done::default(), dummy: Held::new(64)?, keep: Vec::new(), read: Vec::new(), held: Vec::new(), arena: None, arena_cap: cap, building: Vec::new(), written: Vec::new() })
    }

    /// Notes that its kernels write `rel` (`what`): its rows' `needed` now, checked when the batch is done.
    fn writes(&mut self, rel: &DevRel, what: &'static str) {
        let Some(home) = &rel.home else { return };
        let needed = rel.relation().rows.needed.load(std::sync::atomic::Ordering::Relaxed);
        self.written.push(Written { home: home.clone(), at: rel.at, rows: rel.rows, needed, what });
    }

    /// The relations its kernels wrote whose rows counted one they could not write (decision 0427): each named.
    fn overran(&self) -> Vec<String> {
        self.written
            .iter()
            .filter_map(|w| {
                // SAFETY: `at` is a relation's descriptor in `home`'s block, which `home` keeps.
                let r = unsafe { &*w.home.block.ptr::<rerail_blocks::relation::Relation>(w.at as usize) };
                let now = r.rows.needed.load(std::sync::atomic::Ordering::Relaxed);
                (now != w.needed).then(|| {
                    let held = r.rows.reserved.load(std::sync::atomic::Ordering::Relaxed);
                    format!("{}: {} rows past its chunks (Q253), a relation of {} rows its device side says, {} its chunks hold, {} claimed, {} columns, at {} of a block of kind {}",
                            w.what, now.wrapping_sub(w.needed), w.rows, held, r.rows.len.load(std::sync::atomic::Ordering::Relaxed), r.rows.ncols, w.at, w.home.block.header().kind)
                })
            })
            .collect()
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
            // Made in a pool of its own: `new_command_buffer` gives an autoreleased buffer, and a worker thread has no
            // pool to release it. Each one so left held a slot of the queue's (64) after its batch let it go, and a
            // run of many device strata stopped in `commandBuffer` with every slot taken (Q170).
            let queue = queue()?;
            let command = metal::objc::rc::autoreleasepool(|| queue.new_command_buffer().to_owned());
            rerail_accel::exec::command_opened();
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
        rerail_accel::exec::command_committed();
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

    /// Commits what is open and waits for every command buffer. A command buffer the GPU failed fails the batch: its
    /// outputs are partly written, so its strata run on the CPU, and the indexes it built are not marked built (a
    /// later batch would find one half written).
    pub(in crate::code_rule_dsl) fn finish(&mut self) -> Result<Done, Error> {
        self.submit_open()?;
        let mut failed = 0;
        for s in self.committed.drain(..) {
            let (seconds, ok) = s.wait_ok();
            self.done.gpu_seconds += seconds;
            failed += usize::from(!ok);
        }
        let overran = self.overran();
        self.written.clear();
        if !overran.is_empty() {
            // Nothing was written past a relation's rows (`rows_put`), but its rows are short: the batch's outputs are
            // not whole, so its strata run on the CPU, as a failed command buffer's do.
            for o in &overran {
                eprintln!("rerail: a device batch refused: {o}");
            }
            crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "a batch whose rows ran past a relation's (its strata on the CPU)", overran.len() as u64);
            failed += 1;
        }
        if failed > 0 {
            crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "a batch the GPU failed (its strata on the CPU)", failed as u64);
            self.building.clear();
            self.keep.clear();
            self.read.clear();
            self.held.clear();
            return Err(Error::BackendFailure);
        }
        for b in self.building.drain(..) {
            let _g = b.home.ix.lock().unwrap_or_else(|e| e.into_inner());
            b.check("its batch done");
            // SAFETY: the batch's work, which built it, has completed; `home` keeps the relation.
            unsafe { (*b.rel).index_built(&b.room) };
        }
        self.keep.clear();
        self.read.clear();
        self.held.clear();
        Ok(std::mem::take(&mut self.done))
    }

    /// `rel`, the `child` view read in place, probed by its parent (its column 0): no index of it is built, as a
    /// stage reads the parent's children off the tape (decision 0353 step 4c; the kernels' `by_parent`). `None` for
    /// any other relation or key.
    pub(in crate::code_rule_dsl) fn by_parent(&self, rel: &DevRel, cols: &[usize]) -> Result<Option<DevIndex>, Error> {
        use crate::code_rule_dsl::facts::vtab::syntax::device_chunk::VIEW_CHILD;
        let Some(v) = &rel.view else { return Ok(None) };
        if v.kind != 1 + VIEW_CHILD || cols != [0] {
            return Ok(None);
        }
        let home = rel.home.clone().expect("a view's relation of no columns");
        Ok(Some(DevIndex { index: NONE, rows: rel.rows, moved: None, built: false, by_parent: true, buffer: bound(&home)? }))
    }

    /// `rel`'s index on its columns `cols`, the relation's own (decision 0325 step 5): found where it was built
    /// before (by any batch, kept with the relation), else built by this batch's work into the relation's block
    /// (read by the stages after it), and marked built when the batch is done. A relation whose block has no room for
    /// it is copied once into a block of its own that has (with no dead rows: their bits lie in the old block).
    pub(in crate::code_rule_dsl) fn index(&mut self, rel: &DevRel, cols: &[usize]) -> Result<DevIndex, Error> {
        let home = rel.home.clone().expect("an index of a relation, not of words");
        let r = rel.relation();
        if r.base_rows != 0 {
            // Q253: what was read, and where, so a writer that missed its own buffer can be named.
            let h = home.block.header();
            let origin = if rel.view.is_some() { "a view read in place" } else if rel.part.is_some() { "the device's own" } else { "a relation in its block" };
            panic!(
                "an index of a full relation (Q253): {origin} of {} rows, {} columns, at {} of a block of kind {} (magic {:#x}, {} used of {}); its header: arity {}, base rows {} ({:#x}), {} indexes, {} order keys; when made: arity {}, base rows {}\n  descriptor {}",
                rel.rows, rel.cols, rel.at, h.kind, h.magic, home.block.used(), home.block.reserved(), r.arity, r.base_rows, r.base_rows, r.nindex.load(std::sync::atomic::Ordering::Relaxed), r.nokey, rel.made.0, rel.made.1,
                Building::descriptor(std::ptr::from_ref(r)).as_chunks::<8>().0.iter().map(|w| format!("{:016x}", u64::from_le_bytes(*w))).collect::<Vec<_>>().join(" ")
            );
        }
        let key: Vec<u16> = cols.iter().map(|&c| c as u16).collect();
        let room = {
            let _g = home.ix.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(i) = r.find_index(&key, rel.rows as u32) {
                return Ok(DevIndex { index: i as u32, rows: rel.rows, moved: None, built: false, by_parent: false, buffer: bound(&home)? });
            }
            if let Some(b) = self.building.iter().find(|b| std::ptr::eq(b.rel, r) && b.room.key[..b.room.ncols] == *key && b.room.rows_n as usize == rel.rows) {
                return Ok(DevIndex { index: b.room.index as u32, rows: rel.rows, moved: None, built: false, by_parent: false, buffer: bound(&home)? });
            }
            // SAFETY: under the block's index lock; the relation's rows are written (or are by this batch's work
            // before the build, which its passes order).
            unsafe { r.try_reserve_index(&home.block, &key, rel.rows as u32) }
        };
        let Some(room) = room else {
            if rel.dead != 0 {
                return Err(Error::BackendCapacity);
            }
            // A view's relation holds no values (its rows are read through the view's reads): a new one has room.
            if let Some(view) = &rel.view {
                let fresh = DevRel::view(rel.cols, rel.rows, view.clone())?;
                let ix = self.index(&fresh, cols)?;
                return Ok(DevIndex { moved: Some(std::rc::Rc::new(fresh)), ..ix });
            }
// copy (0353 step 6): a second key's index copies the relation. It goes: a relation is made with room for
// the indexes its readers probe (decision 0353).
            static MOVED: rerail_prof::copies::Site = rerail_prof::copies::Site::new(9, "6", "a relation copied for a second key's index (DevIndex::moved)");
            MOVED.copied(8 * rel.cols * rel.rows);
            let copy = DevRel::empty(rel.cols, rel.rows)?;
            let ident: Vec<Col> = (0..rel.cols).map(Col::From).collect();
            self.copy_rows(Rows::Rel(rel), rel.rows, &ident, Rows::Rel(&copy), 0)?;
            let ix = self.index(&copy, cols)?;
            return Ok(DevIndex { moved: Some(std::rc::Rc::new(copy)), ..ix });
        };
        let n = rel.rows;
        let buffer = bound(&home)?;
        assert!(room.rows as u64 + 4 * n as u64 <= buffer.buffer.length(), "the index's room within the buffer bound");
        if n > 0 {
            let bucket = pipeline("join", "ix_bucket", || SOURCE.to_owned())?;
            let place = pipeline("join", "ix_place", || SOURCE.to_owned())?;
            let bits = (u64::BITS - room.mask.leading_zeros()).max(8).div_ceil(8) * 8;
            // A sort is at most 4 passes of three dispatches and two scans' levels, and a few more.
            let pass = self.pass(48)?;
            let words = Held::new(4 * n)?;
            let mut p = vec![n as u32, cols.len() as u32, rel.at as u32, (rel.at >> 32) as u32, room.mask as u32, (room.mask >> 32) as u32];
            p.extend(cols.iter().map(|&c| c as u32));
            // A sort refused after the bucket's dispatch leaves `words`, which the dispatch writes, with the batch
            // until its command buffer is done: dropped here, it would go back to the pool at once.
            rel.resident_in(pass);
            let view = rel.view.as_ref().map_or([0, 0], |v| [v.dir as i64, i64::from(v.kind)]);
            pass.run(&bucket, &[&buffer.buffer, &words.buffer], &p, &view, groups(n));
            let parts = match encode_sort(pass, &words.buffer, Width::W32, bits, n) {
                Ok(parts) => parts,
                Err(e) => {
                    self.keep.push(words);
                    return Err(e.into());
                }
            };
            let at = |o: usize| [o as u32, (o as u64 >> 32) as u32];
            let mut q = vec![n as u32, (room.mask + 1) as u32];
            q.extend(at(room.offs));
            q.extend(at(room.rows));
            pass.run(&place, &[&buffer.buffer, &parts.perm.buffer, &parts.starts.buffer, &parts.rkeys.buffer, &parts.nruns.buffer], &q, &[], groups(n));
            self.keep.extend([words, parts.perm, parts.starts, parts.rkeys, parts.nruns]);
        }
        let index = room.index as u32;
        let before = Building::descriptor(std::ptr::from_ref(r));
        self.building.push(Building { home, rel: std::ptr::from_ref(r), room, before });
        Ok(DevIndex { index, rows: n, moved: None, built: true, by_parent: false, buffer })
    }

    /// The columns `cols` (its source's) of a view made from a chunk block on the device (decision 0289 §2), into
    /// `out`, a relation of `out.rows` rows whose columns are `cols` in order: what `Viewed::row_at` made on the host.
// copy (0353 step 4): a view expanded into a relation (660.8 MB a run on hogql). It goes: a view is a
// read of the tape, and a stage reads it in place (decision 0353).
    pub(in crate::code_rule_dsl) fn expand_view(&mut self, block: &metal::Buffer, root: usize, view: u32, cols: &[usize], out: &DevRel) -> Result<(), Error> {
        let expand = pipeline("chunk_views", "view_expand", chunk_views_source)?;
        static EXPANDED: rerail_prof::copies::Site = rerail_prof::copies::Site::new(4, "4", "a view expanded into a device relation (expand_view)");
        EXPANDED.copied(8 * out.rows * cols.len());
        self.writes(out, "a view expanded");
        let pass = self.pass(1)?;
        let at = |o: usize| u32::try_from(o).map_err(|_| Error::BackendCapacity);
        let rows_at = out.rows_at();
        let mut p = vec![at(root)?, view, out.rows as u32, cols.len() as u32, 0, rows_at as u32, (rows_at >> 32) as u32];
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
        // A probe by keys reads the relation its index is of (`DevIndex::of`), the index in its block.
        let rbuf: metal::Buffer = match right {
            Some((rel, index)) => {
                let rel = index.map_or(rel, |ix| ix.of(rel));
                bind_right(&mut stage, rel, index);
                index.map_or(&*rel.buffer.buffer, |ix| ix.bound()).to_owned()
            }
            None => self.dummy.buffer.to_owned(),
        };
        if left.cap == 1 && stage.mode == JOIN && right.is_some() {
            // From one binding: a thread per candidate row (`scan_count`), its rows in the relation's order.
            let scount = pipeline("join", "scan_count", || SOURCE.to_owned())?;
            let sscatter = pipeline("join", "scan_scatter", || SOURCE.to_owned())?;
            let n = stage.nr as usize + 1;
            let counts = scratch(FREE, 4 * n)?;
            let scan = Scan::new(n)?;
            let pass = self.pass(12)?;
            if let Some((rel, index)) = right {
                index.map_or(rel, |ix| ix.of(rel)).resident_in(pass);
            }
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
        if let Some((rel, index)) = right {
            index.map_or(rel, |ix| ix.of(rel)).resident_in(pass);
        }
        let lbuf: &BufferRef = &left.buffer.buffer;
        let count_bufs: [&BufferRef; 6] = [lbuf, &rbuf, &totals.buffer.buffer, &counters.buffer.buffer, &counts, &arena];
        let scatter_bufs: [&BufferRef; 7] = [lbuf, &rbuf, &totals.buffer.buffer, &counts, &out.buffer.buffer, &arena, &counters.buffer.buffer];
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
        stage.agg[0] = rest.len() as u32;
        let dummy: metal::Buffer = self.dummy.buffer.to_owned();
        let rel = right.1.map_or(right.0, |ix| ix.of(right.0));
        bind_right(&mut stage, rel, right.1);
        let a = right.1.map_or(&*rel.buffer.buffer, |ix| ix.bound()).to_owned();
        let mut probes = [Stage::default(); WITNESS_PROBES];
        let mut bufs: Vec<metal::Buffer> = Vec::new();
        for (k, (b, rel, ix)) in rest.iter().enumerate() {
            let mut b = *b;
            let rel = ix.map_or(*rel, |ix| ix.of(rel));
            bind_right(&mut b, rel, *ix);
            b.cap_in = left.cap as u32;
            probes[k] = b;
            bufs.push(ix.map_or(&*rel.buffer.buffer, |ix| ix.bound()).to_owned());
        }
        while bufs.len() < WITNESS_PROBES {
            bufs.push(dummy.clone());
        }
        let held = Held::of(probes.iter().flat_map(|p| p.bytes().iter().copied()).collect::<Vec<u8>>().into_iter()).map_err(|_| Error::BackendCapacity)?;
        let n = left.cap + 1;
        let counts = scratch(FREE, 4 * n)?;
        let scan = Scan::new(n)?;
        let pass = self.pass(3)?;
        rel.resident_in(pass);
        for &(_, r, ix) in rest {
            ix.map_or(r, |ix| ix.of(r)).resident_in(pass);
        }
        let lbuf: &BufferRef = &left.buffer.buffer;
        let mut count_bufs: Vec<&BufferRef> = vec![lbuf, &a, &totals.buffer.buffer, &counts, &held.buffer];
        count_bufs.extend(bufs.iter().map(|x| &**x));
        pass.run_bytes(&count, &count_bufs, stage.bytes(), groups(n));
        scan.encode(pass, &counts)?;
        // The bindings with a witness out as they are: a semi-join's scatter (mode 1 writes the binding alone).
        let mut write = stage;
        write.mode = SEMI;
        write.nbinds = 0;
        let scatter_bufs: [&BufferRef; 7] = [lbuf, &a, &totals.buffer.buffer, &counts, &out.buffer.buffer, &dummy, &dummy];
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
        let (g, g_at): (metal::Buffer, u64) = auto.map_or_else(|| (self.dummy.buffer.to_owned(), 0), |a| (a.0.buffer.to_owned(), a.0.at));
        let pass = self.pass(4)?;
        let lbuf: &BufferRef = &left.buffer.buffer;
        pass.run_bytes_at(&text, &[(lbuf, 0), (&names.buffer.buffer, 0), (&arena, 0), (&totals.buffer.buffer, 0), (&counters.buffer.buffer, 0), (&counts, 0), (&values.buffer, 0), (&g, g_at), (&work.buffer, 0), (&p.buffer, 0)],
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
    /// holds, at most `cap`; `cap` of them with no slot) into rows `at..` of `dst`, a relation in its block whose
    /// chunks hold `at + cap` rows (the head's own tail past its rows, decision 0384 step 3). Each column of the head
    /// is a column of `src` or a constant (`cols`). The batch keeps `dst`'s block until its work completes.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn place_rows(&mut self, src: Rows<'_>, cap: usize, count: Option<usize>, totals: &Words, cols: &[Col], dst: DevRel, at: usize) -> Result<(), Error> {
        self.copy(src, cap, count.map(|c| (c, totals)), cols, Rows::Rel(&dst), at)?;
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
        if let Rows::Rel(r) = dst {
            self.writes(r, "rows copied");
        }
        assert!(n <= src_rows.max(1), "a copy of {n} rows of a source of {src_rows}");
        assert!(dst_off + n <= dst_rows, "a copy of {n} rows at {dst_off} into a destination of {dst_rows}");
        let mut p = CopyParams {
            konst: [0; 16], src: [NONE; 16], n: n as u32, ncols: cols.len() as u32, src_stride: src_rows as u32, dst_stride: dst_rows as u32,
            dst_off: dst_off as u32, count_slot: count.map_or(NONE, |c| c.0 as u32), pad: [0; 2], src_at, dst_rows_at, v_dir: 0, r_view: 0, v_pad: 0,
        };
        let view = match src {
            Rows::Rel(r) => r.view.as_ref().map(|_| r),
            _ => None,
        };
        if let Some(v) = view.and_then(|r| r.view.as_ref()) {
            p.v_dir = v.dir;
            p.r_view = v.kind;
        }
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
        if let Some(r) = view {
            r.resident_in(pass);
        }
        pass.run_bytes(&copy, &[&s, &d, &totals], bytes, groups(n));
        Ok(())
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
        pass.run_bytes_at(&kernel, &[(&text.packed.buffer.buffer, 0), (&table.table.buffer, table.table.at), (&bits.buffer.buffer, 0), (&scratch.buffer, 0)], bytes, (groups * 32).div_ceil(256));
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
            pass.run_bytes_at(&kernel, &[(&text.packed.buffer.buffer, 0), (&table.buffer, table.at), (&z.buffer, 0), (&lens.buffer, 0), (&out.buffer.buffer, 0), (&scratch.buffer, 0)], bytes, (groups * 32).div_ceil(256));
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
        if self.open.take().is_some() {
            // Not this thread's to commit any more (decision 0386's count).
            rerail_accel::exec::command_committed();
        }
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

    /// A relation's index built on the device (decision 0325 step 5) is the host's (`Relation::build_index`): the same
    /// buckets and the same rows in each, in id order, for a key of one column and one of two, over many duplicate
    /// keys. Built once: the next request, in the same batch or a later one, finds it the relation's own.
    #[test]
    fn the_devices_index_of_a_relation_is_the_hosts() {
        if rerail_accel::gpu::join::device().is_err() {
            return;
        }
        // Its index (some 1.8 MB) past the megabyte a host block's wrap rounds its used extent up to.
        let n = 200_000;
        let (c0, c1) = (rows(n, 31, 97), rows(n, 32, 13));
        // A device relation's block has room for one index: one relation a key.
        let up = || {
            DevRel::upload(2, n, |v| {
                v[..n].copy_from_slice(&c0);
                v[n..].copy_from_slice(&c1);
            })
            .unwrap()
        };
        let (dev0, dev1) = (up(), up());
        // A host relation in its block, bound before its index was added: a block without a slab of its own is bound
        // by a wrap of its used extent, which the index's room lies past. And the host's own builder's, the reference.
        let vals: Vec<i64> = (0..n).flat_map(|i| [c0[i], c1[i]]).collect();
        let host_rows = || DevRel::in_block(&crate::code_rule_dsl::facts::rows::anon_in_block(2, &vals), 2).unwrap();
        let (placed, host) = (host_rows(), host_rows());
        for (dev, cols) in [(&dev0, vec![0usize]), (&dev1, vec![1, 0]), (&placed, vec![0]), (&placed, vec![1, 0])] {
            let mut batch = Batch::new().unwrap();
            let ix = batch.index(dev, &cols).unwrap();
            assert!(ix.built && ix.moved.is_none(), "built in the relation's own block");
            let again = batch.index(dev, &cols).unwrap();
            assert!(!again.built && again.index == ix.index, "one build a batch");
            batch.finish().unwrap();
            let key: Vec<u16> = cols.iter().map(|&c| c as u16).collect();
            let (r, h) = (dev.relation(), host.relation());
            assert_eq!(r.find_index(&key, n as u32), Some(ix.index as usize), "marked built once the batch is done");
            let hosts = if let Some(i) = h.find_index(&key, n as u32) {
                i
            } else {
                let home = host.home.as_ref().unwrap();
                // SAFETY: one thread; the rows are written.
                let room = unsafe { h.try_reserve_index(&home.block, &key, n as u32) }.unwrap();
                unsafe {
                    h.build_index(&room);
                    h.index_built(&room);
                }
                room.index
            };
            let (d, w) = (r.index(ix.index as usize), h.index(hosts));
            if std::ptr::eq(dev, &placed) {
                let end = dev.home.as_ref().unwrap().block.offset_of(d.rows.as_slice().as_ptr()) + 4 * n;
                assert!(end as u64 > placed.buffer.buffer.length(), "the case: the index past the buffer its relation was bound by");
            }
            assert_eq!(d.mask, w.mask);
            assert_eq!(d.offs.as_slice(), w.offs.as_slice(), "{cols:?}: the buckets");
            assert_eq!(d.rows.as_slice(), w.rows.as_slice(), "{cols:?}: each bucket's rows, in id order");
            let mut later = Batch::new().unwrap();
            let found = later.index(dev, &cols).unwrap();
            assert!(!found.built && found.index == ix.index, "a later batch finds it");
            later.finish().unwrap();
        }
    }

    /// Decision 0325 step 3: relations read where they lie in their blocks, with dead rows marked by a delta's dead
    /// bits (`Rows::in_block_dead`), as a maintained relation's kills of the rows it shares. A scan, a keyed join
    /// (through the device's own index of the relation) and an anti-join skip the dead rows: the nested loops'
    /// rows over the live ones.
    #[test]
    fn stages_skip_the_dead_rows_of_relations_in_their_blocks() {
        use crate::code_rule_dsl::facts::rows::Rows;
        if rerail_accel::gpu::join::device().is_err() {
            return;
        }
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
            (DevRel::in_block(&b, k).unwrap(), r)
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

}

/// A lattice relation's key map where it lies (decision 0384 step 5, `setfix::keys::KeyMap`): its block, its slots'
/// offset there, and how many slots (a power of two).
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) struct LatMap<'a> {
    pub(in crate::code_rule_dsl) home: &'a std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>,
    pub(in crate::code_rule_dsl) at: usize,
    pub(in crate::code_rule_dsl) places: usize,
}

/// A lattice the device folds by (`Batch::lattice_fold`): 0 min, 1 max, 2 flat, 3 shortlex (its values arena texts,
/// decision 0307); its top; `widen`'s count.
#[derive(Clone, Copy, Debug)]
pub(in crate::code_rule_dsl) struct LatCode {
    pub(in crate::code_rule_dsl) code: u32,
    pub(in crate::code_rule_dsl) top: i64,
    pub(in crate::code_rule_dsl) after: Option<u32>,
}

/// What a fold does to its key map first: nothing, empty it (new, its words not written), or empty it and place
/// every key of the old one in it (grown).
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) enum MapFirst<'a> {
    Ready,
    Clear,
    Grow(LatMap<'a>),
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LatParams { top: i64, n: u32, had: u32, nkeys: u32, code: u32, after: u32, pmask: u32, hmask: u32, level: u32, omask: u32, pad: u32,
                   r_at: u64, r_rows_at: u64, h_at: u64, h0_at: u64 }

const _: () = assert!(std::mem::size_of::<LatParams>() == 80);

impl Batch {
    /// A lattice relation's round (decision 0124): the `n` rows of `u` (its key's `nkeys` columns, then the value)
    /// folded by key into `r`, the relation itself (`had` rows held, one live row a key), through its key map `map`
    /// (decision 0384 step 5), which `first` empties or grows first. A held key's row is updated where it lies; a new
    /// key's row is written at `had` on (room the caller reserved for `n` more) and its slot put in the map. Each
    /// changed key's row (key and value) into `delta`. `counters`: `[changed, new]`. `level`: where the relation keeps
    /// levels (decision 0311), the round, written to a changed or new key's slot.
    #[allow(clippy::too_many_arguments)]
    /// Code 3 compares its values' texts in `arena`, which holds every value the fold reads.
    pub(in crate::code_rule_dsl) fn lattice_fold(&mut self, u: &Table, n: usize, r: &DevRel, had: usize, nkeys: usize, lat: LatCode, map: LatMap<'_>,
                                                 first: MapFirst<'_>, counters: &Words, delta: &Table, arena: Option<&Arena>, level: Option<u32>) -> Result<(), Error> {
        assert!(lat.code != 3 || arena.is_some(), "a shortlex fold reads its values' texts");
        assert!(nkeys + 1 <= MAX_KEYS && u.cap == n && delta.cap == n);
        assert!(map.places.is_power_of_two() && 2 * n <= map.places, "a key map of {} slots for {n} rows", map.places);
        let pplaces = (2 * n).next_power_of_two().max(16);
        let mut p = LatParams { top: lat.top, n: n as u32, had: had as u32, nkeys: nkeys as u32, code: lat.code, after: lat.after.unwrap_or(NONE), pmask: (pplaces - 1) as u32,
                                hmask: (map.places - 1) as u32, level: level.unwrap_or(0), omask: 0, pad: 0, r_at: r.at, r_rows_at: r.rows_at(), h_at: map.at as u64, h0_at: 0 };
        // The map's block as it is now (reaching its slots), kept until the work completes.
        let hb = bound(map.home)?;
        // Its rows written in place, bounded by their chunks (decision 0427): checked when the work completes.
        self.writes(r, "a lattice's held rows");
        if !matches!(first, MapFirst::Ready) {
            let hclear = pipeline("join", "lat_hclear", || SOURCE.to_owned())?;
            let pass = self.pass(2)?;
            pass.run_bytes(&hclear, &[&hb.buffer], lat_bytes(&p), groups(map.places));
            if let MapFirst::Grow(old) = first {
                p.omask = (old.places - 1) as u32;
                p.h0_at = old.at as u64;
                let ob = bound(old.home)?;
                let rehash = pipeline("join", "lat_rehash", || SOURCE.to_owned())?;
                pass.run_bytes(&rehash, &[&r.buffer.buffer, &ob.buffer, &hb.buffer], lat_bytes(&p), groups(old.places));
                self.keep.push(ob);
            }
        }
        let bytes = lat_bytes(&p);
        let clear = pipeline("join", "lat_clear", || SOURCE.to_owned())?;
        let claim = pipeline("join", "lat_claim", || SOURCE.to_owned())?;
        let best = pipeline("join", "lat_best", || SOURCE.to_owned())?;
        let merge = pipeline("join", "lat_merge", || SOURCE.to_owned())?;
        let places = Held::new(4 * pplaces)?;
        let chosen = Held::new(4 * pplaces)?;
        let flags = Held::new(4 * pplaces)?;
        let a: metal::Buffer = arena.map_or_else(|| self.dummy.buffer.to_owned(), |a| a.buffer.buffer.to_owned());
        let pass = self.pass(4)?;
        let ub: &BufferRef = &u.buffer.buffer;
        pass.run_bytes(&clear, &[&places.buffer, &chosen.buffer, &flags.buffer], bytes, groups(pplaces));
        pass.run_bytes(&claim, &[ub, &places.buffer], bytes, groups(n));
        pass.run_bytes(&best, &[ub, &places.buffer, &chosen.buffer, &flags.buffer, &a], bytes, groups(n));
        pass.run_bytes(&merge, &[ub, &places.buffer, &chosen.buffer, &flags.buffer, &r.buffer.buffer, &hb.buffer, &delta.buffer.buffer, &counters.buffer.buffer, &a], bytes, groups(pplaces));
        self.keep.extend([places, chosen, flags, hb]);
        Ok(())
    }
}

fn lat_bytes(p: &LatParams) -> &[u8] {
    // SAFETY: `LatParams` is `repr(C)`: an i64, eight u32s, two u64s, no padding (its size asserted).
    unsafe { std::slice::from_raw_parts((p as *const LatParams).cast::<u8>(), std::mem::size_of::<LatParams>()) }
}
