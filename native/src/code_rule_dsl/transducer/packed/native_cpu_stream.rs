//! Tier 2's words on the CPU JIT (decisions 0053, 0054). Cranelift lowers the Program's words as
//! `native_ir` decodes them, one block per word, as it lowers the tree walker's: `GOTO` is a
//! jump, `SWITCH` a jump table on the token's byte (simdjson's `switch (*value)`), `RETURN` a
//! switch over the states that `PUSH` names, and `CHECK` a call of the check's compiled block
//! program (`native_cpu_block`) over the token where it is, 64 bytes at a time.
//!
//! The function reads the file's bytes, padded, and the stream: every token's first byte, with
//! the file's length after the last (simdjson's sentinel index), and the tokens' last bytes. It
//! writes the file's tape (v9np's node columns, `tape::TapeImage`) as simdjson's stage 2 writes
//! its tape: each node's code, size, offset and tail, at widths fixed when the function is
//! compiled (`Shape`). A node's code is known when it is compiled: its kind and the field and
//! supertype words after it. At run time a code is numbered at its first use, from a table.
//! Nothing here walks the words or the switch tables at run time: they are the function's code.
//!
//! The widths are guessed from the file and its stream before the run (`Shape::guess`). The run
//! ORs every value of each column, so after it the right widths are known; where the guess was
//! wrong the function compiled for them runs once more. One second pass at most.

use super::Error;
use super::native_cpu_block::{self, CompiledBlock, FRONT};
use super::native_ir::{NativeModule, StructuralOperation};
use super::stream::{REFUSE, Refused};
use crate::code_rule_dsl::tape::TAIL_FROM_START;
use cranelift_codegen::ir::{AbiParam, Block, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};
use std::collections::HashMap;
use std::mem::offset_of;
use std::sync::{Arc, Mutex};

/// A tape's widths (each 2 or 4 bytes): the size, off and tail columns', and whether a tail
/// is from the node's last descendant's start (`from_last`) or from its own start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct Shape {
    pub(super) size: u8,
    pub(super) off: u8,
    pub(super) tail: u8,
    pub(super) from_last: bool,
}

fn width_for(max: u64) -> u8 {
    if max <= 0xffff { 2 } else { 4 }
}

impl Shape {
    /// The widths guessed before the file: a node is at most two per token (an open and a
    /// leaf) and the root; an offset and a tail fit two bytes when the file does, and are
    /// guessed to otherwise (they are distances inside 16 nodes, and inside a node's last
    /// child).
    pub(super) fn guess(tokens: usize) -> Self {
        Self { size: width_for(2 * tokens as u64 + 1), off: 2, tail: 2, from_last: true }
    }

    /// The header's widths byte for byte (`tape::TapeImage`).
    pub(super) fn widths(&self) -> [u8; 3] {
        [self.size, self.off, self.tail | if self.from_last { 0 } else { TAIL_FROM_START }]
    }
}

/// The function's one argument: its inputs, the tape's columns, and what it reports.
#[repr(C)]
struct Context {
    bytes: *const u8,
    len: u64,
    starts: *const u32,
    nstarts: u64,
    lasts: *const u32,
    nlasts: u64,
    code: *mut u8,
    size: *mut u8,
    base: *mut u8,
    off: *mut u8,
    tail: *mut u8,
    node_cap: u64,
    /// Each code slot's number, `u16::MAX` until its first use; and the slots by first use.
    ids: *mut u16,
    order: *mut u16,
    /// The open nodes: each its preorder and its start.
    open: *mut u32,
    returns: *mut u32,
    stack_cap: u64,
    /// The check's output words.
    words: *mut u64,
    /// The check's row (`native_cpu_block::Row`).
    row: [u64; 4],
    node_count: u64,
    code_count: u64,
    /// The ORs of the columns' values: offsets, tails from the last descendant, lengths; and
    /// whether a node ended before its last descendant's start.
    or_off: u64,
    or_last: u64,
    or_len: u64,
    before_last: u64,
    pc: u64,
}

/// The function: 0 the file is accepted, 1 a word refused it (`Context::pc`), 2 a buffer is full.
type Entry = unsafe extern "C" fn(*mut Context) -> u32;

/// Bytes readable past the file's end: `WORD`'s load may read past the token (simdjson's
/// `SIMDJSON_PADDING`).
const PADDING: usize = 64;

/// A file's tape as tier 2 wrote it: the node columns as stored, at `shape`'s widths, and the
/// codes by first use (`kf` and the supertype classes, ascending).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Columns {
    pub(super) nodes: usize,
    pub(super) code: Vec<u8>,
    pub(super) size: Vec<u8>,
    pub(super) base: Vec<u8>,
    pub(super) off: Vec<u8>,
    pub(super) tail: Vec<u8>,
    pub(super) shape: Shape,
    pub(super) codes: Vec<(u16, Vec<u16>)>,
    /// How many times the function ran: 1, or 2 where the guessed widths were wrong.
    pub(super) passes: u8,
}

/// Tier 2 on the CPU: the program's words, compiled once per shape (at most two per file).
pub(in crate::code_rule_dsl) struct Tier2Cpu {
    native: NativeModule,
    /// The node codes the words can write (a slot per distinct code).
    slots: Vec<(u16, Vec<u16>)>,
    checks: Arc<Vec<CompiledBlock>>,
    compiled: Mutex<HashMap<Shape, Arc<CompiledStream>>>,
}

