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
//! **One submission.** A `Batch` encodes every stage of every rule it is given into one submission of the device
//! layer's queue (serial: each dispatch sees the last one's writes), and commits a new one only when the launch budget
//! of one would be exceeded. The kernels are UKL (`join_stages.uglykl`, `chunk_views.uglykl`; decision 0440 step C),
//! on every backend; a view read in place (decision 0353 step 4d) is read through its directory's addresses, which
//! only a device with them has (`ViewInPlace`, made on Metal alone). A table's capacity is fixed when its stage is
//! encoded; a stage whose rows overflow it writes none past it and reports its true count, and the caller runs it again
//! from that stage with the exact size.
//!
//! Every loop has a bound that follows from its input, and threadgroup memory is not used.
//!
//! A parameter block past a launch's own (a stage's, a copy's, a dedup's, an index's: their arrays and a view's
//! address) lies in a buffer of its own (`Batch::block`), which the batch keeps until its work completes.

use super::Error;
use rerail_accel::gpu::join::{FREE, Held, Pass, Scan, Submitted, Width, encode_sort, groups, pipeline, scratch, simd_group};
use rerail_device::device::Buffer;


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
/// A function of symbols' names, each a slot or a constant (keys 0 and 1), their texts read in place through the
/// batch's atoms directory (decision 0478 step 2e), its value into `bind_slot[0]`: by `agg[0]`, `NAMES_CMP` (`str_cmp`) or `NAMES_LEN` (`strlen`).
pub(in crate::code_rule_dsl) const STRCMP: u32 = 6;
pub(in crate::code_rule_dsl) const NAMES_CMP: u32 = 0;
pub(in crate::code_rule_dsl) const NAMES_LEN: u32 = 1;
/// A literal text test of a pattern and a subject (keys 0 and 1, 2 and 3), each a symbol or a cat's text never made
/// (`OPND_*`, `agg[1]`: the pattern's kind, the subject's << 4): `agg[0]` is the want (bit 0), the test (`TEXT_*`
/// << 1), `TEXT_SPLIT` when it is a text test (whose pattern a set separator would split), and `TEXT_BOUND` when its
/// pattern is bound (a relation's value, not the rule's constant set).
pub(in crate::code_rule_dsl) const TEXTOP: u32 = 7;
pub(in crate::code_rule_dsl) const TEXT_EQ: u32 = 0;
pub(in crate::code_rule_dsl) const TEXT_PREFIX: u32 = 1;
pub(in crate::code_rule_dsl) const TEXT_SUFFIX: u32 = 2;
pub(in crate::code_rule_dsl) const TEXT_SPLIT: u32 = 1 << 4;
/// A bound pattern holding the set separator is no set (decision 0478 step 2e): its row sets `TEXTFN_HOST`, and the
/// stratum runs on the host. A constant set is split.
pub(in crate::code_rule_dsl) const TEXT_BOUND: u32 = 1 << 5;
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
/// A text test of a symbol (key 0) by its table, its verdict read from and kept in the one rolling cache (decision
/// 0478 step 2d): the symbol's bytes read through the atoms directory where a verdict is not kept yet.
pub(in crate::code_rule_dsl) const ROW_VERDICT: u32 = 5;
/// With a text function (`ROW_MAP`, `ROW_REPLACE`) of constant arguments of a symbol, or a chain of them: its text kept
/// in the one rolling cache (decision 0478 step 2e) by the functions' id (`P[5]`) and the source symbol (key 7). A
/// kept text is copied into the arena; one made is kept, up to `atoms::TEXT_MOST` bytes.
pub(in crate::code_rule_dsl) const ROW_KEPT: u32 = 1 << 9;
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
/// A `ROW_VERDICT` stage met a symbol the atoms directory does not hold: the stratum runs on the host.
pub(in crate::code_rule_dsl) const TEXTFN_UNHELD: u32 = 4;
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

const _: () = assert!(std::mem::size_of::<Stage>() == 2320 && std::mem::size_of::<Stage>().is_multiple_of(16));

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

/// The chunk views' kernels (decision 0289 §2): the chunk's structures as `rerail-blocks` generates them, then
/// `view_expand`, which makes each row as `Viewed::value_at` does, and the axes' solve (`chunk_views.uglykl`).
fn chunk_views_source() -> String {
    format!(
        "{}{}{}{}",
        rerail_accel::gpu::join::COMMON,
        rerail_blocks::RELATION_READS,
        crate::code_rule_dsl::facts::vtab::syntax::device_chunk::ukl(),
        include_str!("chunk_views.uglykl")
    )
}

/// Every join kernel's source (`join_stages.uglykl`): the relation's reads written once (`rerail_blocks::RELATION_READS`,
/// decision 0325), which a stage reads a relation in its block with, and the views' reads (decision 0353 step 4d),
/// which a stage reads a view in place with: the tape's, the numbering's and the views' own, each document's view read
/// where the batch's directory holds it (`UKL_GLOBAL`).
static SOURCE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "#define CLIF_VIEW_SPACE UKL_GLOBAL\n{}{}{}{}{}{}",
        rerail_accel::gpu::join::COMMON,
        rerail_blocks::RELATION_READS,
        // The reads that read a 16z tape too, through its batch's cache (decision 0353 step 4b).
        "#include <ukl/reads/tape_z.h>\n",
        "#include <ukl/reads/ids.h>\n",
        "#include <ukl/reads/view.h>\n#include <ukl/bits.h>\n",
        include_str!("join_stages.uglykl")
    )
});

/// Whether the process has the device the stages run on.
pub fn available() -> bool {
    rerail_accel::gpu::join::device().is_ok()
}

