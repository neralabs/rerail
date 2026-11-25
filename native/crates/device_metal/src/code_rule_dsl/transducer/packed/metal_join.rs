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
//! **One submission, a launch a level.** A `Batch` encodes every stage of every rule it is given into one submission
//! of the device layer's queue (serial: each dispatch sees the last one's writes). Its stages are queued and launched
//! a level at a time (`Batch::flush`, decision 0601): every rule's stage at one depth in one launch of each kernel,
//! each job's buffers reached by their device addresses, one scan of all their counts; so a stratum's launches are
//! its levels', not its rules'. Every launch counts in the evaluation's launch budget. The kernels are UKL (`join_stages.uglykl`, `chunk_views.uglykl`; decision 0440 step C),
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
use rerail_accel::gpu::join::{FREE, Held, Pass, Scan, Submitted, Width, groups, pipeline, scratch, simd_group};
use rerail_device::device::Buffer;
use std::sync::Arc;


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
/// `agg[0]`. A value with none (`lower` past ASCII) sets the stage's counter word (`TEXTFN_HOST`); the arena full, the
/// program's flag (its phase parks while the host maps the arena further, `Arena::grow`).
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

/// Field `f` (a path: `at.base`) of record type `T` as a `WordAt`: a compile error unless it is a `u32`.
macro_rules! word_at {
    ($t:ty, $($f:ident).+) => {{
        const _: () = {
            #[allow(dead_code)]
            fn a_word(r: &$t) -> &u32 {
                &r.$($f).+
            }
        };
        WordAt(std::mem::offset_of!($t, $($f).+) as u64)
    }};
}

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
    /// The probed relation's screen (decision 0488): its key `screen - 1`'s column's kept filter, a
    /// `rerail_blocks::bloom::Bloom` at byte `screen_at` of the screen's buffer (the filter's block, bound where it
    /// lies); 0 for none. A binding whose key the filter refuses has no candidates, and reads no index group.
    pub(in crate::code_rule_dsl) screen: u32,
    pub(in crate::code_rule_dsl) screen_at: u32,
    /// A `has` axis probed off the tape (decision 0353 step 5, `DevRel::has`): its set's kind atoms, ascending (their
    /// device address), how it is probed (`DevIndex::walk`: 1 by its node, a climb; 2 by its ancestor, the subtree;
    /// 0: not), and the atoms' count.
    pub(in crate::code_rule_dsl) v_set: u64,
    pub(in crate::code_rule_dsl) v_has: u32,
    pub(in crate::code_rule_dsl) v_nset: u32,
    /// The probed relation's first row (`DevRel::from`): a stage reads its rows from `r_from`, by their ids in it (a
    /// fixpoint's delta, its head's rows past those it held: 0353 step 6); 0 for every row.
    pub(in crate::code_rule_dsl) r_from: u32,
    /// 1: R's rows are its length as the device claimed it past `r_from`, at most `nr` (`DevRel::live`, decision 0603
    /// step 3).
    pub(in crate::code_rule_dsl) r_len: u32,
}

const _: () = assert!(std::mem::size_of::<Stage>() == 2352 && std::mem::size_of::<Stage>().is_multiple_of(16));

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
        "{}{}{}{}{}",
        rerail_accel::gpu::join::COMMON,
        rerail_blocks::RELATION_READS,
        crate::code_rule_dsl::facts::vtab::syntax::device_chunk::ukl(),
        // The lines' reads (decision 0612), which place a span on the facts' line index.
        "#include <ukl/reads/lines.h>\n",
        include_str!("chunk_views.uglykl")
    )
}

/// The joins' worker (`join_run`, the program's: decision 0603) and the chunk views' kernels
/// (`chunk_views_source`), to compile ahead of their first use (`MatcherDevice::kernels_ahead`): each asked for as its
/// use asks for it (`pipeline`), so its use finds it.
pub(in crate::code_rule_dsl) fn kernels_ahead() -> Vec<super::device::KernelAhead> {
    let made = |variant: &'static str, name: &'static str, source: fn() -> String| -> super::device::KernelAhead {
        let make = move || {
            let mut made = Ok(());
            rerail_accel::gpu::released(|| rerail_accel::gpu::caught(|| made = pipeline(variant, name, source).map(drop)))?;
            made
        };
        (format!("{variant} {name}"), Box::new(make))
    };
    // The programs' worker for the run's views' format.
    let mut v: Vec<super::device::KernelAhead> = vec![{
        let make = move || {
            let mut made = Ok(());
            rerail_accel::gpu::released(|| rerail_accel::gpu::caught(|| made = program_pipe(run_view_format()).map(drop)))?;
            made
        };
        ("join join_run".to_owned(), Box::new(make))
    }];
    v.extend(["axis_count", "axis_write", "view_expand"].map(|name| made("chunk_views", name, chunk_views_source)));
    v
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

/// The join kernels' source for views of tape format `format` (0: of several, each view read tells its own).
fn view_source(format: u32) -> String {
    if format == 0 {
        return SOURCE.to_owned();
    }
    let at = std::mem::offset_of!(rerail_clif::view::TapeView, format);
    format!("#define clif_view(v,a) (((a)=={at}u)?{format}u:(v)[(a)/4u])\n{}", *SOURCE)
}

/// The format a run's views are of, as it stores its tapes (`Format::of_run`): the view kernels compiled ahead.
pub(in crate::code_rule_dsl) fn run_view_format() -> u32 {
    match crate::code_rule_dsl::tape::formats::Format::of_run() {
        crate::code_rule_dsl::tape::formats::Format::F16 => 16,
        crate::code_rule_dsl::tape::formats::Format::F16Z => rerail_clif::view::F16Z_FORMAT,
    }
}

/// A program's worker (`join_run`, decision 0603) for views of `format` (0: several): the join kernels' source with
/// the program's part, which reads its tables and blocks coherent across the device (other groups of its launch
/// write them). Its stages read views, each compiled for the tape format of the views it reads (decision 0576): on an
/// Arc 140V `stage_count` compiled in 36 s over every format, 16 s over format 16's and 7 s over format 15's.
fn program_pipe(format: u32) -> Result<Arc<rerail_accel::gpu::join::Pipe>, String> {
    pipeline(&format!("joinprog{format}"), "join_run", || format!("#define JOIN_PROGRAM 1\n#define CLIF_BLOCKS_COHERENT 1\n{}", view_source(format)))
}

/// The tape format of the views `rels` read (`ViewInPlace::format`), one for them all: a relation reads none, and with
/// no view the run's format is taken, whose kernels are made already.
fn views_format<'a>(rels: impl IntoIterator<Item = &'a DevRel>) -> u32 {
    let mut found: Option<u32> = None;
    for v in rels.into_iter().filter_map(|r| r.view.as_ref()) {
        found = Some(match found {
            None => v.format,
            Some(f) if f == v.format => f,
            Some(_) => 0,
        });
    }
    found.unwrap_or_else(run_view_format)
}