/// One run's report.
struct Run {
    status: u32,
    pc: u64,
    nodes: usize,
    codes: usize,
    or_off: u64,
    or_last: u64,
    or_len: u64,
    before_last: bool,
}

impl Tier2Cpu {
    /// Tier 2's words of `native`; `UnsupportedBackend` for any other program.
    pub(super) fn new(native: &NativeModule) -> Result<Self, Error> {
        let slots = slots(native)?.1;
        let checks = native
            .block_programs
            .first()
            .map(|b| b.checks.iter().map(native_cpu_block::compile).collect::<Result<Vec<_>, _>>())
            .transpose()?
            .unwrap_or_default();
        let this = Self { native: native.clone(), slots, checks: Arc::new(checks), compiled: Mutex::new(HashMap::new()) };
        this.function(Shape::guess(0))?;
        Ok(this)
    }

    fn function(&self, shape: Shape) -> Result<Arc<CompiledStream>, Error> {
        let mut compiled = self.compiled.lock().expect("not poisoned");
        if let Some(c) = compiled.get(&shape) {
            return Ok(c.clone());
        }
        let c = Arc::new(compile(&self.native, shape, self.checks.clone())?);
        compiled.insert(shape, c.clone());
        Ok(c)
    }

    /// The function for `shape`, as Cranelift IR and as machine code.
    pub(super) fn code(&self, shape: Shape) -> Result<(String, String), Error> {
        let c = self.function(shape)?;
        Ok((c.clif.clone(), c.disassembly.clone()))
    }

    /// The tape tier 2 writes over the stream (`starts`, `lasts`) of `bytes`, or where a word
    /// refused the file: as `stream::run`. The widths are guessed, and fixed by a second pass
    /// where the guess was wrong.
    pub(super) fn tape(&self, bytes: &[u8], starts: &[u32], lasts: &[u32]) -> Result<Columns, Refused> {
        let compile_error = |_| Refused { pc: 0, why: "tier 2 does not compile" };
        let guess = Shape::guess(starts.len());
        let mut padded = vec![0u8; FRONT];
        padded.extend_from_slice(bytes);
        padded.resize(FRONT + bytes.len() + PADDING, 0);
        let mut stream = starts.to_vec();
        stream.push(bytes.len() as u32);
        let mut cap = 2 * starts.len() + 8;
        let mut shape = guess;
        let mut passes = 0u8;
        loop {
            let f = self.function(shape).map_err(compile_error)?;
            let mut out = Buffers::new(cap, shape, self.slots.len(), bytes.len());
            let run = f.call(&padded[FRONT..], bytes.len(), &stream, lasts, &mut out);
            passes += 1;
            match run.status {
                0 => {}
                1 => return Err(Refused { pc: run.pc as usize, why: "a compiled word refused the file" }),
                _ => {
                    cap *= 2;
                    passes -= 1;
                    continue;
                }
            }
            // The widths the columns need, from their ORs.
            let from_last = !run.before_last;
            let need = Shape {
                size: width_for(run.nodes as u64),
                off: width_for(run.or_off),
                tail: width_for(if from_last { run.or_last } else { run.or_len }),
                from_last,
            };
            if need != shape {
                assert_eq!(passes, 1, "the second pass has the widths the first found");
                shape = need;
                continue;
            }
            let n = run.nodes;
            let w = |x: u8| usize::from(x);
            out.code.truncate(2 * n);
            out.size.truncate(w(shape.size) * n);
            out.base.truncate(4 * n.div_ceil(16));
            out.off.truncate(w(shape.off) * n);
            out.tail.truncate(w(shape.tail) * n);
            let codes = out.order[..run.codes].iter().map(|&s| self.slots[usize::from(s)].clone()).collect();
            return Ok(Columns { nodes: n, code: out.code, size: out.size, base: out.base, off: out.off, tail: out.tail, shape, codes, passes });
        }
    }
}

/// The buffers of one run.
struct Buffers {
    code: Vec<u8>,
    size: Vec<u8>,
    base: Vec<u8>,
    off: Vec<u8>,
    tail: Vec<u8>,
    ids: Vec<u16>,
    order: Vec<u16>,
    open: Vec<u32>,
    returns: Vec<u32>,
    words: Vec<u64>,
    cap: usize,
}

impl Buffers {
    fn new(cap: usize, shape: Shape, slots: usize, len: usize) -> Self {
        let w = |x: u8| usize::from(x);
        Self {
            code: vec![0; 2 * cap],
            size: vec![0; w(shape.size) * cap],
            base: vec![0; 4 * cap.div_ceil(16)],
            off: vec![0; w(shape.off) * cap],
            tail: vec![0; w(shape.tail) * cap],
            ids: vec![u16::MAX; slots],
            order: vec![0; slots],
            open: vec![0; 2 * cap],
            returns: vec![0; cap],
            words: vec![0; len / 64 + 3],
            cap,
        }
    }
}

/// A compiled tier-2 program, at one shape.
pub(super) struct CompiledStream {
    _module: JITModule,
    entry: Entry,
    /// The checks the function calls.
    _checks: Arc<Vec<CompiledBlock>>,
    /// The function as Cranelift IR, before code generation, and its machine code.
    pub(super) clif: String,
    pub(super) disassembly: String,
}