/// A kind-set axis's batch solve on the device (decision 0289 §2, `facts::nearest::ChunkAxis`), from `chunks`: each
/// node's rows counted by a climb of its ancestors and the counts scanned into the starts (in its chunk, page-aligned,
/// bound as a buffer of their own with no copy); then, the totals read, the rows written, each the ids of `n` and `a`,
/// into one relation of exactly that many rows (decision 0353 step 5), each chunk's past the one's before (a chunk's
/// batch and its tape values, decision 0431 §8). It is a derived fact like a rule's head: it lies in a run's block,
/// not leased from the device's budget, which a room sized before the count (containment's up to several rows a node)
/// filled. Its starts are the first chunk's.
pub fn axis_in_chunk(chunks: &[&crate::code_rule_dsl::facts::vtab::syntax::device_chunk::Chunk], bits: Option<&[u32]>, inside: bool)
                     -> Result<crate::code_rule_dsl::facts::nearest::InChunk, String> {
    use crate::code_rule_dsl::facts::rows::Part;
    use crate::code_rule_dsl::facts::vtab::syntax::device_chunk::ChunkRoot;
    const PAGE: usize = 16384;
    let full = || "the chunk block cannot hold the axis".to_string();
    let at = |o: usize| u32::try_from(o).map_err(|_| "the axis lies past 4 GB of its chunk".to_string());
    let device = rerail_accel::gpu::join::device()?;
    let count = pipeline("chunk_views", "axis_count", chunk_views_source)?;
    let write = pipeline("chunk_views", "axis_write", chunk_views_source)?;
    // Each chunk's counts scanned into its starts: its parameters, its starts' offset, and its rows.
    let mut each: Vec<([u32; 8], usize, usize)> = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let b = &chunk.block;
        // SAFETY: the chunk's root, which `device_chunk::build` wrote.
        let n = unsafe { (*b.ptr::<ChunkRoot>(chunk.root)).nodes } as usize;
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
        let whole = b.device_buffer(device)?;
        // SAFETY: the starts' pages lie in the chunk's block, which outlives the work: it is waited for below.
        let starts = unsafe { rerail_device::device::wrap_host_memory(device, b.ptr::<u8>(start_at), start_bytes)? };
        let p = [at(chunk.root)?, n as u32, at(bits_at)?, u32::from(inside), 0, 0, 0, 0];
        let scan = Scan::owned(n)?;
        let mut batch = Batch::new().map_err(|e| format!("{e:?}"))?;
        let pass = batch.pass(16).map_err(|e| format!("{e:?}"))?;
        pass.run(&count, &[&whole, &starts], &p, &[], groups(n + 1));
        scan.encode(pass, &starts)?;
        batch.finish().map_err(|e| format!("{e:?}"))?;
        // SAFETY: the scan completed: its last value is the total.
        let len = unsafe { *b.ptr::<u32>(start_at + 4 * n) } as usize;
        each.push((p, start_at, len));
    }
    let total: usize = each.iter().map(|e| e.2).sum();
    let first_start = each.first().map_or(0, |e| e.1);
    // Alone in its block, with room for an index by each column: strata probe an axis by `n` and by `a`.
    let rows = Part::with_index_room(crate::code_rule_dsl::facts::rows::Life::Run, 2, total, 2);
    if total > 0 {
        let needed = rows.rel().rows.needed.load(std::sync::atomic::Ordering::Relaxed);
        let rows_at = rows.rows_at() as i64;
        let out = rows.facts().block.device_buffer(device)?;
        let mut past = 0usize;
        for (chunk, (mut p, start_at, len)) in chunks.iter().zip(each) {
            let b = &chunk.block;
            let n = p[1] as usize;
            // p[4]: the rows the relation holds; p[7]: the first of this chunk's (b0: its rows' descriptor).
            (p[4], p[7]) = (total as u32, past as u32);
            past += len;
            if len == 0 {
                continue;
            }
            let whole = b.device_buffer(device)?;
            // SAFETY: as the count's: the chunk's block outlives the work, waited for below.
            let starts = unsafe { rerail_device::device::wrap_host_memory(device, b.ptr::<u8>(start_at), (4 * (n + 1)).next_multiple_of(PAGE))? };
            let mut batch = Batch::new().map_err(|e| format!("{e:?}"))?;
            let pass = batch.pass(1).map_err(|e| format!("{e:?}"))?;
            pass.run(&write, &[&whole, &starts, &out], &p, &[rows_at], groups(n));
            batch.finish().map_err(|e| format!("{e:?}"))?;
        }
        // Bounded by the relation's chunks (decision 0463): what the writer refused fails the solve, which the host
        // then makes.
        let now = rows.rel().rows.needed.load(std::sync::atomic::Ordering::Relaxed);
        if now != needed {
            return Err(format!("the axis's writer refused {} rows past its relation's chunks (decision 0463)", now.wrapping_sub(needed)));
        }
    }
    // SAFETY: the writes completed, every row below the total written.
    unsafe { rows.written(total as u32) };
    Ok(crate::code_rule_dsl::facts::nearest::InChunk { start: first_start, rows })
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
    /// Its descriptor's arity and base rows when it was made (Q265): a descriptor found otherwise later was
    /// overwritten, not misread.
    made: (u32, u32),
    /// The relation's holder, for one read where it lies in its block (`in_block`): what an index built into it keeps
    /// until its batch is done (`Building`, Q268).
    kept: Option<crate::code_rule_dsl::facts::rows::Part>,
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
        DevRel::of_part(part, cols, rows)
    }

    /// A handle to `part`, a relation of the device's own of `cols` columns and `rows` rows (`Part::for_device`'s).
    fn of_part(part: crate::code_rule_dsl::facts::rows::Part, cols: usize, rows: usize) -> Result<DevRel, Error> {
        let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
        let buffer = Held::kept(part.facts().block.device_buffer(device)?, Box::new(part.facts().clone()));
        debug_assert_eq!(part.rows_at(), part.rel_at() + REL_ROWS as usize);
        let home = Some(part.facts().clone());
        Ok(DevRel { buffer, rows, cols, at: part.rel_at() as u64, dead: 0, part: Some(part), home, view: None, made: (0, 0), kept: None }.noted())
    }

    /// A view of `cols` columns and `rows` rows read in place (decision 0353 step 4d): a relation of no columns of
    /// the device's own, which its indexes go in, and the view's directory, which its rows are read through.
    pub(in crate::code_rule_dsl) fn view(cols: usize, rows: usize, view: crate::code_rule_dsl::facts::vtab::ViewInPlace) -> Result<DevRel, Error> {
        let mut dev = DevRel::empty(0, rows)?;
        dev.cols = cols;
        dev.view = Some(view);
        Ok(dev)
    }

    /// `view`, its indexes in `home` (the view's own relation of no columns, which its source holds: decision 0478
    /// step 3), so a stratum finds the indexes any stratum before it built there.
    pub(in crate::code_rule_dsl) fn view_in(home: crate::code_rule_dsl::facts::rows::Part, cols: usize, rows: usize, view: crate::code_rule_dsl::facts::vtab::ViewInPlace) -> Result<DevRel, Error> {
        let mut dev = DevRel::of_part(home, 0, rows)?;
        dev.cols = cols;
        dev.view = Some(view);
        Ok(dev)
    }

    /// Makes the buffers a view's reads reach by address resident in `pass` (none for a relation).
    pub(in crate::code_rule_dsl) fn resident_in(&self, pass: &Pass) {
        if let Some(v) = &self.view {
            let buffers: Vec<&Buffer> = v.buffers.iter().collect();
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
        const UPLOAD: rerail_prof::copies::Site = rerail_prof::copies::Site::new(5, "6", "rows uploaded from the host (DevRel::upload)");
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
        DevRel { buffer, rows: words, cols: 1, at: 0, dead: 0, part: None, home: None, view: None, made: (0, 0), kept: None }
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
        let buffer = Held::kept(b.facts.block.device_buffer(device)?, Box::new(b.clone()));
        Ok(DevRel { buffer, rows: b.rows as usize, cols, at: b.at as u64, dead: b.dead as u64, part: None, home: Some(b.facts.clone()), view: None, made: (0, 0), kept: b.part.clone() }.noted())
    }

    /// What holds its relation: its own, or the one read in its block (none for words, or a test's relation no
    /// `Part` holds).
    fn holder(&self) -> Option<crate::code_rule_dsl::facts::rows::Part> {
        self.part.clone().or_else(|| self.kept.clone())
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

/// An atoms directory for the device (decision 0478 step 2b): its spans device addresses, the atoms blocks they reach
/// (each its device buffer, made resident where a kernel reads through it), and the atoms it holds, below `end`. Kept
/// with the blocks until its batch's work completes. A device whose buffers have no addresses (OpenCL) has none.
pub(in crate::code_rule_dsl) struct DevAtoms {
    dir: (Buffer, u64),
    reads: Vec<Buffer>,
    end: u32,
    keep: std::sync::Arc<dyn std::any::Any + Send + Sync>,
}

impl DevAtoms {
    /// The directory of `runs` (`Atoms::runs`), for the device's addresses.
    pub(in crate::code_rule_dsl) fn new(runs: Vec<(rerail_blocks::atoms::Run, std::sync::Arc<rerail_blocks::block::Block>)>) -> Result<DevAtoms, Error> {
        let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
        let mut blocks: Vec<std::sync::Arc<rerail_blocks::block::Block>> = Vec::new();
        for (_, b) in &runs {
            if !blocks.iter().any(|k| std::sync::Arc::ptr_eq(k, b)) {
                blocks.push(b.clone());
            }
        }
        let reads: Vec<Buffer> = blocks.iter().map(|b| b.device_buffer(device)).collect::<Result<_, _>>().map_err(|_| Error::BackendCapacity)?;
        let addresses: Vec<u64> = reads.iter().map(Buffer::device_address).collect::<Option<_>>().ok_or(Error::BackendCapacity)?;
        // A host address in a block, as the device's: its buffer's device address plus its offset.
        let at = |p: *const u8| {
            let k = blocks.iter().position(|b| (b.base_ptr() as usize..b.base_ptr() as usize + b.reserved()).contains(&(p as usize))).expect("an address in an atoms block");
            addresses[k] + (p as u64 - blocks[k].base_ptr() as u64)
        };
        let only: Vec<rerail_blocks::atoms::Run> = runs.iter().map(|(r, _)| *r).collect();
        let end = only.iter().map(|r| r.end).max().unwrap_or(0);
        // The directory's own block's address, once it is made (its spans are offsets from it).
        let own = std::cell::Cell::new(true);
        let dir = rerail_blocks::atoms::AtomsDir::new(&only, &at, &|b| match b.device_buffer(device).ok().and_then(|x| x.device_address()) {
            Some(a) => a,
            None => {
                own.set(false);
                0
            }
        });
        if !own.get() {
            return Err(Error::BackendCapacity);
        }
        let dir_buf = dir.block().device_buffer(device).map_err(|_| Error::BackendCapacity)?;
        let dir_at = dir.root_at() as u64;
        Ok(DevAtoms { dir: (dir_buf, dir_at), reads, end, keep: std::sync::Arc::new((dir, blocks)) })
    }

    /// The atoms below which it holds every one.
    pub(in crate::code_rule_dsl) fn end(&self) -> u32 {
        self.end
    }

    /// The atoms blocks made resident for `pass`'s dispatches after it.
    fn resident_in(&self, pass: &Pass) {
        pass.use_read(&self.reads.iter().collect::<Vec<&Buffer>>());
    }
}

/// The store's one rolling cache on the device (decision 0478 steps 2d, 2e: the text tests' verdicts, the text
/// functions' texts): its block's device buffer and the cache's offset in it.
pub(in crate::code_rule_dsl) struct DevKept {
    cache: (Buffer, u64),
}

impl DevKept {
    /// `kept` (a store's atoms' `Kept`) for the device.
    pub(in crate::code_rule_dsl) fn new(kept: &crate::content_store::atoms::Kept) -> Result<DevKept, Error> {
        let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
        Ok(DevKept { cache: (kept.block().device_buffer(device).map_err(|_| Error::BackendCapacity)?, kept.at() as u64) })
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
    fn of(&self) -> (&Buffer, usize, u64, u64) {
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

/// An index's bucket pass's parameters (`ix_bucket`'s `IxP`): the rows, the key's columns, the relation's descriptor's
/// offset in its block, the buckets' mask, each key column, and a view read in place (its directory's address and
/// number; 0: a relation).
#[repr(C)]
struct IxParams {
    n: u32,
    ncols: u32,
    at: u64,
    mask: u64,
    cols: [u32; 8],
    dir: u64,
    kind: u64,
}

const _: () = assert!(std::mem::size_of::<IxParams>() == 72);

impl IxParams {
    fn bytes(&self) -> &[u8] {
        // SAFETY: `repr(C)` integers with no padding (its size asserted).
        unsafe { std::slice::from_raw_parts((self as *const IxParams).cast::<u8>(), std::mem::size_of::<IxParams>()) }
    }
}

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
    /// slab of its own is bound by a wrap of its used extent (`Block::device_buffer`), which a buffer taken before the
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
    fn bound(&self) -> &Buffer {
        &self.buffer.buffer
    }
}

/// The buffer of `home`'s block as it is now (reaching what it holds now), the block kept with it.
fn bound(home: &std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>) -> Result<Held, Error> {
    let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
    Ok(Held::kept(home.block.device_buffer(device)?, Box::new(home.clone())))
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
    /// The relation, held until the batch's work, which writes its index into it, is done (Q268): its last holder
    /// otherwise gives its descriptor and chunks back to the block's free list (`Held`'s drop, decision 0342) while the
    /// index is still being built, and the free list's links and the next relation made there are written over it.
    _holds: Option<crate::code_rule_dsl::facts::rows::Part>,
    room: rerail_blocks::relation::IndexRoom,
    /// The relation's descriptor as it was once the index was reserved (Q265): checked before the index is marked
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
            "a relation's descriptor overwritten while its index was built (Q265): {origin}, at {} of a block of kind {} ({} used of {}), index {} (buckets at {}, rows at {}, {} rows); bytes changed at {changed:?}\n  before {}\n  now    {}",
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
/// fused steps' counts (`counters`). The device writes them too, and holders share them (`Rc`), so no reference to
/// the buffer is ever made: each word is read or written through its address, one access at a time.
pub(in crate::code_rule_dsl) struct Words {
    buffer: Held,
    pub(in crate::code_rule_dsl) len: usize,
}

impl Words {
    pub(in crate::code_rule_dsl) fn zeroed(len: usize) -> Result<Words, Error> {
        let buffer = Held::new((4 * len).max(4))?;
        let w = Words { buffer, len };
        if len > 0 {
            // SAFETY: the buffer's `len` words, new: no command buffer uses them yet.
            unsafe { std::ptr::write_bytes(w.word(0), 0, len) };
        }
        Ok(w)
    }

    /// Word `i`'s address in the shared buffer.
    fn word(&self, i: usize) -> *mut u32 {
        assert!(i < self.len, "word {i} of {}", self.len);
        // SAFETY: within the buffer, which holds `len` words.
        unsafe { self.buffer.buffer.contents().cast::<u32>().add(i) }
    }

    pub(in crate::code_rule_dsl) fn set(&self, i: usize, v: u32) {
        // SAFETY: a word of the buffer, aligned (the buffer is page-aligned); the host writes it only while no
        // command buffer that reads or writes it runs.
        unsafe { self.word(i).write(v) }
    }

    /// Word `i`, once the command buffers that write it completed.
    pub(in crate::code_rule_dsl) fn get(&self, i: usize) -> u32 {
        // SAFETY: as `set`'s: a word of the buffer, no command buffer writing it running.
        unsafe { self.word(i).read() }
    }
}

/// A stage whose table's space is past this many rows has its grids sized on the device (`Scan::size`): below it,
/// the threads past the rows are fewer than the sizing's own dispatch costs. `RERAIL_DEVICE_SIZED_MIN` sets it
/// (0: every stage's, to test them), `RERAIL_DEVICE_SIZED=off` turns the sizing off.
fn sized_min() -> usize {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| match std::env::var("RERAIL_DEVICE_SIZED") {
        Ok(v) if v == "off" => usize::MAX,
        _ => std::env::var("RERAIL_DEVICE_SIZED_MIN").ok().and_then(|v| v.parse().ok()).unwrap_or(4096),
    })
}

/// An automaton the device reads (a test's table, a regex's replacement automata) where the automata block holds it
/// (decision 0478 step 2c): the block's device buffer (a no-copy wrap), bound at the automaton's offset. Nothing is
/// uploaded; the automata it lies in are held while it is.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct DevBytes {
    buffer: Buffer,
    at: u64,
    _of: std::sync::Arc<crate::code_rule_dsl::text::automata::Automata>,
}

impl DevBytes {
    pub(in crate::code_rule_dsl) fn of(a: &crate::code_rule_dsl::text::automata::Automaton) -> Result<DevBytes, Error> {
        let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
        let of = crate::code_rule_dsl::text::automata::automata();
        let buffer = of.block().device_buffer(device).map_err(|_| Error::BackendCapacity)?;
        Ok(DevBytes { buffer, at: a.at as u64, _of: of })
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

/// Work encoded into one submission at a time, committed when the launch budget of one is near and at `finish`;
/// every submission runs on the process's queue, in order.
pub(in crate::code_rule_dsl) struct Batch {
    pass: Option<Pass>,
    committed: Vec<Submitted>,
    done: Done,
    dummy: Held,
    /// Buffers the encoded work reads that nothing else holds: kept until it completes (a pooled buffer is written
    /// by the host when handed out again, and no host write is ordered after a device read).
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
    /// The atoms directory its text kernels read symbols' bytes through (decision 0478 step 2e), made the first
    /// time a stage reads a symbol's text and made again when the store's atoms have grown past it.
    atoms: Option<std::rc::Rc<DevAtoms>>,
    /// The store's one rolling cache, for the verdicts and texts its text kernels keep (decision 0478 step 2e).
    kept: Option<std::rc::Rc<DevKept>>,
    /// The relations its work writes in their blocks, each with its rows' `needed` before (decision 0463): a row
    /// its kernels could not write, past the chunks its relation holds, fails the batch, named.
    written: Vec<Written>,
}

/// A relation a batch's kernels write (`Batch::written`).
struct Written {
    home: std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>,
    /// The relation, held until the batch is done, as `Building`'s (Q268): its descriptor is read then, and the
    /// kernels write its rows meanwhile.
    _holds: Option<crate::code_rule_dsl::facts::rows::Part>,
    at: u64,
    rows: usize,
    needed: u32,
    what: &'static str,
}

impl Batch {
    /// The atoms directory its text kernels read through: the one it has, unless `end` (the store's atoms now) is
    /// past it, then `make`'s, the one before kept until the work that reads it completes.
    pub(in crate::code_rule_dsl) fn atoms(&mut self, end: u32, make: impl FnOnce() -> Result<DevAtoms, Error>) -> Result<(), Error> {
        if self.atoms.as_ref().is_some_and(|a| a.end() >= end) {
            return Ok(());
        }
        if let Some(old) = self.atoms.take() {
            self.held.push(Box::new(old));
        }
        self.atoms = Some(std::rc::Rc::new(make()?));
        Ok(())
    }

    /// The store's one rolling cache its text kernels keep verdicts and texts in, made the first time one asks.
    pub(in crate::code_rule_dsl) fn kept(&mut self, make: impl FnOnce() -> Result<DevKept, Error>) -> Result<(), Error> {
        if self.kept.is_none() {
            self.kept = Some(std::rc::Rc::new(make()?));
        }
        Ok(())
    }

    /// Keeps `rel` (a handle a stratum made for its work) until this batch's work completes.
    pub(in crate::code_rule_dsl) fn keeps(&mut self, rel: std::rc::Rc<DevRel>) {
        self.read.push(rel);
    }

    pub(in crate::code_rule_dsl) fn new() -> Result<Batch, Error> {
        rerail_accel::gpu::join::device()?;
        Batch::with_arena(ARENA_START)
    }

    /// A batch whose text arena holds `cap` bytes.
    pub(in crate::code_rule_dsl) fn with_arena(cap: usize) -> Result<Batch, Error> {
        rerail_accel::gpu::join::device()?;
        Ok(Batch { pass: None, committed: Vec::new(), done: Done::default(), dummy: Held::new(64)?, keep: Vec::new(), read: Vec::new(), held: Vec::new(), arena: None, arena_cap: cap, building: Vec::new(), atoms: None, kept: None, written: Vec::new() })
    }

    /// Notes that its kernels write `rel` (`what`): its rows' `needed` now, checked when the batch is done.
    fn writes(&mut self, rel: &DevRel, what: &'static str) {
        let Some(home) = &rel.home else { return };
        let needed = rel.relation().rows.needed.load(std::sync::atomic::Ordering::Relaxed);
        self.written.push(Written { home: home.clone(), _holds: rel.holder(), at: rel.at, rows: rel.rows, needed, what });
    }

    /// The relations its kernels wrote whose rows counted one they could not write (decision 0463): each named.
    fn overran(&self) -> Vec<String> {
        self.written
            .iter()
            .filter_map(|w| {
                // SAFETY: `at` is a relation's descriptor in `home`'s block, which `home` keeps.
                let r = unsafe { &*w.home.block.ptr::<rerail_blocks::relation::Relation>(w.at as usize) };
                let now = r.rows.needed.load(std::sync::atomic::Ordering::Relaxed);
                (now != w.needed).then(|| {
                    let held = r.rows.reserved.load(std::sync::atomic::Ordering::Relaxed);
                    format!("{}: {} rows past its chunks (Q265), a relation of {} rows its device side says, {} its chunks hold, {} claimed, {} columns, at {} of a block of kind {}",
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

    /// The open submission, with room for `need` more dispatches (a new one when not).
    fn pass(&mut self, need: usize) -> Result<&Pass, Error> {
        if self.pass.as_ref().is_some_and(|p| p.dispatches() + need > BUDGET) {
            self.submit_open()?;
        }
        if self.pass.is_none() {
            super::metal_sort::ready()?;
            self.pass = Some(Pass::new()?);
            rerail_accel::exec::command_opened();
        }
        Ok(self.pass.as_ref().expect("an open pass"))
    }

    /// `bytes` in a buffer of their own, kept until the batch's work completes: a parameter block a kernel reads (a
    /// stage's, a copy's), past what a launch's own parameters hold on every backend.
    fn block(&mut self, bytes: &[u8]) -> Result<Buffer, Error> {
        let held = Held::of(bytes.iter().copied()).map_err(|_| Error::BackendCapacity)?;
        let buffer = held.buffer.clone();
        self.keep.push(held);
        Ok(buffer)
    }

    /// Commits the open submission (`Pass::commit`, checked against the launch budget).
    fn submit_open(&mut self) -> Result<(), Error> {
        let Some(pass) = self.pass.take() else { return Ok(()) };
        self.done.dispatches += pass.dispatches() as u32;
        rerail_accel::exec::command_committed();
        let s = pass.commit()?;
        self.committed.push(s);
        self.done.submissions += 1;
        Ok(())
    }

    /// Commits what is open without waiting: the work encoded so far runs before any submission committed after it on
    /// the one queue (a batch that reads what this one's work writes).
    pub(in crate::code_rule_dsl) fn commit_open(&mut self) -> Result<(), Error> {
        self.submit_open()
    }

    /// Commits what is open and waits for every submission. A submission the device failed fails the batch: its
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
        // A continuation's index is of every row it reads (its base's first: `ix_bucket` reads by id, `rel_get`).
        if r.base_rows != 0 && !r.continues() {
            // Q265: what was read, and where, so a writer that missed its own buffer can be named.
            let h = home.block.header();
            let origin = if rel.view.is_some() { "a view read in place" } else if rel.part.is_some() { "the device's own" } else { "a relation in its block" };
            panic!(
                "an index of a full relation (Q265): {origin} of {} rows, {} columns, at {} of a block of kind {} (magic {:#x}, {} used of {}); its header: arity {}, base rows {} ({:#x}), {} indexes, {} order keys; when made: arity {}, base rows {}\n  descriptor {}",
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
            // Either is kept by the batch until its work, which builds the index into it, completes (Q265: a fresh
            // relation dropped with its stage's handle gave back its index while the GPU built it).
            if let Some(view) = &rel.view {
                let fresh = std::rc::Rc::new(DevRel::view(rel.cols, rel.rows, view.clone())?);
                self.read.push(fresh.clone());
                let ix = self.index(&fresh, cols)?;
                return Ok(DevIndex { moved: Some(fresh), ..ix });
            }
// copy (0353 step 6): a second key's index copies the relation. It goes: a relation is made with room for
// the indexes its readers probe (decision 0353).
            const MOVED: rerail_prof::copies::Site = rerail_prof::copies::Site::new(9, "6", "a relation copied for a second key's index (DevIndex::moved)");
            MOVED.copied(8 * rel.cols * rel.rows);
            let copy = std::rc::Rc::new(DevRel::empty(rel.cols, rel.rows)?);
            self.read.push(copy.clone());
            let ident: Vec<Col> = (0..rel.cols).map(Col::From).collect();
            self.copy_rows(Rows::Rel(rel), rel.rows, &ident, Rows::Rel(&copy), 0)?;
            let ix = self.index(&copy, cols)?;
            return Ok(DevIndex { moved: Some(copy), ..ix });
        };
        let n = rel.rows;
        let buffer = bound(&home)?;
        assert!(room.rows as u64 + 4 * n as u64 <= buffer.buffer.length(), "the index's room within the buffer bound");
        if n > 0 {
            let bucket = pipeline("join", "ix_bucket", || SOURCE.to_owned())?;
            let place = pipeline("join", "ix_place", || SOURCE.to_owned())?;
            let bits = (u64::BITS - room.mask.leading_zeros()).max(8).div_ceil(8) * 8;
            assert!(cols.len() <= 8, "an index of {} columns", cols.len());
            let mut ix = IxParams { n: n as u32, ncols: cols.len() as u32, at: rel.at, mask: room.mask, cols: [0; 8], dir: 0, kind: 0 };
            for (k, &c) in cols.iter().enumerate() {
                ix.cols[k] = c as u32;
            }
            if let Some(v) = &rel.view {
                (ix.dir, ix.kind) = (v.dir, u64::from(v.kind));
            }
            let params = self.block(ix.bytes())?;
            // A sort is at most 4 passes of three dispatches and two scans' levels, and a few more.
            let pass = self.pass(48)?;
            let words = Held::new(4 * n)?;
            // A sort refused after the bucket's dispatch leaves `words`, which the dispatch writes, with the batch
            // until its submission is done: dropped here, it would go back to the pool at once.
            rel.resident_in(pass);
            pass.run_bytes(&bucket, &[&buffer.buffer, &words.buffer, &params], &[], groups(n));
            let parts = match encode_sort(pass, &words.buffer, Width::W32, bits, n) {
                Ok(parts) => parts,
                Err(e) => {
                    self.keep.push(words);
                    return Err(e.into());
                }
            };
            pass.run(&place, &[&buffer.buffer, &parts.perm.buffer, &parts.starts.buffer, &parts.rkeys.buffer, &parts.nruns.buffer], &[n as u32, (room.mask + 1) as u32],
                     &[room.offs as i64, room.rows as i64], groups(n));
            self.keep.extend([words, parts.perm, parts.starts, parts.rkeys, parts.nruns]);
        }
        let index = room.index as u32;
        let before = Building::descriptor(std::ptr::from_ref(r));
        self.building.push(Building { home, rel: std::ptr::from_ref(r), _holds: rel.holder(), room, before });
        Ok(DevIndex { index, rows: n, moved: None, built: true, by_parent: false, buffer })
    }

    /// The columns `cols` (its source's) of a view made from a chunk block on the device (decision 0289 §2), into
    /// `out`, a relation of `out.rows` rows whose columns are `cols` in order: what `Viewed::row_at` made on the host.
// copy (0353 step 4): a view expanded into a relation (660.8 MB a run on hogql). It goes: a view is a
// read of the tape, and a stage reads it in place (decision 0353).
    pub(in crate::code_rule_dsl) fn expand_view(&mut self, block: &Buffer, root: usize, view: u32, cols: &[usize], out: &DevRel) -> Result<(), Error> {
        let expand = pipeline("chunk_views", "view_expand", chunk_views_source)?;
        const EXPANDED: rerail_prof::copies::Site = rerail_prof::copies::Site::new(4, "4", "a view expanded into a device relation (expand_view)");
        EXPANDED.copied(8 * out.rows * cols.len());
        self.writes(out, "a view expanded");
        let at = |o: usize| u32::try_from(o).map_err(|_| Error::BackendCapacity);
        let rows_at = out.rows_at();
        let mut p = vec![at(root)?, view, out.rows as u32, cols.len() as u32, 0, rows_at as u32, (rows_at >> 32) as u32];
        p.extend(cols.iter().map(|&c| c as u32));
        if out.rows > 0 {
            let params = self.block(&p.iter().flat_map(|w| w.to_ne_bytes()).collect::<Vec<u8>>())?;
            let pass = self.pass(1)?;
            pass.run_bytes(&expand, &[block, &out.buffer.buffer, &params], &[], groups(out.rows));
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
        let rbuf: Buffer = match right {
            Some((rel, index)) => {
                let rel = index.map_or(rel, |ix| ix.of(rel));
                bind_right(&mut stage, rel, index);
                index.map_or(&rel.buffer.buffer, |ix| ix.bound()).clone()
            }
            None => self.dummy.buffer.clone(),
        };
        if left.cap == 1 && stage.mode == JOIN && right.is_some() {
            // From one binding: a thread per candidate row (`scan_count`), its rows in the relation's order.
            let scount = pipeline("join", "scan_count", || SOURCE.to_owned())?;
            let sscatter = pipeline("join", "scan_scatter", || SOURCE.to_owned())?;
            let n = stage.nr as usize + 1;
            let counts = scratch(FREE, 4 * (n + 1))?;
            let scan = Scan::new(n)?;
            let sb = self.block(stage.bytes())?;
            let pass = self.pass(12)?;
            if let Some((rel, index)) = right {
                index.map_or(rel, |ix| ix.of(rel)).resident_in(pass);
            }
            let lbuf: &Buffer = &left.buffer.buffer;
            pass.run_bytes(&scount, &[lbuf, &rbuf, &counters.buffer.buffer, &counts, &totals.buffer.buffer, &sb], &[], groups(n));
            scan.encode(pass, &counts)?;
            pass.run_bytes(&sscatter, &[lbuf, &rbuf, &totals.buffer.buffer, &counts, &out.buffer.buffer, &sb], &[], groups(n));
            return Ok(());
        }
        let n = left.cap + 1;
        let counts = scratch(FREE, 4 * (n + 1))?;
        let scan = Scan::new(n)?;
        let arena: Buffer = if matches!(stage.mode, STRCMP | TEXTOP) && (stage.agg[1] & 15 == OPND_ARENA || stage.agg[1] >> 4 == OPND_ARENA) {
            self.arena()?.buffer.buffer.clone()
        } else {
            self.dummy.buffer.clone()
        };
        let sb = self.block(stage.bytes())?;
        // A text op's or `str_cmp`'s symbols read through the batch's atoms directory (decision 0478 step 2e).
        let reads_names = matches!(stage.mode, STRCMP | TEXTOP);
        let atoms = self.atoms.clone().filter(|_| reads_names);
        if reads_names && atoms.is_none() {
            return Err(Error::BackendCapacity);
        }
        let dummy = self.dummy.buffer.clone();
        let pass = self.pass(13)?;
        if let Some((rel, index)) = right {
            index.map_or(rel, |ix| ix.of(rel)).resident_in(pass);
        }
        let lbuf: &Buffer = &left.buffer.buffer;
        let (d, d_at): (&Buffer, u64) = match &atoms {
            Some(a) => {
                a.resident_in(pass);
                (&a.dir.0, a.dir.1)
            }
            None => (&dummy, 0),
        };
        let count_bufs: [(&Buffer, u64); 8] = [(lbuf, 0), (&rbuf, 0), (&totals.buffer.buffer, 0), (&counters.buffer.buffer, 0), (&counts, 0), (&arena, 0), (d, d_at), (&sb, 0)];
        let scatter_bufs: [(&Buffer, u64); 9] =
            [(lbuf, 0), (&rbuf, 0), (&totals.buffer.buffer, 0), (&counts, 0), (&out.buffer.buffer, 0), (&arena, 0), (&counters.buffer.buffer, 0), (d, d_at), (&sb, 0)];
        if n > sized_min() {
            // The table's space is the host's guess: the grids are sized on the device, from the rows the stage
            // before made (decision 0113's GPU-sized work); at most the table's.
            let sizes = scan.size(pass, &totals.buffer.buffer, in_slot, left.cap)?;
            pass.run_bytes_indirect_at(&count, &count_bufs, &[], &sizes.buffer, 4 * 32, groups(n));
            scan.encode_sized(pass, &counts, &sizes)?;
            pass.run_bytes_indirect_at(&scatter, &scatter_bufs, &[], &sizes.buffer, 4 * 32, groups(n));
            self.done.sized += 1;
            self.keep.push(sizes);
            return Ok(());
        }
        pass.run_bytes_at(&count, &count_bufs, &[], groups(n));
        scan.encode(pass, &counts)?;
        pass.run_bytes_at(&scatter, &scatter_bufs, &[], groups(n));
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
        let dummy: Buffer = self.dummy.buffer.clone();
        let rel = right.1.map_or(right.0, |ix| ix.of(right.0));
        bind_right(&mut stage, rel, right.1);
        let a = right.1.map_or(&rel.buffer.buffer, |ix| ix.bound()).clone();
        let mut probes = [Stage::default(); WITNESS_PROBES];
        let mut bufs: Vec<Buffer> = Vec::new();
        for (k, (b, rel, ix)) in rest.iter().enumerate() {
            let mut b = *b;
            let rel = ix.map_or(*rel, |ix| ix.of(rel));
            bind_right(&mut b, rel, *ix);
            b.cap_in = left.cap as u32;
            probes[k] = b;
            bufs.push(ix.map_or(&rel.buffer.buffer, |ix| ix.bound()).clone());
        }
        while bufs.len() < WITNESS_PROBES {
            bufs.push(dummy.clone());
        }
        let held = Held::of(probes.iter().flat_map(|p| p.bytes().iter().copied()).collect::<Vec<u8>>().into_iter()).map_err(|_| Error::BackendCapacity)?;
        let n = left.cap + 1;
        let counts = scratch(FREE, 4 * (n + 1))?;
        let scan = Scan::new(n)?;
        // The bindings with a witness out as they are: a semi-join's scatter (mode 1 writes the binding alone).
        let mut write = stage;
        write.mode = SEMI;
        write.nbinds = 0;
        let (sb, wb) = (self.block(stage.bytes())?, self.block(write.bytes())?);
        let pass = self.pass(3)?;
        rel.resident_in(pass);
        for &(_, r, ix) in rest {
            ix.map_or(r, |ix| ix.of(r)).resident_in(pass);
        }
        let lbuf: &Buffer = &left.buffer.buffer;
        let mut count_bufs: Vec<&Buffer> = vec![lbuf, &a, &totals.buffer.buffer, &counts, &held.buffer];
        count_bufs.extend(bufs.iter());
        count_bufs.push(&sb);
        pass.run_bytes(&count, &count_bufs, &[], groups(n));
        scan.encode(pass, &counts)?;
        let scatter_bufs: [&Buffer; 9] = [lbuf, &a, &totals.buffer.buffer, &counts, &out.buffer.buffer, &dummy, &dummy, &dummy, &wb];
        pass.run_bytes(&scatter, &scatter_bufs, &[], groups(n));
        self.keep.push(held);
        Ok(())
    }

    /// A mode 10 stage (`ROWTEXT`) from `left` into `out`: `row_text` (a SIMD group a row, its symbols' bytes read
    /// through the batch's atoms directory, decision 0478 step 2e), its automaton `auto` (a replacement's or a test's,
    /// in the automata block), and `id`: a test's (its verdicts) or the functions' (their texts, `ROW_KEPT`) in the
    /// batch's rolling cache; then its rows scattered.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn row_stage(&mut self, mut stage: Stage, left: &Table, auto: Option<(&DevBytes, [u32; 4])>, id: u32,
                                              in_slot: usize, out_slot: usize, totals: &Words, counters: &Words, counter: Option<usize>, out: &Table) -> Result<(), Error> {
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
        // [scratch states, states, classes, start]: a replacement's scratch holds its larger automaton's states, two
        // rows a lane.
        let [kept, states, classes, start] = auto.map_or([1, 0, 0, 0], |a| a.1);
        let work = Held::new(ngroups * 2 * simd_group() * kept.max(1) as usize)?;
        let p = Held::of([ngroups as u32, kept, states, classes, start, id, 0, 0].into_iter()).map_err(|_| Error::BackendCapacity)?;
        let counts = scratch(FREE, 4 * (n + 1))?;
        let values = Held::new(8 * n)?;
        let scan = Scan::new(n)?;
        let arena = self.arena()?.buffer.buffer.clone();
        let (g, g_at): (Buffer, u64) = auto.map_or_else(|| (self.dummy.buffer.clone(), 0), |a| (a.0.buffer.clone(), a.0.at));
        let atoms = self.atoms.clone().ok_or(Error::BackendCapacity)?;
        let cache = self.kept.clone().ok_or(Error::BackendCapacity)?;
        let dummy = self.dummy.buffer.clone();
        let sb = self.block(stage.bytes())?;
        let pass = self.pass(4)?;
        let lbuf: &Buffer = &left.buffer.buffer;
        let (vc, vc_at) = (&cache.cache.0, cache.cache.1);
        atoms.resident_in(pass);
        // A SIMD group a row, a threadgroup each.
        pass.run_bytes_simd_at(
            &text,
            &[(lbuf, 0), (&dummy, 0), (&arena, 0), (&totals.buffer.buffer, 0), (&counters.buffer.buffer, 0), (&counts, 0), (&values.buffer, 0), (&g, g_at), (&work.buffer, 0), (&p.buffer, 0), (&atoms.dir.0, atoms.dir.1), (vc, vc_at), (&sb, 0)],
            &[],
            ngroups,
        );
        scan.encode(pass, &counts)?;
        pass.run_bytes(&scatter, &[lbuf, &values.buffer, &totals.buffer.buffer, &counts, &out.buffer.buffer, &sb], &[], groups(n));
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
        let counts = scratch(FREE, 4 * (n + 1))?;
        let scan = Scan::new(n)?;
        let db = self.block(bytes)?;
        let pass = self.pass(12)?;
        let t: &Buffer = &table.buffer.buffer;
        pass.run_bytes(&clear, &[&p.buffer, &db], &[], groups(places));
        pass.run_bytes(&claim, &[t, &totals.buffer.buffer, &p.buffer, &db], &[], groups(table.cap));
        pass.run_bytes(&mark, &[t, &totals.buffer.buffer, &p.buffer, &counts, &db], &[], groups(n));
        scan.encode(pass, &counts)?;
        pass.run_bytes(&scatter, &[t, &totals.buffer.buffer, &counts, &out.buffer.buffer, &db], &[], groups(n));
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
        let (s, d) = (s.clone(), d.clone());
        let totals = count.map_or_else(|| self.dummy.buffer.clone(), |c| c.1.buffer.buffer.clone());
        let cb = self.block(bytes)?;
        let pass = self.pass(1)?;
        if let Some(r) = view {
            r.resident_in(pass);
        }
        pass.run_bytes(&copy, &[&s, &d, &totals, &cb], &[], groups(n));
        Ok(())
    }

    /// Sets slots of `table` in place (`stage`'s binds: program columns of the binding's slots, or parameters).
    pub(in crate::code_rule_dsl) fn map(&mut self, mut stage: Stage, table: &Table, in_slot: usize, totals: &Words) -> Result<(), Error> {
        let map = pipeline("join", "stage_map", || SOURCE.to_owned())?;
        stage.cap_in = table.cap as u32;
        stage.cap_out = table.cap as u32;
        stage.in_slot = in_slot as u32;
        stage.nslots = table.slots as u32;
        let sb = self.block(stage.bytes())?;
        let pass = self.pass(1)?;
        pass.run_bytes(&map, &[&table.buffer.buffer, &totals.buffer.buffer, &sb], &[], groups(table.cap));
        Ok(())
    }
}

impl Drop for Batch {
    /// Work still in flight holds its buffers: wait for it before they return to the pool. Work encoded and never
    /// committed is dropped uncommitted (nothing reads it, and nothing ran it): committing it here ran the launch
    /// guard, whose thread-local a batch dropped in a thread's teardown (a wave left open) can no longer read.
    fn drop(&mut self) {
        if self.pass.take().is_some() {
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

    /// Q265: a relation `Batch::index` moves for an index (its directory full, or its block) is the batch's until its
    /// work, which builds the index into it, completes: a stage's handle to it dropped meanwhile does not give back
    /// its relation, and the index the batch marks built is still its own.
    #[test]
    fn a_relation_moved_for_an_index_lives_until_its_batch_is_done() {
        if rerail_accel::gpu::join::device().is_err() {
            return;
        }
        let n = 1000;
        let cols: Vec<Vec<i64>> = (0..4).map(|c| rows(n, 11 + c, 50)).collect();
        let rel = DevRel::upload(4, n, |v| for (k, c) in cols.iter().enumerate() { v[k * n..(k + 1) * n].copy_from_slice(c) }).unwrap();
        let mut batch = Batch::new().unwrap();
        let keys: [&[usize]; 9] = [&[0], &[1], &[2], &[3], &[0, 1], &[0, 2], &[0, 3], &[1, 2], &[1, 3]];
        let mut moved = None;
        for key in keys {
            let ix = batch.index(&rel, key).unwrap();
            if let Some(m) = ix.moved.clone() {
                moved = Some(m);
                break;
            }
        }
        let moved = moved.expect("a key past the relation's directory moves it");
        assert!(std::rc::Rc::strong_count(&moved) >= 2, "the batch keeps the relation it moved");
        drop(moved);
        batch.finish().expect("its index built into the relation it moved, still held");
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

    /// Decision 0384 steps 7 and 8: a fork (a second relation started from frozen rows whose writer another took), and
    /// a fork of it, is bound as one relation, nothing merged: the kernels read its bases' rows down its chain
    /// (`rel_get`), and the device's index of it is the host's, over every id.
    #[test]
    fn the_device_reads_a_fork_through_its_base() {
        use crate::code_rule_dsl::facts::rows::Rows as FactRows;
        if rerail_accel::gpu::join::device().is_err() {
            return;
        }
        let n = 5_000;
        let (c0, c1) = (rows(n, 31, 97), rows(n, 32, 13));
        let vals = |lo: usize, hi: usize| -> Vec<i64> { (lo..hi).flat_map(|i| [c0[i], c1[i]]).collect() };
        let mut start = FactRows::new(2);
        // copy: not block data: a test's rows
        start.extend_from_slice(&vals(0, 3_000));
        let kept = start.freeze(3_000);
        let mut first = kept.shared(3_000);
        let mut fork = kept.shared(3_000);
        // copy: not block data: a test's rows
        first.extend_from_slice(&vals(3_000, 3_100));
        // copy: not block data: a test's rows
        fork.extend_from_slice(&vals(3_000, 4_000));
        // A fork of the fork (step 8): frozen, its writer taken, forked again.
        let kept = fork.freeze(4_000);
        let mut again = kept.shared(4_000);
        let mut fork = kept.shared(4_000);
        // copy: not block data: a test's rows
        again.extend_from_slice(&vals(0, 10));
        // copy: not block data: a test's rows
        fork.extend_from_slice(&vals(4_000, n));
        let b = fork.in_block().expect("bound");
        assert!(b.relation().continues() && b.relation().base().is_some_and(|r| r.continues()), "a continuation of a continuation");
        assert!(b.rows == n as u32, "a continuation bound whole");
        let dev = DevRel::in_block(&b, 2).unwrap();
        let mut host = FactRows::new(2);
        // copy: not block data: a test's rows
        host.extend_from_slice(&vals(0, n));
        let host = DevRel::in_block(&host.in_block().unwrap(), 2).unwrap();
        for cols in [vec![0usize], vec![1, 0]] {
            let mut batch = Batch::new().unwrap();
            let ix = batch.index(&dev, &cols).unwrap();
            assert!(ix.built && ix.moved.is_none(), "built in the fork's own block");
            batch.finish().unwrap();
            let key: Vec<u16> = cols.iter().map(|&c| c as u16).collect();
            let (r, h) = (dev.relation(), host.relation());
            let hosts = {
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
            assert_eq!(d.offs.as_slice(), w.offs.as_slice(), "{cols:?}: the buckets");
            assert_eq!(d.rows.as_slice(), w.rows.as_slice(), "{cols:?}: each bucket's rows, in id order");
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
        assert!(nkeys < MAX_KEYS && u.cap == n && delta.cap == n);
        assert!(map.places.is_power_of_two() && 2 * n <= map.places, "a key map of {} slots for {n} rows", map.places);
        let pplaces = (2 * n).next_power_of_two().max(16);
        let mut p = LatParams { top: lat.top, n: n as u32, had: had as u32, nkeys: nkeys as u32, code: lat.code, after: lat.after.unwrap_or(NONE), pmask: (pplaces - 1) as u32,
                                hmask: (map.places - 1) as u32, level: level.unwrap_or(0), omask: 0, pad: 0, r_at: r.at, r_rows_at: r.rows_at(), h_at: map.at as u64, h0_at: 0 };
        // Its rows written through the bounded writer, checked after the work (decision 0463).
        self.writes(r, "a lattice's held rows");
        // The map's block as it is now (reaching its slots), kept until the work completes.
        let hb = bound(map.home)?;
        // Its rows written in place, bounded by their chunks (decision 0463): checked when the work completes.
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
        let a: Buffer = arena.map_or_else(|| self.dummy.buffer.clone(), |a| a.buffer.buffer.clone());
        let pass = self.pass(4)?;
        let ub: &Buffer = &u.buffer.buffer;
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