/// The relation a probe reads: its index's (`DevIndex::of`), else itself.
fn probed<'r>(rel: &'r DevRel, index: Option<&'r DevIndex>) -> &'r DevRel {
    index.map_or(rel, |ix| ix.of(rel))
}

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
    // An evaluation of the launch budget, or the caller's when it is in one: every chunk's count and write.
    let _evaluation = super::metal_backend::launch::LaunchScope::enter();
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
        let pass = batch.pass().map_err(|e| format!("{e:?}"))?;
        pass.run(&count, &[&whole, &starts], &p, &[], groups(n + 1));
        scan.encode(pass, &starts)?;
        batch.finish().map_err(|e| format!("{e:?}"))?;
        // SAFETY: the scan completed: its last value is the total.
        let len = unsafe { *b.ptr::<u32>(start_at + 4 * n) } as usize;
        each.push((p, start_at, len));
    }
    let total: usize = each.iter().map(|e| e.2).sum();
    let first_start = each.first().map_or(0, |e| e.1);
    // Each document's first row where it has rows (a chunk's rows past the ones' before): the runs of its ids' keys
    // (decision 0494 step 7), read off the scanned starts where they lie.
    let runs_of = |put: &mut dyn FnMut(u32, u32)| {
        let mut past = 0u32;
        for (chunk, &(_, start_at, len)) in chunks.iter().zip(&each) {
            let b = &chunk.block;
            // SAFETY: the chunk's root and documents, which `device_chunk::build` wrote; its starts, scanned above.
            unsafe {
                let root = &*b.ptr::<ChunkRoot>(chunk.root);
                let n = root.nodes as usize;
                let starts = std::slice::from_raw_parts(b.ptr::<u32>(start_at), n + 1);
                let docs = root.docs.as_slice();
                for (i, d) in docs.iter().enumerate() {
                    let end = docs.get(i + 1).map_or(n, |e| e.node_row as usize);
                    let (from, to) = (starts[d.node_row as usize], starts[end]);
                    if to > from {
                        put(past + from, d.key);
                    }
                }
            }
            past += len as u32;
        }
    };
    let mut runs = 0;
    runs_of(&mut |_, _| runs += 1);
    // Alone in its block, with room for an index by each key strata probe an axis by (`AXIS_KEYS`). Each column
    // its ids' low bits, under the documents' keys.
    let (rows, keys) = Part::over_key_runs(crate::code_rule_dsl::facts::rows::Life::Run, 2, total, crate::code_rule_dsl::facts::rows::AXIS_KEYS, runs);
    let mut k = 0;
    runs_of(&mut |first, key| {
        keys.put(k, first, key);
        k += 1;
    });
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
            let pass = batch.pass().map_err(|e| format!("{e:?}"))?;
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
    /// Its first row: it is its relation's rows `from..from + rows`, read by their ids there (`Stage::r_from`): a
    /// fixpoint's delta, its head's rows past those it held, read in place (0353 step 6). 0 for every row.
    pub(in crate::code_rule_dsl) from: u32,
    /// The relation's holder, for one read where it lies in its block (`in_block`): what an index built into it keeps
    /// until its batch is done (`Building`, Q268).
    kept: Option<crate::code_rule_dsl::facts::rows::Part>,
    /// A `has` axis read off the tape (`DevRel::has`): its set's kind atoms, ascending, two words each.
    has: Option<Words>,
    /// Its rows are its length as the device claims it, `rows` at most (`live`): a recursive component's relation that
    /// grows round after round within one launch (decision 0603 step 3).
    live: bool,
    /// A live relation's rows mapped and its indexes' buckets (`DevRel::mapped`): its indexes' room is for those rows,
    /// grown with them between its program's launches (`Batch::gate`'s rooms), where `rows` is its bound.
    mapped: Option<(usize, usize)>,
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
        DevRel::empty_indexed(cols, rows, 1)
    }

    /// `empty` with room in its block for `indexes` indexes (`Part::for_device_indexed`): one for each key its readers
    /// probe it by, so none of them copies it (`Batch::index`).
    pub(in crate::code_rule_dsl) fn empty_indexed(cols: usize, rows: usize, indexes: usize) -> Result<DevRel, Error> {
        let part = crate::code_rule_dsl::facts::rows::Part::for_device_indexed(cols, rows, indexes).map_err(|_| Error::BackendCapacity)?;
        DevRel::of_part(part, cols, rows)
    }

    /// A handle to `part`, a relation of the device's own of `cols` columns and `rows` rows (`Part::for_device`'s).
    fn of_part(part: crate::code_rule_dsl::facts::rows::Part, cols: usize, rows: usize) -> Result<DevRel, Error> {
        let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
        let buffer = held_block(device, part.facts(), Box::new(part.facts().clone()))?;
        debug_assert_eq!(part.rows_at(), part.rel_at() + REL_ROWS as usize);
        let home = Some(part.facts().clone());
        Ok(DevRel { buffer, rows, cols, at: part.rel_at() as u64, dead: 0, part: Some(part), home, view: None, made: (0, 0), from: 0, kept: None, has: None, live: false, mapped: None }.noted())
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

    /// A `has` axis read off the tape (decision 0353 step 5): the batch's `node` view in place (`view`, of `nodes`
    /// rows) and its set's kind atoms `kinds`, ascending. It has no rows of its own: a stage probes it by one of its
    /// columns only (`Batch::by_has`), each binding's rows walked on the tape.
    pub(in crate::code_rule_dsl) fn has(nodes: usize, view: crate::code_rule_dsl::facts::vtab::ViewInPlace, kinds: &[i64]) -> Result<DevRel, Error> {
        // Nothing indexes it (`Batch::by_has` walks the tape): its relation of no columns has no room for an index.
        let part = crate::code_rule_dsl::facts::rows::Part::for_device_unindexed(nodes).map_err(|_| Error::BackendCapacity)?;
        let mut dev = DevRel::of_part(part, 0, nodes)?;
        dev.cols = 2;
        dev.view = Some(view);
        let set = Words::zeroed(2 * kinds.len())?;
        for (i, &k) in kinds.iter().enumerate() {
            set.set(2 * i, k as u32);
            set.set(2 * i + 1, (k >> 32) as u32);
        }
        dev.has = Some(set);
        Ok(dev)
    }

    /// Whether it is a `has` axis read off the tape (`has`).
    pub(in crate::code_rule_dsl) fn is_has(&self) -> bool {
        self.has.is_some()
    }

    /// Makes the buffers a view's reads reach by address resident in `pass` (none for a relation).
    pub(in crate::code_rule_dsl) fn resident_in(&self, pass: &Pass) {
        if let Some(v) = &self.view {
            let buffers: Vec<&Buffer> = v.buffers.iter().collect();
            pass.use_read(&buffers);
            if let Some(cache) = v.cache.as_ref().and_then(|c| c.buffer.as_ref()) {
                pass.use_read_write(cache);
            }
        }
        if let Some(set) = &self.has {
            pass.use_read(&[&set.buffer.buffer]);
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
        DevRel { buffer, rows: words, cols: 1, at: 0, dead: 0, part: None, home: None, view: None, made: (0, 0), from: 0, kept: None, has: None, live: false, mapped: None }
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
        let buffer = held_block(device, &b.facts, Box::new(b.clone()))?;
        Ok(DevRel { buffer, rows: b.rows as usize, cols, at: b.at as u64, dead: b.dead as u64, part: None, home: Some(b.facts.clone()), view: None, made: (0, 0), from: 0, kept: b.part.clone(), has: None, live: false, mapped: None }.noted())
    }

    /// Rows `from..from + rows` of `b`'s relation, of `cols` columns, read where they lie (`in_block`): a fixpoint's
    /// delta, the rows its head took this round, past those it held (0353 step 6). Its readers read them by their ids
    /// in the head; an index of it is of them alone, and is told from the head's by its rows.
    pub(in crate::code_rule_dsl) fn in_block_from(b: &crate::code_rule_dsl::facts::rows::InBlock, cols: usize, from: usize, rows: usize) -> Result<DevRel, Error> {
        assert!(from + rows <= b.rows as usize, "rows {from}..{} of a relation of {}", from + rows, b.rows);
        let mut d = DevRel::in_block(b, cols)?;
        d.rows = rows;
        d.from = from as u32;
        Ok(d)
    }

    /// `self` read as live (`DevRel::live`): `rows` its bound, its rows its length as the device claims it.
    pub(in crate::code_rule_dsl) fn live(mut self) -> DevRel {
        self.live = true;
        self
    }

    /// `self` (live) with room for `rows` rows mapped of its bound, its indexes' `buckets` (a power of two): its rows'
    /// room grows as its program's rounds reach it (decision 0603, `Batch::gate`'s rooms), and its indexes' rows with
    /// it, their buckets as they are.
    pub(in crate::code_rule_dsl) fn mapped(mut self, rows: usize, buckets: usize) -> DevRel {
        assert!(self.live && rows <= self.rows && buckets.is_power_of_two(), "a live relation's {rows} rows mapped of {}", self.rows);
        self.mapped = Some((rows, buckets));
        self
    }

    /// The rows its indexes have room for: its mapped rows (a live relation mapped as reached), else its rows.
    fn index_rows(&self) -> usize {
        self.mapped.map_or(self.rows, |m| m.0)
    }

    /// The address of its rows' length (`Rows::len`) for a job that reaches its buffer: a gate on its room.
    fn len_at(&self) -> u64 {
        self.rows_at()
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
    /// A table's values' offset in its buffer too (`Table::lies`; 0 for a relation).
    fn of(&self) -> (Buffer, usize, u64, u64, u64) {
        match self {
            Rows::Table(t) => {
                let (b, at) = t.lies();
                (b, t.room(), 0, 0, at)
            }
            Rows::Rel(r) => {
                assert!(r.at != 0, "a copy of words, not a relation");
                (r.buffer.buffer.clone(), r.rows, r.at, r.rows_at(), 0)
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
    /// A source relation's first row (`DevRel::from`): its row `t` is its id `src_from + t`.
    src_from: u32,
    pad: u32,
    /// Where its rows go past `dst_off` when `flags & 1`: a word an earlier phase wrote (`Batch::append_rows`).
    off: u64,
    src_at: u64,
    dst_rows_at: u64,
    /// A source view read in place (`Stage::v_dir`, `r_view`; 0: none).
    v_dir: u64,
    r_view: u32,
    flags: u32,
}

const _: () = assert!(std::mem::size_of::<CopyParams>() == 264);

/// An index's key pass's parameters (`ix_key_one`'s `IxP`): the rows, the key's columns, the relation's descriptor's
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
    /// The relation's first row (`DevRel::from`): the index is of its rows `from..from + n`, by their ids.
    from: u32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<IxParams>() == 80);

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
    /// No index, a walk on the tape: 1 the `child` view probed by its parent, the parent's children (`by_parent`); 2 a
    /// `has` axis probed by its node, 3 by its ancestor (`by_has`); 0 an index.
    walk: u32,
    /// Its block's buffer, taken once the index has its room: what its builder and its probes bind. A block with no
    /// slab of its own is bound by a wrap of its used extent (`Block::device_buffer`), which a buffer taken before the
    /// index was added (its relation's) does not reach.
    buffer: Held,
    /// The relation's columns it is by (`Batch::index`'s `cols`), to find it again (`Batch::again`).
    cols: Vec<usize>,
}

impl DevIndex {
    /// Whether it copied its relation, whose block had no room for it (`moved`, 0353 step 6).
    pub(in crate::code_rule_dsl) fn copied(&self) -> bool {
        self.moved.is_some()
    }

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
    held_block(device, home, Box::new(home.clone()))
}

/// The buffer of `facts`' block as it is now, `keep` kept with it, and the block held by its own region (decision
/// 0491 step 3): a launch that binds it pins that region by the kernel's access.
fn held_block(device: &rerail_device::device::Device, facts: &std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>, keep: Box<dyn std::any::Any + Send + Sync>) -> Result<Held, Error> {
    Ok(Held::of_block(facts.block.device_buffer(device)?, rerail_blocks::region::whole(&facts.block), keep))
}

/// A stage's screen (decision 0488, `Stage::screen`): the probed relation's key `key`'s column's kept filter, a
/// `rerail_blocks::bloom::Bloom` at byte `at` of `held` (the filter's block, bound where it lies: nothing copied).
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Screen {
    pub(in crate::code_rule_dsl) held: std::rc::Rc<Held>,
    pub(in crate::code_rule_dsl) at: u32,
    pub(in crate::code_rule_dsl) key: u32,
}

/// A stage's probed relation `rel` (and its index, read by keys) into its fields.
fn bind_right(stage: &mut Stage, rel: &DevRel, index: Option<&DevIndex>) {
    stage.nr = rel.rows as u32;
    stage.r_from = rel.from;
    stage.r_len = u32::from(rel.live);
    stage.r_at = rel.at;
    stage.r_dead = rel.dead;
    // A relation reads none: a stage bound before to a view reads its new relation, not the view.
    (stage.v_dir, stage.r_view) = rel.view.as_ref().map_or((0, 0), |v| (v.dir, v.kind));
    stage.v_parent = 0;
    (stage.v_set, stage.v_nset, stage.v_has) = rel.has.as_ref().map_or((0, 0, 0), |set| (set.buffer.buffer.device_address().expect("a has axis's set where views are read in place"), (set.len / 2) as u32, 0));
    if let Some(ix) = index {
        stage.r_index = ix.index;
        stage.v_parent = u32::from(ix.walk == 1);
        stage.v_has = ix.walk.saturating_sub(1);
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
    /// Its builder's work all queued in the batch: one that is not (an error before it was) gives its room back when
    /// the batch is done, never marked built.
    encoded: bool,
    /// Its work committed, and so told to its block (`Facts::ix`, `InFlight`): another batch's probe takes it.
    queued: bool,
    /// Whether it was queued after a gate (`Batch::gate`): not built when the gate stopped its program.
    gated: bool,
    /// A live relation's (`DevRel::live`): of its rows as a round claimed them, never marked built (a reader of the
    /// relation as it lies finds none of its rows' count).
    live: bool,
    /// The buffer its builder binds the block by: what this batch's probes of it bind too. Metal orders two launches
    /// by the buffers both bind, and a wrap of the block made after it (the block grown, `Block::device_buffer`) is
    /// another buffer, which the build's writes are not ordered before.
    buffer: Option<Buffer>,
}

impl Building {
    /// Its relation's descriptor's offset in its block: what its `InFlight` is found by.
    fn at(&self) -> usize {
        self.home.block.offset_of(self.rel)
    }

    /// No longer in flight: its block's list forgets it (under the block's index lock, `held`). Whether another
    /// batch took it meanwhile (`InFlight::lent`).
    fn forget(&self, held: &mut Vec<crate::code_rule_dsl::facts::rows::InFlight>) -> bool {
        let mut lent = false;
        if self.queued {
            let at = self.at();
            held.retain(|f| {
                let this = f.at == at && f.room.index == self.room.index;
                lent |= this && f.lent;
                !this
            });
        }
        lent
    }

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
    /// Its rows' room; a placed table's, what its rows were guessed to be (its items, and its program's arena).
    pub(in crate::code_rule_dsl) cap: usize,
    pub(in crate::code_rule_dsl) slots: usize,
    /// Where a placed table lies (`Batch::table`): its program's arena, as its producer's size script placed it.
    place: Option<std::rc::Rc<Place>>,
}

/// A table its program places in its arena (decision 0603 step 5): its producer's size script claims its room from
/// the rows the level's scan counted, and writes where it is and its rows' room (`rec`); the jobs that read it, at
/// their levels' start, take them from there.
struct Place {
    /// `[its offset in the arena in 8-byte units and one, its rows' room]`, the device's.
    rec: Words,
    /// The arena it lies in, once its program launched (`run_program`): read back through it.
    arena: std::cell::RefCell<Option<std::rc::Rc<Held>>>,
    /// Its order against other jobs' tables (`Job::order`): an address no buffer has.
    key: u64,
    /// Whether its batch is done (`Batch::finish`): it lies where its size script placed it, read there as a table
    /// is (`Table::lies`), its rows' room as the device wrote it (`Table::room`).
    fixed: std::cell::Cell<bool>,
}

/// Where the keys of placed tables start (`Place::key`): past any device address.
const PLACE_KEYS: u64 = 1 << 62;

impl Table {
    pub(in crate::code_rule_dsl) fn new(slots: usize, cap: usize) -> Result<Table, Error> {
        Ok(Table { buffer: Held::new((8 * slots.max(1) * cap.max(1)).max(8))?, cap: cap.max(1), slots, place: None })
    }

    /// Whether its batch's programs place it (`Batch::table`), its batch not done: its rows' room is the device's.
    pub(in crate::code_rule_dsl) fn placed(&self) -> bool {
        self.place.as_ref().is_some_and(|p| !p.fixed.get())
    }

    /// Whether its program sized it (`Batch::table`): its room is its rows, before and after its batch is done, so it
    /// never overflows.
    pub(in crate::code_rule_dsl) fn device_sized(&self) -> bool {
        self.place.is_some()
    }

    /// Its rows' room: a placed table's of a batch done, as the device wrote it; else its own.
    pub(in crate::code_rule_dsl) fn room(&self) -> usize {
        match &self.place {
            Some(p) if p.fixed.get() => (p.rec.get(1) as usize).max(1),
            _ => self.cap,
        }
    }

    /// Its buffer, and its values' offset in it: a placed table of a batch done, its arena's, where it lies.
    fn lies(&self) -> (Buffer, u64) {
        match &self.place {
            Some(p) if p.fixed.get() => {
                let at = p.rec.get(0) as u64;
                assert!(at > 0, "a placed table its batch never placed");
                (p.arena.borrow().as_ref().expect("its arena").buffer.clone(), 8 * (at - 1))
            }
            _ => (self.buffer.buffer.clone(), 0),
        }
    }

    /// Its address as `job` reaches it (`lies`).
    fn reach_by(&self, job: &mut Job, write: bool) -> Result<u64, Error> {
        let (b, at) = self.lies();
        job.reach(&b, at, write)
    }

    /// The address its jobs are ordered by (`Job::order`): its buffer's (`at`), or its place's key.
    fn key(&self, at: u64) -> u64 {
        self.place.as_ref().map_or(at, |p| p.key)
    }

    /// The table's one row `values` (a rule's first table: its preset slots).
    pub(in crate::code_rule_dsl) fn one(slots: usize, values: &[i64]) -> Result<Table, Error> {
        let t = Table::new(slots, 1)?;
        // SAFETY: the buffer holds at least `slots.max(1)` values, and nothing else refers to it yet (not placed).
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

    /// Slot `slot`'s first `n` values, once the command buffer that wrote them completed: a placed table's where its
    /// size script placed it.
    pub(in crate::code_rule_dsl) fn column(&self, slot: usize, n: usize) -> &[i64] {
        if let Some(p) = &self.place {
            let (at, cap) = (p.rec.get(0) as usize, p.rec.get(1) as usize);
            let arena = p.arena.borrow();
            let Some(a) = arena.as_ref().filter(|_| at > 0) else { return &[] };
            assert!(8 * (at - 1 + self.slots * cap) <= a.buffer.length() as usize, "a placed table past its arena");
            // SAFETY: its producer placed it in the arena (`at - 1` words in, `cap` values a slot, within the arena:
            // asserted), which its place holds; the writer completed, and the arena is never freed while held.
            return unsafe { std::slice::from_raw_parts(a.buffer.contents().cast::<i64>().add(at - 1 + slot * cap), n.min(cap)) };
        }
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

    /// A reader of word `i` that holds its buffer (a gate's room's growth reads a round's count with it, between
    /// launches: `RoomGrow`).
    pub(in crate::code_rule_dsl) fn reader(&self, i: usize) -> impl Fn() -> u32 + 'static {
        assert!(i < self.len, "word {i} of {}", self.len);
        let b = self.buffer.buffer.clone();
        // SAFETY: as `get`'s: word `i` of the buffer it holds, read once no command buffer writing it runs.
        move || unsafe { b.contents().cast::<u32>().add(i).read() }
    }
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
/// jobs wanted; its second, its capacity. Address space (`rerail_device::device::Reserved`, host-read: on Metal the
/// host's pages, wrapped over the bytes mapped) whose pages are mapped as its texts reach them: a phase whose texts
/// found it full parks, and the host maps twice what they wanted (`grow`) and launches the program again, which runs
/// the phase again (decision 0603's fewer reruns). Where it cannot grow (an allocation of its first bytes), the run
/// runs again in a batch whose arena is as big as its jobs wanted, or on the host past the device's text arena.
pub(in crate::code_rule_dsl) struct Arena {
    reserved: Arc<rerail_device::device::Reserved>,
    cap: std::cell::Cell<usize>,
}

/// A batch's arena, to start with: most strata make few texts.
pub(in crate::code_rule_dsl) const ARENA_START: usize = 64 << 10;

/// A text arena's address space: the device's text arena (its bytes past it run on the host), within the 32-bit
/// bump of its header.
fn text_arena_max() -> usize {
    rerail_accel::budget::metal_share(64 << 20).clamp(ARENA_START, u32::MAX as usize / 2)
}

impl Arena {
    pub(in crate::code_rule_dsl) fn new(cap: usize) -> Result<Arena, Error> {
        let cap = cap.clamp(16, u32::MAX as usize / 2);
        let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
        let reserved = rerail_device::device::reserve(device, 16 + cap.max(text_arena_max()), 16 + cap, true).map_err(|_| Error::BackendCapacity)?;
        let arena = Arena { reserved: Arc::new(reserved), cap: std::cell::Cell::new(cap) };
        arena.header(0, 0);
        arena.header(1, cap as u32);
        Ok(arena)
    }

    /// Its header's word `k` set to `v` (no command buffer that uses the arena pending).
    fn header(&self, k: usize, v: u32) {
        // SAFETY: the 16 bytes of header, mapped from the start; no command buffer is using the arena.
        unsafe { *self.base().cast::<u32>().add(k) = v };
    }

    fn base(&self) -> *mut u8 {
        self.reserved.buffer().contents().cast::<u8>()
    }

    /// The buffer its jobs reach now (a wrapped arena's last wrap: `grow` makes another, at another address).
    pub(in crate::code_rule_dsl) fn buffer(&self) -> Buffer {
        self.reserved.buffer()
    }

    /// Its room for texts, past its header.
    pub(in crate::code_rule_dsl) fn cap(&self) -> usize {
        self.cap.get()
    }

    /// Room for twice what its jobs wanted, mapped further between launches where it has less (another program of
    /// the batch's may have grown it already): false where it cannot be (past its address space, or an allocation
    /// that cannot grow).
    pub(in crate::code_rule_dsl) fn grow(&self) -> Result<bool, Error> {
        let wanted = self.wanted();
        let room = (2 * wanted).min(u32::MAX as usize / 2);
        if wanted > room {
            return Ok(false);
        }
        if room <= self.cap() {
            return Ok(true);
        }
        if !self.reserved.map(16 + room).map_err(|_| Error::BackendCapacity)? {
            return Ok(false);
        }
        let cap = (self.reserved.mapped() - 16).min(u32::MAX as usize / 2);
        self.cap.set(cap);
        self.header(1, cap as u32);
        Ok(true)
    }

    /// The bytes its jobs asked for (past `cap` when it filled), once they completed.
    pub(in crate::code_rule_dsl) fn wanted(&self) -> usize {
        // SAFETY: the header's first word, read once the command buffers that write it completed.
        unsafe { *self.base().cast::<u32>() as usize }
    }

    /// `bytes` put into the arena by the host, between batches (no command buffer writing it is pending): their
    /// value, or none when it has no room.
    pub(in crate::code_rule_dsl) fn put(&self, bytes: &[u8]) -> Option<i64> {
        let off = self.wanted();
        if off + bytes.len() > self.cap() || bytes.len() > 0xff_ffff {
            return None;
        }
        // SAFETY: `bytes.len()` bytes within `cap` past the header, mapped; no command buffer is using the arena (the
        // caller's between batches).
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.base().add(16 + off), bytes.len()) };
        self.header(0, (off + bytes.len()) as u32);
        Some(ARENA_TEXT | ((off as i64) << 24) | bytes.len() as i64)
    }

    /// The text of arena value `v` (`ARENA_TEXT`'s), once the command buffers that made it completed.
    pub(in crate::code_rule_dsl) fn text(&self, v: i64) -> Option<&[u8]> {
        if v & ARENA_TEXT == 0 || v == -1 {
            return None;
        }
        let (off, len) = (((v as u64) >> 24 & ((1 << 39) - 1)) as usize, (v & 0xff_ffff) as usize);
        if off + len > self.cap() {
            return None;
        }
        // SAFETY: within the arena's `cap` bytes past its header, mapped; its pages stay where they are as it grows.
        Some(unsafe { std::slice::from_raw_parts(self.base().add(16 + off), len) })
    }
}

/// What a batch did: its command buffers, dispatches, the GPU's time on them, and the host's CPU encoding them.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::code_rule_dsl) struct Done {
    pub(in crate::code_rule_dsl) submissions: u32,
    pub(in crate::code_rule_dsl) dispatches: u32,
    pub(in crate::code_rule_dsl) gpu_seconds: f64,
    /// Levels run (`Batch::flush`): the jobs queued between two dependencies, their phases in a program.
    pub(in crate::code_rule_dsl) levels: u32,
    /// Stages screened by their relation's kept filter (`Stage::screen`, decision 0488).
    pub(in crate::code_rule_dsl) screened: u32,
    /// Whether a gate stopped its program (`Batch::gate`): the work after it is undone.
    pub(in crate::code_rule_dsl) stopped: bool,
    /// Whether a round's end ended its program (`Batch::round_end`): a round made no rows.
    pub(in crate::code_rule_dsl) ended: bool,
    /// Whether a round's end stopped it past its rounds.
    pub(in crate::code_rule_dsl) past_rounds: bool,
    /// Whether a size script parked past its program's arena (decision 0603 step 5), an arena whose pages could not
    /// be mapped further, with `stopped`; the bytes its claims wanted. The batch's next programs' arenas are twice
    /// that at least (`Batch::arena_floor`).
    pub(in crate::code_rule_dsl) arena_full: bool,
    pub(in crate::code_rule_dsl) arena_wanted: usize,
    /// Its programs' launches again (decision 0603 point 5): a budget's, where a launch spent its budget with items
    /// left, and an arena's, where a size script parked and the arena's pages were mapped for it.
    pub(in crate::code_rule_dsl) relaunches: u32,
    pub(in crate::code_rule_dsl) remaps: u32,
    /// Its remaps of a gate's rooms (`Batch::gate_rooms`): rows mapped as a recursive component's rounds reach them.
    pub(in crate::code_rule_dsl) room_remaps: u32,
    /// Its programs' phases run (their dispatches), over all their launches, and a launch's chain's dispatches, the
    /// least of its programs'.
    pub(in crate::code_rule_dsl) taken: u64,
    pub(in crate::code_rule_dsl) launch_items: u64,
    /// Its programs launched (each once, then its relaunches and remaps).
    pub(in crate::code_rule_dsl) programs: u32,
}

/// Work encoded into one submission at a time, committed at `commit_open` and `finish`; every submission runs on the
/// process's queue, in order. Its stages, maps, text rows, dedups, copies and indexes are queued (`queue`), a level
/// at a time between two dependencies (`flush`, decision 0601), into its program (decision 0603): every level up to
/// the next launch made at once, or the commit, in one launch.
pub(in crate::code_rule_dsl) struct Batch {
    pass: Option<Pass>,
    /// The level's jobs, not in its program yet (`queue`).
    queued: Vec<Job>,
    /// The levels flushed since the program last launched (`flush`, `run_program`, decision 0603).
    program: Program,
    /// The programs launched since the last `finish`: read when their work is done.
    programs: Vec<Ran>,
    committed: Vec<Submitted>,
    done: Done,
    dummy: Held,
    /// The texts its jobs' text functions make, made the first time a job asks.
    arena: Option<std::rc::Rc<Arena>>,
    arena_cap: usize,
    /// The indexes its work builds, marked built when it is done: those queued before a gate that stopped
    /// (`gated` then) alone.
    building: Vec<Building>,
    /// The submissions of indexes other batches build that this one's stages read (`InFlight`): waited for at its finish,
    /// and one the device failed fails this batch too.
    borrowed: Vec<Submitted>,
    /// Whether a gate was queued since the last `finish`.
    gated: bool,
    /// The atoms directory its text kernels read symbols' bytes through (decision 0478 step 2e), made the first
    /// time a stage reads a symbol's text and made again when the store's atoms have grown past it.
    atoms: Option<std::rc::Rc<DevAtoms>>,
    /// The store's one rolling cache, for the verdicts and texts its text kernels keep (decision 0478 step 2e).
    kept: Option<std::rc::Rc<DevKept>>,
    /// The relations its work writes in their blocks, each with its rows' `needed` before (decision 0463): a row
    /// its kernels could not write, past the chunks its relation holds, fails the batch, named.
    written: Vec<Written>,
    /// The tables it placed (`table`), counted: their keys.
    places: u64,
    /// Its programs' arenas' bytes at least (`arena_floor`).
    floor: usize,
    /// Its programs' arena (decision 0603 step 5), made at its first launch; the tables placed in it, fixed where
    /// they lie when the batch is done (`Place::fixed`).
    pool: Option<std::rc::Rc<Held>>,
    made: Vec<std::rc::Rc<Place>>,
    /// Whether its arena is address space whose pages are mapped as its claims reach them (`arena_reserved`), and
    /// that arena once made.
    reserve_arena: bool,
    /// Whether the host reads its arena's placed tables (`arena_reserved`'s): on Metal a wrapped arena then.
    host_reads: bool,
    reserved: Option<std::sync::Arc<rerail_device::device::Reserved>>,
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
        // The one before goes: a pass that binds its buffers holds them until its work completes (decision 0491 step 2).
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

    pub(in crate::code_rule_dsl) fn new() -> Result<Batch, Error> {
        rerail_accel::gpu::join::device()?;
        Batch::with_arena(ARENA_START)
    }

    /// A batch whose text arena holds `cap` bytes.
    pub(in crate::code_rule_dsl) fn with_arena(cap: usize) -> Result<Batch, Error> {
        rerail_accel::gpu::join::device()?;
        Ok(Batch {
            pass: None,
            queued: Vec::new(),
            program: Program::default(),
            programs: Vec::new(),
            gated: false,
            committed: Vec::new(),
            done: Done::default(),
            dummy: Held::new(64)?,
            arena: None,
            arena_cap: cap,
            building: Vec::new(),
            borrowed: Vec::new(),
            atoms: None,
            kept: None,
            written: Vec::new(),
            places: 0,
            floor: 0,
            pool: None,
            made: Vec::new(),
            reserve_arena: false,
            host_reads: false,
            reserved: None,
        })
    }

    /// A table of `slots` columns its program places in its arena (decision 0603 step 5): its rows' room the device's,
    /// from the rows its producer's level counted; `guess` its rows as guessed (its items and its arena's bytes).
    pub(in crate::code_rule_dsl) fn table(&mut self, slots: usize, guess: usize) -> Result<Table, Error> {
        self.places += 1;
        let place = Place { rec: Words::zeroed(2)?, arena: std::cell::RefCell::new(None), key: PLACE_KEYS + self.places, fixed: std::cell::Cell::new(false) };
        Ok(Table { buffer: Held::new(16)?, cap: guess.max(1), slots, place: Some(std::rc::Rc::new(place)) })
    }

    /// Its programs' arenas hold `bytes` at least: a program whose claims passed its arena runs again in a larger
    /// one (`Done::arena_full`).
    pub(in crate::code_rule_dsl) fn arena_floor(&mut self, bytes: usize) {
        self.floor = self.floor.max(bytes);
    }

    /// Its programs' arenas' bytes at least, as its last ones' claims wanted.
    pub(in crate::code_rule_dsl) fn floor(&self) -> usize {
        self.floor
    }

    /// Its programs' arena is `ARENA_MAX` bytes of address space whose pages are mapped as its claims reach them
    /// (`rerail_device::device::Reserved`, decision 0603's fewer reruns): a size script past them parks, and the host
    /// maps twice what it wanted and launches the program again, where it goes on. Real memory is the pages its
    /// claims reached. For a batch whose placed tables the host does not read (a program of rounds'): on Metal the
    /// arena is private storage.
    pub(in crate::code_rule_dsl) fn arena_reserved(&mut self, host_reads: bool) {
        self.reserve_arena = true;
        self.host_reads = host_reads;
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

    /// `stage` screened by `screen` (decision 0488): the buffer to bind as the stage's `F` (the batch's dummy for
    /// none), held by the pass that binds it (decision 0491 step 2).
    fn screen(&mut self, stage: &mut Stage, screen: Option<&Screen>) -> Buffer {
        match screen {
            Some(f) if (f.key as usize) < stage.nkeys as usize => {
                stage.screen = f.key + 1;
                stage.screen_at = f.at;
                self.done.screened += 1;
                f.held.buffer.clone()
            }
            _ => self.dummy.buffer.clone(),
        }
    }

    /// A batch whose text arena is `arena`, kept across batches: a recursive component's rounds (decision 0307),
    /// whose texts one round makes the next reads.
    pub(in crate::code_rule_dsl) fn with_shared_arena(arena: std::rc::Rc<Arena>) -> Result<Batch, Error> {
        let mut b = Batch::with_arena(arena.cap())?;
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

    /// The open submission, for a launch made at once: the program of the levels queued before it launched first
    /// (`run_program`), so it sees their writes. Its launches are charged to the evaluation's launch budget as they
    /// are made (`rerail_accel::gpu::launch`): past it, the hard failure (decision 0601).
    fn pass(&mut self) -> Result<&Pass, Error> {
        self.run_program()?;
        self.open()
    }

    /// The open submission (a new one when none is open), the level left queued: what a queued job makes resident.
    fn open(&mut self) -> Result<&Pass, Error> {
        if self.pass.is_none() {
            super::metal_sort::ready()?;
            self.pass = Some(Pass::new()?);
            rerail_accel::exec::command_opened();
        }
        Ok(self.pass.as_ref().expect("an open pass"))
    }

    /// `bytes` in a buffer of their own: a parameter block a kernel reads (a stage's, a copy's), past what a launch's
    /// own parameters hold on every backend. A launch that binds it holds it until its work completes (decision 0491
    /// step 2), so its holder may drop it once the launch is encoded.
    fn block(&mut self, bytes: &[u8]) -> Result<Held, Error> {
        Held::of(bytes.iter().copied()).map_err(|_| Error::BackendCapacity)
    }

    /// Commits the open submission (`Pass::commit`, checked against the launch budget).
    fn submit_open(&mut self) -> Result<(), Error> {
        self.run_program()?;
        let Some(pass) = self.pass.take() else { return Ok(()) };
        self.done.dispatches += pass.dispatches() as u32;
        rerail_accel::exec::command_committed();
        let s = pass.commit().map_err(Error::from)?;
        // The indexes whose builders it holds are told to their blocks: a probe of another batch takes one, its work
        // committed after this and so run after (`InFlight`). One queued after a gate, or a live relation's, may not
        // be built (`finish`): none is told.
        for b in self.building.iter_mut().filter(|b| b.encoded && !b.queued && !b.gated && !b.live) {
            let mut held = b.home.ix.lock().unwrap_or_else(|e| e.into_inner());
            held.push(crate::code_rule_dsl::facts::rows::InFlight { at: b.at(), room: b.room, done: s.watch(), lent: false });
            b.queued = true;
        }
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
        // Its programs launched again while one left items (a budget's) or parked (an arena's), each relaunch a
        // launch the ledger counts (decision 0601).
        loop {
            for s in self.committed.drain(..) {
                let (seconds, ok) = s.wait_ok();
                self.done.gpu_seconds += seconds;
                failed += usize::from(!ok);
            }
            if failed > 0 {
                self.programs.clear();
                break;
            }
            let (whole, again) = self.programs_done()?;
            failed += usize::from(!whole);
            if again.is_empty() || failed > 0 {
                break;
            }
            for (r, remap) in again {
                // A remap (an arena mapped further) is a run again with more room: its launch charges the run's
                // reruns' budget (decision 0540), as the rerun it replaces did; a budget's relaunch, the evaluation's.
                let _rerun = remap.then(rerail_accel::gpu::launch::rerun);
                self.launch(r.kit, r.ctl)?;
            }
            self.submit_open()?;
        }
        // Its placed tables lie where its programs placed them: later batches read them there (decision 0603 step 5).
        // Its next programs' arena is a new one, as large as its claims wanted.
        for p in self.made.drain(..) {
            // Its arena's last buffer: a wrapped arena's wraps grow, and a table read from here on may lie past an
            // earlier one's end.
            if let Some(a) = &self.pool {
                *p.arena.borrow_mut() = Some(a.clone());
            }
            p.fixed.set(true);
        }
        self.pool = None;
        self.reserved = None;
        if let Some(t) = &self.program.top {
            t.set(0, 0);
        }
        // The indexes of other batches its stages read: one whose builder the device failed is half written.
        for s in self.borrowed.drain(..) {
            failed += usize::from(!s.wait_ok().1);
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
            // Its indexes are half written: their room goes back (the batch's work is done, so nothing writes it). One
            // another batch took (`InFlight::lent`), whose work reads it after this one's, keeps its room reserved,
            // never found (not built), until its block goes: given back, the room was the block's free space again
            // while the other batch read it.
            for b in self.building.drain(..) {
                let mut held = b.home.ix.lock().unwrap_or_else(|e| e.into_inner());
                let lent = b.forget(&mut held);
                b.check("its batch failed");
                if lent {
                    crate::code_rule_dsl::prof::count_named("device.index of a failed batch, lent, kept unbuilt".into(), 4 * b.room.rows_n as usize);
                    continue;
                }
                // SAFETY: the batch's work has completed, and no reader was given the index outside it.
                unsafe { (*b.rel).give_back_index(&b.home.block, &b.room) };
            }
            return Err(Error::BackendFailure);
        }
        let stopped = self.done.stopped;
        self.gated = false;
        for b in self.building.drain(..) {
            let mut held = b.home.ix.lock().unwrap_or_else(|e| e.into_inner());
            let lent = b.forget(&mut held);
            if (stopped && b.gated) || b.live {
                continue;
            }
            b.check("its batch done");
            let (r, room) = (unsafe { &*b.rel }, &b.room);
            // Another batch built the same index meanwhile (each found none, and built its own, on another thread),
            // or this batch's builder was not all encoded: it goes back, its room the block's again, and a later
            // probe finds the one built.
            let other = r.find_index_from(&room.key[..room.ncols], room.from, room.rows_n).is_some_and(|i| i != room.index);
            // Built twice, but another batch's probe took it ("taken from another batch's build", `InFlight::lent`), and
            // that batch's work, committed after this one's, may still read it. Given back, its
            // room was the block's free space again while that read ran (a fourth relation or index written there): it
            // is kept, built, beside the other (a probe finds the first).
            if other && lent {
                crate::code_rule_dsl::prof::count_named("device.index built twice, lent, kept".into(), 4 * room.rows_n as usize);
                // SAFETY: the batch's work, which built it, has completed; `home` keeps the relation.
                unsafe { r.index_built(room) };
                continue;
            }
            if other || !b.encoded {
                if other {
                    crate::code_rule_dsl::prof::count_named("device.index built twice, given back".into(),4 * room.rows_n as usize);
                }
                // SAFETY: the batch's work has completed; its stages, which read it, are done.
                unsafe { r.give_back_index(&b.home.block, room) };
                continue;
            }
            // SAFETY: the batch's work, which built it, has completed; `home` keeps the relation.
            unsafe { r.index_built(room) };
        }
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
        Ok(Some(DevIndex { index: NONE, rows: rel.rows, moved: None, built: false, walk: 1, buffer: bound(&home)?, cols: Vec::new() }))
    }

    /// A `has` axis probed by one of its columns (decision 0353 step 5): no index, each binding's rows walked on the
    /// tape (the kernels' `by_has`), by its node `[0]` a climb, by its ancestor `[1]` the subtree. `None` for any
    /// other relation; an error for any other probe of one, which reads it off the tape or not at all.
    pub(in crate::code_rule_dsl) fn by_has(&self, rel: &DevRel, cols: &[usize]) -> Result<Option<DevIndex>, Error> {
        if !rel.is_has() {
            return Ok(None);
        }
        let walk = match cols {
            [0] => 2,
            [1] => 3,
            _ => return Err(Error::UnsupportedFeature("a has axis probed by both its columns, or none")),
        };
        let home = rel.home.clone().expect("a view's relation of no columns");
        Ok(Some(DevIndex { index: NONE, rows: rel.rows, moved: None, built: false, walk, buffer: bound(&home)?, cols: Vec::new() }))
    }

    /// `rel`'s index on its columns `cols`, the relation's own (decision 0325 step 5): found where it was built
    /// before (by any batch, kept with the relation), else built by this batch's work into the relation's block
    /// (read by the stages after it), and marked built when the batch is done. A relation whose block has no room for
    /// it is copied once into a block of its own that has (with no dead rows: their bits lie in the old block).
    pub(in crate::code_rule_dsl) fn index(&mut self, rel: &DevRel, cols: &[usize]) -> Result<DevIndex, Error> {
        let mut ix = self.index_of(rel, cols, true)?;
        ix.cols = cols.to_vec();
        Ok(ix)
    }

    /// `ix`, an index a stage of an earlier batch probed `rel` by, as a stage encoded into this batch must probe it
    /// (an overflow's stages run again, `setfix::device::complete`). An index the earlier batch built, which another
    /// batch built meanwhile, went back when the earlier one was done (`finish`): its buckets emptied, its entry free
    /// for the next, so a probe of it found no rows (B023's closures, 2026-11-22: a function's body lost, its loop
    /// variable's late reads found). So a relation's index is found again (the one kept); a walk of the tape stands.
    /// An index of a relation the earlier batch copied (`moved`) is the copy's: found again there, as its builder,
    /// cut before it was all encoded, gave it back too.
    pub(in crate::code_rule_dsl) fn again(&mut self, rel: &DevRel, ix: &std::rc::Rc<DevIndex>) -> Result<std::rc::Rc<DevIndex>, Error> {
        if ix.walk != 0 {
            return Ok(std::rc::Rc::clone(ix));
        }
        if let Some(copy) = &ix.moved {
            let found = self.index(copy, &ix.cols)?;
            return Ok(std::rc::Rc::new(DevIndex { moved: Some(std::rc::Rc::clone(copy)), ..found }));
        }
        Ok(std::rc::Rc::new(self.index(rel, &ix.cols)?))
    }

    /// `may_move`: a relation whose block has no room may be copied into one that has; its copy may not (a block of
    /// its own sized for its index that still has no room is a sizing's error, not a reason to copy again).
    fn index_of(&mut self, rel: &DevRel, cols: &[usize], may_move: bool) -> Result<DevIndex, Error> {
        let home = rel.home.clone().expect("an index of a relation, not of words");
        let r = rel.relation();
        // A continuation's index is of every row it reads (its base's first: `ix_key_one` reads by id, `rel_get`).
        if r.base_rows != 0 && !r.continues() {
            // Q265: what was read, and where, so a writer that missed its own buffer can be named.
            let h = home.block.header();
            let origin = if rel.view.is_some() { "a view read in place" } else if rel.part.is_some() { "the device's own" } else { "a relation in its block" };
            panic!(
                "an index of a full relation, not a continuation (Q265): {origin} of {} rows, {} columns, at {} of a block of kind {} (magic {:#x}, {} used of {}); its header: arity {}, base rows {} ({:#x}), {} indexes, {} order keys; when made: arity {}, base rows {}\n  descriptor {}",
                rel.rows, rel.cols, rel.at, h.kind, h.magic, home.block.used(), home.block.reserved(), r.arity, r.base_rows, r.base_rows, r.nindex.load(std::sync::atomic::Ordering::Relaxed), r.nokey, rel.made.0, rel.made.1,
                Building::descriptor(std::ptr::from_ref(r)).as_chunks::<8>().0.iter().map(|w| format!("{:016x}", u64::from_le_bytes(*w))).collect::<Vec<_>>().join(" ")
            );
        }
        let key: Vec<u16> = cols.iter().map(|&c| c as u16).collect();
        // Another batch's build of it, taken: its index and its builder's submission, waited for below.
        let mut taken = None;
        let room = 'room: {
            let mut held = home.ix.lock().unwrap_or_else(|e| e.into_inner());
            // A delta read in place is its rows `from..`: its index is of those, not the relation's first rows. A live
            // relation's index is built again for each round (below): none found.
            if let Some(i) = r.find_index_from(&key, rel.from, rel.rows as u32).filter(|_| !rel.live) {
                return Ok(DevIndex { index: i as u32, rows: rel.rows, moved: None, built: false, walk: 0, buffer: bound(&home)?, cols: Vec::new() });
            }
            // This batch's own build: read by its stages after it. One in the open submission is ordered before them
            // (a submission's launches run in order); one in a submission already committed is not, as another batch's
            // is not (below), and is waited for the same way (B023's closures, 2026-11-22: a stage probing the solve of
            // `within` by an index this batch built in its last submission found 0 or 879 of its 66,093 rows).
            if let Some(b) = self.building.iter().find(|b| b.encoded && b.live == rel.live && std::ptr::eq(b.rel, r) && b.room.key[..b.room.ncols] == *key && b.room.from == rel.from && b.room.rows_n as usize == rel.index_rows()) {
                let (room, queued, at, built_by) = (b.room, b.queued, b.at(), b.buffer.clone());
                if rel.live {
                    // A live relation's index in this batch's room for it, built again of its rows now: each round's,
                    // in the open submission, so its probes after it, binding the buffer it binds, are ordered after it.
                    drop(held);
                    let buffer = bound(&home)?;
                    self.queue_index(rel, cols, &room, &buffer)?;
                    return Ok(DevIndex { index: room.index as u32, rows: rel.rows, moved: None, built: true, walk: 0, buffer, cols: Vec::new() });
                }
                if queued {
                    let f = held.iter().find(|f| f.at == at && f.room.index == room.index).expect("a committed build is told to its block");
                    taken = Some((room.index as u32, f.done.watch()));
                    break 'room None;
                }
                // Bound by the builder's buffer: its launches, before this batch's probes in the open submission,
                // are ordered before them only through a buffer both bind.
                let buffer = match built_by {
                    Some(buffer) => Held::of_block(buffer, rerail_blocks::region::whole(&home.block), Box::new(home.clone())),
                    None => bound(&home)?,
                };
                return Ok(DevIndex { index: room.index as u32, rows: rel.rows, moved: None, built: false, walk: 0, buffer, cols: Vec::new() });
            }
            // Another batch's, its work committed: taken, not built again. Building it again here took the block's room
            // twice (row 9 of 0353: two waves of a level each built [0,1], and the [0] after found no bytes left, and
            // copied the relation). Its builder's work is waited for before this batch's reads it (below): committed
            // first on the one queue is not run first. Metal orders two command buffers only by the buffers both bind,
            // and the builder and this batch bind the block through buffers of their own (`bound`), so this batch's
            // probes ran while the sort still wrote the index, and found some of its rows (B023's closures,
            // 2026-11-22: 961 of a stage's 66,093 rows, the closures' bodies lost, in about 1 of 8 runs on the M3). A
            // live relation's is this batch's own, each round's.
            let at = home.block.offset_of(std::ptr::from_ref(r));
            if let Some(f) = held.iter_mut().find(|f| f.at == at && f.room.key[..f.room.ncols] == *key && f.room.from == rel.from && f.room.rows_n as usize == rel.rows).filter(|_| !rel.live) {
                f.lent = true;
                taken = Some((f.room.index as u32, f.done.watch()));
                crate::code_rule_dsl::prof::count_named("device.index taken from another batch's build".into(), 4 * rel.rows);
                break 'room None;
            }
            // SAFETY: under the block's index lock; the relation's rows are written (or are by this batch's work
            // before the build, which its passes order).
            unsafe {
                match rel.mapped {
                    // A live relation mapped as reached: room for its rows mapped, its buckets its bound's keys'.
                    Some((rows, buckets)) => r.try_reserve_index_live(&home.block, &key, rel.from, rows as u32, buckets),
                    None => r.try_reserve_index_from(&home.block, &key, rel.from, rel.rows as u32),
                }
            }
        };
        // Out of the block's lock: its builder's work done before this batch encodes a read of it. A builder the device
        // failed fails this batch too.
        if let Some((index, builder)) = taken {
            if !builder.wait_ok().1 {
                return Err(Error::BackendFailure);
            }
            return Ok(DevIndex { index, rows: rel.rows, moved: None, built: false, walk: 0, buffer: bound(&home)?, cols: Vec::new() });
        }
        let Some(room) = room else {
            // A live relation grows where it lies: its index has room there, or it is refused.
            if rel.dead != 0 || rel.live {
                return Err(Error::BackendCapacity);
            }
            // A view's relation holds no values (its rows are read through the view's reads): a new one has room.
            // Either is kept by the batch until its work, which builds the index into it, completes (Q265: a fresh
            // relation dropped with its stage's handle gave back its index while the GPU built it).
            if let Some(view) = &rel.view {
                crate::code_rule_dsl::prof::count_named(format!("device.index of a view in a fresh home (its home's room taken): {} rows keyed by {cols:?}", rel.rows), 1);
                if !may_move {
                    return Err(Error::BackendCapacity);
                }
                let fresh = std::rc::Rc::new(DevRel::view(rel.cols, rel.rows, view.clone())?);
                let mut ix = self.index_of(&fresh, cols, false)?;
                ix.cols = cols.to_vec();
                return Ok(DevIndex { moved: Some(fresh), ..ix });
            }
// copy (0353 step 6): a second key's index copies the relation. It goes: a relation is made with room for
// the indexes its readers probe (decision 0353).
            // By where the relation lies: the device's own (an upload, a fixpoint's delta) has room for its first key's
            // index alone; one in a block of the run's shares the block's room with the relations beside it.
            const MOVED_OWN: rerail_prof::copies::Site = rerail_prof::copies::Site::new(9, "6", "a relation copied for a second key's index (DevIndex::moved): the device's own");
            const MOVED_IN_BLOCK: rerail_prof::copies::Site = rerail_prof::copies::Site::new(9, "6", "a relation copied for a key's index (DevIndex::moved): in a block with no room left");
            (if rel.part.is_some() { &MOVED_OWN } else { &MOVED_IN_BLOCK }).copied(8 * rel.cols * rel.rows);
            // Why its block refused the index: its directory full, or no bytes left; and what the relation is.
            if rel.part.is_none() {
                let full = r.nindex.load(std::sync::atomic::Ordering::Relaxed) as usize >= rerail_blocks::relation::ADDED_INDEXES;
                let free = home.block.reserved().saturating_sub(home.block.used());
                let why = format!(
                    "device.moved in a block: {} block, {} columns keyed by {:?}, {}, {}, {} indexes there ({} of other rows: {}), {} of {} MB left",
                    if home.is_shared() { "a shared" } else { "its own" },
                    rel.cols,
                    cols,
                    if full { "its directory full" } else { "no bytes for it" },
                    if r.continues() { "a continuation" } else if rel.dead != 0 { "with dead rows" } else { "a relation" },
                    r.nindex.load(std::sync::atomic::Ordering::Relaxed).min(9),
                    (0..r.nindex.load(std::sync::atomic::Ordering::Relaxed) as usize).filter(|&i| r.indexes.as_slice().get(i).is_some_and(|ix| ix.rows.len() != rel.rows)).count().min(9),
                    // Each entry's key, `o` an order (decision 0478 step 5b), `?` not built.
                    r.indexes.as_slice().iter().take(r.nindex.load(std::sync::atomic::Ordering::Relaxed) as usize).map(|ix| {
                        let k: Vec<String> = ix.key[..(ix.ncols as usize).min(8)].iter().map(u16::to_string).collect();
                        let what = if ix.state == rerail_blocks::relation::DEAD { "x" } else if ix.ncols == 0 { "?" } else if ix.ordered != 0 { "o" } else { "" };
                        format!("[{}]{what}{}", k.join(","), if ix.from != 0 { format!("@{}", ix.from) } else { String::new() })
                    }).collect::<Vec<_>>().join(" "),
                    if free < 1 << 20 { "< 1" } else if free < 16 << 20 { "1-16" } else { ">= 16" },
                    (home.block.reserved() >> 20).next_power_of_two(),
                );
                crate::code_rule_dsl::prof::count_named(why, 8 * rel.cols * rel.rows);
            }
            if !may_move {
                return Err(Error::BackendCapacity);
            }
            let copy = std::rc::Rc::new(DevRel::empty(rel.cols, rel.rows)?);
            let ident: Vec<Col> = (0..rel.cols).map(Col::From).collect();
            self.copy_rows(Rows::Rel(rel), rel.rows, &ident, Rows::Rel(&copy), 0)?;
            let mut ix = self.index_of(&copy, cols, false)?;
            ix.cols = cols.to_vec();
            return Ok(DevIndex { moved: Some(copy), ..ix });
        };
        let n = rel.rows;
        // Held by the batch from here: an error before its builder is all encoded gives the room back when the batch
        // is done (`finish`), as the batch's work may already write it.
        let held = self.building.len();
        let before = Building::descriptor(std::ptr::from_ref(r));
        self.building.push(Building { home: home.clone(), rel: std::ptr::from_ref(r), _holds: rel.holder(), room, before, encoded: false, queued: false, gated: self.gated, live: rel.live, buffer: None });
        let buffer = bound(&home)?;
        self.building[held].buffer = Some(buffer.buffer.clone());
        self.queue_index(rel, cols, &room, &buffer)?;
        self.building[held].encoded = true;
        Ok(DevIndex { index: room.index as u32, rows: n, moved: None, built: true, walk: 0, buffer, cols: Vec::new() })
    }

    /// An index's build into its room (`room`, in the block `buffer` binds), queued in the batch's level: every index of
    /// the level bucketed in one launch, sorted in one sort, placed in one launch (`flush`), before the level's stages,
    /// which probe it. A live relation's (`DevRel::live`) is of its rows as the device claimed them when it runs, at
    /// most `rows`: its keys past them go to its own last bucket (`IX_LIVE`), one more in the level's sort.
    fn queue_index(&mut self, rel: &DevRel, cols: &[usize], room: &rerail_blocks::relation::IndexRoom, buffer: &Held) -> Result<(), Error> {
        // Its keys at most (its items'), and the rows its room holds: a live relation mapped as reached has room for
        // its rows mapped, grown with them (`Kit::grow_rooms`), its items its bound's.
        let (n, m) = (rel.rows, rel.index_rows());
        assert!(room.rows as u64 + 8 * m as u64 <= buffer.buffer.length(), "the index's room (its rows, then their marks) within the buffer bound");
        if n == 0 {
            return Ok(());
        }
        assert!(cols.len() <= 8, "an index of {} columns", cols.len());
        let mut ix = IxParams { n: m as u32, ncols: cols.len() as u32 | if rel.live { IX_LIVE } else { 0 }, at: rel.at, mask: room.mask, cols: [0; 8], dir: 0, kind: 0, from: rel.from, pad: 0 };
        for (k, &c) in cols.iter().enumerate() {
            ix.cols[k] = c as u32;
        }
        if let Some(v) = &rel.view {
            (ix.dir, ix.kind) = (v.dir, u64::from(v.kind));
        }
        rel.resident_in(self.open()?);
        let mut job = Job::new(JobKind::Index, views_format([rel]), Stage::default());
        job.p = Some(ix.bytes().to_vec());
        // It reads the relation and writes its index's room in the block, which no queued job reads.
        job.desc.l = job.reach(&buffer.buffer, 0, true)?;
        job.desc.a = job.desc.l + room.offs as u64;
        job.desc.counters = job.desc.l + room.rows as u64;
        (job.count, job.buckets) = (n, room.mask as usize + 1);
        // A live relation's: its keys its length as claimed when it runs, its items that count's (decision 0603
        // step 5e).
        job.rows = rel.live.then(|| (job.desc.l + rel.at + REL_ROWS + ROWS_LEN, 0));
        job.rows_from = rel.from;
        job.live_ix = rel.mapped.and(rel.home.clone()).map(|h| (h, rel.at, room.index, m as u32));
        job.order(&[job.desc.l], &[]);
        self.queue(job)
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
            let pass = self.pass()?;
            pass.run_bytes(&expand, &[block, &out.buffer.buffer, &params.buffer], &[], groups(out.rows));
        }
        Ok(())
    }

    /// A stage from `left` into `out`, over `right` (its relation and, read by keys, its index), queued in the batch's
    /// level (`queue`): count, the level's scan, scatter. `stage`'s sizes are set here.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn stage(&mut self, mut stage: Stage, left: &Table, right: Option<(&DevRel, Option<&DevIndex>)>, screen: Option<&Screen>, in_slot: usize, out_slot: usize,
                                          totals: &Words, counters: &Words, counter: Option<usize>, out: &Table) -> Result<(), Error> {
        let format = views_format(right.map(|(rel, index)| probed(rel, index)));
        stage.cap_in = left.room() as u32;
        stage.cap_out = out.room() as u32;
        stage.in_slot = in_slot as u32;
        stage.out_slot = out_slot as u32;
        stage.counter = counter.map_or(NONE, |c| c as u32);
        stage.nslots = left.slots as u32;
        // A probe by keys reads the relation its index is of (`DevIndex::of`), the index in its block.
        let rbuf: Buffer = match right {
            Some((rel, index)) => {
                let rel = index.map_or(rel, |ix| ix.of(rel));
                bind_right(&mut stage, rel, index);
                rel.resident_in(self.open()?);
                index.map_or(&rel.buffer.buffer, |ix| ix.bound()).clone()
            }
            None => self.dummy.buffer.clone(),
        };
        let fbuf = self.screen(&mut stage, screen);
        // From one binding: a thread per candidate row (`scan_count`), its rows in the relation's order.
        let from_one = left.room() == 1 && !left.placed() && stage.mode == JOIN && right.is_some();
        let arena: Buffer = if matches!(stage.mode, STRCMP | TEXTOP) && (stage.agg[1] & 15 == OPND_ARENA || stage.agg[1] >> 4 == OPND_ARENA) {
            self.arena()?.buffer()
        } else {
            self.dummy.buffer.clone()
        };
        // A text op's or `str_cmp`'s symbols read through the batch's atoms directory (decision 0478 step 2e).
        let reads_names = matches!(stage.mode, STRCMP | TEXTOP);
        let atoms = self.atoms.clone().filter(|_| reads_names);
        if reads_names && atoms.is_none() {
            return Err(Error::BackendCapacity);
        }
        let n = if from_one { stage.nr as usize + 1 } else { left.room() + 1 };
        let mut job = Job::new(if from_one { JobKind::Scan } else { JobKind::Stage }, format, stage);
        job.reads_placed(left, &[(0, CAP_IN)]);
        job.makes_placed(out, &[(0, CAP_OUT)]);
        job.desc.l = left.reach_by(&mut job, false)?;
        job.desc.r = job.reach(&rbuf, 0, false)?;
        job.desc.totals = job.reach(&totals.buffer.buffer, 0, true)?;
        job.desc.counters = job.reach(&counters.buffer.buffer, 0, true)?;
        job.desc.o = out.reach_by(&mut job, true)?;
        job.desc.a = job.reach(&arena, 0, true)?;
        job.desc.f = job.reach(&fbuf, 0, false)?;
        job.desc.d = match &atoms {
            Some(a) => {
                a.resident_in(self.open()?);
                job.reach(&a.dir.0, a.dir.1, false)?
            }
            None => job.reach(&self.dummy.buffer, 0, false)?,
        };
        // A join's finds a binding (`stage_count_one`'s, its scatter's rows out): placed with its input where that is
        // placed (its room its input's), as a row's values are.
        if !from_one && stage.mode == JOIN {
            let finds = Held::new(if left.placed() { 16 } else { 8 * n })?;
            job.v_place = left.placed();
            job.desc.v = job.reach(&finds.buffer, 0, true)?;
            job.own.push(finds);
        }
        (job.n, job.count, job.scatter) = (n, n, n);
        job.m = match (from_one, stage.r_len != 0) {
            (true, true) => (Some(job.desc.r + stage.r_at + REL_ROWS + ROWS_LEN), stage.nr as usize),
            (true, false) => (None, stage.nr as usize),
            (false, _) => (Some(job.desc.totals + 4 * in_slot as u64), left.room()),
        };
        let (l, o) = (left.key(job.desc.l), out.key(job.desc.o));
        job.order(&[l, job.desc.r, job.desc.totals], &[o, job.desc.totals]);
        self.queue(job)
    }

    /// A witness stage (`WITNESS`, decision 0277) from `left` into `out`: per binding, the candidates of `right` in
    /// order until one that each of `rest`'s probes (semi- or anti-joins, at most `WITNESS_PROBES`) passes; then the
    /// bindings that have one, as a semi-join writes them. `stage`'s sizes, and the probes', are set here.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn witness(&mut self, mut stage: Stage, left: &Table, right: (&DevRel, Option<&DevIndex>), screen: Option<&Screen>, rest: &[(Stage, &DevRel, Option<&DevIndex>)],
                                            in_slot: usize, out_slot: usize, totals: &Words, out: &Table) -> Result<(), Error> {
        assert!(rest.len() <= WITNESS_PROBES, "a witness of more probes than it takes");
        let format = views_format(std::iter::once(probed(right.0, right.1)).chain(rest.iter().map(|(_, rel, index)| probed(rel, *index))));
        stage.cap_in = left.room() as u32;
        stage.cap_out = out.room() as u32;
        stage.in_slot = in_slot as u32;
        stage.out_slot = out_slot as u32;
        stage.counter = NONE;
        stage.nslots = left.slots as u32;
        stage.agg[0] = rest.len() as u32;
        let rel = right.1.map_or(right.0, |ix| ix.of(right.0));
        bind_right(&mut stage, rel, right.1);
        let a = right.1.map_or(&rel.buffer.buffer, |ix| ix.bound()).clone();
        let fbuf = self.screen(&mut stage, screen);
        let mut probes = [Stage::default(); WITNESS_PROBES];
        let mut bufs: Vec<Buffer> = Vec::new();
        for (k, (b, rel, ix)) in rest.iter().enumerate() {
            let mut b = *b;
            let rel = ix.map_or(*rel, |ix| ix.of(rel));
            bind_right(&mut b, rel, *ix);
            b.cap_in = left.room() as u32;
            probes[k] = b;
            bufs.push(ix.map_or(&rel.buffer.buffer, |ix| ix.bound()).clone());
        }
        // The bindings with a witness out as they are: a semi-join's scatter (mode 1 writes the binding alone).
        let mut write = stage;
        write.mode = SEMI;
        write.nbinds = 0;
        write.screen = 0;
        {
            let pass = self.open()?;
            rel.resident_in(pass);
            for &(_, r, ix) in rest {
                ix.map_or(r, |ix| ix.of(r)).resident_in(pass);
            }
        }
        let n = left.room() + 1;
        let dummy = self.dummy.buffer.clone();
        let mut job = Job::new(JobKind::Witness, format, stage);
        job.write = Some(write);
        job.probes = Some(probes);
        let mut caps = vec![(0, CAP_IN), (1, CAP_IN)];
        caps.extend((0..rest.len()).map(|k| (2, CAP_IN.past((std::mem::size_of::<Stage>() * k) as u64))));
        job.reads_placed(left, &caps);
        job.makes_placed(out, &[(0, CAP_OUT), (1, CAP_OUT)]);
        job.m = (Some(job.desc.totals), left.room());
        job.desc.l = left.reach_by(&mut job, false)?;
        job.desc.r = job.reach(&a, 0, false)?;
        job.desc.totals = job.reach(&totals.buffer.buffer, 0, true)?;
        job.desc.o = out.reach_by(&mut job, true)?;
        job.desc.f = job.reach(&fbuf, 0, false)?;
        for k in 0..WITNESS_PROBES {
            job.desc.w[k] = job.reach(bufs.get(k).unwrap_or(&dummy), 0, false)?;
        }
        // A semi-join's scatter reads no arena, counters or names.
        job.desc.a = job.reach(&dummy, 0, false)?;
        job.desc.counters = job.reach(&dummy, 0, false)?;
        job.desc.d = job.reach(&dummy, 0, false)?;
        (job.n, job.count, job.scatter) = (n, n, n);
        job.m.0 = Some(job.desc.totals + 4 * in_slot as u64);
        let (l, o) = (left.key(job.desc.l), out.key(job.desc.o));
        job.order(&[l, job.desc.r, job.desc.w[0], job.desc.w[1], job.desc.w[2], job.desc.totals], &[o, job.desc.totals]);
        self.queue(job)
    }

    /// A mode 10 stage (`ROWTEXT`) from `left` into `out`: `row_text` (a SIMD group a row, its symbols' bytes read
    /// through the batch's atoms directory, decision 0478 step 2e), its automaton `auto` (a replacement's or a test's,
    /// in the automata block), and `id`: a test's (its verdicts) or the functions' (their texts, `ROW_KEPT`) in the
    /// batch's rolling cache; then its rows scattered.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn row_stage(&mut self, mut stage: Stage, left: &Table, auto: Option<(&DevBytes, [u32; 4])>, id: u32,
                                              in_slot: usize, out_slot: usize, totals: &Words, counters: &Words, counter: Option<usize>, out: &Table) -> Result<(), Error> {
        stage.cap_in = left.room() as u32;
        stage.cap_out = out.room() as u32;
        stage.in_slot = in_slot as u32;
        stage.out_slot = out_slot as u32;
        stage.counter = counter.map_or(NONE, |c| c as u32);
        stage.nslots = left.slots as u32;
        let n = left.room() + 1;
        let ngroups = left.room().clamp(1, 1024);
        // [scratch states, states, classes, start]: a replacement's scratch holds its larger automaton's states, two
        // rows a lane.
        let [kept, states, classes, start] = auto.map_or([1, 0, 0, 0], |a| a.1);
        let work = Held::new(ngroups * 2 * simd_group() * kept.max(1) as usize)?;
        // Its values a row: placed with its input where that is placed (its room its input's).
        let values = Held::new(if left.placed() { 16 } else { 8 * n })?;
        let arena = self.arena()?.buffer();
        let (g, g_at): (Buffer, u64) = auto.map_or_else(|| (self.dummy.buffer.clone(), 0), |a| (a.0.buffer.clone(), a.0.at));
        let atoms = self.atoms.clone().ok_or(Error::BackendCapacity)?;
        let cache = self.kept.clone().ok_or(Error::BackendCapacity)?;
        atoms.resident_in(self.open()?);
        let mut job = Job::new(JobKind::Row, 0, stage);
        job.reads_placed(left, &[(0, CAP_IN)]);
        job.makes_placed(out, &[(0, CAP_OUT)]);
        job.v_place = left.placed();
        job.p = Some([ngroups as u32, kept, states, classes, start, id, 0, 0].iter().flat_map(|w| w.to_ne_bytes()).collect());
        job.desc.l = left.reach_by(&mut job, false)?;
        job.desc.a = job.reach(&arena, 0, true)?;
        job.desc.totals = job.reach(&totals.buffer.buffer, 0, true)?;
        job.desc.counters = job.reach(&counters.buffer.buffer, 0, true)?;
        job.desc.v = job.reach(&values.buffer, 0, true)?;
        job.desc.g = job.reach(&g, g_at, false)?;
        job.desc.s = job.reach(&work.buffer, 0, true)?;
        job.desc.d = job.reach(&atoms.dir.0, atoms.dir.1, false)?;
        job.desc.vc = job.reach(&cache.cache.0, cache.cache.1, true)?;
        job.desc.o = out.reach_by(&mut job, true)?;
        // A SIMD group a row, a threadgroup each: its count's grid is its groups.
        (job.n, job.count, job.scatter) = (n, ngroups, n);
        job.m = (Some(job.desc.totals + 4 * in_slot as u64), left.room());
        job.own.extend([work, values]);
        let (l, o) = (left.key(job.desc.l), out.key(job.desc.o));
        job.order(&[l, job.desc.totals], &[o, job.desc.totals]);
        self.queue(job)
    }

    /// `dedup_less`, its rows made distinct and written in `key`'s order (t58 increment A): sorted by their keys in
    /// the level's wide sort, each kept where its key is not the one before it's. The order is the head's first, so
    /// the host takes the rows as they lie and sorts nothing.
    #[allow(clippy::too_many_arguments)]
    /// Less `held`'s rows where given: a row kept where its bit is clear, and its bit set (`HeldBits`).
    pub(in crate::code_rule_dsl) fn dedup_sorted(&mut self, table: &Table, cols: &[usize], in_slot: usize, out_slot: usize, totals: &Words, out: &Table,
                                                 held: Option<HeldBits<'_>>, key: &SortKey) -> Result<(), Error> {
        self.dedup_with(table, cols, in_slot, out_slot, totals, out, None, Some((key, held)))
    }

    /// `table`'s distinct rows over slots `cols` (at most 16) into `out` (a column per slot of `cols`, in that order,
    /// the same capacity), their count into `totals[out_slot]`; the first of equal rows is kept, and the order kept.
    /// Queued in the batch's level, as a stage is: a stratum's heads' dedups launch together.
    pub(in crate::code_rule_dsl) fn dedup(&mut self, table: &Table, cols: &[usize], in_slot: usize, out_slot: usize, totals: &Words, out: &Table) -> Result<(), Error> {
        self.dedup_less(table, cols, in_slot, out_slot, totals, out, None)
    }

    /// `dedup`, less the rows relation `held.0` holds (decision 0567): read through its map (`held.1`, every row of
    /// the relation placed in it, `set_map`), its columns `table`'s `cols` in order. Its rows out are placed in the map
    /// as the rows they become past the relation's rows as the device holds them (`append_live` writes them there).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn dedup_less(&mut self, table: &Table, cols: &[usize], in_slot: usize, out_slot: usize, totals: &Words, out: &Table,
                                               held: Option<(&DevRel, LatMap<'_>)>) -> Result<(), Error> {
        self.dedup_with(table, cols, in_slot, out_slot, totals, out, held, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn dedup_with(&mut self, table: &Table, cols: &[usize], in_slot: usize, out_slot: usize, totals: &Words, out: &Table,
                  held: Option<(&DevRel, LatMap<'_>)>, sorted: Option<(&SortKey, Option<HeldBits<'_>>)>) -> Result<(), Error> {
        let (key, bits) = (sorted.map(|s| s.0), sorted.and_then(|s| s.1));
        assert!(cols.len() <= 16 && out.room() == table.room());
        let places = (2 * table.room()).next_power_of_two().max(16);
        let mut d = [0u32; 32];
        d[0] = table.room() as u32;
        d[1] = in_slot as u32;
        d[2] = out_slot as u32;
        d[3] = cols.len() as u32;
        d[4] = (places - 1) as u32;
        for (i, &c) in cols.iter().enumerate() {
            d[5 + i] = c as u32;
        }
        d[21] = out.room() as u32;
        d[22] = places as u32;
        let hb = match held {
            Some((r, map)) => {
                assert!(map.places.is_power_of_two() && cols.len() == r.cols && cols.len() <= 8);
                (d[23], d[24]) = (1, (map.places - 1) as u32);
                for (w, v) in [(26, r.at), (28, r.rows_at()), (30, map.at as u64)] {
                    (d[w], d[w + 1]) = (v as u32, (v >> 32) as u32);
                }
                Some((r, bound(map.home)?))
            }
            None => None,
        };
        // Sorted (t58 increment A): its keys in the level's wide sort, no places.
        if let Some(k) = key {
            d[25] = 1;
            // Its rows decoded from their keys (`head_row`): every column a part, and none held (no row to probe).
            assert!(k.parts.len() <= 16 && k.bits <= 64 && k.parts.iter().all(|&(c, _, _)| (c as usize) < cols.len()));
            assert!((0..cols.len() as u32).all(|c| k.parts.iter().any(|p| p.0 == c)), "a sorted dedup's every column in its key");
            assert!(held.is_none(), "a sorted dedup less a map's rows");
        }
        // Less a bitmap's rows (`HeldBits`): its index bits in the map's mask word, its place in its block.
        let bb = match bits {
            Some(hb) => {
                let k = key.expect("a bitmap's dedup sorted");
                assert!(k.index_bits() == hb.bits && hb.bits <= HELD_BITS_MAX);
                (d[23], d[24]) = (1, hb.bits);
                (d[30], d[31]) = (hb.at as u32, (hb.at as u64 >> 32) as u32);
                Some(bound(hb.home)?)
            }
            None => None,
        };
        // Its places placed with its input where that is placed: twice its rows, a power of two (5d); and as many
        // again past them, each row's place (`dedup_claim_one`'s, its mark's). A sorted one has none.
        let p = Held::new(if table.placed() || key.is_some() { 16 } else { 2 * 4 * places })?;
        let n = table.room() + 1;
        let mut job = Job::new(JobKind::Dedup, 0, Stage::default());
        job.dedup = Some(d);
        job.reads_placed(table, &[(3, DD_CAP)]);
        job.makes_placed(out, &[(3, DD_OCAP)]);
        job.h_place = (table.placed() && key.is_none()).then_some(HPlace { block: 3, places: Some(DD_PLACES), mask: DD_MASK, arrays: 2 });
        job.desc.l = table.reach_by(&mut job, false)?;
        job.desc.totals = job.reach(&totals.buffer.buffer, 0, true)?;
        job.desc.h = job.reach(&p.buffer, 0, true)?;
        if let Some(k) = key {
            job.p = Some(k.words().iter().flat_map(|x| x.to_ne_bytes()).collect());
            (job.wide, job.key_bits, job.presorted) = (table.room(), k.bits, k.presorted);
        }
        job.desc.o = out.reach_by(&mut job, true)?;
        let (mut reads, mut writes) = (vec![table.key(job.desc.l), job.desc.totals], vec![out.key(job.desc.o), job.desc.totals]);
        if let Some((r, hb)) = hb {
            job.desc.s = job.reach(&r.buffer.buffer, 0, false)?;
            job.desc.vc = job.reach(&hb.buffer, 0, true)?;
            reads.push(job.desc.s);
            writes.push(job.desc.vc);
            job.own.push(hb);
        }
        if let Some(bb) = bb {
            job.desc.vc = job.reach(&bb.buffer, 0, true)?;
            writes.push(job.desc.vc);
            job.own.push(bb);
        }
        // Its clear a thread for 4 places (`dedup_clear_one`); a sorted one's keys instead (`dedup_key_one`).
        (job.n, job.count, job.scatter, job.clear, job.claim) = if key.is_some() { (n, n, n, 0, 0) } else { (n, n, n, places / 4, table.room()) };
        job.m = (Some(job.desc.totals + 4 * in_slot as u64), table.room());
        job.own.push(p);
        job.order(&reads, &writes);
        self.queue(job)
    }

    /// `n` rows of `src` into `dst` at row `dst_off`, column k of `dst` as `cols[k]` says (a column of `src`, or a
    /// constant): a head's rows into its union, a union's new rows onto its relation.
    pub(in crate::code_rule_dsl) fn copy_rows(&mut self, src: Rows<'_>, n: usize, cols: &[Col], dst: Rows<'_>, dst_off: usize) -> Result<(), Error> {
        self.copy(src, n, None, cols, dst, dst_off, None)
    }

    /// A head's rows written in place (decision 0325 step 4): `src`'s first rows (as many as slot `count` of `totals`
    /// holds, at most `cap`; `cap` of them with no slot) into rows `at..` of `dst`, a relation in its block whose
    /// chunks hold `at + cap` rows (the head's own tail past its rows, decision 0384 step 3). Each column of the head
    /// is a column of `src` or a constant (`cols`). The batch keeps `dst`'s block until its work completes.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn place_rows(&mut self, src: Rows<'_>, cap: usize, count: Option<usize>, totals: &Words, cols: &[Col], dst: DevRel, at: usize) -> Result<(), Error> {
        self.copy(src, cap, count.map(|c| (c, totals)), cols, Rows::Rel(&dst), at, None)?;
        Ok(())
    }

    /// A round's new rows onto a live relation (`DevRel::live`, decision 0603 step 3): `src`'s first rows (as many as slot
    /// `count.0` of `count.1` holds, at most `cap`) past `dst`'s rows as the device claimed them when the copy runs.
    /// The round's end claims them (`round_end`).
    pub(in crate::code_rule_dsl) fn append_live(&mut self, src: Rows<'_>, cap: usize, count: (usize, &Words), cols: &[Col], dst: &DevRel) -> Result<(), Error> {
        assert!(dst.live, "rows appended past a live relation's length");
        self.copy_to(src, cap, Some(count), cols, Rows::Rel(dst), 0, None, true)
    }

    /// `n` rows of `src` (as many as slot `count.0` of `count.1` holds, at most `n`), each column of `dst` a column of
    /// `src` or a constant (`cols`), into `dst`'s rows `dst_off..` (past the word at address `off` too, when given):
    /// `rows_copy`, either side a table or a relation in its block.
    #[allow(clippy::too_many_arguments)]
    fn copy(&mut self, src: Rows<'_>, n: usize, count: Option<(usize, &Words)>, cols: &[Col], dst: Rows<'_>, dst_off: usize, off: Option<u64>) -> Result<(), Error> {
        self.copy_to(src, n, count, cols, dst, dst_off, off, false)
    }

    /// `copy`; `at_len`: past the destination's rows as the device claimed them when it runs too (a live relation's,
    /// `DevRel::live`: its rows at most `rows`, so `n` more lie within its room past `rows - n`).
    #[allow(clippy::too_many_arguments)]
    fn copy_to(&mut self, src: Rows<'_>, n: usize, count: Option<(usize, &Words)>, cols: &[Col], dst: Rows<'_>, dst_off: usize, off: Option<u64>, at_len: bool) -> Result<(), Error> {
        if n == 0 {
            return Ok(());
        }
        assert!(cols.len() <= 16);
        let format = views_format(match src {
            Rows::Rel(r) => Some(r),
            Rows::Table(_) => None,
        });
        let (s, src_rows, src_at, _, s_off) = src.of();
        let (d, dst_rows, _, dst_rows_at, d_off) = dst.of();
        let src_from = match src {
            Rows::Rel(r) => r.from,
            Rows::Table(_) => 0,
        };
        if let Rows::Rel(r) = dst {
            assert_eq!(r.from, 0, "a copy into a relation's rows from its first");
        }
        if let Rows::Rel(r) = dst {
            self.writes(r, "rows copied");
        }
        // A placed source's rows are the device's (its room patched by its size script): past its guess.
        assert!(n <= src_rows.max(1) || matches!(src, Rows::Table(t) if t.placed()), "a copy of {n} rows of a source of {src_rows}");
        // A live relation's room is its gate's (`gate`): the count it copies is within it, its threads are `n`.
        let live = matches!(dst, Rows::Rel(r) if r.live);
        assert!(at_len || live || dst_off + n <= dst_rows, "a copy of {n} rows at {dst_off} into a destination of {dst_rows}");
        let mut p = CopyParams {
            konst: [0; 16], src: [NONE; 16], n: n as u32, ncols: cols.len() as u32, src_stride: src_rows as u32, dst_stride: dst_rows as u32,
            dst_off: dst_off as u32, count_slot: count.map_or(NONE, |c| c.0 as u32), src_from, pad: 0, off: off.unwrap_or(0), src_at, dst_rows_at, v_dir: 0, r_view: 0, flags: u32::from(off.is_some()) | if at_len { 2 } else { 0 },
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
        let totals = count.map_or_else(|| self.dummy.buffer.clone(), |c| c.1.buffer.buffer.clone());
        if let Some(r) = view {
            r.resident_in(self.open()?);
        }
        // Queued in the batch's level, whose copies launch first, all in one launch (`flush`).
        let mut job = Job::new(JobKind::Copy, format, Stage::default());
        job.p = Some(bytes.to_vec());
        // A placed source: its rows' room the device's, its threads its own at most.
        let src_key = match src {
            Rows::Table(t) => {
                job.reads_placed(t, &[(4, CP_N), (4, CP_SRC_STRIDE)]);
                t.place.as_ref().map(|p| p.key)
            }
            Rows::Rel(_) => None,
        };
        // A placed destination (a union, placed by its sources' total before its copies): its stride the device's.
        let dst_key = match dst {
            Rows::Table(t) if t.placed() => {
                job.writes_placed(t, &[(4, CP_DST_STRIDE)]);
                t.place.as_ref().map(|p| p.key)
            }
            _ => None,
        };
        job.desc.l = job.reach(&s, s_off, false)?;
        job.desc.o = job.reach(&d, d_off, true)?;
        job.desc.totals = job.reach(&totals, 0, false)?;
        job.rows = count.map(|(slot, _)| (job.desc.totals + 4 * slot as u64, 0));
        job.count = n;
        job.span = Some(if at_len || live { (job.desc.o, 0, dst_rows.max(dst_off + n)) } else { (job.desc.o, dst_off, dst_off + n) });
        let mut reads = vec![src_key.unwrap_or(job.desc.l)];
        reads.extend(count.map(|_| job.desc.totals));
        reads.extend(off);
        job.order(&reads, &[dst_key.unwrap_or(job.desc.o)]);
        self.queue(job)
    }

    /// The rows of `srcs` end to end into `dst` (a union): each source's first rows, as many as slot `slot` of its
    /// `words` holds (at most `cap`), its columns `cols`, past the rows of the sources before it; their total into
    /// slot `total.1` of `total.0`. Every count is the device's, read where an earlier phase wrote it: the offsets
    /// are a phase (`sum_one`), then the copies. A placed `dst` (`Batch::table`) is placed by the total, its room
    /// the device's (decision 0603 step 5d); past an unplaced one's room, the program stops before the copies (a
    /// gate).
    pub(in crate::code_rule_dsl) fn append_rows(&mut self, srcs: &[(&Table, usize, &Words, usize, &[Col])], dst: &Table, total: (&Words, usize)) -> Result<(), Error> {
        if srcs.is_empty() {
            total.0.set(total.1, 0);
            return Ok(());
        }
        assert!(dst.placed() || srcs.iter().map(|s| s.1).sum::<usize>() <= dst.cap, "a union's room for its sources' rows");
        let offs = Held::new(4 * srcs.len())?;
        let mut job = Job::new(JobKind::Sum, 0, Stage::default());
        let oa = job.reach(&offs.buffer, 0, true)?;
        let ta = job.reach(&total.0.buffer.buffer, 0, true)?;
        let mut p: Vec<u64> = vec![srcs.len() as u64, ta + 4 * total.1 as u64];
        let mut reads = Vec::new();
        for (i, (_, _, w, slot, _)) in srcs.iter().enumerate() {
            let wa = job.reach(&w.buffer.buffer, 0, false)?;
            reads.push(wa);
            p.extend([wa + 4 * *slot as u64, oa + 4 * i as u64]);
        }
        job.p = Some(as_bytes(&p).to_vec());
        job.count = 1;
        job.order(&reads, &[oa, ta]);
        self.queue(job)?;
        // A placed source's rows are the device's: past an unplaced destination's room, the program stops before the
        // copies. A placed one is placed by them.
        if !dst.placed() && srcs.iter().any(|s| s.0.placed()) {
            self.gate(&[(At::Word(total.0, total.1), None, dst.cap)])?;
        }
        if dst.placed() {
            // Placed by its sources' total, at the start of the level after the sum's (its copies' level).
            let mut place = Job::new(JobKind::Place, 0, Stage::default());
            let tw = place.reach(&total.0.buffer.buffer, 0, false)?;
            place.makes_placed(dst, &[]);
            place.o_rows = Some(OutRows::Word(tw + 4 * total.1 as u64));
            place.order(&[tw], &[]);
            self.queue(place)?;
        }
        for (i, (src, cap, w, slot, cols)) in srcs.iter().enumerate() {
            self.copy(Rows::Table(src), *cap, Some((*slot, *w)), cols, Rows::Table(dst), 0, Some(oa + 4 * i as u64))?;
        }
        self.program.hold.push(offs);
        Ok(())
    }

    /// A gate (decision 0540's rerun, decision 0603 point 4): the program stops here when a check's word (and its
    /// second, when given: a relation's rows and a round's new rows onto it) holds more than its `cap`, before any later phase reads the short table (`gate_one`). `finish` then says it
    /// stopped (`Done::stopped`), and the work after the gate is left undone: the caller grows the tables and
    /// encodes the work again.
    pub(in crate::code_rule_dsl) fn gate(&mut self, checks: &[(At<'_>, Option<At<'_>>, usize)]) -> Result<(), Error> {
        self.gate_rooms(checks, Vec::new())
    }

    /// `gate`, where check `t` of each of `rooms` is of a room mapped as its rows reach it (decision 0603's fewer
    /// reruns): past its cap the program parks after the gate, the host grows the room (`RoomGrow`), writes its cap
    /// again, re-points what the program reaches of the blocks it grew (`Kit::grow_rooms`), and goes on from the phase
    /// after the gate. A check past a cap of no room stops the program, as `gate`'s do.
    pub(in crate::code_rule_dsl) fn gate_rooms(&mut self, checks: &[(At<'_>, Option<At<'_>>, usize)], rooms: Vec<(usize, RoomGrow)>) -> Result<(), Error> {
        if checks.is_empty() {
            return Ok(());
        }
        let mut job = Job::new(JobKind::Gate, 0, Stage::default());
        let mut p: Vec<u64> = vec![checks.len() as u64, 0];
        let mut reads = Vec::new();
        for (at, plus, cap) in checks {
            let wa = at.reach(&mut job, false)?;
            let pa = match plus {
                Some(b) => b.reach(&mut job, false)?,
                None => wa,
            };
            reads.extend([wa, pa]);
            let cap = u64::try_from(*cap).ok().filter(|&c| c <= u64::from(u32::MAX)).unwrap_or(u64::from(u32::MAX));
            // Check `t` after the gate's two words, three words a check.
            let room = rooms.iter().any(|r| r.0 == p.len() / 3);
            p.extend([wa, pa, cap | u64::from(plus.is_some()) << 32 | u64::from(room) << 33]);
        }
        job.p = Some(as_bytes(&p).to_vec());
        job.rooms = rooms;
        job.count = checks.len();
        job.order(&reads, &[]);
        // Alone in its level: everything queued before it runs before it (an index queued before a gate is built
        // whether it stops or not), and nothing queued after it runs before it.
        self.flush()?;
        self.queue(job)?;
        self.flush()?;
        self.gated = true;
        Ok(())
    }

    /// The phase the next level flushed begins (`Program::phase`): a round's first, which a round's end jumps to.
    pub(in crate::code_rule_dsl) fn next_phase(&mut self) -> Result<u32, Error> {
        self.flush()?;
        Ok(self.program.phases.len() as u32)
    }

    /// A round's end (decision 0603 step 3, `round_one`), alone in its level: `ops` in order (its new rows claimed,
    /// its next deltas' lengths set, its words emptied for the next round, its counts summed and logged), the round
    /// counted in slot 0 of `round`; then the round after, from phase `next` (`next_phase`; none: the phase after its
    /// own). The program ends when no `RoundOp::Any` word is non-zero (`Done::ended`) and fails past round `max`
    /// (`Done::past_rounds`). Its rounds go on across its launches: a launch's GPU time is its workers' budgets'
    /// (decision 0603 point 5).
    pub(in crate::code_rule_dsl) fn round_end(&mut self, ops: &[RoundOp<'_>], round: &Words, max: u32, next: Option<u32>) -> Result<(), Error> {
        // Alone in its level: the round's work all before it, the next round's after. Its phase is the next one
        // (`next_phase`); none given, the round after goes on from the phase after it.
        self.flush()?;
        let next = next.unwrap_or(self.program.phases.len() as u32 + 1);
        let mut job = Job::new(JobKind::Round, 0, Stage::default());
        let ra = job.reach(&round.buffer.buffer, 0, true)?;
        let mut p: Vec<u64> = vec![ra, u64::from(ops.len() as u32) | u64::from(max) << 32, u64::from(next)];
        let (mut reads, mut writes) = (Vec::new(), vec![ra]);
        let word = |job: &mut Job, at: &At<'_>, write: bool| at.reach(job, write);
        for op in ops {
            let (to, from, code, n) = match op {
                RoundOp::Add(to, from) => (word(&mut job, to, true)?, word(&mut job, from, false)?, 0u32, 1usize),
                RoundOp::Set(to, from) => (word(&mut job, to, true)?, word(&mut job, from, false)?, 1, 1),
                RoundOp::Zero(w, at, n) => {
                    assert!(at + n <= w.len, "words emptied past their end");
                    (job.reach(&w.buffer.buffer, 4 * *at as u64, true)?, 0, 2, *n)
                }
                RoundOp::Log(w, from) => (job.reach(&w.buffer.buffer, 0, true)?, word(&mut job, from, false)?, 3, w.len),
                RoundOp::Any(from) => (0, word(&mut job, from, false)?, 4, 1),
                RoundOp::Sum(to, from) => {
                    assert!(from.len <= to.len, "counts summed into fewer words");
                    (job.reach(&to.buffer.buffer, 0, true)?, job.reach(&from.buffer.buffer, 0, false)?, 5, from.len)
                }
            };
            if to != 0 {
                writes.push(to);
            }
            if from != 0 {
                reads.push(from);
            }
            p.extend([to, from, u64::from(code) | (n as u64) << 32]);
        }
        // The arena's top emptied: a round's counts are its own (decision 0603 step 5).
        let top = self.program.top()?;
        p.extend([top, 0, 2 | 1 << 32]);
        p[1] += 1;
        writes.push(top);
        job.p = Some(as_bytes(&p).to_vec());
        job.count = 1;
        job.order(&reads, &writes);
        self.queue(job)?;
        self.flush()?;
        Ok(())
    }

    /// A lattice head's key map made or grown before a program's rounds fold into it (`MapFirst::Ready`): emptied, and
    /// every key of the old one `old` placed in it again (`lat_hclear_one`, `lat_rehash_one`). `r`: the relation its
    /// slots' rows are of, its key its first `nkeys` columns.
    /// A set's map (`dedup_less`) emptied and every row of `r`'s first `n` placed in it, a key of its own (code 4 of
    /// `lat_rehash_one`): a program's first work, so a round its gate stopped leaves nothing in it.
    pub(in crate::code_rule_dsl) fn set_map(&mut self, r: &DevRel, n: usize, map: LatMap<'_>) -> Result<(), Error> {
        assert!(map.places.is_power_of_two() && 2 * n <= map.places && r.cols <= 8);
        let p = LatParams { top: 0, n: n as u32, had: 0, nkeys: r.cols as u32, code: 4, after: NONE, pmask: 0, hmask: (map.places - 1) as u32, level: 0, omask: 0,
                            counted: 0, r_at: r.at, r_rows_at: r.rows_at(), h_at: map.at as u64, h0_at: 0 };
        let hb = bound(map.home)?;
        let mut job = Job::new(JobKind::Lattice, 0, Stage::default());
        job.desc.s = job.reach(&r.buffer.buffer, 0, false)?;
        job.desc.vc = job.reach(&hb.buffer, 0, true)?;
        for d in [&mut job.desc.l, &mut job.desc.a, &mut job.desc.totals, &mut job.desc.h, &mut job.desc.o, &mut job.desc.counters, &mut job.desc.v, &mut job.desc.g] {
            *d = job.desc.s;
        }
        job.p = Some(lat_bytes(&p).to_vec());
        (job.count, job.clear, job.lattice) = (0, 0, [map.places, n]);
        job.order(&[job.desc.s], &[job.desc.vc]);
        job.own.push(hb);
        self.queue(job)
    }

    /// A set head's held bitmap (`HeldBits`) emptied and every row of `r`'s first `n` set in it, its bit at its index
    /// under `key` (code 6 of `lat_hclear_one` and `lat_rehash_one`): a program's first work, as `set_map`'s.
    pub(in crate::code_rule_dsl) fn set_bits(&mut self, r: &DevRel, n: usize, bits: HeldBits<'_>, key: &SortKey) -> Result<(), Error> {
        let words = HeldBits::words_for(bits.bits);
        assert!(key.index_bits() == bits.bits && bits.bits <= HELD_BITS_MAX && r.cols <= 8 && key.parts.iter().all(|p| (p.0 as usize) < r.cols));
        let p = LatParams { top: 0, n: n as u32, had: 0, nkeys: r.cols as u32, code: 6, after: NONE, pmask: 0, hmask: (words - 1) as u32, level: 0, omask: 0,
                            counted: 0, r_at: r.at, r_rows_at: r.rows_at(), h_at: bits.at as u64, h0_at: 0 };
        let hb = bound(bits.home)?;
        let mut job = Job::new(JobKind::Lattice, 0, Stage::default());
        job.desc.s = job.reach(&r.buffer.buffer, 0, false)?;
        job.desc.vc = job.reach(&hb.buffer, 0, true)?;
        for d in [&mut job.desc.l, &mut job.desc.a, &mut job.desc.totals, &mut job.desc.h, &mut job.desc.o, &mut job.desc.counters, &mut job.desc.v, &mut job.desc.g] {
            *d = job.desc.s;
        }
        // Its key's `HeadKeyP` past its `LatP`, for its rows' indexes.
        let mut bytes = lat_bytes(&p).to_vec();
        bytes.extend(key.words().iter().flat_map(|x| x.to_ne_bytes()));
        job.p = Some(bytes);
        (job.count, job.clear, job.lattice) = (0, 0, [words, n]);
        job.order(&[job.desc.s], &[job.desc.vc]);
        job.own.push(hb);
        self.queue(job)
    }

    pub(in crate::code_rule_dsl) fn lattice_grow(&mut self, r: &DevRel, nkeys: usize, map: LatMap<'_>, old: Option<LatMap<'_>>) -> Result<(), Error> {
        assert!(map.places.is_power_of_two() && old.is_none_or(|o| o.places.is_power_of_two() && o.places <= map.places));
        let (omask, h0_at) = old.map_or((0, 0), |o| ((o.places - 1) as u32, o.at as u64));
        let p = LatParams { top: 0, n: 0, had: 0, nkeys: nkeys as u32, code: 0, after: NONE, pmask: 0, hmask: (map.places - 1) as u32, level: 0, omask,
                            counted: 0, r_at: r.at, r_rows_at: r.rows_at(), h_at: map.at as u64, h0_at };
        let hb = bound(map.home)?;
        let ob = match old {
            Some(o) => bound(o.home)?,
            None => bound(map.home)?,
        };
        let mut job = Job::new(JobKind::Lattice, 0, Stage::default());
        job.desc.s = job.reach(&r.buffer.buffer, 0, false)?;
        job.desc.vc = job.reach(&hb.buffer, 0, true)?;
        job.desc.g = job.reach(&ob.buffer, 0, false)?;
        for d in [&mut job.desc.l, &mut job.desc.a, &mut job.desc.totals, &mut job.desc.h, &mut job.desc.o, &mut job.desc.counters, &mut job.desc.v] {
            *d = job.desc.g;
        }
        job.p = Some(lat_bytes(&p).to_vec());
        (job.count, job.clear, job.lattice) = (0, 0, [map.places, old.map_or(0, |o| o.places)]);
        job.order(&[job.desc.s, job.desc.g], &[job.desc.vc]);
        job.own.extend([hb, ob]);
        self.queue(job)
    }

    /// Sets slots of `table` in place (`stage`'s binds: program columns of the binding's slots, or parameters), queued
    /// in the batch's level.
    pub(in crate::code_rule_dsl) fn map(&mut self, mut stage: Stage, table: &Table, in_slot: usize, totals: &Words) -> Result<(), Error> {
        stage.cap_in = table.room() as u32;
        stage.cap_out = table.room() as u32;
        stage.in_slot = in_slot as u32;
        stage.nslots = table.slots as u32;
        let mut job = Job::new(JobKind::Map, 0, stage);
        // A placed table mapped in place: read where its producer placed it.
        job.reads_placed(table, &[(0, CAP_IN), (0, CAP_OUT)]);
        job.o_reads = table.placed();
        job.desc.o = table.reach_by(&mut job, true)?;
        job.desc.totals = job.reach(&totals.buffer.buffer, 0, false)?;
        job.count = table.room();
        let o = table.key(job.desc.o);
        job.order(&[o], &[o]);
        self.queue(job)
    }

    /// Queues `job` in the batch's level: launched with every job of its kernel at the level's end (`flush`). A job
    /// that reads or writes what a queued one writes, or writes what one reads, ends the level first: each sees the
    /// writes of the work before it, as if each were launched alone, in order.
    ///
    /// A level launches its copies first, then its indexes, then the rest (`JobKind::phase`): a job of a later phase
    /// than a queued one sees its writes in the same level, and copies into one table's rows apart are one launch.
    fn queue(&mut self, job: Job) -> Result<(), Error> {
        let after = self.queued.iter().any(|q| {
            if q.kind.phase() < job.kind.phase() {
                return false;
            }
            let apart = q.kind == JobKind::Copy && job.kind == JobKind::Copy && match (q.span, job.span) {
                (Some((a, a0, a1)), Some((b, b0, b1))) => a != b || a1 <= b0 || b1 <= a0,
                _ => false,
            };
            let waw = job.writes.iter().any(|a| q.writes.contains(a)) && !apart;
            waw || job.reads.iter().any(|a| q.writes.contains(a)) || job.writes.iter().any(|a| q.reads.contains(a))
        });
        if after {
            self.flush()?;
        }
        self.queued.push(job);
        Ok(())
    }

    /// Ends the level: the jobs queued (`queue`) into the batch's program (`Program`, decision 0603) as its phases, in
    /// the level's order: the copies; the indexes (their keys, the sort's passes, their places); the dedups' clears
    /// and the maps; the dedups' claims; the counts; the scan of the level's counts (each job's from its `base`); the
    /// scatters. Nothing launches here: the program runs as one launch before the next launch made at once, and at
    /// the commit (`run_program`).
    fn flush(&mut self) -> Result<(), Error> {
        if self.queued.is_empty() {
            return Ok(());
        }
        let mut jobs = std::mem::take(&mut self.queued);
        // The level's sorted dedups (t58 increment A): one wide sort of all their keys, each one's under its rank in
        // the level, above the widest key's bits (`HeadKeyP::rank_shift`, `rank`).
        let wide_jobs: Vec<usize> = (0..jobs.len()).filter(|&i| jobs[i].wide > 0).collect();
        let wide_bits = wide_jobs.iter().map(|&i| jobs[i].key_bits).max().unwrap_or(0);
        let rank_bits = usize::BITS - wide_jobs.len().saturating_sub(1).leading_zeros();
        if wide_bits + rank_bits > 64 {
            return Err(Error::BackendCapacity);
        }
        // None where every one's rows come sorted (`SortKey::presorted`): their keys lie in their order already.
        let wide_passes = if wide_jobs.iter().all(|&i| jobs[i].presorted) { 0 } else { (wide_bits + rank_bits).div_ceil(8).max(1) as usize };
        for (rank, &i) in wide_jobs.iter().enumerate() {
            let shift = if rank_bits == 0 { 64 } else { wide_bits };
            let p = jobs[i].p.as_mut().expect("a sorted dedup's key");
            p[8..12].copy_from_slice(&shift.to_ne_bytes());
            p[12..16].copy_from_slice(&(rank as u32).to_ne_bytes());
            // Its sorted keys in the region's second pair after an odd number of passes (`HeadKeyP::odd`).
            p[HK_ODD.0 as usize..HK_ODD.0 as usize + 4].copy_from_slice(&((wide_passes % 2) as u32).to_ne_bytes());
        }
        // The parameter blocks, each at 16 bytes, in one buffer.
        let mut params: Vec<u8> = Vec::new();
        let mut put = |b: &[u8]| -> u64 {
            let at = params.len() as u64;
            // copy: not block data: the jobs' parameter blocks, laid end to end.
            params.extend_from_slice(b);
            params.resize(params.len().next_multiple_of(16), 0);
            at
        };
        let placed: Vec<[Option<u64>; 5]> = jobs
            .iter()
            .map(|j| {
                [
                    Some(put(j.stage.bytes())),
                    j.write.as_ref().map(|w| put(w.bytes())),
                    j.probes.as_ref().map(|p| put(&p.iter().flat_map(|s| s.bytes().iter().copied()).collect::<Vec<u8>>())),
                    j.dedup.as_ref().map(|d| put(&d.iter().flat_map(|w| w.to_ne_bytes()).collect::<Vec<u8>>())),
                    j.p.as_ref().map(|b| put(b)),
                ]
            })
            .collect();
        // Each job's parameter words that hold an address it reached, and its rooms and live index, where a room is
        // mapped as reached: re-pointed when a block they reach is wrapped again (`Kit::grow_rooms`).
        let rooms = jobs.iter().any(|j| !j.rooms.is_empty() || j.live_ix.is_some()) || !self.program.rooms.gates.is_empty();
        let mut relocs: Vec<u64> = Vec::new();
        if rooms {
            for (j, at) in jobs.iter().zip(&placed) {
                let lens = [
                    Some(j.stage.bytes().len()),
                    j.write.as_ref().map(|w| w.bytes().len()),
                    j.probes.as_ref().map(|p| p.iter().map(|s| s.bytes().len()).sum()),
                    j.dedup.as_ref().map(|d| 4 * d.len()),
                    j.p.as_ref().map(Vec::len),
                ];
                for (a, l) in at.iter().zip(lens) {
                    let (Some(a), Some(l)) = (a, l) else { continue };
                    for o in (*a as usize..*a as usize + l).step_by(8).filter(|o| o + 8 <= *a as usize + l) {
                        let w = u64::from_ne_bytes(params[o..o + 8].try_into().expect("a word"));
                        if j.reached.contains(&w) {
                            relocs.push(o as u64);
                        }
                    }
                }
            }
        }
        let params = self.block(&params)?;
        let pa = address(&params.buffer)?;
        let prog = &mut self.program;
        prog.uses.push((params.buffer.clone(), false));
        prog.rooms.relocs.extend(relocs.into_iter().map(|o| (params.buffer.clone(), o)));
        // Its live indexes mapped as reached, and their keys at most (their bounds).
        let mut level_live: Vec<(usize, usize)> = Vec::new();
        for (j, at) in jobs.iter_mut().zip(&placed) {
            let pat = at[4].unwrap_or(0);
            for (t, grow) in j.rooms.drain(..) {
                // Check `t`'s cap: after the gate's two words, three words a check, its third.
                prog.rooms.gates.push(GateRoom { params: params.buffer.clone(), at: pat + 16 + 24 * t as u64 + 16, grow });
            }
            if let Some((home, rel, index, rows)) = j.live_ix.take() {
                level_live.push((prog.rooms.live_ix.len(), j.count));
                prog.rooms.live_ix.push(LiveIx { home, rel, index, params: params.buffer.clone(), at: pat, rows: std::cell::Cell::new(rows) });
            }
        }
        // The level's indexes' keys, sorted at once: each index's buckets from its first (`kb`), its rows' keys from
        // its first (`eb`), each key with its place; the sort's two pairs of buffers and its histograms.
        let (buckets, ix_rows) = jobs.iter().filter(|j| j.kind == JobKind::Index).fold((0usize, 0usize), |(b, e), j| (b + j.buckets, e + j.count));
        if u32::try_from(buckets).is_err() || u32::try_from(ix_rows).is_err() {
            return Err(Error::BackendCapacity);
        }
        // Where a live index of the level is mapped as reached, its keys at most are its bound's: the sort's buffers
        // are address space mapped for its indexes' rows as mapped (`SortRoom`), as the device's arena is.
        let fixed = ix_rows - level_live.iter().map(|l| l.1).sum::<usize>();
        let rows_now = fixed + level_live.iter().map(|l| prog.rooms.live_ix[l.0].rows.get() as usize).sum::<usize>();
        let mut sort_room = (!level_live.is_empty()).then(|| SortRoom { reserved: Vec::new(), hist: None, fixed, live: level_live.iter().map(|l| l.0).collect() });
        let sort = if ix_rows > 0 {
            let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
            let tiles = ix_rows.div_ceil(SORT_TILE).max(1);
            let mut one = |bytes: usize, now: usize, hist: bool| -> Result<Held, Error> {
                let Some(room) = &mut sort_room else { return Held::new(bytes).map_err(|_| Error::BackendCapacity) };
                let r = std::sync::Arc::new(rerail_device::device::reserve(device, bytes, now.min(bytes), false).map_err(|_| Error::BackendCapacity)?);
                if hist {
                    room.hist = Some(r.clone());
                } else {
                    room.reserved.push(r.clone());
                }
                Ok(Held::kept(r.buffer(), Box::new(r)))
            };
            let (a, b) = ([one(4 * ix_rows, 4 * rows_now, false)?, one(4 * ix_rows, 4 * rows_now, false)?], [one(4 * ix_rows, 4 * rows_now, false)?, one(4 * ix_rows, 4 * rows_now, false)?]);
            let hist = one(4 * (256 * tiles + 1), sort_hist_bytes(rows_now), true)?;
            Some((a, b, hist, tiles))
        } else {
            None
        };
        // Every index's buckets in one span; whole bytes, as the sort's passes are a byte each.
        let bits = (usize::BITS - buckets.saturating_sub(1).leading_zeros()).max(8).div_ceil(8) * 8;
        let passes = bits / 8;
        // Where the sorted keys and places end: in the first pair after an even number of passes.
        let sorted = sort.as_ref().map(|(a, b, ..)| if passes % 2 == 0 { a } else { b });
        // The wide sort's (t58 increment A): its sorted dedups' keys end to end, 64 bits each (a key its row), in a
        // region of the arena its level's size script claims from the rows counted (`WideAt`); as many passes as its
        // keys' and ranks' bits take, a byte each. Its rows as guessed: its items and its arena's bytes.
        let wide_rows: usize = wide_jobs.iter().map(|&i| jobs[i].wide).sum();
        let wide = (!wide_jobs.is_empty()).then(|| wide_rows.div_ceil(SORT_TILE).max(1));
        let (mut kb, mut eb, mut total) = (0usize, 0usize, 0usize);
        let first_job = prog.descs.len();
        for (j, at) in jobs.iter().zip(&placed) {
            let mut d = j.desc;
            let s = pa + at[0].expect("a job's stage");
            d.stage = s;
            d.ws = at[1].map_or(s, |w| pa + w);
            d.bs = at[2].map_or(s, |b| pa + b);
            d.dd = at[3].map_or(s, |b| pa + b);
            d.p = at[4].map_or(s, |b| pa + b);
            // Its counts' first word: the level's size script's (below).
            d.at.base = 0;
            d.at.n = j.n as u32;
            total += j.n;
            if let (JobKind::Index, Some((a, _, _, _)), Some(f)) = (j.kind, &sort, sorted) {
                (d.at.kb, d.at.eb) = (kb as u32, eb as u32);
                (d.h, d.s) = (address(&a[0].buffer)?, address(&a[1].buffer)?);
                (d.v, d.vc) = (address(&f[0].buffer)?, address(&f[1].buffer)?);
                kb += j.buckets;
                eb += j.count;
            }
            prog.descs.push(d);
        }
        if u32::try_from(total + 1).is_err() {
            return Err(Error::BackendCapacity);
        }
        prog.levels += 1;
        let groups = |threads: usize| threads.div_ceil(rerail_accel::gpu::join::GROUP as usize).max(1);
        // A row stage's SIMD groups, a worker's group of them an item.
        let rows_a_group = rerail_accel::gpu::join::GROUP as usize / simd_group();
        // A dedup's clear: its input's rows' count and 4, as its places are its rows' (`dedup_mask`: twice its rows, a
        // power of two, 16 at least), 4 a thread. A fold's, its clear's and its merge's count its mask's word and one.
        let clears: Vec<Option<(u64, u32)>> = jobs.iter().zip(&placed).map(|(j, at)| match (j.kind, j.dedup) {
            (JobKind::Dedup, Some(d)) => Some((j.desc.totals + 4 * u64::from(d[1]), 4)),
            _ => j.h_place.map(|h| match h.places {
                Some(w) => (pa + at[h.block].expect("its places' block") + w.0, 0),
                None => (pa + at[h.block].expect("its places' block") + h.mask.0, 1),
            }),
        }).collect();
        let job_phase = |prog: &mut Program, kinds: &[(JobKind, u32, fn(&Job) -> usize)]| {
            let mut items = Vec::new();
            let mut segs = Vec::new();
            for (i, j) in jobs.iter().enumerate() {
                for &(k, what, threads) in kinds {
                    // A fold's map left as it is has no pass to empty or grow it.
                    // A sorted dedup has no places to clear or claim, and an unsorted one no keys (t58 increment A).
                    if j.kind == k && !(matches!(k, JobKind::Lattice | JobKind::Dedup) && threads(j) == 0) {
                        let n = if what == PK_ROW_TEXT { threads(j).div_ceil(rows_a_group).max(1) } else { groups(threads(j)) };
                        // Its items as many as its rows' count needs, where its kernel's threads past them do nothing;
                        // else all of them (`per` 0).
                        let word = j.rows_word().filter(|_| ROWS_ITEMS.contains(&what)).or_else(|| clears[i].filter(|_| matches!(what, PK_DEDUP_CLEAR | PK_LAT_CLEAR | PK_LAT_MERGE)));
                        let (count, per, plus) = match word {
                            Some((count, plus)) => (count, rerail_accel::gpu::join::GROUP as u32, plus),
                            None => (0, 0, 0),
                        };
                        segs.push(SegAt { first: items.len() as u32, items: n as u32, count, per, plus });
                        items.extend((0..n).map(|g| (what, (first_job + i) as u32, g as u32)));
                    }
                }
            }
            // A phase whose jobs all take every item pushes them as a phase without segments does.
            if segs.iter().all(|g| g.per == 0) {
                segs.clear();
            }
            prog.phase_of(items, segs);
        };
        job_phase(prog, &[(JobKind::Gate, PK_GATE, |j| j.count), (JobKind::Sum, PK_SUM, |j| j.count), (JobKind::Round, PK_ROUND, |j| j.count)]);
        // The level's sizes (decision 0603 step 5): its placed tables read where their producers placed them, its
        // counts' words claimed in the program's arena, each job's first word in its descriptor, the scan's levels there.
        // The sort's histograms' scan, its levels made before the level's size script, which sizes them, and the sort.
        let sort_first = prog.sorts.len();
        let hist_scan = match &sort {
            Some((_, _, hist, tiles)) => Some(prog.scan_levels_sized(ScanData::At(address(&hist.buffer)?), 256 * tiles)),
            None => None,
        };
        let sizes = hist_scan.map(|h| SortSizes { first: sort_first, passes: passes as usize, hist: h, tiles: sort.as_ref().map_or(1, |s| s.3) });
        // The wide sort's passes after the index sort's.
        let wide_first = sort_first + if sort.is_some() { passes as usize } else { 0 };
        let wide_sizes = wide.map(|tiles| SortSizes { first: wide_first, passes: wide_passes, hist: prog.scan_levels_sized(ScanData::Script, 256 * tiles), tiles });
        let sized = level_sizes(prog, &jobs, first_job, total, pa, &placed, sizes, wide_sizes)?;
        job_phase(prog, &[(JobKind::Copy, PK_COPY, |j| j.count)]);
        if let (Some((a, b, hist, tiles)), Some(f)) = (&sort, sorted) {
            job_phase(prog, &[(JobKind::Index, PK_IX_KEY, |j| j.count)]);
            let (mut from, mut to) = (a, b);
            let ha = address(&hist.buffer)?;
            for pass in 0..passes {
                debug_assert_eq!(prog.sorts.len(), sort_first + pass as usize, "the sort's passes where its size script set them");
                let x = prog.sorts.len() as u32;
                prog.sorts.push(SortPass {
                    keys: address(&from[0].buffer)?,
                    values: address(&from[1].buffer)?,
                    hist: ha,
                    out_keys: address(&to[0].buffer)?,
                    out_values: address(&to[1].buffer)?,
                    w: SortWords { n: ix_rows as u32, shift: 8 * pass, tiles: *tiles as u32, wide: 0 },
                    pad2: 0,
                });
                // Its tiles as many as its keys as claimed need (its level's size script's).
                let tile_items = |prog: &mut Program, kind: u32| {
                    prog.phase_of((0..*tiles).map(|g| (kind, x, g as u32)).collect(), vec![SegAt { count: 0, first: 0, items: *tiles as u32, per: 1, plus: 0 }]);
                    prog.seg_late.push((prog.segs.len() - 1, Late::Sort(x as usize, word_at!(SortPass, w.tiles))));
                };
                tile_items(prog, PK_RADIX_HIST);
                prog.scan_sized_phases(hist_scan.expect("the sort's histograms' scan"), 256 * tiles);
                tile_items(prog, PK_RADIX_SCATTER);
                std::mem::swap(&mut from, &mut to);
            }
            debug_assert!(std::ptr::eq(from, f), "the sort ends where `sorted` says");
            prog.rooms.sorts.extend(sort_room.take());
            job_phase(prog, &[(JobKind::Index, PK_IX_PLACE, |j| j.count)]);
            // Its marks (decision 0671), of its rows and buckets placed.
            job_phase(prog, &[(JobKind::Index, PK_IX_MARK, |j| j.count)]);
        }
        job_phase(prog, &[(JobKind::Dedup, PK_DEDUP_CLEAR, |j| j.clear), (JobKind::Dedup, PK_DEDUP_KEY, |j| j.wide), (JobKind::Map, PK_MAP, |j| j.count)]);
        job_phase(prog, &[(JobKind::Dedup, PK_DEDUP_CLAIM, |j| j.claim)]);
        // The wide sort's passes (t58 increment A), as the index sort's: each pass's tiles' histograms, their scan,
        // its tiles' keys and places at their places.
        // Its region and its keys are its size script's (`SortWords::wide`, `n`): its pointers none.
        if let (Some(tiles), Some(ws)) = (wide, wide_sizes) {
            for pass in 0..wide_passes {
                debug_assert_eq!(prog.sorts.len(), wide_first + pass, "the wide sort's passes where its size script set them");
                let x = prog.sorts.len() as u32;
                prog.sorts.push(SortPass { keys: 0, values: 0, hist: 0, out_keys: 0, out_values: 0, w: SortWords { n: 0, shift: 8 * pass as u32, tiles: tiles as u32, wide: 0 }, pad2: 0 });
                for kind in [PK_RADIX_HIST, PK_RADIX_SCATTER] {
                    if kind == PK_RADIX_SCATTER {
                        prog.scan_sized_phases(ws.hist, 256 * tiles);
                    }
                    prog.phase_of((0..tiles).map(|g| (kind, x, g as u32)).collect(), vec![SegAt { count: 0, first: 0, items: tiles as u32, per: 1, plus: 0 }]);
                    prog.seg_late.push((prog.segs.len() - 1, Late::Sort(x as usize, word_at!(SortPass, w.tiles))));
                }
            }
            prog.arena += units(16 * wide_rows.max(1) + 4 * (256 * tiles + 1));
        }
        job_phase(prog, &[
            (JobKind::Stage, PK_STAGE_COUNT, |j| j.count),
            (JobKind::Scan, PK_SCAN_COUNT, |j| j.count),
            (JobKind::Witness, PK_WITNESS_COUNT, |j| j.count),
            (JobKind::Row, PK_ROW_TEXT, |j| j.count),
            (JobKind::Dedup, PK_DEDUP_MARK, |j| j.count),
        ]);
        if total > 0 {
            let level = prog.scan_sized(total);
            for o in &mut prog.script[sized.expect("a level of counts has its sizes")..] {
                if let Late::Pending(k, off) = o.w {
                    o.w = Late::Scan(level + k, off);
                }
            }
            // Its placed tables, from the rows the scan counted (decision 0603 step 5).
            made_sizes(prog, &jobs, first_job, pa, &placed)?;
            job_phase(prog, &[
                (JobKind::Stage, PK_STAGE_SCATTER, |j| j.scatter),
                (JobKind::Witness, PK_STAGE_SCATTER, |j| j.scatter),
                (JobKind::Scan, PK_SCAN_SCATTER, |j| j.scatter),
                (JobKind::Row, PK_ROW_SCATTER, |j| j.scatter),
                (JobKind::Dedup, PK_DEDUP_SCATTER, |j| j.scatter),
            ]);
        }
        job_phase(prog, &[(JobKind::Lattice, PK_LAT_HCLEAR, |j| j.lattice[0])]);
        job_phase(prog, &[(JobKind::Lattice, PK_LAT_REHASH, |j| j.lattice[1])]);
        job_phase(prog, &[(JobKind::Lattice, PK_LAT_CLEAR, |j| j.clear)]);
        job_phase(prog, &[(JobKind::Lattice, PK_LAT_CLAIM, |j| j.count)]);
        job_phase(prog, &[(JobKind::Lattice, PK_LAT_BEST, |j| j.count)]);
        job_phase(prog, &[(JobKind::Lattice, PK_LAT_MERGE, |j| j.clear)]);
        // The view format its worker is compiled for: the jobs' one, else several (0).
        for j in &jobs {
            if matches!(j.kind, JobKind::Stage | JobKind::Scan | JobKind::Witness | JobKind::Copy | JobKind::Index) {
                prog.format = Some(match prog.format {
                    None => j.format,
                    Some(f) if f == j.format => f,
                    Some(_) => 0,
                });
            }
        }
        // What the jobs reach by address: resident at the launch; their own buffers, and the sort's, held to it.
        for mut j in jobs {
            prog.uses.append(&mut j.uses);
            prog.hold.append(&mut j.own);
        }
        prog.hold.push(params);
        if let Some((a, b, hist, _)) = sort {
            prog.hold.extend(a.into_iter().chain(b).chain([hist]));
        }
        Ok(())
    }

    /// Launches the batch's program (`Program`, decision 0603): every level flushed since its last launch, one launch
    /// of `join_run`, a group an item. Its control words are read when the batch is done (`finish`): a phase left
    /// undone, or a push past the queue's room, fails the batch.
    fn run_program(&mut self) -> Result<(), Error> {
        self.flush()?;
        if self.program.phases.is_empty() {
            return Ok(());
        }
        self.done.programs += 1;
        let mut prog = std::mem::take(&mut self.program);
        // The next program's claims go on past this one's, in the same arena: a table one places, a later one reads.
        self.program.top = prog.top.clone();
        // The arena, the batch's programs' one: each level's counts and placed tables at the offsets its size scripts
        // claim (decision 0603 step 5); twice what its first program's tables' rows were guessed to take, and what the
        // batch's programs' claims wanted when they passed one before.
        let sizes = ProgramSizes::now();
        let budget = sizes.budget;
        let counts = match &self.pool {
            Some(a) => a.clone(),
            None => {
                let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
                // Its first pages: its claims' guess, and the floor (a round's reach, `arena_floor`) where mapping
                // commits no memory (the CPU backend's pages are committed as touched) or the arena cannot grow. On
                // Metal a mapped page is real memory, so a reserved arena maps its guess and its claims map the rest
                // as they reach it (a park, then a remap). A test's first arena is its own, so its claims map pages as
                // a larger one's would.
                let floor = if self.reserve_arena && rerail_device::device::map_commits(device) { 0 } else { self.floor };
                let room = sizes.arena.unwrap_or((2 * prog.arena).max(floor)).max(16).next_multiple_of(16);
                if room > ARENA_MAX {
                    return Err(Error::BackendCapacity);
                }
                let held = if self.reserve_arena {
                    let r = std::sync::Arc::new(rerail_device::device::reserve(device, ARENA_MAX, room, self.host_reads).map_err(|_| Error::BackendCapacity)?);
                    self.reserved = Some(r.clone());
                    Held::kept(r.buffer(), Box::new(r))
                } else {
                    Held::new(room)?
                };
                self.pool.insert(std::rc::Rc::new(held)).clone()
            }
        };
        let ca = address(&counts.buffer)?;
        // Where the arena's buffer changes (a wrapped arena mapped further: `Reserved::map`), each launch resolves
        // its address again (`Kit::rebase`): the scripts' ops and the scan levels that name it.
        let arena_scans: Vec<(usize, bool, bool)> = prog.scans.iter().enumerate().map(|(i, l)| (i, matches!(l.data, ScanData::Script), matches!(l.sums, Some(ScanData::Script)))).filter(|&(_, d, s)| d || s).collect();
        let ops = prog.script.clone();
        for p in prog.made.drain(..) {
            *p.arena.borrow_mut() = Some(counts.clone());
            self.made.push(p);
        }
        let scans: Vec<ScanLevel> = prog
            .scans
            .iter()
            .map(|l| {
                let at = |d: ScanData| match d {
                    ScanData::Script => ca,
                    ScanData::At(a) => a,
                };
                let data = at(l.data);
                ScanLevel { data, sums: l.sums.map_or(data, at), w: ScanWords { n: l.n, offsets: u32::from(l.sums.is_some() && l.offsets), at: 0, sums_at: 0 } }
            })
            .collect();
        // Each in a buffer of its own (an empty one of a few bytes: a buffer is never empty).
        let bytes = |v: &[u8]| -> Result<Held, Error> { Held::of(if v.is_empty() { vec![0u8; 16] } else { v.to_vec() }.into_iter()).map_err(|_| Error::BackendCapacity) };
        let descs = bytes(as_bytes(&prog.descs))?;
        let items = bytes(as_bytes(&prog.items))?;
        let phases = bytes(as_bytes(&prog.phases))?;
        let scan_levels = bytes(as_bytes(&scans))?;
        let sorts = bytes(as_bytes(&prog.sorts))?;
        // The size scripts' addresses, now that the buffers they name are made.
        let (da, sa, xa) = (address(&descs.buffer)?, address(&scan_levels.buffer)?, address(&sorts.buffer)?);
        let late = |l: Late| l.at(da, sa, xa, ca);
        let mut seg_counts = prog.segs.clone();
        for &(i, l) in &prog.seg_late {
            seg_counts[i].count = late(l);
        }
        let segs = bytes(as_bytes(&seg_counts))?;
        let script: Vec<SizeOp> =
            prog.script.iter().map(|o| SizeOp { w: late(o.w), c: late(o.c), op: o.op, k: o.k, x: o.x, y: o.y }).collect();
        let script = bytes(as_bytes(&script))?;
        let top = prog.top.take();
        // Its chain's dispatches past its start (decision 0674): its phases, each a dispatch, once; four times them for
        // a program of rounds (its rounds go on across its launches); at least `PROGRAM_BUDGET`, at most
        // `PROGRAM_DISPATCHES` (a launch's GPU time bounded, decision 0603 point 5); a test's budget where it sets one.
        let rounds = prog.items.iter().any(|i| i[0] >> 24 == PK_ROUND);
        let nphases = prog.phases.len() as u32;
        let dispatches = budget.unwrap_or_else(|| (if rounds { 4 * nphases } else { nphases }).clamp(PROGRAM_BUDGET, PROGRAM_DISPATCHES)).max(1);
        let format = prog.format.unwrap_or_else(run_view_format);
        self.done.levels += prog.levels;
        let kit = Kit { descs, items, phases, segs, scans: scan_levels, sorts, script, top, counts, reserved: self.reserve_arena.then(|| self.reserved.clone()).flatten(), phase_items: prog.phases, dispatches, format, uses: prog.uses, hold: prog.hold, rooms: std::mem::take(&mut prog.rooms),
                        rebase: Rebase { ca, ops, scans: arena_scans, at: (da, sa, xa), ta: self.arena.as_ref().map_or(Ok(0), |a| address(&a.buffer()))? } };
        let ctl = Words::zeroed(10 + 2 * kit.phase_items.len() + 4 * (kit.dispatches as usize + 2))?;
        self.launch(kit, ctl)
    }

    /// One launch of `join_run` over `kit`'s program and its control words `ctl` (decisions 0603, 0674): a chain, its
    /// start and `kit.dispatches` dispatches, its first, or one again where the last left it (`programs_done`). Its
    /// arena's room, the pages mapped, in its top's second word.
    fn launch(&mut self, mut kit: Kit, ctl: Words) -> Result<(), Error> {
        let nphases = kit.phase_items.len();
        // A wrapped arena mapped further since its last launch: a new buffer over the same pages, at another address
        // (Metal) or at the same one, longer (CUDA's host memory at the host's address).
        if let Some(r) = &kit.reserved {
            let now = r.buffer();
            if address(&now)? != kit.rebase.ca || now.length() != kit.counts.buffer.length() {
                let held = std::rc::Rc::new(Held::kept(now, Box::new(r.clone())));
                self.pool = Some(held.clone());
                kit.rebase(held)?;
            }
        }
        // A text arena mapped further since (`Arena::grow`): its jobs' descriptors that reach it, its new wrap's.
        if let Some(a) = &self.arena {
            let now = address(&a.buffer())?;
            if kit.rebase.ta != 0 && now != kit.rebase.ta {
                kit.retext(now);
            }
        }
        let pipe = program_pipe(kit.format)?;
        if let Some(t) = &kit.top {
            let room = kit.reserved.as_ref().map_or(kit.counts.buffer.length() as usize, |r| r.mapped());
            t.set(1, u32::try_from(room / 16).map_err(|_| Error::BackendCapacity)?);
        }
        // The chain: its start one group (its counts the host's), its dispatches' counts emptied, its dispatch 0 again.
        let dispatches = u64::from(kit.dispatches);
        let slot = 10 + 2 * nphases;
        for w in 0..4 * (kit.dispatches as usize + 2) {
            ctl.set(slot + w, 0);
        }
        for w in 0..3 {
            ctl.set(slot + w, 1);
        }
        ctl.set(1, 0);
        self.done.launch_items = if self.done.launch_items == 0 { dispatches } else { self.done.launch_items.min(dispatches) };
        let args: Vec<u8> = [nphases as u32, kit.dispatches, 0, 0].iter().flat_map(|w| w.to_ne_bytes()).collect();
        let text = self.arena.as_ref().filter(|_| kit.rebase.ta != 0).map(|a| a.buffer());
        let pass = self.open()?;
        // What its jobs reach by address, and its own buffers: resident for the launch.
        let mut seen = std::collections::HashSet::new();
        for (b, write) in &kit.uses {
            if seen.insert((b.contents() as usize, b.length(), *write)) {
                if *write {
                    pass.use_read_write(b);
                } else {
                    pass.use_read(&[b]);
                }
            }
        }
        for h in &kit.hold {
            pass.use_read_write(&h.buffer);
        }
        if let Some(a) = &text {
            pass.use_read_write(a);
        }
        pass.use_read_write(&kit.counts.buffer);
        if let Some(t) = &kit.top {
            pass.use_read_write(&t.buffer.buffer);
        }
        let offsets: Vec<u64> = (0..=dispatches).map(|d| 4 * (slot as u64 + 4 * d)).collect();
        pass.chain_at(
            &pipe,
            &[(&kit.descs.buffer, 0), (&kit.items.buffer, 0), (&kit.phases.buffer, 0), (&kit.segs.buffer, 0), (&kit.scans.buffer, 0), (&kit.sorts.buffer, 0), (&kit.script.buffer, 0), (&ctl.buffer.buffer, 0), (&kit.counts.buffer, 0)],
            &args,
            &ctl.buffer.buffer,
            &offsets,
        );
        self.programs.push(Ran { ctl, last: nphases - 1, phases: nphases, items: kit.phase_items[nphases - 1][1], kit });
        Ok(())
    }

    /// The programs run since the last `finish`, each whole: its last phase done; and those to launch again (decisions
    /// 0603 point 5, 0674): one whose chain's dispatches ended with its phases left, and one whose size script parked
    /// past its arena's pages, mapped now for twice what it wanted, its item put back.
    fn programs_done(&mut self) -> Result<(bool, Vec<(Ran, bool)>), Error> {
        let mut whole = true;
        let mut again = Vec::new();
        for mut r in std::mem::take(&mut self.programs) {
            let flags = r.ctl.get(6);
            let done = r.ctl.get(10 + r.last);
            // Its last phase's items as pushed: its segments' (0: all of them).
            let want = match r.ctl.get(10 + r.phases + r.last) {
                0 => r.items,
                w => w,
            };
            // A phase parked as its texts found the text arena full (`ctl[9]`'s top bit): the arena mapped for twice
            // what they wanted (`Arena::grow`), and the program launched again, which pushes the phase again; else,
            // stopped, and its runs find the arena full (`wanted` past `cap`): they run again in a larger one, or on
            // the host past the device's text arena.
            // A gate's room mapped past (`ctl[9]`'s top two bits, the gate's phase): the rooms grown, and the program
            // launched again from the phase after the gate (bit 29); else stopped there, as a gate stops it.
            if flags & 32 != 0 && r.ctl.get(9) >> 30 == 3 {
                if r.kit.grow_rooms()? {
                    self.done.remaps += 1;
                    self.done.room_remaps += 1;
                    r.ctl.set(6, flags & !32);
                    r.ctl.set(9, 0x2000_0000 | (r.ctl.get(9) & 0x1fff_ffff));
                    again.push((r, true));
                    continue;
                }
                self.done.stopped = true;
                self.done.taken += u64::from(r.ctl.get(8));
                continue;
            }
            if flags & 32 != 0 && r.ctl.get(9) & 0x8000_0000 != 0 {
                let grown = match &self.arena {
                    Some(a) => a.grow()?,
                    None => false,
                };
                if grown {
                    self.done.remaps += 1;
                    r.ctl.set(6, flags & !32);
                    // Released: the next launch's first group pushes the phase again (bit 30).
                    r.ctl.set(9, 0x4000_0000 | (r.ctl.get(9) & 0x3fff_ffff));
                    again.push((r, true));
                    continue;
                }
                self.done.stopped = true;
                self.done.taken += u64::from(r.ctl.get(8));
                continue;
            }
            // A size script parked past its arena's pages (decision 0603 step 5): the arena mapped for twice what
            // its claims wanted, and the program launched again; else, an arena that cannot grow, stopped as a gate
            // stops it, and the batch's next programs' arenas twice what its claims wanted.
            if flags & 32 != 0 {
                let wanted = 16 * r.kit.top.as_ref().map_or(0, |t| t.get(2) as usize);
                let mapped = match &r.kit.reserved {
                    Some(a) => a.map(2 * wanted).map_err(|_| Error::BackendCapacity)?,
                    None => false,
                };
                if mapped {
                    self.done.remaps += 1;
                    r.put_back();
                    again.push((r, true));
                    continue;
                }
                self.done.arena_full = true;
                self.done.arena_wanted = self.done.arena_wanted.max(wanted);
                self.floor = self.floor.max(2 * wanted);
                self.done.stopped = true;
                self.done.taken += u64::from(r.ctl.get(8));
                continue;
            }
            // A gate stopped it: its phases after the gate are undone, as the caller asked.
            if flags & 2 != 0 {
                self.done.stopped = true;
                self.done.taken += u64::from(r.ctl.get(8));
                continue;
            }
            // A round's end ended it (no new rows) or stopped it past its rounds (`round_end`).
            if flags & (4 | 16) != 0 {
                self.done.ended |= flags & 16 != 0;
                self.done.past_rounds |= flags & 4 != 0;
                self.done.taken += u64::from(r.ctl.get(8));
                continue;
            }
            // Its chain's dispatches ended with a phase left (its last finisher gave the next groups): launched again,
            // where it goes on.
            if r.ctl.get(1) == r.kit.dispatches + 1 && r.ctl.get(4) != 0 {
                self.done.relaunches += 1;
                again.push((r, false));
                continue;
            }
            self.done.taken += u64::from(r.ctl.get(8));
            if done != want {
                eprintln!("rerail: a join program left undone (decision 0603): its last phase {done} of {want} items");
                whole = false;
            }
        }
        Ok((whole, again))
    }
}

// A size script's registers (`size_one`'s): the arena's address, 1, temporaries.
const R_ARENA: u32 = 2;
const R_ONE: u32 = 14;

/// A claim of `bytes` (register) of the arena (`size_one`'s op 9) into register `k`, as an offset in `unit`-byte
/// units, and one where `plus`.
fn claim(prog: &mut Program, top: u64, k: u32, bytes: u32, unit: u64, plus: bool) {
    prog.op(OpAt::of(SZ_CLAIM, k, bytes, 0).w(Late::Abs(top)).c(Late::Arena(0)));
    prog.op(OpAt::of(SZ_SUB, k, k, R_ARENA));
    prog.op(OpAt::of(SZ_DIVUP, k, k, 0).c(Late::Abs(unit)));
    if plus {
        prog.op(OpAt::of(SZ_ADD, k, k, ZERO).c(Late::Abs(1)));
    }
}

/// The bytes of the arena a claim of `bytes` takes (16 a unit).
fn units(bytes: usize) -> usize {
    bytes.next_multiple_of(16)
}

/// A level's size script at its start (decision 0603 step 5, `size_one`), a phase before its copies: each job's placed
/// input where its producer placed it (its offset into the job's descriptor, its rows' room into its parameters), a
/// row's values and a dedup's places placed by it; then the level's counts claimed (each job's its input's room and
/// one), each job's first word, and the scan's three levels' values, sums and counts. None for a level of neither.
/// `pa`: the level's parameter blocks' address; `placed`: each job's blocks in it.
#[allow(clippy::too_many_arguments)]
fn level_sizes(prog: &mut Program, jobs: &[Job], first_job: usize, total: usize, pa: u64, placed: &[[Option<u64>; 5]], sort: Option<SortSizes>, wide: Option<SortSizes>)
               -> Result<Option<usize>, Error> {
    if total == 0 && sort.is_none() && wide.is_none() && jobs.iter().all(|j| j.l_place.is_none() && j.d_place.is_none() && j.o_rows.is_none()) {
        return Ok(None);
    }
    let first = prog.script.len();
    let top = prog.top()?;
    let site = |i: usize, b: usize, off: WordAt| Late::Abs(pa + placed[i][b].expect("a job's parameter block") + off.0);
    let desc = |i: usize, f: WordAt| Late::Desc(first_job + i, f);
    let (t, u, w, x) = (4u32, 5u32, 6u32, 7u32);
    prog.op(OpAt::of(SZ_CONST, R_ARENA, 0, 0).c(Late::Arena(0)));
    prog.op(OpAt::of(SZ_CONST, R_ONE, 0, 0).c(Late::Abs(1)));
    for (i, j) in jobs.iter().enumerate() {
        // Its output placed here: its rows a word's (an earlier level's), its rows' room and where it is into its
        // place, its offset into its descriptor, its room into its parameters. Its jobs after it, here or later,
        // read them (`d_place`, `l_place`).
        if let (Some(p), Some(OutRows::Word(word))) = (&j.o_place, j.o_rows) {
            prog.op(OpAt::of(SZ_LOAD, 11, 0, 0).w(Late::Abs(word)));
            place_out(prog, j, p, first_job, i, pa, placed, top)?;
        }
        if let Some(p) = &j.d_place {
            let rec = address(&p.rec.buffer.buffer)?;
            prog.uses.push((p.rec.buffer.buffer.clone(), false));
            prog.op(OpAt::of(SZ_LOAD, t, 0, 0).w(Late::Abs(rec)));
            prog.op(OpAt::of(SZ_PUT32, t, 0, 0).w(desc(i, word_at!(JobDesc, at.o_at))));
            prog.op(OpAt::of(SZ_LOAD, u, 0, 0).w(Late::Abs(rec + 4)));
            for &(b, off) in &j.d_caps {
                prog.op(OpAt::of(SZ_PUT32, u, 0, 0).w(site(i, b, off)));
            }
        }
        let Some(p) = &j.l_place else { continue };
        let rec = address(&p.rec.buffer.buffer)?;
        prog.uses.push((p.rec.buffer.buffer.clone(), false));
        prog.op(OpAt::of(SZ_LOAD, t, 0, 0).w(Late::Abs(rec)));
        prog.op(OpAt::of(SZ_PUT32, t, 0, 0).w(desc(i, if j.o_reads { word_at!(JobDesc, at.o_at) } else { word_at!(JobDesc, at.l_at) })));
        prog.op(OpAt::of(SZ_LOAD, u, 0, 0).w(Late::Abs(rec + 4)));
        for &(b, off) in &j.in_caps {
            prog.op(OpAt::of(SZ_PUT32, u, 0, 0).w(site(i, b, off)));
        }
        let guess = j.n.max(j.count).max(1);
        if j.v_place {
            prog.op(OpAt::of(SZ_MUL, w, u, 0).c(Late::Abs(8)));
            prog.op(OpAt::of(SZ_ADD, w, w, ZERO).c(Late::Abs(8)));
            claim(prog, top, x, w, 8, true);
            prog.op(OpAt::of(SZ_PUT32, x, 0, 0).w(desc(i, word_at!(JobDesc, at.v_at))));
            prog.arena += units(8 * guess);
        }
        if let Some(h) = j.h_place {
            prog.op(OpAt::of(SZ_MUL, w, u, 0).c(Late::Abs(2)));
            prog.op(OpAt::of(SZ_POW2, w, w, 0).c(Late::Abs(16)));
            if let Some(n) = h.places {
                prog.op(OpAt::of(SZ_PUT32, w, 0, 0).w(site(i, h.block, n)));
            }
            prog.op(OpAt::of(SZ_SUB, x, w, R_ONE));
            prog.op(OpAt::of(SZ_PUT32, x, 0, 0).w(site(i, h.block, h.mask)));
            prog.op(OpAt::of(SZ_MUL, x, w, 0).c(Late::Abs(4 * h.arrays)));
            claim(prog, top, x, x, 4, true);
            prog.op(OpAt::of(SZ_PUT32, x, 0, 0).w(desc(i, word_at!(JobDesc, at.h_at))));
            prog.arena += units(4 * h.arrays as usize * (2 * guess).next_power_of_two().max(16));
        }
        // Its output placed here with its input's room (a fold's changed rows).
        if let (Some(o), Some(OutRows::InRoom)) = (&j.o_place, j.o_rows) {
            prog.op(OpAt::of(SZ_ADD, 11, u, ZERO));
            place_out(prog, j, o, first_job, i, pa, placed, top)?;
        }
    }
    if let Some(ss) = sort {
        // The level's indexes' keys end to end (decision 0603 step 5e): each one's first (`eb`), a live relation's
        // its length as claimed past its first row (at most its bound), else its bound; then each pass's keys and tiles, and its
        // histograms' scan (`UKL_RADIX_DIGITS` words a tile).
        let (sum, r, cap) = (12u32, 13u32, 11u32);
        prog.op(OpAt::of(SZ_CONST, sum, 0, 0).c(Late::Abs(0)));
        for (i, j) in jobs.iter().enumerate().filter(|(_, j)| j.kind == JobKind::Index) {
            prog.op(OpAt::of(SZ_PUT32, sum, 0, 0).w(desc(i, word_at!(JobDesc, at.eb))));
            match j.rows {
                Some((word, _)) => {
                    prog.op(OpAt::of(SZ_LOAD, r, 0, 0).w(Late::Abs(word)));
                    prog.op(OpAt::of(SZ_CONST, cap, 0, 0).c(Late::Abs(u64::from(j.rows_from))));
                    prog.op(OpAt::of(SZ_SUB, r, r, cap));
                    prog.op(OpAt::of(SZ_CONST, cap, 0, 0).c(Late::Abs(j.count as u64)));
                    prog.op(OpAt::of(SZ_MIN, r, r, cap));
                }
                None => prog.op(OpAt::of(SZ_CONST, r, 0, 0).c(Late::Abs(j.count as u64))),
            }
            prog.op(OpAt::of(SZ_ADD, sum, sum, r));
        }
        prog.op(OpAt::of(SZ_DIVUP, r, sum, 0).c(Late::Abs(SORT_TILE as u64)));
        prog.op(OpAt::of(SZ_MAX, r, r, 0).c(Late::Abs(1)));
        for x in ss.first..ss.first + ss.passes {
            prog.op(OpAt::of(SZ_PUT32, sum, 0, 0).w(Late::Sort(x, word_at!(SortPass, w.n))));
            prog.op(OpAt::of(SZ_PUT32, r, 0, 0).w(Late::Sort(x, word_at!(SortPass, w.tiles))));
        }
        prog.op(OpAt::of(SZ_MUL, r, r, 0).c(Late::Abs(256)));
        size_scan(prog, top, r, 256 * ss.tiles, |level, f| Late::Scan(ss.hist + level, f));
    }
    if let Some(ws) = wide {
        // The level's sorted dedups' keys end to end (t58 increment A): each one's first (`eb`), its rows as its
        // input's count says (the device's: its input's room is too); then its region claimed for them (`WideAt`:
        // 16 bytes a key, and its tiles' histograms), its first word and its keys into each pass and each dedup, and
        // its histograms' scan, from their place in it.
        let (sum, r, rg, b) = (12u32, 13u32, 11u32, 4u32);
        prog.op(OpAt::of(SZ_CONST, sum, 0, 0).c(Late::Abs(0)));
        for (i, j) in jobs.iter().enumerate().filter(|(_, j)| j.wide > 0) {
            prog.op(OpAt::of(SZ_PUT32, sum, 0, 0).w(desc(i, word_at!(JobDesc, at.eb))));
            let (word, _) = j.rows_word().expect("a dedup's rows' word");
            prog.op(OpAt::of(SZ_LOAD, r, 0, 0).w(Late::Abs(word)));
            prog.op(OpAt::of(SZ_ADD, sum, sum, r));
        }
        prog.op(OpAt::of(SZ_DIVUP, r, sum, 0).c(Late::Abs(SORT_TILE as u64)));
        prog.op(OpAt::of(SZ_MAX, r, r, 0).c(Late::Abs(1)));
        prog.op(OpAt::of(SZ_MUL, b, r, 0).c(Late::Abs(4 * 256)));
        prog.op(OpAt::of(SZ_ADD, b, b, ZERO).c(Late::Abs(4)));
        prog.op(OpAt::of(SZ_MUL, rg, sum, 0).c(Late::Abs(16)));
        prog.op(OpAt::of(SZ_ADD, b, b, rg));
        claim(prog, top, rg, b, 4, true);
        for x in ws.first..ws.first + ws.passes {
            prog.op(OpAt::of(SZ_PUT32, sum, 0, 0).w(Late::Sort(x, word_at!(SortPass, w.n))));
            prog.op(OpAt::of(SZ_PUT32, r, 0, 0).w(Late::Sort(x, word_at!(SortPass, w.tiles))));
            prog.op(OpAt::of(SZ_PUT32, rg, 0, 0).w(Late::Sort(x, word_at!(SortPass, w.wide))));
        }
        for (i, _) in jobs.iter().enumerate().filter(|(_, j)| j.wide > 0) {
            prog.op(OpAt::of(SZ_PUT32, rg, 0, 0).w(desc(i, word_at!(JobDesc, at.h_at))));
            prog.op(OpAt::of(SZ_PUT32, sum, 0, 0).w(site(i, 4, HK_TOTAL)));
        }
        // Its histograms past its keys: its first word less one, and four words a key.
        prog.op(OpAt::of(SZ_MUL, b, sum, 0).c(Late::Abs(4)));
        prog.op(OpAt::of(SZ_ADD, b, b, rg));
        prog.op(OpAt::of(SZ_SUB, b, b, R_ONE));
        prog.op(OpAt::of(SZ_PUT32, b, 0, 0).w(Late::Scan(ws.hist, word_at!(ScanLevel, w.at))));
        prog.op(OpAt::of(SZ_MUL, r, r, 0).c(Late::Abs(256)));
        size_scan(prog, top, r, 256 * ws.tiles, |level, f| Late::Scan(ws.hist + level, f));
    }
    if total > 0 {
        // Each job's counts: its rows' and one, where its rows are a word's (its count phase's items: the rows its
        // level's earlier levels made, final when its size script runs); at most its placed input's room and one, else
        // as encoded. So the level's scan is of its rows, not of its tables' rooms. The level's total word after them.
        let n_of = |prog: &mut Program, j: &Job, into: u32| -> Result<(), Error> {
            match (&j.l_place, j.n) {
                (Some(p), n) if n > 0 => {
                    prog.op(OpAt::of(SZ_LOAD, u, 0, 0).w(Late::Abs(address(&p.rec.buffer.buffer)? + 4)));
                    prog.op(OpAt::of(SZ_ADD, u, u, ZERO).c(Late::Abs(1)));
                }
                (_, n) => prog.op(OpAt::of(SZ_CONST, u, 0, 0).c(Late::Abs(n as u64))),
            }
            if let Some((word, _)) = j.rows_word() {
                prog.op(OpAt::of(SZ_LOAD, x, 0, 0).w(Late::Abs(word)));
                prog.op(OpAt::of(SZ_ADD, x, x, ZERO).c(Late::Abs(1)));
                prog.op(OpAt::of(SZ_MIN, u, u, x));
            }
            prog.op(OpAt::of(SZ_ADD, into, into, u));
            Ok(())
        };
        prog.op(OpAt::of(SZ_CONST, 1, 0, 0).c(Late::Abs(1)));
        for j in jobs.iter().filter(|j| j.n > 0) {
            n_of(prog, j, 1)?;
        }
        prog.op(OpAt::of(SZ_MUL, w, 1, 0).c(Late::Abs(4)));
        claim(prog, top, 3, w, 4, false);
        // The scan's levels (`Program::scan_sized`): its values in the arena, its tiles' sums, theirs.
        let n0 = 8u32;
        prog.op(OpAt::of(SZ_SUB, n0, 1, R_ONE));
        prog.op(OpAt::of(SZ_PUT32, 3, 0, 0).w(Late::Pending(0, word_at!(ScanLevel, w.at))));
        size_scan(prog, top, n0, total, Late::Pending);
        for (i, j) in jobs.iter().enumerate().filter(|(_, j)| j.n > 0) {
            prog.op(OpAt::of(SZ_PUT32, 3, 0, 0).w(desc(i, word_at!(JobDesc, at.base))));
            n_of(prog, j, 3)?;
        }
        prog.arena += units(4 * (total + 1));
    }
    prog.phase(vec![(PK_SIZE, first as u32, (prog.script.len() - first) as u32)]);
    Ok(Some(first))
}

/// A sized scan's words set by a size script (`Program::scan_levels_sized`): its first level's count (register `n0`,
/// `guess` values guessed), its tiles' sums claimed in the arena, level by level, and the next level's values and
/// count with them. `level`: a level's word's place (`Late::Scan`, or `Late::Pending` before the scan is made).
fn size_scan(prog: &mut Program, top: u64, n0: u32, guess: usize, level: impl Fn(usize, WordAt) -> Late) {
    let (s0, t0, w) = (9u32, 10u32, 6u32);
    let (lat, ln, lsums) = (word_at!(ScanLevel, w.at), word_at!(ScanLevel, w.n), word_at!(ScanLevel, w.sums_at));
    prog.op(OpAt::of(SZ_PUT32, n0, 0, 0).w(level(0, ln)));
    let mut guess = guess;
    for l in 0..2 {
        prog.op(OpAt::of(SZ_DIVUP, t0, n0, 0).c(Late::Abs(SCAN_TILE as u64)));
        prog.op(OpAt::of(SZ_ADD, w, t0, ZERO).c(Late::Abs(1)));
        prog.op(OpAt::of(SZ_MUL, w, w, 0).c(Late::Abs(4)));
        claim(prog, top, s0, w, 4, false);
        prog.op(OpAt::of(SZ_PUT32, s0, 0, 0).w(level(l, lsums)));
        prog.op(OpAt::of(SZ_PUT32, s0, 0, 0).w(level(l + 1, lat)));
        prog.op(OpAt::of(SZ_PUT32, t0, 0, 0).w(level(l + 1, ln)));
        prog.op(OpAt::of(SZ_ADD, n0, t0, ZERO));
        guess = guess.div_ceil(SCAN_TILE).max(1);
        prog.arena += units(4 * (guess + 1));
    }
}

/// A level's sort as its size script sizes it (decision 0603 step 5e, `level_sizes`): its passes from `first`, its
/// histograms' scan's levels from `hist`, its tiles guessed.
#[derive(Clone, Copy)]
struct SortSizes {
    first: usize,
    passes: usize,
    hist: usize,
    tiles: usize,
}

/// Job `j`'s output `p` placed by its level's size script at its start (`Job::o_rows`), its rows in register 11 (at
/// least one): claimed in the arena, where it is and its rows into its place, its offset into its descriptor (the
/// level's `i`-th, from the program's `first_job`), its rows' room into its parameters' words.
#[allow(clippy::too_many_arguments)]
fn place_out(prog: &mut Program, j: &Job, p: &std::rc::Rc<Place>, first_job: usize, i: usize, pa: u64, placed: &[[Option<u64>; 5]], top: u64) -> Result<(), Error> {
    let (rows, w, x) = (11u32, 12u32, 13u32);
    let rec = address(&p.rec.buffer.buffer)?;
    prog.uses.push((p.rec.buffer.buffer.clone(), true));
    prog.made.push(p.clone());
    prog.op(OpAt::of(SZ_MAX, rows, rows, 0).c(Late::Abs(1)));
    prog.op(OpAt::of(SZ_MUL, w, rows, 0).c(Late::Abs(8 * j.out_slots as u64)));
    claim(prog, top, x, w, 8, true);
    prog.op(OpAt::of(SZ_PUT32, x, 0, 0).w(Late::Abs(rec)));
    prog.op(OpAt::of(SZ_PUT32, rows, 0, 0).w(Late::Abs(rec + 4)));
    prog.op(OpAt::of(SZ_PUT32, x, 0, 0).w(Late::Desc(first_job + i, word_at!(JobDesc, at.o_at))));
    for &(b, off) in &j.out_caps {
        prog.op(OpAt::of(SZ_PUT32, rows, 0, 0).w(Late::Abs(pa + placed[i][b].expect("a job's parameter block") + off.0)));
    }
    prog.arena += units(8 * j.out_slots * j.out_guess.max(1));
    Ok(())
}

/// A level's size script after its scan (decision 0603 step 5): each job's placed output, as many rows as its counts
/// made (its counts' offsets' span over the rows it counted), claimed in the arena; where it is and its rows into its
/// place (`Place::rec`), its offset into the job's descriptor, its rows' room into its parameters. None for a level
/// that makes none.
fn made_sizes(prog: &mut Program, jobs: &[Job], first_job: usize, pa: u64, placed: &[[Option<u64>; 5]]) -> Result<(), Error> {
    if jobs.iter().all(|j| j.o_place.is_none() || j.o_reads || j.o_rows.is_some()) {
        return Ok(());
    }
    let first = prog.script.len();
    let top = prog.top()?;
    let (base, m, cap, end, at, rows, w, x) = (1u32, 4u32, 5u32, 6u32, 7u32, 9u32, 10u32, 11u32);
    prog.op(OpAt::of(SZ_CONST, R_ARENA, 0, 0).c(Late::Arena(0)));
    for (i, j) in jobs.iter().enumerate() {
        let Some(p) = j.o_place.as_ref().filter(|_| !j.o_reads && j.o_rows.is_none()) else { continue };
        let rec = address(&p.rec.buffer.buffer)?;
        prog.uses.push((p.rec.buffer.buffer.clone(), true));
        prog.made.push(p.clone());
        prog.op(OpAt::of(SZ_LOAD, base, 0, 0).w(Late::Desc(first_job + i, word_at!(JobDesc, at.base))));
        match j.m.0 {
            Some(word) => prog.op(OpAt::of(SZ_LOAD, m, 0, 0).w(Late::Abs(word))),
            None => prog.op(OpAt::of(SZ_CONST, m, 0, 0).c(Late::Abs(j.m.1 as u64))),
        }
        match &j.l_place {
            Some(l) => prog.op(OpAt::of(SZ_LOAD, cap, 0, 0).w(Late::Abs(address(&l.rec.buffer.buffer)? + 4))),
            None => prog.op(OpAt::of(SZ_CONST, cap, 0, 0).c(Late::Abs(j.m.1 as u64))),
        }
        prog.op(OpAt::of(SZ_MIN, m, m, cap));
        prog.op(OpAt::of(SZ_ADD, end, base, m));
        prog.op(OpAt::of(SZ_LOADX, x, end, 0).w(Late::Arena(0)));
        prog.op(OpAt::of(SZ_LOADX, at, base, 0).w(Late::Arena(0)));
        prog.op(OpAt::of(SZ_SUB, rows, x, at));
        prog.op(OpAt::of(SZ_MAX, rows, rows, 0).c(Late::Abs(1)));
        prog.op(OpAt::of(SZ_MUL, w, rows, 0).c(Late::Abs(8 * j.out_slots as u64)));
        claim(prog, top, x, w, 8, true);
        prog.op(OpAt::of(SZ_PUT32, x, 0, 0).w(Late::Abs(rec)));
        prog.op(OpAt::of(SZ_PUT32, rows, 0, 0).w(Late::Abs(rec + 4)));
        prog.op(OpAt::of(SZ_PUT32, x, 0, 0).w(Late::Desc(first_job + i, word_at!(JobDesc, at.o_at))));
        for &(b, off) in &j.out_caps {
            prog.op(OpAt::of(SZ_PUT32, rows, 0, 0).w(Late::Abs(pa + placed[i][b].expect("a job's parameter block") + off.0)));
        }
        prog.arena += units(8 * j.out_slots * j.out_guess.max(1));
    }
    prog.phase(vec![(PK_SIZE, first as u32, (prog.script.len() - first) as u32)]);
    Ok(())
}

/// The values of `v` as bytes: a `repr(C)` type's, integers with no padding.
fn as_bytes<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: every `T` here is `repr(C)` integers with no padding (`JobDesc`, `SortPass`, `ScanLevel`, `[u32; N]`).
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}

/// A word a gate or a round's end reads or writes: slot `.1` of words, or a relation's rows' length as the device
/// claims it (`Rows::len`, in its block: a live relation's, `DevRel::live`).
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) enum At<'a> {
    Word(&'a Words, usize),
    Len(&'a DevRel),
}

impl At<'_> {
    /// Its address, for `job`, which reaches its buffer.
    fn reach(&self, job: &mut Job, write: bool) -> Result<u64, Error> {
        match *self {
            At::Word(w, slot) => {
                assert!(slot < w.len, "a word past its words");
                job.reach(&w.buffer.buffer, 4 * slot as u64, write)
            }
            At::Len(r) => job.reach(&r.buffer.buffer, r.rows_at() + ROWS_LEN, write),
        }
    }
}

/// What a round's end does (`Batch::round_end`, `round_one`'s ops), in order.
pub(in crate::code_rule_dsl) enum RoundOp<'a> {
    /// `to += from`: a round's new rows claimed onto its relation.
    Add(At<'a>, At<'a>),
    /// `to = from`: a next delta's length.
    Set(At<'a>, At<'a>),
    /// Words `at..at + n` emptied for the next round.
    Zero(&'a Words, usize, usize),
    /// Slot `round` of the words (the round counted before) set to `from`: a round's count, kept by round.
    Log(&'a Words, At<'a>),
    /// A count the round made: the program goes on while any is non-zero.
    Any(At<'a>),
    /// Each of `from`'s words added to `to`'s: a round's counts summed over the rounds.
    Sum(&'a Words, &'a Words),
}

/// `Rows::len`'s offset in its descriptor: a relation's rows as claimed, which the device reads (`clif_rows_len`) and
/// a round's end writes.
const ROWS_LEN: u64 = std::mem::offset_of!(rerail_blocks::rows::Rows, len) as u64;

/// A launched program's buffers (`Batch::launch`), kept while it may launch again (`Batch::programs_done`).
/// What a program's launch resolves again where its arena's buffer changes (`Kit::rebase`): the arena's address it
/// was resolved for, its size scripts' ops, its scan levels that read the arena (data, sums), and the other buffers'
/// addresses (descriptors, scan levels, sort passes).
struct Rebase {
    ca: u64,
    ops: Vec<OpAt>,
    scans: Vec<(usize, bool, bool)>,
    at: (u64, u64, u64),
    /// The batch's text arena's address its descriptors name (0: none), for one mapped further (`Kit::retext`).
    ta: u64,
}

impl Kit {
    /// Its arena now `counts` (a wrapped arena's newer wrap, the same pages at another address): its size scripts'
    /// ops resolved again, and its scan levels' arena pointers written again in place (their words, which its scripts
    /// wrote, kept). Between launches: no work of it runs.
    fn rebase(&mut self, counts: std::rc::Rc<Held>) -> Result<(), Error> {
        let ca = address(&counts.buffer)?;
        let (da, sa, xa) = self.rebase.at;
        let script: Vec<SizeOp> = self.rebase.ops.iter().map(|o| SizeOp { w: o.w.at(da, sa, xa, ca), c: o.c.at(da, sa, xa, ca), op: o.op, k: o.k, x: o.x, y: o.y }).collect();
        self.script = Held::of(if script.is_empty() { vec![0u8; 16] } else { as_bytes(&script).to_vec() }.into_iter()).map_err(|_| Error::BackendCapacity)?;
        let levels = self.scans.buffer.contents().cast::<ScanLevel>();
        for &(i, data, sums) in &self.rebase.scans {
            // SAFETY: level `i` of the levels this buffer was made of; no work of the program runs between launches.
            let l = unsafe { &mut *levels.add(i) };
            if data {
                l.data = ca;
            }
            if sums {
                l.sums = ca;
            }
        }
        self.rebase.ca = ca;
        self.counts = counts;
        Ok(())
    }

    /// Its batch's text arena now at `ta` (a wrapped arena's newer wrap, the same pages at another address): each
    /// descriptor's buffer address that named the old one written again in place. Between launches: no work of it
    /// runs.
    fn retext(&mut self, ta: u64) {
        const WORDS: usize = std::mem::offset_of!(JobDesc, at) / 8;
        let n = self.descs.buffer.length() as usize / std::mem::size_of::<JobDesc>();
        let descs = self.descs.buffer.contents().cast::<[u64; WORDS]>();
        for i in 0..n {
            // SAFETY: descriptor `i` of those this buffer was made of (`JobDesc`, its addresses its first `WORDS`
            // words); no work of the program runs between launches.
            let words = unsafe { &mut *descs.cast::<u8>().add(i * std::mem::size_of::<JobDesc>()).cast::<[u64; WORDS]>() };
            for w in words.iter_mut().filter(|w| **w == self.rebase.ta) {
                *w = ta;
            }
        }
        self.rebase.ta = ta;
    }
}

struct Kit {
    /// Its addresses as resolved, for a launch whose arena moved (`Kit::rebase`).
    rebase: Rebase,
    descs: Held,
    items: Held,
    phases: Held,
    segs: Held,
    scans: Held,
    sorts: Held,
    /// Its size scripts' ops (`SizeOp`), and their arena's top (none where it has none).
    script: Held,
    top: Option<std::rc::Rc<Words>>,
    /// The arena: its counts and placed tables (decision 0603 step 5), held by their places too; where it is address
    /// space mapped as its claims reach it (`Batch::arena_reserved`), that.
    counts: std::rc::Rc<Held>,
    reserved: Option<std::sync::Arc<rerail_device::device::Reserved>>,
    /// Each phase's first item, its items and its segments (`Program::phases`).
    phase_items: Vec<[u32; 4]>,
    /// Its chain's dispatches past its start (`PROGRAM_BUDGET`, `ProgramSizes`).
    dispatches: u32,
    format: u32,
    uses: Vec<(Buffer, bool)>,
    hold: Vec<Held>,
    /// Its gates' rooms, mapped as its rows reach them (`Batch::gate_rooms`), and what a room grown re-points.
    rooms: Rooms,
}

/// What a gate's room grew to (`RoomGrow`): its check's new cap, and the live relations grown with it (their block,
/// their offset there, their rows mapped now), whose indexes' rows and sorts are mapped for those rows.
pub(in crate::code_rule_dsl) struct Grown {
    pub(in crate::code_rule_dsl) cap: usize,
    pub(in crate::code_rule_dsl) rels: Vec<(std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>, u64, u32)>,
}

/// A gate's room's growth (`Batch::gate_rooms`), between its program's launches, once a round's rows passed it: its
/// relations' rows reserved further where they lie (`Ok(Some)`); `Ok(None)` where its rows are within it; an error
/// where its block has no more room (the program then stops there, as a gate stops it).
pub(in crate::code_rule_dsl) type RoomGrow = Box<dyn FnMut() -> Result<Option<Grown>, String>>;

/// A program's rooms mapped as its rows reach them (decision 0603's fewer reruns, on a device whose mapped pages are
/// real memory): its gates' growths, its live indexes, its levels' sorts of them, and the words of its parameters
/// that hold an address its jobs reached (`Job::reached`), found again where a block they reach is wrapped again.
#[derive(Default)]
struct Rooms {
    gates: Vec<GateRoom>,
    live_ix: Vec<LiveIx>,
    sorts: Vec<SortRoom>,
    /// A parameters buffer and a word's byte offset in it.
    relocs: Vec<(Buffer, u64)>,
}

/// A gate's check of a room: its cap, a `u32` at byte `at` of `params`, and its growth.
struct GateRoom {
    params: Buffer,
    at: u64,
    grow: RoomGrow,
}

/// A live relation's index build (`queue_index`): its relation (its block, its offset there), its entry, its rows
/// as mapped, and its parameters (`IxParams`, whose first word is its rows at most) at byte `at` of `params`.
struct LiveIx {
    home: std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>,
    rel: u64,
    index: usize,
    params: Buffer,
    at: u64,
    rows: std::cell::Cell<u32>,
}

/// A level's sort of its indexes' keys whose live ones are mapped as reached: its four buffers reserved for every
/// index's rows at most, mapped for its other indexes' rows (`fixed`) and its live ones' as mapped (`live`, of
/// `Rooms::live_ix`).
struct SortRoom {
    reserved: Vec<std::sync::Arc<rerail_device::device::Reserved>>,
    /// Its histograms (`sort_hist_bytes` of its rows).
    hist: Option<std::sync::Arc<rerail_device::device::Reserved>>,
    fixed: usize,
    live: Vec<usize>,
}

impl Kit {
    /// Its gates' rooms grown, once a gate parked the program past one (decision 0603): each room's relations'
    /// rows reserved further (its growth's), its cap written again, its live indexes' rows given room for them and
    /// their sorts mapped for them, and every address the program holds into a block wrapped again re-pointed. False
    /// where none grew, or one could not (the program then stops there).
    fn grow_rooms(&mut self) -> Result<bool, Error> {
        type Rel = (std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>, u64, u32);
        let mut grew: Vec<Rel> = Vec::new();
        let mut changed = false;
        for g in &mut self.rooms.gates {
            match (g.grow)() {
                Ok(Some(to)) => {
                    changed = true;
                    let cap = u32::try_from(to.cap).unwrap_or(u32::MAX);
                    // SAFETY: the check's cap, a `u32` within the parameters buffer; no work of the program runs.
                    unsafe { g.params.contents().cast::<u8>().add(g.at as usize).cast::<u32>().write_unaligned(cap) };
                    grew.extend(to.rels);
                }
                Ok(None) => {}
                Err(why) => {
                    crate::code_rule_dsl::prof::count_named(format!("device.a gate's room not grown: {why}"), 1);
                    return Ok(false);
                }
            }
        }
        if !changed {
            return Ok(false);
        }
        // The live indexes' rows: room for the rows mapped, in their blocks (their buckets as they are).
        let mut moved: Vec<(std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>, usize, usize)> = Vec::new();
        for ix in &self.rooms.live_ix {
            let Some(rows) = grew.iter().filter(|(f, at, _)| std::sync::Arc::ptr_eq(f, &ix.home) && *at == ix.rel).map(|g| g.2).max() else { continue };
            if rows <= ix.rows.get() {
                continue;
            }
            // SAFETY: the relation at `rel` of its block, which the program's index build of it reaches; no work of
            // the program runs.
            let r = unsafe { &*ix.home.block.ptr::<rerail_blocks::relation::Relation>(ix.rel as usize) };
            let old = ix.home.block.offset_of(r.indexes.as_slice()[ix.index].rows.as_ptr());
            // SAFETY: the entry is this program's live index, built by the device only, between launches.
            let Some(new) = (unsafe { r.grow_index_rows(&ix.home.block, ix.index, rows) }) else {
                crate::code_rule_dsl::prof::count_named("device.a live index's rows not grown: its block is full".into(), 1);
                return Ok(false);
            };
            // SAFETY: its parameters' first word (`IxParams::n`), within the buffer; no work of the program runs.
            unsafe { ix.params.contents().cast::<u8>().add(ix.at as usize).cast::<u32>().write_unaligned(rows) };
            ix.rows.set(rows);
            moved.push((ix.home.clone(), old, new));
        }
        // Each block grown wrapped again where it outgrew its wrap: what the program reaches of it re-pointed.
        let device = rerail_accel::gpu::join::device().map_err(|_| Error::BackendCapacity)?;
        let mut blocks: Vec<std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>> = Vec::new();
        for (f, _, _) in &grew {
            if !blocks.iter().any(|b| std::sync::Arc::ptr_eq(b, f)) {
                blocks.push(f.clone());
            }
        }
        for f in &blocks {
            let old = f.block.kept_device_buffer();
            let new = f.block.device_buffer(device).map_err(|_| Error::BackendCapacity)?;
            if let Some((ob, len)) = old {
                self.rebind(address(&ob)?, len as u64, &new)?;
            }
        }
        // The index builds' rows, where they moved in their blocks.
        for (f, old, new) in moved {
            let base = address(&f.block.device_buffer(device).map_err(|_| Error::BackendCapacity)?)?;
            self.repoint(base + old as u64, base + new as u64);
        }
        // The levels' sorts of the live indexes, mapped for their rows.
        let mut remapped: Vec<(u64, u64, Buffer)> = Vec::new();
        for s in &self.rooms.sorts {
            let rows = s.fixed + s.live.iter().map(|&i| self.rooms.live_ix[i].rows.get() as usize).sum::<usize>();
            for (r, bytes) in s.reserved.iter().map(|r| (r, 4 * rows)).chain(s.hist.iter().map(|h| (h, sort_hist_bytes(rows)))) {
                let old = r.buffer();
                if !r.map(bytes).map_err(|_| Error::BackendCapacity)? {
                    return Ok(false);
                }
                let new = r.buffer();
                if address(&old)? != address(&new)? || old.length() != new.length() {
                    remapped.push((address(&old)?, old.length(), new));
                }
            }
        }
        for (old, len, new) in remapped {
            self.rebind(old, len, &new)?;
        }
        Ok(true)
    }

    /// A buffer the program reached at `old` (`len` bytes) now `new` (the same memory wrapped again: a block grown, a
    /// sort's room mapped further): every address into it re-pointed (its jobs' descriptors' addresses, its
    /// parameters' words that held one, its segments' counts, its scans' levels, its sorts' passes), and the launch's
    /// resident buffers.
    fn rebind(&mut self, old: u64, len: u64, new: &Buffer) -> Result<(), Error> {
        let na = address(new)?;
        let at = |v: u64| if v >= old && v < old + len { v - old + na } else { v };
        // Field `o` (a `u64` address) of each record of `stride` bytes the buffer was made of.
        let words = |b: &Buffer, stride: usize, fields: &[usize]| {
            let p = b.contents().cast::<u8>();
            for i in 0..b.length() as usize / stride {
                for &o in fields {
                    // SAFETY: within the buffer, a field of its records; no work of the program runs between launches.
                    unsafe {
                        let w = p.add(i * stride + o).cast::<u64>();
                        w.write_unaligned(at(w.read_unaligned()));
                    }
                }
            }
        };
        let desc_words: Vec<usize> = (0..std::mem::offset_of!(JobDesc, at) / 8).map(|w| 8 * w).collect();
        words(&self.descs.buffer, std::mem::size_of::<JobDesc>(), &desc_words);
        words(&self.segs.buffer, std::mem::size_of::<SegAt>(), &[std::mem::offset_of!(SegAt, count)]);
        words(&self.scans.buffer, std::mem::size_of::<ScanLevel>(), &[std::mem::offset_of!(ScanLevel, data), std::mem::offset_of!(ScanLevel, sums)]);
        let sort_words = [
            std::mem::offset_of!(SortPass, keys),
            std::mem::offset_of!(SortPass, values),
            std::mem::offset_of!(SortPass, hist),
            std::mem::offset_of!(SortPass, out_keys),
            std::mem::offset_of!(SortPass, out_values),
        ];
        words(&self.sorts.buffer, std::mem::size_of::<SortPass>(), &sort_words);
        // Its size scripts' absolute addresses (a live relation's length, `Job::rows`), in the ops an arena's rebase
        // resolves again (`Rebase::ops`) and as resolved (`script`, one `SizeOp` an op).
        let ops = self.script.buffer.contents().cast::<SizeOp>();
        for (i, o) in self.rebase.ops.iter_mut().enumerate() {
            for (late, field) in [(&mut o.w, std::mem::offset_of!(SizeOp, w)), (&mut o.c, std::mem::offset_of!(SizeOp, c))] {
                if let Late::Abs(a) = late
                    && *a >= old
                    && *a < old + len
                {
                    *a = *a - old + na;
                    // SAFETY: op `i` of the script's ops, one `SizeOp` each; no work of the program runs.
                    unsafe { ops.add(i).cast::<u8>().add(field).cast::<u64>().write_unaligned(*a) };
                }
            }
        }
        for (b, o) in &self.rooms.relocs {
            // SAFETY: a parameters word that held an address its job reached (`Rooms::relocs`); no work runs.
            unsafe {
                let w = b.contents().cast::<u8>().add(*o as usize).cast::<u64>();
                w.write_unaligned(at(w.read_unaligned()));
            }
        }
        for (b, _) in &mut self.uses {
            if address(b)? == old {
                *b = new.clone();
            }
        }
        Ok(())
    }

    /// Its jobs' descriptors' addresses `old` written `new` (a live index's rows moved in its block).
    fn repoint(&mut self, old: u64, new: u64) {
        const WORDS: usize = std::mem::offset_of!(JobDesc, at) / 8;
        let n = self.descs.buffer.length() as usize / std::mem::size_of::<JobDesc>();
        let descs = self.descs.buffer.contents().cast::<u8>();
        for i in 0..n {
            // SAFETY: descriptor `i`'s addresses, its first `WORDS` words; no work of the program runs.
            let words = unsafe { &mut *descs.add(i * std::mem::size_of::<JobDesc>()).cast::<[u64; WORDS]>() };
            for w in words.iter_mut().filter(|w| **w == old) {
                *w = new;
            }
        }
    }
}

/// A level sort's histograms' bytes for `rows` keys: a tile's 256 counts, and one.
fn sort_hist_bytes(rows: usize) -> usize {
    4 * (256 * rows.div_ceil(SORT_TILE).max(1) + 1)
}

/// The radix sort's tile (`UKL_RADIX_TILE`): a group's rows of a pass.
const SORT_TILE: usize = 1024;
/// `join_stages.uglykl`'s `IX_LIVE`: an index's rows its relation's length on the device.
const IX_LIVE: u32 = 0x8000_0000;
/// The scan's tile (`UKL_SCAN_TILE`): a group's values.
const SCAN_TILE: usize = 1024;

// What a program's item runs (`join_stages.uglykl`'s `PK_*`).
const PK_COPY: u32 = 1;
const PK_IX_KEY: u32 = 2;
const PK_RADIX_HIST: u32 = 3;
const PK_RADIX_SCATTER: u32 = 4;
/// A sorted dedup's keys (t58 increment A, `dedup_key_one`).
const PK_DEDUP_KEY: u32 = 31;

/// `HeadKeyP`'s level's keys and its passes' parity (its size script's, `flush`'s).
const HK_TOTAL: WordAt = WordAt::word(68);
const HK_ODD: WordAt = WordAt::word(69);

/// A sorted dedup's key (t58 increment A, `join_stages.uglykl`'s `HeadKeyP`): the order's leading column's 16-bit
/// tag where `tagged`, then each part, most significant first: (column of the dedup's `cols`, bits, least).
/// `presorted`: its rows come in its order already (one rule's, sorted by its run's dedup), so its level sorts no
/// keys where every sorted dedup of it says so.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) struct SortKey {
    pub tagged: bool,
    pub parts: Vec<(u32, u32, i64)>,
    pub bits: u32,
    pub presorted: bool,
}

impl SortKey {
    /// Its parts' bits, less its tag: a row's index among every row its bounds allow (`HeldBits`).
    pub(in crate::code_rule_dsl) fn index_bits(&self) -> u32 {
        self.bits - if self.tagged { 16 } else { 0 }
    }

    /// `HeadKeyP`'s words: tagged, its parts, its rank's shift and its rank (its level's, `flush`); each part its
    /// column, bits, least's words.
    fn words(&self) -> [u32; 72] {
        let mut w = [0u32; 72];
        w[0] = u32::from(self.tagged);
        w[1] = self.parts.len() as u32;
        for (i, &(c, bits, lo)) in self.parts.iter().enumerate() {
            w[4 + 4 * i..8 + 4 * i].copy_from_slice(&[c, bits, lo as u64 as u32, (lo as u64 >> 32) as u32]);
        }
        w
    }
}

/// The largest held bitmap's index bits (`HeldBits`): 2^24 rows, 2 MB.
pub(in crate::code_rule_dsl) const HELD_BITS_MAX: u32 = 24;

/// A recursive set head's rows as a bitmap of every row its key's bounds allow (t58 increment A), in its relation's
/// block: a row's bit at its key's index bits (`SortKey::index_bits`, its parts less their least, end to end). Made
/// from its relation's rows at a program's start (`Batch::set_bits`); a sorted dedup less it (`dedup_sorted`) keeps a
/// row whose bit is clear and sets it.
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) struct HeldBits<'a> {
    pub(in crate::code_rule_dsl) home: &'a std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>,
    /// Its first word's byte offset in its block, and its words (`2^bits / 32`, at least one).
    pub(in crate::code_rule_dsl) at: usize,
    pub(in crate::code_rule_dsl) bits: u32,
}