impl CompiledStream {
    fn call(&self, bytes: &[u8], len: usize, starts: &[u32], lasts: &[u32], out: &mut Buffers) -> Run {
        let cap = out.cap as u64;
        let mut cx = Context {
            bytes: bytes.as_ptr(),
            len: len as u64,
            starts: starts.as_ptr(),
            nstarts: (starts.len() - 1) as u64,
            lasts: lasts.as_ptr(),
            nlasts: lasts.len() as u64,
            code: out.code.as_mut_ptr(),
            size: out.size.as_mut_ptr(),
            base: out.base.as_mut_ptr(),
            off: out.off.as_mut_ptr(),
            tail: out.tail.as_mut_ptr(),
            node_cap: cap,
            ids: out.ids.as_mut_ptr(),
            order: out.order.as_mut_ptr(),
            open: out.open.as_mut_ptr(),
            returns: out.returns.as_mut_ptr(),
            stack_cap: cap,
            words: out.words.as_mut_ptr(),
            row: [0; 4],
            node_count: 0,
            code_count: 0,
            or_off: 0,
            or_last: 0,
            or_len: 0,
            before_last: 0,
            pc: 0,
        };
        // SAFETY: every pointer is to a buffer of the size the context gives (`Buffers::new`
        // at this function's shape): `bytes` with `FRONT` before it and `PADDING` after,
        // `starts` with its sentinel, `words` sized for the longest token, the file.
        let status = unsafe { (self.entry)(&mut cx) };
        Run {
            status,
            pc: cx.pc,
            nodes: cx.node_count as usize,
            codes: cx.code_count as usize,
            or_off: cx.or_off,
            or_last: cx.or_last,
            or_len: cx.or_len,
            before_last: cx.before_last != 0,
        }
    }
}

/// The node codes the words write: each `OPEN` and `LEAF` word's slot, and the slots, a code
/// each (`kf`: kind and field `<< 10`; the supertypes ascending). A node's field and
/// supertypes are the `NODE_FIELD` and `NODE_SUPER` words right after it, which no jump enters.
fn slots(native: &NativeModule) -> Result<(Vec<Option<u16>>, Vec<(u16, Vec<u16>)>), Error> {
    use StructuralOperation as S;
    let words = &native.structural;
    let attribute = |op: &S| matches!(op, S::NodeField { .. } | S::NodeSuper { .. });
    // No jump lands on a field or supertype word.
    let mut targets: Vec<u16> = native.switches.iter().flat_map(|s| s.targets.iter().copied().chain([s.end])).filter(|&t| t != REFUSE).collect();
    for b in words {
        match b.operation {
            S::Goto { target } | S::Push { target } => targets.extend([target, target + 1]),
            _ => {}
        }
    }
    if targets.iter().any(|&t| words.get(usize::from(t)).is_some_and(|b| attribute(&b.operation))) {
        return Err(Error::UnsupportedBackend);
    }
    let mut of = vec![None; words.len()];
    let mut slots: Vec<(u16, Vec<u16>)> = Vec::new();
    for (pc, b) in words.iter().enumerate() {
        let (S::Open { kind } | S::Leaf { kind }) = b.operation else { continue };
        let (mut field, mut supers) = (0u16, Vec::new());
        let mut at = b.next;
        while let Some(n) = at.map(usize::from).filter(|&n| attribute(&words[n].operation)) {
            match words[n].operation {
                S::NodeField { field: f } => field = f,
                S::NodeSuper { kind } => supers.push(kind),
                _ => unreachable!(),
            }
            at = words[n].next;
        }
        if kind >= 1024 || field >= 64 || supers.len() > 8 {
            return Err(Error::UnsupportedBackend);
        }
        supers.sort_unstable();
        supers.dedup();
        let code = (kind | (field << 10), supers);
        let slot = slots.iter().position(|s| *s == code).unwrap_or_else(|| {
            slots.push(code);
            slots.len() - 1
        });
        of[pc] = Some(slot as u16);
    }
    Ok((of, slots))
}

/// The registers of the state machine, as Cranelift variables.
struct Regs {
    next: Variable,
    taken: Variable,
    start: Variable,
    end: Variable,
    limit: Variable,
    last_end: Variable,
    at_end: Variable,
    nodes: Variable,
    open: Variable,
    returns: Variable,
    codes: Variable,
    /// The base of the nodes' block of 16, and the last node's start.
    base: Variable,
    last_start: Variable,
    or_off: Variable,
    or_last: Variable,
    or_len: Variable,
    before_last: Variable,
}

/// The context: each field is loaded where a word uses it, as simdjson reads its parser's fields
/// through `this`, so that none is live across the function.
struct Inputs {
    cx: Value,
}

impl Inputs {
    fn get(&self, f: &mut FunctionBuilder<'_>, at: usize) -> Value {
        f.ins().load(types::I64, MemFlags::trusted().with_readonly(), self.cx, at as i32)
    }
}

/// Branch to `to` (with `args`) when `cond`, and go on in a new block otherwise.
fn guard(f: &mut FunctionBuilder<'_>, cond: Value, to: Block, args: &[Value]) {
    let go_on = f.create_block();
    let args: Vec<cranelift_codegen::ir::BlockArg> = args.iter().map(|&v| v.into()).collect();
    f.ins().brif(cond, to, &args, go_on, &[]);
    f.switch_to_block(go_on);
}

