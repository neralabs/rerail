//! Tier 2 on the CPU JIT: Cranelift emits a program's words (`native_ir::StructuralOperation`'s
//! tier-2 words) as one function `tempo_stream(ctx)`. Each word is one block of code made from
//! the word and its operand alone, and its tables (a switch's targets, a string's bytes). Nothing in it is fixed for a language or a tape format.
//!
//! The machine's state (`stream::run`, the reference) is in SSA variables where every token
//! uses it: the stream's cursors, the token's start, end and limit, the end of the token
//! before, the node count, and the open and return stacks' depths. What few words touch (the
//! supertype count, pending extras, wraps, dropped records, a shared arm's target) is in a
//! stack slot (`Mem`): an SSA variable is carried through every block where paths meet. Its output is the node table, one row per node
//! in preorder (kind, field, start, end, size), and the supertypes, one row each (node, kind).
//!
//! The loops are the stream's, as the verifier requires: every cycle passes a `TOKEN`.
//! `native_cpu_shape` refuses any other loop, and any call.
//!
//! Room is sized from the program, not checked per node: between two `TOKEN`s the words write at
//! most `per_token` nodes, supertypes and returns (the longest path, `Bounds`), so a stream of
//! `n` starts needs `per_token * (n + 2)`.

use crate::Error;
// The front end's: the calls' depth (`stream`), the region reads (`byte_needs`), the JIT's tables and switch trees
// (`native_cpu_codegen`), under the names this tier's callers read.
pub use crate::byte_needs::{Seq, sequential};
pub use crate::native_cpu_codegen::{TABLES, TABLES_DATA, dispatch, table_address};
pub use crate::stream::DEPTH;
// The stream program's analyses (`stream_analysis`), the front end's.
pub use crate::stream_analysis::*;

use crate::native_cpu_shape::Step;
use crate::native_ir::{FlagOp, On, Structure, StructuralOperation as Op};
use crate::stream::{ByteSwitch, DEAD, GONE, KEEP, Node, REFUSE, REMEMBERED, Refused, WORD_KIND};
use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, Block, BlockArg, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

/// The function's one argument: where everything is. Each field is 8 bytes.
#[repr(C)]
pub struct Ctx {
    /// The file's first byte. One readable byte before it and 64 after its end.
    bytes: *const u8,
    len: u64,
    /// The extract `TOKEN` names, and its count; two entries past the count, each the file's length.
    starts: *const u32,
    nstarts: u64,
    /// The records' other columns (`records`): each token's last byte and its code, zipped
    /// with `starts`.
    lasts: *const u32,
    codes: *const u32,
    totals: *const u64,
    /// The working rows, 12 bytes a node (`native_cpu_tape`): key, start, end.
    rows: *mut u8,
    super_node: *mut u32,
    super_kind: *mut u32,
    open: *mut u32,
    returns: *mut u32,
    /// Written at `ACCEPT`: the node count, the supertype count.
    counts: *mut u64,
    /// The bytes from one of a mask's words to the next.
    lasts_stride: u64,
    /// Every output's word 0, interleaved as `lasts` is (output `o`'s word `w` at
    /// `o + w * stride` words): the masks `MARK` reads.
    outputs: *const u64,
    /// Only a program with `WRAP` uses these (docs/parser-generators/24-rr-and-bitlex-plan.md
    /// §2.5). Per open node, by depth: its subtree's base, and the last node made at its level
    /// (one more than its record; 0: none) with that node's base.
    frame_base: *mut u32,
    frame_last: *mut u32,
    frame_last_base: *mut u32,
    /// Per record: the latest wrap on this base, one more than its record (0: none; `u32::MAX`:
    /// the record is a wrap); per wrap, the wrap before it on its base, so; each record's place.
    head: *mut u32,
    link: *mut u32,
    place: *mut u32,
    /// Unused: the table is written in place (decision 0136).
    out_rows: *mut u8,
    /// Tier 1's layout: the bytes from one output's words to the next's (8 on the CPU, where a
    /// block's outputs are together; a file's blocks times 8 on Metal, an output's words
    /// together); the bytes from one total to the next, and the mask of a total's bits (64-bit
    /// totals on the CPU, 32-bit on Metal).
    output_stride: u64,
    totals_stride: u64,
    totals_mask: u64,
    /// The tape's room: its node, supertype and return slots (`Tape`'s lengths). A token word
    /// stops the run (`OVERFLOW`) when the path it starts could pass them.
    room_nodes: u64,
    room_supers: u64,
    room_pushes: u64,
    /// With the calls open counted by the machine's stack (`calls_on_stack`): the stack
    /// pointer where the run's first rule is called, the lowest it may go (`STACK_BYTES` below
    /// it), and the log of the smallest rule function's frame, in bytes (so the calls open are
    /// at most the stack's depth shifted down by it).
    stack_base: u64,
    stack_limit: u64,
    frame_shift: u64,
    /// In clusters (`compile_cut`), the run's state (`Mem`): every cluster reaches it through
    /// `ctx`, its first argument.
    state: [u64; Mem::COUNT as usize],
    /// The tape (docs/parser-generators/37): its columns at the compiled widths, the bases, the
    /// image (the header's room, then the code column in place), and the code ids: each key's
    /// (`NO_CODE`: none), and each code's key and first node.
    size_c: *mut u8,
    off_c: *mut u8,
    tail_c: *mut u8,
    base_c: *mut u32,
    image: *mut u8,
    id_of: *mut u16,
    code_kf: *mut u32,
    code_first: *mut u32,
    /// Tier 1's records' layout (`records::RecordLayout`): the bytes from a token's start,
    /// last or code to the next token's (4 in columns, 12 in rows).
    record_stride: u64,
    /// A format 16 run's (decision 0434 §6, R17; `CompiledStream::sixteen`): the writer's state, its ops'
    /// argument; each slot key's number in the language's table (`Numbers`). Null in a format 15 run.
    writer: *mut u8,
    numbers: *const u32,
    /// Tier 1's feed (decision 0496, `native_cpu_block::Feed`): its `more` and the file's feed, which tier 2 asks
    /// for the next records where it reaches `nstarts`. Zero where tier 1 made them all.
    more: u64,
    feed: u64,
    /// A statement's mark (decision 0650, `STATEMENT`): whether one is set, whether a refusing token mark held the
    /// token (`MARK_HELD`), the state as the statement found it, and its open nodes (`MARK_OPEN`).
    mark: [u64; MARK_WORDS],
}

/// The mark's words (`Ctx::mark`): set, held, the state's fields (`Mem`), then the open nodes, at most `MARK_LEVELS`
/// (more: no mark).
const MARK_LEVELS: usize = 4;
const MARK_WORDS: usize = 2 + Mem::COUNT as usize + MARK_LEVELS;
const MARK: i32 = std::mem::offset_of!(Ctx, mark) as i32;
const MARK_HELD: i32 = MARK + 8;
const MARK_STATE: i32 = MARK + 16;
const MARK_OPEN: i32 = MARK_STATE + 8 * Mem::COUNT as i32;

const WRITER: i32 = std::mem::offset_of!(Ctx, writer) as i32;
const NUMBERS: i32 = std::mem::offset_of!(Ctx, numbers) as i32;
const RECORD_STRIDE: i32 = std::mem::offset_of!(Ctx, record_stride) as i32;
const SIZE_C: i32 = std::mem::offset_of!(Ctx, size_c) as i32;
const OFF_C: i32 = std::mem::offset_of!(Ctx, off_c) as i32;
const TAIL_C: i32 = std::mem::offset_of!(Ctx, tail_c) as i32;
const BASE_C: i32 = std::mem::offset_of!(Ctx, base_c) as i32;
const IMAGE: i32 = std::mem::offset_of!(Ctx, image) as i32;
const ID_OF: i32 = std::mem::offset_of!(Ctx, id_of) as i32;
const CODE_KF: i32 = std::mem::offset_of!(Ctx, code_kf) as i32;
const CODE_FIRST: i32 = std::mem::offset_of!(Ctx, code_first) as i32;

/// The run stopped for the tape's room: the host runs it again with the bound's room.
pub const OVERFLOW: u64 = u64::MAX;


/// The field offsets of `Ctx`.
const BYTES: i32 = 0;
const LEN: i32 = 8;
const STARTS: i32 = 16;
const NSTARTS: i32 = 24;
const LASTS: i32 = 32;
const CODES: i32 = 40;
const TOTALS: i32 = 48;
const ROWS: i32 = 56;
const SUPER_NODE: i32 = 64;
const SUPER_KIND: i32 = 72;
const OPEN: i32 = 80;
const RETURNS: i32 = 88;
const COUNTS: i32 = 96;
const LASTS_STRIDE: i32 = 104;
const OUTPUTS: i32 = 112;
const FRAME_BASE: i32 = 120;
const FRAME_LAST: i32 = 128;
const FRAME_LAST_BASE: i32 = 136;
const HEAD: i32 = 144;
const LINK: i32 = 152;
const PLACE: i32 = 160;
const OUT_ROWS: i32 = 168;
const OUTPUT_STRIDE: i32 = 176;
const TOTALS_STRIDE: i32 = 184;
const TOTALS_MASK: i32 = 192;
const ROOM_NODES: i32 = 200;
const ROOM_SUPERS: i32 = 208;
const ROOM_PUSHES: i32 = 216;
const STACK_BASE: i32 = 224;
const STACK_LIMIT: i32 = 232;
const FRAME_SHIFT: i32 = 240;
const STATE: i32 = 248;
// The offsets are a 64-bit target's: the JIT that reads them runs on none other (wasm32 has none).
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::offset_of!(Ctx, out_rows) == OUT_ROWS as usize && std::mem::offset_of!(Ctx, stack_base) == STACK_BASE as usize && std::mem::offset_of!(Ctx, state) == STATE as usize);

// The working rows (`native_cpu_tape`): a node's key, start and end, 12 bytes. The tape's columns
// are apart, at the compiled widths.
use crate::native_cpu_tape::{AT_END, AT_FIELD, AT_KIND, AT_START, ROW};

/// A field of the node rows: the rows' address and the field's place in a row.
#[derive(Clone, Copy)]
struct NodeCol {
    rows: Value,
    at: i32,
}

impl NodeCol {
    fn of(rows: Value, at: i32) -> Self {
        NodeCol { rows, at }
    }

    /// Row `i`'s address: one `madd`, then each field at an offset from it, as the lean
    /// parser's push. Behind a `bitcast` of its own type (no instruction): Cranelift folds an
    /// `iadd` it can see into every load's and store's address (an `add` and a register offset
    /// each).
    fn row(f: &mut FunctionBuilder<'_>, rows: Value, i: Value) -> Value {
        let off = f.ins().imul_imm(i, ROW);
        let a = f.ins().iadd(rows, off);
        f.ins().bitcast(types::I64, MemFlags::new(), a)
    }

    /// Row `i`'s field, stored (its width: kind and field 16 bits).
    fn put(self, f: &mut FunctionBuilder<'_>, i: Value, x: Value) {
        let a = Self::row(f, self.rows, i);
        self.put_row(f, a, x);
    }

    /// The same, at a row's address (`row`): a word's stores to one row share it, as the lean
    /// parser's push computes its row once.
    fn put_row(self, f: &mut FunctionBuilder<'_>, a: Value, x: Value) {
        let flags = MemFlags::trusted();
        match self.at {
            AT_KIND | AT_FIELD => f.ins().istore16(flags, x, a, self.at),
            _ => f.ins().istore32(flags, x, a, self.at),
        };
    }

    /// Row `i`'s field, loaded.
    fn get(self, f: &mut FunctionBuilder<'_>, i: Value) -> Value {
        let a = Self::row(f, self.rows, i);
        self.get_row(f, a)
    }

    /// The same, at a row's address.
    fn get_row(self, f: &mut FunctionBuilder<'_>, a: Value) -> Value {
        let flags = MemFlags::trusted();
        match self.at {
            AT_KIND | AT_FIELD => f.ins().uload16(types::I64, flags, a, self.at),
            _ => f.ins().uload32(flags, a, self.at),
        }
    }
}

/// Returns 0 when the words accept, else one more than the refusing word's place.
/// Tier 2's entry: `ctx`, the run's end.
pub type Entry = unsafe extern "C" fn(*const Ctx) -> u64;


/// A format 16 run's document (`CompiledStream::write16`): its source, its language, the numbers of its codes, in 16z
/// where `z`, and what writes its newline index (tier 1's bits, or a scan).
#[derive(Clone, Copy)]
struct Doc16<'a> {
    source: &'a [u8],
    lang: &'a rerail_tape_format::f16::Lang,
    numbers: &'a Numbers,
    z: bool,
    lines: &'a dyn Fn(&mut Vec<u8>),
}

/// A language's numbers by tier 2's slot keys (`native_cpu_tape::id_slots`, as `native_cpu_tape::commit` keys a
/// node): what a format 16 run passes each node's number by (decision 0434 §6, R17). `u32::MAX` where the
/// language's table has none for the key.
pub struct Numbers(Vec<u32>);

/// A compiled tier-2 program.
pub struct CompiledStream {
    _module: JITModule,
    /// The tables a split region's codes read (`SplitParts`), as long as the code.
    _split: Option<Box<SplitParts>>,
    /// The tables `TOKEN`'s records read (`Fetch`), as long as the code.
    _fetch: Option<Box<Fetch>>,
    /// The refusals a statement does not recover from (`Recovery::limits`), as long as the code.
    _limits: Box<[u64]>,
    entry: Entry,
    /// The log of the smallest rule function's frame (`Ctx::frame_shift`).
    frame_shift: u64,
    bounds: Bounds,
    /// The extract `TOKEN` reads, and the output of the mask `SPAN m=1` reads.
    pub starts: Option<usize>,
    pub lasts: Option<usize>,
    /// The marks `MARK` tests: a token can be switched on once per mark and once more.
    marks: u64,
    /// The `TOKEN`s that hold the scan for a region's next part (`parts_pending`), where it is
    /// read as simdjson's `parse_string`.
    pub part_scans: Vec<usize>,
    /// Those whose scan takes each outcome (the closer, the content, a part) straight to the
    /// place the switch after takes it to (`Onward`): one branch a part, as `parse_string`'s.
    pub part_onward: Vec<usize>,
    /// The `CHECK`s a switch's edge makes in the pass that finds the token's end, from the one
    /// read of its bytes (`fused_check`), as simdjson's `parse_number` validates while reading.
    pub fused_checks: Vec<usize>,
    /// The `CHECK`s a switch's edge decides as it compiles, where it takes a known word (an
    /// atom): the word's bytes, not read again.
    pub static_checks: Vec<usize>,
    /// The program has `WRAP`: the tape has the frames, chains and preorder columns.
    pub wraps: bool,
    /// The widths the tape is written at (docs/parser-generators/37), and the language's tag.
    pub widths: rerail_tape_format::Widths,
    tag: u8,
    /// The supertype classes the codes carry (`supertype_classes`).
    classes: Vec<u16>,
    /// The kinds that are extras (a code's flag).
    extras: Vec<u16>,
    /// The run passes its nodes to the format 16 writer's ops (`compile_with`), and writes no format 15 image.
    pub sixteen: bool,
    /// The program has context beside each return (`has_context`): two return slots a call.
    context: bool,
    pub clif: String,
    pub disassembly: String,
    /// The machine code as emitted, each function's after the last: after the branches are
    /// folded, which `disassembly` (Cranelift's code before emission) is not.
    pub code_bytes: Vec<u8>,
    /// The CLIF blocks of the loops' heads, with their steps (`native_cpu_shape`): the token
    /// loop's function's.
    pub heads: Vec<(String, Step)>,
    /// Its calls, each of an outlined function made from the same program (`outline`): the
    /// only calls the machine code may have.
    pub own_calls: usize,
    /// Of `own_calls`, the calls of a pass over the table's tail (`TableFns`' `shift` and `drop`, a
    /// wrap's and a dropped node's): none where nothing wraps or is dropped. The others are the
    /// tape's rare steps (a block's base, a commit of codes, the image's end) and the words'.
    pub table_passes: usize,
    /// Of `own_calls`, the calls of a step on the working rows (`TableFns`' `blocks`, `commit`, `uncode`, `finish`, and a
    /// format 16 run's `pass` and `retag`): none in a direct format 16 run (`direct`).
    pub row_calls: usize,
    /// A format 16 run whose nodes go to the writer's ops at their words, with no row (`stream_facts::direct_keys`).
    pub direct: bool,
    /// Its most nodes open (`stream_facts::open_bound`): the writer's stack reserved for them up front, so the Rust's
    /// run checks the stack's room once, where it starts.
    pub open_bound: Option<u64>,
    /// Its functions of clusters (`compile_cut`: one, the token loop's alone), and their tail
    /// calls of each other: one where a jump left its cluster.
    pub clusters: usize,
    pub crossings: usize,
    /// Each word's Cranelift instructions as lowered (a shared run's body counted at its first
    /// place), for the profile: times how often each word runs, where tier 2's work goes.
    pub word_insts: Vec<u32>,
    /// Each word's machine code's bytes, in its rule's function (not the functions it calls),
    /// for the shape: where the code goes.
    pub word_bytes: Vec<u32>,
    /// Each word's machine code's ranges in `code_bytes` (its place, start, end), for the shape.
    pub word_ranges: Vec<(u32, u32, u32)>,
}

/// The calls open, at most (`calls_on_stack`): the stack's depth below the run's first call,
/// shifted down by the log of the smallest rule frame (each call open has one such frame, or
/// more).
fn calls_open_by_stack(f: &mut FunctionBuilder<'_>, ctx: Value) -> Value {
    let flags = MemFlags::trusted();
    let sp = f.ins().get_stack_pointer(types::I64);
    let base = f.ins().load(types::I64, flags, ctx, STACK_BASE);
    let shift = f.ins().load(types::I64, flags, ctx, FRAME_SHIFT);
    let depth = f.ins().isub(base, sp);
    f.ins().ushr(depth, shift)
}

/// `TOKEN`, taking a token: the extra a mark lost at it (`Mem::Lost`), made again after its
/// leaf, of no width at its start, as the lean lexer's `asi` places it.
fn lost_leaf(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, wrapping: bool) {
    let lost = (e.get)(f, Mem::Lost);
    let (remake, done) = (f.create_block(), f.create_block());
    f.ins().brif(lost, remake, &[], done, &[]);
    f.switch_to_block(remake);
    f.set_cold_block(remake);
    let zero = f.ins().iconst(types::I64, 0);
    (e.set)(f, Mem::Lost, zero);
    let kind = f.ins().iadd_imm(lost, -1);
    leaf_word(f, e, t, kind, LeafShape { deferred: true, extra: true, end: EndAt::Untaken }, wrapping);
    f.ins().jump(done, &[]);
    f.switch_to_block(done);
}

/// `dispatch` weighted by a profile: a run with half the weight or more is tested first (one
/// range compare), else the runs split at their weighted middle. `ends` are the runs' ends
/// (the next run's first byte; the last's, past every byte).
fn dispatch_weighted(f: &mut FunctionBuilder<'_>, x: Value, runs: &[(u32, u32, Block, u64)]) {
    if runs.len() == 1 {
        f.ins().jump(runs[0].2, &[]);
        return;
    }
    let total: u64 = runs.iter().map(|r| r.3).sum();
    let (top, &(lo, hi, b, w)) = runs.iter().enumerate().max_by_key(|(i, r)| (r.3, std::cmp::Reverse(*i))).expect("runs");
    if total > 0 && 2 * w >= total {
        // In the run: lo <= x < hi, as one unsigned compare of x - lo.
        let rest = f.create_block();
        let inside = if top == 0 {
            f.ins().icmp_imm(IntCC::UnsignedLessThan, x, i64::from(hi))
        } else if top == runs.len() - 1 {
            f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, x, i64::from(lo))
        } else {
            let d = f.ins().iadd_imm(x, -i64::from(lo));
            f.ins().icmp_imm(IntCC::UnsignedLessThan, d, i64::from(hi - lo))
        };
        f.ins().brif(inside, b, &[], rest, &[]);
        f.switch_to_block(rest);
        // Without the run: x is in no byte of it, so its neighbours' ranges may cover it.
        let others: Vec<_> = runs.iter().enumerate().filter(|&(i, _)| i != top).map(|(_, r)| *r).collect();
        dispatch_weighted(f, x, &others);
        return;
    }
    // The split nearest the weighted middle (by count when the profile saw none).
    let weight = |r: &(u32, u32, Block, u64)| if total > 0 { r.3 } else { 1 };
    let all: u64 = runs.iter().map(weight).sum();
    let (mut left, mut mid, mut best) = (0u64, 1usize, u64::MAX);
    for (i, r) in runs.iter().enumerate().take(runs.len() - 1) {
        left += weight(r);
        let gap = left.abs_diff(all - left);
        if gap < best {
            (best, mid) = (gap, i + 1);
        }
    }
    let (low, high) = (f.create_block(), f.create_block());
    let below = f.ins().icmp_imm(IntCC::UnsignedLessThan, x, i64::from(runs[mid].0));
    f.ins().brif(below, low, &[], high, &[]);
    f.switch_to_block(low);
    dispatch_weighted(f, x, &runs[..mid]);
    f.switch_to_block(high);
    dispatch_weighted(f, x, &runs[mid..]);
}

/// Word `w` of a mask (`w` below its count).
fn mask_word(f: &mut FunctionBuilder<'_>, first: Value, stride: Value, w: Value) -> Value {
    let off = f.ins().imul(w, stride);
    let at = f.ins().iadd(first, off);
    f.ins().load(types::I64, MemFlags::trusted().with_readonly(), at, 0)
}

/// A place past every word's: a switch's kind that it switches on by word, `BY_WORD` and the
/// kind (below `REFUSE`).
const BY_WORD: u16 = 0xfe00;

/// A switch's runs: each maximal range of bytes with one target, as (first byte, block).
fn runs(s: &ByteSwitch, word: &dyn Fn(u16) -> Block) -> Vec<(u32, Block)> {
    let mut out: Vec<(u32, u16)> = Vec::new();
    for (b, &t) in s.targets.iter().enumerate() {
        if out.last().is_none_or(|&(_, last)| last != t) {
            out.push((b as u32, t));
        }
    }
    out.into_iter().map(|(b, t)| (b, word(t))).collect()
}


/// A function of tier 2's own, made from the same program and called once per file: `build`
/// makes its body from its `params` words and gives its heads. Only the token loop is inlined;
/// the node table's passes run once, at `ACCEPT`, and would only crowd it. Checked: it calls
/// nothing, and each loop is a stated one.
///
/// `pinned`: its first parameter is `ctx`, in the pinned register (`pinned_ctx`): not an
/// argument, read where it is entered; its callers pass the rest (`call_word`).
fn outline(module: &mut JITModule, name: &str, params: usize, returns: usize, pinned: bool, own: &[cranelift_codegen::ir::UserExternalName], build: impl FnOnce(&mut FunctionBuilder<'_>, &[Value], &mut Vec<(Block, Step)>)) -> Result<(cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function), Error> {
    let mut signature = module.make_signature();
    for _ in 0..params - usize::from(pinned) {
        signature.params.push(AbiParam::new(types::I64));
    }
    for _ in 0..returns {
        signature.returns.push(AbiParam::new(types::I64));
    }
    // A cold helper preserves the caller's general registers (`preserve_most`, the vendored
    // Cranelift's): the caller's loop state stays in its registers across the call, where the
    // platform's convention leaves it 10 (x19 to x28) and spills the rest around every call.
    // Its returns are in the two registers the convention gives up.
    #[cfg(cranelift_preserve_most)]
    if returns <= 2 {
        signature.call_conv = cranelift_codegen::isa::CallConv::PreserveMost;
    }
    let id = module.declare_function(name, Linkage::Local, &signature).map_err(|e| {
        if std::env::var_os("RERAIL_DEBUG_STREAM").is_some() {
            eprintln!("tier 2's own function {name}: {e:?}");
        }
        Error::BackendFailure
    })?;
    let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, id.as_u32()), signature.clone());
    let mut fctx = FunctionBuilderContext::new();
    let mut heads = Vec::new();
    {
        let mut f = FunctionBuilder::new(&mut func, &mut fctx);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        f.switch_to_block(entry);
        let mut p = f.block_params(entry).to_vec();
        if pinned {
            let ctx = f.ins().get_pinned_reg(types::I64);
            p.insert(0, ctx);
        }
        build(&mut f, &p, &mut heads);
        f.seal_all_blocks();
        f.finalize();
    }
    crate::native_cpu_shape::check(&func, &heads, own).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
    Ok((id, signature, func))
}

/// A node closed (`Table::close_cells`): its row, its size, and its last descendant's row (itself, for a leaf).
#[derive(Clone, Copy)]
struct Closed {
    at: Value,
    size: Value,
    last: Value,
}

/// A leaf word's shape (`leaf_word`, `WordFn::Leaf`): whether its field is deferred, whether the pending extras
/// follow it, and where it reads its token's end.
#[derive(Clone, Copy)]
struct LeafShape {
    deferred: bool,
    extra: bool,
    end: EndAt,
}

/// Where an outlined word reads its token's end (`Lowering::token_end`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum EndAt {
    /// After its `SPAN`: its record's.
    Taken,
    /// Before: its start, or at the stream's end the last record's.
    Untaken,
    /// A program whose `TOKEN` has both phases: the field.
    Field,
}

/// A word made as a function of its own (`outline`), which every place of it calls: the lean
/// parser's helpers (`push`, `bump_as`), called from the grammar's functions. The state is in
/// `ctx` (`Ctx::state`), as the lean parser's in `self`. Its kind or field is an argument.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum WordFn {
    /// `by_word`: a keyword's leaf, its kind its token's word's (`WORD_KIND`).
    Leaf { deferred: bool, extra: bool, end: EndAt, by_word: bool },
    Open,
    Close { this: bool, end: EndAt },
    Wrap,
    /// `RETAG m=1`: the node the last wrap moved.
    RetagMoved { remembered: bool },
    /// `RETAG m=2`, `NODE_FIELD m=2`: the node made last at the level.
    Made { retag: bool, remembered: bool, dead: bool, unset: bool },
    /// `RETAG`: the innermost open node.
    Here,
    /// A direct format 16 run's (`Table::direct`): a node's word, its supertypes' key bits; a leaf's, its end.
    Open16 { classes: u32 },
    Leaf16 { end: EndAt, by_word: bool, classes: u32 },
    Close16 { this: bool, end: EndAt },
    /// `MERGE m=1`.
    Fold,
    /// `SETTLE m=2`: the extra pending last made again at the token, of no width.
    Lose,
    /// `REMEMBER #WORD_KIND`: the level's word, its token's word's kind.
    RememberWord,
    Merge { end: EndAt },
    /// `TOKEN`, taking a token: 0, or 1 past the stream's end, or `OVERFLOW` out of room.
    /// `extras`: the extras taken too (`TOKEN_EXTRAS`).
    Token { any_mark: Option<usize>, extras: bool },
    /// A token mark's tokens (`Words::token_marks`, its index): 1 where they follow, else 0.
    TokenMark { index: u16 },
    /// A local check over the token's bytes (`Words::local_checks`, its index): 1 where it holds.
    Check { index: u16 },
    /// A gap mark tier 2 tests itself (`native_cpu_block::lazy_marks`), its order.
    LazyMark { order: u8 },
}

/// What a word's body reads and writes, wherever it is made (`WordFn`): in its rule's function,
/// or in a function of its own that each place of the word calls.
struct WordEnv<'a> {
    get: &'a dyn Fn(&mut FunctionBuilder<'_>, Mem) -> Value,
    set: &'a dyn Fn(&mut FunctionBuilder<'_>, Mem, Value),
    /// A field of `ctx`: a column's address, a count.
    ctx: &'a dyn Fn(&mut FunctionBuilder<'_>, i32) -> Value,
    /// Refuse where the value is set, at the word's place; else go on in a new block.
    guard: &'a dyn Fn(&mut FunctionBuilder<'_>, Value),
}

/// A state field of `ctx`, loaded or stored, in a function of its own.
fn field_get(f: &mut FunctionBuilder<'_>, ctx: Value, k: Mem) -> Value {
    f.ins().load(types::I64, MemFlags::trusted(), ctx, STATE + 8 * k as i32)
}

fn field_set(f: &mut FunctionBuilder<'_>, ctx: Value, k: Mem, x: Value) {
    f.ins().store(MemFlags::trusted(), x, ctx, STATE + 8 * k as i32);
}

/// A word's function (`WordFn`): params `ctx` and the word's arguments, its body, then 0 (where
/// it may refuse, 1 at a refusal, which its caller makes at its own place).
fn own_word(f: &mut FunctionBuilder<'_>, p: &[Value], refuses: bool, body: &dyn Fn(&mut FunctionBuilder<'_>, &WordEnv<'_>, &[Value])) {
    let ctx = p[0];
    let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
    let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
    let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
    let guard = |f: &mut FunctionBuilder<'_>, bad: Value| {
        let (ok, no) = (f.create_block(), f.create_block());
        f.ins().brif(bad, no, &[], ok, &[]);
        f.switch_to_block(no);
        f.set_cold_block(no);
        let one = f.ins().iconst(types::I64, 1);
        f.ins().return_(&[one]);
        f.switch_to_block(ok);
    };
    let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &guard };
    body(f, &env, &p[1..]);
    if refuses {
        let zero = f.ins().iconst(types::I64, 0);
        f.ins().return_(&[zero]);
    } else {
        f.ins().return_(&[]);
    }
}

/// Whether gap mark `o`'s mask has a bit from the byte after the grammar token before's first
/// to the token's (`records::gap_marks`), 1 or 0: the records before past the extras (a stated
/// loop, `Step::Token`), none where that token's word is an `unless after` one; then the mask's
/// words from the bytes (`native_cpu_bytes`), a stated loop of words.
#[allow(clippy::too_many_arguments)]
fn lazy_mark(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, bw: &crate::native_cpu_bytes::Bytewise<'_>, o: u16, extra: Option<u8>, unless: &[u16], coded: Option<u8>, heads: &mut Vec<(Block, Step)>) -> Value {
    let flags = MemFlags::trusted();
    let (codes, starts, bytes, len) = ((e.ctx)(f, CODES), (e.ctx)(f, STARTS), (e.ctx)(f, BYTES), (e.ctx)(f, LEN));
    let next = (e.get)(f, Mem::Next);
    let s = (e.get)(f, Mem::Start);
    let here = f.ins().iadd_imm(next, -1);
    let zero = f.ins().iconst(types::I64, 0);
    let one = f.ins().iconst(types::I64, 1);
    let (back, none, found, done) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(back, types::I64);
    f.append_block_param(found, types::I64);
    f.append_block_param(found, types::I64);
    f.append_block_param(done, types::I64);
    // A mark with token tests (`lazy_coded`): its tests' hit, in the token's code, first.
    if let Some(m) = coded {
        let stride = (e.ctx)(f, RECORD_STRIDE);
        let off = f.ins().imul(here, stride);
        let ca = f.ins().iadd(codes, off);
        let c = f.ins().uload32(flags, ca, 0);
        let bit = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT) + i64::from(m));
        let bit = f.ins().band_imm(bit, 1);
        let gap = f.create_block();
        f.ins().brif(bit, done, &[BlockArg::from(one)], gap, &[]);
        f.switch_to_block(gap);
    }
    f.ins().jump(back, &[BlockArg::from(here)]);
    // The grammar token before: the records before, past the extras.
    f.switch_to_block(back);
    heads.push((back, Step::Token));
    let j = f.block_params(back)[0];
    let first = f.ins().icmp_imm(IntCC::Equal, j, 0);
    let on = f.create_block();
    f.ins().brif(first, none, &[], on, &[]);
    f.switch_to_block(on);
    let j = f.ins().iadd_imm(j, -1);
    let stride = (e.ctx)(f, RECORD_STRIDE);
    let off = f.ins().imul(j, stride);
    let ca = f.ins().iadd(codes, off);
    let c = f.ins().uload32(flags, ca, 0);
    let sa = f.ins().iadd(starts, off);
    let st = f.ins().uload32(flags, sa, 0);
    let from = f.ins().iadd_imm(st, 1);
    match extra {
        Some(x) => {
            let bit = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT) + i64::from(x));
            let bit = f.ins().band_imm(bit, 1);
            f.ins().brif(bit, back, &[BlockArg::from(j)], found, &[BlockArg::from(from), BlockArg::from(c)]);
        }
        None => {
            f.ins().jump(found, &[BlockArg::from(from), BlockArg::from(c)]);
        }
    }
    // None before: from the file's first byte, no word before.
    f.switch_to_block(none);
    f.ins().jump(found, &[BlockArg::from(zero), BlockArg::from(zero)]);
    f.switch_to_block(found);
    let (from, c) = (f.block_params(found)[0], f.block_params(found)[1]);
    // `unless after`: the word before, one of its words.
    let word = f.ins().ushr_imm(c, i64::from(crate::records::KIND_BITS));
    let word = f.ins().band_imm(word, 0xff);
    for &w in unless {
        let same = f.ins().icmp_imm(IntCC::Equal, word, i64::from(w));
        let on = f.create_block();
        f.ins().brif(same, done, &[BlockArg::from(zero)], on, &[]);
        f.switch_to_block(on);
    }
    // The mask's words from `from` to `s`, from the bytes.
    let consts = crate::native_cpu_block::Consts::make(f);
    let file = crate::native_cpu_bytes::File { bytes, len, nibble: consts.nibble, weights: consts.weights, even: consts.even };
    let words = f.create_block();
    f.append_block_param(words, types::I64);
    let q0 = f.ins().ushr_imm(from, 6);
    let qs = f.ins().ushr_imm(s, 6);
    let past = f.ins().icmp(IntCC::UnsignedGreaterThan, from, s);
    let go = f.create_block();
    f.ins().brif(past, done, &[BlockArg::from(zero)], go, &[]);
    f.switch_to_block(go);
    f.ins().jump(words, &[BlockArg::from(q0)]);
    f.switch_to_block(words);
    heads.push((words, Step::Block));
    let q = f.block_params(words)[0];
    let p = f.ins().ishl_imm(q, 6);
    let x = bw.word(f, &file, o, p, heads);
    let lim = f.ins().iadd_imm(s, 1);
    let m = within_word(f, q, from, lim);
    let x = f.ins().band(x, m);
    let any = f.ins().icmp_imm(IntCC::NotEqual, x, 0);
    let more = f.create_block();
    f.ins().brif(any, done, &[BlockArg::from(one)], more, &[]);
    f.switch_to_block(more);
    let last = f.ins().icmp(IntCC::Equal, q, qs);
    let q1 = f.ins().iadd_imm(q, 1);
    f.ins().brif(last, done, &[BlockArg::from(zero)], words, &[BlockArg::from(q1)]);
    f.switch_to_block(done);
    f.block_params(done)[0]
}

/// Whether token mark `m`'s tokens follow at the token (`Words::token_marks`): each element's
/// record, the records' codes from the one before `Next`, the extras (`extra_bit`) skipped; 1 or
/// 0. Each turn of a skip is a record's: a stated `Token` loop.
/// A local check over the token's bytes (`CHECK`), word-parallel: the mask's words from the
/// token's start to its end (a stated loop, `Step::Block`); at a word with bits, the check's
/// value at each of its 64 places at once (`local_word`), and the test one AND. 1 where it holds
/// at a bit, else 0. No loop over bits or bytes. The token's start and end are passed: the
/// state may be the caller's variables.
#[allow(clippy::too_many_arguments)]
fn check_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, c: &crate::program::LocalCheck, form: &crate::stream_facts::WordForm, output: usize, bytewise: Option<&crate::native_cpu_bytes::Bytewise<'_>>, start: Value, limit: Value, heads: &mut Vec<(Block, Step)>) -> Value {
    let (outputs_p, stride, lasts_stride) = ((e.ctx)(f, OUTPUTS), (e.ctx)(f, OUTPUT_STRIDE), (e.ctx)(f, LASTS_STRIDE));
    let (bytes, len) = ((e.ctx)(f, BYTES), (e.ctx)(f, LEN));
    let consts = crate::native_cpu_block::Consts::make(f);
    let off = f.ins().imul_imm(stride, output as i64);
    let base = f.ins().iadd(outputs_p, off);
    let w0 = f.ins().ushr_imm(start, 6);
    let l1 = f.ins().iadd_imm(limit, -1);
    let wl = f.ins().ushr_imm(l1, 6);
    let (words, done) = (f.create_block(), f.create_block());
    f.append_block_param(words, types::I64);
    f.append_block_param(done, types::I64);
    f.ins().jump(words, &[BlockArg::from(w0)]);
    // A word of the mask, cut to the token's bytes.
    f.switch_to_block(words);
    heads.push((words, Step::Block));
    let w = f.block_params(words)[0];
    let past = f.ins().icmp(IntCC::UnsignedGreaterThan, w, wl);
    let (body, zero) = (f.create_block(), f.ins().iconst(types::I64, 0));
    f.ins().brif(past, done, &[BlockArg::from(zero)], body, &[]);
    f.switch_to_block(body);
    // From the bytes, where they make the mask (`native_cpu_bytes`): the CPU's tier 1 makes
    // none of it. Else its row's word.
    let word = match bytewise {
        Some(b) => {
            let file = crate::native_cpu_bytes::File { bytes, len, nibble: consts.nibble, weights: consts.weights, even: consts.even };
            let p = f.ins().ishl_imm(w, 6);
            b.word(f, &file, output as u16, p, heads)
        }
        None => mask_word(f, base, lasts_stride, w),
    };
    let ones = f.ins().iconst(types::I64, -1);
    let s = f.ins().band_imm(start, 63);
    let lo = f.ins().ishl(ones, s);
    let first = f.ins().icmp(IntCC::Equal, w, w0);
    let lo = f.ins().select(first, lo, ones);
    let t = f.ins().band_imm(l1, 63);
    let t = f.ins().irsub_imm(t, 63);
    let hi = f.ins().ushr(ones, t);
    let last = f.ins().icmp(IntCC::Equal, w, wl);
    let hi = f.ins().select(last, hi, ones);
    let word = f.ins().band(word, lo);
    let a = f.ins().band(word, hi);
    let (look, next) = (f.create_block(), f.create_block());
    f.ins().brif(a, look, &[], next, &[]);
    // The check at the word's places: its value at place `64 w + b + delay`, bit `b`.
    f.switch_to_block(look);
    let mut env = LocalWords { form, bytes, len, w, consts: &consts, values: Default::default(), classes: Default::default(), tables: Default::default() };
    let top = c.ops.len() - 1;
    let (dq, dr) = (i64::from(c.delay) / 64, i64::from(c.delay) % 64);
    let l = local_word(f, &c.ops, &mut env, top, dq);
    let l = if dr == 0 {
        l
    } else {
        let after = local_word(f, &c.ops, &mut env, top, dq + 1);
        let x = f.ins().ushr_imm(l, dr);
        let y = f.ins().ishl_imm(after, 64 - dr);
        f.ins().bor(x, y)
    };
    let hit = f.ins().band(a, l);
    let one = f.ins().iconst(types::I64, 1);
    f.ins().brif(hit, done, &[BlockArg::from(one)], next, &[]);
    f.switch_to_block(next);
    let w1 = f.ins().iadd_imm(w, 1);
    f.ins().jump(words, &[BlockArg::from(w1)]);
    f.switch_to_block(done);
    f.block_params(done)[0]
}

/// A token's check (`LocalCheck::token`, decision 0398) over the token's own bytes, as
/// simdjson's stage 2 parses a number: a loop over the token's 64-byte windows from its first
/// byte (a stated loop, `Step::Block`; one turn for a token of 64 bytes or fewer). Each window's
/// ops are `token_window`'s: a `prev` reads its operand's word from the window before, a `fill`
/// its carry. 1 where the check holds at a place of the token, else 0. No loop over bits or
/// bytes. The file is readable 64 bytes past its end (`local_classes`), and the places past the
/// token are cut from the test; they reach no earlier place. A token of 16 bytes or fewer is
/// checked in its arm (`token_check_short`); this is the rest.
#[allow(clippy::too_many_arguments)]
fn token_check_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, c: &crate::program::LocalCheck, form: &crate::stream_facts::WordForm, start: Value, limit: Value, next: Value, heads: &mut Vec<(Block, Step)>) -> Value {
    use crate::program::LocalOp as L;
    let bytes = (e.ctx)(f, BYTES);
    let consts = crate::native_cpu_block::Consts::make(f);
    let n = f.ins().isub(limit, start);
    // The tokens' ends before it, once (decision 0428).
    let ended = (c.back != 0).then(|| {
        let (lasts, stride) = ((e.ctx)(f, LASTS), (e.ctx)(f, RECORD_STRIDE));
        ended_word(f, lasts, stride, next, start, c.back)
    });
    // The carried words: each `prev`'s operand, the window before's; each `fill`'s carry.
    let carried = token_carried(c);
    let fills: Vec<usize> = (0..c.ops.len()).filter(|&k| matches!(c.ops[k], L::Fill(..))).collect();
    let (turn, done) = (f.create_block(), f.create_block());
    f.append_block_param(turn, types::I64);
    for _ in 0..carried.len() + fills.len() {
        f.append_block_param(turn, types::I64);
    }
    f.append_block_param(done, types::I64);
    let zero = f.ins().iconst(types::I64, 0);
    let first: Vec<BlockArg> = std::iter::repeat_n(BlockArg::from(zero), 1 + carried.len() + fills.len()).collect();
    f.ins().jump(turn, &first);
    f.switch_to_block(turn);
    heads.push((turn, Step::Block));
    let params = f.block_params(turn).to_vec();
    let (j, before, cin) = (params[0], &params[1..1 + carried.len()], &params[1 + carried.len()..]);
    let p = f.ins().ishl_imm(j, 6);
    // The window's first place: `back` bytes before the token in the first (decision 0428).
    let at = f.ins().iadd(start, p);
    let (from, pre) = frame_begin(f, at, c.back);
    let at = f.ins().iadd(bytes, from);
    let vectors: Vec<Value> = [0, 16, 32, 48].iter().map(|&off| f.ins().load(types::I8X16, MemFlags::trusted(), at, off)).collect();
    // Places of the window left in the token, and in the frame (`delay` more): 64 or more for
    // all of them.
    let left = f.ins().isub(n, p);
    let left = f.ins().iadd_imm(left, i64::from(c.back));
    let frame = f.ins().iadd_imm(left, i64::from(c.delay));
    let is_first = f.ins().icmp_imm(IntCC::Equal, j, 0);
    let len = (e.ctx)(f, LEN);
    let file_left = f.ins().isub(len, start);
    let file_left = f.ins().isub(file_left, p);
    let file_left = f.ins().iadd_imm(file_left, i64::from(c.back));
    let back = ended.map(|ended| Back { pre, ended });
    let v = token_window(f, c, form, &consts, &vectors, Some(is_first), left, file_left, Some((&carried, before)), Some((&fills, cin)), back, None);
    // The places tested: the frame's, from `delay` on.
    let (ones, one) = (f.ins().iconst(types::I64, -1), f.ins().iconst(types::I64, 1));
    let full = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, frame, 64);
    let below = f.ins().ishl(one, frame);
    let below = f.ins().iadd_imm(below, -1);
    let valid = f.ins().select(full, ones, below);
    let from = f.ins().iconst(types::I64, -1i64 << (c.delay + c.back));
    let from = f.ins().select(is_first, from, ones);
    let valid = f.ins().band(valid, from);
    let hit = f.ins().band(v[c.ops.len() - 1], valid);
    let (next, holds) = (f.create_block(), f.ins().iconst(types::I64, 1));
    f.ins().brif(hit, done, &[BlockArg::from(holds)], next, &[]);
    f.switch_to_block(next);
    let j1 = f.ins().iadd_imm(j, 1);
    let mut args: Vec<BlockArg> = vec![BlockArg::from(j1)];
    args.extend(carried.iter().map(|&a| BlockArg::from(v[a])));
    for &k in &fills {
        let out = f.ins().ushr_imm(v[k], 63);
        args.push(BlockArg::from(out));
    }
    let more = f.ins().icmp_imm(IntCC::SignedGreaterThan, frame, 64);
    f.ins().brif(more, turn, &args, done, &[BlockArg::from(zero)]);
    f.switch_to_block(done);
    f.block_params(done)[0]
}

/// A token's check on a token of `n` bytes, 1 to 16, in its arm: one load of its 16 bytes, one
/// vector a table, its ops on 16 places with nothing carried in (the token's one window). Not 0
/// where the check holds at a place of the token. Most numbers are this short: the check costs
/// its classes and ops, with no call and no loop (decision 0398).
/// A check that reads before its token (decision 0428) loads from `back` bytes before it, and
/// takes the tokens' ends there (`ended`, `ended_word`).
#[allow(clippy::too_many_arguments)]
fn token_check_short(f: &mut FunctionBuilder<'_>, c: &crate::program::LocalCheck, form: &crate::stream_facts::WordForm, bytes: Value, len: Value, start: Value, n: Value, ended: Option<Value>) -> Value {
    let consts = crate::native_cpu_block::Consts::make(f);
    let (from, pre) = frame_begin(f, start, c.back);
    let at = f.ins().iadd(bytes, from);
    let vector = f.ins().load(types::I8X16, MemFlags::trusted(), at, 0);
    let begin = f.ins().iadd_imm(start, -i64::from(c.back));
    let file_left = f.ins().isub(len, begin);
    let left = f.ins().iadd_imm(n, i64::from(c.back));
    let back = (c.back != 0).then(|| Back { pre, ended: ended.unwrap_or_else(|| f.ins().iconst(types::I64, 0)) });
    let v = token_window(f, c, form, &consts, &[vector], None, left, file_left, None, None, back, None);
    // Places `back + delay` to `back + n + delay`.
    let one = f.ins().iconst(types::I64, 1);
    let below = f.ins().ishl(one, n);
    let valid = f.ins().iadd_imm(below, -1);
    let shift = i64::from(c.delay) + i64::from(c.back);
    let valid = if shift == 0 { valid } else { f.ins().ishl_imm(valid, shift) };
    f.ins().band(v[c.ops.len() - 1], valid)
}

/// Whether byte `b` (a word) is in `set`: its word of the set, by the byte's top two bits.
fn byte_in(f: &mut FunctionBuilder<'_>, b: Value, set: [u64; 4]) -> Value {
    let hi = f.ins().ushr_imm(b, 6);
    let mut word = f.ins().iconst(types::I64, set[0] as i64);
    for (q, &w) in set.iter().enumerate().skip(1) {
        if w == set[0] {
            continue;
        }
        let is = f.ins().icmp_imm(IntCC::Equal, hi, q as i64);
        let wv = f.ins().iconst(types::I64, w as i64);
        word = f.ins().select(is, wv, word);
    }
    let lo = f.ins().band_imm(b, 63);
    let bit = f.ins().ushr(word, lo);
    f.ins().band_imm(bit, 1)
}

/// A token's check that reads before its token (`LocalCheck::back`, decision 0428), in a window:
/// `pre`, the window's places before the file (its bytes loaded from the file's first); `ended`,
/// bit `p` where a token before this one ends at the frame's place `p` (`ended_word`).
#[derive(Clone, Copy)]
struct Back {
    pre: Value,
    ended: Value,
}

/// Op `k`'s byte set's bits over the window (`LocalOp::Bytes`), from its table's classes.
fn token_bytes(f: &mut FunctionBuilder<'_>, form: &crate::stream_facts::WordForm, consts: &crate::native_cpu_block::Consts, vectors: &[Value], tables: &mut std::collections::HashMap<usize, Vec<Value>>, k: usize) -> Value {
    match form.class_of[&k] {
        (_, 0) => f.ins().iconst(types::I64, 0),
        (ti, bits) => {
            let t = token_classes(f, form, consts, vectors, tables, ti);
            let mask = {
                let b = f.ins().iconst(types::I8, i64::from(bits));
                f.ins().splat(types::I8X16, b)
            };
            let zero16 = {
                let z = f.ins().iconst(types::I8, 0);
                f.ins().splat(types::I8X16, z)
            };
            let parts: Vec<Value> = t
                .iter()
                .map(|&x| {
                    let y = f.ins().band(x, mask);
                    f.ins().icmp(IntCC::NotEqual, y, zero16)
                })
                .collect();
            crate::native_cpu_block::bitmask(f, &parts, consts.weights)
        }
    }
}

/// Where a token's check's frame begins, `back` bytes before the token at `start`: the place
/// to load from (the file's first byte, where it begins before the file), and the places before
/// the file (decision 0428).
fn frame_begin(f: &mut FunctionBuilder<'_>, start: Value, back: u8) -> (Value, Value) {
    let zero = f.ins().iconst(types::I64, 0);
    if back == 0 {
        return (start, zero);
    }
    let from = f.ins().iadd_imm(start, -i64::from(back));
    let before = f.ins().icmp_imm(IntCC::SignedLessThan, from, 0);
    let at = f.ins().select(before, zero, from);
    let neg = f.ins().ineg(from);
    let pre = f.ins().select(before, neg, zero);
    (at, pre)
}

/// A token's check's `Ended` (decision 0428): bit `p` (under `back`) where a token before the
/// one at `start` has its last byte at `start - back + p`. From the records before it, read
/// from the last (`next - 1` is the token's own): at most `back` of them end there, each a byte
/// or more. Unrolled, no branch; a record before the first is none.
fn ended_word(f: &mut FunctionBuilder<'_>, lasts: Value, stride: Value, next: Value, start: Value, back: u8) -> Value {
    let (zero, one) = (f.ins().iconst(types::I64, 0), f.ins().iconst(types::I64, 1));
    let from = f.ins().iadd_imm(start, -i64::from(back));
    let mut word = zero;
    for j in 1..=i64::from(back) {
        let at = f.ins().iadd_imm(next, -1 - j);
        let some = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, at, 0);
        let at = f.ins().select(some, at, zero);
        let off = f.ins().imul(at, stride);
        let addr = f.ins().iadd(lasts, off);
        let last = f.ins().uload32(MemFlags::trusted(), addr, 0);
        let p = f.ins().isub(last, from);
        let inside = f.ins().icmp_imm(IntCC::UnsignedLessThan, p, i64::from(back));
        let hit = f.ins().band(some, inside);
        let bit = f.ins().ishl(one, p);
        let bit = f.ins().select(hit, bit, zero);
        word = f.ins().bor(word, bit);
    }
    word
}

/// Table `ti`'s classes of the window's bytes, a vector each, made once.
fn token_classes(f: &mut FunctionBuilder<'_>, form: &crate::stream_facts::WordForm, consts: &crate::native_cpu_block::Consts, vectors: &[Value], tables: &mut std::collections::HashMap<usize, Vec<Value>>, ti: usize) -> Vec<Value> {
    if let Some(t) = tables.get(&ti) {
        return t.clone();
    }
    let (lo, hi) = &form.tables[ti];
    let hl = f.func.dfg.constants.insert(cranelift_codegen::ir::ConstantData::from(&lo[..]));
    let hh = f.func.dfg.constants.insert(cranelift_codegen::ir::ConstantData::from(&hi[..]));
    let (tlo, thi) = (f.ins().vconst(types::I8X16, hl), f.ins().vconst(types::I8X16, hh));
    // The bytes' nibbles, split once for every table (kept under `NIBBLES`: low, high a vector).
    const NIBBLES: usize = usize::MAX;
    tables.entry(NIBBLES).or_insert_with(|| vectors.iter().flat_map(|&b| [f.ins().band(b, consts.nibble), f.ins().ushr_imm(b, 4)]).collect());
    let split = tables[&NIBBLES].clone();
    let t: Vec<Value> = split
        .chunks(2)
        .map(|n| {
            let l = f.ins().swizzle(tlo, n[0]);
            let h = f.ins().swizzle(thi, n[1]);
            f.ins().band(l, h)
        })
        .collect();
    tables.insert(ti, t.clone());
    t
}

/// The operands a token's check's `prev`s read: carried from window to window.
fn token_carried(c: &crate::program::LocalCheck) -> Vec<usize> {
    let mut carried: Vec<usize> = Vec::new();
    for op in &c.ops {
        if let crate::program::LocalOp::Prev(a, _) = *op
            && !carried.contains(&usize::from(a))
        {
            carried.push(usize::from(a));
        }
    }
    carried
}

/// A token's check's ops over one window of its bytes, `vectors` (four for 64 places, one for
/// 16), each op a word of the window's places. Its bytes are classified on the vectors as tier
/// 1's (the check's tables, then `bitmask`). `first`: whether the window is the token's first
/// (`None`: it is). `left`: the token's places from the window's first; `file_left`, the
/// file's bytes from it (a `within`'s test, decision 0399). `before`: each carried
/// operand's word in the window before (`None`: no window before); `cin`: each `fill`'s carry in
/// (`None`: none). `back`: a check that reads before its token (decision 0428): the window's
/// places before the file, and the tokens' ends before it (`Back`).
#[allow(clippy::too_many_arguments)]
/// On one vector, the masks of a check's byte sets (`Bytes` ops, by op), made together
/// (`bitmasks16`): one split of the bytes into nibbles, each table once, one tree of adds.
fn token_sets(f: &mut FunctionBuilder<'_>, c: &crate::program::LocalCheck, form: &crate::stream_facts::WordForm, consts: &crate::native_cpu_block::Consts, vector: Value, tables: &mut std::collections::HashMap<usize, Vec<Value>>) -> std::collections::HashMap<usize, Value> {
    use crate::program::LocalOp as L;
    let sets: Vec<(usize, usize, u8)> = (0..c.ops.len()).filter(|&k| matches!(c.ops[k], L::Bytes(_))).map(|k| (k, form.class_of[&k].0, form.class_of[&k].1)).filter(|&(_, _, bits)| bits != 0).collect();
    let zero16 = {
        let z = f.ins().iconst(types::I8, 0);
        f.ins().splat(types::I8X16, z)
    };
    let mut parts = Vec::with_capacity(sets.len());
    for &(_, ti, bits) in &sets {
        let cl = token_classes(f, form, consts, &[vector], tables, ti)[0];
        let b = f.ins().iconst(types::I8, i64::from(bits));
        let mask = f.ins().splat(types::I8X16, b);
        let y = f.ins().band(cl, mask);
        parts.push(f.ins().icmp(IntCC::NotEqual, y, zero16));
    }
    let masks = crate::native_cpu_block::bitmasks16(f, &parts, consts.weights);
    sets.iter().map(|s| s.0).zip(masks).collect()
}

/// `made_in`: the byte sets' masks (`token_sets`), where the caller made them.
#[allow(clippy::too_many_arguments)]
fn token_window(f: &mut FunctionBuilder<'_>, c: &crate::program::LocalCheck, form: &crate::stream_facts::WordForm, consts: &crate::native_cpu_block::Consts, vectors: &[Value], first: Option<Value>, left: Value, file_left: Value, before: Option<(&[usize], &[Value])>, cin: Option<(&[usize], &[Value])>, back: Option<Back>, made_in: Option<std::collections::HashMap<usize, Value>>) -> Vec<Value> {
    use crate::program::LocalOp as L;
    // Each table's classes of the window's bytes, made once.
    let mut tables: std::collections::HashMap<usize, Vec<Value>> = std::collections::HashMap::new();
    let zero = f.ins().iconst(types::I64, 0);
    let one = f.ins().iconst(types::I64, 1);
    // On one vector, the byte sets' masks are made together (`bitmasks16`): one tree of adds.
    let made: std::collections::HashMap<usize, Value> = match made_in {
        Some(m) => m,
        None if vectors.len() == 1 => token_sets(f, c, form, consts, vectors[0], &mut tables),
        None => std::collections::HashMap::new(),
    };
    let mut v: Vec<Value> = Vec::with_capacity(c.ops.len());
    for (k, op) in c.ops.iter().enumerate() {
        let x = match *op {
            // Loaded from the file's first byte where the window begins before it: the bytes'
            // bits moved up past the places before the file, which read 0.
            L::Bytes(_) if back.is_some() => {
                let b = back.expect("a check that reads back");
                let x = if made.contains_key(&k) { made[&k] } else { token_bytes(f, form, consts, vectors, &mut tables, k) };
                f.ins().ishl(x, b.pre)
            }
            L::Bytes(_) if made.contains_key(&k) => made[&k],
            L::Bytes(_) => token_bytes(f, form, consts, vectors, &mut tables, k),
            L::Const(pat) => f.ins().iconst(types::I64, pat as i64),
            L::Within(k) => {
                // Place `b` with `b - k` in the file (below `file_left`), and in the frame (from
                // `k` on in the first window).
                let ones = f.ins().iconst(types::I64, -1);
                let hi = f.ins().iadd_imm(file_left, i64::from(k));
                let some = f.ins().ishl(one, hi);
                let some = f.ins().iadd_imm(some, -1);
                let full = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, hi, 64);
                let none = f.ins().icmp_imm(IntCC::SignedLessThanOrEqual, hi, 0);
                let upper = f.ins().select(full, ones, some);
                let upper = f.ins().select(none, zero, upper);
                // And past the file's start: `pre` places more.
                let low = match back {
                    Some(b) => {
                        let s = f.ins().iadd_imm(b.pre, i64::from(k));
                        let low = f.ins().ishl(ones, s);
                        let past = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, s, 64);
                        f.ins().select(past, zero, low)
                    }
                    None => f.ins().iconst(types::I64, -1i64 << k),
                };
                let lower = match first {
                    Some(is) => f.ins().select(is, low, ones),
                    None => low,
                };
                f.ins().band(upper, lower)
            }
            L::Start => {
                let at = f.ins().iconst(types::I64, 1i64 << c.back);
                match first {
                    Some(is) => f.ins().select(is, at, zero),
                    None => at,
                }
            }
            // The tokens' ends before it, in the first window only (`back` is under 64).
            L::Ended => {
                let ended = back.expect("a check that reads back").ended;
                match first {
                    Some(is) => f.ins().select(is, ended, zero),
                    None => ended,
                }
            }
            L::Last => {
                // Bit `left - 1` where it is in the window.
                let r = f.ins().iadd_imm(left, -1);
                let bit = f.ins().ishl(one, r);
                if vectors.len() == 1 {
                    bit
                } else {
                    let inside = f.ins().icmp_imm(IntCC::UnsignedLessThan, r, 64);
                    f.ins().select(inside, bit, zero)
                }
            }
            L::Not(a) => f.ins().bnot(v[usize::from(a)]),
            L::And(a, b) => f.ins().band(v[usize::from(a)], v[usize::from(b)]),
            L::Or(a, b) => f.ins().bor(v[usize::from(a)], v[usize::from(b)]),
            L::Xor(a, b) => f.ins().bxor(v[usize::from(a)], v[usize::from(b)]),
            L::AndNot(a, b) => f.ins().band_not(v[usize::from(a)], v[usize::from(b)]),
            L::Prev(a, sh) => {
                let w = f.ins().ishl_imm(v[usize::from(a)], i64::from(sh));
                match before {
                    Some((carried, words)) => {
                        let back = words[carried.iter().position(|&x| x == usize::from(a)).expect("carried")];
                        let back = f.ins().ushr_imm(back, 64 - i64::from(sh));
                        f.ins().bor(w, back)
                    }
                    None => w,
                }
            }
            L::Fill(a, b) => {
                // `block::carry::fill`: one add with the carry in.
                let (s0, e0) = (v[usize::from(a)], v[usize::from(b)]);
                let ne = f.ins().bnot(e0);
                let bb = f.ins().bor(ne, s0);
                let sum = f.ins().iadd(s0, bb);
                let sum = match cin {
                    Some((fills, carries)) => f.ins().iadd(sum, carries[fills.iter().position(|&x| x == k).expect("a fill")]),
                    None => sum,
                };
                let into = f.ins().bxor(sum, s0);
                let into = f.ins().bxor(into, bb);
                let kept = f.ins().band(ne, into);
                f.ins().bor(s0, kept)
            }
        };
        v.push(x);
    }
    v
}

/// What `local_word` reads: the check's classes, the file, the mask's word `w`; each op's value
/// at a word (by its distance from `w`), each word's classes and the tables, made once.
struct LocalWords<'a> {
    form: &'a crate::stream_facts::WordForm,
    bytes: Value,
    len: Value,
    w: Value,
    consts: &'a crate::native_cpu_block::Consts,
    values: std::collections::HashMap<(usize, i64), Value>,
    classes: std::collections::HashMap<(usize, i64), [Value; 4]>,
    tables: std::collections::HashMap<usize, (Value, Value)>,
}

/// The bits of word `q` whose places are in `lo..hi`.
fn within_word(f: &mut FunctionBuilder<'_>, q: Value, lo: Value, hi: Value) -> Value {
    let q64 = f.ins().ishl_imm(q, 6);
    let (zero, sixty_four) = (f.ins().iconst(types::I64, 0), f.ins().iconst(types::I64, 64));
    let clamp = |f: &mut FunctionBuilder<'_>, x: Value| {
        let x = f.ins().isub(x, q64);
        let x = f.ins().smax(x, zero);
        f.ins().smin(x, sixty_four)
    };
    let (a, b) = (clamp(f, lo), clamp(f, hi));
    let ones = f.ins().iconst(types::I64, -1);
    let one = f.ins().iconst(types::I64, 1);
    let below = f.ins().ishl(one, b);
    let below = f.ins().iadd_imm(below, -1);
    let full = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, b, 64);
    let below = f.ins().select(full, ones, below);
    let from = f.ins().ishl(ones, a);
    let none = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, a, 64);
    let from = f.ins().select(none, zero, from);
    f.ins().band(below, from)
}

/// Op `k` of a local check at word `w + o`, a bit a place (`program::local_value`): the byte
/// sets by tier 1's classes over the word's four vectors (`native_cpu_lanes`' lookup, then
/// `bitmask`), the rest word operations; a `Prev` reads the word before its operand's too.
/// Straight-line, each (op, word) once.
fn local_word(f: &mut FunctionBuilder<'_>, ops: &[crate::program::LocalOp], e: &mut LocalWords<'_>, k: usize, o: i64) -> Value {
    use crate::program::LocalOp as L;
    if let Some(&v) = e.values.get(&(k, o)) {
        return v;
    }
    let q = f.ins().iadd_imm(e.w, o);
    let v = match ops[k] {
        L::Bytes(_) => match e.form.class_of[&k] {
            (_, 0) => f.ins().iconst(types::I64, 0),
            (ti, bits) => {
                let c = local_classes(f, e, ti, o);
                let mask = {
                    let b = f.ins().iconst(types::I8, i64::from(bits));
                    f.ins().splat(types::I8X16, b)
                };
                let zero16 = {
                    let z = f.ins().iconst(types::I8, 0);
                    f.ins().splat(types::I8X16, z)
                };
                let parts: Vec<Value> = c
                    .iter()
                    .map(|&v| {
                        let x = f.ins().band(v, mask);
                        f.ins().icmp(IntCC::NotEqual, x, zero16)
                    })
                    .collect();
                let m = crate::native_cpu_block::bitmask(f, &parts, e.consts.weights);
                let zero = f.ins().iconst(types::I64, 0);
                let w = within_word(f, q, zero, e.len);
                f.ins().band(m, w)
            }
        },
        L::Const(p) => {
            let x = f.ins().iconst(types::I64, p as i64);
            let zero = f.ins().iconst(types::I64, 0);
            let neg = f.ins().icmp_imm(IntCC::SignedLessThan, q, 0);
            f.ins().select(neg, zero, x)
        }
        L::Within(d) => {
            let lo = f.ins().iconst(types::I64, i64::from(d));
            let hi = f.ins().iadd_imm(e.len, i64::from(d));
            within_word(f, q, lo, hi)
        }
        L::Not(a) => {
            let x = local_word(f, ops, e, usize::from(a), o);
            f.ins().bnot(x)
        }
        L::And(a, b) | L::Or(a, b) | L::Xor(a, b) | L::AndNot(a, b) => {
            let x = local_word(f, ops, e, usize::from(a), o);
            let y = local_word(f, ops, e, usize::from(b), o);
            match ops[k] {
                L::And(..) => f.ins().band(x, y),
                L::Or(..) => f.ins().bor(x, y),
                L::Xor(..) => f.ins().bxor(x, y),
                _ => f.ins().band_not(x, y),
            }
        }
        L::Prev(a, s) => {
            // Its operand `s` places before: this word's bits up, the word before's top bits in;
            // none before the file's start.
            let x = local_word(f, ops, e, usize::from(a), o);
            let y = local_word(f, ops, e, usize::from(a), o - 1);
            let x = f.ins().ishl_imm(x, i64::from(s));
            let y = f.ins().ushr_imm(y, 64 - i64::from(s));
            let zero = f.ins().iconst(types::I64, 0);
            let first = f.ins().icmp_imm(IntCC::SignedLessThan, q, 1);
            let y = f.ins().select(first, zero, y);
            f.ins().bor(x, y)
        }
        L::Fill(..) | L::Start | L::Last | L::Ended => unreachable!("a token's check's op in a local check (`validate_local_checks`)"),
    };
    e.values.insert((k, o), v);
    v
}

/// Table `ti`'s classes of the 64 bytes of word `w + o` (zeros outside the file): four vectors.
fn local_classes(f: &mut FunctionBuilder<'_>, e: &mut LocalWords<'_>, ti: usize, o: i64) -> [Value; 4] {
    if let Some(&c) = e.classes.get(&(ti, o)) {
        return c;
    }
    let (tlo, thi) = match e.tables.get(&ti) {
        Some(&t) => t,
        None => {
            let (lo, hi) = &e.form.tables[ti];
            let hl = f.func.dfg.constants.insert(cranelift_codegen::ir::ConstantData::from(&lo[..]));
            let hh = f.func.dfg.constants.insert(cranelift_codegen::ir::ConstantData::from(&hi[..]));
            let t = (f.ins().vconst(types::I8X16, hl), f.ins().vconst(types::I8X16, hh));
            e.tables.insert(ti, t);
            t
        }
    };
    // The word's address, where it is inside the file; else the file's first (readable: 64
    // bytes after its end), its classes then zeros.
    let q = f.ins().iadd_imm(e.w, o);
    let q64 = f.ins().ishl_imm(q, 6);
    let after = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, q, 0);
    let before_end = f.ins().icmp(IntCC::SignedLessThan, q64, e.len);
    let inside = f.ins().band(after, before_end);
    let at = f.ins().iadd(e.bytes, q64);
    let at = f.ins().select(inside, at, e.bytes);
    let zero16 = {
        let z = f.ins().iconst(types::I8, 0);
        f.ins().splat(types::I8X16, z)
    };
    let c = [0, 16, 32, 48].map(|off| {
        let v = f.ins().load(types::I8X16, MemFlags::trusted(), at, off);
        let lo = f.ins().band(v, e.consts.nibble);
        let hi = f.ins().ushr_imm(v, 4);
        let l = f.ins().swizzle(tlo, lo);
        let h = f.ins().swizzle(thi, hi);
        let x = f.ins().band(l, h);
        f.ins().select(inside, x, zero16)
    });
    e.classes.insert((ti, o), c);
    c
}

/// The token marks that refuse (`TokenMark::refuses`), at a token of code `c`: their bits, one
/// test; where one is set (cold), each set bit's tokens, to `refused` where they follow. Then
/// on, in a new block.
fn refusal_tests(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, c: Value, marks: &[crate::program::TokenMark], extra_bit: Option<u8>, refused: Block, heads: &mut Vec<(Block, Step)>) {
    let refusing: Vec<&crate::program::TokenMark> = marks.iter().filter(|m| m.refuses).collect();
    if refusing.is_empty() {
        return;
    }
    let shift = i64::from(crate::records::MARK_SHIFT);
    let bits = refusing.iter().fold(0i64, |b, m| b | 1 << (shift + i64::from(m.order)));
    let any = f.ins().band_imm(c, bits);
    let (test, on) = (f.create_block(), f.create_block());
    f.ins().brif(any, test, &[], on, &[]);
    f.switch_to_block(test);
    f.set_cold_block(test);
    for m in refusing {
        let bit = f.ins().ushr_imm(c, shift + i64::from(m.order));
        let bit = f.ins().band_imm(bit, 1);
        let (set, next) = (f.create_block(), f.create_block());
        f.ins().brif(bit, set, &[], next, &[]);
        f.switch_to_block(set);
        f.set_cold_block(set);
        let holds = token_pattern(f, e, m, extra_bit, heads);
        f.ins().brif(holds, refused, &[], next, &[]);
        f.switch_to_block(next);
    }
    f.ins().jump(on, &[]);
    f.switch_to_block(on);
}

/// A token element's test on a record's code `c` (`TokenElem::holds`): a kind of its set, or a
/// word of its; a `not` element, none; its `with` and `without` marks. Where it fails, to `fail`;
/// else on, in a new block.
fn elem_test(f: &mut FunctionBuilder<'_>, c: Value, el: &crate::program::TokenElem, fail: Block) {
    // `without M`: a code with one of the marks' bits fails (`TokenElem::without`).
    if el.without != 0 {
        let x = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT));
        let x = f.ins().band_imm(x, i64::from(el.without));
        let on = f.create_block();
        f.ins().brif(x, fail, &[], on, &[]);
        f.switch_to_block(on);
    }
    // `with M`: a code without one of the marks' bits fails (`TokenElem::with`).
    if el.with != 0 {
        let x = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT));
        let x = f.ins().band_imm(x, i64::from(el.with));
        let x = f.ins().icmp_imm(IntCC::Equal, x, i64::from(el.with));
        let on = f.create_block();
        f.ins().brif(x, on, &[], fail, &[]);
        f.switch_to_block(on);
    }
    let (ok, miss) = (f.create_block(), f.create_block());
    let ok_to = if el.not { miss } else { ok };
    if el.kinds != 0 || el.rows != 0 {
        let kind = f.ins().band_imm(c, 0xff);
        let small = f.ins().icmp_imm(IntCC::UnsignedLessThan, kind, 64);
        let set = f.ins().iconst(types::I64, el.kinds as i64);
        let bit = f.ins().ushr(set, kind);
        let bit = f.ins().band_imm(bit, 1);
        let zero = f.ins().iconst(types::I64, 0);
        let mut bit = f.ins().select(small, bit, zero);
        // A kind of a shared first byte: its rows' start's mark bit (`TokenElem::rows`).
        if el.rows != 0 {
            let r = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT));
            let r = f.ins().band_imm(r, i64::from(el.rows));
            bit = f.ins().bor(bit, r);
        }
        let other = f.create_block();
        if el.except.is_empty() {
            f.ins().brif(bit, ok_to, &[], other, &[]);
        } else {
            // Of the kinds, but not one of their hard keywords (`TokenElem::except`): those
            // go on to the words.
            let kind_hit = f.create_block();
            f.ins().brif(bit, kind_hit, &[], other, &[]);
            f.switch_to_block(kind_hit);
            let w = f.ins().ushr_imm(c, i64::from(crate::records::KIND_BITS));
            let w = f.ins().band_imm(w, 0xff);
            for &s in &el.except {
                let same = f.ins().icmp_imm(IntCC::Equal, w, i64::from(s) + 1);
                let next = f.create_block();
                f.ins().brif(same, other, &[], next, &[]);
                f.switch_to_block(next);
            }
            f.ins().jump(ok_to, &[]);
        }
        f.switch_to_block(other);
    }
    if !el.words.is_empty() {
        let w = f.ins().ushr_imm(c, i64::from(crate::records::KIND_BITS));
        let w = f.ins().band_imm(w, 0xff);
        for &s in &el.words {
            let same = f.ins().icmp_imm(IntCC::Equal, w, i64::from(s) + 1);
            let other = f.create_block();
            f.ins().brif(same, ok_to, &[], other, &[]);
            f.switch_to_block(other);
        }
    }
    // None of the choices: a `not` element holds, any other fails.
    if el.not {
        f.ins().jump(ok, &[]);
        f.switch_to_block(miss);
        f.ins().jump(fail, &[]);
    } else {
        f.ins().jump(fail, &[]);
        f.switch_to_block(miss);
        f.ins().jump(fail, &[]);
    }
    f.switch_to_block(ok);
}

fn token_pattern(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, m: &crate::program::TokenMark, extra_bit: Option<u8>, heads: &mut Vec<(Block, Step)>) -> Value {
    let flags = MemFlags::trusted();
    let codes = (e.ctx)(f, CODES);
    let n = (e.ctx)(f, NSTARTS);
    let next = (e.get)(f, Mem::Next);
    let here = f.ins().iadd_imm(next, -1);
    let (no, yes) = (f.create_block(), f.create_block());
    let stride = (e.ctx)(f, RECORD_STRIDE);
    let code_at = |f: &mut FunctionBuilder<'_>, j: Value| {
        let off = f.ins().imul(j, stride);
        let a = f.ins().iadd(codes, off);
        f.ins().uload32(flags, a, 0)
    };
    // 1 where a record is an extra (its code has the extra's bit); with no extras, none is.
    let is_extra = |f: &mut FunctionBuilder<'_>, c: Value| match extra_bit {
        Some(b) => {
            let x = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT) + i64::from(b));
            f.ins().band_imm(x, 1)
        }
        None => f.ins().iconst(types::I64, 0),
    };
    let test = elem_test;
    // The record after `at` that is no extra: a stated loop over the extras. Past the last, to
    // `past`; else the found record and its code.
    let after = |f: &mut FunctionBuilder<'_>, at: Value, past: Block, heads: &mut Vec<(Block, Step)>| -> (Value, Value) {
        let head = f.create_block();
        f.append_block_param(head, types::I64);
        f.ins().jump(head, &[BlockArg::from(at)]);
        f.switch_to_block(head);
        heads.push((head, Step::Token));
        let j = f.block_params(head)[0];
        let j = f.ins().iadd_imm(j, 1);
        let beyond = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, j, n);
        let on = f.create_block();
        f.ins().brif(beyond, past, &[], on, &[]);
        f.switch_to_block(on);
        let c = code_at(f, j);
        let x = is_extra(f, c);
        let found = f.create_block();
        f.ins().brif(x, head, &[BlockArg::from(j)], found, &[]);
        f.switch_to_block(found);
        (j, c)
    };
    // The elements after `at`, in turn: each at the next record; past the last, an `end`
    // element (the last) holds, any other goes to `fail`.
    let run_of = |f: &mut FunctionBuilder<'_>, els: &[&crate::program::TokenElem], mut at: Value, fail: Block, heads: &mut Vec<(Block, Step)>| -> Value {
        for el in els {
            let past = f.create_block();
            let (j, c) = after(f, at, past, heads);
            test(f, c, el, fail);
            let tested = f.current_block().expect("a block");
            f.switch_to_block(past);
            f.ins().jump(if el.end { yes } else { fail }, &[]);
            f.switch_to_block(tested);
            at = j;
        }
        at
    };
    for el in m.elems.iter().filter(|e| e.offset < 0) {
        // The record before, past the extras. No record before: an `end` element (`after ~E`)
        // holds there, any other fails.
        let head = f.create_block();
        f.append_block_param(head, types::I64);
        f.ins().jump(head, &[BlockArg::from(here)]);
        f.switch_to_block(head);
        heads.push((head, Step::Token));
        let j = f.block_params(head)[0];
        let first = f.ins().icmp_imm(IntCC::Equal, j, 0);
        let on = f.create_block();
        let held = f.create_block();
        f.ins().brif(first, if el.end { held } else { no }, &[], on, &[]);
        f.switch_to_block(on);
        let j = f.ins().iadd_imm(j, -1);
        let c = code_at(f, j);
        let x = is_extra(f, c);
        let found = f.create_block();
        f.ins().brif(x, head, &[BlockArg::from(j)], found, &[]);
        f.switch_to_block(found);
        test(f, c, el, no);
        f.ins().jump(held, &[]);
        f.switch_to_block(held);
    }
    // The token's own element, then those before the run, then the run (decision 0416): the
    // rest tried from each record of it on, as a forward scan, each turn a record's.
    let rest: Vec<&crate::program::TokenElem> = m.elems.iter().filter(|e| e.offset >= 0).collect();
    let run = rest.iter().position(|e| e.run).unwrap_or(rest.len());
    let mut at = here;
    for el in rest.iter().take(run) {
        if el.offset == 0 {
            let c = code_at(f, here);
            test(f, c, el, no);
        } else {
            at = run_of(f, std::slice::from_ref(el), at, no, heads);
        }
    }
    if let Some(r) = rest.get(run) {
        // The run's depths (Q339): a block parameter each, from 0.
        let counters = r.nests.iter().map(|n| usize::from(n.counter) + 1).max().unwrap_or(0);
        let scan = f.create_block();
        for _ in 0..=counters {
            f.append_block_param(scan, types::I64);
        }
        let zero = f.ins().iconst(types::I64, 0);
        let start: Vec<BlockArg> = std::iter::once(at).chain(std::iter::repeat_n(zero, counters)).map(BlockArg::from).collect();
        f.ins().jump(scan, &start);
        f.switch_to_block(scan);
        heads.push((scan, Step::Token));
        let from = f.block_params(scan)[0];
        let depth: Vec<Value> = f.block_params(scan)[1..].to_vec();
        let step = f.create_block();
        // The rest only where every depth is 0.
        if counters > 0 {
            let any = depth[1..].iter().fold(depth[0], |a, &d| f.ins().bor(a, d));
            let try_rest = f.create_block();
            f.ins().brif(any, step, &[], try_rest, &[]);
            f.switch_to_block(try_rest);
        }
        // Its `ending` (Q342): the record it took last (`from`, the one before it where it took
        // none) one the test takes, else one more record.
        if let Some(x) = &r.ending {
            let c = code_at(f, from);
            test(f, c, x, step);
        }
        run_of(f, &rest[run + 1..], from, step, heads);
        f.ins().jump(yes, &[]);
        // The rest fails here: one more record of the run, or none.
        f.switch_to_block(step);
        let (j, c) = after(f, from, no, heads);
        test(f, c, r, no);
        // 1 where a side of a depth takes the record, else 0.
        let hit = |f: &mut FunctionBuilder<'_>, c: Value, el: &crate::program::TokenElem| -> Value {
            let (miss, merged) = (f.create_block(), f.create_block());
            f.append_block_param(merged, types::I64);
            test(f, c, el, miss);
            let one = f.ins().iconst(types::I64, 1);
            f.ins().jump(merged, &[BlockArg::from(one)]);
            f.switch_to_block(miss);
            let zero = f.ins().iconst(types::I64, 0);
            f.ins().jump(merged, &[BlockArg::from(zero)]);
            f.switch_to_block(merged);
            f.block_params(merged)[0]
        };
        let mut next: Vec<BlockArg> = vec![BlockArg::from(j)];
        for (k, &d) in depth.iter().enumerate() {
            let mut d2 = d;
            for n in r.nests.iter().filter(|n| usize::from(n.counter) == k) {
                let h = hit(f, c, &n.test);
                if n.open {
                    d2 = f.ins().iadd(d2, h);
                } else {
                    // A closer where the depth is 0 ends the run: the rest did not hold.
                    let at_zero = f.ins().icmp_imm(IntCC::Equal, d2, 0);
                    let at_zero = f.ins().uextend(types::I64, at_zero);
                    let bad = f.ins().band(h, at_zero);
                    let on = f.create_block();
                    f.ins().brif(bad, no, &[], on, &[]);
                    f.switch_to_block(on);
                    d2 = f.ins().isub(d2, h);
                }
            }
            next.push(BlockArg::from(d2));
        }
        f.ins().jump(scan, &next);
    } else {
        f.ins().jump(yes, &[]);
    }
    let done = f.create_block();
    f.append_block_param(done, types::I64);
    f.switch_to_block(no);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().jump(done, &[BlockArg::from(zero)]);
    f.switch_to_block(yes);
    let one = f.ins().iconst(types::I64, 1);
    f.ins().jump(done, &[BlockArg::from(one)]);
    f.switch_to_block(done);
    f.block_params(done)[0]
}

/// Whether the token marks of one order hold (`program::patterns_hold`), from entry `first` of
/// `marks`: each entry's `token_pattern`, none of the `unless` ones and, where there are others,
/// one of them. 1 or 0.
fn token_patterns(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, marks: &[crate::program::TokenMark], first: usize, extra_bit: Option<u8>, heads: &mut Vec<(Block, Step)>) -> Value {
    let order = marks[first].order;
    let (mut none_of, mut one_of): (Option<Value>, Option<Value>) = (None, None);
    for m in marks[first..].iter().take_while(|m| m.order == order) {
        let v = token_pattern(f, e, m, extra_bit, heads);
        if m.unless {
            none_of = Some(none_of.map_or(v, |a| f.ins().bor(a, v)));
        } else {
            one_of = Some(one_of.map_or(v, |a| f.ins().bor(a, v)));
        }
    }
    let one = one_of.unwrap_or_else(|| f.ins().iconst(types::I64, 1));
    match none_of {
        Some(x) => {
            let not = f.ins().bxor_imm(x, 1);
            f.ins().band(one, not)
        }
        None => one,
    }
}

/// A gap pattern's entry (`Words::gap_patterns`, decision 0451) at the token, record `Next - 1`:
/// each element the record at its offset, as the lexer wrote them, extras included; past either
/// end, an element with `end` holds; its `starting`, the record's first byte, as the lean
/// lexer's `asi` reads it. 1 or 0.
fn gap_pattern(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, m: &crate::program::TokenMark, extra_bit: Option<u8>, heads: &mut Vec<(Block, Step)>) -> Value {
    let flags = MemFlags::trusted();
    let (codes, starts, bytes, n) = ((e.ctx)(f, CODES), (e.ctx)(f, STARTS), (e.ctx)(f, BYTES), (e.ctx)(f, NSTARTS));
    let stride = (e.ctx)(f, RECORD_STRIDE);
    // The token's record, the one before `Next` (at the stream's end, past the last, since the
    // end's token sets `Next` one past the count: `refuses` asks there).
    let next = (e.get)(f, Mem::Next);
    let here = f.ins().iadd_imm(next, -1);
    let (no, yes, done) = (f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(done, types::I64);
    // `since`: one past the grammar token before (0: none before the file's first), back over
    // the extras from the token: a stated loop, a few records at most (the comments of a gap).
    let since = match extra_bit.filter(|_| m.elems.iter().any(|el| el.since)) {
        Some(b) => {
            let (head, test, found) = (f.create_block(), f.create_block(), f.create_block());
            f.append_block_param(head, types::I64);
            f.append_block_param(found, types::I64);
            f.ins().jump(head, &[BlockArg::from(here)]);
            f.switch_to_block(head);
            heads.push((head, Step::Token));
            let g = f.block_params(head)[0];
            let first = f.ins().icmp_imm(IntCC::Equal, g, 0);
            f.ins().brif(first, found, &[BlockArg::from(g)], test, &[]);
            f.switch_to_block(test);
            let j = f.ins().iadd_imm(g, -1);
            let off = f.ins().imul(j, stride);
            let a = f.ins().iadd(codes, off);
            let c = f.ins().uload32(flags, a, 0);
            let x = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT) + i64::from(b));
            let x = f.ins().band_imm(x, 1);
            f.ins().brif(x, head, &[BlockArg::from(j)], found, &[BlockArg::from(g)]);
            f.switch_to_block(found);
            Some(f.block_params(found)[0])
        }
        // No extras: the grammar token before is the record before.
        None => m.elems.iter().any(|el| el.since).then_some(here),
    };
    for el in &m.elems {
        let (on, past) = (f.create_block(), f.create_block());
        let j = match since.filter(|_| el.since) {
            Some(g) => {
                // From the grammar token before, before the token; before the file's first,
                // an element with `end` holds, and the next element is tested.
                let j = f.ins().iadd_imm(g, i64::from(el.offset) - 1);
                let inside = f.ins().icmp(IntCC::UnsignedLessThan, j, here);
                let out = f.create_block();
                f.ins().brif(inside, on, &[], out, &[]);
                f.switch_to_block(out);
                if el.end {
                    let none = f.ins().icmp_imm(IntCC::Equal, j, -1);
                    f.ins().brif(none, past, &[], no, &[]);
                } else {
                    f.ins().jump(no, &[]);
                }
                j
            }
            None => {
                // Past either end (a negative index is past the last too, unsigned): an
                // element with `end` holds there, and the next element is tested.
                let j = f.ins().iadd_imm(here, i64::from(el.offset));
                let inside = f.ins().icmp(IntCC::UnsignedLessThan, j, n);
                f.ins().brif(inside, on, &[], if el.end { past } else { no }, &[]);
                j
            }
        };
        f.switch_to_block(on);
        let off = f.ins().imul(j, stride);
        let a = f.ins().iadd(codes, off);
        let c = f.ins().uload32(flags, a, 0);
        elem_test(f, c, el, no);
        if el.first != [0; 4] {
            let sa = f.ins().iadd(starts, off);
            let st = f.ins().uload32(flags, sa, 0);
            let ba = f.ins().iadd(bytes, st);
            let b = f.ins().uload8(types::I64, flags, ba, 0);
            // The set's word of the byte, and its bit.
            let q = f.ins().ushr_imm(b, 6);
            let mut word = f.ins().iconst(types::I64, el.first[3] as i64);
            for k in (0..3).rev() {
                let is = f.ins().icmp_imm(IntCC::Equal, q, k);
                let w = f.ins().iconst(types::I64, el.first[k as usize] as i64);
                word = f.ins().select(is, w, word);
            }
            let sh = f.ins().band_imm(b, 63);
            let bit = f.ins().ushr(word, sh);
            let bit = f.ins().band_imm(bit, 1);
            let on = f.create_block();
            f.ins().brif(bit, on, &[], no, &[]);
            f.switch_to_block(on);
        }
        f.ins().jump(past, &[]);
        f.switch_to_block(past);
    }
    f.ins().jump(yes, &[]);
    f.switch_to_block(yes);
    let one = f.ins().iconst(types::I64, 1);
    f.ins().jump(done, &[BlockArg::from(one)]);
    f.switch_to_block(no);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().jump(done, &[BlockArg::from(zero)]);
    f.switch_to_block(done);
    f.block_params(done)[0]
}

/// A gap pattern's entries together, from `first`: none of the `unless` ones and one of the
/// others (`program::patterns_hold`).
fn gap_patterns(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, marks: &[crate::program::TokenMark], first: usize, extra_bit: Option<u8>, heads: &mut Vec<(Block, Step)>) -> Value {
    let order = marks[first].order;
    let (mut none_of, mut one_of): (Option<Value>, Option<Value>) = (None, None);
    for m in marks[first..].iter().take_while(|m| m.order == order) {
        let v = gap_pattern(f, e, m, extra_bit, heads);
        if m.unless {
            none_of = Some(none_of.map_or(v, |a| f.ins().bor(a, v)));
        } else {
            one_of = Some(one_of.map_or(v, |a| f.ins().bor(a, v)));
        }
    }
    let one = one_of.unwrap_or_else(|| f.ins().iconst(types::I64, 1));
    match none_of {
        Some(x) => {
            let not = f.ins().bxor_imm(x, 1);
            f.ins().band(one, not)
        }
        None => one,
    }
}

/// The token's end, as a word reads it (`EndAt`).
fn token_end_at(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, end_at: EndAt) -> Value {
    match end_at {
        EndAt::Taken => (e.get)(f, Mem::Limit),
        EndAt::Field => (e.get)(f, Mem::End),
        EndAt::Untaken => {
            let (next, nstarts, limit, start) = ((e.get)(f, Mem::Next), (e.ctx)(f, NSTARTS), (e.get)(f, Mem::Limit), (e.get)(f, Mem::Start));
            let at_end = f.ins().icmp(IntCC::UnsignedGreaterThan, next, nstarts);
            f.ins().select(at_end, limit, start)
        }
    }
}

/// A u32 column's slot `i`, loaded or stored.
fn slot_get(f: &mut FunctionBuilder<'_>, c: Value, i: Value) -> Value {
    let off = f.ins().ishl_imm(i, 2);
    let a = f.ins().iadd(c, off);
    f.ins().uload32(MemFlags::trusted(), a, 0)
}

fn slot_put(f: &mut FunctionBuilder<'_>, c: Value, i: Value, x: Value) {
    let off = f.ins().ishl_imm(i, 2);
    let a = f.ins().iadd(c, off);
    f.ins().istore32(MemFlags::trusted(), x, a, 0);
}

/// The kind a word names as its level's word (`REMEMBERED`): refused where the level has none.
fn remembered_kind(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>) -> Value {
    let w = (e.get)(f, Mem::Word);
    let none = f.ins().icmp_imm(IntCC::Equal, w, 0);
    (e.guard)(f, none);
    w
}

/// A keyword's leaf kind (`WORD_KIND`): its token's word's (`Words::word_kinds`), refused where
/// the word has none. As LLVM lowers a `match` that maps a value to a value: runs of strings
/// whose kinds rise with them are one add each (`Kind::DotDecl + (d - W_DECL)`), a compare
/// tree picks the run.
fn word_kind(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, word_kinds: &[(u16, u16)]) -> Value {
    let code = (e.get)(f, Mem::Taken);
    let w = f.ins().ushr_imm(code, i64::from(crate::records::KIND_BITS));
    let w = f.ins().band_imm(w, 0xff);
    // The word's string: the word less one (0, no word, wraps past every string).
    let s = f.ins().iadd_imm(w, -1);
    // Runs: (first string, past the last, kind less string).
    let mut runs: Vec<(u32, u32, i64)> = Vec::new();
    for &(at, k) in word_kinds {
        let delta = i64::from(k) - i64::from(at);
        match runs.last_mut() {
            Some(r) if r.1 == u32::from(at) && r.2 == delta => r.1 += 1,
            _ => runs.push((u32::from(at), u32::from(at) + 1, delta)),
        }
    }
    let join = f.create_block();
    f.append_block_param(join, types::I64);
    let none = f.create_block();
    let mut cases: Vec<(u32, Block)> = Vec::new();
    let mut made = Vec::new();
    let mut at = 0;
    for &(a, b, delta) in &runs {
        if a > at {
            cases.push((at, none));
        }
        let blk = f.create_block();
        cases.push((a, blk));
        made.push((blk, delta));
        at = b;
    }
    cases.push((at, none));
    dispatch(f, s, &cases, u64::MAX);
    for (blk, delta) in made {
        f.switch_to_block(blk);
        let k = f.ins().iadd_imm(s, delta);
        f.ins().jump(join, &[k.into()]);
    }
    f.switch_to_block(none);
    let bad = f.ins().iconst(types::I8, 1);
    (e.guard)(f, bad);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().jump(join, &[zero.into()]);
    f.switch_to_block(join);
    f.block_params(join)[0]
}

/// What the node words need to write the tape as it stands at the end (`TableFns`): whether
/// rows carry marks (`head`, `stream_facts::marks`: a wrap, `WRAPPED`; a dropped row taken out
/// from among its children, `DROPPED_IN`), the tape's widths, the functions.
#[derive(Clone, Copy)]
struct Table<'a> {
    inner: bool,
    w: rerail_tape_format::Widths,
    fns: &'a TableFns,
    /// A region read in parts that `TOKEN` splits (`Split::Tier2`), and tier 1's block.
    split: Option<(&'a SplitParts, &'a crate::block_native::BlockNative)>,
    /// The records `TOKEN` makes from each token's start (`Fetch`, page 40).
    fetch: Option<&'a Fetch>,
    /// A format 16 run whose every node's key is known at its word (`stream_facts::direct_keys`): each node word's key
    /// bits from the words after it, by its place. Each node then goes to the writer's ops at its word, with no row.
    direct: Option<&'a [(u32, u32)]>,
    /// A padded root (`Structure::padded`): its start given at its pass (`pass`).
    padded: bool,
}

/// The first bit of output `o` at or after byte `from`, a word at a time (`records::first_bit`),
/// and whether there is none in the file (then the byte is the file's length). Bits past the
/// file's end are zero in every output.
fn first_bit_from(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, bw: &crate::native_cpu_bytes::Bytewise<'_>, o: u16, from: Value, heads: &mut Vec<(Block, Step)>) -> (Value, Value) {
    let len = (e.ctx)(f, LEN);
    let (scan, found, none) = (f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(scan, types::I64);
    f.append_block_param(scan, types::I64);
    let done = f.create_block();
    f.append_block_param(done, types::I64);
    f.append_block_param(done, types::I64);
    let q = f.ins().ushr_imm(from, 6);
    let sh = f.ins().band_imm(from, 63);
    let ones = f.ins().iconst(types::I64, -1);
    let m = f.ins().ishl(ones, sh);
    f.ins().jump(scan, &[q.into(), m.into()]);
    f.switch_to_block(scan);
    heads.push((scan, Step::Word));
    let (w, m) = (f.block_params(scan)[0], f.block_params(scan)[1]);
    let x = output_word(f, e, bw, o, w, heads);
    let x = f.ins().band(x, m);
    let more = f.create_block();
    f.ins().brif(x, found, &[], more, &[]);
    f.switch_to_block(more);
    let w1 = f.ins().iadd_imm(w, 1);
    let at = f.ins().ishl_imm(w1, 6);
    let past = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, at, len);
    let all = f.ins().iconst(types::I64, -1);
    f.ins().brif(past, none, &[], scan, &[w1.into(), all.into()]);
    f.switch_to_block(found);
    let tz = f.ins().ctz(x);
    let w64 = f.ins().ishl_imm(w, 6);
    let at = f.ins().iadd(w64, tz);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().jump(done, &[at.into(), zero.into()]);
    f.switch_to_block(none);
    let one = f.ins().iconst(types::I64, 1);
    f.ins().jump(done, &[len.into(), one.into()]);
    f.switch_to_block(done);
    (f.block_params(done)[0], f.block_params(done)[1])
}

/// The block of end rule `rule` for a token from `s` (`at` its address, `b` its first byte), its
/// last and whether it refuses the file, given to `join` (`fetch_last`).
#[allow(clippy::too_many_arguments)]
fn emit_rule(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, ft: &Fetch, bw: &crate::native_cpu_bytes::Bytewise<'_>, rule: &crate::records::EndRule, s: Value, at: Value, b: Value, join: Block, heads: &mut Vec<(Block, Step)>) {
    use crate::records::{EndRule, ReadNone};
    let zero = f.ins().iconst(types::I64, 0);
    let one = f.ins().iconst(types::I64, 1);
    match *rule {
        // The first byte, and the next where the output holds at the first.
        EndRule::Pair(o) => {
            let q = f.ins().ushr_imm(s, 6);
            let w = output_word(f, e, bw, o, q, heads);
            let sh = f.ins().band_imm(s, 63);
            let x = f.ins().ushr(w, sh);
            let bit = f.ins().band_imm(x, 1);
            let l = f.ins().iadd(s, bit);
            f.ins().jump(join, &[l.into(), zero.into()]);
        }
        // The first byte, then the output's run after it: the first byte from `s + 1` the
        // output does not hold, less one.
        EndRule::Run(o) => {
            let ones = f.ins().iconst(types::I64, -1);
            let s1 = f.ins().iadd_imm(s, 1);
            // From the bytes: the 16 from `s + 1` first, one vector (`short_word`), as a lexer
            // reads a short token's bytes and no more; past them, 64 at a time below.
            let from = if bw.can(o) {
                let consts = crate::native_cpu_block::Consts::make(f);
                let file = crate::native_cpu_bytes::File { bytes: (e.ctx)(f, BYTES), len: (e.ctx)(f, LEN), nibble: consts.nibble, weights: consts.weights, even: consts.even };
                let x = bw.short_word(f, &file, o, s1, heads);
                let stride = i64::from(bw.short_stride(&[o]));
                let lanes = if 16 * stride >= 64 { -1 } else { ((1u64 << (16 * stride)) - 1) as i64 };
                let not = f.ins().bnot(x);
                let not = f.ins().band_imm(not, lanes);
                let (short, long) = (f.create_block(), f.create_block());
                f.ins().brif(not, short, &[], long, &[]);
                f.switch_to_block(short);
                let tz = f.ins().ctz(not);
                let k = lane_of(f, tz, stride);
                let l = f.ins().iadd(s, k);
                f.ins().jump(join, &[l.into(), zero.into()]);
                f.switch_to_block(long);
                f.ins().iadd_imm(s1, 16)
            } else {
                s1
            };
            // The output's complement's first bit from `from`: a word at a time.
            let (scan, found) = (f.create_block(), f.create_block());
            f.append_block_param(scan, types::I64);
            f.append_block_param(scan, types::I64);
            let q = f.ins().ushr_imm(from, 6);
            let sh = f.ins().band_imm(from, 63);
            let m = f.ins().ishl(ones, sh);
            f.ins().jump(scan, &[q.into(), m.into()]);
            f.switch_to_block(scan);
            heads.push((scan, Step::Word));
            let (w, m) = (f.block_params(scan)[0], f.block_params(scan)[1]);
            let x = output_word(f, e, bw, o, w, heads);
            let not = f.ins().bnot(x);
            let not = f.ins().band(not, m);
            let more = f.create_block();
            f.ins().brif(not, found, &[], more, &[]);
            f.switch_to_block(more);
            let w1 = f.ins().iadd_imm(w, 1);
            f.ins().jump(scan, &[w1.into(), ones.into()]);
            f.switch_to_block(found);
            let tz = f.ins().ctz(not);
            let w64 = f.ins().ishl_imm(w, 6);
            let past = f.ins().iadd(w64, tz);
            let l = f.ins().iadd_imm(past, -1);
            f.ins().jump(join, &[l.into(), zero.into()]);
        }
        // The output's first bit at or after the first byte.
        EndRule::Last(o) => {
            let (l, none) = first_bit_from(f, e, bw, o, s, heads);
            f.ins().jump(join, &[l.into(), none.into()]);
        }
        // A region's token: its closer's first bit after the opener, its last `off` from it.
        // At a byte its opener does not hold, the file is refused (`read_last` with no other
        // rule; `fetch_of` takes no other).
        EndRule::Read(i) => {
            let a = ft.readers[usize::from(i)].arms[0];
            let q = f.ins().ushr_imm(s, 6);
            let w = output_word(f, e, bw, a.opener, q, heads);
            let sh = f.ins().band_imm(s, 63);
            let x = f.ins().ushr(w, sh);
            let opens = f.ins().band_imm(x, 1);
            let (read, not) = (f.create_block(), f.create_block());
            f.ins().brif(opens, read, &[], not, &[]);
            f.switch_to_block(not);
            f.set_cold_block(not);
            f.ins().jump(join, &[s.into(), one.into()]);
            f.switch_to_block(read);
            // Read as `parse_string` (`sequential`): the record is the opener alone, its parts
            // and closer split from it.
            if ft.seq[usize::from(i)] {
                f.ins().jump(join, &[s.into(), zero.into()]);
                return;
            }
            let s1 = f.ins().iadd_imm(s, 1);
            let (c, none) = first_bit_from(f, e, bw, a.closer, s1, heads);
            let (shut, open) = (f.create_block(), f.create_block());
            f.ins().brif(none, open, &[], shut, &[]);
            f.switch_to_block(shut);
            let l = f.ins().iadd_imm(c, i64::from(a.off));
            let l = f.ins().smax(l, s);
            f.ins().jump(join, &[l.into(), zero.into()]);
            f.switch_to_block(open);
            f.set_cold_block(open);
            match a.none {
                ReadNone::Refuse => {
                    f.ins().jump(join, &[s.into(), one.into()]);
                }
                ReadNone::End => {
                    let len = (e.ctx)(f, LEN);
                    let l = f.ins().iadd_imm(len, -1);
                    f.ins().jump(join, &[l.into(), zero.into()]);
                }
                ReadNone::Bytes(m) => {
                    let l = f.ins().iadd_imm(s, i64::from(m) - 1);
                    f.ins().jump(join, &[l.into(), zero.into()]);
                }
            }
        }
        // The first byte, and the next where the pair is one of the table's: the first byte's
        // row, then the next byte's bit in the row's set (`PairTable::holds`).
        EndRule::Next(i) => {
            let (row, sets) = &ft.nexts[usize::from(i)];
            let ro = MemFlags::trusted().with_readonly();
            let rows = table_address(f, row.as_ptr());
            let ra = f.ins().iadd(rows, b);
            let r = f.ins().uload8(types::I64, ro, ra, 0);
            let next = f.ins().uload8(types::I64, MemFlags::trusted(), at, 1);
            let base = table_address(f, sets.as_ptr());
            let off = f.ins().ishl_imm(r, 5);
            let set = f.ins().iadd(base, off);
            let q = f.ins().ushr_imm(next, 6);
            let q = f.ins().ishl_imm(q, 3);
            let wa = f.ins().iadd(set, q);
            let w = f.ins().load(types::I64, ro, wa, 0);
            let sh = f.ins().band_imm(next, 63);
            let x = f.ins().ushr(w, sh);
            let bit = f.ins().band_imm(x, 1);
            let l = f.ins().iadd(s, bit);
            f.ins().jump(join, &[l.into(), zero.into()]);
        }
    }
}

/// A token's last byte from its first, `s`, by its first byte's end rule (`records::lasts`, the
/// rules as `last_by_rule` and `read_last` read them), and whether it refuses the file (a region
/// with no closer). A byte no rule names is a token alone.
fn fetch_last(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, ft: &Fetch, bw: &crate::native_cpu_bytes::Bytewise<'_>, s: Value, heads: &mut Vec<(Block, Step)>) -> (Value, Value) {
    let bytes = (e.ctx)(f, BYTES);
    let at = f.ins().iadd(bytes, s);
    let b = f.ins().uload8(types::I64, MemFlags::trusted(), at, 0);
    let ends = table_address(f, ft.ends.as_ptr());
    let ka = f.ins().iadd(ends, b);
    let k = f.ins().uload8(types::I32, MemFlags::trusted().with_readonly(), ka, 0);
    let join = f.create_block();
    f.append_block_param(join, types::I64);
    f.append_block_param(join, types::I64);
    let zero = f.ins().iconst(types::I64, 0);
    // A byte no rule names: the byte alone.
    let alone = f.create_block();
    let blocks: Vec<Block> = ft.rules.iter().map(|_| f.create_block()).collect();
    let calls: Vec<_> = blocks.iter().map(|&x| f.func.dfg.block_call(x, &[])).collect();
    let default = f.func.dfg.block_call(alone, &[]);
    let jt = f.create_jump_table(cranelift_codegen::ir::JumpTableData::new(default, &calls));
    f.ins().br_table(k, jt);
    f.switch_to_block(alone);
    f.ins().jump(join, &[s.into(), zero.into()]);
    for (rule, &blk) in ft.rules.iter().zip(&blocks) {
        f.switch_to_block(blk);
        emit_rule(f, e, ft, bw, rule, s, at, b, join, heads);
    }
    f.switch_to_block(join);
    (f.block_params(join)[0], f.block_params(join)[1])
}

/// `fetch_last` where the token's first byte's rule is known (`k`, a `SWITCH` edge's, page 40):
/// that rule's block alone, no dispatch; a token of one byte by its rule (`one_byte`), none.
fn fetch_last_by(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, ft: &Fetch, bw: &crate::native_cpu_bytes::Bytewise<'_>, k: u8, s: Value, heads: &mut Vec<(Block, Step)>) -> (Value, Value) {
    let zero = f.ins().iconst(types::I64, 0);
    let Some(rule) = ft.rules.get(usize::from(k)).filter(|r| !one_byte(ft, r)) else { return (s, zero) };
    let bytes = (e.ctx)(f, BYTES);
    let at = f.ins().iadd(bytes, s);
    let b = f.ins().uload8(types::I64, MemFlags::trusted(), at, 0);
    let join = f.create_block();
    f.append_block_param(join, types::I64);
    f.append_block_param(join, types::I64);
    emit_rule(f, e, ft, bw, rule, s, at, b, join, heads);
    f.switch_to_block(join);
    (f.block_params(join)[0], f.block_params(join)[1])
}

const WRAPPED: i64 = 1;
const DROPPED_IN: i64 = 2;

impl Table<'_> {
    /// A kind's change on row `at`: its key's kind (16 bits).
    /// Node `at`'s kind made `kind`. Where it is committed, its codes taken back (`uncode`); in a format 16 run,
    /// where the ops already took it (the root, which they hold open), retagged there too.
    fn retag(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, rows: Value, at: Value, kind: Value) {
        let row = NodeCol::row(f, rows, at);
        f.ins().istore16(MemFlags::trusted(), kind, row, AT_KIND);
        let Some(s) = &self.fns.sixteen else {
            self.uncode(f, e, at);
            return;
        };
        let passed = (e.get)(f, Mem::Codes);
        let before = f.ins().icmp(IntCC::UnsignedLessThan, at, passed);
        let (again, after, done) = (f.create_block(), f.create_block(), f.create_block());
        f.ins().brif(before, again, &[], after, &[]);
        f.switch_to_block(again);
        f.set_cold_block(again);
        let (desc, numbers) = ((e.ctx)(f, WRITER), (e.ctx)(f, NUMBERS));
        let (snode, skind, supers) = ((e.ctx)(f, SUPER_NODE), (e.ctx)(f, SUPER_KIND), (e.get)(f, Mem::Supers));
        call_own(f, s.retag.0, &s.retag.1, &[desc, numbers, rows, snode, skind, supers, at]);
        f.ins().jump(done, &[]);
        f.switch_to_block(after);
        self.uncode(f, e, at);
        f.ins().jump(done, &[]);
        f.switch_to_block(done);
    }

    /// Node `node`'s offset, at its push with start `start`: from its block's base, which it sets
    /// where it starts a block; where it starts before the base or 0x10000 past it, the block's
    /// base and offsets again (`blocks`). `n`: the nodes, it counted.
    fn push_off(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, node: Value, start: Value, n: Value) {
        // A format 16 run's writer makes its own blocks (decision 0434): no format 15 offsets or bases.
        if self.fns.sixteen.is_some() {
            return;
        }
        let off_c = (e.ctx)(f, OFF_C);
        let low = f.ins().band_imm(node, rerail_tape_format::CHECKPOINT_STRIDE as i64 - 1);
        let base = (e.get)(f, Mem::BlockBase);
        let off = f.ins().isub(start, base);
        let off = f.ins().band_imm(off, 0xffff_ffff);
        let far = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, off, 0xffff);
        let (first, other, near, again, done) = (f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block());
        f.ins().brif(low, other, &[], first, &[]);
        // The block's first node: its start is the base.
        f.switch_to_block(first);
        let base_c = (e.ctx)(f, BASE_C);
        let b = f.ins().ushr_imm(node, rerail_tape_format::CHECKPOINT_STRIDE.trailing_zeros() as i64);
        slot_put(f, base_c, b, start);
        (e.set)(f, Mem::BlockBase, start);
        let zero = f.ins().iconst(types::I64, 0);
        crate::native_cpu_tape::put(f, off_c, node, zero, self.w.off);
        f.ins().jump(done, &[]);
        f.switch_to_block(other);
        f.ins().brif(far, again, &[], near, &[]);
        f.switch_to_block(near);
        crate::native_cpu_tape::put(f, off_c, node, off, self.w.off);
        f.ins().jump(done, &[]);
        f.switch_to_block(again);
        f.set_cold_block(again);
        let b = f.ins().ushr_imm(node, rerail_tape_format::CHECKPOINT_STRIDE.trailing_zeros() as i64);
        self.blocks(f, e, b, n);
        f.ins().jump(done, &[]);
        f.switch_to_block(done);
    }

    /// Blocks `b` on, of `n` nodes: their bases and offsets again (`native_cpu_tape::blocks_from`),
    /// the last base kept, a wide offset flagged.
    fn blocks(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, b: Value, n: Value) {
        if self.fns.sixteen.is_some() {
            return;
        }
        let (rows, base_c, off_c) = ((e.ctx)(f, ROWS), (e.ctx)(f, BASE_C), (e.ctx)(f, OFF_C));
        let (id, sig, _) = &self.fns.blocks;
        let r = call_own(f, *id, sig, &[rows, base_c, off_c, b, n]);
        (e.set)(f, Mem::BlockBase, r[0]);
        let wide = (e.get)(f, Mem::Wide);
        let wide = f.ins().bor(wide, r[1]);
        (e.set)(f, Mem::Wide, wide);
    }

    /// The table cut back to `n` nodes (a fold, a merge, a drop): its last block's base again,
    /// where a block is left part full.
    fn cut_to(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, n: Value) {
        if self.fns.sixteen.is_some() {
            return;
        }
        let low = f.ins().band_imm(n, rerail_tape_format::CHECKPOINT_STRIDE as i64 - 1);
        let (part, done) = (f.create_block(), f.create_block());
        f.ins().brif(low, part, &[], done, &[]);
        f.switch_to_block(part);
        let n1 = f.ins().iadd_imm(n, -1);
        let b = f.ins().ushr_imm(n1, rerail_tape_format::CHECKPOINT_STRIDE.trailing_zeros() as i64);
        self.blocks(f, e, b, n);
        f.ins().jump(done, &[]);
        f.switch_to_block(done);
    }

    /// Node `at` closed: its size, and its tail from its last descendant (`last`, a row) to
    /// `end`; a tail that does not fit 2 bytes (or wraps: an end before its last descendant's
    /// start) flagged. `start`: the node's start where the caller has it (a leaf just written, `at` == `last`),
    /// not read back from its row.
    fn close_cells(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, node: Closed, end: Value, start: Option<Value>) {
        let Closed { at, size, last } = node;
        // A format 16 run's pass reads only the sizes (`native_cpu_tape::pass`): no tails.
        if self.fns.sixteen.is_some() {
            let size_c = (e.ctx)(f, SIZE_C);
            crate::native_cpu_tape::put(f, size_c, at, size, self.w.size);
            return;
        }
        let rows = (e.ctx)(f, ROWS);
        let from = if self.w.from_start { at } else { last };
        let s = match start {
            Some(s) => s,
            None => crate::native_cpu_tape::start(f, rows, from),
        };
        let tail = f.ins().isub(end, s);
        let tail = f.ins().band_imm(tail, 0xffff_ffff);
        let (size_c, tail_c) = ((e.ctx)(f, SIZE_C), (e.ctx)(f, TAIL_C));
        crate::native_cpu_tape::put(f, size_c, at, size, self.w.size);
        crate::native_cpu_tape::put(f, tail_c, at, tail, self.w.tail);
        let mut odd = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, tail, if self.w.tail == 2 { 0xffff } else { 0x7fff_ffff });
        if self.w.from_start {
            let ls = match start {
                Some(s) => s,
                None => crate::native_cpu_tape::start(f, rows, last),
            };
            let early = f.ins().icmp(IntCC::UnsignedLessThan, end, ls);
            odd = f.ins().bor(odd, early);
        }
        let (flag, done) = (f.create_block(), f.create_block());
        f.ins().brif(odd, flag, &[], done, &[]);
        f.switch_to_block(flag);
        f.set_cold_block(flag);
        let wide = (e.get)(f, Mem::Wide);
        let wide = f.ins().bor_imm(wide, crate::native_cpu_tape::WIDE_TAIL);
        (e.set)(f, Mem::Wide, wide);
        f.ins().jump(done, &[]);
        f.switch_to_block(done);
    }

    /// Nodes `from..to` final: their codes (`native_cpu_tape::commit`). In a format 16 run (decision 0434 §6,
    /// R17), `to` is the commit point: the nodes from the last passed up to the commit point before it are passed
    /// to the ops (`pass`). A commit point is final only where no word reaches before it, and one word does: a
    /// fold of the root's two leaves retags the first, which is the commit point before the second's
    /// (`fold_word`). So the ops take nodes one commit point behind. `Mem::Coded` is the commit point and
    /// `Mem::Codes` the nodes passed.
    fn commit(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, to: Value) {
        if self.fns.sixteen.is_some() {
            let (passed, point) = ((e.get)(f, Mem::Codes), (e.get)(f, Mem::Coded));
            self.pass(f, e, passed, point, true);
            let passed = f.ins().umax(passed, point);
            (e.set)(f, Mem::Codes, passed);
            (e.set)(f, Mem::Coded, to);
            return;
        }
        let from = (e.get)(f, Mem::Coded);
        let (rows, image, id_of, kf, first) = ((e.ctx)(f, ROWS), (e.ctx)(f, IMAGE), (e.ctx)(f, ID_OF), (e.ctx)(f, CODE_KF), (e.ctx)(f, CODE_FIRST));
        let code = f.ins().iadd_imm(image, rerail_tape_format::HEADER_BYTES as i64);
        let codes = (e.get)(f, Mem::Codes);
        let (id, sig, _) = &self.fns.commit;
        let (snode, skind, supers) = ((e.ctx)(f, SUPER_NODE), (e.ctx)(f, SUPER_KIND), (e.get)(f, Mem::Supers));
        let r = call_own(f, *id, sig, &[rows, code, id_of, kf, first, from, to, codes, snode, skind, supers]);
        (e.set)(f, Mem::Codes, r[0]);
        (e.set)(f, Mem::Coded, to);
    }

    /// Nodes `from..to` passed to the ops (`native_cpu_tape::pass`); the root kept open where `keep`.
    fn pass(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, from: Value, to: Value, keep: bool) {
        let s = self.fns.sixteen.as_ref().expect("a format 16 run's");
        let (desc, rows, size_c, numbers) = ((e.ctx)(f, WRITER), (e.ctx)(f, ROWS), (e.ctx)(f, SIZE_C), (e.ctx)(f, NUMBERS));
        // The root passed first: its start the node's after it (none: the file's end), as
        // tree-sitter's root starts past the file's leading blanks. That node is made: a pass
        // leaves the last node, and at the end there is none after the root only in a file of
        // no token or comment.
        let (root, on) = (f.create_block(), f.create_block());
        let from_root = if self.padded { from } else { f.ins().iconst(types::I64, 1) };
        f.ins().brif(from_root, on, &[], root, &[]);
        f.switch_to_block(root);
        let nodes = (e.get)(f, Mem::Nodes);
        let more = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, nodes, 1);
        let one = f.ins().iconst(types::I64, 1);
        let zero = f.ins().iconst(types::I64, 0);
        let after = f.ins().select(more, one, zero);
        let after_row = NodeCol::row(f, rows, after);
        let first = NodeCol::of(rows, AT_START).get_row(f, after_row);
        let len = (e.ctx)(f, LEN);
        let first = f.ins().select(more, first, len);
        let root_row = NodeCol::row(f, rows, zero);
        NodeCol::of(rows, AT_START).put_row(f, root_row, first);
        f.ins().jump(on, &[]);
        f.switch_to_block(on);
        let (snode, skind, supers) = ((e.ctx)(f, SUPER_NODE), (e.ctx)(f, SUPER_KIND), (e.get)(f, Mem::Supers));
        let keep = f.ins().iconst(types::I64, i64::from(keep));
        call_own(f, s.pass.0, &s.pass.1, &[desc, rows, size_c, numbers, snode, skind, supers, from, to, keep]);
    }

    /// An edit of node `node`: where it is committed, the codes from it taken back
    /// (`native_cpu_tape::uncode`), numbered again at the next commit.
    fn uncode(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, node: Value) {
        let coded = (e.get)(f, Mem::Coded);
        let before = f.ins().icmp(IntCC::UnsignedLessThan, node, coded);
        let (back, done) = (f.create_block(), f.create_block());
        f.ins().brif(before, back, &[], done, &[]);
        f.switch_to_block(back);
        f.set_cold_block(back);
        if let Some(s) = &self.fns.sixteen {
            // The commit point moved back to the edit; an edit of a node passed is the ops' failure.
            let passed = (e.get)(f, Mem::Codes);
            let early = f.ins().icmp(IntCC::UnsignedLessThan, node, passed);
            let (bad, moved) = (f.create_block(), f.create_block());
            f.ins().brif(early, bad, &[], moved, &[]);
            f.switch_to_block(bad);
            f.set_cold_block(bad);
            let desc = (e.ctx)(f, WRITER);
            crate::native_cpu_tape::fail_op(f, &s.ops, desc, rerail_clif::writer::fail::PASSED, node);
            f.ins().jump(moved, &[]);
            f.switch_to_block(moved);
            (e.set)(f, Mem::Coded, node);
            f.ins().jump(done, &[]);
            f.switch_to_block(done);
            return;
        }
        let (id_of, kf, first) = ((e.ctx)(f, ID_OF), (e.ctx)(f, CODE_KF), (e.ctx)(f, CODE_FIRST));
        let codes = (e.get)(f, Mem::Codes);
        let (id, sig, _) = &self.fns.uncode;
        let r = call_own(f, *id, sig, &[id_of, kf, first, codes, node]);
        (e.set)(f, Mem::Codes, r[0]);
        (e.set)(f, Mem::Coded, node);
        f.ins().jump(done, &[]);
        f.switch_to_block(done);
    }
}

/// `LEAF`: its row (its key, its field above its kind or none; its bytes), its offset, its size
/// (1) and tail, the pending extras, the level's last node.
fn leaf_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value, shape: LeafShape, wrapping: bool) {
    let LeafShape { deferred, extra, end: end_at } = shape;
    let flags = MemFlags::trusted();
    let zero = f.ins().iconst(types::I64, 0);
    let pending = if deferred {
        // The first pending leaf keeps the end of the token before it.
        let (q, keep, last_end) = ((e.get)(f, Mem::Pending), (e.get)(f, Mem::Keep), (e.get)(f, Mem::LastEnd));
        let keep = f.ins().select(q, keep, last_end);
        (e.set)(f, Mem::Keep, keep);
        f.ins().iadd_imm(q, 1)
    } else {
        zero
    };
    (e.set)(f, Mem::Pending, pending);
    let (nodes, start, end) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Start), token_end_at(f, e, end_at));
    let rows = (e.ctx)(f, ROWS);
    let row = NodeCol::row(f, rows, nodes);
    f.ins().istore32(flags, kind, row, AT_KIND);
    NodeCol::of(rows, AT_START).put_row(f, row, start);
    NodeCol::of(rows, AT_END).put_row(f, row, end);
    let n1 = f.ins().iadd_imm(nodes, 1);
    t.push_off(f, e, nodes, start, n1);
    let one = f.ins().iconst(types::I64, 1);
    t.close_cells(f, e, Closed { at: nodes, size: one, last: nodes }, end, Some(start));
    if t.inner {
        let head = (e.ctx)(f, HEAD);
        slot_put(f, head, nodes, zero);
    }
    if wrapping && !deferred && !extra {
        // The last node made at the innermost open node's level.
        (e.set)(f, Mem::LastMade, n1);
    }
    (e.set)(f, Mem::Nodes, n1);
}

/// A node's word in a direct format 16 run (`Table::direct`): its number by
/// its key `kf` (its kind, its field above it) and its supertypes' bits, then `w_open` at the token's start (and open
/// on the stack of open nodes) or, a leaf's (`end`), `w_leaf` with the token's bytes; then `w_commit`. No row. A key
/// with no number is the ops' failure (`fail::NUMBER`); an op's failure is the writer's, which the host reads.
fn node16_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kf: Value, classes: u32, end: Option<EndAt>) {
    let s = t.fns.sixteen.as_ref().expect("a format 16 run's");
    let zero = f.ins().iconst(types::I64, 0);
    (e.set)(f, Mem::Pending, zero);
    let (nodes, start) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Start));
    let kind = f.ins().band_imm(kf, 0x3ff);
    let field = f.ins().ushr_imm(kf, 16);
    let field = f.ins().band_imm(field, 0x3f);
    let field = f.ins().ishl_imm(field, i64::from(rerail_tape_format::KIND_BITS));
    let slot = f.ins().bor(kind, field);
    let slot = f.ins().bor_imm(slot, i64::from(classes));
    let (desc, numbers) = ((e.ctx)(f, WRITER), (e.ctx)(f, NUMBERS));
    let number = crate::native_cpu_tape::number_of(f, numbers, slot);
    let none = f.ins().icmp_imm(IntCC::Equal, number, i64::from(u32::MAX));
    let (missing, known, done) = (f.create_block(), f.create_block(), f.create_block());
    f.ins().brif(none, missing, &[], known, &[]);
    f.switch_to_block(missing);
    f.set_cold_block(missing);
    crate::native_cpu_tape::fail_op(f, &s.ops, desc, rerail_clif::writer::fail::NUMBER, nodes);
    f.ins().jump(done, &[]);
    f.switch_to_block(known);
    match end {
        Some(end_at) => {
            let end = token_end_at(f, e, end_at);
            crate::native_cpu_tape::call_op(f, &s.ops.leaf, desc, &[number, start, end]);
        }
        None => {
            crate::native_cpu_tape::call_op(f, &s.ops.open, desc, &[number, start]);
        }
    }
    crate::native_cpu_tape::call_op(f, &s.ops.commit, desc, &[]);
    f.ins().jump(done, &[]);
    f.switch_to_block(done);
    if end.is_none() {
        let open = (e.get)(f, Mem::Open);
        let open = f.ins().iadd_imm(open, 1);
        (e.set)(f, Mem::Open, open);
    }
    let n1 = f.ins().iadd_imm(nodes, 1);
    (e.set)(f, Mem::Nodes, n1);
}

/// `CLOSE` in a direct format 16 run (`Table::direct`): the innermost open node's end (`this`: the token's; else the
/// last taken's: no extra is pending in such a run), `w_close` there; off the stack of open nodes. No row.
fn close16_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, this: bool, end_at: EndAt) {
    let s = t.fns.sixteen.as_ref().expect("a format 16 run's");
    let open = (e.get)(f, Mem::Open);
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    (e.guard)(f, none);
    let open = f.ins().iadd_imm(open, -1);
    (e.set)(f, Mem::Open, open);
    let end = if this { token_end_at(f, e, end_at) } else { (e.get)(f, Mem::LastEnd) };
    let desc = (e.ctx)(f, WRITER);
    crate::native_cpu_tape::call_op(f, &s.ops.close, desc, &[end]);
}

/// `OPEN`: its row (its key, its field above its kind or none; its start), its offset; open.
fn open_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value, wrapping: bool) {
    let zero = f.ins().iconst(types::I64, 0);
    (e.set)(f, Mem::Pending, zero);
    let (nodes, start, open) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Start), (e.get)(f, Mem::Open));
    let rows = (e.ctx)(f, ROWS);
    let row = NodeCol::row(f, rows, nodes);
    f.ins().istore32(MemFlags::trusted(), kind, row, AT_KIND);
    NodeCol::of(rows, AT_START).put_row(f, row, start);
    let n1 = f.ins().iadd_imm(nodes, 1);
    t.push_off(f, e, nodes, start, n1);
    let open_c = (e.ctx)(f, OPEN);
    slot_put(f, open_c, open, nodes);
    if t.inner {
        let head = (e.ctx)(f, HEAD);
        slot_put(f, head, nodes, zero);
    }
    if wrapping {
        // No node made at its level yet.
        (e.set)(f, Mem::LastMade, zero);
    }
    let open = f.ins().iadd_imm(open, 1);
    (e.set)(f, Mem::Open, open);
    (e.set)(f, Mem::Nodes, n1);
}

/// `CLOSE`: the innermost open node's end (`this`: the token's; else the last taken's, or
/// the one before the pending extras), size and tail; the pending extras its parent's (they
/// follow its subtree); with wraps, it is the last node made at its parent's level. A node
/// closed at the top level: the nodes before it are final, their codes committed.
fn close_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, this: bool, end_at: EndAt, wrapping: bool) {
    let open = (e.get)(f, Mem::Open);
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    (e.guard)(f, none);
    let open = f.ins().iadd_imm(open, -1);
    (e.set)(f, Mem::Open, open);
    let open_c = (e.ctx)(f, OPEN);
    let at = slot_get(f, open_c, open);
    let (nodes, pending) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Pending));
    let end = if this {
        token_end_at(f, e, end_at)
    } else {
        let (keep, last_end) = ((e.get)(f, Mem::Keep), (e.get)(f, Mem::LastEnd));
        f.ins().select(pending, keep, last_end)
    };
    let rows = (e.ctx)(f, ROWS);
    let at_row = NodeCol::row(f, rows, at);
    NodeCol::of(rows, AT_END).put_row(f, at_row, end);
    if wrapping {
        let at1 = f.ins().iadd_imm(at, 1);
        (e.set)(f, Mem::LastMade, at1);
    }
    let kept = f.ins().isub(nodes, pending);
    let size = f.ins().isub(kept, at);
    let last = f.ins().iadd_imm(kept, -1);
    t.close_cells(f, e, Closed { at, size, last }, end, None);
    let top = f.ins().icmp_imm(IntCC::Equal, open, 1);
    let (commit, done) = (f.create_block(), f.create_block());
    f.ins().brif(top, commit, &[], done, &[]);
    f.switch_to_block(commit);
    t.commit(f, e, at);
    f.ins().jump(done, &[]);
    f.switch_to_block(done);
}

/// `WRAP`: a node before the last node made at the innermost open node's level, its field that
/// one's; open around it. That node and the rest of the table (the pending extras) move up a
/// cell: one row and nothing else where it is a leaf no supertype names, else `shift`, then
/// the blocks from it again.
fn wrap_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value) {
    let flags = MemFlags::trusted();
    let zero = f.ins().iconst(types::I64, 0);
    let open = (e.get)(f, Mem::Open);
    let last = (e.get)(f, Mem::LastMade);
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
    // A branch each, not the tests joined.
    (e.guard)(f, none);
    (e.guard)(f, no_last);
    let r = f.ins().iadd_imm(last, -1);
    t.uncode(f, e, r);
    let (nodes, supers) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Supers));
    let n1 = f.ins().iadd_imm(nodes, 1);
    let rows = (e.ctx)(f, ROWS);
    let head = if t.inner { (e.ctx)(f, HEAD) } else { zero };
    let snode = (e.ctx)(f, SUPER_NODE);
    let (size_c, tail_c) = ((e.ctx)(f, SIZE_C), (e.ctx)(f, TAIL_C));
    // One row: the leaf at the end, no supertype on it.
    let tail = f.ins().isub(nodes, r);
    let one = f.ins().icmp_imm(IntCC::Equal, tail, 1);
    let any = f.ins().icmp_imm(IntCC::NotEqual, supers, 0);
    let s1 = f.ins().iadd_imm(supers, -1);
    let s1 = f.ins().select(any, s1, zero);
    let ls = slot_get(f, snode, s1);
    let named = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, ls, r);
    let named = f.ins().band(any, named);
    let (swap, long, placed) = (f.create_block(), f.create_block(), f.create_block());
    let plain = f.ins().band_not(one, named);
    f.ins().brif(plain, swap, &[], long, &[]);
    f.switch_to_block(swap);
    let (src, dst) = (NodeCol::row(f, rows, r), NodeCol::row(f, rows, nodes));
    let w = row_words(f, src);
    put_row_words(f, dst, w);
    let size = crate::native_cpu_tape::get(f, size_c, r, t.w.size);
    crate::native_cpu_tape::put(f, size_c, nodes, size, t.w.size);
    // A format 16 run's pass reads only the sizes: no tails.
    if t.fns.sixteen.is_none() {
        let tl = crate::native_cpu_tape::get(f, tail_c, r, t.w.tail);
        crate::native_cpu_tape::put(f, tail_c, nodes, tl, t.w.tail);
    }
    let st = f.ins().uload32(flags, dst, AT_START);
    t.push_off(f, e, nodes, st, n1);
    if t.inner {
        let h = slot_get(f, head, r);
        slot_put(f, head, nodes, h);
    }
    f.ins().jump(placed, &[]);
    f.switch_to_block(long);
    f.set_cold_block(long);
    let (id, sig, _) = t.fns.shift.as_ref().expect("a program that wraps has its shift");
    call_own(f, *id, sig, &[rows, head, snode, r, nodes, supers, size_c, tail_c]);
    let b = f.ins().ushr_imm(r, rerail_tape_format::CHECKPOINT_STRIDE.trailing_zeros() as i64);
    t.blocks(f, e, b, n1);
    f.ins().jump(placed, &[]);
    f.switch_to_block(placed);
    // The wrap at r (its start the wrapped node's: r's offset holds), the wrapped node after it.
    let (to, from) = (NodeCol::row(f, rows, r), NodeCol::row(f, rows, last));
    let fld = f.ins().uload16(types::I64, flags, from, AT_FIELD);
    let kf = f.ins().ishl_imm(fld, 16);
    let kf = f.ins().bor(kf, kind);
    f.ins().istore32(flags, kf, to, AT_KIND);
    f.ins().istore16(flags, zero, from, AT_FIELD);
    if t.inner {
        let mark = f.ins().iconst(types::I64, WRAPPED);
        slot_put(f, head, r, mark);
    }
    let open_c = (e.ctx)(f, OPEN);
    slot_put(f, open_c, open, r);
    let made = f.ins().iadd_imm(r, 2);
    (e.set)(f, Mem::LastMade, made);
    (e.set)(f, Mem::Moved, last);
    (e.set)(f, Mem::Pending, zero);
    let open1 = f.ins().iadd_imm(open, 1);
    (e.set)(f, Mem::Open, open1);
    (e.set)(f, Mem::Nodes, n1);
}

/// A dropped node's rows taken out from row `from` on (`drop`), then the tails of the nodes
/// left there and the blocks from there again.
fn drop_from(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, from: Value) {
    let zero = f.ins().iconst(types::I64, 0);
    t.uncode(f, e, from);
    let rows = (e.ctx)(f, ROWS);
    let (before, to, stack) = ((e.ctx)(f, FRAME_BASE), (e.ctx)(f, LINK), (e.ctx)(f, PLACE));
    let head = if t.inner { (e.ctx)(f, HEAD) } else { zero };
    let (snode, skind) = ((e.ctx)(f, SUPER_NODE), (e.ctx)(f, SUPER_KIND));
    let (size_c, tail_c) = ((e.ctx)(f, SIZE_C), (e.ctx)(f, TAIL_C));
    let (nodes, supers, moved) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Supers), (e.get)(f, Mem::Moved));
    // The innermost open node: a dropped row's parent where no row of the range holds it.
    let open = (e.get)(f, Mem::Open);
    let o1 = f.ins().iadd_imm(open, -1);
    let open_c = (e.ctx)(f, OPEN);
    let outer = slot_get(f, open_c, o1);
    let (id, sig, _) = t.fns.drop.as_ref().expect("a program that drops has its drop");
    let r = call_own(f, *id, sig, &[rows, before, to, head, snode, skind, from, nodes, supers, moved, size_c, tail_c, stack, outer]);
    (e.set)(f, Mem::Nodes, r[0]);
    (e.set)(f, Mem::Supers, r[1]);
    (e.set)(f, Mem::Moved, r[2]);
    let wide = (e.get)(f, Mem::Wide);
    let wide = f.ins().bor(wide, r[3]);
    (e.set)(f, Mem::Wide, wide);
    let b = f.ins().ushr_imm(from, rerail_tape_format::CHECKPOINT_STRIDE.trailing_zeros() as i64);
    t.blocks(f, e, b, r[0]);
}

/// `RETAG m=2`, `NODE_FIELD m=2`: the node made last at the innermost open node's level
/// (`relabel`), its frame's last one more than its record (0: none), its kind or field `x`.
fn made_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, x: Value, retag: bool, dead: bool, unset: bool) {
    let open = (e.get)(f, Mem::Open);
    let last = (e.get)(f, Mem::LastMade);
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
    // A branch each, not the tests joined.
    (e.guard)(f, none);
    (e.guard)(f, no_last);
    let r = f.ins().iadd_imm(last, -1);
    let rows = (e.ctx)(f, ROWS);
    if retag {
        t.retag(f, e, rows, r, x);
    } else {
        let col = NodeCol::of(rows, AT_FIELD);
        // A call's field (`On::MadeUnset`): the node's own, where it has one.
        let x = if unset {
            let had = col.get(f, r);
            f.ins().select(had, had, x)
        } else {
            x
        };
        col.put(f, r, x);
    }
    if dead {
        // `drop`: the node goes (a token the grammar implies), taken out now; none is made last
        // at this level until the next.
        let zero = f.ins().iconst(types::I64, 0);
        (e.set)(f, Mem::LastMade, zero);
        drop_from(f, e, t, r);
    }
}

/// `RETAG`: the innermost open node's kind.
fn here_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value) {
    let open = (e.get)(f, Mem::Open);
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    (e.guard)(f, none);
    let top = f.ins().iadd_imm(open, -1);
    let open_c = (e.ctx)(f, OPEN);
    let at = slot_get(f, open_c, top);
    let rows = (e.ctx)(f, ROWS);
    t.retag(f, e, rows, at, kind);
}

/// `MERGE m=1`: the innermost open node holds two leaves and nothing else, no extra pending:
/// the first becomes one leaf through the second's end, closed again.
fn fold_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value, wrapping: bool) {
    let (nodes, pending, open) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Pending), (e.get)(f, Mem::Open));
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    (e.guard)(f, none);
    let top = f.ins().iadd_imm(open, -1);
    let open_c = (e.ctx)(f, OPEN);
    let o = slot_get(f, open_c, top);
    let want = f.ins().iadd_imm(o, 3);
    let count = f.ins().icmp(IntCC::NotEqual, nodes, want);
    let waiting = f.ins().icmp_imm(IntCC::NotEqual, pending, 0);
    // A branch each, not the tests joined.
    (e.guard)(f, count);
    (e.guard)(f, waiting);
    let (a, b) = (f.ins().iadd_imm(o, 1), f.ins().iadd_imm(o, 2));
    let rows = (e.ctx)(f, ROWS);
    let size_c = (e.ctx)(f, SIZE_C);
    let end_c = NodeCol::of(rows, AT_END);
    let (ra, rb) = (NodeCol::row(f, rows, a), NodeCol::row(f, rows, b));
    let (sa, sb) = (crate::native_cpu_tape::get(f, size_c, a, t.w.size), crate::native_cpu_tape::get(f, size_c, b, t.w.size));
    let na = f.ins().icmp_imm(IntCC::NotEqual, sa, 1);
    let nb = f.ins().icmp_imm(IntCC::NotEqual, sb, 1);
    (e.guard)(f, na);
    (e.guard)(f, nb);
    let end = end_c.get_row(f, rb);
    // The first may be committed (the root's two leaves, each closed at the top level): the retag takes its codes
    // back.
    t.retag(f, e, rows, a, kind);
    end_c.put_row(f, ra, end);
    let one = f.ins().iconst(types::I64, 1);
    t.close_cells(f, e, Closed { at: a, size: one, last: a }, end, None);
    (e.set)(f, Mem::Nodes, b);
    t.cut_to(f, e, b);
    if wrapping {
        // The first is the last node made at this level now.
        (e.set)(f, Mem::LastMade, b);
    }
}

/// `MERGE`: the grammar's leaf before the pending extras, a leaf inside the innermost open
/// node; the extras are its text: dropped. The leaf closed again at its new end.
fn merge_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value, end_at: EndAt) {
    let zero = f.ins().iconst(types::I64, 0);
    let (nodes, pending, open) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Pending), (e.get)(f, Mem::Open));
    let kept = f.ins().isub(nodes, pending);
    let none = f.ins().icmp_imm(IntCC::Equal, kept, 0);
    (e.guard)(f, none);
    let leaf = f.ins().iadd_imm(kept, -1);
    let rows = (e.ctx)(f, ROWS);
    let size_c = (e.ctx)(f, SIZE_C);
    let end_c = NodeCol::of(rows, AT_END);
    let leaf_row = NodeCol::row(f, rows, leaf);
    let size = crate::native_cpu_tape::get(f, size_c, leaf, t.w.size);
    let not_leaf = f.ins().icmp_imm(IntCC::NotEqual, size, 1);
    (e.guard)(f, not_leaf);
    let any = f.ins().icmp_imm(IntCC::NotEqual, open, 0);
    let top = f.ins().iadd_imm(open, -1);
    let top = f.ins().select(any, top, zero);
    let open_c = (e.ctx)(f, OPEN);
    let o = slot_get(f, open_c, top);
    let inside = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, o, leaf);
    let bad = f.ins().band(any, inside);
    (e.guard)(f, bad);
    let end = token_end_at(f, e, end_at);
    t.retag(f, e, rows, leaf, kind);
    end_c.put_row(f, leaf_row, end);
    let one = f.ins().iconst(types::I64, 1);
    t.close_cells(f, e, Closed { at: leaf, size: one, last: leaf }, end, None);
    (e.set)(f, Mem::Nodes, kept);
    (e.set)(f, Mem::Pending, zero);
    // The extras taken back: where there were any, the last block's base again.
    let (cut, done) = (f.create_block(), f.create_block());
    f.ins().brif(pending, cut, &[], done, &[]);
    f.switch_to_block(cut);
    t.cut_to(f, e, kept);
    f.ins().jump(done, &[]);
    f.switch_to_block(done);
}

/// What `TOKEN` reads besides `ctx`: the room each token's path may need, and the marks.
#[derive(Clone, Copy)]
struct TokenShape {
    bound: Bounds,
    /// No room check (`Bounds::unchecked`).
    unchecked: bool,
    in_code: usize,
    rest_bits: i64,
    end_bits: i64,
    /// The stream's end as a token's code (`end_kind`), where it has one.
    end_code: Option<i64>,
    /// The calls open counted by the machine's stack (`calls_on_stack`): no argument for them,
    /// and no room for returns (none is written).
    on_stack: bool,
    /// The extra this `TOKEN` takes (`TOKEN_EXTRAS`): its code's mark bit and its leaf's kind.
    extra: Option<(u8, u16)>,
    /// The extras' later kinds (`stream::extra_kind`), each with its bit.
    more: MoreExtras,
    /// The program has wraps (`framed`): a leaf's frame slot too.
    wrapping: bool,
    /// A mark loses an extra (`SETTLE m=2`): the token taken makes it again (`lost_leaf`).
    lost: bool,
    /// Statements recover (`Words::recover`, decision 0650): a refusing token mark holds the token
    /// (`MARK_HELD`), refused where the next token is taken or the next statement starts.
    recovers: bool,
}

/// The extras' kinds after the first (`Words::extras`), each with its mark bit: at most
/// `MORE_EXTRAS`.
#[derive(Clone, Copy, Default)]
struct MoreExtras {
    kinds: [(u8, u16); MORE_EXTRAS],
    count: usize,
}

const MORE_EXTRAS: usize = 3;

impl MoreExtras {
    /// The extras taken (`TOKEN_EXTRAS`): the first, and the later kinds; `None` for none.
    fn of(extras: &[(u8, u16)], taken: bool) -> Result<(Option<(u8, u16)>, Self), Error> {
        let Some((&first, rest)) = extras.split_first().filter(|_| taken) else { return Ok((None, Self::default())) };
        if rest.len() > MORE_EXTRAS {
            return Err(Error::UnsupportedBackend);
        }
        let mut more = Self::default();
        more.kinds[..rest.len()].copy_from_slice(rest);
        more.count = rest.len();
        Ok((Some(first), more))
    }

    /// The leaf's kind of the extra whose record's code is `c`: `kind`, or the last later kind
    /// whose bit `c` has (`stream::extra_kind`).
    fn kind(&self, f: &mut FunctionBuilder<'_>, c: Value, kind: u16) -> Value {
        let mut k = f.ins().iconst(types::I64, i64::from(kind));
        for &(bit, later) in &self.kinds[..self.count] {
            let b = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT) + i64::from(bit));
            let b = f.ins().band_imm(b, 1);
            let l = f.ins().iconst(types::I64, i64::from(later));
            k = f.ins().select(b, l, k);
        }
        k
    }
}

/// Output `o`'s word `w` for tier 2 (decision 0323): from the bytes where tier 1 makes it from
/// them (`native_cpu_bytes`, which the CPU's tier 1 does not store), else its row's word.
fn output_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, bw: &crate::native_cpu_bytes::Bytewise<'_>, o: u16, w: Value, heads: &mut Vec<(Block, Step)>) -> Value {
    let load = |f: &mut FunctionBuilder<'_>, at: i32| (e.ctx)(f, at);
    if bw.can(o) {
        let consts = crate::native_cpu_block::Consts::make(f);
        let file = crate::native_cpu_bytes::File { bytes: load(f, BYTES), len: load(f, LEN), nibble: consts.nibble, weights: consts.weights, even: consts.even };
        let p = f.ins().ishl_imm(w, 6);
        return bw.word(f, &file, o, p, heads);
    }
    let (outputs_p, output_stride, lasts_stride) = (load(f, OUTPUTS), load(f, OUTPUT_STRIDE), load(f, LASTS_STRIDE));
    let off = f.ins().imul_imm(output_stride, i64::from(o));
    let base = f.ins().iadd(outputs_p, off);
    mask_word(f, base, lasts_stride, w)
}

/// A part of a split region (`SplitParts`) as the token: its bytes `s..=l` and its code
/// (`plain_code`), then on to `fetched`.
/// `firsts`: the bytes its first byte can be, where the caller knows them (a sequential region's
/// closer or content): the kinds read only where one of them has them, and a kind that is
/// every one's, not read, a constant.
fn split_token(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, sp: &SplitParts, (s, l): (Value, Value), fetched: Block, read: Option<&[u8]>, firsts: Option<[u64; 4]>) {
    let bytes = (e.ctx)(f, BYTES);
    // The kinds its first byte can have (the record tables' first 256 bytes, a byte's kind).
    let kinds_of = firsts.map(|set| {
        let mut ks: Vec<u8> = (0..256usize).filter(|&b| set[b / 64] >> (b % 64) & 1 == 1).map(|b| (sp.tables.words[b / 8] >> (8 * (b % 8))) as u8).collect();
        ks.sort_unstable();
        ks.dedup();
        ks
    });
    let read: Option<Vec<u8>> = read.map(|r| r.iter().copied().filter(|x| kinds_of.as_ref().is_none_or(|ks| ks.contains(x))).collect());
    let read = read.as_deref();
    let whole = |f: &mut FunctionBuilder<'_>| {
        let k = crate::native_cpu_block::tables_symbols(f, &sp.tables);
        let n = f.ins().isub(l, s);
        let len = f.ins().iadd_imm(n, 1);
        crate::native_cpu_block::plain_code(f, &sp.tables, k, bytes, s, len)
    };
    let code = match read {
        None => whole(f),
        // The first byte's kind, one load; its word only where a word after reads its kind's
        // (`words_read`).
        Some(kinds) => {
            let kind = match kinds_of.as_deref() {
                Some(&[k]) => f.ins().iconst(types::I64, i64::from(k)),
                _ => {
                    let at = f.ins().iadd(bytes, s);
                    let b = f.ins().uload8(types::I64, MemFlags::trusted(), at, 0);
                    let kind_table = crate::native_cpu_block::tables_symbols(f, &sp.tables).0;
                    let ka = f.ins().iadd(kind_table, b);
                    f.ins().uload8(types::I64, MemFlags::trusted().with_readonly(), ka, 0)
                }
            };
            if kinds.is_empty() {
                kind
            } else {
                let (worded, done) = (f.create_block(), f.create_block());
                f.append_block_param(done, types::I64);
                let mut read_here = f.ins().iconst(types::I64, 0);
                for &x in kinds {
                    let same = f.ins().icmp_imm(IntCC::Equal, kind, i64::from(x));
                    let same = f.ins().uextend(types::I64, same);
                    read_here = f.ins().bor(read_here, same);
                }
                f.ins().brif(read_here, worded, &[], done, &[kind.into()]);
                f.switch_to_block(worded);
                let c = whole(f);
                f.ins().jump(done, &[c.into()]);
                f.switch_to_block(done);
                f.block_params(done)[0]
            }
        }
    };
    let l1 = f.ins().iadd_imm(l, 1);
    (e.set)(f, Mem::Start, s);
    (e.set)(f, Mem::Limit, l1);
    (e.set)(f, Mem::Taken, code);
    f.ins().jump(fetched, &[]);
}

/// A split region's next part, where one is pending (`Mem::PartAt`), as `records::read_parts`
/// gives it: at the closer, the closer; else the nearest part's bit before the closer, a word at
/// a time; a part there, its bytes; else the bytes before it (or the closer), the content. Then
/// on to `fetched`; none pending, on in the current block.
#[allow(clippy::too_many_arguments)]
fn split_next(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, sp: &SplitParts, bw: &crate::native_cpu_bytes::Bytewise<'_>, fetched: Block, onward: Option<&Onward>, read_word: Option<&[u8]>, heads: &mut Vec<(Block, Step)>) {
    let (part, record) = (f.create_block(), f.create_block());
    let at = (e.get)(f, Mem::PartAt);
    f.ins().brif(at, part, &[], record, &[]);
    f.switch_to_block(part);
    if let Some(seq) = &sp.seq {
        split_next_seq(f, e, sp, seq, bw, at, fetched, onward, read_word, heads);
        f.switch_to_block(record);
        return;
    }
    // Its reader's walk (`PartRd`), one where there is one.
    let rdv = (e.get)(f, Mem::PartRd);
    let n = sp.readers.len();
    for (k, rd) in sp.readers.iter().enumerate() {
        let this = f.create_block();
        let other = (k + 1 < n).then(|| f.create_block());
        match other {
            Some(o) => {
                let is = f.ins().icmp_imm(IntCC::Equal, rdv, k as i64);
                f.ins().brif(is, this, &[], o, &[]);
            }
            None => {
                f.ins().jump(this, &[]);
            }
        }
        f.switch_to_block(this);
        split_next_reader(f, e, sp, rd, bw, at, fetched, heads);
        if let Some(o) = other {
            f.switch_to_block(o);
        }
    }
    f.switch_to_block(record);
    split_resume(f, e, sp, bw, fetched, heads);
}

/// With a string's record open over its template's code (`Mem::Strings`, decision 0600): where
/// the next opener of a reader that resumes (the template's `}`) comes before the next record,
/// that reader entered there, with no record, as the lean parser goes back to its string after
/// the template's code. Else on in the current block, to the next record.
fn split_resume(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, sp: &SplitParts, bw: &crate::native_cpu_bytes::Bytewise<'_>, fetched: Block, heads: &mut Vec<(Block, Step)>) {
    let Some((k, rd)) = sp.readers.iter().enumerate().find(|(_, rd)| rd.resumes) else { return };
    let (open, enter, on) = (f.create_block(), f.create_block(), f.create_block());
    let n = (e.get)(f, Mem::Strings);
    f.ins().brif(n, open, &[], on, &[]);
    f.switch_to_block(open);
    f.set_cold_block(open);
    let from = (e.get)(f, Mem::Limit);
    let (p, _) = first_bit_from(f, e, bw, rd.opener, from, heads);
    // The next record's start (none: past the file).
    let next = (e.get)(f, Mem::Next);
    let count = (e.ctx)(f, NSTARTS);
    let stride = (e.ctx)(f, RECORD_STRIDE);
    let starts = (e.ctx)(f, STARTS);
    let more = f.ins().icmp(IntCC::UnsignedLessThan, next, count);
    let (read, cmp) = (f.create_block(), f.create_block());
    f.append_block_param(cmp, types::I64);
    let len = (e.ctx)(f, LEN);
    f.ins().brif(more, read, &[], cmp, &[len.into()]);
    f.switch_to_block(read);
    let off = f.ins().imul(next, stride);
    let a = f.ins().iadd(starts, off);
    let s = f.ins().uload32(MemFlags::trusted(), a, 0);
    f.ins().jump(cmp, &[s.into()]);
    f.switch_to_block(cmp);
    let s = f.block_params(cmp)[0];
    let first = f.ins().icmp(IntCC::UnsignedLessThan, p, s);
    f.ins().brif(first, enter, &[], on, &[]);
    f.switch_to_block(enter);
    let p1 = f.ins().iadd_imm(p, 1);
    (e.set)(f, Mem::PartAt, p1);
    let kv = f.ins().iconst(types::I64, k as i64);
    (e.set)(f, Mem::PartRd, kv);
    let (c, _) = first_bit_from(f, e, bw, rd.closer, p1, heads);
    (e.set)(f, Mem::PartClose, c);
    let pe = closer_last(f, e, bw, rd, c, heads);
    (e.set)(f, Mem::PartEnd, pe);
    let code = match rd.code {
        Some(x) => f.ins().iconst(types::I64, i64::from(x)),
        None => {
            let bytes = (e.ctx)(f, BYTES);
            let kc = crate::native_cpu_block::tables_consts(f, &sp.tables);
            let one = f.ins().iconst(types::I64, 1);
            crate::native_cpu_block::plain_code(f, &sp.tables, kc, bytes, p, one)
        }
    };
    (e.set)(f, Mem::Start, p);
    (e.set)(f, Mem::Limit, p1);
    (e.set)(f, Mem::Taken, code);
    f.ins().jump(fetched, &[]);
    f.switch_to_block(on);
}

/// A split reader's closer's last from its first byte `c`: its last's first bit from `c`
/// (`ReadArm::last`), else `c` and its offset, as `records::read_parts` ends it.
fn closer_last(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, bw: &crate::native_cpu_bytes::Bytewise<'_>, rd: &SplitReader, c: Value, heads: &mut Vec<(Block, Step)>) -> Value {
    match rd.last {
        Some(o) => first_bit_from(f, e, bw, o, c, heads).0,
        None => f.ins().iadd_imm(c, i64::from(rd.off)),
    }
}

/// A split region's next piece by its reader `rd`, as `records::read_parts` and the lean
/// lexer's string arm read it: at the closer (`PartClose`), the closer to the record's last
/// (`PartEnd`); else the nearest part's bit before the closer, a word at a time; a part there,
/// its bytes or to its last; else the bytes before it (or the closer), the content. Each with its
/// own kind. Then on to `fetched`.
#[allow(clippy::too_many_arguments)]
fn split_next_reader(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, sp: &SplitParts, rd: &SplitReader, bw: &crate::native_cpu_bytes::Bytewise<'_>, at: Value, fetched: Block, heads: &mut Vec<(Block, Step)>) {
    let pe = (e.get)(f, Mem::PartEnd);
    let c0 = (e.get)(f, Mem::PartClose);
    let (closer, look) = (f.create_block(), f.create_block());
    let here = f.ins().icmp(IntCC::Equal, at, c0);
    f.ins().brif(here, closer, &[], look, &[]);
    // The closer: the region's last token, its own kind by its byte. At a template's `$`
    // (decision 0600), the string's record stays open over the code; a read that resumes one
    // and closes at its quote ends it.
    f.switch_to_block(closer);
    let zero = f.ins().iconst(types::I64, 0);
    (e.set)(f, Mem::PartAt, zero);
    if let Some(t) = rd.template {
        let q = f.ins().ushr_imm(c0, 6);
        let x = output_word(f, e, bw, t, q, heads);
        let sh = f.ins().band_imm(c0, 63);
        let x = f.ins().ushr(x, sh);
        let tb = f.ins().band_imm(x, 1);
        let n = (e.get)(f, Mem::Strings);
        let n = f.ins().iadd(n, tb);
        let n = if rd.resumes { f.ins().iadd_imm(n, -1) } else { n };
        (e.set)(f, Mem::Strings, n);
    }
    split_piece(f, e, sp, c0, pe, Piece::Closer(&rd.closers), fetched);
    f.switch_to_block(look);
    if rd.parts.is_empty() {
        (e.set)(f, Mem::PartAt, c0);
        let l = f.ins().iadd_imm(c0, -1);
        split_piece(f, e, sp, at, l, Piece::Kind(rd.content), fetched);
        return;
    }
    // The nearest part's bit from `at`, below the closer's byte: a word at a time.
    let (scan, found) = (f.create_block(), f.create_block());
    f.append_block_param(scan, types::I64);
    f.append_block_param(scan, types::I64);
    f.append_block_param(found, types::I64);
    let q = f.ins().ushr_imm(at, 6);
    let sh = f.ins().band_imm(at, 63);
    let ones = f.ins().iconst(types::I64, -1);
    let m = f.ins().ishl(ones, sh);
    f.ins().jump(scan, &[q.into(), m.into()]);
    f.switch_to_block(scan);
    heads.push((scan, Step::Word));
    let (w, m) = (f.block_params(scan)[0], f.block_params(scan)[1]);
    let words: Vec<Value> = rd.parts.iter().map(|&(o, ..)| {
        let x = output_word(f, e, bw, o, w, heads);
        f.ins().band(x, m)
    }).collect();
    let any = words.iter().skip(1).fold(words[0], |a, &x| f.ins().bor(a, x));
    let w64 = f.ins().ishl_imm(w, 6);
    let tz = f.ins().ctz(any);
    let near = f.ins().iadd(w64, tz);
    // Past the closer's word with none: the closer's byte.
    let qc = f.ins().ushr_imm(c0, 6);
    let last = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, w, qc);
    let (hit, more) = (f.create_block(), f.create_block());
    f.ins().brif(any, hit, &[], more, &[]);
    f.switch_to_block(more);
    let w1 = f.ins().iadd_imm(w, 1);
    let ones = f.ins().iconst(types::I64, -1);
    let (next, none) = (f.create_block(), f.create_block());
    f.ins().brif(last, none, &[], next, &[]);
    f.switch_to_block(next);
    f.ins().jump(scan, &[w1.into(), ones.into()]);
    f.switch_to_block(none);
    f.ins().jump(found, &[c0.into()]);
    f.switch_to_block(hit);
    let near = f.ins().umin(near, c0);
    f.ins().jump(found, &[near.into()]);
    f.switch_to_block(found);
    let near = f.block_params(found)[0];
    let (content, parted) = (f.create_block(), f.create_block());
    let at_part = f.ins().icmp(IntCC::Equal, near, at);
    f.ins().brif(at_part, parted, &[], content, &[]);
    // The bytes before it: a content token, the next part at it.
    f.switch_to_block(content);
    (e.set)(f, Mem::PartAt, near);
    let l = f.ins().iadd_imm(near, -1);
    split_piece(f, e, sp, at, l, Piece::Kind(rd.content), fetched);
    // A part at `at`: the shortest whose bit is there, its bytes or to its last.
    f.switch_to_block(parted);
    let low = {
        let neg = f.ins().ineg(any);
        f.ins().band(any, neg)
    };
    for (j, (&(_, n, to, kind), &x)) in rd.parts.iter().zip(&words).enumerate() {
        let this = f.create_block();
        let other = (j + 1 < rd.parts.len()).then(|| f.create_block());
        match other {
            Some(o) => {
                let is = f.ins().band(x, low);
                f.ins().brif(is, this, &[], o, &[]);
            }
            None => {
                f.ins().jump(this, &[]);
            }
        }
        f.switch_to_block(this);
        let l = match to {
            Some(o) => first_bit_from(f, e, bw, o, at, heads).0,
            None => f.ins().iadd_imm(at, i64::from(n) - 1),
        };
        let after = f.ins().iadd_imm(l, 1);
        (e.set)(f, Mem::PartAt, after);
        split_piece(f, e, sp, at, l, Piece::Kind(kind), fetched);
        if let Some(o) = other {
            f.switch_to_block(o);
        }
    }
}

/// A split region's piece's kind: its own (none: its first byte's), or a closer's by its byte.
enum Piece<'a> {
    Kind(Option<u8>),
    Closer(&'a [(u8, u8)]),
}

/// A split region's piece `s..=l` as the token: its code `plain_code`'s, its kind its own
/// (`Piece`). Then on to `fetched`.
fn split_piece(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, sp: &SplitParts, s: Value, l: Value, kind: Piece<'_>, fetched: Block) {
    let bytes = (e.ctx)(f, BYTES);
    let k = crate::native_cpu_block::tables_consts(f, &sp.tables);
    let n = f.ins().isub(l, s);
    let len = f.ins().iadd_imm(n, 1);
    let mut code = crate::native_cpu_block::plain_code(f, &sp.tables, k, bytes, s, len);
    let own = |f: &mut FunctionBuilder<'_>, code: Value, k: u8| {
        let rest = f.ins().band_imm(code, !0xff);
        f.ins().bor_imm(rest, i64::from(k))
    };
    match kind {
        Piece::Kind(None) => {}
        Piece::Kind(Some(k)) => code = own(f, code, k),
        Piece::Closer(closers) if !closers.is_empty() => {
            let a = f.ins().iadd(bytes, s);
            let b = f.ins().uload8(types::I64, MemFlags::trusted(), a, 0);
            for &(x, k) in closers {
                let is = f.ins().icmp_imm(IntCC::Equal, b, i64::from(x));
                let tagged = own(f, code, k);
                code = f.ins().select(is, tagged, code);
            }
        }
        Piece::Closer(_) => {}
    }
    let l1 = f.ins().iadd_imm(l, 1);
    (e.set)(f, Mem::Start, s);
    (e.set)(f, Mem::Limit, l1);
    (e.set)(f, Mem::Taken, code);
    f.ins().jump(fetched, &[]);
}

/// `split_next` where the region reads as simdjson's `parse_string` (`Seq`): from `at`, a clean
/// place, the first byte of the closer's class or the part's, 16 bytes a vector (as
/// `backslash_and_quote::copy_and_find`): the closer there, or a part of two bytes, or the
/// bytes before it, the content. No closer before the file's end refuses it.
#[allow(clippy::too_many_arguments)]
fn split_next_seq(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, sp: &SplitParts, seq: &Seq, bw: &crate::native_cpu_bytes::Bytewise<'_>, at: Value, fetched: Block, onward: Option<&Onward>, read_word: Option<&[u8]>, heads: &mut Vec<(Block, Step)>) {
    let to = |pick: fn(&Onward) -> Option<Block>| onward.and_then(pick).unwrap_or(fetched);
    let consts = crate::native_cpu_block::Consts::make(f);
    let file = crate::native_cpu_bytes::File { bytes: (e.ctx)(f, BYTES), len: (e.ctx)(f, LEN), nibble: consts.nibble, weights: consts.weights, even: consts.even };
    let stride = i64::from(bw.short_stride_x(&[seq.x, seq.e]));
    let (scan, hit) = (f.create_block(), f.create_block());
    f.append_block_param(scan, types::I64);
    f.ins().jump(scan, &[at.into()]);
    f.switch_to_block(scan);
    heads.push((scan, Step::Word));
    let p = f.block_params(scan)[0];
    let m = bw.short_terms(f, &file, &[(seq.x, 0), (seq.e, 0)], p, heads);
    let any = f.ins().bor(m[0], m[1]);
    let more = f.create_block();
    f.ins().brif(any, hit, &[], more, &[]);
    // None in these 16: on, unless they reach the file's end (the region open there).
    f.switch_to_block(more);
    let p16 = f.ins().iadd_imm(p, 16);
    let past = f.ins().icmp(IntCC::SignedGreaterThanOrEqual, p16, file.len);
    (e.guard)(f, past);
    f.ins().jump(scan, &[p16.into()]);
    f.switch_to_block(hit);
    let tz = f.ins().ctz(any);
    let k = lane_of(f, tz, stride);
    let i = f.ins().iadd(p, k);
    let (here, content) = (f.create_block(), f.create_block());
    let at_it = f.ins().icmp(IntCC::Equal, i, at);
    f.ins().brif(at_it, here, &[], content, &[]);
    // The bytes before it: a content token, the next at it.
    f.switch_to_block(content);
    (e.set)(f, Mem::PartAt, i);
    let l = f.ins().iadd_imm(i, -1);
    split_token(f, e, sp, (at, l), to(|o| o.content), read_word, Some(std::array::from_fn(|k| !seq.stops[k])));
    // At `at`: the closer, or a part of two bytes.
    f.switch_to_block(here);
    let sh = if stride == 1 { tz } else { f.ins().imul_imm(k, stride) };
    let xbit = f.ins().ushr(m[0], sh);
    let xbit = f.ins().band_imm(xbit, 1);
    let (closer, part) = (f.create_block(), f.create_block());
    f.ins().brif(xbit, closer, &[], part, &[]);
    f.switch_to_block(closer);
    let zero = f.ins().iconst(types::I64, 0);
    (e.set)(f, Mem::PartAt, zero);
    split_token(f, e, sp, (at, at), to(|o| o.closer), read_word, Some(seq.xs));
    f.switch_to_block(part);
    let after = f.ins().iadd_imm(at, 2);
    (e.set)(f, Mem::PartAt, after);
    let l = f.ins().iadd_imm(at, 1);
    match &seq.codes2 {
        // Its code by its second byte, one load.
        Some(t) => {
            let at2 = f.ins().iadd(file.bytes, l);
            let b = f.ins().uload8(types::I64, MemFlags::trusted(), at2, 0);
            let base = table_address(f, t.as_ptr());
            let off = f.ins().ishl_imm(b, 2);
            let a = f.ins().iadd(base, off);
            let code = f.ins().uload32(MemFlags::trusted().with_readonly(), a, 0);
            let l1 = f.ins().iadd_imm(l, 1);
            (e.set)(f, Mem::Start, at);
            (e.set)(f, Mem::Limit, l1);
            (e.set)(f, Mem::Taken, code);
            f.ins().jump(to(|o| o.part), &[]);
        }
        None => split_token(f, e, sp, (at, l), fetched, read_word, None),
    }
}

/// Where a sequential region's parts go once read (`split_next_seq`), where the `SWITCH` after
/// the `TOKEN` takes each outcome to one place: the closer, the content and the part (an
/// escape), each to the switch's target for its kinds, or to its words' switch. As
/// `parse_string`'s outcomes, each branch once. None: on through the token's common path.
#[derive(Clone, Copy, Default)]
struct Onward {
    closer: Option<Block>,
    content: Option<Block>,
    part: Option<Block>,
}

/// A mask's lane from its bit (`short_word`'s masks have `stride` bits a lane): a shift where
/// the stride is a power of two.
fn lane_of(f: &mut FunctionBuilder<'_>, bit: Value, stride: i64) -> Value {
    match stride {
        1 => bit,
        s if (s as u64).is_power_of_two() => f.ins().ushr_imm(bit, i64::from(s.trailing_zeros())),
        s => f.ins().udiv_imm(bit, s),
    }
}

/// A sequential region's record (`Seq`): its opener alone, the token, its parts pending from
/// the byte after (`Mem::PartAt`). `opens`: whether the token is the region's (none: it is).
fn split_enter_seq(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, sp: &SplitParts, s: Value, opens: Option<Value>, words: Option<&[u8]>) {
    let (enter, on) = (f.create_block(), f.create_block());
    match opens {
        Some(o) => {
            f.ins().brif(o, enter, &[], on, &[]);
        }
        None => {
            f.ins().jump(enter, &[]);
        }
    }
    f.switch_to_block(enter);
    let s1 = f.ins().iadd_imm(s, 1);
    (e.set)(f, Mem::PartAt, s1);
    split_token(f, e, sp, (s, s), on, words, sp.seq.as_ref().map(|q| q.xs));
    f.switch_to_block(on);
}

/// A record of a split region (`SplitParts`, `Split::Tier2`): at its opener's bit and past its
/// byte. Its opener's byte is the token, its parts pending from the byte after (`Mem::PartAt`)
/// to its closer's last (`Mem::PartEnd`, as `records::read_parts` ends them). The record is the
/// whole region (the token loop's, the masks'), its last not read. In a grammar with templates
/// (decision 0600) it is also its opener alone, the masks' of every region, its parts no records;
/// in another, a record of its opener alone is split already (tier 1's, `Split::Tier1`): a token
/// as it is.
fn split_enter(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, sp: &SplitParts, bw: &crate::native_cpu_bytes::Bytewise<'_>, (s, l): (Value, Value), words: Option<&[u8]>, heads: &mut Vec<(Block, Step)>) {
    let _ = words;
    let templates = sp.readers.iter().any(|rd| rd.template.is_some());
    let whole = f.ins().icmp(IntCC::UnsignedGreaterThan, l, s);
    let whole = f.ins().uextend(types::I64, whole);
    let on = f.create_block();
    for (k, rd) in sp.readers.iter().enumerate() {
        let q = f.ins().ushr_imm(s, 6);
        let x = output_word(f, e, bw, rd.opener, q, heads);
        let sh = f.ins().band_imm(s, 63);
        let x = f.ins().ushr(x, sh);
        let opens = f.ins().band_imm(x, 1);
        let opens = if templates { opens } else { f.ins().band(opens, whole) };
        let (enter, next) = (f.create_block(), f.create_block());
        f.ins().brif(opens, enter, &[], next, &[]);
        f.switch_to_block(enter);
        let s1 = f.ins().iadd_imm(s, 1);
        (e.set)(f, Mem::PartAt, s1);
        let kv = f.ins().iconst(types::I64, k as i64);
        (e.set)(f, Mem::PartRd, kv);
        // Its closer's first byte, by the closer's bits past the opener, as the record's last
        // was read (`records::read_last`).
        let (c, _) = first_bit_from(f, e, bw, rd.closer, s1, heads);
        (e.set)(f, Mem::PartClose, c);
        // Its closer piece to the closer's own last: a string's record runs to its last quote
        // (decision 0600), and the masks' to its opener.
        let pe = closer_last(f, e, bw, rd, c, heads);
        (e.set)(f, Mem::PartEnd, pe);
        // The opener's token: the record's code (its marks), its word its byte's alone where it
        // has no tag.
        let taken = (e.get)(f, Mem::Taken);
        let code = if rd.tagged {
            taken
        } else {
            let bytes = (e.ctx)(f, BYTES);
            let kc = crate::native_cpu_block::tables_consts(f, &sp.tables);
            let one = f.ins().iconst(types::I64, 1);
            let plain = crate::native_cpu_block::plain_code(f, &sp.tables, kc, bytes, s, one);
            let w = f.ins().band_imm(plain, 0xff00);
            let rest = f.ins().band_imm(taken, !0xff00);
            f.ins().bor(rest, w)
        };
        (e.set)(f, Mem::Start, s);
        (e.set)(f, Mem::Limit, s1);
        (e.set)(f, Mem::Taken, code);
        f.ins().jump(on, &[]);
        f.switch_to_block(next);
    }
    f.ins().jump(on, &[]);
    f.switch_to_block(on);
}

/// What a statement's recovery reads (decision 0650, `recover_statement`): the patterns, the extras' bit, the line
/// ends' output (stored by tier 1, or made from the bytes) and its block program, the refusals that are limits (a bit
/// a refusal's value), and whether the program wraps.
struct Recovery<'a> {
    recover: &'a [crate::program::Recover],
    extra_bit: Option<u8>,
    lines: Option<(u16, bool)>,
    block: Option<&'a crate::block_native::BlockNative>,
    limits: &'a [u64],
    wrapping: bool,
}

/// 1 where refusal `v` (a word's place plus one, `OVERFLOW`; 0, none) is one a statement recovers from: a statement
/// is marked, and `v` is no limit (`Recovery::limits`: the depth, a count, a check, tier 1's, the file's end), else 0.
fn recoverable(f: &mut FunctionBuilder<'_>, ctx: Value, r: &Recovery<'_>, v: Value) -> Value {
    let flags = MemFlags::trusted();
    let marked = f.ins().load(types::I64, flags, ctx, MARK);
    let inside = f.ins().icmp_imm(IntCC::UnsignedLessThan, v, (r.limits.len() * 64) as i64);
    let (look, done) = (f.create_block(), f.create_block());
    f.append_block_param(done, types::I64);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().brif(inside, look, &[], done, &[BlockArg::from(zero)]);
    f.switch_to_block(look);
    let w = f.ins().ushr_imm(v, 6);
    let w = f.ins().ishl_imm(w, 3);
    let base = table_address(f, r.limits.as_ptr());
    let a = f.ins().iadd(base, w);
    let word = f.ins().load(types::I64, flags.with_readonly(), a, 0);
    let sh = f.ins().band_imm(v, 63);
    let bit = f.ins().ushr(word, sh);
    let bit = f.ins().band_imm(bit, 1);
    let free = f.ins().bxor_imm(bit, 1);
    f.ins().jump(done, &[BlockArg::from(free)]);
    f.switch_to_block(done);
    let free = f.block_params(done)[0];
    f.ins().band(free, marked)
}

/// 1 where a bit of the line ends' mask is among bytes `from..to` (`Recovery::lines`), else 0: a stated loop of
/// words.
fn line_end_among(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, r: &Recovery<'_>, from: Value, to: Value, heads: &mut Vec<(Block, Step)>) -> Value {
    let zero = f.ins().iconst(types::I64, 0);
    let Some((o, stored)) = r.lines else { return zero };
    let one = f.ins().iconst(types::I64, 1);
    let (go, words, done) = (f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(words, types::I64);
    f.append_block_param(done, types::I64);
    let none = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, from, to);
    f.ins().brif(none, done, &[BlockArg::from(zero)], go, &[]);
    f.switch_to_block(go);
    let q0 = f.ins().ushr_imm(from, 6);
    let to1 = f.ins().iadd_imm(to, -1);
    let qe = f.ins().ushr_imm(to1, 6);
    // Tier 1's words, or the mask's words from the bytes (`native_cpu_bytes`).
    let made = if stored {
        let (outputs, stride, lasts_stride) = ((e.ctx)(f, OUTPUTS), (e.ctx)(f, OUTPUT_STRIDE), (e.ctx)(f, LASTS_STRIDE));
        let off = f.ins().imul_imm(stride, i64::from(o));
        let base = f.ins().iadd(outputs, off);
        Err((base, lasts_stride))
    } else {
        let consts = crate::native_cpu_block::Consts::make(f);
        let (bytes, len) = ((e.ctx)(f, BYTES), (e.ctx)(f, LEN));
        Ok(crate::native_cpu_bytes::File { bytes, len, nibble: consts.nibble, weights: consts.weights, even: consts.even })
    };
    f.ins().jump(words, &[BlockArg::from(q0)]);
    f.switch_to_block(words);
    heads.push((words, Step::Block));
    let q = f.block_params(words)[0];
    let x = match &made {
        Err((base, lasts_stride)) => mask_word(f, *base, *lasts_stride, q),
        Ok(file) => {
            let bw = crate::native_cpu_bytes::Bytewise::of(r.block.expect("a block program"));
            let p = f.ins().ishl_imm(q, 6);
            bw.word(f, file, o, p, heads)
        }
    };
    let m = within_word(f, q, from, to);
    let x = f.ins().band(x, m);
    let any = f.ins().icmp_imm(IntCC::NotEqual, x, 0);
    let more = f.create_block();
    f.ins().brif(any, done, &[BlockArg::from(one)], more, &[]);
    f.switch_to_block(more);
    let last = f.ins().icmp(IntCC::Equal, q, qe);
    let q1 = f.ins().iadd_imm(q, 1);
    f.ins().brif(last, done, &[BlockArg::from(zero)], words, &[BlockArg::from(q1)]);
    f.switch_to_block(done);
    f.block_params(done)[0]
}

/// A refusal in a marked statement, recovered as the lean parser's `recover` (decision 0650; the interpreter's
/// `run_seen`): the state back to the mark (the tape's codes taken back, its last block's base again), the
/// statement before closed at its last token as its `CLOSE` does, then from the record after the statement's first
/// the first where a pattern holds (`at` it, or `past` it; extras skipped; a stated loop of records), and one ERROR
/// leaf from the statement's first token to the last before there. Then the root's head takes the next token
/// (`Need`); the mark is gone. On, in the current block.
fn recover_statement(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, ctx: Value, t: Table<'_>, r: &Recovery<'_>, heads: &mut Vec<(Block, Step)>) {
    let flags = MemFlags::trusted();
    // The state as the statement found it; the tape's own (its codes, last block's base, wide flags) as the
    // table's steps leave them.
    for k in Mem::ALL {
        if matches!(k, Mem::Coded | Mem::Codes | Mem::Wide | Mem::BlockBase) {
            continue;
        }
        let x = f.ins().load(types::I64, flags, ctx, MARK_STATE + 8 * k as i32);
        (e.set)(f, k, x);
    }
    let zero = f.ins().iconst(types::I64, 0);
    (e.set)(f, Mem::Shift, zero);
    let nodes = (e.get)(f, Mem::Nodes);
    t.uncode(f, e, nodes);
    t.cut_to(f, e, nodes);
    let open_c = (e.ctx)(f, OPEN);
    for d in 0..MARK_LEVELS {
        let x = f.ins().load(types::I64, flags, ctx, MARK_OPEN + 8 * d as i32);
        let i = f.ins().iconst(types::I64, d as i64);
        slot_put(f, open_c, i, x);
    }
    // The levels above the root closed, the statement before's.
    let (head, close, closed) = (f.create_block(), f.create_block(), f.create_block());
    f.ins().jump(head, &[]);
    f.switch_to_block(head);
    heads.push((head, Step::Node));
    let open = (e.get)(f, Mem::Open);
    let above = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, open, 1);
    f.ins().brif(above, close, &[], closed, &[]);
    f.switch_to_block(close);
    close_word(f, e, t, false, EndAt::Field, r.wrapping);
    f.ins().jump(head, &[]);
    f.switch_to_block(closed);
    // The resume point.
    let (codes, starts, lasts, stride, n) = ((e.ctx)(f, CODES), (e.ctx)(f, STARTS), (e.ctx)(f, LASTS), (e.ctx)(f, RECORD_STRIDE), (e.ctx)(f, NSTARTS));
    let column = |f: &mut FunctionBuilder<'_>, c: Value, j: Value| {
        let off = f.ins().imul(j, stride);
        let a = f.ins().iadd(c, off);
        f.ins().uload32(flags, a, 0)
    };
    let is_extra = |f: &mut FunctionBuilder<'_>, c: Value| -> Option<Value> {
        let b = r.extra_bit?;
        let x = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT) + i64::from(b));
        Some(f.ins().band_imm(x, 1))
    };
    let next = (e.get)(f, Mem::Next);
    let first = f.ins().iadd_imm(next, -1);
    let (scan, test, step, found) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(scan, types::I64);
    f.append_block_param(found, types::I64);
    f.ins().jump(scan, &[BlockArg::from(next)]);
    f.switch_to_block(scan);
    heads.push((scan, Step::Token));
    let k = f.block_params(scan)[0];
    let end = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, k, n);
    let on = f.create_block();
    f.ins().brif(end, found, &[BlockArg::from(k)], on, &[]);
    f.switch_to_block(on);
    let c = column(f, codes, k);
    match is_extra(f, c) {
        Some(x) => f.ins().brif(x, step, &[], test, &[]),
        None => f.ins().jump(test, &[]),
    };
    f.switch_to_block(test);
    let k1 = f.ins().iadd_imm(k, 1);
    // Each pattern from record `k` on: a token mark's elements, `k` its token (`Next` one past it).
    let get = |f: &mut FunctionBuilder<'_>, m: Mem| if matches!(m, Mem::Next) { k1 } else { (e.get)(f, m) };
    let at_k = WordEnv { get: &get, set: e.set, ctx: e.ctx, guard: e.guard };
    for past in [false, true] {
        for p in r.recover.iter().filter(|p| p.past == past) {
            let m = crate::program::TokenMark { order: 0, elems: p.elems.clone(), refuses: false, unless: false };
            let mut holds = token_pattern(f, &at_k, &m, r.extra_bit, heads);
            if p.first {
                // First on its line: the record's the file's first, or a line end since the record before.
                let (lines, other) = (f.create_block(), f.create_block());
                f.append_block_param(lines, types::I64);
                let at0 = f.ins().icmp_imm(IntCC::Equal, k, 0);
                let one = f.ins().iconst(types::I64, 1);
                f.ins().brif(at0, lines, &[BlockArg::from(one)], other, &[]);
                f.switch_to_block(other);
                let before = f.ins().iadd_imm(k, -1);
                let from = column(f, lasts, before);
                let from = f.ins().iadd_imm(from, 1);
                let to = column(f, starts, k);
                let x = line_end_among(f, e, r, from, to, heads);
                f.ins().jump(lines, &[BlockArg::from(x)]);
                f.switch_to_block(lines);
                let x = f.block_params(lines)[0];
                holds = f.ins().band(holds, x);
            }
            let other = f.create_block();
            f.ins().brif(holds, found, &[BlockArg::from(if past { k1 } else { k })], other, &[]);
            f.switch_to_block(other);
        }
    }
    f.ins().jump(step, &[]);
    f.switch_to_block(step);
    let k1 = f.ins().iadd_imm(k, 1);
    f.ins().jump(scan, &[BlockArg::from(k1)]);
    f.switch_to_block(found);
    let k = f.block_params(found)[0];
    // The leaf's last record: the last before the resume point that is no extra (the statement's first, at least).
    let past_n = f.ins().icmp(IntCC::UnsignedGreaterThan, k, n);
    let k = f.ins().select(past_n, n, k);
    let j0 = f.ins().iadd_imm(k, -1);
    let (back, more, got) = (f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(back, types::I64);
    f.append_block_param(got, types::I64);
    f.ins().jump(back, &[BlockArg::from(j0)]);
    f.switch_to_block(back);
    heads.push((back, Step::Token));
    let j = f.block_params(back)[0];
    let c = column(f, codes, j);
    match is_extra(f, c) {
        Some(x) => f.ins().brif(x, more, &[], got, &[BlockArg::from(j)]),
        None => f.ins().jump(got, &[BlockArg::from(j)]),
    };
    f.switch_to_block(more);
    let at_first = f.ins().icmp(IntCC::UnsignedLessThanOrEqual, j, first);
    let j1 = f.ins().iadd_imm(j, -1);
    f.ins().brif(at_first, got, &[BlockArg::from(first)], back, &[BlockArg::from(j1)]);
    f.switch_to_block(got);
    let g = f.block_params(got)[0];
    // The leaf: the statement's first token's start to the last's end.
    let s = column(f, starts, first);
    let l = column(f, lasts, g);
    let leaf_end = f.ins().iadd_imm(l, 1);
    (e.set)(f, Mem::Start, s);
    (e.set)(f, Mem::End, leaf_end);
    let kind = f.ins().iconst(types::I64, i64::from(crate::native_cpu_tape::ERROR_ROW));
    leaf_word(f, e, t, kind, LeafShape { deferred: false, extra: false, end: EndAt::Field }, r.wrapping);
    // The head takes the token after it.
    let g1 = f.ins().iadd_imm(g, 1);
    (e.set)(f, Mem::Next, g1);
    (e.set)(f, Mem::Limit, leaf_end);
    let one = f.ins().iconst(types::I64, 1);
    (e.set)(f, Mem::Need, one);
    (e.set)(f, Mem::Lost, zero);
    f.ins().store(flags, zero, ctx, MARK);
    f.ins().store(flags, zero, ctx, MARK_HELD);
}

/// `TOKEN`'s body (`WordFn::Token`), taking a token: params `ctx`, the calls open. The room for
/// the path to the next token, then the next record, or the stream's end (its own kind's code). 0, or 1 where the stream had ended
/// already, or `OVERFLOW` out of room.
fn take_token(f: &mut FunctionBuilder<'_>, p: &[Value], any_mark: Option<usize>, t: TokenShape, table: Table<'_>, refusals: &[crate::program::TokenMark], heads: &mut Vec<(Block, Step)>) {
    let ctx = p[0];
    let flags = MemFlags::trusted();
    let load = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, flags, ctx, at);
    let pops = t.bound.pop_nodes > 0 || t.bound.pop_supers > 0;
    let open_calls = if !t.on_stack { p[1] } else if pops { calls_open_by_stack(f, ctx) } else { f.ins().iconst(types::I64, 0) };
    // Each token from here: an extra taken (`TOKEN_EXTRAS`) comes back for the next.
    let top = f.create_block();
    f.ins().jump(top, &[]);
    f.switch_to_block(top);
    heads.push((top, Step::Token));
    let over = f.create_block();
    // None where the tape is the bound's from the start (`Bounds::unchecked`).
    let (mut counts, mut rooms, mut most) = (vec![], vec![], vec![]);
    if !t.unchecked {
        counts = vec![field_get(f, ctx, Mem::Nodes), field_get(f, ctx, Mem::Supers)];
        rooms = vec![load(f, ROOM_NODES), load(f, ROOM_SUPERS)];
        most = vec![(t.bound.nodes, t.bound.pop_nodes), (t.bound.supers, t.bound.pop_supers)];
    }
    if t.unchecked {
        // The block before is filled before the cold one is entered.
        let fits = f.create_block();
        f.ins().jump(fits, &[]);
        f.switch_to_block(fits);
    }
    if !t.on_stack && !t.unchecked {
        counts.push(open_calls);
        rooms.push(load(f, ROOM_PUSHES));
        most.push((t.bound.pushes, 0));
    }
    for (at, (room, (most, per_call))) in counts.into_iter().zip(rooms.into_iter().zip(most)) {
        let at = if per_call > 0 {
            let popped = f.ins().imul_imm(open_calls, per_call as i64);
            f.ins().iadd(at, popped)
        } else {
            at
        };
        let reach = f.ins().iadd_imm(at, most as i64);
        let past = f.ins().icmp(IntCC::UnsignedGreaterThan, reach, room);
        let fits = f.create_block();
        f.ins().brif(past, over, &[], fits, &[]);
        f.switch_to_block(fits);
    }
    let fits = f.current_block().expect("in a block");
    f.switch_to_block(over);
    f.set_cold_block(over);
    let overflow = f.ins().iconst(types::I64, OVERFLOW as i64);
    f.ins().return_(&[overflow]);
    f.switch_to_block(fits);
    // A token a refusing mark held, taken: its statement refused here (decision 0650).
    if t.recovers {
        let held = load(f, MARK_HELD);
        let (no, on) = (f.create_block(), f.create_block());
        f.ins().brif(held, no, &[], on, &[]);
        f.switch_to_block(no);
        f.set_cold_block(no);
        let one = f.ins().iconst(types::I64, 1);
        f.ins().return_(&[one]);
        f.switch_to_block(on);
    }
    let (next, end) = (field_get(f, ctx, Mem::Next), field_get(f, ctx, Mem::Limit));
    field_set(f, ctx, Mem::LastEnd, end);
    if t.lost {
        let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
        let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
        let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
        let guard = |_: &mut FunctionBuilder<'_>, _: Value| unreachable!("a leaf does not refuse");
        lost_leaf(f, &WordEnv { get: &get, set: &set, ctx: &at, guard: &guard }, table, t.wrapping);
    }
    let zero = f.ins().iconst(types::I64, 0);
    // A region tier 2 splits (decision 0323): its next part, where one is pending; else the
    // next record, and a record that is a whole region its opener's byte, its parts pending.
    let split = table.split.map(|(sp, block)| (sp, crate::native_cpu_bytes::Bytewise::of(block)));
    let fetched = split.as_ref().map(|_| f.create_block());
    let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
    let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
    let at_ctx = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
    let no_guard = |_: &mut FunctionBuilder<'_>, _: Value| unreachable!("a token's fetch does not refuse");
    let senv = WordEnv { get: &get, set: &set, ctx: &at_ctx, guard: &no_guard };
    if let (Some((sp, bw)), Some(fetched)) = (&split, fetched) {
        split_next(f, &senv, sp, bw, fetched, None, None, heads);
    }
    // The stream's end, one compare a token (`Next` is one past the count once the end is
    // taken): the end's token, or past it.
    let nstarts = load(f, NSTARTS);
    let is_end = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, next, nstarts);
    let (ends, last, ended, token) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
    f.ins().brif(is_end, ends, &[], token, &[]);
    f.switch_to_block(ends);
    f.set_cold_block(ends);
    let past = f.ins().icmp(IntCC::UnsignedGreaterThan, next, nstarts);
    f.ins().brif(past, ended, &[], last, &[]);
    f.switch_to_block(ended);
    f.set_cold_block(ended);
    let one = f.ins().iconst(types::I64, 1);
    f.ins().return_(&[one]);
    f.switch_to_block(last);
    f.set_cold_block(last);
    let past = f.ins().iadd_imm(nstarts, 1);
    field_set(f, ctx, Mem::Next, past);
    let at_end_live = f.ins().iconst(types::I64, t.end_bits);
    field_set(f, ctx, Mem::Live, at_end_live);
    if let Some(code) = t.end_code {
        let code = f.ins().iconst(types::I64, code);
        field_set(f, ctx, Mem::Taken, code);
    }
    f.ins().return_(&[zero]);
    // The token's record: its first byte, last byte and code, where tier 1's layout has them
    // (`Ctx::record_stride`).
    f.switch_to_block(token);
    let stride = load(f, RECORD_STRIDE);
    let off = f.ins().imul(next, stride);
    let column = |f: &mut FunctionBuilder<'_>, at: i32| {
        let c = load(f, at);
        let a = f.ins().iadd(c, off);
        f.ins().uload32(flags, a, 0)
    };
    let (s, l, c) = (column(f, STARTS), column(f, LASTS), column(f, CODES));
    let l1 = f.ins().iadd_imm(l, 1);
    let after = f.ins().iadd_imm(next, 1);
    field_set(f, ctx, Mem::Start, s);
    field_set(f, ctx, Mem::Next, after);
    field_set(f, ctx, Mem::Limit, l1);
    field_set(f, ctx, Mem::Taken, c);
    let (s, c) = match (&split, fetched) {
        (Some((sp, bw)), Some(fetched)) => {
            split_enter(f, &senv, sp, bw, (s, l), None, heads);
            f.ins().jump(fetched, &[]);
            f.switch_to_block(fetched);
            (field_get(f, ctx, Mem::Start), field_get(f, ctx, Mem::Taken))
        }
        _ => (s, c),
    };
    // An extra: its bytes taken and its leaf deferred (`SPAN m=1; LEAF m=1 #K`), then the next
    // token, as the lean parser places a comment where it passes it.
    if let Some((bit, kind)) = t.extra {
        let b = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT) + i64::from(bit));
        let b = f.ins().band_imm(b, 1);
        let (extra, on) = (f.create_block(), f.create_block());
        f.ins().brif(b, extra, &[], on, &[]);
        f.switch_to_block(extra);
        let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
        let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
        let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
        let guard = |_: &mut FunctionBuilder<'_>, _: Value| unreachable!("a leaf does not refuse");
        let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &guard };
        let k = t.more.kind(f, c, kind);
        leaf_word(f, &env, table, k, LeafShape { deferred: true, extra: true, end: EndAt::Taken }, t.wrapping);
        f.ins().jump(top, &[]);
        f.switch_to_block(on);
    }
    // A token mark that refuses, at this token: 1, as the refusals past the end.
    if refusals.iter().any(|m| m.refuses) {
        let refused = f.create_block();
        let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
        let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
        let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
        let guard = |_: &mut FunctionBuilder<'_>, _: Value| unreachable!("a token mark does not refuse");
        let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &guard };
        refusal_tests(f, &env, c, refusals, t.extra.map(|e| e.0), refused, heads);
        let on = f.current_block().expect("in a block");
        f.switch_to_block(refused);
        f.set_cold_block(refused);
        let one = f.ins().iconst(types::I64, 1);
        if t.recovers {
            // Held: the statement that takes it refuses (decision 0650).
            f.ins().store(flags, one, ctx, MARK_HELD);
            f.ins().jump(on, &[]);
        } else {
            f.ins().return_(&[one]);
        }
        f.switch_to_block(on);
    }
    let live = match any_mark {
        _ if t.in_code > 0 => {
            let marks = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT));
            if t.rest_bits == 0 { marks } else { f.ins().bor_imm(marks, t.rest_bits) }
        }
        Some(output) => {
            let (outputs_p, output_stride, lasts_stride) = (load(f, OUTPUTS), load(f, OUTPUT_STRIDE), load(f, LASTS_STRIDE));
            let w = f.ins().ushr_imm(s, 6);
            let o = f.ins().imul_imm(output_stride, output as i64);
            let base = f.ins().iadd(outputs_p, o);
            let word = mask_word(f, base, lasts_stride, w);
            let sh = f.ins().band_imm(s, 63);
            let bit = f.ins().ushr(word, sh);
            let bit = f.ins().band_imm(bit, 1);
            f.ins().ineg(bit)
        }
        None => f.ins().iconst(types::I64, -1),
    };
    field_set(f, ctx, Mem::Live, live);
    f.ins().return_(&[zero]);
}

/// `TOKEN`'s function (`WordFn::Token`), made where first called.
fn token_fn<'a>(module: &mut JITModule, fns: &'a mut std::collections::HashMap<WordFn, (cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function)>, any_mark: Option<usize>, shape: TokenShape, table: Table<'_>, refusals: &[crate::program::TokenMark], pinned: bool) -> Result<&'a (cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function), Error> {
    let key = WordFn::Token { any_mark, extras: shape.extra.is_some() };
    // `TOKEN` as a function of its own reads tier 1's records: it makes none (page 40's `Fetch`
    // is the inline `TOKEN`'s).
    if table.fetch.is_some() {
        return Err(Error::UnsupportedFeature("records made in tier 2, with `TOKEN` a function of its own"));
    }
    if !fns.contains_key(&key) {
        let name = format!("tempo_word_{}", fns.len());
        let made = outline(module, &name, if shape.on_stack { 1 } else { 2 }, 1, pinned, &table.fns.names(), |g, q, heads| take_token(g, q, any_mark, shape, table, refusals, heads))?;
        fns.insert(key, made);
    }
    Ok(&fns[&key])
}

/// A call of a word's function (`outline`): `ctx`, then its arguments; with `ctx` pinned, the
/// arguments alone.
fn call_word(f: &mut FunctionBuilder<'_>, id: cranelift_module::FuncId, signature: &cranelift_codegen::ir::Signature, pinned: bool, ctx: Value, args: &[Value]) -> Vec<Value> {
    let all: Vec<Value> = (!pinned).then_some(ctx).into_iter().chain(args.iter().copied()).collect();
    call_own(f, id, signature, &all)
}

/// A call of an outlined function (`outline`), from `f`.
pub(crate) fn call_own(f: &mut FunctionBuilder<'_>, id: cranelift_module::FuncId, signature: &cranelift_codegen::ir::Signature, args: &[Value]) -> Vec<Value> {
    let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
    let signature = f.import_signature(signature.clone());
    let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
    let call = f.ins().call(callee, args);
    f.inst_results(call).to_vec()
}

/// A function of tier 2's own (`outline`): its id, signature and body.
type OwnFn = (cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function);

/// The functions the node words call to keep the table as it stands at the end, as the lean
/// parser keeps its tape: each rare, each a
/// loop over the table's tail. `shift`: a wrap's (where it wraps more than a leaf, or a node a
/// supertype names); `unpend`: a close the pending extras outlive; `drop`: a dropped node's.
struct TableFns {
    shift: Option<OwnFn>,
    drop: Option<OwnFn>,
    /// The tape's (`native_cpu_tape`): a block's base and offsets again, a commit of code ids,
    /// codes taken back, the image's end.
    blocks: OwnFn,
    commit: OwnFn,
    uncode: OwnFn,
    finish: OwnFn,
    /// A format 16 run's (`compile_with`): the writer's ops, defined in the module, and the passes that call them.
    sixteen: Option<Sixteen>,
}

/// The format 16 writer's ops in tier 2's module, and the functions of tier 2's own that call them
/// (`native_cpu_tape::pass`, `native_cpu_tape::retag`).
struct Sixteen {
    ops: crate::native_cpu_tape::Ops,
    pass: OwnFn,
    retag: OwnFn,
}

impl TableFns {
    /// `near`: the module's code in one arena (`Cut::near`), so the writer's ops call each other directly.
    fn make(module: &mut JITModule, words: &[crate::native_ir::StructuralBlock], x: crate::native_cpu_tape::Image<'_>, sixteen: bool, near: bool) -> Result<Self, Error> {
        let w = x.widths;
        let inner = words.iter().any(|b| matches!(b.operation, Op::NodeField { on: On::Inner, .. } | Op::Reread { .. }));
        let wraps = words.iter().any(|b| matches!(b.operation, Op::Wrap { .. }));
        let drops = words.iter().any(|b| matches!(b.operation, Op::Reread { .. } | Op::Retag { kind: DEAD, on: On::Made }));
        let shift = if wraps { Some(outline(module, "tempo_stream_shift", 8, 0, false, &[], |f, p, heads| shift_rows(f, p, inner, !sixteen, w, heads))?) } else { None };
        let drop = if drops { Some(outline(module, "tempo_stream_drop", 14, 4, false, &[], |f, p, heads| drop_rows(f, p, inner, !sixteen, w, heads))?) } else { None };
        let blocks = outline(module, "tempo_tape_blocks", 5, 2, false, &[], |f, p, heads| crate::native_cpu_tape::blocks_from(f, p, w.off, heads))?;
        let commit = outline(module, "tempo_tape_commit", 11, 1, false, &[], |f, p, heads| crate::native_cpu_tape::commit(f, p, x.classes, heads))?;
        let uncode = outline(module, "tempo_tape_uncode", 5, 1, false, &[], crate::native_cpu_tape::uncode)?;
        let finish = outline(module, "tempo_tape_finish", 10, 1, false, &[], |f, p, heads| crate::native_cpu_tape::finish(f, p, x, heads))?;
        let sixteen = if sixteen {
            let ids = rerail_clif::writer::define_ops(module, cranelift_module::Linkage::Local, near).map_err(|e| {
                if std::env::var_os("RERAIL_DEBUG_STREAM").is_some() {
                    eprintln!("the writer's ops: {e}");
                }
                Error::BackendFailure
            })?;
            let ops = crate::native_cpu_tape::Ops::of(&ids);
            let names = ops.names();
            let pass = outline(module, "tempo_tape_pass", 10, 0, false, &names, |f, p, heads| crate::native_cpu_tape::pass(f, p, x.classes, w.size, &ops, heads))?;
            let retag = outline(module, "tempo_tape_retag", 7, 0, false, &names, |f, p, heads| crate::native_cpu_tape::retag(f, p, x.classes, &ops, heads))?;
            Some(Sixteen { ops, pass, retag })
        } else {
            None
        };
        Ok(TableFns { shift, drop, blocks, commit, uncode, finish, sixteen })
    }

    /// The functions' names: the node words' functions call them.
    fn names(&self) -> Vec<cranelift_codegen::ir::UserExternalName> {
        let passes = self.sixteen.iter().flat_map(|s| [&s.pass, &s.retag]);
        let mut names: Vec<_> = self.shift.iter().chain(&self.drop).chain([&self.blocks, &self.commit, &self.uncode, &self.finish]).chain(passes).map(|f| cranelift_codegen::ir::UserExternalName::new(0, f.0.as_u32())).collect();
        if let Some(s) = &self.sixteen {
            names.extend(s.ops.names());
        }
        names
    }

    fn all(self) -> impl Iterator<Item = OwnFn> {
        let passes = self.sixteen.into_iter().flat_map(|s| [s.pass, s.retag]);
        self.shift.into_iter().chain(self.drop).chain([self.blocks, self.commit, self.uncode, self.finish]).chain(passes)
    }
}

/// A row's 12 bytes (its key and start, its end), loaded or stored.
fn row_words(f: &mut FunctionBuilder<'_>, row: Value) -> [Value; 2] {
    [f.ins().load(types::I64, MemFlags::trusted(), row, 0), f.ins().uload32(MemFlags::trusted(), row, 8)]
}

fn put_row_words(f: &mut FunctionBuilder<'_>, row: Value, w: [Value; 2]) {
    f.ins().store(MemFlags::trusted(), w[0], row, 0);
    f.ins().istore32(MemFlags::trusted(), w[1], row, 8);
}

/// `(rows, head, snode, r, n, supers, size, tail)`: a wrap at row `r` of a table of `n`: rows
/// `r..n` up a row, latest first, with their size and tail cells; `head` (which rows are wraps)
/// moves with them. The supertypes' nodes from `r` on, the last ones, one more. A step a row
/// (`Step::Node`). The caller sets the blocks from `r` again.
/// `tails`: the tails moved too (a format 15 run's; a format 16 run's pass reads only the sizes).
fn shift_rows(f: &mut FunctionBuilder<'_>, p: &[Value], inner: bool, tails: bool, w: rerail_tape_format::Widths, heads: &mut Vec<(Block, Step)>) {
    let (rows, head, snode, r, n, supers, size_c, tail_c) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]);
    let (step, body, sups, done) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(step, types::I64);
    f.append_block_param(sups, types::I64);
    heads.extend([(step, Step::Node), (sups, Step::Node)]);
    f.ins().jump(step, &[BlockArg::from(n)]);
    // step(i): row i - 1 to row i.
    f.switch_to_block(step);
    let i = f.block_params(step)[0];
    let at_r = f.ins().icmp(IntCC::Equal, i, r);
    f.ins().brif(at_r, sups, &[BlockArg::from(supers)], body, &[]);
    f.switch_to_block(body);
    let from = f.ins().iadd_imm(i, -1);
    let (src, dst) = (NodeCol::row(f, rows, from), NodeCol::row(f, rows, i));
    let x = row_words(f, src);
    put_row_words(f, dst, x);
    for (col, width) in [(size_c, w.size), (tail_c, w.tail)].into_iter().take(if tails { 2 } else { 1 }) {
        let v = crate::native_cpu_tape::get(f, col, from, width);
        crate::native_cpu_tape::put(f, col, i, v, width);
    }
    if inner {
        let h = slot_get(f, head, from);
        slot_put(f, head, i, h);
    }
    f.ins().jump(step, &[BlockArg::from(from)]);
    // sups(k): the supertype k - 1, while its node is from r on.
    f.switch_to_block(sups);
    let k = f.block_params(sups)[0];
    let (look, bump) = (f.create_block(), f.create_block());
    f.ins().brif(k, look, &[], done, &[]);
    f.switch_to_block(look);
    let k1 = f.ins().iadd_imm(k, -1);
    let s = slot_get(f, snode, k1);
    let moved = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, s, r);
    f.ins().brif(moved, bump, &[], done, &[]);
    f.switch_to_block(bump);
    let s1 = f.ins().iadd_imm(s, 1);
    slot_put(f, snode, k1, s1);
    f.ins().jump(sups, &[BlockArg::from(k1)]);
    f.switch_to_block(done);
    f.ins().return_(&[]);
}

/// `(rows, before, to, head, snode, skind, from, n, supers, moved, size, tail, stack, outer)`: the
/// dropped rows (`DEAD`) of rows `from..n` taken out, the rest moved down. First each row's
/// count of dropped before it (`before`); then each kept row to its place, its size less the
/// dropped in its range (`to`: a kept row's place, a dropped one's parent's). A dropped row's
/// parent is the innermost of the rows before it whose range holds it: a stack of them
/// (`stack`, each its row and its range's end), or else `outer`, the innermost open node; it is
/// marked (`DROPPED_IN`). Then each kept
/// row's tail again, from its last descendant now; then the supertypes of the tail's nodes on
/// to their rows, a dropped node's gone. Gives back the rows, the supertypes, the node the last
/// wrap moved at its place, and `WIDE_TAIL` where a tail does not fit. A step a row.
/// `with_tails`: each kept row's tail again (a format 15 run's; a format 16 run's pass reads only the sizes).
fn drop_rows(f: &mut FunctionBuilder<'_>, p: &[Value], inner: bool, with_tails: bool, w: rerail_tape_format::Widths, heads: &mut Vec<(Block, Step)>) {
    let flags = MemFlags::trusted();
    let (rows, before, to, head, snode, skind, from, n, supers, moved, size_c, tail_c, stack, outer) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8], p[9], p[10], p[11], p[12], p[13]);
    let zero = f.ins().iconst(types::I64, 0);
    let (count, cbody, place, pbody, pop, popped, kept_row, tails, tbody, back, sback, sstep, sbody, sput, done) = (
        f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(),
        f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(),
    );
    // count(i, d); place(i, depth); pop(depth); tails(j, wide); back(k); sstep(s, kept); done(kept, wide).
    for (x, k) in [(count, 2), (place, 2), (pop, 1), (popped, 1), (tails, 2), (back, 2), (sback, 1), (sstep, 3), (sbody, 1), (sput, 1), (done, 2)] {
        for _ in 0..k {
            f.append_block_param(x, types::I64);
        }
    }
    heads.extend([(count, Step::Node), (place, Step::Node), (pop, Step::Node), (tails, Step::Node), (back, Step::Node), (sstep, Step::Node)]);
    f.ins().jump(count, &[BlockArg::from(from), BlockArg::from(zero)]);
    f.switch_to_block(count);
    let (i, d) = (f.block_params(count)[0], f.block_params(count)[1]);
    slot_put(f, before, i, d);
    let end = f.ins().icmp(IntCC::Equal, i, n);
    f.ins().brif(end, place, &[BlockArg::from(from), BlockArg::from(zero)], cbody, &[]);
    f.switch_to_block(cbody);
    let k = NodeCol::of(rows, AT_KIND).get(f, i);
    let dead = f.ins().icmp_imm(IntCC::Equal, k, i64::from(DEAD));
    let dead = f.ins().uextend(types::I64, dead);
    let d1 = f.ins().iadd(d, dead);
    let i1 = f.ins().iadd_imm(i, 1);
    f.ins().jump(count, &[BlockArg::from(i1), BlockArg::from(d1)]);
    // place(i, depth): row i; the stack holds `depth` rows whose ranges were open before it.
    f.switch_to_block(place);
    let (i, depth) = (f.block_params(place)[0], f.block_params(place)[1]);
    let end = f.ins().icmp(IntCC::Equal, i, n);
    let wide0 = zero;
    f.ins().brif(end, tails, &[BlockArg::from(from), BlockArg::from(wide0)], pbody, &[]);
    f.switch_to_block(pbody);
    f.ins().jump(pop, &[BlockArg::from(depth)]);
    // pop(depth): the rows whose ranges end at or before i, off the stack.
    f.switch_to_block(pop);
    let depth = f.block_params(pop)[0];
    let (look, off) = (f.create_block(), f.create_block());
    f.ins().brif(depth, look, &[], popped, &[BlockArg::from(depth)]);
    f.switch_to_block(look);
    let d1 = f.ins().iadd_imm(depth, -1);
    let e_at = f.ins().ishl_imm(d1, 1);
    let e_at = f.ins().iadd_imm(e_at, 1);
    let top_end = slot_get(f, stack, e_at);
    let ended = f.ins().icmp(IntCC::UnsignedLessThanOrEqual, top_end, i);
    f.ins().brif(ended, off, &[], popped, &[BlockArg::from(depth)]);
    f.switch_to_block(off);
    f.ins().jump(pop, &[BlockArg::from(d1)]);
    f.switch_to_block(popped);
    let depth = f.block_params(popped)[0];
    // The parent: the stack's top (its place, `to`), or else the innermost open node (`outer`,
    // before `from`).
    let has = f.ins().icmp_imm(IntCC::NotEqual, depth, 0);
    let d1 = f.ins().iadd_imm(depth, -1);
    let d1 = f.ins().select(has, d1, zero);
    let j_at = f.ins().ishl_imm(d1, 1);
    let pj = slot_get(f, stack, j_at);
    let pj = f.ins().select(has, pj, from);
    let mapped = slot_get(f, to, pj);
    let pr = f.ins().select(has, mapped, outer);
    let size = crate::native_cpu_tape::get(f, size_c, i, w.size);
    let e = f.ins().iadd(i, size);
    let e = f.ins().umin(e, n);
    // Row i on the stack, its range to `e`.
    let at = f.ins().ishl_imm(depth, 1);
    slot_put(f, stack, at, i);
    let at1 = f.ins().iadd_imm(at, 1);
    slot_put(f, stack, at1, e);
    let depth1 = f.ins().iadd_imm(depth, 1);
    let k = NodeCol::of(rows, AT_KIND).get(f, i);
    let dead = f.ins().icmp_imm(IntCC::Equal, k, i64::from(DEAD));
    let (gone, i1) = (f.create_block(), f.ins().iadd_imm(i, 1));
    f.ins().brif(dead, gone, &[], kept_row, &[]);
    f.switch_to_block(gone);
    slot_put(f, to, i, pr);
    if inner {
        let m = slot_get(f, head, pr);
        let m = f.ins().bor_imm(m, DROPPED_IN);
        slot_put(f, head, pr, m);
    }
    f.ins().jump(place, &[BlockArg::from(i1), BlockArg::from(depth1)]);
    f.switch_to_block(kept_row);
    let bi = slot_get(f, before, i);
    let j = f.ins().isub(i, bi);
    slot_put(f, to, i, j);
    let be = slot_get(f, before, e);
    let lost = f.ins().isub(be, bi);
    let size = f.ins().isub(size, lost);
    let (src, dst) = (NodeCol::row(f, rows, i), NodeCol::row(f, rows, j));
    let x = row_words(f, src);
    put_row_words(f, dst, x);
    crate::native_cpu_tape::put(f, size_c, j, size, w.size);
    if inner {
        let h = slot_get(f, head, i);
        slot_put(f, head, j, h);
    }
    f.ins().jump(place, &[BlockArg::from(i1), BlockArg::from(depth1)]);
    // tails(j, wide): each kept row's tail, from its last descendant now.
    f.switch_to_block(tails);
    let (j, wide) = (f.block_params(tails)[0], f.block_params(tails)[1]);
    let dn = slot_get(f, before, n);
    let kept_n = f.ins().isub(n, dn);
    let end = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, j, kept_n);
    let end = if with_tails { end } else { f.ins().iconst(types::I8, 1) };
    f.ins().brif(end, back, &[BlockArg::from(supers), BlockArg::from(wide)], tbody, &[]);
    f.switch_to_block(tbody);
    let size = crate::native_cpu_tape::get(f, size_c, j, w.size);
    let last = f.ins().iadd(j, size);
    let last = f.ins().iadd_imm(last, -1);
    let base = if w.from_start { j } else { last };
    let s = crate::native_cpu_tape::start(f, rows, base);
    let row_j = NodeCol::row(f, rows, j);
    let end_j = f.ins().uload32(flags, row_j, AT_END);
    let tl = f.ins().isub(end_j, s);
    let tl = f.ins().band_imm(tl, 0xffff_ffff);
    crate::native_cpu_tape::put(f, tail_c, j, tl, w.tail);
    let odd = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, tl, if w.tail == 2 { 0xffff } else { 0x7fff_ffff });
    let odd = f.ins().uextend(types::I64, odd);
    let odd = f.ins().imul_imm(odd, crate::native_cpu_tape::WIDE_TAIL);
    let wide = f.ins().bor(wide, odd);
    let j1 = f.ins().iadd_imm(j, 1);
    f.ins().jump(tails, &[BlockArg::from(j1), BlockArg::from(wide)]);
    // back(k, wide): the first supertype of a node from `from` on.
    f.switch_to_block(back);
    let (k, wide) = (f.block_params(back)[0], f.block_params(back)[1]);
    let look = f.create_block();
    f.ins().brif(k, look, &[], sback, &[BlockArg::from(k)]);
    f.switch_to_block(look);
    let k1 = f.ins().iadd_imm(k, -1);
    let s = slot_get(f, snode, k1);
    let tail = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, s, from);
    f.ins().brif(tail, back, &[BlockArg::from(k1), BlockArg::from(wide)], sback, &[BlockArg::from(k)]);
    f.switch_to_block(sback);
    let k = f.block_params(sback)[0];
    f.ins().jump(sstep, &[BlockArg::from(k), BlockArg::from(k), BlockArg::from(wide)]);
    // sstep(s, kept, wide).
    f.switch_to_block(sstep);
    let (s, kept, wide) = (f.block_params(sstep)[0], f.block_params(sstep)[1], f.block_params(sstep)[2]);
    let end = f.ins().icmp(IntCC::Equal, s, supers);
    f.ins().brif(end, done, &[BlockArg::from(kept), BlockArg::from(wide)], sbody, &[BlockArg::from(wide)]);
    f.switch_to_block(sbody);
    let wide = f.block_params(sbody)[0];
    let at = slot_get(f, snode, s);
    let a1 = f.ins().iadd_imm(at, 1);
    let (b0, b1) = (slot_get(f, before, at), slot_get(f, before, a1));
    let s1 = f.ins().iadd_imm(s, 1);
    let lost = f.ins().icmp(IntCC::NotEqual, b0, b1);
    f.ins().brif(lost, sstep, &[BlockArg::from(s1), BlockArg::from(kept), BlockArg::from(wide)], sput, &[BlockArg::from(wide)]);
    f.switch_to_block(sput);
    let wide = f.block_params(sput)[0];
    let place_at = slot_get(f, to, at);
    slot_put(f, snode, kept, place_at);
    let sk = slot_get(f, skind, s);
    slot_put(f, skind, kept, sk);
    let kept1 = f.ins().iadd_imm(kept, 1);
    f.ins().jump(sstep, &[BlockArg::from(s1), BlockArg::from(kept1), BlockArg::from(wide)]);
    f.switch_to_block(done);
    let (kept, wide) = (f.block_params(done)[0], f.block_params(done)[1]);
    let dn = slot_get(f, before, n);
    let nodes = f.ins().isub(n, dn);
    let lo = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, moved, from);
    let hi = f.ins().icmp(IntCC::UnsignedLessThan, moved, n);
    let inside = f.ins().band(lo, hi);
    let safe = f.ins().select(inside, moved, from);
    let m = slot_get(f, to, safe);
    let moved = f.ins().select(inside, m, moved);
    f.ins().return_(&[nodes, kept, moved, wide]);
}


pub fn compile(native: &Structure) -> Result<CompiledStream, Error> {
    compile_cut(native, &one_cut(native))
}

/// `compile`'s cut: every word in one cluster, at the configured speed.
pub(crate) fn one_cut(native: &Structure) -> crate::native_cpu_clusters::Cut {
    let speed = crate::native_cpu_codegen::options().optimize_tier2.load(std::sync::atomic::Ordering::Relaxed);
    crate::native_cpu_clusters::Cut { speed, held: HELD_IN_ONE.to_vec(), reload: true, ..crate::native_cpu_clusters::Cut::one(native.structural.len()) }
}

/// A cluster's function, lowered: its loop heads, the places it is entered at (each with its
/// block), its tail calls' blocks (each with the place it calls), and their number.
struct Lowered {
    func: cranelift_codegen::ir::Function,
    heads: Vec<(Block, Step)>,
    targets: Vec<(usize, Block)>,
    exits: std::collections::HashMap<Block, usize>,
    tails: usize,
}

/// Tier 2 lowered in the clusters of `cut` (`native_cpu_clusters`): one function a cluster.
/// One cluster is the one function `tempo_stream(ctx)`, its state in its stack frame. With
/// more, `tempo_stream` holds the state and calls the cluster of the first word; a cluster's
/// function takes `ctx` (and the state in it) and the index of the place it enters at, and a jump
/// to a word of another cluster is a tail call of that cluster (`return_call`, the `tail`
/// calling convention): no stack grows, and every function returns the run's result to
/// `tempo_stream`.
pub fn compile_cut(native: &Structure, cut: &crate::native_cpu_clusters::Cut) -> Result<CompiledStream, Error> {
    compile_at(native, cut, rerail_tape_format::Narrow::WIDTHS, native.tag)
}

/// `compile_cut`, the tape written at `widths` (constants of the code: docs/parser-generators/37)
/// with the language's tag `tag`.
pub fn compile_at(native: &Structure, cut: &crate::native_cpu_clusters::Cut, widths: rerail_tape_format::Widths, tag: u8) -> Result<CompiledStream, Error> {
    compile_with(native, cut, widths, tag, false)
}

/// `compile_at`; where `sixteen`, the run writes format 16 by the writer's ops (decision 0434 §6, R17): at each
/// commit point it passes the nodes before the last one to the ops (`native_cpu_tape::pass`), at `ACCEPT` the rest,
/// and it writes no format 15 image (no codes, no image's end). Its run takes the writer (`Parser::image16`).
pub fn compile_with(native: &Structure, cut: &crate::native_cpu_clusters::Cut, widths: rerail_tape_format::Widths, tag: u8, sixteen: bool) -> Result<CompiledStream, Error> {
    let widths_tag = (widths, tag);
    if cut.of.len() != native.structural.len() || cut.count == 0 || (native.structural.is_empty() && cut.count > 1) {
        return Err(Error::UnsupportedBackend);
    }
    let words = &native.structural;
    let pushes = push_targets(native);
    // Each word's phase (`stream::phases`): where every `TOKEN` knows whether it takes a token,
    // no flag says so, and a word after a `SPAN` reads its token's end from its record.
    let phase_of = phases(native, &pushes);
    let flagged = phase_of.contains(&(crate::stream::TAKEN | crate::stream::UNTAKEN));
    let taken = |pc: usize| !flagged && phase_of[pc] == crate::stream::TAKEN;
    let end_at = |pc: usize| if flagged { EndAt::Field } else if taken(pc) { EndAt::Taken } else { EndAt::Untaken };
    let named = |op: fn(&Op) -> Option<u16>| words.iter().find_map(|b| op(&b.operation)).map(usize::from);
    let starts = named(|o| if let Op::Token { extract, .. } = *o { Some(extract) } else { None });
    // The mask `SPAN m=1` names, as its output's index.
    // The records' last bytes, an extract (`records`).
    let lasts = starts.and(native.block_programs.first().and_then(|b| b.records.as_ref())).map(|r| usize::from(r.lasts));
    // Each kind's least byte (`records`): a switch on the first byte is a jump table on the kind.
    let kind_reps: Option<Vec<u8>> = starts.and(native.block_programs.first().and_then(|b| b.records.as_ref())).map(|r| {
        let n = r.kinds.iter().copied().max().map_or(0, |m| usize::from(m) + 1);
        (0..n).map(|k| r.kinds.iter().position(|&x| usize::from(x) == k).unwrap_or(0) as u8).collect()
    });
    // Each word's kind: its first byte's.
    // The stream's end as a kind of its own, past the records' (no word, no mark): a switch on
    // kinds takes it to its end's target, as the lean parser's end token, with no test of its
    // own. None where the kinds fill a byte.
    let end_kind: Option<u8> = kind_reps.as_ref().and_then(|r| u8::try_from(r.len()).ok());
    let word_kinds: Vec<u8> = starts.and(native.block_programs.first().and_then(|b| b.records.as_ref())).map_or_else(Vec::new, |r| r.words.iter().map(|w| r.kinds[usize::from(w[0])]).collect());
    // A switch's targets by kind, on the first byte with records (`records`): each kind's
    // target, its least byte's; the end's, where it is a kind (`end_kind`); a kind whose words
    // the switch names, `BY_WORD` and the kind, with the kind's own target beside it.
    let kind_targets = |s: &ByteSwitch, end_in_tree: Option<u8>| -> Option<(ByteSwitch, Vec<(usize, u16)>)> {
        let reps = kind_reps.as_ref().filter(|_| s.offset == 0)?;
        let mut by_word = Vec::new();
        let mut t = vec![REFUSE; 256];
        for (k, &b) in reps.iter().enumerate() {
            t[k] = s.targets[usize::from(b)];
        }
        if let Some(e) = end_in_tree {
            t[usize::from(e)] = s.end;
        }
        for (k, tk) in t.iter_mut().enumerate().take(reps.len()) {
            if s.words.iter().any(|&(w, _)| word_kinds.get(usize::from(w)).is_some_and(|&x| usize::from(x) == k)) {
                by_word.push((k, *tk));
                *tk = BY_WORD + k as u16;
            }
        }
        Some((ByteSwitch { targets: t, ..s.clone() }, by_word))
    };
    // Whether the `TOKEN` at `pc` is taken by a `SWITCH` on its first byte next, by kinds: where
    // tier 2 makes the records, that switch's edges make them (page 40).
    let lazy_switch = |pc: usize| -> bool {
        kind_reps.is_some()
            && !native.token_marks.iter().any(|m| m.refuses)
            && match native.structural.get(past_gotos(native, pc + 1)).map(|w| w.operation) {
                Some(Op::Switch { table }) => native.switches[usize::from(table)].offset == 0,
                _ => false,
            }
    };

    let clustered = cut.count > 1 || cut.calls;
    // `Cut::speed`: Cranelift's optimizer (`opt_level = speed`: value numbering, dead code,
    // redundant loads, invariants out of loops); by default it is off (`none`).
    let mut flags: Vec<(&str, &str)> = Vec::new();
    if cut.speed {
        flags.push(("opt_level", "speed"));
    }
    // With a function a grammar rule, `ctx` is in a register of its own through the run
    // (Cranelift's pinned register, x21): `tempo_stream` sets it (and gives the host's back),
    // each function reads it once where it is entered, into one value with no use fixed to an
    // argument's register (a call passes no `ctx`), which the allocator keeps in one
    // callee-saved register through the function, as rustc keeps the lean parser's `self`.
    let pinned_ctx = clustered && cut.calls && !native.rules.is_empty();
    // One function (`Cut::one`): `ctx` in the pinned register too, set where it is entered and
    // the host's given back where it leaves, and read from it at each use. As one value
    // through the function, the allocator split it around the cold calls and reloaded it from
    // the stack at each use (486 loads over JSON's, page 40).
    let pin_one = !clustered;
    if pinned_ctx || pin_one {
        flags.push(("enable_pinned_reg", "true"));
    }
    // The host's (`rerail_clif::jit::flags`): more results than registers through a return area (any function's
    // results past the target's registers, and the passed fields that come back with a function's result), and
    // x86-64's frame pointers.
    let flags = rerail_clif::jit::flags(&flags);
    let mut builder = if cut.unsigned {
        unsigned_builder(&flags)?
    } else {
        JITBuilder::with_flags(&flags, default_libcall_names()).map_err(|_| Error::BackendFailure)?
    };
    // `Cut::near`, or format 16's: the module's code in one arena, well within a direct branch's
    // 128 MB. The tape writer's ops (`writer::define_ops`) call each other directly, so a module
    // that defines them is one arena too, as every module that compiles the reads is
    // (`rerail_clif::jit::builder`): two chunks of the default provider may lie past a `bl`'s
    // reach (`compiled_blob`'s `Arm64Call`).
    if cut.near || sixteen {
        builder.memory_provider(Box::new(cranelift_jit::ArenaMemoryProvider::new_with_size(rerail_clif::jit::CODE_RESERVE).map_err(|_| Error::BackendFailure)?));
    }
    builder.symbol(TABLES, std::ptr::null());
    let mut module = JITModule::new(builder);
    if module.declare_data(TABLES, Linkage::Import, false, false).map_err(|_| Error::BackendFailure)?.as_u32() != TABLES_DATA {
        return Err(Error::BackendFailure);
    }
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer));
    signature.returns.push(AbiParam::new(types::I64));
    let id = module.declare_function("tempo_stream", Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
    // The calls open (`Mem::Ret`) are a field of the state in `ctx`, read and written where a
    // call is made and returns, and read by `TOKEN`'s room check, as the lean parser's
    // `self` fields are: no value of them lives across a call. Passed as an argument, one value
    // through a whole function, given to each call and to the code of other rules it goes
    // into, was a value regalloc2 split twice at most and then cut into a piece a use, each
    // reloaded from the stack (1,148 loads over `.rerail`'s tier 2, against 17 so).
    // A cluster's function: (ctx, the entry's index), the `tail` convention.
    let csig = if clustered {
        let mut c = module.make_signature();
        c.call_conv = cranelift_codegen::isa::CallConv::Tail;
        // `ctx` in the pinned register (`pinned_ctx`), else the first argument.
        if !pinned_ctx {
            c.params.push(AbiParam::new(pointer));
        }
        c.params.push(AbiParam::new(types::I64));
        // `Cut::pass`: those fields as arguments, in registers, and given back with the result,
        // as a call's arguments and results: a call stores none of them.
        c.params.extend(cut.pass.iter().map(|_| AbiParam::new(types::I64)));
        c.returns.push(AbiParam::new(types::I64));
        c.returns.extend(cut.pass.iter().map(|_| AbiParam::new(types::I64)));
        c
    } else {
        signature.clone()
    };
    let cids: Vec<cranelift_module::FuncId> = if clustered {
        (0..cut.count).map(|c| module.declare_function(&format!("tempo_stream_c{c}"), Linkage::Local, &csig).map_err(|_| Error::BackendFailure)).collect::<Result<_, _>>()?
    } else {
        vec![id]
    };
    let mut fctx = FunctionBuilderContext::new();
    // The node words' rare loops, outlined (`TableFns`).
    let classes = supertype_classes(native)?;
    let image = crate::native_cpu_tape::Image { tag: widths_tag.1, widths: widths_tag.0, extras: &native.readings.extras, classes: &classes };
    // A format 16 module is one arena (above), so its ops call each other directly.
    let table_fns = TableFns::make(&mut module, &native.structural, image, sixteen, cut.near || sixteen)?;
    // A region `TOKEN` splits (decision 0323, lowering B).
    let split = split_parts(native)?;
    // The records `TOKEN` makes from each start (page 40).
    let fetch = fetch_of(native)?;
    let direct = if sixteen { crate::stream_facts::direct_keys(native, widths_tag.0, widths_tag.1) } else { None };
    let table = Table { inner: crate::stream_facts::marks(native), w: widths_tag.0, fns: &table_fns, split: split.as_deref().zip(native.block_programs.first()), fetch: fetch.as_deref(), direct: direct.as_deref(), padded: native.padded };
    let passes: Vec<u32> = table_fns.shift.iter().chain(&table_fns.drop).map(|f| f.0.as_u32()).collect();
    let row_fns: Vec<u32> = [&table_fns.blocks, &table_fns.commit, &table_fns.uncode, &table_fns.finish].into_iter().chain(table_fns.sixteen.iter().flat_map(|s| [&s.pass, &s.retag])).map(|f| f.0.as_u32()).collect();
    // The words made as functions of their own (`WordFn`), with a function a grammar rule.
    let outlined = !native.rules.is_empty() && cut.calls;
    // The calls open counted by the machine's stack (`calls_on_stack`), with `ctx` pinned.
    let on_stack = pinned_ctx && outlined && !flagged && calls_on_stack(native);
    // Statements recover where the grammar says how (`Words::recover`, decision 0650): a refusal of a statement's
    // syntax, in a marked statement (`STATEMENT`), cut back to the mark and made one ERROR leaf, then the root's head
    // again (`restart`, the `TOKEN` of its first `STATEMENT`). A limit refuses the file (`limits`, a bit a refusal's
    // value: none, the depth, a count, a check, tier 1's, the file's end; past the words, out of room).
    let recovers = !native.recover.is_empty();
    let restart = words.iter().position(|w| matches!(w.operation, Op::Statement)).map_or(0, |p| p.saturating_sub(1));
    let mut limits = vec![0u64; if recovers { (words.len() + 2).div_ceil(64) } else { 0 }];
    if recovers {
        if sixteen || table.split.is_some() || table.fetch.is_some() {
            return Err(Error::UnsupportedFeature("statements that recover, in a format 16 run or with a region tier 2 splits or records it makes"));
        }
        if !matches!(words.get(restart).map(|w| w.operation), Some(Op::Token { .. })) || !flagged && phase_of[restart] != crate::stream::TAKEN {
            return Err(Error::UnsupportedFeature("statements that recover, with no root head that takes a token"));
        }
        if words.iter().any(|w| matches!(w.operation, Op::Leaf { kind, .. } | Op::Open { kind } | Op::Wrap { kind } if kind == crate::native_cpu_tape::ERROR_ROW)) {
            return Err(Error::UnsupportedFeature("a kind that is the ERROR row's"));
        }
        let mut limit = |v: usize| limits[v / 64] |= 1 << (v % 64);
        limit(0);
        if on_stack {
            limit(1);
        }
        for (pc, w) in words.iter().enumerate() {
            if matches!(w.operation, Op::Push { .. } | Op::Count { .. } | Op::Check { .. } | Op::Empty { .. } | Op::Accept) {
                limit(pc + 1);
            }
        }
    }
    let limits = limits.into_boxed_slice();
    let recovery = Recovery {
        recover: &native.recover,
        extra_bit: native.extras.first().map(|e| e.0),
        lines: native.block_programs.first().and_then(|b| b.newlines.filter(|_| recovers).map(|(o, _)| (o as u16, !b.cpu_unstored().contains(&o)))),
        block: native.block_programs.first(),
        limits: &limits,
        wrapping: framed(words),
    };
    // The callee takes its caller's token (`RETURN m=0`), as the lean parser's callee consumes
    // it and its caller peeks: where every call's place to return to is one `TOKEN`, alike
    // (the same mask, and taking a token there), a `RETURN m=0` takes it and returns as `m=1`
    // does, to the caller's switch. So every call returns one way and is tested once (`cbnz`).
    let landing_token: Option<(Option<u8>, bool)> = {
        let pushes = push_targets(native);
        let mut anys = pushes.iter().map(|&p| match words.get(usize::from(p)).map(|b| b.operation) {
            Some(Op::Token { any, extras, .. }) if phase_of[usize::from(p)] == crate::stream::TAKEN => Some((any, extras)),
            _ => None,
        });
        let first = anys.next().flatten();
        first.filter(|&a| on_stack && anys.all(|x| x == Some(a)))
    };
    let callee_takes = landing_token.is_some();
    // With a function a grammar rule, the ways a call returns (bit 0: `RETURN m=0`, bit 1:
    // `m=1`), by the place it enters: the `RETURN`s reached from there at the same depth
    // (`rule_successors`), in its function and in the code of earlier rules it goes into (which
    // it calls, passing their result on). A call of a place with one way tests once (`cbnz`), as
    // a `Result`'s: a `RETURN m=1` gives back 0 where every place reaching it has that way alone.
    // By place, not function: a rule's function holds code other rules go into, with ways of
    // its own.
    let rule_of = |pc: usize| native.rules.partition_point(|&s| usize::from(s) <= pc).saturating_sub(1);
    let by_rule = outlined && (0..words.len()).all(|pc| cut.of[pc] as usize == rule_of(pc));
    let n_structural = words.len();
    let entered: Vec<usize> = {
        let mut e: Vec<usize> = (0..n_structural).filter(|&pc| matches!(words[pc].operation, Op::Push { .. })).map(|pc| callee(native, pc)).collect();
        e.sort_unstable();
        e.dedup();
        e
    };
    // The `RETURN`s each entered place reaches.
    let returns_from: Vec<Vec<usize>> = entered
        .iter()
        .map(|&e| {
            let mut seen = vec![false; n_structural];
            let (mut todo, mut out) = (vec![e], Vec::new());
            while let Some(pc) = todo.pop() {
                if pc >= n_structural || std::mem::replace(&mut seen[pc], true) {
                    continue;
                }
                if matches!(words[pc].operation, Op::Return { .. }) {
                    out.push(pc);
                }
                todo.extend(rule_successors(native, pc));
            }
            out
        })
        .collect();
    let again_of = |r: usize| matches!(words[r].operation, Op::Return { again: true });
    let mut one: Vec<Option<u16>> = returns_from
        .iter()
        .map(|rs| {
            let ways = rs.iter().fold(0u8, |w, &r| w | 1 << u8::from(again_of(r)));
            if !by_rule { None } else { match ways { 1 => Some(0), 2 => Some(1), _ => None } }
        })
        .collect();
    // Each `RETURN m=1`'s places (the entered places reaching it).
    let mut users: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for (i, rs) in returns_from.iter().enumerate() {
        for &r in rs.iter().filter(|&&r| again_of(r)) {
            users.entry(r).or_default().push(i);
        }
    }
    // A `RETURN m=1` a place with both ways reaches gives back 1: then every place reaching it
    // tests for 1. To a fixed point.
    loop {
        let mut changed = false;
        for us in users.values() {
            if us.iter().any(|&u| one[u].is_none()) {
                for &u in us {
                    if one[u] == Some(1) {
                        one[u] = None;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    // A `RETURN m=1` gives back 0 where every place reaching it reads 0 so.
    let again_as_one: Vec<bool> = (0..n_structural).map(|r| users.get(&r).is_some_and(|us| us.iter().all(|&u| one[u] == Some(1)))).collect();
    // The one way a call of place `e` returns, where it has one.
    let one_way = |e: usize| -> Option<u16> { if callee_takes { Some(1) } else { entered.binary_search(&e).ok().and_then(|i| one[i]) } };
    let mut word_fns: std::collections::HashMap<WordFn, (cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function)> = std::collections::HashMap::new();
    let mut lowered: Vec<Lowered> = Vec::new();
    let mut all_scans: Vec<usize> = Vec::new();
    let mut all_onward: Vec<usize> = Vec::new();
    let mut all_fused: Vec<usize> = Vec::new();
    let mut all_static: Vec<usize> = Vec::new();
    let mut word_insts: Vec<u32> = vec![0; words.len()];
    let phase = std::time::Instant::now();
    let times = std::env::var_os("RERAIL_STREAM_TIMES").is_some();
    // The `TOKEN`s where a string's part may be pending (`parts_pending`); all, where a region
    // is read in parts with an escape carry.
    let pending_all = table.fetch.filter(|ft| ft.seq.iter().any(|&x| x) && table.split.is_some_and(|(sp, _)| sp.seq.is_some())).map(|ft| parts_pending(native, &pushes, ft));
    // With native calls, a function a rule: no arms shared across them.
    let shared_all = if cut.calls { SharedRuns { bodies: Vec::new(), at: Default::default(), ends: Default::default(), skipped: vec![false; words.len()] } } else { shared_runs(native, &pushes, pending_all.as_deref(), table.direct) };
    if times {
        eprintln!("tier 2: shared runs {:.2?}", phase.elapsed());
    }
    // Where each word can go on to: its successors, a token's next word, a shared body's end's
    // targets.
    let n_words = words.len();
    let goes_to = |pc: usize| -> Vec<usize> {
        let mut r = if cut.calls { rule_successors(native, pc) } else { successors(native, &pushes, pc) };
        if matches!(words[pc].operation, Op::Token { .. }) {
            r.push(pc + 1);
        }
        if let Some(&b) = shared_all.ends.get(&pc) {
            r.extend(shared_all.bodies[b].targets.iter().map(|&t| usize::from(t)));
        }
        // A shared run's place goes on into its body, not to its own next word (never lowered).
        r.retain(|&t| t < n_words && !shared_all.skipped[t]);
        r
    };
    // A place's cluster: a word's, or (past the words) a shared body's first place's.
    let cluster_of = |g: usize| (if g < n_words { cut.of[g] } else { cut.of[shared_all.bodies[g - n_words].start] }) as usize;
    // Each cluster's entries, before any is lowered: the first word, and every place of it
    // another cluster's word can go on to. A call names its entry by its index here.
    let mut incoming: Vec<std::collections::BTreeSet<usize>> = vec![std::collections::BTreeSet::new(); cut.count as usize];
    if clustered {
        incoming[cut.of[0] as usize].insert(0);
        // A statement recovered goes on at the root's head (`restart`): the driver calls it.
        if recovers {
            if shared_all.skipped[restart] {
                return Err(Error::UnsupportedFeature("statements that recover, the root's head in a shared run"));
            }
            incoming[cut.of[restart] as usize].insert(restart);
        }
        // A call's callee: an entry of its function, wherever it is called from.
        for pc in (0..n_words).filter(|&pc| cut.calls && matches!(words[pc].operation, Op::Push { .. })) {
            let e = callee(native, pc);
            if e < n_words {
                incoming[cut.of[e] as usize].insert(e);
            }
        }
        for pc in (0..n_words).filter(|&pc| !shared_all.skipped[pc]) {
            for t in goes_to(pc) {
                if cut.of[t] != cut.of[pc] {
                    incoming[cut.of[t] as usize].insert(t);
                }
            }
            if let Some(&(b, _)) = shared_all.at.get(&pc)
                && cut.of[shared_all.bodies[b].start] != cut.of[pc]
            {
                incoming[cluster_of(n_words + b)].insert(n_words + b);
            }
        }
    }
    let index: Vec<std::collections::HashMap<usize, usize>> = incoming.iter().map(|s| s.iter().enumerate().map(|(i, &g)| (g, i)).collect()).collect();
    // Each cluster's level may change its word or flags (`REMEMBER`, `FLAG` set or clear): in
    // its own words, or in a cluster it goes on to (a tail call); its own calls give them back.
    // A call keeps the caller's only where its callee's may change (`Op::Push`).
    let mut changes_context: Vec<bool> = vec![false; cut.count as usize];
    for pc in (0..n_words).filter(|&pc| matches!(words[pc].operation, Op::Remember { .. } | Op::Flag { op: FlagOp::Set | FlagOp::Clear, .. })) {
        changes_context[cut.of[pc] as usize] = true;
    }
    // With a function a grammar rule and `ctx` pinned (every exit through the function's one
    // `exit`), the word and flags are callee-saved, as registers are: a function whose own
    // words change them keeps the values it was entered with and gives them back where it
    // leaves, as the lean parser's are its callees' locals. A call keeps nothing; code of
    // another rule a function goes into keeps its own.
    let own_changes = changes_context.clone();
    let callee_keeps = pinned_ctx && has_context(words);
    loop {
        let mut more = false;
        for pc in (0..n_words).filter(|&pc| !shared_all.skipped[pc]) {
            let c = cut.of[pc] as usize;
            if changes_context[c] {
                continue;
            }
            if goes_to(pc).into_iter().any(|t| cut.of[t] as usize != c && changes_context[cut.of[t] as usize]) {
                changes_context[c] = true;
                more = true;
            }
        }
        if !more {
            break;
        }
    }
    for c in 0..cut.count {
        // The places this cluster's function names: its own words and every place they can go
        // on to. A block each, and no more: Cranelift's passes size their tables by the blocks.
        let mut named = vec![false; words.len()];
        for pc in (0..words.len()).filter(|&pc| cut.of[pc] == c && !shared_all.skipped[pc]) {
            named[pc] = true;
            let next = if cut.calls { rule_successors(native, pc) } else { successors(native, &pushes, pc) };
            for t in next.into_iter().chain([pc + 1, pc + 2]) {
                if t < words.len() {
                    named[t] = true;
                }
            }
            if let Some(&b) = shared_all.ends.get(&pc) {
                for &t in &shared_all.bodies[b].targets {
                    if usize::from(t) < words.len() {
                        named[usize::from(t)] = true;
                    }
                }
            }
        }
        if let Some(first) = named.first_mut() {
            *first |= cut.of[0] == c;
        }
        let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, cids[c as usize].as_u32()), csig.clone());
        let mut heads: Vec<(Block, Step)> = Vec::new();
        let foreign = std::cell::RefCell::new(std::collections::BTreeSet::new());
        let mut exits: std::collections::HashMap<Block, usize> = std::collections::HashMap::new();
        let mut targets: Vec<(usize, Block)> = Vec::new();
        let mut tails = 0usize;
        {
        let mut f = FunctionBuilder::new(&mut func, &mut fctx);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        let refuse = f.create_block();
        f.append_block_param(refuse, types::I64);
        let wb: Vec<Option<Block>> = named.iter().map(|&n| n.then(|| f.create_block())).collect();
        // A place past the words refuses, as the interpreter's `STOP`.
        let off_end = f.create_block();
        // A word's block: its own, in its cluster; else the block that calls its cluster (made
        // after the clusters are lowered, where the jump is kept). A block that only jumps costs
        // nothing: emission folds it (`CompiledStream::code_bytes`). Resolving the jumps here
        // instead made the code larger and slower (594 KB from 590 KB, 500M instructions from
        // 484M over the `.rerail` corpus).
        let goto = |pc: usize| -> Block {
            match wb.get(pc) {
                None => off_end,
                Some(&Some(b)) => {
                    if cut.of[pc] != c {
                        foreign.borrow_mut().insert(pc);
                    }
                    b
                }
                Some(None) => unreachable!("place {pc}: not among the places cluster {c} names"),
            }
        };
        let word = |t: u16| -> Block { if t == REFUSE { off_end } else { goto(usize::from(t)) } };
        // Where returns chain, each `RETURN` is a loop's head: a turn pops a call.
        let bound = bounds(native);
        let chains = bound.chains;
        let unchecked = bound.unchecked(marks_of(words));
        let pending = &pending_all;
        // Programs without `WRAP` or a `relabel` pay nothing for them: no frames, no chains, no
        // reorder (which runs only once a wrap was made).
        let wrapping = framed(words);
        let loses = words.iter().any(|b| matches!(b.operation, Op::LoseLast { .. }));

        f.switch_to_block(entry);
        // `ctx`: read from its register where the function is entered, else its first argument.
        let ctx = if pinned_ctx { f.ins().get_pinned_reg(types::I64) } else { f.block_params(entry)[0] };
        let host = pin_one.then(|| {
            let host = f.ins().get_pinned_reg(types::I64);
            f.ins().set_pinned_reg(ctx);
            host
        });
        let first_param = usize::from(!pinned_ctx);
        // `can_move`: a pure value to the optimizer, removed where unused (`Cut::speed`).
        let ro = MemFlags::trusted().with_readonly().with_can_move();
        let load = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, ro, ctx, at);
        let bytes = load(&mut f, BYTES);
        let len = load(&mut f, LEN);
        let starts_p = load(&mut f, STARTS);
        let nstarts = load(&mut f, NSTARTS);
        let lasts_p = load(&mut f, LASTS);
        let codes_p = load(&mut f, CODES);
        let lasts_stride = load(&mut f, LASTS_STRIDE);
        let totals_p = load(&mut f, TOTALS);
        let (output_stride, totals_stride, totals_mask) = (load(&mut f, OUTPUT_STRIDE), load(&mut f, TOTALS_STRIDE), load(&mut f, TOTALS_MASK));
        let rooms = [load(&mut f, ROOM_NODES), load(&mut f, ROOM_SUPERS), load(&mut f, ROOM_PUSHES)];
        let outputs_p = load(&mut f, OUTPUTS);
        let cols: Vec<Value> = [ROWS, SUPER_NODE, SUPER_KIND, OPEN, RETURNS, COUNTS].iter().map(|&at| load(&mut f, at)).collect();
        let (rows_v, snode_c, skind_c, open_c, ret_c, counts_c) = (cols[0], cols[1], cols[2], cols[3], cols[4], cols[5]);
        // The table's final pass, shared by every `ACCEPT`: (its rows, records).
        let fin = f.create_block();
        for _ in 0..2 {
            f.append_block_param(fin, types::I64);
        }
        let mut fin_used = false;
        let wcols: Vec<Value> = if wrapping {
            [FRAME_BASE, FRAME_LAST, FRAME_LAST_BASE, HEAD, LINK, PLACE, OUT_ROWS].iter().map(|&at| load(&mut f, at)).collect()
        } else {
            Vec::new()
        };
        let _ = &wcols;
        let (fbase_c, head_c, link_c) = (0, 3, 4);
        let zero = f.ins().iconst(types::I64, 0);
        // The state few words touch, in memory (`Mem`): not carried through every block where
        // paths meet, as an SSA variable is.
        // In clusters, the state is `tempo_stream`'s, in `ctx`.
        let slot = (!clustered).then(|| f.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(cranelift_codegen::ir::StackSlotKind::ExplicitSlot, 8 * Mem::COUNT, 3)));
        // The state's address: `ctx`, every cluster's first argument, as the lean parser's
        // `&mut self` (one register the allocator keeps, no move a block).
        // In one function, read from the pinned register at each use (`pin_one`).
        let pinned = |f: &mut FunctionBuilder<'_>| -> Value { if pin_one { f.ins().get_pinned_reg(types::I64) } else { ctx } };
        // The state's fields the cut holds in registers (`Cut::held`): SSA variables within the
        // function, loaded where it is entered and stored where it calls another cluster.
        let vars: Vec<Option<cranelift_frontend::Variable>> = (0..Mem::COUNT).map(|k| cut.held.contains(&k).then(|| f.declare_var(types::I64))).collect();
        // A field no word can make other than 0, a constant: `Pending` where no leaf is
        // deferred (no extras), as a program without comments has no comment to keep.
        let never_pending = native.extras.is_empty() && !words.iter().any(|w| matches!(w.operation, Op::Leaf { deferred: true, .. } | Op::Leaf { extra: true, .. }));
        let constant = |k: Mem| never_pending && matches!(k, Mem::Pending);
        let mget = |f: &mut FunctionBuilder<'_>, k: Mem| match (vars[k as usize], slot) {
            _ if constant(k) => f.ins().iconst(types::I64, 0),
            (Some(v), _) => f.use_var(v),
            (None, Some(s)) => f.ins().stack_load(types::I64, s, 8 * k as i32),
            (None, None) => {
                let p = pinned(f);
                f.ins().load(types::I64, MemFlags::trusted(), p, STATE + 8 * k as i32)
            }
        };
        let mset = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| match (vars[k as usize], slot) {
            _ if constant(k) => {}
            (Some(v), _) => f.def_var(v, x),
            (None, Some(s)) => {
                f.ins().stack_store(x, s, 8 * k as i32);
            }
            (None, None) => {
                let p = pinned(f);
                f.ins().store(MemFlags::trusted(), x, p, STATE + 8 * k as i32);
            }
        };
        // At the stream's end: `Next` past the count (the end's token sets it one past).
        let ended_now = |f: &mut FunctionBuilder<'_>, next: Value| -> Value {
            let b = pinned(f);
            let n = f.ins().load(types::I64, MemFlags::trusted(), b, NSTARTS);
            f.ins().icmp(IntCC::UnsignedGreaterThan, next, n)
        };
        // The token's kind and word, read from its code in memory as the lean parser reads a
        // token's (`ldrb` of the kind; the word's bytes after it): no shift or mask a test, which
        // Cranelift would make again in every block that tests (`band` with a constant is
        // rematerialized).
        let in_memory = vars[Mem::Taken as usize].is_none() && slot.is_none();
        let token_kind = |f: &mut FunctionBuilder<'_>| -> Value {
            if in_memory {
                f.ins().uload8(types::I64, MemFlags::trusted(), ctx, STATE + 8 * Mem::Taken as i32)
            } else {
                let code = mget(f, Mem::Taken);
                f.ins().band_imm(code, 0xff)
            }
        };
        let token_word = |f: &mut FunctionBuilder<'_>| -> Value {
            if in_memory {
                // The code's second byte (`records::word_of`), as the lean parser loads a
                // token's word.
                const _: () = assert!(crate::records::KIND_BITS == 8);
                f.ins().uload8(types::I64, MemFlags::trusted(), ctx, STATE + 8 * Mem::Taken as i32 + 1)
            } else {
                let code = mget(f, Mem::Taken);
                let w = f.ins().ushr_imm(code, i64::from(crate::records::KIND_BITS));
                f.ins().band_imm(w, 0xff)
            }
        };
        // The marks the records hold (`Records::marks`): mark `k` below `in_code` is bit `k` of
        // the code's top half. At a token, `Live` is those bits (each mark that holds there),
        // the others all set (tested by their masks); at the stream's end, the bits of the
        // marks the end has. A mark the record holds is then one bit test.
        let in_code = native.block_programs.first().and_then(|b| b.records.as_ref()).map_or(0, |r| r.mark_bits());
        let alone_bits: i64 = native.block_programs.first().and_then(|b| b.records.as_ref()).map_or(0, |r| i64::from(r.alone));
        let mut end_bits: i64 = (if in_code >= 64 { 0 } else { -1i64 << in_code }) | alone_bits;
        for b in words {
            if let Op::Mark { order, end: true, .. } = b.operation
                && usize::from(order) < in_code
            {
                end_bits |= 1 << order;
            }
        }
        // The gap marks tier 2 tests itself (`lazy_marks`): live at every token, each tested
        // where a state asks.
        let lazy: Vec<(u8, u16)> = native.block_programs.first().and_then(|b| b.records.as_ref().map(|r| (b, r))).map_or(Vec::new(), |(b, r)| {
            let bw = crate::native_cpu_bytes::Bytewise::of(b);
            crate::native_cpu_block::lazy_marks(r, &|o| bw.can(o))
        });
        let lazy_bits: i64 = lazy.iter().fold(0, |a, &(k, _)| a | 1 << k);
        // The code's token marks with no bit (`Records::alone`): live at every token, as past it.
        let rest_bits: i64 = (if in_code >= 64 { 0 } else { -1i64 << in_code }) | lazy_bits | alone_bits;
        let one = f.ins().iconst(types::I64, 1);
        let before_first = f.ins().iconst(types::I64, -1);
        let pick = f.create_block();
        // A rule's function's results: the value, then the passed fields (`Cut::pass`).
        // `end`: `v` is the run's end, its top bit flipped here (`ENDED`).
        let results = |f: &mut FunctionBuilder<'_>, v: Value, end: bool| -> Vec<Value> {
            let v = if clustered && end { f.ins().bxor_imm(v, ENDED as i64) } else { v };
            let mut r = vec![v];
            if clustered {
                r.extend(cut.pass.iter().map(|&k| f.use_var(vars[k as usize].expect("a passed field is held"))));
            }
            r
        };
        // The function's one exit, as the lean parser's functions have one epilogue: every
        // return (a rule's, a call's run end passed on, a refusal, the table's end) jumps here
        // with its results. A return each would be an epilogue each (its callee-saved
        // registers restored).
        let exit = f.create_block();
        // The word and flags this function gives back where it leaves (`callee_keeps`).
        let mut entry_kept: Option<(Value, Value)> = None;
        for _ in 0..1 + if clustered { cut.pass.len() } else { 0 } {
            f.append_block_param(exit, types::I64);
        }
        let leave = |f: &mut FunctionBuilder<'_>, r: &[Value]| {
            let args: Vec<BlockArg> = r.iter().map(|&v| BlockArg::from(v)).collect();
            f.ins().jump(exit, &args);
        };
        if clustered {
            for (k, v) in vars.iter().enumerate() {
                if let Some(v) = *v {
                    let x = match cut.pass.iter().position(|&q| q as usize == k) {
                        Some(j) => f.block_params(entry)[first_param + 1 + j],
                        None => {
                            let p = pinned(&mut f);
                            f.ins().load(types::I64, MemFlags::trusted(), p, STATE + 8 * k as i32)
                        }
                    };
                    f.def_var(v, x);
                }
            }
            // The word and flags the function was entered with, where its own words change them
            // (`callee_keeps`).
            if callee_keeps && own_changes[c as usize] {
                entry_kept = Some((mget(&mut f, Mem::Word), mget(&mut f, Mem::Flags)));
            }
            // The calls open on the machine's stack (`on_stack`): its depth checked once, where
            // the function is entered, as a stack check in a prologue (one load, one compare).
            if on_stack {
                let sp = f.ins().get_stack_pointer(types::I64);
                let limit = f.ins().load(types::I64, MemFlags::trusted(), ctx, STACK_LIMIT);
                let deep = f.ins().icmp(IntCC::UnsignedLessThan, sp, limit);
                let (ok, no) = (f.create_block(), f.create_block());
                f.ins().brif(deep, no, &[], ok, &[]);
                f.switch_to_block(no);
                f.set_cold_block(no);
                let at = f.ins().iconst(types::I64, 1);
                f.ins().jump(refuse, &[BlockArg::from(at)]);
                f.switch_to_block(ok);
            }
            f.ins().jump(pick, &[]);
        } else {
            for k in 0..Mem::COUNT {
                f.ins().stack_store(zero, slot.expect("a slot"), 8 * k as i32);
                if let Some(v) = vars[k as usize] {
                    f.def_var(v, zero);
                }
            }
            if flagged {
                mset(&mut f, Mem::Need, one);
            }
            mset(&mut f, Mem::Lw, before_first);
            if recovers {
                f.ins().store(MemFlags::trusted(), zero, ctx, MARK);
                f.ins().store(MemFlags::trusted(), zero, ctx, MARK_HELD);
            }
            match wb.first().copied().flatten() {
                Some(b) => f.ins().jump(b, &[]),
                None => f.ins().jump(off_end, &[]),
            };
        }

        let flags = MemFlags::trusted();
        // The other functions this one calls or tail-calls, each imported once.
        let mut call_refs: std::collections::HashMap<usize, cranelift_codegen::ir::FuncRef> = std::collections::HashMap::new();
        // Refuse at `pc` when `bad`; else go on in a new block.
        // The refusal's place is set in a cold block of its own, not on the hot path.
        let guard = |f: &mut FunctionBuilder<'_>, bad: Value, pc: usize| {
            let (ok, no) = (f.create_block(), f.create_block());
            f.ins().brif(bad, no, &[], ok, &[]);
            f.switch_to_block(no);
            f.set_cold_block(no);
            let at = f.ins().iconst(types::I64, pc as i64 + 1);
            f.ins().jump(refuse, &[BlockArg::from(at)]);
            f.switch_to_block(ok);
        };
        // A kind as a word names it: its level's word's (`REMEMBERED`, `REMEMBER`), refused where
        // the level has none; else the kind.
        let two_slots = has_context(words);
        let kind_of = |f: &mut FunctionBuilder<'_>, kind: u16, pc: usize| {
            if kind == REMEMBERED {
                let w = mget(f, Mem::Word);
                let none = f.ins().icmp_imm(IntCC::Equal, w, 0);
                guard(f, none, pc);
                w
            } else {
                f.ins().iconst(types::I64, i64::from(kind))
            }
        };
        // Column `c`'s slot `i` (u32s).
        let slot = |f: &mut FunctionBuilder<'_>, c: Value, i: Value| {
            let off = f.ins().ishl_imm(i, 2);
            f.ins().iadd(c, off)
        };
        let put = |f: &mut FunctionBuilder<'_>, c: Value, i: Value, x: Value| {
            let at = slot(f, c, i);
            f.ins().istore32(flags, x, at, 0);
        };

        // Shared arms (`shared_runs`): each run's code is made once, at its first place; every
        // place enters it with the run's next place (an index into its targets) and the shift
        // of its words from the first place's, for a refusal's word.
        let shared = &shared_all;
        let entries: Vec<Block> = shared.bodies.iter().map(|_| f.create_block()).collect();
        // A body's entry: its own, where its first place is in this cluster; else the call of its
        // cluster.
        let body_entry = |b: usize| -> Block {
            if cut.of[shared.bodies[b].start] != c {
                foreign.borrow_mut().insert(words.len() + b);
            }
            entries[b]
        };
        // The token's check a switch's edge can make itself (`fused_check`'s caller): where its
        // target is a run's, the target's words a token's `SPAN`s then a `CHECK` over the token's
        // bytes alone (none back, no gate), the run's class a byte set. Its place, the check,
        // and the bytes not in the class (the run's end).
        // Place `t`'s words a token's `SPAN`s then a `CHECK` over its bytes alone (none back, no
        // gate): the check's place and the check.
        let check_at = |t: usize| -> Option<(usize, crate::program::LocalCheck)> {
            if flagged || clustered {
                return None;
            }
            let mut q = t;
            while matches!(words.get(q).map(|w| w.operation), Some(Op::Span { last: true, .. })) {
                q += 1;
            }
            let Some(Op::Check { check }) = words.get(q).map(|w| w.operation) else { return None };
            let c = native.local_checks.get(usize::from(check))?.clone();
            if !c.token || c.back != 0 || token_gate(&c).is_some() || c.delay >= 15 || q + 1 >= words.len() {
                return None;
            }
            Some((q, c))
        };
        let fused_check = |t: usize, rule: u8, ft: &Fetch| -> Option<(usize, crate::program::LocalCheck, [u64; 4])> {
            let class = ft.run_class.get(usize::from(rule)).and_then(Option::as_ref)?;
            let (q, c) = check_at(t)?;
            let mut not_c = [0u64; 4];
            for (b, &x) in class.iter().enumerate() {
                if x == 0 {
                    not_c[b / 64] |= 1 << (b % 64);
                }
            }
            Some((q, c, not_c))
        };
        // On to place `p`'s word, from the current block: its own, or inside a shared run, the
        // body's word at the same offset, its place's target index and shift set as its entry
        // sets them.
        let jump_to_place = |f: &mut FunctionBuilder<'_>, p: usize| {
            let run = shared.at.iter().find(|&(&r, &(b, _))| r <= p && p <= r + (shared.bodies[b].end - shared.bodies[b].start)).map(|(&r, &x)| (r, x));
            match run {
                Some((r, _)) if r == p => {
                    f.ins().jump(wb[p].expect("its own place"), &[]);
                }
                Some((r, (b, ti))) => {
                    let body = &shared.bodies[b];
                    if body.targets.len() > 1 {
                        let i = f.ins().iconst(types::I64, ti as i64);
                        mset(f, Mem::Sel, i);
                    }
                    if r != body.start {
                        let sh = f.ins().iconst(types::I64, r as i64 - body.start as i64);
                        mset(f, Mem::Shift, sh);
                    }
                    f.ins().jump(wb[body.start + (p - r)].expect("the body's word"), &[]);
                }
                None => {
                    f.ins().jump(goto(p), &[]);
                }
            }
        };
        let mut marks_at: Vec<(usize, usize)> = Vec::new();
        for (pc, w) in words.iter().enumerate() {
            if shared.skipped[pc] || cut.of[pc] != c {
                continue;
            }
            // Each word's instructions, for the profile (`CompiledStream::word_insts`).
            marks_at.push((pc, f.func.dfg.num_insts()));
            // The word's place as its instructions' source location: its machine code's bytes
            // (`CompiledStream::word_bytes`).
            f.set_srcloc(cranelift_codegen::ir::SourceLoc::new(pc as u32 + 1));
            if let Some(&(body, ti)) = shared.at.get(&pc) {
                let first = shared.bodies[body].start;
                f.switch_to_block(wb[pc].expect("its own place"));
                // The target's index, where the body's end picks among more than one; the
                // shift, where it is not 0 (outside a shared run it is: every end sets it so).
                if shared.bodies[body].targets.len() > 1 {
                    let i = f.ins().iconst(types::I64, ti as i64);
                    mset(&mut f, Mem::Sel, i);
                }
                if pc != first {
                    let sh = f.ins().iconst(types::I64, pc as i64 - first as i64);
                    mset(&mut f, Mem::Shift, sh);
                }
                f.ins().jump(body_entry(body), &[]);
                if pc != first {
                    continue;
                }
                f.switch_to_block(entries[body]);
            } else {
                f.switch_to_block(wb[pc].expect("its own place"));
            }
            if let Some(&body) = shared.ends.get(&pc) {
                // The run's end: its place's next word, by the index the place gave; the shift
                // back to 0, where a place other than the first enters.
                if shared.at.iter().any(|(&p, &(b, _))| b == body && p != shared.bodies[body].start) {
                    let zero = f.ins().iconst(types::I64, 0);
                    mset(&mut f, Mem::Shift, zero);
                }
                let targets = &shared.bodies[body].targets;
                if let [t] = targets.as_slice() {
                    f.ins().jump(word(*t), &[]);
                } else {
                    let i = mget(&mut f, Mem::Sel);
                    let i = f.ins().ireduce(types::I32, i);
                    let calls: Vec<_> = targets.iter().map(|&t| f.func.dfg.block_call(word(t), &[])).collect();
                    let default = f.func.dfg.block_call(off_end, &[]);
                    let jt = f.create_jump_table(cranelift_codegen::ir::JumpTableData::new(default, &calls));
                    f.ins().br_table(i, jt);
                }
                continue;
            }
            // `Cut::reload`: the `ctx` fields this word reads, loaded here, so none lives past
            // its word (its registers are the word's and the state's).
            let (bytes, len, starts_p, nstarts, lasts_p, codes_p, lasts_stride, totals_p, output_stride, totals_stride, totals_mask, rooms, outputs_p) =
                (bytes, len, starts_p, nstarts, lasts_p, codes_p, lasts_stride, totals_p, output_stride, totals_stride, totals_mask, rooms, outputs_p);
            let (rows_v, snode_c, skind_c, open_c, ret_c, counts_c) = (rows_v, snode_c, skind_c, open_c, ret_c, counts_c);
            let wcols_here: Vec<Value>;
            let (bytes, len, starts_p, nstarts, lasts_p, codes_p, lasts_stride, totals_p, output_stride, totals_stride, totals_mask, rooms, outputs_p, rows_here, snode_c, skind_c, open_c, ret_c, counts_c, wc) = {
                let mut reads = ctx_reads(&w.operation, wrapping);
                // No room read where none is checked (`Bounds::unchecked`).
                if unchecked {
                    reads.retain(|&at| ![ROOM_NODES, ROOM_SUPERS, ROOM_PUSHES].contains(&at));
                }
                let all = reads.is_empty() && cut.reload && cold_op(&w.operation);
                let get = |f: &mut FunctionBuilder<'_>, at: i32, old: Value| -> Value {
                    if cut.reload && (all || reads.contains(&at)) {
                        // Not read-only: Cranelift would merge it with the entry's load (one
                        // value again, live everywhere).
                        let b = pinned(f);
                        f.ins().load(types::I64, MemFlags::trusted(), b, at)
                    } else {
                        old
                    }
                };
                let v = (
                    get(&mut f, BYTES, bytes), get(&mut f, LEN, len), get(&mut f, STARTS, starts_p), get(&mut f, NSTARTS, nstarts), get(&mut f, LASTS, lasts_p), get(&mut f, CODES, codes_p),
                    get(&mut f, LASTS_STRIDE, lasts_stride), get(&mut f, TOTALS, totals_p), get(&mut f, OUTPUT_STRIDE, output_stride), get(&mut f, TOTALS_STRIDE, totals_stride), get(&mut f, TOTALS_MASK, totals_mask),
                    [get(&mut f, ROOM_NODES, rooms[0]), get(&mut f, ROOM_SUPERS, rooms[1]), get(&mut f, ROOM_PUSHES, rooms[2])], get(&mut f, OUTPUTS, outputs_p),
                    get(&mut f, ROWS, rows_v), get(&mut f, SUPER_NODE, snode_c), get(&mut f, SUPER_KIND, skind_c),
                    get(&mut f, OPEN, open_c), get(&mut f, RETURNS, ret_c), get(&mut f, COUNTS, counts_c),
                );
                wcols_here = if wrapping {
                    [FRAME_BASE, FRAME_LAST, FRAME_LAST_BASE, HEAD, LINK, PLACE, OUT_ROWS].iter().enumerate().map(|(i, &at)| get(&mut f, at, wcols[i])).collect()
                } else {
                    Vec::new()
                };
                let wc = |i: usize| wcols_here[i];
                (v.0, v.1, v.2, v.3, v.4, v.5, v.6, v.7, v.8, v.9, v.10, v.11, v.12, v.13, v.14, v.15, v.16, v.17, v.18, wc)
            };
            // The node rows' fields.
            let (kind_c, field_c, end_c) = (NodeCol::of(rows_here, AT_KIND), NodeCol::of(rows_here, AT_FIELD), NodeCol::of(rows_here, AT_END));
            let fall = goto(pc + 1);
            // What a word's body reads and writes here (`WordEnv`): the state's fields, `ctx`'s
            // columns as this word has them, its refusal at its place.
            let env_get = |f: &mut FunctionBuilder<'_>, k: Mem| mget(f, k);
            let env_set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| mset(f, k, x);
            let env_ctx = |f: &mut FunctionBuilder<'_>, at: i32| -> Value {
                match at {
                    ROWS => rows_here,
                    OPEN => open_c,
                    FRAME_BASE if wrapping => wc(fbase_c),
                    HEAD if wrapping => wc(head_c),
                    LINK if wrapping => wc(link_c),
                    _ => {
                        let c = pinned(f);
                        f.ins().load(types::I64, MemFlags::trusted(), c, at)
                    }
                }
            };
            let env_guard = |f: &mut FunctionBuilder<'_>, bad: Value| guard(f, bad, pc);
            let env = WordEnv { get: &env_get, set: &env_set, ctx: &env_ctx, guard: &env_guard };
            // A word's body: made here, or with a function a grammar rule, a function of its own
            // (`WordFn`) every place calls with its arguments, which gives back whether it refuses.
            macro_rules! word {
                ($key:expr, $args:expr, $refuses:expr, $body:expr) => {
                    word!($key, $args, $refuses, $body, fall)
                };
                ($key:expr, $args:expr, $refuses:expr, $body:expr, $fall:expr) => {{
                    let body = $body;
                    if outlined {
                        let key = $key;
                        let args: &[Value] = $args;
                        if !word_fns.contains_key(&key) {
                            let name = format!("tempo_word_{}", word_fns.len());
                            let made = outline(&mut module, &name, 1 + args.len(), usize::from($refuses), pinned_ctx, &table.fns.names(), |g, q, _| own_word(g, q, $refuses, &body))?;
                            word_fns.insert(key, made);
                        }
                        let (id, sig, _) = &word_fns[&key];
                        let c = pinned(&mut f);
                        let r = call_word(&mut f, *id, sig, pinned_ctx, c, args);
                        if $refuses {
                            guard(&mut f, r[0], pc);
                        }
                    } else {
                        body(&mut f, &env, $args);
                    }
                    f.ins().jump($fall, &[]);
                }};
            }
            // A node's field set by the next word (`NODE_FIELD` on the node just made), stored
            // with its kind (one 32-bit store of both), the field word passed: as a call's
            // argument, with a function a grammar rule.
            let with_field = |kind: u16| -> (i64, Block) {
                match words.get(pc + 1).map(|w| w.operation) {
                    Some(Op::NodeField { field, on: On::Here }) if outlined => (i64::from(kind) | i64::from(field) << 16, goto(pc + 2)),
                    _ => (i64::from(kind), fall),
                }
            };
            match w.operation {
                Op::Token { any, extras, .. } => {
                    // The extras: the first kind, and the later ones by their bits.
                    let (extra, more) = MoreExtras::of(&native.extras, extras)?;
                    // Any mark's mask (`TOKEN m=1+k`): its output.
                    let any_mark: Option<usize> = match any {
                        Some(k) => Some(native.block_programs.first().and_then(|b| b.masks.get(usize::from(k)).copied()).ok_or(Error::UnsupportedBackend)?),
                        None => None,
                    };
                    heads.push((wb[pc].expect("its own place"), Step::Token));
                    if outlined && !flagged {
                        if phase_of[pc] != crate::stream::TAKEN {
                            // No token to take: the state switches on the one it has.
                            f.ins().jump(fall, &[]);
                            continue;
                        }
                        let shape = TokenShape { bound, unchecked, in_code, rest_bits, end_bits, end_code: end_kind.map(i64::from), on_stack, extra, more, wrapping, lost: loses, recovers };
                        let (id, sig, _) = token_fn(&mut module, &mut word_fns, any_mark, shape, table, &native.token_marks, pinned_ctx)?;
                        let c = pinned(&mut f);
                        let open_calls: Vec<Value> = if on_stack { vec![] } else { vec![mget(&mut f, Mem::Ret)] };
                        let r = call_word(&mut f, *id, sig, pinned_ctx, c, &open_calls)[0];
                        // A refusal: past the stream's end at this word, or out of room.
                        let no = f.create_block();
                        f.ins().brif(r, no, &[], fall, &[]);
                        f.switch_to_block(no);
                        f.set_cold_block(no);
                        let past = f.ins().icmp_imm(IntCC::Equal, r, 1);
                        let here = f.ins().iconst(types::I64, pc as i64 + 1);
                        let at = f.ins().select(past, here, r);
                        f.ins().jump(refuse, &[BlockArg::from(at)]);
                        continue;
                    }
                    // The path from here to the next token word writes at most `bound`'s nodes,
                    // supertypes and returns: past the tape's room, the run stops for a bigger
                    // tape. Three compares a token, not one a write.
                    // None where the tape is the bound's from the start (`Bounds::unchecked`).
                    let open_calls = if unchecked { zero } else { mget(&mut f, Mem::Ret) };
                    // Out of room: one cold block, which sets the run's end.
                    let over = f.create_block();
                    let counts = if unchecked { vec![] } else { vec![mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Supers), mget(&mut f, Mem::Ret)] };
                    if unchecked {
                        // The block before is filled before the cold one is entered.
                        let fits = f.create_block();
                        f.ins().jump(fits, &[]);
                        f.switch_to_block(fits);
                    }
                    for (at, (room, (most, per_call))) in counts.into_iter().zip(rooms.into_iter().zip([(bound.nodes, bound.pop_nodes), (bound.supers, bound.pop_supers), (bound.pushes, 0)])) {
                        let at = if per_call > 0 {
                            let popped = f.ins().imul_imm(open_calls, per_call as i64);
                            f.ins().iadd(at, popped)
                        } else {
                            at
                        };
                        let reach = f.ins().iadd_imm(at, most as i64);
                        // A branch each, not the compares joined (`cset`, `orr`, `cbnz`).
                        let past = f.ins().icmp(IntCC::UnsignedGreaterThan, reach, room);
                        let fits = f.create_block();
                        f.ins().brif(past, over, &[], fits, &[]);
                        f.switch_to_block(fits);
                    }
                    let fits = f.current_block().expect("in a block");
                    f.switch_to_block(over);
                    f.set_cold_block(over);
                    let overflow = f.ins().iconst(types::I64, OVERFLOW as i64);
                    f.ins().jump(refuse, &[BlockArg::from(overflow)]);
                    f.switch_to_block(fits);
                    // A token whose bytes are not taken yet (after a mark's arm) is kept: by the
                    // word's phase, or where a `TOKEN` has both, by a flag.
                    if flagged {
                        let take = f.create_block();
                        let need = mget(&mut f, Mem::Need);
                        f.ins().brif(need, take, &[], fall, &[]);
                        f.switch_to_block(take);
                        mset(&mut f, Mem::Need, zero);
                    } else if phase_of[pc] != crate::stream::TAKEN {
                        f.ins().jump(fall, &[]);
                        let dead = f.create_block();
                        f.switch_to_block(dead);
                    }
                    // A token a refusing mark held, taken: its statement refused here (decision 0650).
                    if recovers {
                        let p = pinned(&mut f);
                        let held = f.ins().load(types::I64, MemFlags::trusted(), p, MARK_HELD);
                        guard(&mut f, held, pc);
                    }
                    let next = mget(&mut f, Mem::Next);
                    let end = if flagged { mget(&mut f, Mem::End) } else { mget(&mut f, Mem::Limit) };
                    mset(&mut f, Mem::LastEnd, end);
                    if loses {
                        lost_leaf(&mut f, &env, table, wrapping);
                    }
                    // The stream's end: once a file, a cold block (the lean parser's index past
                    // its tokens). The token's fields stay; every mark is live (a mark the end
                    // has is there).
                    let all = f.ins().iconst(types::I64, -1);
                    // A region tier 2 splits (decision 0323): its next part, where one is
                    // pending, as `take_token`'s.
                    let split = table.split.map(|(sp, block)| (sp, crate::native_cpu_bytes::Bytewise::of(block)));
                    let fetched = split.as_ref().map(|_| f.create_block());
                    if let (Some((sp, bw)), Some(fetched)) = (&split, fetched) {
                        if flagged {
                            return Err(Error::UnsupportedFeature("a region tier 2 splits, with a token's taking flagged"));
                        }
                        // Only where a part may be pending (`parts_pending`).
                        if pending.as_ref().is_none_or(|p| p[pc]) {
                            all_scans.push(pc);
                            // Each part's outcome straight to the switch's place for it, where
                            // the token's path does nothing a part needs (no extra, no refusing
                            // mark, every mark live), as `parse_string` branches once a part.
                            let after = match words.get(pc + 1).map(|w| w.operation) {
                                Some(Op::Switch { table: at }) if extra.is_none() && any_mark.is_none() && in_code == 0 && !native.token_marks.iter().any(|m| m.refuses) => {
                                    let s = &native.switches[usize::from(at)];
                                    kind_targets(s, end_kind.filter(|_| kind_reps.is_some() && s.offset == 0)).map(|(byk, by_word)| (s, byk, by_word))
                                }
                                _ => None,
                            };
                            let mut dests: Vec<(Block, u16)> = Vec::new();
                            let onward = match (&sp.seq, &after) {
                                (Some(seq), Some((_, byk, _))) => {
                                    let kind_of = |b: usize| (sp.tables.words[b / 8] >> (8 * (b % 8))) as u8;
                                    let of_bytes = |set: [u64; 4]| -> Vec<u8> { (0..256usize).filter(|&b| set[b / 64] >> (b % 64) & 1 == 1).map(kind_of).collect() };
                                    let mut dest = |kinds: Vec<u8>| -> Option<Block> {
                                        let mut ts: Vec<u16> = kinds.iter().map(|&k| byk.targets[usize::from(k)]).collect();
                                        ts.sort_unstable();
                                        ts.dedup();
                                        let &[t] = ts.as_slice() else { return None };
                                        let b = f.create_block();
                                        dests.push((b, t));
                                        Some(b)
                                    };
                                    let closer = dest(of_bytes(seq.xs));
                                    let content = dest(of_bytes(std::array::from_fn(|k| !seq.stops[k])));
                                    let part = seq.codes2.as_ref().and_then(|t| dest(t.iter().map(|&c| c as u8).collect()));
                                    if closer.is_some() && content.is_some() && part.is_some() {
                                        all_onward.push(pc);
                                    }
                                    Some(Onward { closer, content, part })
                                }
                                _ => None,
                            };
                            split_next(&mut f, &env, sp, bw, fetched, onward.as_ref(), words_read(native, &pushes, &word_kinds, pc + 1).as_deref(), &mut heads);
                            // Each outcome's place: the target, or the words' switch of a kind
                            // whose words the switch names; a refusal at the switch's place.
                            if let Some((s, _, by_word)) = &after {
                                let back = f.current_block().expect("in a block");
                                let no = f.create_block();
                                for &(b, t) in &dests {
                                    f.switch_to_block(b);
                                    mset(&mut f, Mem::Live, all);
                                    match by_word.iter().find(|&&(k, _)| t != REFUSE && BY_WORD + k as u16 == t) {
                                        Some(&(k, plain)) => {
                                            let w = token_word(&mut f);
                                            let mut ts = vec![plain; word_kinds.len() + 1];
                                            for &(x, to) in &s.words {
                                                if word_kinds.get(usize::from(x)).is_some_and(|&y| usize::from(y) == k) {
                                                    ts[usize::from(x) + 1] = to;
                                                }
                                            }
                                            let sw = ByteSwitch { targets: ts, ..ByteSwitch::refusing(0) };
                                            let r = runs(&sw, &|t| if t == REFUSE { no } else { word(t) });
                                            dispatch(&mut f, w, &r, 256);
                                        }
                                        None => {
                                            f.ins().jump(if t == REFUSE { no } else { word(t) }, &[]);
                                        }
                                    }
                                }
                                f.switch_to_block(no);
                                f.set_cold_block(no);
                                let at = f.ins().iconst(types::I64, pc as i64 + 2);
                                f.ins().jump(refuse, &[BlockArg::from(at)]);
                                f.switch_to_block(back);
                            }
                        }
                    }
                    // One compare a token (`Next` is one past the count once the end is
                    // taken): the end's token, or past it (refused).
                    let is_end = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, next, nstarts);
                    let (ends, last, token) = (f.create_block(), f.create_block(), f.create_block());
                    f.ins().brif(is_end, ends, &[], token, &[]);
                    f.switch_to_block(ends);
                    f.set_cold_block(ends);
                    let past = f.ins().icmp(IntCC::UnsignedGreaterThan, next, nstarts);
                    guard(&mut f, past, pc);
                    f.ins().jump(last, &[]);
                    f.switch_to_block(last);
                    f.set_cold_block(last);
                    let past = f.ins().iadd_imm(nstarts, 1);
                    mset(&mut f, Mem::Next, past);
                    let at_end_live = f.ins().iconst(types::I64, end_bits);
                    mset(&mut f, Mem::Live, at_end_live);
                    if let Some(k) = end_kind {
                        let code = f.ins().iconst(types::I64, i64::from(k));
                        mset(&mut f, Mem::Taken, code);
                    }
                    f.ins().jump(fall, &[]);
                    // The token's record (`records`): its first byte, last byte and code, from
                    // three columns; no select, no compare but the end's. The record pass checked
                    // the columns (the totals' flag).
                    f.switch_to_block(token);
                    // The records' stride (`records::RecordLayout`): columns or rows, tier 1's.
                    // Where tier 2 makes the records (page 40), the starts alone: a column, as
                    // simdjson's `structural_indexes` (`call` checks the stride).
                    let roff = if table.fetch.is_some() {
                        f.ins().ishl_imm(next, 2)
                    } else {
                        let rb = pinned(&mut f);
                        let stride = f.ins().load(types::I64, MemFlags::trusted().with_readonly().with_can_move(), rb, RECORD_STRIDE);
                        f.ins().imul(next, stride)
                    };
                    let load_at = |f: &mut FunctionBuilder<'_>, c: Value| {
                        let at = f.ins().iadd(c, roff);
                        f.ins().uload32(flags, at, 0)
                    };
                    // A token the `SWITCH` after takes on its first byte (page 40): its start
                    // alone here, as simdjson's `advance()`; each of the switch's edges makes the
                    // record of its own bytes (`Limit` 0 until then: unfetched).
                    let lazy = table.fetch.is_some() && lazy_switch(pc);
                    let (s, l, c) = match table.fetch {
                        Some(_) if lazy => {
                            let s = load_at(&mut f, starts_p);
                            let early = f.ins().icmp(IntCC::UnsignedLessThan, s, end);
                            let (no, ok) = (f.create_block(), f.create_block());
                            f.ins().brif(early, no, &[], ok, &[]);
                            f.switch_to_block(no);
                            f.set_cold_block(no);
                            let at = f.ins().iconst(types::I64, pc as i64 + 1);
                            f.ins().jump(refuse, &[BlockArg::from(at)]);
                            f.switch_to_block(ok);
                            let unfetched = f.ins().iconst(types::I64, -1);
                            (s, unfetched, zero)
                        }
                        // The record made here from its start (page 40): its last by its first
                        // byte's rule, its code by the record pass's tables; a start before the
                        // token before's end, or a region with no closer, refuses the file, as
                        // the record pass's order check.
                        Some(ft) => {
                            let s = load_at(&mut f, starts_p);
                            let bw = crate::native_cpu_bytes::Bytewise::of(native.block_programs.first().expect("a block program"));
                            let (l, bad) = fetch_last(&mut f, &env, ft, &bw, s, &mut heads);
                            let early = f.ins().icmp(IntCC::UnsignedLessThan, s, end);
                            let early = f.ins().uextend(types::I64, early);
                            let bad = f.ins().bor(bad, early);
                            let (no, ok) = (f.create_block(), f.create_block());
                            f.ins().brif(bad, no, &[], ok, &[]);
                            f.switch_to_block(no);
                            f.set_cold_block(no);
                            let at = f.ins().iconst(types::I64, pc as i64 + 1);
                            f.ins().jump(refuse, &[BlockArg::from(at)]);
                            f.switch_to_block(ok);
                            let bytes_v = (env.ctx)(&mut f, BYTES);
                            let k = crate::native_cpu_block::tables_symbols(&mut f, &ft.tables);
                            let n = f.ins().isub(l, s);
                            let len1 = f.ins().iadd_imm(n, 1);
                            let c = crate::native_cpu_block::plain_code(&mut f, &ft.tables, k, bytes_v, s, len1);
                            (s, l, c)
                        }
                        None => (load_at(&mut f, starts_p), load_at(&mut f, lasts_p), load_at(&mut f, codes_p)),
                    };
                    let l1 = f.ins().iadd_imm(l, 1);
                    let after = f.ins().iadd_imm(next, 1);
                    mset(&mut f, Mem::Start, s);
                    if flagged {
                        mset(&mut f, Mem::End, s);
                    }
                    mset(&mut f, Mem::Next, after);
                    mset(&mut f, Mem::Limit, l1);
                    mset(&mut f, Mem::Taken, c);
                    let (s, c) = match (&split, fetched) {
                        (Some((sp, bw)), Some(fetched)) => {
                            // A lazy token's region is entered at its switch's edge.
                            if !lazy {
                                let read = words_read(native, &pushes, &word_kinds, pc + 1);
                                match (&sp.seq, table.fetch) {
                                    // The token is the region's where its first byte's rule is
                                    // its reader's (that rule tested the opener).
                                    (Some(_), Some(ft)) => {
                                        let bytes = (env.ctx)(&mut f, BYTES);
                                        let at = f.ins().iadd(bytes, s);
                                        let b = f.ins().uload8(types::I64, MemFlags::trusted(), at, 0);
                                        let q = f.ins().ushr_imm(b, 6);
                                        let q = f.ins().ishl_imm(q, 3);
                                        let base = table_address(&mut f, ft.seq_bytes.as_ptr());
                                        let wa = f.ins().iadd(base, q);
                                        let w = f.ins().load(types::I64, MemFlags::trusted().with_readonly(), wa, 0);
                                        let sh = f.ins().band_imm(b, 63);
                                        let x = f.ins().ushr(w, sh);
                                        let opens = f.ins().band_imm(x, 1);
                                        split_enter_seq(&mut f, &env, sp, s, Some(opens), read.as_deref());
                                    }
                                    _ => split_enter(&mut f, &env, sp, bw, (s, l), read.as_deref(), &mut heads),
                                }
                            }
                            f.ins().jump(fetched, &[]);
                            f.switch_to_block(fetched);
                            (mget(&mut f, Mem::Start), mget(&mut f, Mem::Taken))
                        }
                        _ => (s, c),
                    };
                    // An extra (`TOKEN_EXTRAS`): its bytes taken and its leaf deferred, then this
                    // word again for the next token.
                    if let Some((bit, kind)) = extra {
                        let b = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT) + i64::from(bit));
                        let b = f.ins().band_imm(b, 1);
                        let (xb, on) = (f.create_block(), f.create_block());
                        f.ins().brif(b, xb, &[], on, &[]);
                        f.switch_to_block(xb);
                        if flagged {
                            mset(&mut f, Mem::End, l1);
                            let one = f.ins().iconst(types::I64, 1);
                            mset(&mut f, Mem::Need, one);
                        }
                        let k = more.kind(&mut f, c, kind);
                        let again = wb[pc].expect("its own place");
                        word!(WordFn::Leaf { deferred: true, extra: true, end: EndAt::Taken, by_word: false }, &[k], false, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| leaf_word(g, e, table, q[0], LeafShape { deferred: true, extra: true, end: EndAt::Taken }, wrapping), again);
                        f.switch_to_block(on);
                    }
                    // A token mark that refuses, at this token: refused at this word.
                    if native.token_marks.iter().any(|m| m.refuses) {
                        let refused = f.create_block();
                        refusal_tests(&mut f, &env, c, &native.token_marks, native.extras.first().map(|e| e.0), refused, &mut heads);
                        let on = f.current_block().expect("in a block");
                        f.switch_to_block(refused);
                        f.set_cold_block(refused);
                        if recovers {
                            // Held: the statement that takes it refuses (decision 0650).
                            let p = pinned(&mut f);
                            let one = f.ins().iconst(types::I64, 1);
                            f.ins().store(MemFlags::trusted(), one, p, MARK_HELD);
                            f.ins().jump(on, &[]);
                        } else {
                            let at = f.ins().iconst(types::I64, pc as i64 + 1);
                            f.ins().jump(refuse, &[BlockArg::from(at)]);
                        }
                        f.switch_to_block(on);
                    }
                    // The marks live at this token: every one; with any mark's mask
                    // (`TOKEN m=1+k`), none where no mark holds, so a state's marks fail at their
                    // first test, with no load.
                    let live = match any_mark {
                        _ if in_code > 0 => {
                            let marks = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT));
                            if rest_bits == 0 { marks } else { f.ins().bor_imm(marks, rest_bits) }
                        }
                        Some(output) => {
                            let w = f.ins().ushr_imm(s, 6);
                            let off = f.ins().imul_imm(output_stride, output as i64);
                            let base = f.ins().iadd(outputs_p, off);
                            let word = mask_word(&mut f, base, lasts_stride, w);
                            let sh = f.ins().band_imm(s, 63);
                            let bit = f.ins().ushr(word, sh);
                            let bit = f.ins().band_imm(bit, 1);
                            f.ins().ineg(bit)
                        }
                        None => all,
                    };
                    mset(&mut f, Mem::Live, live);
                    f.ins().jump(fall, &[]);
                }
                Op::Switch { table: switch_at } => {
                    let s = &native.switches[usize::from(switch_at)];
                    // On kinds, the end is a kind (`end_kind`): its target in the tree.
                    let end_in_tree = end_kind.filter(|_| kind_reps.is_some() && s.offset == 0);
                    if end_in_tree.is_none() {
                        let at_end = {
                            let next = mget(&mut f, Mem::Next);
                            ended_now(&mut f, next)
                        };
                        let (ended, taking) = (f.create_block(), f.create_block());
                        f.ins().brif(at_end, ended, &[], taking, &[]);
                        f.switch_to_block(ended);
                        if s.end == REFUSE {
                            let at = f.ins().iconst(types::I64, pc as i64 + 1);
                            f.ins().jump(refuse, &[BlockArg::from(at)]);
                        } else {
                            f.ins().jump(word(s.end), &[]);
                        }
                        f.switch_to_block(taking);
                    }
                    // On the first byte, with records: the record's kind (`records`), the
                    // switch's targets by kind (its least byte's), a compare tree as on bytes.
                    // A switch that names words: a kind whose words it names goes to a block of
                    // its own (a place past every word's, `BY_WORD` and the kind), which takes
                    // a token that is no word to the kind's target, else switches on the word.
                    // A word's kind is its first byte's, so a token of the kind is one of its
                    // words or none. Tokens that are no keyword pay one test, as a parser's
                    // keywords are its identifiers' case.
                    let (by_kind, by_word) = match kind_targets(s, end_in_tree) {
                        Some((b, w)) => (Some(b), w),
                        None => (None, Vec::new()),
                    };
                    let s = by_kind.as_ref().unwrap_or(s);
                    let kind_blocks: Vec<Block> = by_word.iter().map(|_| f.create_block()).collect();
                    let plain_of = |t: u16| by_word.iter().find(|&&(k, _)| BY_WORD + k as u16 == t).map(|&(_, p)| p);
                    let word = |t: u16| -> Block {
                        match by_word.iter().position(|&(k, _)| t != REFUSE && BY_WORD + k as u16 == t) {
                            Some(i) => kind_blocks[i],
                            None => word(t),
                        }
                    };
                    // Where tier 2 makes the records (page 40): an unfetched token (`Limit` 0, a
                    // lazy `TOKEN`'s) takes the switch on its first byte, as simdjson's `switch
                    // (*value)`; each edge, its bytes alike in target, end rule, kind and whether
                    // a word begins with them, makes the token's record for those bytes alone,
                    // then goes to its target. A fetched token (a region's part, the stream's
                    // end) takes the switch on its kind below.
                    if let (Some(ft), Some(byk)) = (table.fetch, by_kind.as_ref()) {
                        let limit = mget(&mut f, Mem::Limit);
                        let (bytepath, kindpath) = (f.create_block(), f.create_block());
                        f.ins().brif(limit, kindpath, &[], bytepath, &[]);
                        f.switch_to_block(bytepath);
                        let start = mget(&mut f, Mem::Start);
                        let addr = f.ins().iadd(bytes, start);
                        let b = f.ins().uload8(types::I64, flags, addr, 0);
                        // The key's last: a one-byte token that is a word, its byte its word (`{`,
                        // `,`): the word's code and target, taken at once, as simdjson's `case ','`.
                        let mut keys: Vec<(u16, u8, u8, bool, Option<(u16, u32)>)> = Vec::new();
                        let mut edge_of = vec![REFUSE; 256];
                        // A byte only a word the switch names can take (its kind's own target a
                        // refusal): refused at once where no such word begins with it, as
                        // simdjson's `array_continue` takes `,` and `]` and refuses the rest.
                        let named_from = |x: usize| s.words.iter().any(|&(y, _)| ft.words.get(usize::from(y)).is_some_and(|(w, _)| w.first() == Some(&(x as u8))));
                        for (x, edge) in edge_of.iter_mut().enumerate() {
                            let k = ft.kinds[x];
                            let t = byk.targets[usize::from(k)];
                            if t == REFUSE || (plain_of(t) == Some(REFUSE) && !named_from(x)) {
                                continue;
                            }
                            let one = ft.rules.get(usize::from(ft.ends[x])).is_none_or(|r| one_byte(ft, r));
                            let direct = plain_of(t).filter(|_| one && ft.worded[x]).and_then(|plain| {
                                let y = ft.words.iter().position(|(w, _)| w.as_slice() == [x as u8])?;
                                let to = s.words.iter().find(|&&(w, _)| usize::from(w) == y).map_or(plain, |&(_, to)| to);
                                Some((to, ft.codes1[x]))
                            });
                            let key = (t, ft.ends[x], k, ft.worded[x], direct);
                            let i = keys.iter().position(|&y| y == key).unwrap_or_else(|| {
                                keys.push(key);
                                keys.len() - 1
                            });
                            *edge = i as u16;
                        }
                        let edges: Vec<Block> = keys.iter().map(|_| f.create_block()).collect();
                        let no_e = f.create_block();
                        let sw = ByteSwitch { targets: edge_of.clone(), ..ByteSwitch::refusing(0) };
                        let r = runs(&sw, &|t| if t == REFUSE { no_e } else { edges[usize::from(t)] });
                        dispatch(&mut f, b, &r, 256);
                        f.switch_to_block(no_e);
                        f.set_cold_block(no_e);
                        let at = f.ins().iconst(types::I64, pc as i64 + 1);
                        f.ins().jump(refuse, &[BlockArg::from(at)]);
                        let bw = crate::native_cpu_bytes::Bytewise::of(native.block_programs.first().expect("a block program"));
                        let split_bw = table.split.map(|(sp, block)| (sp, crate::native_cpu_bytes::Bytewise::of(block)));
                        for (&(t, rule, k, worded, direct), &edge) in keys.iter().zip(&edges) {
                            f.switch_to_block(edge);
                            let start = mget(&mut f, Mem::Start);
                            // A run whose bytes begin words, its output a byte class: each word
                            // as simdjson's atom test (`str4ncmp(value, "true")`, then the byte
                            // after not in the run's class), the word's target taken at once;
                            // another token, the rule's run and the word table below.
                            let class = ft.run_class.get(usize::from(rule)).and_then(Option::as_ref).filter(|_| worded);
                            if let Some(class) = class {
                                let bytes_at: Vec<u8> = (0..256usize).filter(|&x| edge_of[x] != REFUSE && keys[usize::from(edge_of[x])] == (t, rule, k, worded, direct)).map(|x| x as u8).collect();
                                let cands: Vec<(usize, &(Vec<u8>, u32))> = ft
                                    .words
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, (w, _))| (1..=8).contains(&w.len()) && bytes_at.contains(&w[0]) && w.iter().all(|&b| class[usize::from(b)] == 1))
                                    // Where the kind's own target refuses, the words the switch names.
                                    .filter(|&(x, _)| plain_of(t) != Some(REFUSE) || s.words.iter().any(|&(y, _)| usize::from(y) == x))
                                    .collect();
                                if !cands.is_empty() {
                                    let at = f.ins().iadd(bytes, start);
                                    let raw = f.ins().load(types::I64, flags, at, 0);
                                    let ctab = table_address(&mut f, class.as_ptr());
                                    for (x, (w, code)) in cands {
                                        let n = w.len();
                                        let mask = if n == 8 { -1i64 } else { ((1u64 << (8 * n)) - 1) as i64 };
                                        let want = w.iter().rev().fold(0u64, |a, &b| a << 8 | u64::from(b)) as i64;
                                        let got = f.ins().band_imm(raw, mask);
                                        let same = f.ins().icmp_imm(IntCC::Equal, got, want);
                                        let (cand, other) = (f.create_block(), f.create_block());
                                        f.ins().brif(same, cand, &[], other, &[]);
                                        f.switch_to_block(cand);
                                        let after = f.ins().uload8(types::I64, flags, at, n as i32);
                                        let ca = f.ins().iadd(ctab, after);
                                        let goes_on = f.ins().uload8(types::I64, MemFlags::trusted().with_readonly(), ca, 0);
                                        let hit = f.create_block();
                                        f.ins().brif(goes_on, other, &[], hit, &[]);
                                        f.switch_to_block(hit);
                                        let l1 = f.ins().iadd_imm(start, n as i64);
                                        mset(&mut f, Mem::Limit, l1);
                                        let c = f.ins().iconst(types::I64, i64::from(*code));
                                        mset(&mut f, Mem::Taken, c);
                                        // The word's own target, where the switch names it.
                                        let to = match plain_of(t) {
                                            Some(plain) => s.words.iter().find(|&&(y, _)| usize::from(y) == x).map_or(plain, |&(_, to)| to),
                                            None => t,
                                        };
                                        // A check there over the token's bytes alone, of no
                                        // delay: the word's bytes decide it here, as simdjson
                                        // tests an atom by its bytes and nothing else.
                                        match check_at(usize::from(to)).filter(|(_, c)| c.delay == 0) {
                                            Some((q, c)) if to != REFUSE => {
                                                if !all_static.contains(&q) {
                                                    all_static.push(q);
                                                }
                                                if crate::program::token_value(&c, w, 0, w.len(), &|_| false) {
                                                    let at_q = f.ins().iconst(types::I64, q as i64 + 1);
                                                    f.ins().jump(refuse, &[BlockArg::from(at_q)]);
                                                } else {
                                                    jump_to_place(&mut f, q + 1);
                                                }
                                            }
                                            _ => {
                                                f.ins().jump(if to == REFUSE { no_e } else { word(to) }, &[]);
                                            }
                                        }
                                        f.switch_to_block(other);
                                    }
                                }
                            }
                            // A run whose target takes the token's check over its bytes next (past
                            // its `SPAN`s): both from one read of its first 16 bytes, as
                            // simdjson's `parse_number` validates while it finds the number's end
                            // (the first byte of no number, then `is_not_structural_or_whitespace`).
                            // Its end, the first byte past the start not in the run's class; the
                            // check, over the bytes to it. A token longer than the frame, as before.
                            let fused = fused_check(usize::from(t), rule, ft);
                            let (l, bad, checked) = match &fused {
                                Some((q, c, not_c)) => {
                                    if !all_fused.contains(q) {
                                        all_fused.push(*q);
                                    }
                                    // The check's ops and the run's end's byte set, one set of
                                    // classes of the bytes: the end's mask is the last op's.
                                    let mut both = c.clone();
                                    both.ops.push(crate::program::LocalOp::Bytes(*not_c));
                                    let form = crate::stream_facts::word_form(&both.ops).map_err(|_| Error::UnsupportedBackend)?;
                                    let len_v = (env.ctx)(&mut f, LEN);
                                    let at = f.ins().iadd(bytes, start);
                                    let v = f.ins().load(types::I8X16, MemFlags::trusted(), at, 0);
                                    let consts = crate::native_cpu_block::Consts::make(&mut f);
                                    let file_left = f.ins().isub(len_v, start);
                                    let mut tables = std::collections::HashMap::new();
                                    let made = token_sets(&mut f, &both, &form, &consts, v, &mut tables);
                                    let Some(&ends) = made.get(&c.ops.len()) else { return Err(Error::UnsupportedFeature("a run's end with no byte set")) };
                                    // A last byte the frame holds the check at: `n <= 16 - delay`.
                                    let most = 16 - u32::from(c.delay);
                                    let ends = f.ins().band_imm(ends, (((1u64 << (most + 1)) - 1) & !1) as i64);
                                    let (short, long, join) = (f.create_block(), f.create_block(), f.create_block());
                                    for _ in 0..3 {
                                        f.append_block_param(join, types::I64);
                                    }
                                    f.ins().brif(ends, short, &[], long, &[]);
                                    f.switch_to_block(short);
                                    let n = f.ins().ctz(ends);
                                    let l = f.ins().iadd(start, n);
                                    let l = f.ins().iadd_imm(l, -1);
                                    // The check over places 0 to `n - 1` (and its delay), as
                                    // `token_check_short`'s.
                                    let vals = token_window(&mut f, &both, &form, &consts, &[v], None, n, file_left, None, None, None, Some(made));
                                    let one = f.ins().iconst(types::I64, 1);
                                    let below = f.ins().ishl(one, n);
                                    let valid = f.ins().iadd_imm(below, -1);
                                    let valid = if c.delay == 0 { valid } else { f.ins().ishl_imm(valid, i64::from(c.delay)) };
                                    let hit = f.ins().band(vals[c.ops.len() - 1], valid);
                                    let (passed, failed) = (f.create_block(), f.create_block());
                                    f.ins().brif(hit, failed, &[], passed, &[]);
                                    f.switch_to_block(failed);
                                    f.set_cold_block(failed);
                                    let at_q = f.ins().iconst(types::I64, *q as i64 + 1);
                                    f.ins().jump(refuse, &[BlockArg::from(at_q)]);
                                    f.switch_to_block(passed);
                                    let one = f.ins().iconst(types::I64, 1);
                                    f.ins().jump(join, &[l.into(), zero.into(), one.into()]);
                                    f.switch_to_block(long);
                                    let (l, bad) = fetch_last_by(&mut f, &env, ft, &bw, rule, start, &mut heads);
                                    f.ins().jump(join, &[l.into(), bad.into(), zero.into()]);
                                    f.switch_to_block(join);
                                    let p = f.block_params(join).to_vec();
                                    (p[0], p[1], Some(p[2]))
                                }
                                None => {
                                    let (l, bad) = fetch_last_by(&mut f, &env, ft, &bw, rule, start, &mut heads);
                                    (l, bad, None)
                                }
                            };
                            let (no, ok) = (f.create_block(), f.create_block());
                            f.ins().brif(bad, no, &[], ok, &[]);
                            f.switch_to_block(no);
                            f.set_cold_block(no);
                            let at = f.ins().iconst(types::I64, pc as i64 + 1);
                            f.ins().jump(refuse, &[BlockArg::from(at)]);
                            f.switch_to_block(ok);
                            if let Some((to, code)) = direct {
                                let l1 = f.ins().iadd_imm(l, 1);
                                mset(&mut f, Mem::Limit, l1);
                                let c = f.ins().iconst(types::I64, i64::from(code));
                                mset(&mut f, Mem::Taken, c);
                                f.ins().jump(if to == REFUSE { no_e } else { word(to) }, &[]);
                                continue;
                            }
                            let one_b = ft.rules.get(usize::from(rule)).is_none_or(|r| one_byte(ft, r));
                            // Its code: the kind, where no word begins with its bytes; the
                            // one-byte token's, by its byte; else the record pass's.
                            let code = if !worded {
                                f.ins().iconst(types::I64, i64::from(k))
                            } else if one_b {
                                let base = table_address(&mut f, ft.codes1.as_ptr());
                                let at = f.ins().iadd(bytes, start);
                                let b = f.ins().uload8(types::I64, flags, at, 0);
                                let o = f.ins().ishl_imm(b, 2);
                                let a = f.ins().iadd(base, o);
                                f.ins().uload32(MemFlags::trusted().with_readonly(), a, 0)
                            } else {
                                let kc = crate::native_cpu_block::tables_symbols(&mut f, &ft.tables);
                                let n = f.ins().isub(l, start);
                                let len1 = f.ins().iadd_imm(n, 1);
                                crate::native_cpu_block::plain_code(&mut f, &ft.tables, kc, bytes, start, len1)
                            };
                            let l1 = f.ins().iadd_imm(l, 1);
                            mset(&mut f, Mem::Limit, l1);
                            mset(&mut f, Mem::Taken, code);
                            // A region read in parts: entered here, its opener the token.
                            if let (Some(crate::records::EndRule::Read(i)), Some((sp, sbw))) = (ft.rules.get(usize::from(rule)), &split_bw) {
                                let read = words_read(native, &pushes, &word_kinds, usize::from(t));
                                if sp.seq.is_some() && ft.seq[usize::from(*i)] {
                                    split_enter_seq(&mut f, &env, sp, start, None, read.as_deref());
                                } else {
                                    split_enter(&mut f, &env, sp, sbw, (start, l), read.as_deref(), &mut heads);
                                }
                            }
                            match (checked, &fused) {
                                // Checked here: on past the check's word.
                                (Some(ch), Some((q, _, _))) => {
                                    let past = f.create_block();
                                    f.ins().brif(ch, past, &[], word(t), &[]);
                                    f.switch_to_block(past);
                                    jump_to_place(&mut f, *q + 1);
                                }
                                _ => {
                                    f.ins().jump(word(t), &[]);
                                }
                            }
                        }
                        f.switch_to_block(kindpath);
                    }
                    let b = if by_kind.is_some() {
                        token_kind(&mut f)
                    } else {
                        let (start, limit) = (mget(&mut f, Mem::Start), mget(&mut f, Mem::Limit));
                        let at = f.ins().iadd_imm(start, i64::from(s.offset));
                        let past = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, at, limit);
                        guard(&mut f, past, pc);
                        let addr = f.ins().iadd(bytes, at);
                        f.ins().uload8(types::I64, flags, addr, 0)
                    };
                    // A refusing target refuses at this word.
                    let no = f.create_block();
                    match cut.pick {
                        crate::native_cpu_clusters::Pick::Tree => {
                            let t = runs(s, &|t| if t == REFUSE { no } else { word(t) });
                            dispatch(&mut f, b, &t, 256);
                        }
                        crate::native_cpu_clusters::Pick::Weighted => {
                            // Each run's weight: its target's share of this word's profile, split
                            // among the target's runs.
                            let mut first: Vec<(u32, u16)> = Vec::new();
                            for (x, &t) in s.targets.iter().enumerate() {
                                if first.last().is_none_or(|&(_, l)| l != t) {
                                    first.push((x as u32, t));
                                }
                            }
                            let per: std::collections::HashMap<u16, u64> = first.iter().fold(Default::default(), |mut m, &(_, t)| {
                                *m.entry(t).or_default() += 1;
                                m
                            });
                            let t: Vec<(u32, u32, Block, u64)> = first
                                .iter()
                                .enumerate()
                                .map(|(i, &(lo, t))| {
                                    let hi = first.get(i + 1).map_or(s.targets.len() as u32, |r| r.0);
                                    // A kind's block by word: its target's traffic as the kind's.
                                    let traffic = plain_of(t).unwrap_or(t);
                                    let w = if t == REFUSE { 0 } else { cut.pick_traffic.get(&(pc as u32, u32::from(traffic))).copied().unwrap_or(0) / per[&t] };
                                    (lo, hi, if t == REFUSE { no } else { word(t) }, w)
                                })
                                .collect();
                            dispatch_weighted(&mut f, b, &t);
                        }
                        crate::native_cpu_clusters::Pick::Table => {
                            // One indirect branch on the byte.
                            let i = f.ins().ireduce(types::I32, b);
                            let calls: Vec<_> = s.targets.iter().map(|&t| f.func.dfg.block_call(if t == REFUSE { no } else { word(t) }, &[])).collect();
                            let default = f.func.dfg.block_call(no, &[]);
                            let jt = f.create_jump_table(cranelift_codegen::ir::JumpTableData::new(default, &calls));
                            f.ins().br_table(i, jt);
                        }
                    }
                    // Each kind's block by word: its words' index plus one (0: no word), a tree
                    // over them; a word the switch does not name, and no word, the kind's target.
                    for (&(k, plain), &blk) in by_word.iter().zip(&kind_blocks) {
                        f.switch_to_block(blk);
                        let w = token_word(&mut f);
                        let mut t = vec![plain; word_kinds.len() + 1];
                        for &(x, to) in &s.words {
                            if word_kinds.get(usize::from(x)).is_some_and(|&y| usize::from(y) == k) {
                                t[usize::from(x) + 1] = to;
                            }
                        }
                        let sw = ByteSwitch { targets: t, ..ByteSwitch::refusing(0) };
                        let r = runs(&sw, &|t| if t == REFUSE { no } else { word(t) });
                        dispatch(&mut f, w, &r, 256);
                    }
                    f.switch_to_block(no);
                    let at = f.ins().iconst(types::I64, pc as i64 + 1);
                    f.ins().jump(refuse, &[BlockArg::from(at)]);
                }
                Op::Span { last: false, len: n } => {
                    // A length, which must be the record's.
                    let start = mget(&mut f, Mem::Start);
                    let e = f.ins().iadd_imm(start, i64::from(n));
                    let rec_end = mget(&mut f, Mem::Limit);
                    let past = f.ins().icmp(IntCC::NotEqual, e, rec_end);
                    guard(&mut f, past, pc);
                    // Taken: the words after read the record's end (`taken`).
                    if flagged {
                        mset(&mut f, Mem::End, e);
                        let one = f.ins().iconst(types::I64, 1);
                        mset(&mut f, Mem::Need, one);
                    }
                    f.ins().jump(fall, &[]);
                }
                Op::Span { last: true, .. } => {
                    // The record's end: the words after read it there (`taken`).
                    if flagged {
                        let rec_end = mget(&mut f, Mem::Limit);
                        mset(&mut f, Mem::End, rec_end);
                        let one = f.ins().iconst(types::I64, 1);
                        mset(&mut f, Mem::Need, one);
                    }
                    f.ins().jump(fall, &[]);
                }
                Op::Word { string, test } => {
                    // The record's word: its code names the string (`records`), one compare.
                    let w = token_word(&mut f);
                    if test {
                        let same = f.ins().icmp_imm(IntCC::Equal, w, i64::from(string) + 1);
                        let skip = goto(pc + 2);
                        f.ins().brif(same, fall, &[], skip, &[]);
                    } else {
                        // One compare and a branch on it, as the lean parser's (no `bnot` of a
                        // compare, which Cranelift makes a `cset`, an `eor` and a `uxtb`).
                        let differs = f.ins().icmp_imm(IntCC::NotEqual, w, i64::from(string) + 1);
                        guard(&mut f, differs, pc);
                        f.ins().jump(fall, &[]);
                    }
                }
                Op::Mark { mask, order, end, keep, word: mark_word, pattern } => {
                    // The mark's bit at the token's start, if the mark is live: one load. At the
                    // stream's end, the mark is there when the end has it.
                    let output = native.block_programs.first().and_then(|b| b.masks.get(usize::from(mask)).copied()).ok_or(Error::UnsupportedBackend)?;
                    let skip = goto(pc + 2);
                    // A token mark past the code's (decision 0407): its bit in `Live` set at every
                    // token, as a mark's in the record is where it holds; its tokens tested below.
                    if usize::from(order) < in_code || pattern.is_some() {
                        // In the record: its bit in `Live`, one test. Taken, it and the marks
                        // before it are done at this token.
                        let live = mget(&mut f, Mem::Live);
                        let lit = f.ins().band_imm(live, 1i64 << order);
                        let taken = f.create_block();
                        f.ins().brif(lit, taken, &[], skip, &[]);
                        f.switch_to_block(taken);
                        // A gap mark tier 2 tests itself: at the stream's end, the end's (its
                        // bit in `Live` there); else its mask from the grammar token before.
                        if let Some(&(_, o)) = lazy.iter().find(|&&(k, _)| k == order) {
                            let at_end = {
                            let next = mget(&mut f, Mem::Next);
                            ended_now(&mut f, next)
                        };
                            let (ask, holds) = (f.create_block(), f.create_block());
                            f.ins().brif(at_end, holds, &[], ask, &[]);
                            f.switch_to_block(ask);
                            let key = WordFn::LazyMark { order };
                            let bn = native.block_programs.first().ok_or(Error::UnsupportedBackend)?;
                            let r = bn.records.as_ref().ok_or(Error::UnsupportedBackend)?;
                            let unless: Vec<u16> = r.unless.iter().filter(|&&(k, _)| k == order).map(|&(_, w)| w).collect();
                            let extra = r.extra;
                            let coded = (crate::native_cpu_block::lazy_coded(r) >> order & 1 == 1).then_some(order);
                            // A function of its own where the state is `ctx`'s (`outlined`), as a
                            // token mark's below; else here, the state in this function's slot or
                            // registers, which a word's function would not read.
                            if !outlined {
                                let bw = crate::native_cpu_bytes::Bytewise::of(bn);
                                let r = lazy_mark(&mut f, &env, &bw, o, extra, &unless, coded, &mut heads);
                                f.ins().brif(r, holds, &[], skip, &[]);
                            } else {
                                if !word_fns.contains_key(&key) {
                                    let name = format!("tempo_word_{}", word_fns.len());
                                    let made = outline(&mut module, &name, 1, 1, pinned_ctx, &[], |g, q, hs| {
                                        let ctx = q[0];
                                        let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
                                        let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
                                        let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
                                        let guard = |_: &mut FunctionBuilder<'_>, _: Value| unreachable!("a mark does not refuse");
                                        let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &guard };
                                        let bw = crate::native_cpu_bytes::Bytewise::of(bn);
                                        let v = lazy_mark(g, &env, &bw, o, extra, &unless, coded, hs);
                                        g.ins().return_(&[v]);
                                    })?;
                                    word_fns.insert(key, made);
                                }
                                let (id, sig, _) = &word_fns[&key];
                                let c = pinned(&mut f);
                                let r = call_word(&mut f, *id, sig, pinned_ctx, c, &[])[0];
                                f.ins().brif(r, holds, &[], skip, &[]);
                            }
                            f.switch_to_block(holds);
                        }
                        // A token mark (decision 0129): its tokens follow, from the records. At the
                        // stream's end none: a mark the end has (a gap mark's `before`, decision
                        // 0416) holds there, as its `Live` bit says.
                        if let Some(i) = pattern {
                            let follows = f.create_block();
                            let ask = f.create_block();
                            let next = mget(&mut f, Mem::Next);
                            let at_end = ended_now(&mut f, next);
                            let ended = if end { follows } else { skip };
                            f.ins().brif(at_end, ended, &[], ask, &[]);
                            f.switch_to_block(ask);
                            // Its entries together: `before`s and `unless before`s (decision 0416).
                            let marks = native.token_marks.clone();
                            let first = usize::from(i);
                            let bit = native.extras.first().map(|e| e.0);
                            let r = if outlined {
                                let key = WordFn::TokenMark { index: i };
                                if !word_fns.contains_key(&key) {
                                    let name = format!("tempo_word_{}", word_fns.len());
                                    let made = outline(&mut module, &name, 1, 1, pinned_ctx, &[], |g, q, hs| {
                                        let ctx = q[0];
                                        let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
                                        let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
                                        let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
                                        let guard = |_: &mut FunctionBuilder<'_>, _: Value| unreachable!("a token mark does not refuse");
                                        let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &guard };
                                        let v = token_patterns(g, &env, &marks, first, bit, hs);
                                        g.ins().return_(&[v]);
                                    })?;
                                    word_fns.insert(key, made);
                                }
                                let (id, sig, _) = &word_fns[&key];
                                let c = pinned(&mut f);
                                call_word(&mut f, *id, sig, pinned_ctx, c, &[])[0]
                            } else {
                                token_patterns(&mut f, &env, &marks, first, bit, &mut heads)
                            };
                            f.ins().brif(r, follows, &[], skip, &[]);
                            f.switch_to_block(follows);
                        }
                        // Kept: the callee its arm calls takes it.
                        if !keep {
                            let rest = f.ins().band_imm(live, !(native.taken[usize::from(order)] as i64));
                            mset(&mut f, Mem::Live, rest);
                        }
                        f.ins().jump(fall, &[]);
                        continue;
                    }
                    let (at_end, live) = ({
                            let next = mget(&mut f, Mem::Next);
                            ended_now(&mut f, next)
                        }, mget(&mut f, Mem::Live));
                    let lit = f.ins().ushr_imm(live, i64::from(order));
                    let lit = f.ins().band_imm(lit, 1);
                    let (look, ended) = (f.create_block(), f.create_block());
                    let taken = f.create_block();
                    let mid = f.create_block();
                    f.ins().brif(lit, mid, &[], skip, &[]);
                    f.switch_to_block(mid);
                    f.ins().brif(at_end, ended, &[], look, &[]);
                    f.switch_to_block(ended);
                    if end {
                        f.ins().jump(taken, &[]);
                    } else {
                        f.ins().jump(skip, &[]);
                    }
                    f.switch_to_block(look);
                    // A mark of one word: the record's word first, a compare on a register.
                    if let Some(s) = mark_word {
                        let w = token_word(&mut f);
                        let same = f.ins().icmp_imm(IntCC::Equal, w, i64::from(s) + 1);
                        let bits = f.create_block();
                        f.ins().brif(same, bits, &[], skip, &[]);
                        f.switch_to_block(bits);
                    }
                    let start = mget(&mut f, Mem::Start);
                    let w = f.ins().ushr_imm(start, 6);
                    let off = f.ins().imul_imm(output_stride, output as i64);
                    let base = f.ins().iadd(outputs_p, off);
                    let word = mask_word(&mut f, base, lasts_stride, w);
                    let s = f.ins().band_imm(start, 63);
                    let bit = f.ins().ushr(word, s);
                    let bit = f.ins().band_imm(bit, 1);
                    f.ins().brif(bit, taken, &[], skip, &[]);
                    f.switch_to_block(taken);
                    // It and the marks before it are done at this token; kept, the callee its arm
                    // calls takes it.
                    if !keep {
                        let live = mget(&mut f, Mem::Live);
                        let rest = f.ins().band_imm(live, !(native.taken[usize::from(order)] as i64));
                        mset(&mut f, Mem::Live, rest);
                    }
                    f.ins().jump(fall, &[]);
                }
                Op::Row { mask } => {
                    // The row's start bit at the token's start: one load.
                    let output = native.block_programs.first().and_then(|b| b.masks.get(usize::from(mask)).copied()).ok_or(Error::UnsupportedBackend)?;
                    let skip = goto(pc + 2);
                    let start = mget(&mut f, Mem::Start);
                    let w = f.ins().ushr_imm(start, 6);
                    let off = f.ins().imul_imm(output_stride, output as i64);
                    let base = f.ins().iadd(outputs_p, off);
                    let word = mask_word(&mut f, base, lasts_stride, w);
                    let s = f.ins().band_imm(start, 63);
                    let bit = f.ins().ushr(word, s);
                    let bit = f.ins().band_imm(bit, 1);
                    f.ins().brif(bit, fall, &[], skip, &[]);
                }
                Op::Open { kind } if table.direct.is_some() => {
                    let (code, next) = with_field(kind);
                    let (field, classes) = table.direct.expect("direct")[pc];
                    let kf = if field != 0 { code & 0xffff | i64::from(field) } else { code };
                    let k = f.ins().iconst(types::I64, kf);
                    word!(WordFn::Open16 { classes }, &[k], false, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| node16_word(g, e, table, q[0], classes, None), next);
                }
                Op::Open { kind } => {
                    let (code, next) = with_field(kind);
                    let k = f.ins().iconst(types::I64, code);
                    word!(WordFn::Open, &[k], false, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| open_word(g, e, table, q[0], wrapping), next);
                }
                Op::Close { this } if table.direct.is_some() => {
                    let end = end_at(pc);
                    word!(WordFn::Close16 { this, end }, &[], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, _: &[Value]| close16_word(g, e, table, this, end));
                }
                Op::Close { this } => {
                    let end = end_at(pc);
                    word!(WordFn::Close { this, end }, &[], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, _: &[Value]| close_word(g, e, table, this, end, wrapping));
                }
                Op::Leaf { kind, .. } if table.direct.is_some() => {
                    let end = end_at(pc);
                    let by_word = kind == WORD_KIND;
                    let (code, next) = with_field(kind);
                    let (field, classes) = table.direct.expect("direct")[pc];
                    let code = if field != 0 { code & 0xffff | i64::from(field) } else { code };
                    // A keyword's leaf: the field above, the kind from the token's word.
                    let code = if by_word { code & !0xffff } else { code };
                    let k = f.ins().iconst(types::I64, code);
                    let word_kinds = &native.word_kinds;
                    word!(WordFn::Leaf16 { end, by_word, classes }, &[k], by_word, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| {
                        let k = if by_word {
                            let kind = word_kind(g, e, word_kinds);
                            g.ins().bor(q[0], kind)
                        } else {
                            q[0]
                        };
                        node16_word(g, e, table, k, classes, Some(end))
                    }, next);
                }
                Op::Leaf { kind, deferred, extra } => {
                    let end = end_at(pc);
                    let by_word = kind == WORD_KIND;
                    let (code, next) = with_field(kind);
                    // A keyword's leaf: the field above, the kind from the token's word.
                    let code = if by_word { code & !0xffff } else { code };
                    let k = f.ins().iconst(types::I64, code);
                    let word_kinds = &native.word_kinds;
                    word!(WordFn::Leaf { deferred, extra, end, by_word }, &[k], by_word, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| {
                        let k = if by_word {
                            let kind = word_kind(g, e, word_kinds);
                            g.ins().bor(q[0], kind)
                        } else {
                            q[0]
                        };
                        leaf_word(g, e, table, k, LeafShape { deferred, extra, end }, wrapping)
                    }, next);
                }
                Op::Settle { .. } => {
                    // A token the grammar implies: no node; the extras pending stay where they are.
                    mset(&mut f, Mem::Pending, zero);
                    f.ins().jump(fall, &[]);
                }
                Op::SettleBefore { mark } => {
                    // The pending extras, from the first, while their codes have no bit of the
                    // mark (the marks of an extra's own gap, `Records::own`): settled. A few at
                    // most (the comments on one line). They are the records before the token's,
                    // the one lost last left out.
                    let (nodes, pending) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Pending));
                    let first = f.ins().isub(nodes, pending);
                    // The token's record is `Next - 1`, at the end too: `Next` is one past the
                    // count once the end is taken, so the end's token is the count.
                    let (next, lost) = (mget(&mut f, Mem::Next), mget(&mut f, Mem::Lost));
                    let token = f.ins().iadd_imm(next, -1);
                    let was_lost = f.ins().icmp_imm(IntCC::NotEqual, lost, 0);
                    let was_lost = f.ins().uextend(types::I64, was_lost);
                    let from = f.ins().isub(token, pending);
                    let from = f.ins().isub(from, was_lost);
                    // Record `from + (i - first)` for node `i`.
                    let record_of = f.ins().isub(from, first);
                    let (codes, stride) = (codes_p, load(&mut f, RECORD_STRIDE));
                    let (head, test, done) = (f.create_block(), f.create_block(), f.create_block());
                    f.append_block_param(head, types::I64);
                    f.append_block_param(test, types::I64);
                    f.append_block_param(done, types::I64);
                    f.ins().jump(head, &[first.into()]);
                    heads.push((head, Step::Node));
                    f.switch_to_block(head);
                    let i = f.block_params(head)[0];
                    let more = f.ins().icmp(IntCC::UnsignedLessThan, i, nodes);
                    f.ins().brif(more, test, &[i.into()], done, &[i.into()]);
                    f.switch_to_block(test);
                    let i = f.block_params(test)[0];
                    let j = f.ins().iadd(record_of, i);
                    let off = f.ins().imul(j, stride);
                    let a = f.ins().iadd(codes, off);
                    let code = f.ins().uload32(MemFlags::trusted(), a, 0);
                    let bit = f.ins().ushr_imm(code, i64::from(crate::records::MARK_SHIFT) + i64::from(mark));
                    let bit = f.ins().band_imm(bit, 1);
                    let i1 = f.ins().iadd_imm(i, 1);
                    f.ins().brif(bit, done, &[i.into()], head, &[i1.into()]);
                    f.switch_to_block(done);
                    let stop = f.block_params(done)[0];
                    // An extra a `loses since` moved (of no width: no other pending is) and the
                    // one after it go with those before: the lean lexer's semicolon is past that
                    // one.
                    let (ask, moved, stopped) = (f.create_block(), f.create_block(), f.create_block());
                    f.append_block_param(stopped, types::I64);
                    let stop2 = f.ins().iadd_imm(stop, 2);
                    let two = f.ins().icmp(IntCC::UnsignedLessThanOrEqual, stop2, nodes);
                    f.ins().brif(two, ask, &[], stopped, &[stop.into()]);
                    f.switch_to_block(ask);
                    let (s0, e0) = (NodeCol::of(end_c.rows, AT_START).get(&mut f, stop), end_c.get(&mut f, stop));
                    let none = f.ins().icmp(IntCC::Equal, s0, e0);
                    f.ins().brif(none, moved, &[], stopped, &[stop.into()]);
                    f.switch_to_block(moved);
                    f.set_cold_block(moved);
                    f.ins().jump(stopped, &[stop2.into()]);
                    f.switch_to_block(stopped);
                    let stop = f.block_params(stopped)[0];
                    let settled = f.ins().isub(stop, first);
                    let (some, after) = (f.create_block(), f.create_block());
                    f.ins().brif(settled, some, &[], after, &[]);
                    f.switch_to_block(some);
                    let last = f.ins().iadd_imm(stop, -1);
                    let e = end_c.get(&mut f, last);
                    mset(&mut f, Mem::Keep, e);
                    let left = f.ins().isub(nodes, stop);
                    mset(&mut f, Mem::Pending, left);
                    f.ins().jump(after, &[]);
                    f.switch_to_block(after);
                    f.ins().jump(fall, &[]);
                }
                Op::DropLast { pattern } => {
                    // `drops`: an extra pending, and the question holding at the token (the lean
                    // lexer's `asi`): the extra pending last dropped; the next node made takes
                    // its row and cells.
                    // A token's question: none at the stream's end.
                    let start = mget(&mut f, Mem::Start);
                    let pending = mget(&mut f, Mem::Pending);
                    let some = f.ins().icmp_imm(IntCC::NotEqual, pending, 0);
                    let next = mget(&mut f, Mem::Next);
                    let at_end = ended_now(&mut f, next);
                    let not_end = f.ins().bxor_imm(at_end, 1);
                    let some = f.ins().band(some, not_end);
                    let ask = f.create_block();
                    f.ins().brif(some, ask, &[], fall, &[]);
                    f.switch_to_block(ask);
                    let holds = gap_patterns(&mut f, &env, &native.gap_patterns, usize::from(pattern), native.extras.first().map(|e| e.0), &mut heads);
                    let drop = f.create_block();
                    f.ins().brif(holds, drop, &[], fall, &[]);
                    f.switch_to_block(drop);
                    let p = f.ins().iadd_imm(pending, -1);
                    mset(&mut f, Mem::Pending, p);
                    // None left pending: the nodes closed next end at the token (`Asi::Swallow`).
                    let le = mget(&mut f, Mem::LastEnd);
                    let none = f.ins().icmp_imm(IntCC::Equal, p, 0);
                    let le = f.ins().select(none, start, le);
                    mset(&mut f, Mem::LastEnd, le);
                    let n = mget(&mut f, Mem::Nodes);
                    let n = f.ins().iadd_imm(n, -1);
                    mset(&mut f, Mem::Nodes, n);
                    f.ins().jump(fall, &[]);
                }
                Op::RefuseIf { pattern, flag } => {
                    // `refuses`: the question holding at the token, at the mark's level.
                    let ask = f.create_block();
                    match flag {
                        Some(k) => {
                            let fl = mget(&mut f, Mem::Flags);
                            let fl = f.ins().ushr_imm(fl, i64::from(k));
                            let fl = f.ins().band_imm(fl, 1);
                            f.ins().brif(fl, ask, &[], fall, &[]);
                        }
                        None => {
                            f.ins().jump(ask, &[]);
                        }
                    }
                    f.switch_to_block(ask);
                    let holds = gap_patterns(&mut f, &env, &native.gap_patterns, usize::from(pattern), native.extras.first().map(|e| e.0), &mut heads);
                    let at = f.ins().iconst(types::I64, pc as i64 + 1);
                    f.ins().brif(holds, refuse, &[BlockArg::from(at)], fall, &[]);
                }
                Op::LoseLast { pattern, flag } => {
                    // `loses`: an extra pending, and the question holding at the token (the lean
                    // lexer's `asi`): the extra pending last taken out, its kind kept: the
                    // `TOKEN` that takes this token makes it again after the token's leaf
                    // (`lost_leaf`). One a token: a second test at it (a nested level's) finds
                    // it taken out.
                    let pending = mget(&mut f, Mem::Pending);
                    let some = f.ins().icmp_imm(IntCC::NotEqual, pending, 0);
                    let lost = mget(&mut f, Mem::Lost);
                    let none = f.ins().icmp_imm(IntCC::Equal, lost, 0);
                    let some = f.ins().band(some, none);
                    // A token's question: none at the stream's end.
                    let next = mget(&mut f, Mem::Next);
                    let at_end = ended_now(&mut f, next);
                    let not_end = f.ins().bxor_imm(at_end, 1);
                    let some = f.ins().band(some, not_end);
                    let some = f.ins().uextend(types::I64, some);
                    // Where a caller's level is the mark's: its flag.
                    let some = match flag {
                        Some(k) => {
                            let fl = mget(&mut f, Mem::Flags);
                            let fl = f.ins().ushr_imm(fl, i64::from(k));
                            let fl = f.ins().band_imm(fl, 1);
                            f.ins().band(some, fl)
                        }
                        None => some,
                    };
                    let ask = f.create_block();
                    f.ins().brif(some, ask, &[], fall, &[]);
                    f.switch_to_block(ask);
                    let holds = gap_patterns(&mut f, &env, &native.gap_patterns, usize::from(pattern), native.extras.first().map(|e| e.0), &mut heads);
                    let lose = f.create_block();
                    f.ins().brif(holds, lose, &[], fall, &[]);
                    f.switch_to_block(lose);
                    word!(WordFn::Lose, &[], false, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, _: &[Value]| {
                        let p = (e.get)(g, Mem::Pending);
                        let p = g.ins().iadd_imm(p, -1);
                        (e.set)(g, Mem::Pending, p);
                        let n = (e.get)(g, Mem::Nodes);
                        let n = g.ins().iadd_imm(n, -1);
                        (e.set)(g, Mem::Nodes, n);
                        let rows = (e.ctx)(g, ROWS);
                        let row = NodeCol::row(g, rows, n);
                        let kind = g.ins().uload32(MemFlags::trusted(), row, AT_KIND);
                        let kind = g.ins().iadd_imm(kind, 1);
                        (e.set)(g, Mem::Lost, kind);
                        // None left pending: the nodes closed next end at the token before.
                        let (keep, le) = ((e.get)(g, Mem::Keep), (e.get)(g, Mem::LastEnd));
                        let le = g.ins().select(p, le, keep);
                        (e.set)(g, Mem::LastEnd, le);
                    });
                }
                Op::MoveFirst { pattern, flag } => {
                    // `loses since`: two extras pending or more, and the question holding at the
                    // token (the lean lexer's `asi`, which reads a block comment through to
                    // another comment): the first of no width at the next one's start, in its
                    // place. Twice at a token (a nested level's test) is once.
                    let pending = mget(&mut f, Mem::Pending);
                    let some = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, pending, 1);
                    let next = mget(&mut f, Mem::Next);
                    let at_end = ended_now(&mut f, next);
                    let not_end = f.ins().bxor_imm(at_end, 1);
                    let some = f.ins().band(some, not_end);
                    let some = f.ins().uextend(types::I64, some);
                    let some = match flag {
                        Some(k) => {
                            let fl = mget(&mut f, Mem::Flags);
                            let fl = f.ins().ushr_imm(fl, i64::from(k));
                            let fl = f.ins().band_imm(fl, 1);
                            f.ins().band(some, fl)
                        }
                        None => some,
                    };
                    let ask = f.create_block();
                    f.ins().brif(some, ask, &[], fall, &[]);
                    f.switch_to_block(ask);
                    let holds = gap_patterns(&mut f, &env, &native.gap_patterns, usize::from(pattern), native.extras.first().map(|e| e.0), &mut heads);
                    let moving = f.create_block();
                    f.ins().brif(holds, moving, &[], fall, &[]);
                    f.switch_to_block(moving);
                    f.set_cold_block(moving);
                    let rows = env_ctx(&mut f, ROWS);
                    let (n, p) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Pending));
                    let first = f.ins().isub(n, p);
                    let second = f.ins().iadd_imm(first, 1);
                    let (a, b) = (NodeCol::row(&mut f, rows, first), NodeCol::row(&mut f, rows, second));
                    let s = NodeCol::of(rows, AT_START).get_row(&mut f, b);
                    NodeCol::of(rows, AT_START).put_row(&mut f, a, s);
                    NodeCol::of(rows, AT_END).put_row(&mut f, a, s);
                    // Its cells again: its block's base and offsets from the rows (it may be in a
                    // block before the last: the extras pending can cross one), and its tail.
                    let b = f.ins().ushr_imm(first, rerail_tape_format::CHECKPOINT_STRIDE.trailing_zeros() as i64);
                    table.blocks(&mut f, &env, b, n);
                    let one = f.ins().iconst(types::I64, 1);
                    table.close_cells(&mut f, &env, Closed { at: first, size: one, last: first }, s, Some(s));
                    f.ins().jump(fall, &[]);
                }
                // A keyword's: its token's word's kind (`WORD_KIND`), in a word of its own.
                Op::Remember { kind: WORD_KIND } => {
                    let word_kinds = &native.word_kinds;
                    word!(WordFn::RememberWord, &[], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, _: &[Value]| {
                        let k = word_kind(g, e, word_kinds);
                        (e.set)(g, Mem::Word, k);
                    });
                }
                Op::Remember { kind } => {
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    mset(&mut f, Mem::Word, k);
                    f.ins().jump(fall, &[]);
                }
                Op::Check { check } => {
                    let c = native.local_checks.get(usize::from(check)).cloned().ok_or(Error::UnsupportedBackend)?;
                    let form = crate::stream_facts::word_form(&c.ops).map_err(|_| Error::UnsupportedBackend)?;
                    // A token's check reads no mask: its function at every token of the kind.
                    let output = if c.token { 0 } else { native.block_programs.first().and_then(|b| b.masks.get(usize::from(c.mask)).copied()).ok_or(Error::UnsupportedBackend)? };
                    // The mask from the bytes where they make it: the check's function reads it
                    // there, a call a token of the kind, no row to load first.
                    let bytewise = native.block_programs.first().map(crate::native_cpu_bytes::Bytewise::of).filter(|b| !c.token && b.can(output as u16));
                    let (start, limit) = (mget(&mut f, Mem::Start), mget(&mut f, Mem::Limit));
                    // A token whose last byte the check can not hold at: none of it (decision
                    // 0428), as an escape that is no `\u` skips the `\u`'s checks.
                    if let Some(set) = token_gate(&c).filter(|_| c.token) {
                        let at = f.ins().iadd(bytes, limit);
                        let b = f.ins().uload8(types::I64, MemFlags::trusted(), at, -1);
                        let can = byte_in(&mut f, b, set);
                        let go = f.create_block();
                        f.ins().brif(can, go, &[], fall, &[]);
                        f.switch_to_block(go);
                    }
                    let slow = f.create_block();
                    if c.token && c.back != 0 && table.split.is_some() {
                        return Err(Error::UnsupportedFeature("a token's check that reads back, in a region tier 2 splits"));
                    }
                    if c.token && usize::from(c.delay) + usize::from(c.back) >= 16 {
                        f.ins().jump(slow, &[]);
                    } else if c.token {
                        // A frame of 16 bytes or fewer (the bytes it reads back, the token and
                        // its delay), here; a longer one, the window loop's.
                        let n = f.ins().isub(limit, start);
                        let short = f.create_block();
                        let long = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, n, 16 - i64::from(c.delay) - i64::from(c.back));
                        f.ins().brif(long, slow, &[], short, &[]);
                        f.switch_to_block(short);
                        // The tokens' ends before it (decision 0428): from the records.
                        let ended = (c.back != 0).then(|| {
                            let rb = pinned(&mut f);
                            let stride = f.ins().load(types::I64, MemFlags::trusted().with_readonly().with_can_move(), rb, RECORD_STRIDE);
                            let next = mget(&mut f, Mem::Next);
                            ended_word(&mut f, lasts_p, stride, next, start, c.back)
                        });
                        let hit = token_check_short(&mut f, &c, &form, bytes, len, start, n, ended);
                        guard(&mut f, hit, pc);
                        f.ins().jump(fall, &[]);
                    } else if bytewise.is_some() {
                        f.ins().jump(slow, &[]);
                    } else {
                    // Most tokens: no bit of the mask from the start on, in the start's word, and
                    // the token ends in it. One load.
                    let w = f.ins().ushr_imm(start, 6);
                    let off = f.ins().imul_imm(output_stride, output as i64);
                    let base = f.ins().iadd(outputs_p, off);
                    let word = mask_word(&mut f, base, lasts_stride, w);
                    let s = f.ins().band_imm(start, 63);
                    let word = f.ins().ushr(word, s);
                    let l1 = f.ins().iadd_imm(limit, -1);
                    let wl = f.ins().ushr_imm(l1, 6);
                    let other = f.ins().icmp(IntCC::NotEqual, wl, w);
                    let any = f.ins().icmp_imm(IntCC::NotEqual, word, 0);
                    let go = f.ins().bor(any, other);
                    f.ins().brif(go, slow, &[], fall, &[]);
                    }
                    f.switch_to_block(slow);
                    // Always a function of its own, one a check: rare code, out of the arm's.
                    let r = {
                        let key = WordFn::Check { index: check };
                        if !word_fns.contains_key(&key) {
                            let name = format!("tempo_word_{}", word_fns.len());
                            let made = outline(&mut module, &name, 4, 1, pinned_ctx, &[], |g, q, hs| {
                                let ctx = q[0];
                                let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
                                let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
                                let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
                                let guard = |_: &mut FunctionBuilder<'_>, _: Value| unreachable!("a check returns whether it holds");
                                let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &guard };
                                let v = if c.token { token_check_word(g, &env, &c, &form, q[1], q[2], q[3], hs) } else { check_word(g, &env, &c, &form, output, bytewise.as_ref(), q[1], q[2], hs) };
                                g.ins().return_(&[v]);
                            })?;
                            word_fns.insert(key, made);
                        }
                        let (id, sig, _) = &word_fns[&key];
                        let cv = pinned(&mut f);
                        // The token's index past it: the state's `Next` is in this function's
                        // frame, not `ctx`, where it is one function (decision 0428).
                        let next = mget(&mut f, Mem::Next);
                        call_word(&mut f, *id, sig, pinned_ctx, cv, &[start, limit, next])[0]
                    };
                    guard(&mut f, r, pc);
                    f.ins().jump(fall, &[]);
                }
                Op::Count { count, up, max } => {
                    // One count (a file's depth) for now.
                    if count != 0 {
                        return Err(Error::UnsupportedBackend);
                    }
                    let d = mget(&mut f, Mem::Depth);
                    if up {
                        let d = f.ins().iadd_imm(d, 1);
                        mset(&mut f, Mem::Depth, d);
                        let past = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, d, i64::from(max));
                        guard(&mut f, past, pc);
                    } else {
                        let above = f.ins().icmp_imm(IntCC::NotEqual, d, 0);
                        let one = f.ins().uextend(types::I64, above);
                        let d = f.ins().isub(d, one);
                        mset(&mut f, Mem::Depth, d);
                    }
                    f.ins().jump(fall, &[]);
                }
                Op::Flag { flag, op } => {
                    let fl = mget(&mut f, Mem::Flags);
                    let bit = 1i64 << flag;
                    match op {
                        FlagOp::Set => {
                            let fl = f.ins().bor_imm(fl, bit);
                            mset(&mut f, Mem::Flags, fl);
                            f.ins().jump(fall, &[]);
                        }
                        FlagOp::Clear => {
                            let fl = f.ins().band_imm(fl, !bit);
                            mset(&mut f, Mem::Flags, fl);
                            f.ins().jump(fall, &[]);
                        }
                        // The next word where the flag is set; else the one after.
                        FlagOp::Test => {
                            let skip = goto(pc + 2);
                            let set = f.ins().band_imm(fl, bit);
                            f.ins().brif(set, fall, &[], skip, &[]);
                        }
                    }
                }
                Op::Retag { kind, on: On::Moved } => {
                    let remembered = kind == REMEMBERED;
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::RetagMoved { remembered }, &[k], remembered, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| {
                        let k = if remembered { remembered_kind(g, e) } else { q[0] };
                        let m = (e.get)(g, Mem::Moved);
                        let rows = (e.ctx)(g, ROWS);
                        table.retag(g, e, rows, m, k);
                    });
                }
                Op::Wrap { kind } if wrapping => {
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Wrap, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| wrap_word(g, e, table, q[0]));
                }
                Op::NodeField { field, on: On::Inner } => {
                    // The node the node made last, a wrap, holds first: the row after it, unless
                    // that is a wrap too (`head`).
                    let open = mget(&mut f, Mem::Open);
                    let last = mget(&mut f, Mem::LastMade);
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
                    // A branch each, not the tests joined.
                    guard(&mut f, none, pc);
                    guard(&mut f, no_last, pc);
                    let r = f.ins().iadd_imm(last, -1);
                    let (mark, inner_mark) = {
                        let a = slot(&mut f, wc(head_c), r);
                        (f.ins().uload32(flags, a, 0), f.ins().uload32(flags, a, 4))
                    };
                    let mark = f.ins().band_imm(mark, WRAPPED);
                    let not_wrap = f.ins().icmp_imm(IntCC::Equal, mark, 0);
                    guard(&mut f, not_wrap, pc);
                    let inner_mark = f.ins().band_imm(inner_mark, WRAPPED);
                    let nested = f.ins().icmp_imm(IntCC::NotEqual, inner_mark, 0);
                    guard(&mut f, nested, pc);
                    let x = f.ins().iconst(types::I64, i64::from(field));
                    field_c.put(&mut f, last, x);
                    f.ins().jump(fall, &[]);
                }
                Op::Retag { kind, on: On::Made } | Op::NodeField { field: kind, on: On::Made | On::MadeUnset } => {
                    let retag = matches!(w.operation, Op::Retag { .. });
                    let unset = matches!(w.operation, Op::NodeField { on: On::MadeUnset, .. });
                    let remembered = retag && kind == REMEMBERED;
                    let dead = retag && kind == DEAD;
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Made { retag, remembered, dead, unset }, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| {
                        let k = if remembered { remembered_kind(g, e) } else { q[0] };
                        made_word(g, e, table, k, retag, dead, unset)
                    });
                }
                Op::Reread { reading } if wrapping => {
                    // The node made last at the innermost open node's level and the nodes inside
                    // it: its rows, in preorder, to the pending extras. A loop over the rows, a
                    // step a row (`Step::Node`); a kind each, by a run of compares. A dropped row
                    // is taken out after (`drop`), where any was.
                    let r = native.readings.maps.get(usize::from(reading)).ok_or(Error::UnsupportedBackend)?;
                    let open = mget(&mut f, Mem::Open);
                    let last = mget(&mut f, Mem::LastMade);
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
                    // A branch each, not the tests joined.
                    guard(&mut f, none, pc);
                    guard(&mut f, no_last, pc);
                    let root = f.ins().iadd_imm(last, -1);
                    let (nodes, pending) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Pending));
                    let end = f.ins().isub(nodes, pending);
                    let (scan, body, after) = (f.create_block(), f.create_block(), f.create_block());
                    f.append_block_param(scan, types::I64);
                    f.append_block_param(scan, types::I64);
                    f.append_block_param(after, types::I64);
                    heads.push((scan, Step::Node));
                    f.ins().jump(scan, &[BlockArg::from(root), BlockArg::from(zero)]);
                    // scan(i, gone): row i; whether a row was dropped.
                    f.switch_to_block(scan);
                    let (i, gone) = (f.block_params(scan)[0], f.block_params(scan)[1]);
                    let done = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, end);
                    f.ins().brif(done, after, &[BlockArg::from(gone)], body, &[]);
                    f.switch_to_block(body);
                    let nrow = NodeCol::row(&mut f, rows_here, i);
                    let k = kind_c.get_row(&mut f, nrow);
                    // Its field, unless it is the node read: the reading's, by a run of selects.
                    let fld = field_c.get_row(&mut f, nrow);
                    let mut to = fld;
                    for &(a, b) in &r.fields {
                        let is = f.ins().icmp_imm(IntCC::Equal, fld, i64::from(a));
                        let g = f.ins().iconst(types::I64, i64::from(b));
                        to = f.ins().select(is, g, to);
                    }
                    let is_root = f.ins().icmp(IntCC::Equal, i, root);
                    let to = f.ins().select(is_root, fld, to);
                    field_c.put_row(&mut f, nrow, to);
                    let i1 = f.ins().iadd_imm(i, 1);
                    // Its way: the root's, for the node read, when the reading gives one; else
                    // its kind's, a compare each, the last refusing.
                    let ways: Vec<(Option<u16>, u16)> = r.root.map(|w| (None, w)).into_iter().chain(r.kinds.iter().map(|&(kk, w)| (Some(kk), w))).collect();
                    for (kk, way) in ways {
                        let (hit, miss) = (f.create_block(), f.create_block());
                        let is = match kk {
                            Some(kk) => f.ins().icmp_imm(IntCC::Equal, k, i64::from(kk)),
                            None => is_root,
                        };
                        f.ins().brif(is, hit, &[], miss, &[]);
                        f.switch_to_block(hit);
                        if way != KEEP {
                            // Reading on into its children, where a dropped row was: the
                            // interpreter meets that row and refuses.
                            let m = {
                                let a = slot(&mut f, wc(head_c), i);
                                f.ins().uload32(flags, a, 0)
                            };
                            let m = f.ins().band_imm(m, DROPPED_IN);
                            guard(&mut f, m, pc);
                        }
                        match way {
                            KEEP => {
                                let sc = env_ctx(&mut f, SIZE_C);
                                let size = crate::native_cpu_tape::get(&mut f, sc, i, table.w.size);
                                let past = f.ins().iadd(i, size);
                                f.ins().jump(scan, &[BlockArg::from(past), BlockArg::from(gone)]);
                            }
                            GONE => {
                                // The node read going: none made last at this level until the next.
                                let kept = f.ins().select(is_root, zero, last);
                                mset(&mut f, Mem::LastMade, kept);
                                let d = f.ins().iconst(types::I64, i64::from(DEAD));
                                kind_c.put_row(&mut f, nrow, d);
                                let one = f.ins().iconst(types::I64, 1);
                                f.ins().jump(scan, &[BlockArg::from(i1), BlockArg::from(one)]);
                            }
                            m => {
                                let m = kind_of(&mut f, m, pc);
                                table.retag(&mut f, &env, rows_here, i, m);
                                f.ins().jump(scan, &[BlockArg::from(i1), BlockArg::from(gone)]);
                            }
                        }
                        f.switch_to_block(miss);
                    }
                    let at = f.ins().iconst(types::I64, pc as i64 + 1);
                    f.ins().jump(refuse, &[BlockArg::from(at)]);
                    f.switch_to_block(after);
                    let gone = f.block_params(after)[0];
                    let (dropping, kept) = (f.create_block(), f.create_block());
                    f.ins().brif(gone, dropping, &[], kept, &[]);
                    f.switch_to_block(dropping);
                    f.set_cold_block(dropping);
                    drop_from(&mut f, &env, table, root);
                    f.ins().jump(fall, &[]);
                    f.switch_to_block(kept);
                    f.ins().jump(fall, &[]);
                }
                Op::Retag { kind, on: On::Here } => {
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Here, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| here_word(g, e, table, q[0]));
                }
                Op::Merge { kind, fold: true } => {
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Fold, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| fold_word(g, e, table, q[0], wrapping));
                }
                Op::Merge { kind, fold: false } => {
                    let end = end_at(pc);
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Merge { end }, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| merge_word(g, e, table, q[0], end));
                }
                // In a direct format 16 run the node's field and supertypes are in its number, taken at its word.
                Op::NodeField { on: On::Here, .. } | Op::NodeSuper { .. } if table.direct.is_some() => {
                    f.ins().jump(fall, &[]);
                }
                Op::NodeField { field, on } => {
                    let last = if on == On::Moved {
                        mget(&mut f, Mem::Moved)
                    } else {
                        let nodes = mget(&mut f, Mem::Nodes);
                        f.ins().iadd_imm(nodes, -1)
                    };
                    let x = f.ins().iconst(types::I64, i64::from(field));
                    field_c.put(&mut f, last, x);
                    f.ins().jump(fall, &[]);
                }
                Op::NodeSuper { kind } => {
                    let (nodes, supers) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Supers));
                    let last = f.ins().iadd_imm(nodes, -1);
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    put(&mut f, snode_c, supers, last);
                    put(&mut f, skind_c, supers, k);
                    let supers = f.ins().iadd_imm(supers, 1);
                    mset(&mut f, Mem::Supers, supers);
                    f.ins().jump(fall, &[]);
                }
                Op::Push { target, flag } if cut.calls => {
                    // A native call of the callee's function (`Cut::calls`): the calls open are
                    // counted (`ACCEPT` and `RETURN` check them), the held state stored for the
                    // callee and loaded again after, the level's word and flags kept in this
                    // frame. Back at the return's place, or the one after; any other value is
                    // the run's end, passed on. The call is a loop's head: back at the place after
                    // the return's (`RETURN m=1`), a rule switches on the same token again, and
                    // each turn calls a rule, which takes a token before it returns.
                    heads.push((wb[pc].expect("its own place"), Step::Return));
                    // Counted by the machine's stack (`on_stack`), as the lean parser's: nothing
                    // here, the depth checked where the callee is entered.
                    if !on_stack {
                        let ret = mget(&mut f, Mem::Ret);
                        let deep = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, ret, DEPTH);
                        guard(&mut f, deep, pc);
                        let ret = f.ins().iadd_imm(ret, 1);
                        mset(&mut f, Mem::Ret, ret);
                    }
                    let kept = (two_slots && !callee_keeps && changes_context[cut.of[callee(native, pc)] as usize]).then(|| (mget(&mut f, Mem::Word), mget(&mut f, Mem::Flags)));
                    // The flag the call runs with, as a parameter: the caller's flags given back
                    // after it (`let f = self.flags; ..; self.call(); self.flags = f`).
                    let flag_back = flag.map(|(k, set)| {
                        let fl = mget(&mut f, Mem::Flags);
                        let changed = if set { f.ins().bor_imm(fl, 1i64 << k) } else { f.ins().band_imm(fl, !(1i64 << k)) };
                        mset(&mut f, Mem::Flags, changed);
                        fl
                    });
                    for (k, v) in vars.iter().enumerate() {
                        if let Some(v) = v.filter(|_| !cut.pass.contains(&(k as u32))) {
                            let x = f.use_var(v);
                            let p = pinned(&mut f);
                            f.ins().store(MemFlags::trusted(), x, p, STATE + 8 * k as i32);
                        }
                    }
                    let e = callee(native, pc);
                    let d = cut.of[e] as usize;
                    let i = index[d][&e];
                    let fref = *call_refs.entry(d).or_insert_with(|| {
                        use cranelift_codegen::ir::{ExtFuncData, ExternalName, UserExternalName};
                        let name = f.func.declare_imported_user_function(UserExternalName::new(0, cids[d].as_u32()));
                        let signature = f.import_signature(csig.clone());
                        f.import_function(ExtFuncData { name: ExternalName::user(name), signature, colocated: true })
                    });
                    let iv = f.ins().iconst(types::I64, i as i64);
                    let passed: Vec<Value> = cut.pass.iter().map(|&k| f.use_var(vars[k as usize].expect("a passed field is held"))).collect();
                    let args: Vec<Value> = (!pinned_ctx).then_some(ctx).into_iter().chain([iv]).chain(passed).collect();
                    let call = f.ins().call(fref, &args);
                    let back = f.inst_results(call).to_vec();
                    let v = back[0];
                    for (j, &k) in cut.pass.iter().enumerate() {
                        f.def_var(vars[k as usize].expect("a passed field is held"), back[1 + j]);
                    }
                    for (k, var) in vars.iter().enumerate() {
                        if let Some(var) = var.filter(|_| !cut.pass.contains(&(k as u32))) {
                            let p = pinned(&mut f);
                            let x = f.ins().load(types::I64, MemFlags::trusted(), p, STATE + 8 * k as i32);
                            f.def_var(var, x);
                        }
                    }
                    if let Some((w, fl)) = kept {
                        mset(&mut f, Mem::Word, w);
                        mset(&mut f, Mem::Flags, fl);
                    }
                    if let Some(fl) = flag_back {
                        mset(&mut f, Mem::Flags, fl);
                    }
                    let (other, not_next) = (f.create_block(), f.create_block());
                    if let Some(way) = one_way(e) {
                        // Zero: back at the call's place, or the one after, as the callee always
                        // returns; else the run's end, passed on.
                        f.ins().brif(v, other, &[], word(target + way), &[]);
                        f.switch_to_block(not_next);
                        f.ins().jump(other, &[]);
                        f.switch_to_block(other);
                        f.set_cold_block(other);
                        let r = results(&mut f, v, false);
                        leave(&mut f, &r);
                        continue;
                    }
                    // Zero: back at the call's place; else one, the place after.
                    f.ins().brif(v, not_next, &[], word(target), &[]);
                    f.switch_to_block(not_next);
                    let again = f.ins().icmp_imm(IntCC::Equal, v, RETURNED as i64 + 1);
                    f.ins().brif(again, word(target + 1), &[], other, &[]);
                    f.switch_to_block(other);
                    f.set_cold_block(other);
                    let r = results(&mut f, v, false);
                    leave(&mut f, &r);
                }
                Op::Return { again } if cut.calls => {
                    // With the calls open on the stack, a call is always open here
                    // (`calls_on_stack`).
                    if !on_stack {
                        let ret = mget(&mut f, Mem::Ret);
                        let none = f.ins().icmp_imm(IntCC::Equal, ret, 0);
                        guard(&mut f, none, pc);
                        let ret = f.ins().iadd_imm(ret, -1);
                        mset(&mut f, Mem::Ret, ret);
                    }
                    for (k, v) in vars.iter().enumerate() {
                        if let Some(v) = v.filter(|_| !cut.pass.contains(&(k as u32))) {
                            let x = f.use_var(v);
                            let p = pinned(&mut f);
                            f.ins().store(MemFlags::trusted(), x, p, STATE + 8 * k as i32);
                        }
                    }
                    // The caller's token, taken here (`callee_takes`): a refusal is this word's.
                    if let (Some((any, extras)), false) = (landing_token, again) {
                        if extras && native.extras.is_empty() {
                            return Err(Error::UnsupportedBackend);
                        }
                        let (extra, more) = MoreExtras::of(&native.extras, extras)?;
                        let any_mark: Option<usize> = match any {
                            Some(k) => Some(native.block_programs.first().and_then(|b| b.masks.get(usize::from(k)).copied()).ok_or(Error::UnsupportedBackend)?),
                            None => None,
                        };
                        let shape = TokenShape { bound, unchecked, in_code, rest_bits, end_bits, end_code: end_kind.map(i64::from), on_stack, extra, more, wrapping, lost: loses, recovers };
                        let (id, sig, _) = token_fn(&mut module, &mut word_fns, any_mark, shape, table, &native.token_marks, pinned_ctx)?;
                        let c = pinned(&mut f);
                        let r = call_word(&mut f, *id, sig, pinned_ctx, c, &[])[0];
                        let (no, taken) = (f.create_block(), f.create_block());
                        f.ins().brif(r, no, &[], taken, &[]);
                        f.switch_to_block(no);
                        f.set_cold_block(no);
                        let past = f.ins().icmp_imm(IntCC::Equal, r, 1);
                        let here = f.ins().iconst(types::I64, pc as i64 + 1);
                        let at = f.ins().select(past, here, r);
                        f.ins().jump(refuse, &[BlockArg::from(at)]);
                        f.switch_to_block(taken);
                    }
                    // A function that returns one way gives back 0 for it (`one_way`); with the
                    // caller's token taken here, every return (`callee_takes`).
                    let way = if callee_takes || (again && by_rule && again_as_one[pc]) { 0 } else { i64::from(again) };
                    let v = f.ins().iconst(types::I64, RETURNED as i64 + way);
                    let r = results(&mut f, v, false);
                    leave(&mut f, &r);
                }
                Op::Push { target, flag } => {
                    // Calls nested at most `DEPTH` deep, as a native call's (above) and the Rust's (0537).
                    let ret = mget(&mut f, Mem::Ret);
                    let deep = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, ret, DEPTH);
                    guard(&mut f, deep, pc);
                    let t = f.ins().iconst(types::I64, i64::from(target));
                    // With context (`REMEMBER`, `FLAG`), two slots a call: its return, then the
                    // caller's word and flags (`context`).
                    if two_slots {
                        let at = f.ins().ishl_imm(ret, 1);
                        put(&mut f, ret_c, at, t);
                        let (w, fl) = (mget(&mut f, Mem::Word), mget(&mut f, Mem::Flags));
                        let fl = f.ins().ishl_imm(fl, 16);
                        let wf = f.ins().bor(w, fl);
                        let at = f.ins().iadd_imm(at, 1);
                        put(&mut f, ret_c, at, wf);
                    } else {
                        put(&mut f, ret_c, ret, t);
                    }
                    let ret = f.ins().iadd_imm(ret, 1);
                    mset(&mut f, Mem::Ret, ret);
                    // The flag the call runs with: the caller's saved beside its return above.
                    if let Some((k, set)) = flag {
                        let fl = mget(&mut f, Mem::Flags);
                        let changed = if set { f.ins().bor_imm(fl, 1i64 << k) } else { f.ins().band_imm(fl, !(1i64 << k)) };
                        mset(&mut f, Mem::Flags, changed);
                    }
                    f.ins().jump(fall, &[]);
                }
                Op::Return { again } => {
                    if chains {
                        heads.push((wb[pc].expect("its own place"), Step::Return));
                    }
                    let ret = mget(&mut f, Mem::Ret);
                    let none = f.ins().icmp_imm(IntCC::Equal, ret, 0);
                    guard(&mut f, none, pc);
                    let ret = f.ins().iadd_imm(ret, -1);
                    mset(&mut f, Mem::Ret, ret);
                    let p = if two_slots {
                        // The caller's word and flags back, from beside its return.
                        let at = f.ins().ishl_imm(ret, 1);
                        let a = slot(&mut f, ret_c, at);
                        let wf = f.ins().uload32(flags, a, 4);
                        let w = f.ins().band_imm(wf, 0xffff);
                        let fl = f.ins().ushr_imm(wf, 16);
                        mset(&mut f, Mem::Word, w);
                        mset(&mut f, Mem::Flags, fl);
                        f.ins().uload32(flags, a, 0)
                    } else {
                        let a = slot(&mut f, ret_c, ret);
                        f.ins().uload32(flags, a, 0)
                    };
                    // A compare tree over the places `PUSH` names.
                    let t: Vec<(u32, Block)> = pushes.iter().map(|&p| (u32::from(p), word(p + u16::from(again)))).collect();
                    if t.is_empty() {
                        f.ins().jump(off_end, &[]);
                    } else {
                        dispatch(&mut f, p, &t, u64::MAX);
                    }
                }
                Op::Accept => {
                    let (next, open) = (mget(&mut f, Mem::Next), mget(&mut f, Mem::Open));
                    let not_end = ended_now(&mut f, next);
                    let not_end = f.ins().bxor_imm(not_end, 1);
                    let not_one = f.ins().icmp_imm(IntCC::NotEqual, open, 1);
                    // A branch each, not the tests joined.
                    guard(&mut f, not_end, pc);
                    guard(&mut f, not_one, pc);
                    // Within no call (`calls_on_stack`), or checked.
                    if !on_stack {
                        let ret = mget(&mut f, Mem::Ret);
                        let returns = f.ins().icmp_imm(IntCC::NotEqual, ret, 0);
                        guard(&mut f, returns, pc);
                    }
                    let nodes = mget(&mut f, Mem::Nodes);
                    if let (Some(_), Some(s)) = (table.direct, &table.fns.sixteen) {
                        // The root closed in the ops at the file's end; no row.
                        let desc = env_ctx(&mut f, WRITER);
                        crate::native_cpu_tape::call_op(&mut f, &s.ops.close, desc, &[len]);
                    } else {
                        let root = f.ins().uload32(flags, open_c, 0);
                        let root_row = NodeCol::row(&mut f, rows_here, root);
                        end_c.put_row(&mut f, root_row, len);
                        // From its first token or comment, the row after it (none: the file's
                        // end), as tree-sitter's root starts past the file's leading blanks.
                        let after = f.ins().iadd_imm(root, 1);
                        let more = f.ins().icmp(IntCC::UnsignedLessThan, after, nodes);
                        let after = f.ins().select(more, after, root);
                        let after_row = NodeCol::row(&mut f, rows_here, after);
                        let first = NodeCol::of(rows_here, AT_START).get_row(&mut f, after_row);
                        let first = f.ins().select(more, first, len);
                        // A format 16 run gave the root's start to its writer when it passed it
                        // (`Table::pass`). Else, where it moved (leading blanks): its row, and
                        // the first block's base and offsets again.
                        if table.fns.sixteen.is_none() && native.padded {
                            let old = NodeCol::of(rows_here, AT_START).get_row(&mut f, root_row);
                            let moved = f.ins().icmp(IntCC::NotEqual, first, old);
                            let (again, on) = (f.create_block(), f.create_block());
                            f.ins().brif(moved, again, &[], on, &[]);
                            f.switch_to_block(again);
                            f.set_cold_block(again);
                            NodeCol::of(rows_here, AT_START).put_row(&mut f, root_row, first);
                            let b0 = f.ins().iconst(types::I64, 0);
                            table.blocks(&mut f, &env, b0, nodes);
                            f.ins().jump(on, &[]);
                            f.switch_to_block(on);
                        }
                        let size = f.ins().isub(nodes, root);
                        let last = f.ins().iadd_imm(nodes, -1);
                        table.close_cells(&mut f, &env, Closed { at: root, size, last }, len, None);
                    }
                    if !native.readings.provisional.is_empty() {
                        // No node left holds a provisional field (a dropped record aside): a loop
                        // over the table, a step a record.
                        let (check, step, after) = (f.create_block(), f.create_block(), f.create_block());
                        f.append_block_param(check, types::I64);
                        heads.push((check, Step::Node));
                        f.ins().jump(check, &[BlockArg::from(zero)]);
                        f.switch_to_block(check);
                        let i = f.block_params(check)[0];
                        let done = f.ins().icmp(IntCC::Equal, i, nodes);
                        f.ins().brif(done, after, &[], step, &[]);
                        f.switch_to_block(step);
                        let (fld, k) = {
                            let row = NodeCol::row(&mut f, rows_here, i);
                            (field_c.get_row(&mut f, row), kind_c.get_row(&mut f, row))
                        };
                        let mut hit = f.ins().iconst(types::I8, 0);
                        for &p in &native.readings.provisional {
                            let is = f.ins().icmp_imm(IntCC::Equal, fld, i64::from(p));
                            hit = f.ins().bor(hit, is);
                        }
                        let _ = k;
                        guard(&mut f, hit, pc);
                        let i1 = f.ins().iadd_imm(i, 1);
                        f.ins().jump(check, &[BlockArg::from(i1)]);
                        f.switch_to_block(after);
                    }
                    // A value past its width (an offset, a tail, the node count): refused at
                    // these widths (docs/parser-generators/37: parsed again at wider ones). A
                    // format 16 run reads only its sizes, compiled at 4 bytes: it passes the
                    // nodes not yet passed, and the root is closed (`native_cpu_tape::pass`).
                    let total = if table.direct.is_some() {
                        f.ins().iconst(types::I64, 0)
                    } else if table.fns.sixteen.is_some() {
                        let passed = mget(&mut f, Mem::Codes);
                        table.pass(&mut f, &env, passed, nodes, false);
                        f.ins().iconst(types::I64, 0)
                    } else {
                        let wide = mget(&mut f, Mem::Wide);
                        guard(&mut f, wide, pc);
                        if table.w.size == 2 {
                            let many = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, nodes, 0xffff);
                            guard(&mut f, many, pc);
                        }
                        // The codes of the nodes not yet committed, then the image's end.
                        table.commit(&mut f, &env, nodes);
                        let image_p = env_ctx(&mut f, IMAGE);
                        let cols = [SIZE_C, BASE_C, OFF_C, TAIL_C, CODE_KF, ID_OF].map(|at| env_ctx(&mut f, at));
                        let codes = mget(&mut f, Mem::Codes);
                        let (id, sig, _) = &table.fns.finish;
                        call_own(&mut f, *id, sig, &[image_p, cols[0], cols[1], cols[2], cols[3], cols[4], cols[5], nodes, codes, len])[0]
                    };
                    f.ins().store(flags, nodes, counts_c, 0);
                    let supers = mget(&mut f, Mem::Supers);
                    f.ins().store(flags, supers, counts_c, 8);
                    // The image's length.
                    f.ins().store(flags, total, counts_c, 16);
                    f.ins().jump(fin, &[BlockArg::from(rows_here), BlockArg::from(nodes)]);
                    fin_used = true;
                }
                Op::Empty { total } => {
                    // A total of tier 1's layout: 8 readable bytes at each (`Ctx::totals_mask`).
                    let off = f.ins().imul_imm(totals_stride, i64::from(total));
                    let at = f.ins().iadd(totals_p, off);
                    let t = f.ins().load(types::I64, flags, at, 0);
                    let t = f.ins().band(t, totals_mask);
                    let some = f.ins().icmp_imm(IntCC::NotEqual, t, 0);
                    guard(&mut f, some, pc);
                    f.ins().jump(fall, &[]);
                }
                Op::Goto { target } => {
                    f.ins().jump(word(target), &[]);
                }
                // A statement's start (decision 0650): the statement before ends here, one that holds a provisional field
                // refused; then its mark (`Ctx::mark`), where the stream goes on and the levels open are at most
                // `MARK_LEVELS`; then a token a refusing mark held (`MARK_HELD`) refuses it.
                Op::Statement if recovers => {
                    let p = pinned(&mut f);
                    if !native.readings.provisional.is_empty() {
                        let marked = f.ins().load(types::I64, MemFlags::trusted(), p, MARK);
                        let (from_mark, check, step, after) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
                        f.append_block_param(check, types::I64);
                        f.ins().brif(marked, from_mark, &[], after, &[]);
                        f.switch_to_block(from_mark);
                        let p = pinned(&mut f);
                        let from = f.ins().load(types::I64, MemFlags::trusted(), p, MARK_STATE + 8 * Mem::Nodes as i32);
                        let nodes = mget(&mut f, Mem::Nodes);
                        f.ins().jump(check, &[BlockArg::from(from)]);
                        f.switch_to_block(check);
                        heads.push((check, Step::Node));
                        let i = f.block_params(check)[0];
                        let done = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, nodes);
                        f.ins().brif(done, after, &[], step, &[]);
                        f.switch_to_block(step);
                        let (fld, k) = {
                            let row = NodeCol::row(&mut f, rows_here, i);
                            (field_c.get_row(&mut f, row), kind_c.get_row(&mut f, row))
                        };
                        let mut hit = f.ins().iconst(types::I8, 0);
                        for &q in &native.readings.provisional {
                            let is = f.ins().icmp_imm(IntCC::Equal, fld, i64::from(q));
                            hit = f.ins().bor(hit, is);
                        }
                        let live = f.ins().icmp_imm(IntCC::NotEqual, k, i64::from(DEAD));
                        let hit = f.ins().band(hit, live);
                        guard(&mut f, hit, pc);
                        let i1 = f.ins().iadd_imm(i, 1);
                        f.ins().jump(check, &[BlockArg::from(i1)]);
                        f.switch_to_block(after);
                    }
                    let (save, unmark, marked) = (f.create_block(), f.create_block(), f.create_block());
                    let next = mget(&mut f, Mem::Next);
                    let ended = ended_now(&mut f, next);
                    f.ins().brif(ended, unmark, &[], save, &[]);
                    f.switch_to_block(save);
                    let p = pinned(&mut f);
                    for k in Mem::ALL {
                        let x = mget(&mut f, k);
                        f.ins().store(MemFlags::trusted(), x, p, MARK_STATE + 8 * k as i32);
                    }
                    let open = mget(&mut f, Mem::Open);
                    for d in 0..MARK_LEVELS {
                        let (copy, on) = (f.create_block(), f.create_block());
                        let has = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, open, d as i64);
                        f.ins().brif(has, copy, &[], on, &[]);
                        f.switch_to_block(copy);
                        let i = f.ins().iconst(types::I64, d as i64);
                        let x = slot_get(&mut f, open_c, i);
                        f.ins().store(MemFlags::trusted(), x, p, MARK_OPEN + 8 * d as i32);
                        f.ins().jump(on, &[]);
                        f.switch_to_block(on);
                    }
                    let few = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, open, MARK_LEVELS as i64);
                    let few = f.ins().uextend(types::I64, few);
                    f.ins().store(MemFlags::trusted(), few, p, MARK);
                    f.ins().jump(marked, &[]);
                    f.switch_to_block(unmark);
                    let p = pinned(&mut f);
                    f.ins().store(MemFlags::trusted(), zero, p, MARK);
                    f.ins().jump(marked, &[]);
                    f.switch_to_block(marked);
                    let p = pinned(&mut f);
                    let held = f.ins().load(types::I64, MemFlags::trusted(), p, MARK_HELD);
                    guard(&mut f, held, pc);
                    f.ins().jump(fall, &[]);
                }
                Op::Statement => {
                    f.ins().jump(fall, &[]);
                }
                Op::Stop => {
                    let at = f.ins().iconst(types::I64, pc as i64 + 1);
                    f.ins().jump(refuse, &[BlockArg::from(at)]);
                }
                _ => return Err(Error::UnsupportedBackend),
            }
        }
        marks_at.push((usize::MAX, f.func.dfg.num_insts()));
        for w in marks_at.windows(2) {
            word_insts[w[0].0] = (w[1].1 - w[0].1) as u32;
        }
        if clustered {
            // Each place of another cluster a word here goes to: the held fields stored, then
            // the tail call of its cluster with the place's index there. With a function a
            // grammar rule (`pinned_ctx`), the code of an earlier rule a word goes into
            // (`merge_words`): a call of that rule's function, whose result is this one's (the
            // lean parser's `return self.helper()`), to its one exit. A rule goes into earlier
            // rules alone, so these calls nest no deeper than the rules are many. One call for
            // each place gone to, its index made there, however many words go there: an index
            // passed in from many places would be one constant, folded, live through the whole
            // function. A tail call each was an epilogue each (785 restores, against 170).
            use cranelift_codegen::ir::{ExtFuncData, ExternalName, UserExternalName};
            let refs = &mut call_refs;
            let named_foreign: Vec<usize> = foreign.borrow().iter().copied().collect();
            let mut into: std::collections::BTreeMap<(usize, usize), Block> = std::collections::BTreeMap::new();
            for g in named_foreign {
                let d = cluster_of(g);
                // Named only by a word's fall it never takes (no word goes on to it).
                let Some(&i) = index[d].get(&g) else { continue };
                let b = if g < n_words { wb[g].expect("a place named") } else { entries[g - n_words] };
                f.switch_to_block(b);
                if pinned_ctx {
                    let to = *into.entry((d, i)).or_insert_with(|| f.create_block());
                    f.ins().jump(to, &[]);
                    exits.insert(b, g);
                    continue;
                }
                for (k, v) in vars.iter().enumerate() {
                    if let Some(v) = v.filter(|_| !cut.pass.contains(&(k as u32))) {
                        let x = f.use_var(v);
                        let p = pinned(&mut f);
                        f.ins().store(MemFlags::trusted(), x, p, STATE + 8 * k as i32);
                    }
                }
                let passed: Vec<Value> = cut.pass.iter().map(|&k| f.use_var(vars[k as usize].expect("a passed field is held"))).collect();
                let fref = *refs.entry(d).or_insert_with(|| {
                    let name = f.func.declare_imported_user_function(UserExternalName::new(0, cids[d].as_u32()));
                    let signature = f.import_signature(csig.clone());
                    f.import_function(ExtFuncData { name: ExternalName::user(name), signature, colocated: true })
                });
                let iv = f.ins().iconst(types::I64, i as i64);
                let args: Vec<Value> = (!pinned_ctx).then_some(ctx).into_iter().chain([iv]).chain(passed).collect();
                f.ins().return_call(fref, &args);
                exits.insert(b, g);
                tails += 1;
            }
            for ((d, i), to) in into {
                f.switch_to_block(to);
                let iv = f.ins().iconst(types::I64, i as i64);
                for (k, v) in vars.iter().enumerate() {
                    if let Some(v) = v.filter(|_| !cut.pass.contains(&(k as u32))) {
                        let x = f.use_var(v);
                        let p = pinned(&mut f);
                        f.ins().store(MemFlags::trusted(), x, p, STATE + 8 * k as i32);
                    }
                }
                let passed: Vec<Value> = cut.pass.iter().map(|&k| f.use_var(vars[k as usize].expect("a passed field is held"))).collect();
                let fref = *refs.entry(d).or_insert_with(|| {
                    let name = f.func.declare_imported_user_function(UserExternalName::new(0, cids[d].as_u32()));
                    let signature = f.import_signature(csig.clone());
                    f.import_function(ExtFuncData { name: ExternalName::user(name), signature, colocated: true })
                });
                let args: Vec<Value> = std::iter::once(iv).chain(passed).collect();
                let call = f.ins().call(fref, &args);
                let v = f.inst_results(call)[0];
                let r = results(&mut f, v, false);
                leave(&mut f, &r);
            }
            // The entry's place, by its index among this cluster's entries.
            f.switch_to_block(pick);
            for &g in &incoming[c as usize] {
                targets.push((g, if g < n_words { wb[g].expect("an own place") } else { entries[g - n_words] }));
            }
            let at = f.block_params(entry)[first_param];
            if targets.len() == 1 {
                // One entry: no choice to make.
                f.ins().jump(targets[0].1, &[]);
            } else if !cut.pick_traffic.is_empty() {
                // Each entry weighted by the profile's steps into it from other clusters, the
                // hottest tested first (`dispatch_weighted`), not one indirect branch.
                let into: Vec<u64> = targets
                    .iter()
                    .map(|&(g, _)| {
                        let pc = if g < n_words { g } else { shared_all.bodies[g - n_words].start };
                        cut.pick_traffic.iter().filter(|&(&(a, b), _)| b as usize == pc && (a as usize) < n_words && cut.of[a as usize] != c).map(|(_, &w)| w).sum()
                    })
                    .collect();
                let runs: Vec<(u32, u32, Block, u64)> = targets.iter().enumerate().map(|(i, &(_, b))| (i as u32, i as u32 + 1, b, into[i])).collect();
                dispatch_weighted(&mut f, at, &runs);
            } else {
                let i = f.ins().ireduce(types::I32, at);
                let default = f.func.dfg.block_call(off_end, &[]);
                let calls: Vec<_> = targets.iter().map(|&(_, b)| f.func.dfg.block_call(b, &[])).collect();
                let jt = f.create_jump_table(cranelift_codegen::ir::JumpTableData::new(default, &calls));
                f.ins().br_table(i, jt);
            }
        }
        if fin_used {
            f.switch_to_block(fin);
            let p = f.block_params(fin).to_vec();
            let _ = p;
            let r = [zero];
            let r = results(&mut f, r[0], true);
            leave(&mut f, &r);
        }
        // Refusals are cold: placed after the hot code, branched to forward.
        f.switch_to_block(off_end);
        f.set_cold_block(off_end);
        let past = f.ins().iconst(types::I64, words.len() as i64 + 1);
        f.ins().jump(refuse, &[BlockArg::from(past)]);
        f.switch_to_block(refuse);
        f.set_cold_block(refuse);
        let pc = f.block_params(refuse)[0];
        // Inside a shared run, its words are the first place's: the shift makes them this one's.
        let sh = mget(&mut f, Mem::Shift);
        let pc = f.ins().iadd(pc, sh);
        if recovers && !clustered {
            // A statement recovered (decision 0650): on at the root's head.
            let p = pinned(&mut f);
            let ok = recoverable(&mut f, p, &recovery, pc);
            let (again, out) = (f.create_block(), f.create_block());
            f.ins().brif(ok, again, &[], out, &[]);
            f.switch_to_block(again);
            f.set_cold_block(again);
            let get = |f: &mut FunctionBuilder<'_>, k: Mem| mget(f, k);
            let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| mset(f, k, x);
            let at = |f: &mut FunctionBuilder<'_>, at: i32| {
                let b = pinned(f);
                f.ins().load(types::I64, MemFlags::trusted(), b, at)
            };
            // A level to close where none is open: the refusal, as it was (none is: the mark has the root's).
            let to_out = |f: &mut FunctionBuilder<'_>, bad: Value| {
                let ok = f.create_block();
                f.ins().brif(bad, out, &[], ok, &[]);
                f.switch_to_block(ok);
            };
            let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &to_out };
            let p = pinned(&mut f);
            recover_statement(&mut f, &env, p, table, &recovery, &mut heads);
            jump_to_place(&mut f, restart);
            f.switch_to_block(out);
            f.set_cold_block(out);
        } else if recovers {
            // In clusters the driver recovers (below): the state all in `ctx` for it.
            for (k, v) in vars.iter().enumerate() {
                if let Some(v) = v.filter(|_| !cut.pass.contains(&(k as u32))) {
                    let x = f.use_var(v);
                    let p = pinned(&mut f);
                    f.ins().store(MemFlags::trusted(), x, p, STATE + 8 * k as i32);
                }
            }
        }
        let r = results(&mut f, pc, true);
        leave(&mut f, &r);
        f.switch_to_block(exit);
        let r = f.block_params(exit).to_vec();
        if let Some((w, fl)) = entry_kept {
            mset(&mut f, Mem::Word, w);
            mset(&mut f, Mem::Flags, fl);
        }
        if let Some(host) = host {
            f.ins().set_pinned_reg(host);
        }
        f.ins().return_(&r);
        f.seal_all_blocks();
        f.finalize();
        }
        if !pinned_ctx {
            state_in_registers(&mut func);
        }
        drop_dead_params(&mut func);
        drop_unused(&mut func);
        drop_dead_slot_stores(&mut func);
        remat_far(&mut func);
        lowered.push(Lowered { func, heads, targets, exits, tails });
    }
    if times {
        eprintln!("tier 2: lowered {:.2?}", phase.elapsed());
    }
    let cluster_names: Vec<_> = cids.iter().map(|id| cranelift_codegen::ir::UserExternalName::new(0, id.as_u32())).collect();
    // `tempo_stream` itself, in clusters: the state, then the first word's cluster.
    let driver = if clustered {
        let mut heads_d: Vec<(Block, Step)> = Vec::new();
        let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, id.as_u32()), signature.clone());
        {
            let mut f = FunctionBuilder::new(&mut func, &mut fctx);
            let entry = f.create_block();
            f.append_block_params_for_function_params(entry);
            f.switch_to_block(entry);
            let ctx = f.block_params(entry)[0];
            // The state, in `ctx` (`Ctx::state`).
            let zero = f.ins().iconst(types::I64, 0);
            for k in 0..Mem::COUNT {
                f.ins().store(MemFlags::trusted(), zero, ctx, STATE + 8 * k as i32);
            }
            let one = f.ins().iconst(types::I64, 1);
            f.ins().store(MemFlags::trusted(), one, ctx, STATE + 8 * Mem::Need as i32);
            let before_first = f.ins().iconst(types::I64, -1);
            f.ins().store(MemFlags::trusted(), before_first, ctx, STATE + 8 * Mem::Lw as i32);
            if recovers {
                f.ins().store(MemFlags::trusted(), zero, ctx, MARK);
                f.ins().store(MemFlags::trusted(), zero, ctx, MARK_HELD);
            }
            // The first word is its cluster's first entry (`join_clusters`).
            let first = f.ins().iconst(types::I64, 0);
            // The passed fields' first values, as the state's in `ctx`.
            let passed: Vec<Value> = cut.pass.iter().map(|&k| if k == Mem::Need as u32 { one } else if k == Mem::Lw as u32 { before_first } else { zero }).collect();
            // With `ctx` pinned: the host's x21 kept, `ctx` there for the run, the host's back
            // after (Cranelift saves no pinned register).
            let host = pinned_ctx.then(|| {
                let host = f.ins().get_pinned_reg(types::I64);
                f.ins().set_pinned_reg(ctx);
                host
            });
            // The calls open on the machine's stack (`on_stack`): its base and limit, here.
            if on_stack {
                let sp = f.ins().get_stack_pointer(types::I64);
                f.ins().store(MemFlags::trusted(), sp, ctx, STACK_BASE);
                let limit = f.ins().iadd_imm(sp, -STACK_BYTES);
                f.ins().store(MemFlags::trusted(), limit, ctx, STACK_LIMIT);
            }
            let args: Vec<Value> = (!pinned_ctx).then_some(ctx).into_iter().chain([first]).chain(passed).collect();
            let r = call_own(&mut f, cids[cut.of[0] as usize], &csig, &args);
            // The run's end: the first cluster's, or after a statement recovered, the root's head's.
            let ended = f.create_block();
            for _ in 0..r.len() {
                f.append_block_param(ended, types::I64);
            }
            let rs: Vec<BlockArg> = r.iter().map(|&v| BlockArg::from(v)).collect();
            f.ins().jump(ended, &rs);
            f.switch_to_block(ended);
            let r = f.block_params(ended).to_vec();
            // The passed fields' last values, in the state, as the host reads it.
            for (j, &k) in cut.pass.iter().enumerate() {
                f.ins().store(MemFlags::trusted(), r[1 + j], ctx, STATE + 8 * k as i32);
            }
            let end = f.ins().bxor_imm(r[0], ENDED as i64);
            if recovers {
                // A statement recovered (decision 0650), the state in `ctx`: on at the root's head, a call of its
                // cluster at its entry. A loop: a turn a statement.
                heads_d.push((ended, Step::Token));
                let ok = recoverable(&mut f, ctx, &recovery, end);
                let (again, out) = (f.create_block(), f.create_block());
                f.ins().brif(ok, again, &[], out, &[]);
                f.switch_to_block(again);
                f.set_cold_block(again);
                let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
                let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
                let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
                let to_out = |f: &mut FunctionBuilder<'_>, bad: Value| {
                    let ok = f.create_block();
                    f.ins().brif(bad, out, &[], ok, &[]);
                    f.switch_to_block(ok);
                };
                let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &to_out };
                recover_statement(&mut f, &env, ctx, table, &recovery, &mut heads_d);
                let d = cut.of[restart] as usize;
                let iv = f.ins().iconst(types::I64, index[d][&restart] as i64);
                let passed: Vec<Value> = cut.pass.iter().map(|&k| f.ins().load(types::I64, MemFlags::trusted(), ctx, STATE + 8 * k as i32)).collect();
                let args: Vec<Value> = (!pinned_ctx).then_some(ctx).into_iter().chain([iv]).chain(passed).collect();
                let r = call_own(&mut f, cids[d], &csig, &args);
                let rs: Vec<BlockArg> = r.iter().map(|&v| BlockArg::from(v)).collect();
                f.ins().jump(ended, &rs);
                f.switch_to_block(out);
            }
            if let Some(host) = host {
                f.ins().set_pinned_reg(host);
            }
            f.ins().return_(&[end]);
            f.seal_all_blocks();
            f.finalize();
        }
        let own_d: Vec<_> = cluster_names.iter().cloned().chain(table.fns.names()).collect();
        crate::native_cpu_shape::check(&func, &heads_d, &own_d).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
        Some(func)
    } else {
        None
    };
    let mut word_fns: Vec<_> = word_fns.into_values().collect();
    word_fns.sort_by_key(|w| w.0.as_u32());
    // A format 16 run's word code calls the writer's ops too (a failure, `Table::uncode`).
    let ops = table_fns.sixteen.as_ref().map(|s| s.ops.names()).unwrap_or_default();
    let own: Vec<_> = table_fns.all().chain(word_fns).collect();
    let names: Vec<_> = own.iter().map(|(id, _, _)| cranelift_codegen::ir::UserExternalName::new(0, id.as_u32())).chain(ops).collect();
    let crossings: usize = lowered.iter().map(|l| l.tails).sum();
    if clustered {
        check_across(&lowered)?;
    }
    for l in &lowered {
        // With native calls, a rule's function calls the others as well as tail-calling them.
        let callable: Vec<_> = names.iter().chain(if cut.calls { &cluster_names[..] } else { &[] }).cloned().collect();
        crate::native_cpu_shape::check_tails(&l.func, &l.heads, &callable, if clustered { &cluster_names } else { &[] }).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
    }
    if times {
        eprintln!("tier 2: joined and checked {:.2?}", phase.elapsed());
    }
    // The token loop's function first (a driver, in clusters), then the clusters', then the
    // outlined functions: compiled at once (`define_functions`), their IR and machine code
    // appended in that order.
    let mut heads: Vec<(String, Step)> = Vec::new();
    let mut items: Vec<(cranelift_module::FuncId, Context)> = Vec::new();
    let context_of = |module: &JITModule, func: cranelift_codegen::ir::Function| {
        let mut c: Context = module.make_context();
        c.func = func;
        c.set_disasm(true);
        c
    };
    let has_driver = driver.is_some();
    if let Some(func) = driver {
        items.push((id, context_of(&module, func)));
    }
    for (c, l) in lowered.into_iter().enumerate() {
        heads.extend(l.heads.iter().map(|(b, s)| (if clustered { format!("c{c}:{b}") } else { b.to_string() }, *s)));
        items.push((cids[c], context_of(&module, l.func)));
    }
    for (fid, _, func) in own {
        items.push((fid, context_of(&module, func)));
    }
    // Each function's constants folded (`clif_fold`) before either backend is given it.
    {
        let mut funcs: Vec<&mut cranelift_codegen::ir::Function> = items.iter_mut().map(|(_, c)| &mut c.func).collect();
        crate::clif_fold::fold_all(&mut funcs);
    }
    let mut clif = String::new();
    for (_, c) in &items {
        clif.push_str(&c.func.display().to_string());
    }
    let defined = crate::native_cpu_codegen::define_functions(&mut module, items, cut.near).map_err(|e| {
        if std::env::var_os("RERAIL_DEBUG_STREAM").is_some() {
            eprintln!("tier 2: {e:?}");
        }
        Error::BackendFailure
    })?;
    if times {
        eprintln!("tier 2: compiled {:.2?}", phase.elapsed());
    }
    // The smallest rule function's frame (its saved registers, slots, and the frame pointer and
    // return address): each call open has one, or more (`calls_open_by_stack`).
    let frame_shift = defined
        .iter()
        .skip(usize::from(has_driver))
        .take(cut.count as usize)
        .filter_map(|c| c.compiled_code().map(|code| code.frame_size + 16))
        .min()
        .map_or(4, |least| u64::from(least.max(16).ilog2()));
    let mut disassembly = String::new();
    let mut code_bytes = Vec::new();
    let mut word_bytes = vec![0u32; words.len()];
    let mut word_ranges = Vec::new();
    for c in &defined {
        if let Some(code) = c.compiled_code() {
            for l in code.buffer.get_srclocs_sorted() {
                if let Some(pc) = (l.loc.bits() as usize).checked_sub(1).filter(|&pc| pc < words.len()) {
                    word_bytes[pc] += l.end - l.start;
                    let at = code_bytes.len() as u32;
                    word_ranges.push((pc as u32, at + l.start, at + l.end));
                }
            }
        }
        disassembly.push_str(&c.compiled_code().and_then(|code| code.vcode.clone()).unwrap_or_default());
        code_bytes.extend_from_slice(c.compiled_code().map_or(&[][..], |code| code.code_buffer()));
    }
    // A call's line, its source location (`@` and the word's place) before it or not.
    let calls = clif.lines().filter(|l| l.trim().starts_with("call ") || l.contains(" call ")).count();
    module.finalize_definitions().map_err(|e| {
        if std::env::var_os("RERAIL_DEBUG_STREAM").is_some() {
            eprintln!("tier 2's module: {e:?}");
        }
        Error::BackendFailure
    })?;
    let code = module.get_finalized_function(id);
    // SAFETY: `tempo_stream` was declared and defined with this signature.
    let entry = unsafe { std::mem::transmute::<*const u8, Entry>(code) };
    let own_calls = calls;
    let table_passes = crate::clif_text::callees(&clif).iter().filter(|n| passes.contains(n)).count();
    let row_calls = crate::clif_text::callees(&clif).iter().filter(|n| row_fns.contains(n)).count();
    let direct = direct.is_some();
    let marks = marks_of(words);
    let wraps = framed(words);
    let two_slots = has_context(words);
    Ok(CompiledStream { _module: module, _split: split, _fetch: fetch, _limits: limits, entry, frame_shift, bounds: bounds(native), starts, lasts, marks, part_scans: { all_scans.sort_unstable(); all_scans }, part_onward: { all_onward.sort_unstable(); all_onward }, fused_checks: { all_fused.sort_unstable(); all_fused }, static_checks: { all_static.sort_unstable(); all_static }, wraps, widths, tag, classes: supertype_classes(native)?, extras: native.readings.extras.clone(), sixteen, context: two_slots, clif, disassembly, code_bytes, heads, own_calls, table_passes, row_calls, direct, open_bound: if sixteen && direct { crate::stream_facts::open_bound(native, widths, tag) } else { None }, clusters: cut.count as usize, crossings, word_insts, word_bytes, word_ranges })
}

/// What an instruction does to `ctx`'s fields (`state_in_registers`).
#[derive(Clone, Copy, PartialEq)]
enum Access {
    Load(usize),
    Store(usize, Value),
    /// A narrow load (the record's byte), or a field not on a word boundary.
    Reads(u64),
    /// A narrow store: reads (not all of it written) and changes it.
    Clobbers(usize),
    /// A call, a return, `ctx` elsewhere: every field.
    All,
    None,
}

/// The fields' constants (`state_in_registers`): forward, to a fixed point, each field at each
/// block's start is a constant every predecessor ends with, or not known. A load of a known
/// field is its constant; a branch on a constant is a jump. Whether anything changed.
fn constant_fields(func: &mut cranelift_codegen::ir::Function, access: &dyn Fn(&cranelift_codegen::ir::Function, cranelift_codegen::ir::Inst) -> Access) -> bool {
    use cranelift_codegen::cursor::{Cursor, FuncCursor};
    use cranelift_codegen::ir::{InstBuilder, InstructionData, Opcode};
    #[derive(Clone, Copy, PartialEq)]
    enum C {
        /// No path seen yet.
        Top,
        Is(i64),
        Unknown,
    }
    let meet = |a: C, b: C| match (a, b) {
        (C::Top, x) | (x, C::Top) => x,
        (C::Is(x), C::Is(y)) if x == y => C::Is(x),
        _ => C::Unknown,
    };
    let constant = |func: &cranelift_codegen::ir::Function, v: Value| -> Option<i64> {
        let v = func.dfg.resolve_aliases(v);
        match func.dfg.value_def(v) {
            cranelift_codegen::ir::ValueDef::Result(i, _) => match func.dfg.insts[i] {
                InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } => Some(imm.bits()),
                _ => None,
            },
            _ => None,
        }
    };
    let n = (STATE / 8 + Mem::COUNT as i32) as usize;
    let cfg = cranelift_codegen::flowgraph::ControlFlowGraph::with_function(func);
    let dom = cranelift_codegen::dominator_tree::DominatorTree::with_function(func, &cfg);
    let order: Vec<Block> = dom.cfg_rpo().copied().collect();
    let Some(entry) = func.layout.entry_block() else { return false };
    let transfer = |func: &cranelift_codegen::ir::Function, b: Block, mut at: Vec<C>| -> Vec<C> {
        for inst in func.layout.block_insts(b) {
            match access(func, inst) {
                Access::Store(k, v) => at[k] = constant(func, v).map_or(C::Unknown, C::Is),
                Access::Clobbers(k) => at[k] = C::Unknown,
                Access::All => at.iter_mut().for_each(|x| *x = C::Unknown),
                _ => {}
            }
        }
        at
    };
    let mut start: std::collections::HashMap<Block, Vec<C>> = order.iter().map(|&b| (b, vec![C::Top; n])).collect();
    start.insert(entry, vec![C::Unknown; n]);
    let mut end: std::collections::HashMap<Block, Vec<C>> = std::collections::HashMap::new();
    loop {
        let mut changed = false;
        for &b in &order {
            let at = if b == entry {
                vec![C::Unknown; n]
            } else {
                let mut at = vec![C::Top; n];
                for p in cfg.pred_iter(b) {
                    if let Some(e) = end.get(&p.block) {
                        for k in 0..n {
                            at[k] = meet(at[k], e[k]);
                        }
                    }
                }
                at
            };
            if start.get(&b) != Some(&at) || !end.contains_key(&b) {
                let e = transfer(func, b, at.clone());
                start.insert(b, at);
                if end.get(&b) != Some(&e) {
                    end.insert(b, e);
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // The loads of known fields, then the branches on constants.
    let mut any = false;
    for &b in &order {
        let mut at = start[&b].clone();
        let insts: Vec<_> = func.layout.block_insts(b).collect();
        for inst in insts {
            let a = access(func, inst);
            if let Access::Load(k) = a
                && let C::Is(c) = at[k]
            {
                let r = func.dfg.inst_results(inst)[0];
                let mut cur = FuncCursor::new(func).at_inst(inst);
                let x = cur.ins().iconst(types::I64, c);
                func.dfg.detach_inst_results(inst);
                func.dfg.change_to_alias(r, x);
                func.layout.remove_inst(inst);
                any = true;
                continue;
            }
            // A store of the constant the field holds: nothing changes.
            if let Access::Store(k, v) = a
                && let Some(c) = constant(func, v)
                && at[k] == C::Is(c)
            {
                func.layout.remove_inst(inst);
                any = true;
                continue;
            }
            match a {
                Access::Store(k, v) => at[k] = constant(func, v).map_or(C::Unknown, C::Is),
                Access::Clobbers(k) => at[k] = C::Unknown,
                Access::All => at.iter_mut().for_each(|x| *x = C::Unknown),
                _ => {}
            }
        }
        if let Some(last) = func.layout.last_inst(b)
            && let InstructionData::Brif { arg, blocks: [then, other], .. } = func.dfg.insts[last]
            && let Some(c) = constant(func, arg)
        {
            let taken = if c != 0 { then } else { other };
            let target = taken.block(&func.dfg.value_lists);
            let args: Vec<cranelift_codegen::ir::BlockArg> = taken.args(&func.dfg.value_lists).collect();
            func.layout.remove_inst(last);
            let mut cur = FuncCursor::new(func).at_bottom(b);
            cur.ins().jump(target, &args);
            any = true;
        }
    }
    any
}

/// `ctx`'s fields as the lean parser's locals: within a function, a field's value goes from
/// the word that stores it to the word that reads it in a register, not through memory.
///
/// - **Forwarded:** along a chain of blocks each of one predecessor (a word falling into the
///   next), a field's load after its store or an earlier load is that value. A call ends what
///   is known (the callee may write any field).
/// - **Dead stores:** a store every path writes again before any read is removed (a backward
///   pass over the fields, a bit each). A call or a return reads every field (the callee and
///   the host read the state).
///
/// The fields are `ctx`'s, at fixed offsets from its address (the function's first argument);
/// the tape's buffers are elsewhere. A function where `ctx` goes anywhere but a load's or a
/// store's address, or a call, is left as it is.
fn state_in_registers(func: &mut cranelift_codegen::ir::Function) {
    use cranelift_codegen::ir::{InstructionData, Opcode};
    let Some(entry) = func.layout.entry_block() else { return };
    let ctx = func.dfg.block_params(entry)[0];
    // Field `k` at offset `8k`: a full load or store of it, or another access of it.
    const FIELDS: i32 = STATE / 8 + Mem::COUNT as i32;
    const _: () = assert!(FIELDS <= 64);
    let field_of = |off: i32| -> Option<usize> { (0..8 * FIELDS).contains(&off).then_some((off / 8) as usize) };
    let access = |func: &cranelift_codegen::ir::Function, inst: cranelift_codegen::ir::Inst| -> Access {
        let data = &func.dfg.insts[inst];
        let op = data.opcode();
        if op.is_call() || op.is_return() {
            return Access::All;
        }
        match *data {
            InstructionData::Load { opcode, arg, offset, .. } if func.dfg.resolve_aliases(arg) == ctx => {
                let off = i32::from(offset);
                let Some(k) = field_of(off) else { return Access::All };
                let full = opcode == Opcode::Load && off % 8 == 0 && func.dfg.value_type(func.dfg.inst_results(inst)[0]) == types::I64;
                if full { Access::Load(k) } else { Access::Reads(1 << k) }
            }
            InstructionData::Store { opcode, args, offset, .. } if func.dfg.resolve_aliases(args[1]) == ctx => {
                let off = i32::from(offset);
                let Some(k) = field_of(off) else { return Access::All };
                if func.dfg.resolve_aliases(args[0]) == ctx {
                    return Access::All;
                }
                let full = opcode == Opcode::Store && off % 8 == 0 && func.dfg.value_type(args[0]) == types::I64;
                if full { Access::Store(k, args[0]) } else { Access::Clobbers(k) }
            }
            _ => {
                if func.dfg.inst_values(inst).any(|v| func.dfg.resolve_aliases(v) == ctx) {
                    Access::All
                } else {
                    Access::None
                }
            }
        }
    };
    let blocks: Vec<Block> = func.layout.blocks().collect();
    // `ctx` as a branch's argument: another name for it, which this does not follow.
    for &b in &blocks {
        for inst in func.layout.block_insts(b) {
            if func.dfg.insts[inst].opcode().is_branch() && func.dfg.inst_values(inst).any(|v| func.dfg.resolve_aliases(v) == ctx) {
                return;
            }
        }
    }
    // Constants: a field every path into a word gives one constant (the transducer's `need`
    // after a `SPAN`, `pending` after a node) is that constant there, and a branch on it is a
    // jump. Folded branches make more chains of one predecessor: again, a few times.
    for _ in 0..4 {
        if !constant_fields(func, &access) {
            break;
        }
    }
    let cfg = cranelift_codegen::flowgraph::ControlFlowGraph::with_function(func);
    let dom = cranelift_codegen::dominator_tree::DominatorTree::with_function(func, &cfg);
    let order: Vec<Block> = dom.cfg_rpo().copied().collect();

    // Forwarded, in reverse postorder: a block of one predecessor starts from what its
    // predecessor knew at its end.
    let mut known_at_end: std::collections::HashMap<Block, Vec<Option<Value>>> = std::collections::HashMap::new();
    for &b in &order {
        let preds: Vec<Block> = cfg.pred_iter(b).map(|p| p.block).collect();
        let mut known: Vec<Option<Value>> = match preds.as_slice() {
            [p] if *p != b => known_at_end.get(p).cloned().unwrap_or_else(|| vec![None; FIELDS as usize]),
            _ => vec![None; FIELDS as usize],
        };
        let insts: Vec<_> = func.layout.block_insts(b).collect();
        for inst in insts {
            match access(func, inst) {
                Access::Load(k) => {
                    let r = func.dfg.inst_results(inst)[0];
                    match known[k] {
                        Some(v) => {
                            func.dfg.detach_inst_results(inst);
                            func.dfg.change_to_alias(r, v);
                            func.layout.remove_inst(inst);
                        }
                        None => known[k] = Some(r),
                    }
                }
                // A store of the value the field holds (loaded or stored before): none.
                Access::Store(k, v) if known[k].is_some_and(|x| func.dfg.resolve_aliases(x) == func.dfg.resolve_aliases(v)) => func.layout.remove_inst(inst),
                Access::Store(k, v) => known[k] = Some(v),
                Access::Clobbers(k) => known[k] = None,
                Access::All => known.iter_mut().for_each(|x| *x = None),
                Access::Reads(_) | Access::None => {}
            }
        }
        known_at_end.insert(b, known);
    }

    // Dead stores: the fields live (read before written again) at each block's start, to a
    // fixed point, then each block backward.
    let all = if FIELDS == 64 { u64::MAX } else { (1u64 << FIELDS) - 1 };
    let step = |a: Access, live: u64| -> u64 {
        match a {
            Access::Load(k) => live | 1 << k,
            Access::Store(k, _) => live & !(1 << k),
            Access::Reads(m) => live | m,
            Access::Clobbers(k) => live | 1 << k,
            Access::All => all,
            Access::None => live,
        }
    };
    let succs = |func: &cranelift_codegen::ir::Function, b: Block| -> Vec<Block> {
        let Some(last) = func.layout.last_inst(b) else { return Vec::new() };
        func.dfg.insts[last].branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables).iter().map(|c| c.block(&func.dfg.value_lists)).collect()
    };
    let accesses: std::collections::HashMap<Block, Vec<(cranelift_codegen::ir::Inst, Access)>> =
        blocks.iter().map(|&b| (b, func.layout.block_insts(b).map(|i| (i, access(func, i))).collect())).collect();
    let next: std::collections::HashMap<Block, Vec<Block>> = blocks.iter().map(|&b| (b, succs(func, b))).collect();
    let mut live_in: std::collections::HashMap<Block, u64> = blocks.iter().map(|&b| (b, 0)).collect();
    loop {
        let mut changed = false;
        for &b in order.iter().rev() {
            let mut live = next[&b].iter().fold(0, |m, s| m | live_in[s]);
            for &(_, a) in accesses[&b].iter().rev() {
                live = step(a, live);
            }
            if live != live_in[&b] {
                live_in.insert(b, live);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for &b in &order {
        let mut live = next[&b].iter().fold(0, |m, s| m | live_in[s]);
        for &(inst, a) in accesses[&b].iter().rev() {
            if let Access::Store(k, _) = a
                && live & 1 << k == 0
            {
                func.layout.remove_inst(inst);
                continue;
            }
            live = step(a, live);
        }
    }
}

/// Every load that can not trap and whose value nothing reads, removed, until none is left. A
/// word loads the `ctx` fields it may read where it starts (`Cut::reload`); those it does not
/// read on a path are dead, but Cranelift keeps a load that is not read-only (it orders it with
/// the stores). A load that can not trap has no effect but its value: removing an unread one
/// changes nothing, as the lean parser's code loads a field only where it is used.
/// Values made again where they are used (rematerialized), not kept live from far away: a
/// constant used in a block other than its own, and where `ctx` is in the pinned register (the
/// one function, `pin_one`), a read-only load from it (a `ctx` field) likewise, from the
/// register. One value through the function held a register (or a spill slot) through all of
/// it, as the constant zero every word uses did, against the loop's state. Once a block, before
/// its first use there.
fn remat_far(func: &mut cranelift_codegen::ir::Function) {
    use cranelift_codegen::cursor::{Cursor, FuncCursor};
    use cranelift_codegen::ir::{InstructionData, Opcode};
    let entry = func.layout.entry_block().expect("an entry");
    let param = func.dfg.block_params(entry).first().copied();
    // `ctx` in the pinned register: the entry sets it from its parameter.
    let pinned = func.layout.block_insts(entry).any(|i| func.dfg.insts[i].opcode() == Opcode::SetPinnedReg && func.dfg.inst_args(i).first().map(|&v| func.dfg.resolve_aliases(v)) == param);
    let blocks: Vec<Block> = func.layout.blocks().collect();
    for b in blocks {
        let mut made: std::collections::HashMap<Value, Value> = std::collections::HashMap::new();
        let insts: Vec<cranelift_codegen::ir::Inst> = func.layout.block_insts(b).collect();
        for inst in insts {
            let mut vals: Vec<Value> = func.dfg.inst_values(inst).map(|v| func.dfg.resolve_aliases(v)).collect();
            vals.dedup();
            for v in vals {
                if made.contains_key(&v) {
                    continue;
                }
                let Some(d) = func.dfg.value_def(v).inst() else { continue };
                if func.layout.inst_block(d) == Some(b) {
                    continue;
                }
                let data = func.dfg.insts[d];
                let ty = func.dfg.value_type(v);
                let new = match data {
                    InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } => {
                        let mut pos = FuncCursor::new(func).at_inst(inst);
                        Some(pos.ins().iconst(ty, imm))
                    }
                    InstructionData::Load { opcode: Opcode::Load, arg, flags, offset }
                        if pinned && flags.readonly() && flags.can_move() && Some(func.dfg.resolve_aliases(arg)) == param =>
                    {
                        let mut pos = FuncCursor::new(func).at_inst(inst);
                        let base = pos.ins().get_pinned_reg(types::I64);
                        Some(pos.ins().load(ty, flags, base, offset))
                    }
                    _ => None,
                };
                if let Some(n) = new {
                    made.insert(v, n);
                }
            }
            let raw: std::collections::HashMap<Value, Value> = func.dfg.inst_values(inst).filter_map(|x| made.get(&func.dfg.resolve_aliases(x)).map(|&n| (x, n))).collect();
            if !raw.is_empty() {
                func.dfg.map_inst_values(inst, |x| raw.get(&x).copied().unwrap_or(x));
            }
        }
    }
}

/// Stores to an explicit stack slot at an offset no `stack_load` of that slot reads, where no
/// `stack_addr` takes the slot's address, dropped: state a program never reads (the marks'
/// `Live` in a program with no mark) is not written a token.
fn drop_dead_slot_stores(func: &mut cranelift_codegen::ir::Function) {
    use cranelift_codegen::ir::{InstructionData, Opcode};
    let mut read: std::collections::HashSet<(cranelift_codegen::ir::StackSlot, i64)> = std::collections::HashSet::new();
    let mut escaped: std::collections::HashSet<cranelift_codegen::ir::StackSlot> = std::collections::HashSet::new();
    let blocks: Vec<Block> = func.layout.blocks().collect();
    for &b in &blocks {
        for inst in func.layout.block_insts(b) {
            match func.dfg.insts[inst] {
                InstructionData::StackLoad { opcode: Opcode::StackLoad, stack_slot, offset } => {
                    read.insert((stack_slot, i64::from(offset)));
                }
                InstructionData::StackLoad { stack_slot, .. } => {
                    escaped.insert(stack_slot);
                }
                _ => {}
            }
        }
    }
    let mut dead = Vec::new();
    for &b in &blocks {
        for inst in func.layout.block_insts(b) {
            if let InstructionData::StackStore { opcode: Opcode::StackStore, stack_slot, offset, .. } = func.dfg.insts[inst]
                && !escaped.contains(&stack_slot)
                && !read.contains(&(stack_slot, i64::from(offset)))
            {
                dead.push(inst);
            }
        }
    }
    for inst in dead {
        func.layout.remove_inst(inst);
    }
}

/// Block parameters no instruction reads, dropped with their arguments: a parameter is read
/// where an instruction other than a branch's block call uses it, or where it is passed on to a
/// parameter that is read. A field held as an SSA value (`HELD_IN_ONE`) is a parameter of every
/// block it reaches, and a value written and never read before its next write (a part's code
/// where the word after takes no code) kept its load alive through it (`drop_unused`).
fn drop_dead_params(func: &mut cranelift_codegen::ir::Function) {
    use cranelift_codegen::ir::{BlockArg, Opcode};
    let Some(entry) = func.layout.entry_block() else { return };
    let blocks: Vec<Block> = func.layout.blocks().collect();
    let mut live: std::collections::HashSet<Value> = func.dfg.block_params(entry).iter().copied().collect();
    let mut edges: Vec<(Value, Value)> = Vec::new();
    for &b in &blocks {
        for inst in func.layout.block_insts(b) {
            let d = &func.dfg.insts[inst];
            if matches!(d.opcode(), Opcode::TryCall | Opcode::TryCallIndirect) {
                return;
            }
            for &v in d.arguments(&func.dfg.value_lists) {
                live.insert(func.dfg.resolve_aliases(v));
            }
            for call in d.branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables) {
                let params = func.dfg.block_params(call.block(&func.dfg.value_lists));
                for (a, &p) in call.args(&func.dfg.value_lists).zip(params) {
                    if let BlockArg::Value(v) = a {
                        edges.push((p, func.dfg.resolve_aliases(v)));
                    }
                }
            }
        }
    }
    loop {
        let mut grew = false;
        for &(p, v) in &edges {
            if live.contains(&p) && live.insert(v) {
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let dead: std::collections::HashMap<Block, Vec<usize>> = blocks
        .iter()
        .filter(|&&b| b != entry)
        .filter_map(|&b| {
            let d: Vec<usize> = func.dfg.block_params(b).iter().enumerate().filter(|(_, p)| !live.contains(p)).map(|(i, _)| i).collect();
            (!d.is_empty()).then_some((b, d))
        })
        .collect();
    if dead.is_empty() {
        return;
    }
    for &b in &blocks {
        let insts: Vec<cranelift_codegen::ir::Inst> = func.layout.block_insts(b).collect();
        for inst in insts {
            let dfg = &mut func.dfg;
            for call in dfg.insts[inst].branch_destination_mut(&mut dfg.jump_tables, &mut dfg.exception_tables) {
                if let Some(d) = dead.get(&call.block(&dfg.value_lists)) {
                    for &i in d.iter().rev() {
                        call.remove(i, &mut dfg.value_lists);
                    }
                }
            }
        }
    }
    for (b, d) in dead {
        for &i in d.iter().rev() {
            let p = func.dfg.block_params(b)[i];
            func.dfg.remove_block_param(p);
        }
    }
}

/// Instructions whose results nothing uses, dropped: a load that cannot trap, and one with no
/// effect but its results. To a fixpoint, so a chain of them goes.
fn drop_unused(func: &mut cranelift_codegen::ir::Function) {
    loop {
        let mut used = std::collections::HashSet::new();
        let blocks: Vec<Block> = func.layout.blocks().collect();
        for &b in &blocks {
            for inst in func.layout.block_insts(b) {
                for v in func.dfg.inst_values(inst) {
                    used.insert(func.dfg.resolve_aliases(v));
                }
            }
        }
        let mut dead = Vec::new();
        for &b in &blocks {
            for inst in func.layout.block_insts(b) {
                let data = &func.dfg.insts[inst];
                let op = data.opcode();
                let quiet = data.memflags().is_some_and(|m| m.trap_code().is_none());
                let unused = !func.dfg.inst_results(inst).is_empty() && func.dfg.inst_results(inst).iter().all(|r| !used.contains(r));
                // A load that cannot trap, or an instruction with no effect but its results
                // (whose operands a load gave).
                let pure = !op.can_load() && !op.can_store() && !op.is_call() && !op.is_branch() && !op.is_terminator() && !op.other_side_effects() && !op.can_trap();
                if unused && ((op.can_load() && !op.can_store() && !op.is_call() && quiet) || pure) {
                    dead.push(inst);
                }
            }
        }
        if dead.is_empty() {
            return;
        }
        for inst in dead {
            func.layout.remove_inst(inst);
        }
    }
}

/// The loops across clusters, checked as a function's are: from each place a cluster is
/// entered at, the tail calls it reaches before any loop head; over all clusters, no cycle of
/// those (`native_cpu_shape::acyclic`).
fn check_across(lowered: &[Lowered]) -> Result<(), Error> {
    let mut edges: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for l in lowered {
        let blocks: Vec<Block> = l.targets.iter().map(|&(_, b)| b).collect();
        for (&(g, _), reach) in l.targets.iter().zip(crate::native_cpu_shape::exits(&l.func, &l.heads, &blocks, &l.exits)) {
            edges.insert(g, reach);
        }
    }
    crate::native_cpu_shape::acyclic(&edges).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// The `ctx` fields a word of the hot path reads (`Cut::reload`); none for the words that read
/// many and are cold (`cold_op`: they load every field).
fn ctx_reads(op: &Op, wrapping: bool) -> Vec<i32> {
    let mut r: Vec<i32> = match op {
        // With any mark's mask, `TOKEN` reads it (an output).
        Op::Token { .. } => vec![STARTS, NSTARTS, LASTS, CODES, ROOM_NODES, ROOM_SUPERS, ROOM_PUSHES, OUTPUTS, OUTPUT_STRIDE, LASTS_STRIDE],
        Op::Switch { .. } => vec![BYTES],
        Op::Mark { .. } | Op::Row { .. } => vec![OUTPUTS, OUTPUT_STRIDE, LASTS_STRIDE],
        Op::SettleBefore { .. } => vec![CODES, RECORD_STRIDE, ROWS],
        Op::DropLast { .. } => vec![CODES, STARTS, BYTES, NSTARTS, RECORD_STRIDE],
        Op::LoseLast { .. } => vec![CODES, STARTS, BYTES, NSTARTS, RECORD_STRIDE, ROWS],
        Op::RefuseIf { .. } => vec![CODES, STARTS, BYTES, NSTARTS, RECORD_STRIDE],
        Op::MoveFirst { .. } => vec![CODES, STARTS, BYTES, NSTARTS, RECORD_STRIDE, ROWS],
        Op::Open { .. } => vec![ROWS, OPEN],
        Op::Close { .. } => vec![OPEN, ROWS],
        Op::Leaf { .. } => vec![ROWS],
        Op::Retag { on: On::Moved, .. } => vec![ROWS],
        Op::Retag { on: On::Here, .. } => vec![OPEN, ROWS],
        Op::Merge { .. } => vec![OPEN, ROWS],
        Op::NodeField { on: On::Moved | On::Here, .. } => vec![ROWS],
        Op::NodeSuper { .. } => vec![SUPER_NODE, SUPER_KIND],
        Op::Push { .. } | Op::Return { .. } => vec![RETURNS],
        Op::Empty { .. } => vec![TOTALS, TOTALS_STRIDE, TOTALS_MASK],
        _ => vec![],
    };
    if wrapping && matches!(op, Op::Open { .. } | Op::Close { .. } | Op::Leaf { .. } | Op::Merge { .. } | Op::LoseLast { .. }) {
        r.extend([FRAME_BASE, HEAD, LINK, PLACE]);
    }
    r
}

/// The words that read many `ctx` fields and run rarely: each loads them all (`Cut::reload`).
fn cold_op(op: &Op) -> bool {
    matches!(op, Op::Accept | Op::Wrap { .. } | Op::Reread { .. } | Op::Retag { on: On::Made | On::Inner, .. } | Op::NodeField { on: On::Made | On::MadeUnset | On::Inner, .. })
}

/// Whether the program has context beside each call's return (a level's word, its flags): two
/// return slots a call.
fn has_context(words: &[crate::native_ir::StructuralBlock]) -> bool {
    words.iter().any(|b| matches!(b.operation, Op::Remember { .. } | Op::Flag { .. } | Op::Push { flag: Some(_), .. }))
}

/// Whether the program keeps a frame per open node: it wraps, or it relabels the node made last
/// at a level (`On::Made`).
fn framed(words: &[crate::native_ir::StructuralBlock]) -> bool {
    words.iter().any(|b| matches!(b.operation, Op::Wrap { .. } | Op::Reread { .. } | Op::Retag { on: On::Made, .. } | Op::NodeField { on: On::Made | On::MadeUnset | On::Inner, .. }))
}

/// Tier 1's buffers as tier 2 reads them, where tier 1 wrote them (the CPU's batch, or the
/// device's buffers in shared memory): the starts (two entries past `nstarts`, each the file's length, as
/// simdjson's stage 1 ends its index: `Batch::end_streams`); the
/// mask's words, word `i` at `lasts + i * lasts_stride` bytes, `nlasts` of them; output `o`'s
/// words from `outputs + o * output_stride` bytes, a word every `lasts_stride`; total `t` the
/// bits `totals_mask` of the 8 readable bytes at `totals + t * totals_stride`.
#[derive(Clone, Copy)]
pub struct Tier1View {
    pub starts: *const u32,
    pub nstarts: usize,
    /// The records' last bytes and codes, zipped with `starts` (`records`).
    pub lasts: *const u32,
    pub codes: *const u32,
    pub lasts_stride: usize,
    pub outputs: *const u64,
    pub output_stride: usize,
    pub totals: *const u8,
    pub totals_stride: usize,
    pub totals_mask: u64,
    /// The bytes from a token's record field to the next token's (`records::RecordLayout`).
    pub record_stride: usize,
    /// Tier 1's feed (`Ctx::more`): zero where tier 1 made every record.
    pub more: u64,
    pub feed: u64,
}

/// The records a call reads at most: tier 1's count, or with a feed (decision 0496) a token a byte and the end.
fn fed_bound(tier1: &Tier1View, len: usize) -> usize {
    if tier1.feed != 0 { len + 1 } else { tier1.nstarts }
}

/// The buffers one call writes, sized from the stream and the program's `Bounds`.
pub struct Tape {
    /// The working rows, 12 bytes a node (key, start, end): never part of the tape.
    rows: Vec<u8>,
    /// The tape's columns at the compiled widths (`SLACK` bytes past each), the bases, and the
    /// image: the header's room, the code column in place, then room for the columns moved
    /// after it and the code table (`native_cpu_tape::finish`).
    size: Vec<u8>,
    off: Vec<u8>,
    tail: Vec<u8>,
    base: Vec<u32>,
    pub image: Vec<u8>,
    /// The code ids: each key's (the thread's table, cleared by the image's end), each code's key
    /// and first node.
    id_of: Vec<u16>,
    code_kf: Vec<u32>,
    code_first: Vec<u32>,
    pub super_node: Vec<u32>,
    pub super_kind: Vec<u32>,
    open: Vec<u32>,
    returns: Vec<u32>,
    /// The node count, the supertype count, the image's length.
    pub counts: [u64; 3],
    /// A program with `WRAP`'s: frames, chains, places (a drop's stack: two slots a node).
    frames: [Vec<u32>; 3],
    chains: [Vec<u32>; 3],
    /// The nodes, supertypes and calls it has room for: the code's room checks (`Ctx::room_nodes`). A format 16 run
    /// holds no buffer of format 15's (its image, columns and code ids), and a direct one (`CompiledStream::direct`) no
    /// working row either, so the room is not a buffer's.
    room: [usize; 3],
    /// A format 16 run's writer state and numbers (`Ctx::writer`, `Ctx::numbers`): null in a format 15 run.
    writer: *mut u8,
    numbers: *const u32,
}

thread_local! {
    /// The code ids by key, kept between a thread's tapes: each image's end clears what it set.
    static IDS: std::cell::RefCell<Vec<u16>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl Drop for Tape {
    /// The id table back to the thread. A run that stopped before its end clears what it set.
    fn drop(&mut self) {
        // A format 16 run's tape took no table (`sized`).
        if self.id_of.capacity() == 0 {
            return;
        }
        let mut ids = std::mem::take(&mut self.id_of);
        // A format 16 run numbers no codes: the table is as it found it.
        if self.counts[2] == 0 && self.writer.is_null() {
            ids.fill(crate::native_cpu_tape::NO_CODE as u16);
        }
        IDS.with(|t| *t.borrow_mut() = ids);
    }
}

impl CompiledStream {
    /// `lang`'s numbers by this program's keys (`Numbers`): each entry of its table that a key names (no alias, a
    /// kind and field within a key's bits, its flags the extra's where its kind is one, its classes among the
    /// program's), at the key.
    pub fn numbers(&self, lang: &rerail_tape_format::f16::Lang) -> Numbers {
        Numbers(numbers_by_key(lang, &self.classes, &self.extras))
    }

    /// The widths the tape is written at, and the language's tag.
    pub fn widths_tag(&self) -> (rerail_tape_format::Widths, u8) {
        (self.widths, self.tag)
    }

    /// Room for a stream of `n` starts: from the token count, as the lean parser sizes its
    /// tape (half again as many nodes as tokens; on the `.rerail` corpus, 0.76 a token), and a
    /// token's path past it. A run that could pass it stops at a token word (`OVERFLOW`) and
    /// `call` runs it again on `full_tape`.
    pub fn tape(&self, n: usize) -> Tape {
        if self.bounds.unchecked(self.marks) {
            return self.full_tape(n);
        }
        let b = &self.bounds;
        let tokens = n as u64 + 2;
        self.sized((tokens + tokens / 2 + b.nodes + 1) as usize, (tokens / 2 + b.supers + 1) as usize, (tokens / 4 + b.pushes + 64) as usize)
    }

    /// `TOKEN` checks the tape's room (`Bounds::unchecked`: not where the run's tape is the
    /// bound's from the start).
    pub fn room_checked(&self) -> bool {
        !self.bounds.unchecked(self.marks)
    }

    /// The most tokens a file of `len` bytes with `records` records gives: its records, or where
    /// `TOKEN` splits a region (`SplitParts`, decision 0323), each part too, a byte at least.
    pub fn tokens(&self, records: usize, len: usize) -> usize {
        if self._split.is_some() { records + len } else { records }
    }

    /// A tape with the program's bound's room for `n` starts: no run passes it.
    fn full_tape(&self, n: usize) -> Tape {
        // A token is switched on once, and once more after each mark's arm.
        let paths = (n as u64 + 2) * (1 + self.marks);
        self.sized((self.bounds.nodes * paths + 1) as usize, (self.bounds.supers * paths + 1) as usize, (self.bounds.pushes * paths + 1) as usize)
    }

    /// The buffers, room only: not zeroed. The code writes every slot before it reads it (a
    /// record's columns and its chain's head where it is made, a frame where it opens, the
    /// table's passes over the records made); the columns' lengths are set to what it wrote
    /// (`Tape::written`). `RERAIL_TAPE_POISON=1` fills them with ones instead, to show a slot
    /// read before it is written.
    fn sized(&self, nodes: usize, supers: usize, pushes: usize) -> Tape {
        use crate::native_cpu_tape::SLACK;
        let poison = std::env::var_os("RERAIL_TAPE_POISON").is_some();
        // Format 15's buffers only in a format 15 run; the working rows, their sizes and the supertypes only where the
        // nodes are passed from rows (not `direct`).
        let (f15, rowed) = (!self.sixteen, !self.direct);
        let room = |n: usize, used: bool| -> Vec<u32> {
            match (used, poison) {
                (false, _) => Vec::new(),
                (true, true) => vec![u32::MAX; n],
                (true, false) => Vec::with_capacity(n),
            }
        };
        let bytes = |n: usize, used: bool| -> Vec<u8> {
            match (used, poison) {
                (false, _) => Vec::new(),
                (true, true) => vec![u8::MAX; n],
                (true, false) => Vec::with_capacity(n),
            }
        };
        let wraps = |n: usize| if self.wraps { n } else { 0 };
        let w = self.widths;
        let pad = rerail_tape_format::padded;
        let blocks = nodes.div_ceil(rerail_tape_format::CHECKPOINT_STRIDE) + 1;
        let image = rerail_tape_format::HEADER_BYTES + pad(2 * nodes) + pad(usize::from(w.size) * nodes) + pad(4 * blocks) + pad(usize::from(w.off) * nodes) + pad(usize::from(w.tail) * nodes) + (6 + 2 * self.classes.len()) * nodes + 2 * SLACK;
        let id_of = if f15 {
            IDS.with(|t| {
                let mut ids = std::mem::take(&mut *t.borrow_mut());
                let slots = crate::native_cpu_tape::id_slots(self.classes.len());
                if ids.len() < slots {
                    ids.resize(slots, crate::native_cpu_tape::NO_CODE as u16);
                }
                ids
            })
        } else {
            Vec::new()
        };
        let returns = if self.context { 2 * pushes } else { pushes };
        let mut t = Tape {
            rows: bytes(ROW as usize * nodes + SLACK, rowed),
            size: bytes(usize::from(w.size) * nodes + SLACK, rowed),
            off: bytes(usize::from(w.off) * nodes + SLACK, f15),
            tail: bytes(usize::from(w.tail) * nodes + SLACK, f15),
            base: room(blocks + SLACK, f15),
            image: bytes(image, f15),
            id_of,
            code_kf: if f15 { Vec::with_capacity(nodes + 1) } else { Vec::new() },
            code_first: room(nodes + 1, f15),
            super_node: room(supers, rowed),
            super_kind: room(supers, rowed),
            // The stack of open nodes: the Rust's format 16 run keeps its own here.
            open: room(nodes, true),
            returns: room(returns, true),
            counts: [0; 3],
            frames: std::array::from_fn(|_| room(wraps(nodes + 1), true)),
            chains: std::array::from_fn(|k| room(wraps(if k == 2 { 2 * nodes + 2 } else { nodes }), true)),
            room: [nodes, supers, pushes],
            writer: std::ptr::null_mut(),
            numbers: std::ptr::null(),
        };
        t.clear();
        t
    }

    /// One call over a file. `bytes` is the file (`len` its length), with 64 readable bytes
    /// after it; `tier1` is tier 1's buffers for it.
    ///
    /// # Safety
    /// The pointers must be as stated, and `tape` from `self.tape(self.tokens(tier1.nstarts, len))`.
    pub unsafe fn call(&self, bytes: *const u8, len: usize, tier1: Tier1View, tape: &mut Tape) -> u64 {
        // Where tier 2 makes the records, `TOKEN` reads the starts as a column.
        assert!(self._fetch.is_none() || tier1.record_stride == crate::records::RecordLayout::Columns.stride() as usize, "the starts as a column where tier 2 makes the records");
        // With no room check (`Bounds::unchecked`), a tape short of the bound's is replaced first.
        if self.bounds.unchecked(self.marks) {
            let n = self.tokens(fed_bound(&tier1, len), len);
            let paths = n as u64 + 2;
            let fits = tape.room[0] as u64 > self.bounds.nodes * paths && tape.room[1] as u64 > self.bounds.supers * paths && tape.room[2] as u64 > self.bounds.pushes * paths;
            if !fits {
                *tape = self.full_tape(n);
            }
        }
        // SAFETY: the caller's contract.
        let r = unsafe { self.call_once(bytes, len, tier1, tape) };
        // A format 16 run's writer took the nodes passed so far: its caller runs it again (`Parser::image16`).
        if r != OVERFLOW || !tape.writer.is_null() {
            // SAFETY: the code wrote the image and supertypes it counted.
            unsafe { tape.written(r == 0) };
            return r;
        }
        crate::native_cpu_codegen::options().room_retries.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *tape = self.full_tape(self.tokens(fed_bound(&tier1, len), len));
        // SAFETY: as above, on the bound's tape.
        let r = unsafe { self.call_once(bytes, len, tier1, tape) };
        unsafe { tape.written(r == 0) };
        if r == OVERFLOW { self.overflow_pc() } else { r }
    }

    /// A run that passed even the bound's room (a conservative check where returns chain):
    /// refused at the program's end, the file goes to the fallback.
    fn overflow_pc(&self) -> u64 {
        u64::MAX - 1
    }

    /// `call` on the tape as given.
    ///
    /// # Safety
    /// As `call`.
    unsafe fn call_once(&self, bytes: *const u8, len: usize, tier1: Tier1View, tape: &mut Tape) -> u64 {
        let mut ctx = Ctx {
            bytes,
            len: len as u64,
            starts: tier1.starts,
            nstarts: tier1.nstarts as u64,
            lasts: tier1.lasts,
            codes: tier1.codes,
            totals: tier1.totals.cast(),
            rows: tape.rows.as_mut_ptr(),
            super_node: tape.super_node.as_mut_ptr(),
            super_kind: tape.super_kind.as_mut_ptr(),
            open: tape.open.as_mut_ptr(),
            returns: tape.returns.as_mut_ptr(),
            counts: tape.counts.as_mut_ptr(),
            lasts_stride: tier1.lasts_stride as u64,
            outputs: tier1.outputs,
            frame_base: tape.frames[0].as_mut_ptr(),
            frame_last: tape.frames[1].as_mut_ptr(),
            frame_last_base: tape.frames[2].as_mut_ptr(),
            head: tape.chains[0].as_mut_ptr(),
            link: tape.chains[1].as_mut_ptr(),
            place: tape.chains[2].as_mut_ptr(),
            out_rows: std::ptr::null_mut(),
            output_stride: tier1.output_stride as u64,
            totals_stride: tier1.totals_stride as u64,
            totals_mask: tier1.totals_mask,
            room_nodes: tape.room[0] as u64,
            room_supers: tape.room[1] as u64,
            room_pushes: tape.room[2] as u64,
            stack_base: 0,
            stack_limit: 0,
            frame_shift: self.frame_shift,
            state: [0; Mem::COUNT as usize],
            size_c: tape.size.as_mut_ptr(),
            off_c: tape.off.as_mut_ptr(),
            tail_c: tape.tail.as_mut_ptr(),
            base_c: tape.base.as_mut_ptr(),
            image: tape.image.as_mut_ptr(),
            id_of: tape.id_of.as_mut_ptr(),
            code_kf: tape.code_kf.as_mut_ptr(),
            code_first: tape.code_first.as_mut_ptr(),
            record_stride: tier1.record_stride as u64,
            writer: tape.writer,
            numbers: tape.numbers,
            more: tier1.more,
            feed: tier1.feed,
            mark: [0; MARK_WORDS],
        };
        // SAFETY: the caller's contract; the function writes only the tape's buffers and the
        // state in `ctx`.
        unsafe { (self.entry)(&raw mut ctx) }
    }

    /// The words over tier 1's run of `bytes`, as the interpreter's `stream::run` gives them.
    pub fn run(&self, bytes: &[u8], tier1: &crate::block::BlockRun) -> Result<Vec<Node>, Refused> {
        let mut tape = self.tape(self.tokens(self.records_of(tier1), bytes.len()));
        let r = self.run_raw(bytes, tier1, &mut tape);
        if r != 0 {
            return Err(Refused { pc: r as usize - 1, why: "the compiled words refuse", at: None });
        }
        Ok(tape.nodes())
    }

    /// The records tier 1's run of a file gives the words.
    fn records_of(&self, tier1: &crate::block::BlockRun) -> usize {
        self.starts.map_or(0, |e| tier1.positions[e].len())
    }

    /// The format 15 image `run` writes (a test's), or the call's result.
    #[cfg(test)]
    pub fn image_on(&self, bytes: &[u8], tier1: &crate::block::BlockRun) -> Result<Vec<u8>, u64> {
        let mut tape = self.tape(self.tokens(self.records_of(tier1), bytes.len()));
        match self.run_raw(bytes, tier1, &mut tape) {
            0 => Ok(tape.image()),
            r => Err(r),
        }
    }

    /// `Parser::image16` over tier 1's run `tier1` (a test's), by this tier 2, compiled to write format 16.
    #[cfg(test)]
    pub fn image16_on(&self, bytes: &[u8], terminators: rerail_tape_format::newlines::Terminators, tier1: &crate::block::BlockRun, lang: &rerail_tape_format::f16::Lang, numbers: &Numbers, z: bool) -> Result<rerail_tape_format::f16::Written, String> {
        let scan = |out: &mut Vec<u8>| rerail_tape_format::newlines::index_into(out, bytes, terminators);
        let doc = Doc16 { source: bytes, lang, numbers, z, lines: &scan };
        self.write16(&mut rerail_tape_format::f16::Writer::empty(), self.tokens(self.records_of(tier1), bytes.len()), &doc, |tape| self.run_raw(bytes, tier1, tape))
    }

    /// A format 16 run of `source` (`Parser::image16`): the writer made and its room reserved for the tape's nodes and
    /// a guess of the depth, then `call` on the tape that carries it; again with the bound's room where a part was
    /// full (the open nodes past the guess) or the call overflowed tier 2's own room.
    fn write16(&self, w: &mut rerail_tape_format::f16::Writer, tokens: usize, doc: &Doc16<'_>, mut call: impl FnMut(&mut Tape) -> u64) -> Result<rerail_tape_format::f16::Written, String> {
        use rerail_clif::writer::{fail, part};
        if !self.sixteen {
            return Err("a tier 2 compiled for format 15".into());
        }
        let mut tape = self.tape(tokens);
        let (mut depth, mut full) = (64, false);
        loop {
            let nodes = tape.room_nodes();
            // With each block's presence, as every other producer's writer (decision 0434 §4, step 6).
            let Doc16 { source, lang, numbers, z, lines } = *doc;
            w.begin_with(&rerail_tape_format::f16::Document { lang, source, tag: self.tag, z, blocks: true, nodes }, lines);
            // The stack: the program's bound of open nodes where it has one (`open_bound`, which the Rust's run checks
            // once), else a guess, then the nodes' room.
            w.reserve_run(nodes, self.open_bound.map_or(depth.min(nodes), |b| b as usize));
            tape.writer = w.state_address();
            tape.numbers = numbers.0.as_ptr();
            match call(&mut tape) {
                0 => match w.failure() {
                    None => return w.finish().map(|f| f.tape),
                    Some((code, _)) if code == fail::FULL + part::OPEN && depth < nodes => depth = nodes,
                    Some((code, node)) => return Err(format!("the writer's ops failed: {code:#x} at node {node}")),
                },
                OVERFLOW if !full => {
                    full = true;
                    tape = self.full_tape(tokens);
                }
                r => return Err(format!("tier 2 refuses the file at word {}", r.wrapping_sub(1))),
            }
        }
    }

    /// `run`'s call, on `tape`: 0, one more than the word that refused, or `OVERFLOW` (a format 16 run's).
    fn run_raw(&self, bytes: &[u8], tier1: &crate::block::BlockRun, tape: &mut Tape) -> u64 {
        let mut padded = vec![0u8; bytes.len() + 65];
        padded[1..=bytes.len()].copy_from_slice(bytes);
        // Each stream ended in two entries of the file's length, as the batch's (`Batch::end_streams`).
        let stream = |e: Option<usize>| -> (Vec<u32>, usize) {
            let mut v = e.map_or_else(Vec::new, |e| tier1.positions[e].clone());
            let n = v.len();
            v.extend([bytes.len() as u32; 2]);
            (v, n)
        };
        let starts = stream(self.starts);
        // The outputs interleaved, as a batch holds them: output `o`'s word `w` at `o + w * n`.
        let n = tier1.outputs.len().max(1);
        let count = tier1.outputs.iter().map(Vec::len).max().unwrap_or(0);
        let mut all = vec![0u64; n * count.max(1)];
        for (o, words) in tier1.outputs.iter().enumerate() {
            for (w, &x) in words.iter().enumerate() {
                all[o + w * n] = x;
            }
        }
        let lasts = stream(self.lasts);
        let mut codes = tier1.codes.clone();
        codes.extend([0, 0]);
        let totals = if tier1.totals.is_empty() { vec![0] } else { tier1.totals.clone() };
        // SAFETY: `padded` has the byte before and 64 after; each stream two slots past its count;
        // the tape is sized for the stream; every output has `count` words in `all`.
        let view = Tier1View {
            starts: starts.0.as_ptr(),
            // Where tier 2 makes the records (page 40), tier 1 writes no lasts.
            nstarts: if self._fetch.is_some() { starts.1 } else { starts.1.min(lasts.1) },
            lasts: lasts.0.as_ptr(),
            codes: codes.as_ptr(),
            lasts_stride: 8 * n,
            outputs: all.as_ptr(),
            output_stride: 8,
            record_stride: crate::records::RecordLayout::Columns.stride() as usize,
            totals: totals.as_ptr().cast(),
            totals_stride: 8,
            totals_mask: u64::MAX,
            more: 0,
            feed: 0,
        };
        unsafe { self.call(padded.as_ptr().wrapping_add(1), bytes.len(), view, tape) }
    }
}

impl Tape {
    /// The columns' lengths: none, before a call (the code writes through their room).
    fn clear(&mut self) {
        for v in [&mut self.super_node, &mut self.super_kind, &mut self.open, &mut self.returns, &mut self.base, &mut self.code_first] {
            v.clear();
        }
        for v in self.frames.iter_mut().chain(&mut self.chains) {
            v.clear();
        }
        for v in [&mut self.rows, &mut self.size, &mut self.off, &mut self.tail, &mut self.image] {
            v.clear();
        }
        self.code_kf.clear();
    }

    /// The lengths after a call: where it accepted, the image's (`counts[2]`) and the
    /// supertypes; else none.
    ///
    /// # Safety
    /// The call wrote, where it accepted, the image's `counts[2]` bytes and `counts[1]`
    /// supertypes.
    unsafe fn written(&mut self, accepted: bool) {
        let (len, s) = (self.counts[2] as usize, self.counts[1] as usize);
        self.clear();
        if !accepted {
            self.counts[2] = 0;
            return;
        }
        // SAFETY: the caller's contract; each is within its room (the code checks it).
        unsafe {
            for v in [&mut self.super_node, &mut self.super_kind] {
                v.set_len(s);
            }
            self.image.set_len(len);
        }
    }

    /// The bytes it holds of format 15's buffers (the image, the size, offset and tail columns, the block bases, the
    /// code ids) and of the working rows (the rows, their sizes, the supertypes): a format 16 run's tape holds none of
    /// the first, a direct one's (`CompiledStream::direct`) none of the second either.
    pub fn held(&self) -> [usize; 2] {
        let f15 = self.off.capacity() + self.tail.capacity() + 4 * self.base.capacity() + self.image.capacity() + 2 * self.id_of.capacity() + 4 * (self.code_kf.capacity() + self.code_first.capacity());
        let rows = self.rows.capacity() + self.size.capacity() + 4 * (self.super_node.capacity() + self.super_kind.capacity());
        [f15, rows]
    }

    /// The nodes the tape has room for.
    pub fn room_nodes(&self) -> usize {
        self.room[0]
    }

    /// The engine's tape: the image's bytes, as tier 2 wrote them (no copy).
    pub fn image(mut self) -> Vec<u8> {
        std::mem::take(&mut self.image)
    }

    /// The tape's nodes as `stream::Node`s (the tests' comparisons with the interpreter), read
    /// back from the image: each node's code's kind and field, its start (its block's base and
    /// its offset), its size, and its end (its tail from where tails are measured).
    pub fn nodes(&self) -> Vec<Node> {
        let b = &self.image;
        let u64_at = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"));
        let (n, codes) = (u64_at(rerail_tape_format::header::NODES) as usize, u64_at(rerail_tape_format::header::CODES) as usize);
        let (ws, wo, wt) = (usize::from(b[9]), usize::from(b[10]), usize::from(b[11] & 0x7f));
        let from_start = b[11] & rerail_tape_format::TAIL_FROM_START != 0;
        let pad = rerail_tape_format::padded;
        let code_at = rerail_tape_format::HEADER_BYTES;
        let size_at = pad(code_at + 2 * n);
        let base_at = pad(size_at + ws * n);
        let off_at = pad(base_at + 4 * n.div_ceil(rerail_tape_format::CHECKPOINT_STRIDE));
        let tail_at = pad(off_at + wo * n);
        let table_at = pad(tail_at + wt * n);
        let cell = |at: usize, w: usize, i: usize| -> u32 {
            let c = &b[at + w * i..at + w * i + w];
            if w == 2 { u32::from(u16::from_le_bytes([c[0], c[1]])) } else { u32::from_le_bytes([c[0], c[1], c[2], c[3]]) }
        };
        // A code's entry: `kf`, flags, alias, its classes' count, the classes.
        let mut kfs: Vec<u16> = Vec::with_capacity(codes);
        let mut e = table_at;
        for _ in 0..codes {
            // A statement recovered (`CODE_ERROR`): the interpreter's `ERROR_LEAF`.
            kfs.push(if b[e + 2] & rerail_tape_format::CODE_ERROR != 0 { crate::stream_ir::ERROR_LEAF } else { u16::from_le_bytes([b[e], b[e + 1]]) });
            e += 6 + 2 * usize::from(b[e + 5]);
        }
        let starts: Vec<u32> = (0..n).map(|i| cell(base_at, 4, i / rerail_tape_format::CHECKPOINT_STRIDE) + cell(off_at, wo, i)).collect();
        let mut out: Vec<Node> = (0..n)
            .map(|i| {
                let kf = kfs[cell(code_at, 2, i) as usize];
                let size = cell(size_at, ws, i);
                let from = if from_start { starts[i] } else { starts[i + size as usize - 1] };
                let (kind, field) = if kf == crate::stream_ir::ERROR_LEAF { (kf, 0) } else { (kf & 0x3ff, kf >> rerail_tape_format::KIND_BITS) };
                Node { kind, field, supers: Vec::new(), start: starts[i], end: from.wrapping_add(cell(tail_at, wt, i)), size }
            })
            .collect();
        for s in 0..self.counts[1] as usize {
            out[self.super_node[s] as usize].supers.push(self.super_kind[s] as u16);
        }
        out
    }
}

/// A language's whole compiled path: tier 1's function, then tier 2's over the buffers tier 1
/// wrote. Between and around the two calls the host only lays out the file (padded, as simdjson
/// requires) and reads the node table back.
pub struct Parser {
    pub tier1: crate::native_cpu_block::CompiledBlock,
    pub tier2: CompiledStream,
    /// Tier 2 at wider widths, each compiled the first time a file needs it, from the structure
    /// and cut kept here (`again`). `None` for a parser made `from_parts`: such a file is refused.
    wide: Option<Wide>,
}

/// What a parser compiles its wider tier 2s from, and those compiled: one a set of widths, by
/// which of size, offset and tail take 4 bytes (`Wide::slot`).
struct Wide {
    native: Structure,
    cut: crate::native_cpu_clusters::Cut,
    streams: [std::sync::OnceLock<Option<CompiledStream>>; 8],
    /// Tier 2 writing format 16 (`Parser::sixteen`).
    sixteen: std::sync::OnceLock<Option<CompiledStream>>,
}

impl Wide {
    fn new(native: &Structure, cut: crate::native_cpu_clusters::Cut) -> Option<Self> {
        Some(Wide { native: native.clone(), cut, streams: Default::default(), sixteen: Default::default() })
    }

    fn slot(w: rerail_tape_format::Widths) -> usize {
        usize::from(w.size == 4) | usize::from(w.off == 4) << 1 | usize::from(w.tail == 4) << 2
    }

    /// Tier 2 at `w` (tails from the last descendant), compiled on first use; `None` where the
    /// compiler refuses the structure at those widths.
    fn at(&self, w: rerail_tape_format::Widths) -> Option<&CompiledStream> {
        self.streams[Self::slot(w)].get_or_init(|| compile_at(&self.native, &self.cut, w, self.native.tag).ok()).as_ref()
    }
}

/// The narrowest widths an image's values fit, as the engine's writer chooses them
/// (`rerail_tape_format::Writer::finish`): sizes by the node count, offsets and tails by their
/// largest. An image written at wider widths is written again at these, so that the tape's bytes
/// are the writer's.
pub fn least_widths(image: &[u8]) -> rerail_tape_format::Widths {
    use rerail_tape_format::{CHECKPOINT_STRIDE, HEADER_BYTES, header, padded, width_for};
    let n = u64::from_le_bytes(image[header::NODES..header::NODES + 8].try_into().expect("8 bytes")) as usize;
    let w = |k: usize| usize::from(image[header::WIDTHS + k] & !rerail_tape_format::TAIL_FROM_START);
    let (ws, wo, wt) = (w(0), w(1), w(2));
    let size_at = padded(HEADER_BYTES + 2 * n);
    let off_at = padded(padded(size_at + ws * n) + 4 * n.div_ceil(CHECKPOINT_STRIDE));
    let tail_at = padded(off_at + wo * n);
    let most = |at: usize, width: usize| -> u32 {
        if width == 2 {
            return 0;
        }
        image[at..at + 4 * n].as_chunks::<4>().0.iter().map(|&c| u32::from_le_bytes(c)).max().unwrap_or(0)
    };
    rerail_tape_format::Widths { size: width_for(n as u32), off: width_for(most(off_at, wo)), tail: width_for(most(tail_at, wt)), from_start: false }
}

impl Parser {
    pub fn new(native: &Structure) -> Result<Self, Error> {
        let block = native.block_programs.first().ok_or(Error::UnsupportedBackend)?;
        Ok(Parser { tier1: crate::native_cpu_block::compile(block)?, tier2: compile(native)?, wide: Wide::new(native, one_cut(native)) })
    }

    /// `new`, with tier 2 in the clusters of `cut` (`compile_cut`).
    pub fn with_cut(native: &Structure, cut: &crate::native_cpu_clusters::Cut) -> Result<Self, Error> {
        let block = native.block_programs.first().ok_or(Error::UnsupportedBackend)?;
        Ok(Parser { tier1: crate::native_cpu_block::compile(block)?, tier2: compile_cut(native, cut)?, wide: Wide::new(native, cut.clone()) })
    }

    /// Tiers 1 and 2 of `native` cut by `cut`, compiled by the caller (`native_cpu_block::compile`,
    /// `compile_cut`, timed apart), with the wide tier 2s `with_cut` keeps.
    pub fn with_tiers(native: &Structure, cut: &crate::native_cpu_clusters::Cut, tier1: crate::native_cpu_block::CompiledBlock, tier2: CompiledStream) -> Self {
        Parser { tier1, tier2, wide: Wide::new(native, cut.clone()) }
    }

    /// Tiers 1 and 2 compiled elsewhere (a probe timing tier 2's compile): no wide tier 2.
    pub fn from_parts(tier1: crate::native_cpu_block::CompiledBlock, tier2: CompiledStream) -> Self {
        Parser { tier1, tier2, wide: None }
    }

    /// Tier 2 at `Widths::WIDE`, compiled on first use; `None` where it cannot be (a parser
    /// `from_parts`, or a structure the compiler refuses at those widths).
    pub fn wide(&self) -> Option<&CompiledStream> {
        self.wide.as_ref()?.at(rerail_tape_format::Widths::WIDE)
    }

    /// A file the narrow tier 2 refused (`narrow`), read again (decision 0140's guess, then
    /// docs/parser-generators/37's wider parse): by `read` at `Widths::WIDE`, then, where its
    /// values fit narrower columns (`least_widths`), at those, so the image is the writer's.
    /// A refusal at `WIDE` is the grammar's: the file goes to the fallback.
    pub fn again(&self, narrow: Refused, read: impl Fn(&CompiledStream) -> Result<Vec<u8>, Refused>) -> Result<Vec<u8>, Refused> {
        let (Some(w), Some(wide)) = (self.wide.as_ref(), self.wide()) else { return Err(narrow) };
        let image = read(wide)?;
        let least = least_widths(&image);
        match w.at(least) {
            Some(t2) if least != rerail_tape_format::Widths::WIDE => read(t2),
            _ => Ok(image),
        }
    }

    /// The engine's tape of `bytes` as tier 2 wrote it (its bytes), or the word that refused
    /// it (the file goes to the fallback). A file whose tape does not fit the narrow widths is
    /// read again at wider ones, over the same tier 1 run (`again`).
    pub fn image(&self, bytes: &[u8]) -> Result<Vec<u8>, Refused> {
        let mut batch = crate::native_cpu_block::Batch::new(&self.tier1, &[bytes]);
        batch.call(&self.tier1);
        let mut tape = self.tape(&batch);
        match self.words(&batch, &mut tape) {
            Ok(()) => Ok(tape.image()),
            Err(narrow) => self.again(narrow, |t2| {
                let mut tape = t2.tape(self.records_bound(&batch, t2.starts));
                self.words_with(t2, &batch, &mut tape)?;
                Ok(tape.image())
            }),
        }
    }


    /// The nodes of `bytes` as `stream::Node`s, for the tests' comparisons with the interpreter.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn parse(&self, bytes: &[u8]) -> Result<Vec<Node>, Refused> {
        let mut batch = crate::native_cpu_block::Batch::new(&self.tier1, &[bytes]);
        batch.call(&self.tier1);
        let mut tape = self.tape(&batch);
        match self.words(&batch, &mut tape) {
            Ok(()) => Ok(tape.nodes()),
            Err(narrow) => {
                let Some(wide) = self.wide() else { return Err(narrow) };
                let mut tape = wide.tape(self.records_bound(&batch, wide.starts));
                self.words_with(wide, &batch, &mut tape)?;
                Ok(tape.nodes())
            }
        }
    }

    /// The records tier 2 reads in `batch` (its first file): tier 1's count; with a feed (decision 0496), which
    /// makes them as tier 2 asks, the most a file of its bytes has (a token a byte, and the end).
    fn records_bound(&self, batch: &crate::native_cpu_block::Batch, e: Option<usize>) -> usize {
        if batch.feeds.is_empty() { self.stream(batch, e).1 } else { batch.rows[0].len as usize + 1 }
    }

    /// Tier 1's stream `e` in `batch` (its first file), as tier 1 wrote it: its positions (room
    /// for two past its count) and count.
    fn stream<'b>(&self, batch: &'b crate::native_cpu_block::Batch, e: Option<usize>) -> (&'b [u32], usize) {
        let row = batch.rows[0];
        match e {
            Some(e) => {
                let at = (row.positions + e as u64 * row.capacity) as usize;
                (&batch.positions[at..at + row.capacity as usize], batch.counts[row.counts as usize + e] as usize)
            }
            None => (&batch.positions[..0], 0),
        }
    }

    /// A tape for the stream tier 1 wrote in `batch`.
    pub fn tape(&self, batch: &crate::native_cpu_block::Batch) -> Tape {
        self.tier2.tape(self.tier2.tokens(self.records_bound(batch, self.tier2.starts), batch.rows[0].len as usize))
    }

    /// Tier 2's function over tier 1's buffers in `batch` (one file), into `tape`; or the word
    /// that refused the file.
    pub fn words(&self, batch: &crate::native_cpu_block::Batch, tape: &mut Tape) -> Result<(), Refused> {
        self.words_with(&self.tier2, batch, tape)
    }

    /// `words`, by tier 2 `t2` (the narrow one or the wide one).
    fn words_with(&self, t2: &CompiledStream, batch: &crate::native_cpu_block::Batch, tape: &mut Tape) -> Result<(), Refused> {
        let r = self.words_raw(t2, batch, tape);
        if r != 0 {
            return Err(Refused { pc: r as usize - 1, why: "the compiled words refuse", at: None });
        }
        Ok(())
    }

    /// `words_with`'s call: 0, one more than the word that refused, or `OVERFLOW` (a format 16 run's).
    fn words_raw(&self, t2: &CompiledStream, batch: &crate::native_cpu_block::Batch, tape: &mut Tape) -> u64 {
        let row = batch.rows[0];
        // Tier 1's feed, where it made the file's (decision 0496): tier 2 reads the records made, so has them all
        // made first.
        batch.fed_all();
        let extracts = self.tier1.extracts.len();
        let starts = self.stream(batch, t2.starts);
        let outputs = batch.out[row.out as usize..].as_ptr();
        let lasts = self.stream(batch, t2.lasts);
        // The records where tier 1's layout has them (`records::RecordLayout`): a token's start,
        // last and code, a column each or a row a token.
        let (layout, cols) = match &self.tier1.records {
            Some(t) => (t.layout, [u64::from(t.starts), u64::from(t.lasts), extracts as u64]),
            None => (crate::records::RecordLayout::Columns, [t2.starts.unwrap_or(0) as u64, t2.lasts.unwrap_or(0) as u64, extracts as u64]),
        };
        let field = |k: usize| batch.positions[(row.positions + layout.index(k, 0, cols, row.capacity)) as usize..].as_ptr();
        let (starts_p, lasts_p, codes) = (field(crate::records::FIELD_START), field(crate::records::FIELD_LAST), field(crate::records::FIELD_CODE));
        let none = [0u64];
        let totals = match batch.counts.get(row.counts as usize + extracts..) {
            Some(t) if !t.is_empty() => t,
            _ => &none[..],
        };
        debug_assert!(starts.0.len() >= starts.1 + 2 || t2.starts.is_none());
        // SAFETY: the file's bytes in the batch have `FRONT` zero bytes before them and 64 after
        // its blocks; each stream has `capacity` slots, at least two past its count; the tape is
        // sized for the stream.
        let view = Tier1View {
            starts: if t2.starts.is_some() { starts_p } else { starts.0.as_ptr() },
            // Where tier 2 makes the records (page 40), tier 1 writes no lasts.
            nstarts: if t2._fetch.is_some() { starts.1 } else { starts.1.min(lasts.1) },
            lasts: if t2.lasts.is_some() { lasts_p } else { lasts.0.as_ptr() },
            codes,
            lasts_stride: 8 * self.tier1.outputs,
            outputs,
            output_stride: 8,
            totals: totals.as_ptr().cast(),
            totals_stride: 8,
            totals_mask: u64::MAX,
            record_stride: layout.stride() as usize,
            more: 0,
            feed: 0,
        };
        unsafe { t2.call(batch.bytes.as_ptr().wrapping_add(row.bytes as usize), row.len as usize, view, tape) }
    }

    /// Tier 2 compiled to write format 16 (`compile_with`), on first use: `None` for a parser made `from_parts`, or a
    /// structure the compiler refuses so. Its sizes are 4 bytes, the one column its passes read.
    pub fn sixteen(&self) -> Option<&CompiledStream> {
        let w = self.wide.as_ref()?;
        let widths = rerail_tape_format::Widths { size: 4, ..rerail_tape_format::Narrow::WIDTHS };
        w.sixteen.get_or_init(|| compile_with(&w.native, &w.cut, widths, w.native.tag, true).ok()).as_ref()
    }

    /// The format 16 tape (16z where `z`) of `bytes` in `lang`, written by tier 2 as it parses, through the
    /// writer's ops (decision 0434 §6, R17): no format 15 image is made. `numbers`: `lang`'s numbers by the format
    /// 16 tier 2's keys (`CompiledStream::numbers`). The writer's room is reserved for the bound's nodes and a
    /// guess of the depth; a run that found a part full (the open nodes past the guess, or tier 2's own room past
    /// its guess) runs again with the bound's room. `Err` for a file tier 2 refuses, or a failure of the ops.
    /// Its line index is of the lines `terminators` end (decision 0612).
    pub fn image16(&self, bytes: &[u8], terminators: rerail_tape_format::newlines::Terminators, lang: &rerail_tape_format::f16::Lang, numbers: &Numbers, z: bool) -> Result<rerail_tape_format::f16::Written, String> {
        self.image16_in(&mut rerail_tape_format::f16::Writer::empty(), bytes, terminators, lang, numbers, z)
    }

    /// `image16` in `w`, a writer the caller keeps from one document to the next (`f16::Writer::begin`), as simdjson's
    /// caller keeps its parser: no room made or zeroed for a document no larger than one before. A tape given back
    /// (`f16::Writer::recycle`) is the next one's room.
    pub fn image16_in(&self, w: &mut rerail_tape_format::f16::Writer, bytes: &[u8], terminators: rerail_tape_format::newlines::Terminators, lang: &rerail_tape_format::f16::Lang, numbers: &Numbers, z: bool) -> Result<rerail_tape_format::f16::Written, String> {
        let t2 = self.sixteen().ok_or("no format 16 tier 2 for this parser")?;
        let mut batch = crate::native_cpu_block::Batch::new(&self.tier1, &[bytes]);
        batch.call(&self.tier1);
        let tokens = t2.tokens(self.records_bound(&batch, t2.starts), bytes.len());
        // The line index from tier 1's line-end bits (`CompiledBlock::newlines`), where it has them for the
        // document's terminators: no scan of the source. Bit `p` of the output's words is byte `p` of the file: the host puts a stripped prefix's bytes back
        // (`Batch::call`), and a prefix holds no newline.
        let row = batch.rows[0];
        let from_bits = |out: &mut Vec<u8>| {
            let (o, n) = (self.tier1.newlines.expect("tier 1's line ends").0, self.tier1.outputs);
            let at = |w: usize| batch.out[row.out as usize + o + w * n];
            // The candidates kept where they end a terminator: a test of the bytes at each, no scan.
            let ends = || rerail_tape_format::newlines::bits_of(row.len, 0, row.blocks as usize, at).1.filter(|&o| terminators.ends_at(bytes, o as usize));
            rerail_tape_format::newlines::index_of(out, bytes.len() as u64, ends().count() as u64, ends());
            #[cfg(debug_assertions)]
            debug_assert!(out[..] == rerail_tape_format::newlines::index(bytes, terminators)[..], "the line index from tier 1's bits is the scan's");
        };
        let scan = |out: &mut Vec<u8>| rerail_tape_format::newlines::index_into(out, bytes, terminators);
        let lines: &dyn Fn(&mut Vec<u8>) = if self.tier1.newlines.is_some_and(|(_, set)| set == terminators) { &from_bits } else { &scan };
        t2.write16(w, tokens, &Doc16 { source: bytes, lang, numbers, z, lines }, |tape| self.words_raw(t2, &batch, tape))
    }
}

/// `Cut::unsigned`'s builder: return addresses not signed (Cranelift signs them on every macOS target; an arm64
/// process, not arm64e, does not need it). The JIT's own flags, as `JITBuilder::with_flags` sets them.
#[cfg(not(target_family = "wasm"))]
fn unsigned_builder(flags: &[(&str, &str)]) -> Result<JITBuilder, Error> {
    use cranelift_codegen::settings::Configurable;
    let mut shared = cranelift_codegen::settings::builder();
    for (k, v) in flags.iter().copied().chain([("use_colocated_libcalls", "false"), ("is_pic", "false")]) {
        shared.set(k, v).map_err(|_| Error::BackendFailure)?;
    }
    // The host's ISA with its detected flags, as the JIT's (a toggle's `clmul` needs
    // `has_pmull`, `native_cpu_block::clmul`).
    let isa = cranelift_native::builder()
        .map_err(|_| Error::BackendFailure)?
        .finish(cranelift_codegen::settings::Flags::new(shared))
        .map_err(|_| Error::BackendFailure)?;
    Ok(JITBuilder::with_isa(isa, default_libcall_names()))
}

/// wasm32 has no JIT (`cranelift-jit-wasm`): the builder fails as `with_flags`'s does, and the caller takes the
/// interpreter's path.
#[cfg(target_family = "wasm")]
fn unsigned_builder(flags: &[(&str, &str)]) -> Result<JITBuilder, Error> {
    JITBuilder::with_flags(flags, default_libcall_names()).map_err(|_| Error::BackendFailure)
}