impl HeldBits<'_> {
    /// The words of a bitmap of `bits` index bits.
    pub(in crate::code_rule_dsl) fn words_for(bits: u32) -> usize {
        ((1usize << bits) / 32).max(1)
    }
}
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
const PK_IX_MARK: u32 = 30;
/// The phases whose items are their jobs' rows' (`Job::rows_word`): each kernel's threads past its rows do nothing.
/// Not a row's text (its SIMD groups stride its rows), an index (its sort is of every key), a clear, a fold's map or
/// places, a scan's or a sort's tiles.
const ROWS_ITEMS: [u32; 17] = [PK_COPY, PK_MAP, PK_DEDUP_KEY, PK_DEDUP_CLAIM, PK_STAGE_COUNT, PK_SCAN_COUNT, PK_WITNESS_COUNT, PK_DEDUP_MARK, PK_STAGE_SCATTER, PK_SCAN_SCATTER,
                               PK_ROW_SCATTER, PK_DEDUP_SCATTER, PK_LAT_CLAIM, PK_LAT_BEST, PK_IX_KEY, PK_IX_PLACE, PK_IX_MARK];

/// A job's items in a phase as many as a count on the device needs (`join_stages`' `Seg`).
#[repr(C)]
#[derive(Clone, Copy)]
struct SegAt {
    count: u64,
    first: u32,
    items: u32,
    per: u32,
    plus: u32,
}