/// Jump to the run `byte` falls in: `runs` are each run's first byte, ascending, and its target.
fn search(f: &mut FunctionBuilder<'_>, byte: Value, runs: &[(u32, Block)]) {
    if runs.len() == 1 {
        f.ins().jump(runs[0].1, &[]);
        return;
    }
    let mid = runs.len() / 2;
    let (low, high) = (f.create_block(), f.create_block());
    let below = f.ins().icmp_imm(IntCC::UnsignedLessThan, byte, i64::from(runs[mid].0));
    f.ins().brif(below, low, &[], high, &[]);
    f.switch_to_block(low);
    search(f, byte, &runs[..mid]);
    f.switch_to_block(high);
    search(f, byte, &runs[mid..]);
}

/// `base[index]` for elements of `size` bytes.
fn element(f: &mut FunctionBuilder<'_>, base: Value, index: Value, size: i64) -> Value {
    let off = f.ins().imul_imm(index, size);
    f.ins().iadd(base, off)
}

/// Store `value` as cell `index` of a column of `width` bytes.
fn put(f: &mut FunctionBuilder<'_>, column: Value, index: Value, value: Value, width: u8) {
    let at = element(f, column, index, i64::from(width));
    let flags = MemFlags::trusted();
    if width == 2 {
        f.ins().istore16(flags, value, at, 0);
    } else {
        f.ins().istore32(flags, value, at, 0);
    }
}