const _: () = assert!(std::mem::size_of::<SegAt>() == 24);

/// A batch's program (decision 0603): the levels flushed since its last launch, as phases of items (`Item`: a group
/// of a job's, a scan level's or a sort pass's work), run by one launch of `join_run`. A phase's items follow the
/// phase before's, all of them: the group that finishes a phase pushes the next.
#[derive(Default)]
struct Program {
    descs: Vec<JobDesc>,
    items: Vec<[u32; 4]>,
    /// Each phase's first item, its items, its first segment and its segments.
    phases: Vec<[u32; 4]>,
    /// The phases' segments (`SegAt`): a job's items as many as its rows need.
    segs: Vec<SegAt>,
    scans: Vec<ScanAt>,
    sorts: Vec<SortPass>,
    /// Its levels' size scripts (`size_one`), their ops end to end; the bytes of the arena they claim; its top's word
    /// (made with the first script, emptied at each round's end).
    script: Vec<OpAt>,
    arena: usize,
    top: Option<std::rc::Rc<Words>>,
    /// The tables its size scripts place (their places, given its arena when it launches).
    made: Vec<std::rc::Rc<Place>>,
    /// Segments whose counts are words a size script writes (a scan level's), named late.
    seg_late: Vec<(usize, Late)>,
    /// The view format its jobs read (0: several); none read a view.
    format: Option<u32>,
    levels: u32,
    /// What its jobs reach by address, each with whether they write it.
    uses: Vec<(Buffer, bool)>,
    /// Its levels' own buffers: parameter blocks, the jobs' own, the scans' sums, the sorts'.
    hold: Vec<Held>,
    /// Its rooms mapped as its rows reach them (`Kit::rooms`).
    rooms: Rooms,
}

impl Program {
    /// Its arena's top (`size_one`'s claims), its address: made at its first use.
    fn top(&mut self) -> Result<u64, Error> {
        if self.top.is_none() {
            // Its top, its room and what a parked script wanted, in units (`size_one`'s claim).
            self.top = Some(std::rc::Rc::new(Words::zeroed(3)?));
        }
        address(&self.top.as_ref().expect("made").buffer.buffer)
    }

    fn op(&mut self, o: OpAt) {
        self.script.push(o);
    }

    /// A phase of `items` (`(kind, which, group)`), after the ones before; none for no items.
    fn phase(&mut self, items: Vec<(u32, u32, u32)>) {
        self.phase_of(items, Vec::new());
    }

    /// `phase`, with segments of its items (`SegAt`, their firsts its own): pushed as many as their counts need.
    fn phase_of(&mut self, items: Vec<(u32, u32, u32)>, segs: Vec<SegAt>) {
        if items.is_empty() {
            return;
        }
        let p = self.phases.len() as u32;
        let base = self.items.len() as u32;
        let s0 = self.segs.len() as u32;
        self.phases.push([base, items.len() as u32, s0, segs.len() as u32]);
        // Each item with its segment and one (0: none), which it strides its job's rows by.
        let mut seg_of = vec![0u32; items.len()];
        for (k, g) in segs.iter().enumerate() {
            seg_of[g.first as usize..(g.first + g.items) as usize].fill(s0 + k as u32 + 1);
        }
        self.segs.extend(segs.into_iter().map(|g| SegAt { first: base + g.first, ..g }));
        self.items.extend(items.into_iter().zip(seg_of).map(|((kind, which, group), seg)| [kind << 24 | which, group, p, seg]));
    }