/// Compile a tier-2 program's native IR at `shape`. Refuses (`UnsupportedBackend`) a module
/// with any other word.
fn compile(native: &NativeModule, shape: Shape, checks: Arc<Vec<CompiledBlock>>) -> Result<CompiledStream, Error> {
    use StructuralOperation as S;
    let words = &native.structural;
    let ours = |op: &S| {
        matches!(
            op,
            S::Token
                | S::Switch { .. }
                | S::Span { .. }
                | S::Word { .. }
                | S::Check { .. }
                | S::Open { .. }
                | S::Close { .. }
                | S::Leaf { .. }
                | S::NodeField { .. }
                | S::NodeSuper { .. }
                | S::Push { .. }
                | S::Return { .. }
                | S::Accept
                | S::Stop
                | S::Goto { .. }
        )
    };
    if words.is_empty() || !words.iter().all(|b| ours(&b.operation)) || !words.iter().any(|b| b.operation == S::Token) {
        return Err(Error::UnsupportedBackend);
    }
    let (slot_of, _) = slots(native)?;
    // The states `RETURN` may land on: each `PUSH`'s target.
    let mut pushed: Vec<u16> = words
        .iter()
        .filter_map(|b| if let S::Push { target } = b.operation { Some(target) } else { None })
        .collect();
    pushed.sort_unstable();
    pushed.dedup();

    // Not optimised: `opt_level=speed` measured slower on the corpus (doc 19).
    let builder = JITBuilder::new(default_libcall_names()).map_err(|_| Error::BackendFailure)?;
    let mut module = JITModule::new(builder);
    let ptr = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(ptr));
    signature.returns.push(AbiParam::new(types::I32));
    let id = module
        .declare_function("tempo_stream", Linkage::Local, &signature)
        .map_err(|_| Error::BackendFailure)?;
    // The checks' signature (`native_cpu_block::Entry`).
    let mut check_sig = module.make_signature();
    for t in [ptr, ptr, types::I64, ptr] {
        check_sig.params.push(AbiParam::new(t));
    }
    let mut context = module.make_context();
    context.func.signature = signature;
    context.set_disasm(true);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut f = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let check_sig = f.import_signature(check_sig);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        let blocks: Vec<Block> = words.iter().map(|_| f.create_block()).collect();
        let refused = f.create_block();
        f.append_block_param(refused, types::I64);
        let full = f.create_block();

        f.switch_to_block(entry);
        let cx = f.block_params(entry)[0];
        let ro = MemFlags::trusted().with_readonly();
        let i = Inputs { cx };
        let var = |f: &mut FunctionBuilder<'_>| {
            let v = f.declare_var(types::I64);
            let zero = f.ins().iconst(types::I64, 0);
            f.def_var(v, zero);
            v
        };
        let r = Regs {
            next: var(&mut f),
            taken: var(&mut f),
            start: var(&mut f),
            end: var(&mut f),
            limit: var(&mut f),
            last_end: var(&mut f),
            at_end: var(&mut f),
            nodes: var(&mut f),
            open: var(&mut f),
            returns: var(&mut f),
            codes: var(&mut f),
            base: var(&mut f),
            last_start: var(&mut f),
            or_off: var(&mut f),
            or_last: var(&mut f),
            or_len: var(&mut f),
            before_last: var(&mut f),
        };
        f.ins().jump(blocks[0], &[]);

        let rw = MemFlags::trusted();
        // Bytes past a token: the file's padding (`PADDING`).
        let any = MemFlags::new().with_notrap();
        // A node closes (`CLOSE`, `ACCEPT`) at `e`: its size and tail, and the columns' ORs.
        let close = |f: &mut FunctionBuilder<'_>, r: &Regs, i: &Inputs, at: Value, own_start: Value, e: Value| {
            let nn = f.use_var(r.nodes);
            let size = f.ins().isub(nn, at);
            let in_size = i.get(f, offset_of!(Context, size));
            put(f, in_size, at, size, shape.size);
            let last = f.use_var(r.last_start);
            let from_last = f.ins().isub(e, last);
            let length = f.ins().isub(e, own_start);
            let before = f.ins().icmp(IntCC::UnsignedLessThan, e, last);
            let before = f.ins().uextend(types::I64, before);
            let b = f.use_var(r.before_last);
            let b = f.ins().bor(b, before);
            f.def_var(r.before_last, b);
            let o = f.use_var(r.or_last);
            let o = f.ins().bor(o, from_last);
            f.def_var(r.or_last, o);
            let o = f.use_var(r.or_len);
            let o = f.ins().bor(o, length);
            f.def_var(r.or_len, o);
            let in_tail = i.get(f, offset_of!(Context, tail));
            put(f, in_tail, at, if shape.from_last { from_last } else { length }, shape.tail);
        };
        for (pc, block) in words.iter().enumerate() {
            f.switch_to_block(blocks[pc]);
            let here = f.ins().iconst(types::I64, pc as i64);
            let mut fall_through = true;
            match block.operation {
                S::Token => {
                    let at_end = f.use_var(r.at_end);
                    guard(&mut f, at_end, refused, &[here]);
                    let end = f.use_var(r.end);
                    f.def_var(r.last_end, end);
                    let next = f.use_var(r.next);
                    let in_nstarts = i.get(&mut f, offset_of!(Context, nstarts));
                    let is_end = f.ins().icmp(IntCC::Equal, next, in_nstarts);
                    let (ended, take, after) = (f.create_block(), f.create_block(), f.create_block());
                    f.ins().brif(is_end, ended, &[], take, &[]);
                    f.switch_to_block(ended);
                    let one = f.ins().iconst(types::I64, 1);
                    f.def_var(r.at_end, one);
                    f.ins().jump(after, &[]);
                    f.switch_to_block(take);
                    let in_starts = i.get(&mut f, offset_of!(Context, starts));
                    let at = element(&mut f, in_starts, next, 4);
                    let s = f.ins().uload32(ro, at, 0);
                    let inside = f.ins().icmp(IntCC::UnsignedLessThan, s, end);
                    guard(&mut f, inside, refused, &[here]);
                    f.def_var(r.start, s);
                    f.def_var(r.end, s);
                    let n1 = f.ins().iadd_imm(next, 1);
                    f.def_var(r.next, n1);
                    // The next token's first byte bounds this one; after the last, the
                    // sentinel, the file's length.
                    let limit = f.ins().uload32(ro, at, 4);
                    f.def_var(r.limit, limit);
                    f.ins().jump(after, &[]);
                    f.switch_to_block(after);
                }
                S::Span { last, len } => {
                    let start = f.use_var(r.start);
                    let e = if last {
                        let taken = f.use_var(r.taken);
                        let in_nlasts = i.get(&mut f, offset_of!(Context, nlasts));
                        let none = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, taken, in_nlasts);
                        guard(&mut f, none, refused, &[here]);
                        let in_lasts = i.get(&mut f, offset_of!(Context, lasts));
                        let at = element(&mut f, in_lasts, taken, 4);
                        let l = f.ins().uload32(ro, at, 0);
                        let t1 = f.ins().iadd_imm(taken, 1);
                        f.def_var(r.taken, t1);
                        let before = f.ins().icmp(IntCC::UnsignedLessThan, l, start);
                        guard(&mut f, before, refused, &[here]);
                        f.ins().iadd_imm(l, 1)
                    } else {
                        f.ins().iadd_imm(start, i64::from(len))
                    };
                    let limit = f.use_var(r.limit);
                    let past = f.ins().icmp(IntCC::UnsignedGreaterThan, e, limit);
                    guard(&mut f, past, refused, &[here]);
                    f.def_var(r.end, e);
                }
                S::Switch { table } => {
                    let s = &native.switches[usize::from(table)];
                    let refuse = f.create_block();
                    let to = |t: u16| if t == REFUSE { refuse } else { blocks[usize::from(t)] };
                    let at_end = f.use_var(r.at_end);
                    let on = f.create_block();
                    f.ins().brif(at_end, to(s.end), &[], on, &[]);
                    f.switch_to_block(on);
                    let start = f.use_var(r.start);
                    let at = f.ins().iadd_imm(start, i64::from(s.offset));
                    let limit = f.use_var(r.limit);
                    let past = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, at, limit);
                    guard(&mut f, past, refused, &[here]);
                    let in_bytes = i.get(&mut f, offset_of!(Context, bytes));
                    let p = f.ins().iadd(in_bytes, at);
                    let byte = f.ins().uload8(types::I32, ro, p, 0);
                    // The byte's runs of one target, searched by compares, as a C compiler lowers
                    // simdjson's `switch (*value)`: a jump table's `csdb` barrier would stop
                    // speculation at every token.
                    let mut runs: Vec<(u32, Block)> = Vec::new();
                    for (b, &t) in s.targets.iter().enumerate() {
                        if runs.last().is_none_or(|&(_, last)| last != to(t)) {
                            runs.push((b as u32, to(t)));
                        }
                    }
                    search(&mut f, byte, &runs);
                    f.switch_to_block(refuse);
                    f.ins().jump(refused, &[here.into()]);
                    fall_through = false;
                }
                S::Word { string } => {
                    let s = &native.strings[usize::from(string)];
                    let (start, end) = (f.use_var(r.start), f.use_var(r.end));
                    let n = f.ins().isub(end, start);
                    let other = f.ins().icmp_imm(IntCC::NotEqual, n, s.len() as i64);
                    guard(&mut f, other, refused, &[here]);
                    // One load of the token's bytes and a compare, as simdjson's
                    // `is_valid_true_atom`.
                    let in_bytes = i.get(&mut f, offset_of!(Context, bytes));
                    let p = f.ins().iadd(in_bytes, start);
                    let w = f.ins().load(types::I64, any, p, 0);
                    let w = if s.len() < 8 { f.ins().band_imm(w, ((1u64 << (8 * s.len())) - 1) as i64) } else { w };
                    let mut want = [0u8; 8];
                    want[..s.len()].copy_from_slice(s);
                    let ne = f.ins().icmp_imm(IntCC::NotEqual, w, u64::from_le_bytes(want) as i64);
                    guard(&mut f, ne, refused, &[here]);
                }
                S::Check { check } => {
                    let (address, delay) = checks[usize::from(check)].entry();
                    let (start, end) = (f.use_var(r.start), f.use_var(r.end));
                    let n = f.ins().isub(end, start);
                    // The check runs where the token is, as simdjson's `parse_number`: its row
                    // is the token's start in the file's bytes, its blocks and its length. The
                    // block function's edge blocks read nothing outside the token.
                    let zero = f.ins().iconst(types::I64, 0);
                    let blocks_n = f.ins().iadd_imm(n, i64::from(delay) + 63);
                    let blocks_n = f.ins().ushr_imm(blocks_n, 6);
                    let row = offset_of!(Context, row) as i32;
                    f.ins().store(rw, start, i.cx, row);
                    f.ins().store(rw, blocks_n, i.cx, row + 8);
                    f.ins().store(rw, n, i.cx, row + 16);
                    f.ins().store(rw, zero, i.cx, row + 24);
                    let rows = f.ins().iadd_imm(i.cx, i64::from(row));
                    let one = f.ins().iconst(types::I64, 1);
                    let callee = f.ins().iconst(ptr, address as i64);
                    let in_bytes = i.get(&mut f, offset_of!(Context, bytes));
                    let in_words = i.get(&mut f, offset_of!(Context, words));
                    f.ins().call_indirect(check_sig, callee, &[in_bytes, rows, one, in_words]);
                    // The check's one output: any bit refuses, a word per 64 bytes.
                    let (head, body, done) = (f.create_block(), f.create_block(), f.create_block());
                    f.append_block_param(head, types::I64);
                    f.append_block_param(head, types::I64);
                    f.append_block_param(done, types::I64);
                    f.ins().jump(head, &[zero.into(), zero.into()]);
                    f.switch_to_block(head);
                    let (j, acc) = (f.block_params(head)[0], f.block_params(head)[1]);
                    let more = f.ins().icmp(IntCC::UnsignedLessThan, j, blocks_n);
                    f.ins().brif(more, body, &[], done, &[acc.into()]);
                    f.switch_to_block(body);
                    let in_words = i.get(&mut f, offset_of!(Context, words));
                    let at = element(&mut f, in_words, j, 8);
                    let w = f.ins().load(types::I64, rw, at, 0);
                    let acc = f.ins().bor(acc, w);
                    let j1 = f.ins().iadd_imm(j, 1);
                    f.ins().jump(head, &[j1.into(), acc.into()]);
                    f.switch_to_block(done);
                    let acc = f.block_params(done)[0];
                    let fails = f.ins().icmp_imm(IntCC::NotEqual, acc, 0);
                    guard(&mut f, fails, refused, &[here]);
                }
                S::Open { .. } | S::Leaf { .. } => {
                    let leaf = matches!(block.operation, S::Leaf { .. });
                    let slot = slot_of[pc].expect("every node word has a slot");
                    let nn = f.use_var(r.nodes);
                    let in_node_cap = i.get(&mut f, offset_of!(Context, node_cap));
                    let no_room = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, nn, in_node_cap);
                    guard(&mut f, no_room, full, &[]);
                    let start = f.use_var(r.start);
                    // The base of each 16 nodes is its first node's start (starts do not fall
                    // in preorder), and a node's offset its start less the base.
                    let (first, based) = (f.create_block(), f.create_block());
                    let low = f.ins().band_imm(nn, 15);
                    f.ins().brif(low, based, &[], first, &[]);
                    f.switch_to_block(first);
                    let b = f.ins().ushr_imm(nn, 4);
                    let in_base = i.get(&mut f, offset_of!(Context, base));
                    put(&mut f, in_base, b, start, 4);
                    f.def_var(r.base, start);
                    f.ins().jump(based, &[]);
                    f.switch_to_block(based);
                    let b = f.use_var(r.base);
                    let off = f.ins().isub(start, b);
                    let in_off = i.get(&mut f, offset_of!(Context, off));
                    put(&mut f, in_off, nn, off, shape.off);
                    let o = f.use_var(r.or_off);
                    let o = f.ins().bor(o, off);
                    f.def_var(r.or_off, o);
                    // The code: its slot's number, given at its first use.
                    let in_ids = i.get(&mut f, offset_of!(Context, ids));
                    let at = f.ins().iadd_imm(in_ids, 2 * i64::from(slot));
                    let id = f.ins().uload16(types::I64, rw, at, 0);
                    let unset = f.ins().icmp_imm(IntCC::Equal, id, 0xffff);
                    let (new, coded) = (f.create_block(), f.create_block());
                    f.append_block_param(coded, types::I64);
                    f.ins().brif(unset, new, &[], coded, &[id.into()]);
                    f.switch_to_block(new);
                    let nc = f.use_var(r.codes);
                    f.ins().istore16(rw, nc, at, 0);
                    let s = f.ins().iconst(types::I64, i64::from(slot));
                    let in_order = i.get(&mut f, offset_of!(Context, order));
                    put(&mut f, in_order, nc, s, 2);
                    let nc1 = f.ins().iadd_imm(nc, 1);
                    f.def_var(r.codes, nc1);
                    f.ins().jump(coded, &[nc.into()]);
                    f.switch_to_block(coded);
                    let id = f.block_params(coded)[0];
                    let in_code = i.get(&mut f, offset_of!(Context, code));
                    put(&mut f, in_code, nn, id, 2);
                    f.def_var(r.last_start, start);
                    if leaf {
                        // A leaf is its own last descendant: its tail is its length.
                        let one = f.ins().iconst(types::I64, 1);
                        let in_size = i.get(&mut f, offset_of!(Context, size));
                        put(&mut f, in_size, nn, one, shape.size);
                        let end = f.use_var(r.end);
                        let length = f.ins().isub(end, start);
                        let in_tail = i.get(&mut f, offset_of!(Context, tail));
                        put(&mut f, in_tail, nn, length, shape.tail);
                        for v in [r.or_last, r.or_len] {
                            let o = f.use_var(v);
                            let o = f.ins().bor(o, length);
                            f.def_var(v, o);
                        }
                    } else {
                        let no = f.use_var(r.open);
                        let in_stack_cap = i.get(&mut f, offset_of!(Context, stack_cap));
                        let deep = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, no, in_stack_cap);
                        guard(&mut f, deep, full, &[]);
                        let in_open = i.get(&mut f, offset_of!(Context, open));
                        let at = element(&mut f, in_open, no, 8);
                        let nn32 = f.ins().ireduce(types::I32, nn);
                        f.ins().store(rw, nn32, at, 0);
                        let s32 = f.ins().ireduce(types::I32, start);
                        f.ins().store(rw, s32, at, 4);
                        let no1 = f.ins().iadd_imm(no, 1);
                        f.def_var(r.open, no1);
                    }
                    let nn1 = f.ins().iadd_imm(nn, 1);
                    f.def_var(r.nodes, nn1);
                }
                S::Close { this } => {
                    let no = f.use_var(r.open);
                    let none = f.ins().icmp_imm(IntCC::Equal, no, 0);
                    guard(&mut f, none, refused, &[here]);
                    let no1 = f.ins().iadd_imm(no, -1);
                    f.def_var(r.open, no1);
                    let in_open = i.get(&mut f, offset_of!(Context, open));
                    let slot = element(&mut f, in_open, no1, 8);
                    let at = f.ins().uload32(rw, slot, 0);
                    let own = f.ins().uload32(rw, slot, 4);
                    let e = if this { f.use_var(r.end) } else { f.use_var(r.last_end) };
                    close(&mut f, &r, &i, at, own, e);
                }
                // Written with their node, whose code they are.
                S::NodeField { .. } | S::NodeSuper { .. } => {}
                S::Push { target } => {
                    let nr = f.use_var(r.returns);
                    let in_stack_cap = i.get(&mut f, offset_of!(Context, stack_cap));
                    let deep = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, nr, in_stack_cap);
                    guard(&mut f, deep, full, &[]);
                    let in_returns = i.get(&mut f, offset_of!(Context, returns));
                    let slot = element(&mut f, in_returns, nr, 4);
                    let t = f.ins().iconst(types::I32, i64::from(target));
                    f.ins().store(rw, t, slot, 0);
                    let nr1 = f.ins().iadd_imm(nr, 1);
                    f.def_var(r.returns, nr1);
                }
                S::Return { again } => {
                    let nr = f.use_var(r.returns);
                    let none = f.ins().icmp_imm(IntCC::Equal, nr, 0);
                    guard(&mut f, none, refused, &[here]);
                    let nr1 = f.ins().iadd_imm(nr, -1);
                    f.def_var(r.returns, nr1);
                    let in_returns = i.get(&mut f, offset_of!(Context, returns));
                    let slot = element(&mut f, in_returns, nr1, 4);
                    let p = f.ins().uload32(rw, slot, 0);
                    let refuse = f.create_block();
                    // The states that `PUSH` names, searched by compares (no jump table, whose
                    // `csdb` would stop speculation).
                    let mut runs: Vec<(u32, Block)> = vec![(0, refuse)];
                    for &t in &pushed {
                        let to = (u32::from(t), blocks[usize::from(t) + usize::from(again)]);
                        match runs.last_mut() {
                            Some(last) if last.0 == to.0 => *last = to,
                            _ => runs.push(to),
                        }
                        runs.push((u32::from(t) + 1, refuse));
                    }
                    search(&mut f, p, &runs);
                    f.switch_to_block(refuse);
                    f.ins().jump(refused, &[here.into()]);
                    fall_through = false;
                }
                S::Accept => {
                    let at_end = f.use_var(r.at_end);
                    let not_end = f.ins().icmp_imm(IntCC::Equal, at_end, 0);
                    let no = f.use_var(r.open);
                    let not_one = f.ins().icmp_imm(IntCC::NotEqual, no, 1);
                    let nr = f.use_var(r.returns);
                    let calls = f.ins().icmp_imm(IntCC::NotEqual, nr, 0);
                    let taken = f.use_var(r.taken);
                    let in_nlasts = i.get(&mut f, offset_of!(Context, nlasts));
                    let left = f.ins().icmp(IntCC::NotEqual, taken, in_nlasts);
                    let a = f.ins().bor(not_end, not_one);
                    let b = f.ins().bor(calls, left);
                    let bad = f.ins().bor(a, b);
                    guard(&mut f, bad, refused, &[here]);
                    // The root closes at the file's end.
                    let in_open = i.get(&mut f, offset_of!(Context, open));
                    let at = f.ins().uload32(rw, in_open, 0);
                    let in_open = i.get(&mut f, offset_of!(Context, open));
                    let own = f.ins().uload32(rw, in_open, 4);
                    let in_len = i.get(&mut f, offset_of!(Context, len));
                    close(&mut f, &r, &i, at, own, in_len);
                    for (v, o) in [
                        (r.nodes, offset_of!(Context, node_count)),
                        (r.codes, offset_of!(Context, code_count)),
                        (r.or_off, offset_of!(Context, or_off)),
                        (r.or_last, offset_of!(Context, or_last)),
                        (r.or_len, offset_of!(Context, or_len)),
                        (r.before_last, offset_of!(Context, before_last)),
                    ] {
                        let x = f.use_var(v);
                        f.ins().store(rw, x, i.cx, o as i32);
                    }
                    let ok = f.ins().iconst(types::I32, 0);
                    f.ins().return_(&[ok]);
                    fall_through = false;
                }
                S::Goto { target } => {
                    f.ins().jump(blocks[usize::from(target)], &[]);
                    fall_through = false;
                }
                _ => {
                    // `STOP`: the file goes to tree-sitter.
                    f.ins().jump(refused, &[here.into()]);
                    fall_through = false;
                }
            }
            if fall_through {
                match block.next {
                    Some(n) => f.ins().jump(blocks[usize::from(n)], &[]),
                    None => f.ins().jump(refused, &[here.into()]),
                };
            }
        }

        f.switch_to_block(refused);
        let pc = f.block_params(refused)[0];
        f.ins().store(rw, pc, i.cx, offset_of!(Context, pc) as i32);
        let one = f.ins().iconst(types::I32, 1);
        f.ins().return_(&[one]);
        f.switch_to_block(full);
        let two = f.ins().iconst(types::I32, 2);
        f.ins().return_(&[two]);
        f.seal_all_blocks();
        f.finalize();
    }
    let clif = context.func.display().to_string();
    super::native_cpu_codegen::define_function(&mut module, id, &mut context).map_err(|_| Error::BackendFailure)?;
    let disassembly = context.compiled_code().and_then(|code| code.vcode.clone()).unwrap_or_default();
    module.clear_context(&mut context);
    module.finalize_definitions().map_err(|_| Error::BackendFailure)?;
    let code = module.get_finalized_function(id);
    // SAFETY: `tempo_stream` was declared and defined with this signature.
    let entry = unsafe { std::mem::transmute::<*const u8, Entry>(code) };
    Ok(CompiledStream { _module: module, entry, _checks: checks, clif, disassembly })
}