    /// A level's counts' scan, its sizes its size script's (decision 0603 step 5, `level_sizes`): three levels (its
    /// values, its tiles' sums, theirs: one tile of at most `SCAN_TILE` sums, as the arena holds at most 2^30 words),
    /// each level's values, sums and count set by the script, its tiles' items as many as its count needs, each
    /// striding its tiles past those encoded (as many as `n`'s, the values guessed). Its first level.
    fn scan_sized(&mut self, n: usize) -> usize {
        let first = self.scan_levels_sized(ScanData::Script, n);
        self.scan_sized_phases(first, n);
        first
    }

    /// `scan_sized`'s levels, its values at `data` (the arena's: `Script`), with no phases: its first.
    fn scan_levels_sized(&mut self, data: ScanData, n: usize) -> usize {
        let first = self.scans.len();
        let t0 = n.div_ceil(SCAN_TILE).max(1);
        let t1 = t0.div_ceil(SCAN_TILE).max(1);
        for (k, m) in [(0, n), (1, t0), (2, t1)] {
            self.scans.push(ScanAt { data: if k == 0 { data } else { ScanData::Script }, sums: (k < 2).then_some(ScanData::Script), n: m as u32, offsets: k < 2 });
        }
        first
    }

    /// The phases of a scan whose levels are `scan_levels_sized`'s from `first`, `n` values guessed: as many tiles'
    /// items as each level's count, set by its size script, needs.
    fn scan_sized_phases(&mut self, first: usize, n: usize) {
        let t0 = n.div_ceil(SCAN_TILE).max(1);
        let t1 = t0.div_ceil(SCAN_TILE).max(1);
        let n_word = word_at!(ScanLevel, w.n);
        let tiles = |p: &mut Program, kind: u32, level: usize, items: usize| {
            p.phase_of((0..items).map(|g| (kind, (first + level) as u32, g as u32)).collect(), vec![SegAt { count: 0, first: 0, items: items as u32, per: SCAN_TILE as u32, plus: 0 }]);
            p.seg_late.push((p.segs.len() - 1, Late::Scan(first + level, n_word)));
        };
        tiles(self, PK_SCAN_SUMS, 0, t0);
        tiles(self, PK_SCAN_SUMS, 1, t1);
        self.phase(vec![(PK_SCAN_TILES, (first + 2) as u32, 0)]);
        tiles(self, PK_SCAN_TILES, 1, t1);
        tiles(self, PK_SCAN_TILES, 0, t0);
    }

    /// An exclusive scan of the `n` values at `data` in place, with their total after them, as phases: each level's
    /// tiles' sums up to a level of one tile, that tile scanned, then each level's tiles from their offsets
    /// (`ukl/scan.h`'s plan, its passes `ukl/body/scan_tile.h`'s).
    fn scan(&mut self, data: ScanData, n: usize) -> Result<(), Error> {
        let mut levels: Vec<(ScanData, usize, Option<(u64, usize)>)> = Vec::new();
        let (mut d, mut m) = (data, n);
        loop {
            let tiles = m.div_ceil(SCAN_TILE).max(1);
            if tiles == 1 {
                levels.push((d, m, None));
                break;
            }
            let sums = Held::new(4 * (tiles + 1))?;
            let sa = address(&sums.buffer)?;
            self.hold.push(sums);
            levels.push((d, m, Some((sa, tiles))));
            (d, m) = (ScanData::At(sa), tiles);
        }
        let mut at = Vec::with_capacity(levels.len());
        for &(d, m, sums) in &levels {
            at.push(self.scans.len() as u32);
            self.scans.push(ScanAt { data: d, sums: sums.map(|s| ScanData::At(s.0)), n: m as u32, offsets: sums.is_some() });
        }
        for (k, &(_, _, sums)) in levels.iter().enumerate() {
            if let Some((_, tiles)) = sums {
                self.phase((0..tiles).map(|g| (PK_SCAN_SUMS, at[k], g as u32)).collect());
            }
        }
        for (k, &(_, _, sums)) in levels.iter().enumerate().rev() {
            let tiles = sums.map_or(1, |s| s.1);
            self.phase((0..tiles).map(|g| (PK_SCAN_TILES, at[k], g as u32)).collect());
        }
        Ok(())
    }
}

/// Where a scan level's values lie: where its level's size script claimed them (a level's counts), or at an address.
#[derive(Clone, Copy)]
enum ScanData {
    Script,
    At(u64),
}

impl Late {
    /// Its address, the program's buffers at `da` (descriptors), `sa` (scan levels), `xa` (sort passes) and `ca` (the
    /// arena).
    fn at(self, da: u64, sa: u64, xa: u64, ca: u64) -> u64 {
        match self {
            Late::Zero => 0,
            Late::Abs(a) => a,
            Late::Desc(i, o) => da + (std::mem::size_of::<JobDesc>() * i) as u64 + o.0,
            Late::Scan(i, o) => sa + (std::mem::size_of::<ScanLevel>() * i) as u64 + o.0,
            Late::Sort(i, o) => xa + (std::mem::size_of::<SortPass>() * i) as u64 + o.0,
            Late::Arena(o) => ca + o,
            Late::Pending(..) => unreachable!("a scan's word named before its scan was made"),
        }
    }
}

/// An address a size script names, known when its program launches (`run_program`): none, an address, a job's
/// descriptor's word, a scan level's, the arena's.
#[derive(Clone, Copy)]
enum Late {
    Zero,
    Abs(u64),
    /// A word of job `.0`'s descriptor (`word_at!`).
    Desc(usize, WordAt),
    /// A word of scan level `.0` (`word_at!`).
    Scan(usize, WordAt),
    /// A word of sort pass `.0` (`word_at!`).
    Sort(usize, WordAt),
    Arena(u64),
    /// A word of the level's scan's level `.0` (`Scan` once the scan is made).
    Pending(usize, WordAt),
}

/// A size script's op as it is built (`size_one`'s `SizeOp`, its addresses late).
#[derive(Clone, Copy)]
struct OpAt {
    w: Late,
    c: Late,
    op: u32,
    k: u32,
    x: u32,
    y: u32,
}

impl OpAt {
    fn of(op: u32, k: u32, x: u32, y: u32) -> OpAt {
        OpAt { w: Late::Zero, c: Late::Zero, op, k, x, y }
    }
    fn w(self, w: Late) -> OpAt {
        OpAt { w, ..self }
    }
    fn c(self, c: Late) -> OpAt {
        OpAt { c, ..self }
    }
}

/// `join_stages.uglykl`'s `SizeOp`, byte for byte.
#[repr(C)]
#[derive(Clone, Copy)]
struct SizeOp {
    w: u64,
    c: u64,
    op: u32,
    k: u32,
    x: u32,
    y: u32,
}

const _: () = assert!(std::mem::size_of::<SizeOp>() == 32);

// `size_one`'s ops.
const SZ_LOAD: u32 = 0;
const SZ_CONST: u32 = 2;
const SZ_ADD: u32 = 3;
const SZ_SUB: u32 = 4;
const SZ_MUL: u32 = 5;
const SZ_DIVUP: u32 = 6;
const SZ_MIN: u32 = 7;
const SZ_MAX: u32 = 8;
const SZ_CLAIM: u32 = 9;
const SZ_PUT32: u32 = 11;
const SZ_LOADX: u32 = 12;
const SZ_POW2: u32 = 13;
/// A register no op writes: 0.
const ZERO: u32 = 15;
/// A program's arena at most: its counts' words and its placed tables' 8-byte values are offsets of 32 bits.
const ARENA_MAX: usize = 4 << 30;

/// A program's scan level, as it is built: its values, its tiles' sums (none for one tile), whether they offset it.
struct ScanAt {
    data: ScanData,
    sums: Option<ScanData>,
    n: u32,
    offsets: bool,
}

/// `join_stages.uglykl`'s `ScanLevel`, byte for byte.
#[repr(C)]
#[derive(Clone, Copy)]
struct ScanLevel {
    data: u64,
    sums: u64,
    w: ScanWords,
}

/// `join_stages.uglykl`'s `ScanWords`, byte for byte: a word record (its `UKL_WORD_RECORD`).
#[repr(C)]
#[derive(Clone, Copy)]
struct ScanWords {
    n: u32,
    offsets: u32,
    /// Its values' and its sums' first words past `data` and `sums` (a size script's).
    at: u32,
    sums_at: u32,
}

const _: () = assert!(std::mem::size_of::<ScanLevel>() == 32);

/// `join_stages.uglykl`'s `SortPass`, byte for byte.
#[repr(C)]
#[derive(Clone, Copy)]
struct SortPass {
    keys: u64,
    values: u64,
    hist: u64,
    out_keys: u64,
    out_values: u64,
    w: SortWords,
    pad2: u64,
}

/// `join_stages.uglykl`'s `SortWords`, byte for byte: a word record (its `UKL_WORD_RECORD`). Its keys and tiles are
/// its level's size script's (decision 0603 step 5e).
#[repr(C)]
#[derive(Clone, Copy)]
struct SortWords {
    n: u32,
    shift: u32,
    tiles: u32,
    /// Its keys are 64 bits (the level's wide sort, a sorted dedup's: t58 increment A), else 32.
    wide: u32,
}

const _: () = assert!(std::mem::size_of::<SortPass>() == 64);

/// A program launched (`Batch::run_program`): its control words, its last phase and that phase's items.
struct Ran {
    ctl: Words,
    last: usize,
    phases: usize,
    items: u32,
    kit: Kit,
}

impl Ran {
    /// Its parked item (a size script's, `ctl[9]`) put back (`ctl[2]`: the next chain's start runs it alone), and its
    /// park flag cleared.
    fn put_back(&self) {
        self.ctl.set(2, self.ctl.get(9));
        self.ctl.set(9, 0);
        self.ctl.set(6, self.ctl.get(6) & !32);
    }
}

/// A chain's dispatches past its start at least (decisions 0603 point 5, 0674): a program of few phases still runs
/// the rounds of a few phases a launch. A launch's GPU time is its dispatches', whatever its rules and rounds, and the
/// host launches it again while phases are left.
pub(in crate::code_rule_dsl) const PROGRAM_BUDGET: u32 = 16;

/// A chain's dispatches past its start at most: its encoding's host time and its GPU time bounded.
const PROGRAM_DISPATCHES: u32 = 1024;

/// Whether a recursive component's rooms are mapped as its rounds reach them (decision 0603's fewer reruns, its gate's
/// rooms: `Batch::gate_rooms`): where a mapped page is real memory (`rerail_device::device::map_commits`), its rows
/// past its rows now and twice a round's (0); a test build's `RERAIL_ROOMS_MAPPED=N` maps them on any device, `N`
/// rows past its rows first, so the CPU backend's tests run the mechanism.
pub(in crate::code_rule_dsl) fn rooms_mapped() -> Option<usize> {
    #[cfg(any(test, feature = "test-hooks"))]
    if let Some(n) = crate::code_rule_dsl::logic::profile::var("RERAIL_ROOMS_MAPPED").ok().and_then(|v| v.parse::<usize>().ok()) {
        return Some(n);
    }
    rerail_accel::gpu::join::device().is_ok_and(rerail_device::device::map_commits).then_some(0)
}

/// Whether mapping a reserved arena's pages commits real memory on this run's device (Metal's do;
/// `rerail_device::device::map_commits`).
#[cfg(test)]
pub(in crate::code_rule_dsl) fn map_commits() -> bool {
    rerail_accel::gpu::join::device().is_ok_and(rerail_device::device::map_commits)
}

/// A program's sizes where a test sets them: its chain's dispatches (`PROGRAM_BUDGET` at least, else), and its first arena's
/// bytes (twice what its claims were guessed to take, else), so a test's launches go again and its arena's pages are
/// mapped as they would be at a larger scale. A test build's switches (`RERAIL_PROGRAM_BUDGET`,
/// `RERAIL_PROGRAM_ARENA`, set through `switches::set`); none in any other build.
#[derive(Clone, Copy, Default)]
struct ProgramSizes {
    budget: Option<u32>,
    arena: Option<usize>,
}

impl ProgramSizes {
    fn now() -> ProgramSizes {
        #[cfg(any(test, feature = "test-hooks"))]
        {
            fn var<T: std::str::FromStr>(name: &str) -> Option<T> {
                crate::code_rule_dsl::logic::profile::var(name).ok().and_then(|v| v.parse().ok())
            }
            ProgramSizes { budget: var("RERAIL_PROGRAM_BUDGET"), arena: var("RERAIL_PROGRAM_ARENA") }
        }
        #[cfg(not(any(test, feature = "test-hooks")))]
        ProgramSizes::default()
    }
}

/// A buffer's address for the device's kernels (decision 0601's jobs): a device whose buffers have none
/// (OpenCL without unified shared memory) runs no join on the device, as a view read in place needs one too.
fn address(b: &Buffer) -> Result<u64, Error> {
    b.device_address().ok_or(Error::UnsupportedBackend)
}

/// `join_stages.uglykl`'s `Job`, byte for byte: a queued launch's buffers by device address, its parameter blocks'
/// (`flush` places them), and its counts' place in its level's.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct JobDesc {
    l: u64,
    r: u64,
    totals: u64,
    counters: u64,
    o: u64,
    a: u64,
    d: u64,
    f: u64,
    w: [u64; WITNESS_PROBES],
    g: u64,
    s: u64,
    vc: u64,
    v: u64,
    h: u64,
    stage: u64,
    ws: u64,
    bs: u64,
    dd: u64,
    p: u64,
    /// A count an earlier phase wrote, by address (a lattice fold's rows).
    cnt: u64,
    /// Its words, some its level's size script writes (`word_at!`).
    at: JobWords,
}

const _: () = assert!(std::mem::size_of::<JobDesc>() == 208);

/// `join_stages.uglykl`'s `JobWords`, byte for byte: a word record (its `UKL_WORD_RECORD`), 32-bit words only.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct JobWords {
    /// Its counts' first word in the program's arena (its level's size script's) and their number.
    base: u32,
    n: u32,
    /// An index's first bucket and first key in the level's sort (`ix_key_one`).
    kb: u32,
    eb: u32,
    /// Its placed tables' offsets in the arena and one (0: none), its size script's: `l`, `o`, a row's values, a
    /// dedup's places.
    l_at: u32,
    o_at: u32,
    v_at: u32,
    h_at: u32,
}

/// A 32-bit word's place in a record a size script writes or reads (decision 0603 step 5b): its byte offset in it.
/// Made by `word_at!` (or a word array's index, `WordAt::word`), which refuses a field that is not a `u32`: a size
/// script's words are offsets, counts and sizes, never a pointer or an address (the kernel's records refuse them too,
/// `UKL_WORD_RECORD`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct WordAt(u64);

impl WordAt {
    /// Word `i` of a record of words only (`[u32; N]`).
    const fn word(i: u64) -> WordAt {
        WordAt(4 * i)
    }

    /// The same word in a record `bytes` on (the `k`-th of an array of records).
    const fn past(self, bytes: u64) -> WordAt {
        WordAt(self.0 + bytes)
    }
}


/// What a queued job runs (`Batch::flush`'s phases; `join_stages.uglykl`'s `*_one` bodies).
#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    /// Its count, the level's scan, its scatter (`stage_count_one`, `stage_scatter_one`).
    Stage,
    /// A join from one binding (a rule's first table), a thread per candidate row (`scan_count_one`,
    /// `scan_scatter_one`).
    Scan,
    /// Its count (`witness_count_one`), the scan, the scatter of its write (a semi-join's).
    Witness,
    /// `row_text_one` (a SIMD group a row), the scan, `row_scatter_one`.
    Row,
    /// `stage_map_one`.
    Map,
    /// Its places cleared, claimed, its rows marked, the scan, scattered (`dedup_*_one`).
    Dedup,
    /// `rows_copy_one`, first in its level.
    Copy,
    /// Its keys (`ix_key_one`), the level's one sort of them, its places (`ix_place_one`): after the copies, before
    /// the rest.
    Index,
    /// A lattice relation's round (`Batch::lattice_fold`): its key map emptied and grown, its places emptied, claimed,
    /// the best row a key, merged (`lat_*_one`), after the level's scatters.
    Lattice,
    /// A union's sources' offsets and total (`sum_one`), first in its level.
    Sum,
    /// A gate (`gate_one`), alone in its level.
    Gate,
    /// A round's end (`round_one`), alone in its level.
    Round,
    /// A placed table its level's size script places (`Job::o_rows`): no items of its own (a union's, its rows its
    /// sources' total: `Batch::append_rows`).
    Place,
}

impl JobKind {
    /// Its place in its level's launches: the copies, the indexes, the rest.
    fn phase(self) -> u8 {
        match self {
            JobKind::Copy => 0,
            JobKind::Index => 1,
            _ => 2,
        }
    }
}

/// A launch queued in a batch's level (`Batch::queue`).
struct Job {
    kind: JobKind,
    /// The view format its kernels are compiled for (`views_format`); 0 for the kernels that read no view.
    format: u32,
    desc: JobDesc,
    /// Its parameter blocks: its stage, its scatter's (a witness's write), its probes, its dedup's, its row's words.
    stage: Stage,
    write: Option<Stage>,
    probes: Option<[Stage; WITNESS_PROBES]>,
    dedup: Option<[u32; 32]>,
    /// Its parameters read through `P`: a row's words, a copy's, an index's.
    p: Option<Vec<u8>>,
    /// Its counts' values (0: none); its grids' threads: the count's (a map's, a copy's, an index's; a row's, its
    /// SIMD groups), the scatter's, a dedup's clear's and claim's.
    n: usize,
    count: usize,
    scatter: usize,
    clear: usize,
    claim: usize,
    /// An index's buckets, its keys' span in the level's sort.
    buckets: usize,
    /// A sorted dedup's (t58 increment A): its rows at most, its keys' span in the level's wide sort; and its key's
    /// bits, below its rank's. 0: not one.
    wide: usize,
    key_bits: u32,
    /// Its rows come in its key's order (`SortKey::presorted`).
    presorted: bool,
    /// A lattice fold's key map's slots to empty, and its old map's to place again (0: none).
    lattice: [usize; 2],
    /// A copy's rows written: its destination's address, and its rows from and to.
    span: Option<(u64, usize, usize)>,
    /// A copy's or a fold's rows' count on the device, and the threads past it its grids take (`Job::rows_word`).
    rows: Option<(u64, u32)>,
    /// A live index's relation's first row (`IxP::from`): its rows are its length past it, at most `count`.
    rows_from: u32,
    /// The buffers it reaches by address, each with whether it writes them.
    uses: Vec<(Buffer, bool)>,
    /// The tables it reads and writes, by address: its order against the jobs queued before it.
    reads: Vec<u64>,
    writes: Vec<u64>,
    /// Buffers of its own (a row's values and scratch, a dedup's places), kept until its level is launched.
    own: Vec<Held>,
    /// Its placed tables (decision 0603 step 5): its input (`l`; a map's table, `o_reads`), its output (`o`), and a
    /// placed table it writes into, placed before (`d`, a union a copy appends to), with the words of its parameter
    /// blocks that hold their rows' room; a row's values and a dedup's or a fold's places placed as its input is
    /// (`v_place`, `h_place`).
    l_place: Option<std::rc::Rc<Place>>,
    o_place: Option<std::rc::Rc<Place>>,
    o_reads: bool,
    d_place: Option<std::rc::Rc<Place>>,
    in_caps: Vec<(usize, WordAt)>,
    out_caps: Vec<(usize, WordAt)>,
    d_caps: Vec<(usize, WordAt)>,
    out_slots: usize,
    /// The addresses it reached (`reach`): its parameters' words that hold one are found again where a block it
    /// reaches is wrapped again (`Relocs`).
    reached: Vec<u64>,
    /// A gate's rooms (`Batch::gate_rooms`): its check, and what grows its room.
    rooms: Vec<(usize, RoomGrow)>,
    /// A live relation's index build (`queue_index`): its relation's block, its relation's offset there, its entry.
    live_ix: Option<(std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>, u64, usize, u32)>,
    /// Its output placed by its level's size script at its start, its rows a word's or its input's room, not
    /// after the level's scan (`made_sizes`): a union's (`Batch::append_rows`), a fold's changed rows.
    o_rows: Option<OutRows>,
    v_place: bool,
    h_place: Option<HPlace>,
    /// Its rows counted: the word that holds them (none: all), at most its input's room where that is not placed.
    m: (Option<u64>, usize),
    /// Its placed output's rows as guessed (its arena's bytes).
    out_guess: usize,
}

/// A `Stage`'s words its size script writes (`Job::in_caps`, `out_caps`).
const CAP_IN: WordAt = word_at!(Stage, cap_in);
const CAP_OUT: WordAt = word_at!(Stage, cap_out);
/// A dedup's (`Job::dedup`, its words): its rows' stride, its mask, its distinct rows' stride, its places.
const DD_CAP: WordAt = WordAt::word(0);
const DD_MASK: WordAt = WordAt::word(4);
const DD_OCAP: WordAt = WordAt::word(21);
const DD_PLACES: WordAt = WordAt::word(22);
/// A copy's (`CopyParams`): its rows at most, its source's stride, its destination's.
const CP_N: WordAt = word_at!(CopyParams, n);
const CP_SRC_STRIDE: WordAt = word_at!(CopyParams, src_stride);
const CP_DST_STRIDE: WordAt = word_at!(CopyParams, dst_stride);
/// A fold's (`LatParams`): its rows' room (its union's, its columns' stride), its places' mask.
const LP_N: WordAt = word_at!(LatParams, n);
const LP_PMASK: WordAt = word_at!(LatParams, pmask);

/// Where a job's output placed at its level's start takes its rows from (`Job::o_rows`).
#[derive(Clone, Copy)]
enum OutRows {
    /// The word at this address, an earlier level's (a union's sources' total).
    Word(u64),
    /// Its input's room (a fold's changed rows: its union's at most).
    InRoom,
}

/// Places a job's size script claims with its input's room (decision 0603 step 5d): `arrays` arrays of 32-bit
/// words, each of a power of two of twice its input's rows (16 at least), in its parameter block `block`, whose
/// words `places` (their number) and `mask` (it less one) the script writes.
#[derive(Clone, Copy)]
struct HPlace {
    block: usize,
    places: Option<WordAt>,
    mask: WordAt,
    arrays: u64,
}

impl Job {
    fn new(kind: JobKind, format: u32, stage: Stage) -> Job {
        Job {
            kind,
            format,
            desc: JobDesc::default(),
            stage,
            write: None,
            probes: None,
            dedup: None,
            p: None,
            n: 0,
            count: 0,
            scatter: 0,
            clear: 0,
            claim: 0,
            buckets: 0,
            wide: 0,
            key_bits: 0,
            presorted: false,
            lattice: [0; 2],
            span: None,
            rows: None,
            rows_from: 0,
            uses: Vec::new(),
            reads: Vec::new(),
            writes: Vec::new(),
            own: Vec::new(),
            l_place: None,
            o_place: None,
            o_reads: false,
            d_place: None,
            d_caps: Vec::new(),
            o_rows: None,
            in_caps: Vec::new(),
            out_caps: Vec::new(),
            out_slots: 0,
            v_place: false,
            h_place: None,
            m: (None, 0),
            out_guess: 0,
            reached: Vec::new(),
            rooms: Vec::new(),
            live_ix: None,
        }
    }