/// The nodes a tape's columns hold, as `stream::run` gives them (supertypes ascending): for
/// tests, which compare the compiled words with their interpreter.
#[cfg(test)]
pub(super) fn nodes_of(c: &Columns) -> Vec<super::stream::Node> {
    let cell = |col: &[u8], i: usize, w: u8| -> u32 {
        let at = i * usize::from(w);
        if w == 2 { u32::from(u16::from_le_bytes([col[at], col[at + 1]])) } else { u32::from_le_bytes(col[at..at + 4].try_into().expect("4 bytes")) }
    };
    let start = |i: usize| cell(&c.base, i / 16, 4) + cell(&c.off, i, c.shape.off);
    (0..c.nodes)
        .map(|i| {
            let (kf, supers) = &c.codes[cell(&c.code, i, 2) as usize];
            let size = cell(&c.size, i, c.shape.size);
            let from = if c.shape.from_last { start(i + size as usize - 1) } else { start(i) };
            super::stream::Node { kind: kf & 0x3ff, field: kf >> 10, supers: supers.clone(), start: start(i), end: from + cell(&c.tail, i, c.shape.tail), size }
        })
        .collect()
}

#[cfg(test)]
impl Tier2Cpu {
    /// The compiled function's CPU time over one file (thread user + system), best of `reps`,
    /// at the widths its tape takes, its buffers made before: the cost of the code alone, as
    /// simdjson's stage 2 is timed with its tape allocated.
    pub(super) fn call_cpu_seconds(&self, bytes: &[u8], starts: &[u32], lasts: &[u32], reps: usize) -> Option<f64> {
        let shape = self.tape(bytes, starts, lasts).ok()?.shape;
        let f = self.function(shape).ok()?;
        let mut padded = vec![0u8; FRONT];
        padded.extend_from_slice(bytes);
        padded.resize(FRONT + bytes.len() + PADDING, 0);
        let mut stream = starts.to_vec();
        stream.push(bytes.len() as u32);
        let mut out = Buffers::new(4 * starts.len() + 8, shape, self.slots.len(), bytes.len());
        let now = || {
            let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
            // SAFETY: `t` is a valid timespec to write.
            unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
            t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
        };
        (0..reps)
            .map(|_| {
                out.ids.fill(u16::MAX);
                let start = now();
                let run = f.call(&padded[FRONT..], bytes.len(), &stream, lasts, &mut out);
                let t = now() - start;
                assert_eq!(run.status, 0);
                t
            })
            .reduce(f64::min)
    }
}