    /// Its input `t`, its rows' room in its parameters' words `caps`, where `t` is placed.
    fn reads_placed(&mut self, t: &Table, caps: &[(usize, WordAt)]) {
        if let Some(p) = t.place.as_ref().filter(|_| t.placed()) {
            self.l_place = Some(p.clone());
            self.in_caps.extend_from_slice(caps);
        }
    }

    /// Its output `t`, its rows' room in its parameters' words `caps`, where `t` is placed: its size script places
    /// it after its level's scan.
    fn makes_placed(&mut self, t: &Table, caps: &[(usize, WordAt)]) {
        if let Some(p) = &t.place {
            self.o_place = Some(p.clone());
            self.out_caps.extend_from_slice(caps);
            self.out_slots = t.slots.max(1);
            self.out_guess = t.cap;
        }
    }

    /// A placed table `t` it writes into, placed before it (a union a copy appends to), its rows' room in its
    /// parameters' words `caps`.
    fn writes_placed(&mut self, t: &Table, caps: &[(usize, WordAt)]) {
        if let Some(p) = t.place.as_ref().filter(|_| t.placed()) {
            self.d_place = Some(p.clone());
            // copy: not block data: the words of its parameter blocks its size script writes.
            self.d_caps.extend_from_slice(caps);
        }
    }

    /// `b`'s address `at` bytes in, which the job reaches (and writes, when `write`).
    fn reach(&mut self, b: &Buffer, at: u64, write: bool) -> Result<u64, Error> {
        let a = address(b)?;
        self.uses.push((b.clone(), write));
        self.reached.push(a + at);
        Ok(a + at)
    }

    /// Where its rows' count lies on the device, and the threads past it its grids take: its phases' items are that
    /// count's (`Seg`, decision 0603 step 5). Its input table's (`totals[in]`), a live relation's length (a scan of
    /// it), a copy's or a fold's count; none where its grids are its bound's.
    fn rows_word(&self) -> Option<(u64, u32)> {
        match self.kind {
            JobKind::Stage | JobKind::Witness | JobKind::Row => Some((self.desc.totals + 4 * u64::from(self.stage.in_slot), 1)),
            JobKind::Map => Some((self.desc.totals + 4 * u64::from(self.stage.in_slot), 0)),
            JobKind::Scan if self.stage.r_len != 0 => Some((self.desc.r + self.stage.r_at + REL_ROWS + ROWS_LEN, 1)),
            JobKind::Dedup => self.dedup.map(|d| (self.desc.totals + 4 * u64::from(d[1]), 1)),
            JobKind::Copy | JobKind::Lattice | JobKind::Index => self.rows,
            _ => None,
        }
    }

    /// The tables it reads and writes (their addresses).
    fn order(&mut self, reads: &[u64], writes: &[u64]) {
        // copy: not block data: the tables' addresses.
        self.reads.extend_from_slice(reads);
        self.writes.extend_from_slice(writes);
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
        // Its indexes, never marked built: no batch takes one from now on (their entries stay reserved).
        for b in self.building.drain(..) {
            b.forget(&mut b.home.ix.lock().unwrap_or_else(|e| e.into_inner()));
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

    /// B023's closures (2026-11-22): two batches that each found no index of a relation built one, and the second's
    /// went back when it was done (`finish`), its buckets emptied, so a probe of it finds no rows. A stage that probed
    /// it, encoded again in a later batch (an overflow's stages run again, `setfix::device::complete`), finds the
    /// relation's kept index instead (`Batch::again`), and builds none.
    #[test]
    fn an_index_built_twice_is_found_again_where_it_is_kept() {
        if rerail_accel::gpu::join::device().is_err() {
            return;
        }
        // Its batches' launches are an evaluation's (decision 0601).
        let _evaluation = rerail_accel::gpu::launch::LaunchScope::enter();
        let n = 300;
        let keys = rows(n, 11, 40);
        let rel = DevRel::empty_indexed(1, n, 2).unwrap();
        let r = rel.part.as_ref().expect("its own").rel();
        for (i, &v) in keys.iter().enumerate() {
            // SAFETY: row `i` is claimed (`for_device`), and nothing else writes the relation yet.
            unsafe { r.rows.put(i as u32, 0, v) };
        }
        let (mut first, mut second) = (Batch::new().unwrap(), Batch::new().unwrap());
        let kept = first.index(&rel, &[0]).unwrap();
        let lost = std::rc::Rc::new(second.index(&rel, &[0]).unwrap());
        assert!(kept.built && lost.built && kept.index != lost.index, "each batch built its own");
        first.finish().unwrap();
        second.finish().unwrap();
        let r = rel.relation();
        assert_eq!(r.find_index_from(&[0], 0, n as u32), Some(kept.index as usize), "the first's is kept");
        let gone = &r.indexes.as_slice()[lost.index as usize];
        assert_eq!(gone.ncols, 0, "the second's went back");
        let mut later = Batch::new().unwrap();
        let found = later.again(&rel, &lost).unwrap();
        assert_eq!(found.index, kept.index, "found again: the index kept");
        assert!(!found.built, "none built again");
        later.finish().unwrap();
    }

    /// B023's closures, the race (2026-11-22): batch `a` builds an index another batch `c` built too, and batch `b`
    /// takes `a`'s, its work committed. `c` finishes first, so at `a`'s finish the index is built twice; `a`'s is
    /// `b`'s, whose work runs after it, and is kept (built), not given back, whose room the block then hands out while
    /// `b` reads it.
    #[test]
    fn an_index_built_twice_that_a_batch_took_is_kept() {
        if rerail_accel::gpu::join::device().is_err() {
            return;
        }
        let n = 300;
        let keys = rows(n, 13, 40);
        let rel = DevRel::empty_indexed(1, n, 3).unwrap();
        let r = rel.part.as_ref().expect("its own").rel();
        for (i, &v) in keys.iter().enumerate() {
            // SAFETY: row `i` is claimed (`for_device`), and nothing else writes the relation yet.
            unsafe { r.rows.put(i as u32, 0, v) };
        }
        let _evaluation_batch = rerail_accel::gpu::launch::LaunchScope::enter();
        let (mut a, mut b, mut c) = (Batch::new().unwrap(), Batch::new().unwrap(), Batch::new().unwrap());
        let other = c.index(&rel, &[0]).unwrap();
        let built = a.index(&rel, &[0]).unwrap();
        assert!(other.built && built.built && other.index != built.index, "neither committed: each built its own");
        a.commit_open().unwrap();
        let taken = b.index(&rel, &[0]).unwrap();
        assert!(!taken.built && taken.index == built.index, "b took a's");
        c.finish().unwrap();
        a.finish().unwrap();
        let r = rel.relation();
        assert_eq!(r.find_index_from(&[0], 0, n as u32), Some(other.index as usize), "c's is found first");
        assert_eq!(r.index_at(built.index as usize).ncols, 1, "a's, which b took, is kept built");
        b.finish().unwrap();
    }


    /// Decision 0491 steps 3 and 5: a device relation's block is held by its own region, which a kernel's bind pins by
    /// the kernel's declared access: a copy into a relation moves its block's version once the batch's work is done,
    /// and the block it reads from, bound read-only, keeps its version.
    #[test]
    fn a_kernels_write_moves_its_blocks_version_and_a_read_does_not() {
        if rerail_accel::gpu::join::device().is_err() {
            return;
        }
        let n = 300;
        let src = DevRel::upload(2, n, |v| for (i, x) in v.iter_mut().enumerate() { *x = i as i64 }).unwrap();
        let dst = DevRel::empty(2, n).unwrap();
        let (sb, db) = (src.home.clone().unwrap(), dst.home.clone().unwrap());
        let (s0, d0) = (rerail_blocks::region::whole(&sb.block).region().version(), rerail_blocks::region::whole(&db.block).region().version());
        let _evaluation_batch = rerail_accel::gpu::launch::LaunchScope::enter();
        let mut batch = Batch::new().unwrap();
        batch.copy_rows(Rows::Rel(&src), n, &[Col::From(0), Col::From(1)], Rows::Rel(&dst), 0).unwrap();
        batch.finish().unwrap();
        let d1 = rerail_blocks::region::whole(&db.block).region().version();
        assert!(d1 > d0, "the written block's version moved ({d0} to {d1})");
        if !std::sync::Arc::ptr_eq(&sb, &db) {
            assert_eq!(rerail_blocks::region::whole(&sb.block).region().version(), s0, "the block only read keeps its version");
        }
    }

    /// Q265: a relation `Batch::index` moves for an index (its directory full, or its block) is the batch's until its
    /// work, which builds the index into it, completes: a stage's handle to it dropped meanwhile does not give back
    /// its relation, and the index the batch marks built is still its own. The batch holds it as the index it builds
    /// (`Building`, which `finish` checks against the descriptor as it was reserved); its buffers, as every launch's,
    /// its passes' fences hold (decision 0491 step 2).
    #[test]
    fn a_relation_moved_for_an_index_lives_until_its_batch_is_done() {
        if rerail_accel::gpu::join::device().is_err() {
            return;
        }
        let n = 1000;
        let cols: Vec<Vec<i64>> = (0..4).map(|c| rows(n, 11 + c, 50)).collect();
        let rel = DevRel::upload(4, n, |v| for (k, c) in cols.iter().enumerate() { v[k * n..(k + 1) * n].copy_from_slice(c) }).unwrap();
        let _evaluation_batch = rerail_accel::gpu::launch::LaunchScope::enter();
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
        assert_eq!(std::rc::Rc::strong_count(&moved), 1, "no handle of the stage's is left but this one");
        drop(moved);
        // A relation made meanwhile takes the room a moved relation given back would have left.
        let _after = DevRel::upload(4, n, |v| v.fill(-1)).unwrap();
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
        let _evaluation_batch = rerail_accel::gpu::launch::LaunchScope::enter();
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
        batch.stage(scan, &t0, Some((&ra, None)), None, 0, 1, &totals, &counters, None, &t1).unwrap();
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
        batch.stage(join, &t1, Some((&rb, Some(&ib))), None, 1, 2, &totals, &counters, Some(0), &t2).unwrap();
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
        batch.stage(join, &t1, Some((&rb, Some(&ib))), None, 1, 2, &totals, &counters, None, &t2).unwrap();
        let t3 = Table::new(slots, want2.len()).unwrap();
        let mut anti = Stage { mode: ANTI, nkeys: 1, ..Stage::default() };
        anti.key_kind[0] = SLOT;
        anti.key_slot[0] = 2;
        batch.stage(anti, &t2, Some((&rc, Some(&ic))), None, 2, 3, &totals, &counters, None, &t3).unwrap();
        let t4 = Table::new(slots, want2.len()).unwrap();
        let mut semi = Stage { mode: SEMI, nkeys: 1, ..Stage::default() };
        semi.key_kind[0] = SLOT;
        semi.key_slot[0] = 0;
        semi.key_rcol[0] = 1;
        batch.stage(semi, &t3, Some((&rb, Some(&ib1))), None, 3, 4, &totals, &counters, None, &t4).unwrap();
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

    /// Decision 0488: a stage screened by its relation's kept filter (a `bloom::Bloom` in a block of the run's
    /// relations, bound where it lies) makes the rows it makes unscreened: a join, an anti-join and a semi-join, each
    /// keyed by a column whose filter holds every key of it. A filter of none of the keys refuses every binding: the
    /// join and the semi-join make no rows, the anti-join keeps them all (the kernels test it).
    #[test]
    fn a_screened_stage_makes_the_unscreened_rows() {
        let Ok(device) = rerail_accel::gpu::join::device() else { return };
        let (na, nb, nc) = (3000, 5000, 700);
        let (a0, a1) = (rows(na, 1, 4000), rows(na, 2, 3000));
        let (b0, b1) = (rows(nb, 3, 300), rows(nb, 4, 4000));
        let c0 = rows(nc, 5, 4000);
        let up = |cols: &[&Vec<i64>]| {
            let n = cols[0].len();
            // copy: not block data (a test's literal rows)
            DevRel::upload(cols.len(), n, |v| for (k, c) in cols.iter().enumerate() { v[k * n..(k + 1) * n].copy_from_slice(c) }).unwrap()
        };
        let screen = |keys: &[i64], key: u32| {
            let f = crate::code_rule_dsl::facts::rows::FactsBloom::new(crate::code_rule_dsl::logic::bloom::kept_words_for(keys.len()));
            for &k in keys {
                f.add_hashed(rerail_blocks::bloom::hash(k));
            }
            let at = f.at() as u32;
            let buffer = f.facts().block.device_buffer(device).unwrap();
            Screen { held: std::rc::Rc::new(Held::kept(buffer, Box::new(f))), at, key }
        };
        let (ra, rb, rc) = (up(&[&a0, &a1]), up(&[&b0, &b1]), up(&[&c0]));
        let (sb0, sb1, sc0, none) = (screen(&b0, 0), screen(&b1, 0), screen(&c0, 0), screen(&[], 0));
        let run = |join_screen: Option<&Screen>, anti_screen: Option<&Screen>, semi_screen: Option<&Screen>| {
            let _evaluation_batch = rerail_accel::gpu::launch::LaunchScope::enter();
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
            // copy: not block data (a test's literal stage)
            scan.bind_slot[..2].copy_from_slice(&[0, 1]);
            scan.bind_kind[..2].copy_from_slice(&[BIND_RIGHT, BIND_RIGHT]);
            scan.bind_idx[..2].copy_from_slice(&[0, 1]);
            batch.stage(scan, &t0, Some((&ra, None)), None, 0, 1, &totals, &counters, None, &t1).unwrap();
            let cap = 4 * na;
            let t2 = Table::new(slots, cap).unwrap();
            let mut join = Stage { mode: JOIN, nkeys: 1, nbinds: 1, ..Stage::default() };
            join.key_kind[0] = SLOT;
            join.key_slot[0] = 1;
            join.key_rcol[0] = 0;
            join.bind_slot[0] = 2;
            join.bind_kind[0] = BIND_RIGHT;
            join.bind_idx[0] = 1;
            batch.stage(join, &t1, Some((&rb, Some(&ib))), join_screen, 1, 2, &totals, &counters, None, &t2).unwrap();
            let t3 = Table::new(slots, cap).unwrap();
            let mut anti = Stage { mode: ANTI, nkeys: 1, ..Stage::default() };
            anti.key_kind[0] = SLOT;
            anti.key_slot[0] = 2;
            batch.stage(anti, &t2, Some((&rc, Some(&ic))), anti_screen, 2, 3, &totals, &counters, None, &t3).unwrap();
            let t4 = Table::new(slots, cap).unwrap();
            let mut semi = Stage { mode: SEMI, nkeys: 1, ..Stage::default() };
            semi.key_kind[0] = SLOT;
            semi.key_slot[0] = 0;
            semi.key_rcol[0] = 1;
            batch.stage(semi, &t3, Some((&rb, Some(&ib1))), semi_screen, 3, 4, &totals, &counters, None, &t4).unwrap();
            let done = batch.finish().unwrap();
            let got = |t: &Table, n: usize| (0..n).map(|r| [t.column(0, n)[r], t.column(1, n)[r], t.column(2, n)[r]]).collect::<Vec<_>>();
            assert!((2..5).all(|s| (totals.get(s) as usize) <= cap), "the tables held the rows");
            (got(&t2, totals.get(2) as usize), got(&t3, totals.get(3) as usize), got(&t4, totals.get(4) as usize), done.screened)
        };
        let plain = run(None, None, None);
        assert!(!plain.2.is_empty() && plain.2.len() < plain.1.len() && plain.1.len() < plain.0.len(), "every stage keeps some rows and drops some");
        let screened = run(Some(&sb0), Some(&sc0), Some(&sb1));
        assert_eq!((&screened.0, &screened.1, &screened.2), (&plain.0, &plain.1, &plain.2), "the screened stages' rows");
        assert_eq!((plain.3, screened.3), (0, 3), "three stages screened");
        // A filter of no keys: the join makes nothing; over the plain join's rows, the anti-join keeps them all and
        // the semi-join none.
        let refused = run(Some(&none), None, None);
        assert!(refused.0.is_empty(), "a join whose screen refuses every key");
        let all = run(None, Some(&none), Some(&none));
        assert_eq!(all.1, plain.0, "an anti-join whose screen refuses every key keeps every binding");
        assert!(all.2.is_empty(), "a semi-join whose screen refuses every key keeps none");
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
            let _evaluation_batch = rerail_accel::gpu::launch::LaunchScope::enter();
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
            let (d, w) = (r.index_at(ix.index as usize), h.index_at(hosts));
            if std::ptr::eq(dev, &placed) {
                let end = dev.home.as_ref().unwrap().block.offset_of(d.rows.as_slice().as_ptr()) + 4 * n;
                assert!(end as u64 > placed.buffer.buffer.length(), "the case: the index past the buffer its relation was bound by");
            }
            assert_eq!(d.mask, w.mask);
            assert_eq!(d.offs.as_slice(), w.offs.as_slice(), "{cols:?}: the buckets");
            assert_eq!(d.rows.as_slice(), w.rows.as_slice(), "{cols:?}: each bucket's rows, in id order");
            assert_eq!(d.marks.as_slice(), w.marks.as_slice(), "{cols:?}: each place's mark (decision 0671)");
            let _evaluation_later = rerail_accel::gpu::launch::LaunchScope::enter();
            let mut later = Batch::new().unwrap();
            let found = later.index(dev, &cols).unwrap();
            assert!(!found.built && found.index == ix.index, "a later batch finds it");
            later.finish().unwrap();
        }
    }

    /// An index one batch builds, its work committed and the batch not finished (a level's wave), is taken by another
    /// batch's probe, not built again: one entry in the relation's directory, and the taker's reads are the index's.
    /// One not yet committed is not taken (its work could run after the taker's): the taker builds its own, and
    /// whichever batch finishes second gives its back. A delta read in place has an index of its own rows, not the
    /// first rows' of its count.
    #[test]
    fn a_batch_takes_an_index_another_batch_has_committed() {
        if rerail_accel::gpu::join::device().is_err() {
            return;
        }
        let _evaluation = rerail_accel::gpu::launch::LaunchScope::enter();
        let n = 20_000;
        let (c0, c1) = (rows(n, 41, 97), rows(n, 42, 13));
        let vals: Vec<i64> = (0..n).flat_map(|i| [c0[i], c1[i]]).collect();
        let held = crate::code_rule_dsl::facts::rows::anon_in_block(2, &vals);
        let placed = DevRel::in_block(&held, 2).unwrap();
        let r = placed.relation();
        let count =|| r.nindex.load(std::sync::atomic::Ordering::Relaxed);
        // Committed: taken.
        let mut builder = Batch::new().unwrap();
        let built = builder.index(&placed, &[0]).unwrap();
        assert!(built.built);
        builder.commit_open().unwrap();
        let mut taker = Batch::new().unwrap();
        let taken = taker.index(&placed, &[0]).unwrap();
        assert!(!taken.built && taken.index == built.index, "taken, not built again");
        assert_eq!(count(), 1);
        taker.finish().unwrap();
        builder.finish().unwrap();
        assert_eq!(r.find_index(&[0], n as u32), Some(built.index as usize));
        // Not committed: each builds its own, and the second to finish gives its back.
        let (mut a, mut b) = (Batch::new().unwrap(), Batch::new().unwrap());
        let (ia, ib) = (a.index(&placed, &[1]).unwrap(), b.index(&placed, &[1]).unwrap());
        assert!(ia.built && ib.built && ia.index != ib.index, "an uncommitted build is not taken");
        a.finish().unwrap();
        b.finish().unwrap();
        assert_eq!(r.find_index(&[1], n as u32), Some(ia.index as usize));
        let ix = r.index_at(ib.index as usize);
        assert_eq!((ix.ncols, ix.state), (0, rerail_blocks::relation::DEAD), "the second given back");
        // A delta read in place (its rows `from..`): its own index, though its count is that of the first rows'.
        let half = n / 2;
        let first = DevRel::in_block_from(&held, 2, 0, half).unwrap();
        let delta = DevRel::in_block_from(&held, 2, half, half).unwrap();
        let mut batch = Batch::new().unwrap();
        let (i0, i1) = (batch.index(&first, &[0]).unwrap(), batch.index(&delta, &[0]).unwrap());
        assert!(i0.built && i1.built && i0.index != i1.index, "the delta's index is its own");
        batch.finish().unwrap();
        let rows_of = |i: u32| {
            let mut v = r.index_at(i as usize).rows.as_slice().to_vec();
            v.sort_unstable();
            v
        };
        assert_eq!(rows_of(i0.index), (0..half as u32).collect::<Vec<_>>());
        assert_eq!(rows_of(i1.index), (half as u32..n as u32).collect::<Vec<_>>(), "the delta's rows");
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
            let _evaluation_batch = rerail_accel::gpu::launch::LaunchScope::enter();
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
            let (d, w) = (r.index_at(ix.index as usize), h.index_at(hosts));
            assert_eq!(d.offs.as_slice(), w.offs.as_slice(), "{cols:?}: the buckets");
            assert_eq!(d.rows.as_slice(), w.rows.as_slice(), "{cols:?}: each bucket's rows, in id order");
            assert_eq!(d.marks.as_slice(), w.marks.as_slice(), "{cols:?}: each place's mark (decision 0671)");
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
        let _evaluation_batch = rerail_accel::gpu::launch::LaunchScope::enter();
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
        batch.stage(scan, &t0, Some((&ra, None)), None, 0, 1, &totals, &counters, None, &t1).unwrap();
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
        batch.stage(join, &t1, Some((&rb, Some(&ib))), None, 1, 2, &totals, &counters, None, &t2).unwrap();
        let t3 = Table::new(slots, want2.len().max(1)).unwrap();
        let mut anti = Stage { mode: ANTI, nkeys: 1, ..Stage::default() };
        anti.key_kind[0] = SLOT;
        anti.key_slot[0] = 2;
        batch.stage(anti, &t2, Some((&rc, Some(&ic))), None, 2, 3, &totals, &counters, None, &t3).unwrap();
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
        let _evaluation_batch = rerail_accel::gpu::launch::LaunchScope::enter();
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

/// A fold's level (`Batch::lattice_fold`): a round the host knows, or slot 0 of a program's round word
/// (`Batch::round_end`).
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) enum LatLevel<'a> {
    Round(u32),
    Word(&'a Words),
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
struct LatParams { top: i64, n: u32, had: u32, nkeys: u32, code: u32, after: u32, pmask: u32, hmask: u32, level: u32, omask: u32, counted: u32,
                   r_at: u64, r_rows_at: u64, h_at: u64, h0_at: u64 }

const _: () = assert!(std::mem::size_of::<LatParams>() == 80);

impl Batch {
    /// A lattice relation's round (decision 0124): the `n` rows of `u` (its key's `nkeys` columns, then the value)
    /// folded by key into `r`, the relation itself (`had` rows held, one live row a key), through its key map `map`
    /// (decision 0384 step 5), which `first` empties or grows first. A held key's row is updated where it lies; a new
    /// key's row is written at `had` on (room the caller reserved for `n` more) and its slot put in the map. Each
    /// changed key's row (key and value) into `delta`. `counters`: `[changed, new]`. `level`: where the relation keeps
    /// levels (decision 0311), the round, written to a changed or new key's slot. A live relation (`DevRel::live`, a
    /// program's rounds: decision 0603 step 3) holds its rows as the device claimed them, not `had`; its map is ready.
    /// A placed `u` (`Batch::table`, its rows `count`'s) has its places placed with it and `delta` placed by its
    /// room, at the fold's level's start (decision 0603 step 5d): a power of two of twice its rows, and their mask.
    #[allow(clippy::too_many_arguments)]
    /// Code 3 compares its values' texts in `arena`, which holds every value the fold reads.
    pub(in crate::code_rule_dsl) fn lattice_fold(&mut self, u: &Table, n: usize, r: &DevRel, had: usize, nkeys: usize, lat: LatCode, map: LatMap<'_>,
                                                 first: MapFirst<'_>, counters: &Words, delta: &Table, arena: Option<&Arena>, level: Option<LatLevel<'_>>, count: Option<(&Words, usize)>) -> Result<(), Error> {
        assert!(lat.code != 3 || arena.is_some(), "a shortlex fold reads its values' texts");
        assert!(!r.live || matches!(first, MapFirst::Ready), "a live relation's fold into a map made before its rounds");
        assert!(nkeys < MAX_KEYS && u.cap == n && delta.cap == n && u.placed() == delta.placed() && (count.is_some() || !u.placed()));
        // A live relation's keys are at most its bound, which its map holds at half full (its gate's).
        assert!(map.places.is_power_of_two() && (r.live || 2 * n <= map.places), "a key map of {} slots for {n} rows", map.places);
        let pplaces = (2 * n).next_power_of_two().max(16);
        let mut p = LatParams { top: lat.top, n: n as u32, had: had as u32, nkeys: nkeys as u32, code: lat.code, after: lat.after.unwrap_or(NONE), pmask: (pplaces - 1) as u32,
                                hmask: (map.places - 1) as u32, level: match level { Some(LatLevel::Round(l)) => l, _ => 0 }, omask: 0,
                                counted: u32::from(count.is_some()) | if r.live { 2 } else { 0 } | if matches!(level, Some(LatLevel::Word(_))) { 4 } else { 0 },
                                r_at: r.at, r_rows_at: r.rows_at(), h_at: map.at as u64, h0_at: 0 };
        // Its rows are written by the fold, through the writer (decision 0463): checked when the batch is done.
        self.writes(r, "a lattice's held rows");
        // The map's block as it is now (reaching its slots), kept until the work completes.
        let hb = bound(map.home)?;
        // Its rows written in place, bounded by their chunks (decision 0463): checked when the work completes.
        self.writes(r, "a lattice's held rows");
        // Queued in the batch's level as one job (decision 0603): its map emptied (and grown), its places emptied,
        // claimed, each key's best row chosen, merged: the program's phases, in that order.
        // Its places, its keys' chosen rows and their flags: placed in the arena where `u` is, three arrays of its
        // places (`HPlace`), its rows' room and its mask its script's.
        let placed = u.placed();
        let array = || Held::new(if placed { 16 } else { 4 * pplaces });
        let (places, chosen, flags) = (array()?, array()?, array()?);
        let mut job = Job::new(JobKind::Lattice, 0, Stage::default());
        job.reads_placed(u, &[(4, LP_N)]);
        if placed {
            job.makes_placed(delta, &[]);
            job.o_rows = Some(OutRows::InRoom);
            job.h_place = Some(HPlace { block: 4, places: None, mask: LP_PMASK, arrays: 3 });
        }
        job.desc.l = u.reach_by(&mut job, false)?;
        job.desc.a = job.reach(&places.buffer, 0, true)?;
        job.desc.totals = job.reach(&chosen.buffer, 0, true)?;
        job.desc.h = job.reach(&flags.buffer, 0, true)?;
        job.desc.s = job.reach(&r.buffer.buffer, 0, true)?;
        job.desc.vc = job.reach(&hb.buffer, 0, true)?;
        job.desc.o = delta.reach_by(&mut job, true)?;
        job.desc.counters = job.reach(&counters.buffer.buffer, 0, true)?;
        let a: Buffer = arena.map_or_else(|| self.dummy.buffer.clone(), |a| a.buffer());
        // Its values' texts, read only (`lat_text_less`).
        job.desc.v = job.reach(&a, 0, false)?;
        let mut maps = [0usize; 2];
        if !matches!(first, MapFirst::Ready) {
            maps[0] = map.places;
            if let MapFirst::Grow(old) = first {
                p.omask = (old.places - 1) as u32;
                p.h0_at = old.at as u64;
                let ob = bound(old.home)?;
                job.desc.g = job.reach(&ob.buffer, 0, false)?;
                job.own.push(ob);
                maps[1] = old.places;
            }
        }
        // The round's word, slot 0 (`counted & 4`), where the map is ready (no old map read).
        if let Some(LatLevel::Word(w)) = level {
            job.desc.g = job.reach(&w.buffer.buffer, 0, false)?;
        }
        job.desc.g = if job.desc.g == 0 { job.desc.l } else { job.desc.g };
        let (l, o) = (u.key(job.desc.l), delta.key(job.desc.o));
        let mut reads = vec![l, job.desc.v];
        if let Some((w, slot)) = count {
            let wa = job.reach(&w.buffer.buffer, 0, false)?;
            job.desc.cnt = wa + 4 * slot as u64;
            job.rows = Some((job.desc.cnt, 0));
            reads.push(wa);
        }
        job.p = Some(lat_bytes(&p).to_vec());
        (job.count, job.clear, job.lattice) = (n, pplaces, maps);
        job.order(&reads, &[job.desc.s, job.desc.vc, o, job.desc.counters]);
        job.own.extend([places, chosen, flags, hb]);
        self.queue(job)
    }
}

fn lat_bytes(p: &LatParams) -> &[u8] {
    // SAFETY: `LatParams` is `repr(C)`: an i64, eight u32s, two u64s, no padding (its size asserted).
    unsafe { std::slice::from_raw_parts((p as *const LatParams).cast::<u8>(), std::mem::size_of::<LatParams>()) }
}

#[cfg(test)]
mod join_rust {
    /// The join program's Rust (`rerail_ukl_rust::join_run`, decision 0567) was written from this source: each view
    /// format's program text has a digest it runs for. A change to the source is a change to that Rust too.
    #[test]
    fn the_join_programs_rust_is_of_this_source() {
        let digests: Vec<u64> = [0, 16, rerail_clif::view::F16Z_FORMAT]
            .iter()
            .map(|&f| rerail_ukl_rust::digest(&format!("#define JOIN_PROGRAM 1\n#define CLIF_BLOCKS_COHERENT 1\n{}", super::view_source(f)), &[]))
            .collect();
        assert_eq!(digests, rerail_ukl_rust::JOIN_PROGRAM, "join_stages.uglykl changed: bring rerail_ukl_rust::join_run to it, then its digests ({digests:#018x?})");
    }
}
