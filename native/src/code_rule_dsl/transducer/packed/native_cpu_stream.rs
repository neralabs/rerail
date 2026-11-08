//! Tier 2's words on the CPU JIT (decision 0053). Cranelift lowers the Program's words as
//! `native_ir` decodes them, one block per word, as it lowers the tree walker's: `GOTO` is a
//! jump, `SWITCH` a jump table on the token's byte (simdjson's `switch (*value)`), `RETURN` a
//! switch over the states that `PUSH` names, and `CHECK` a call of the check's compiled block
//! program (`native_cpu_block`), which runs over the token 64 bytes at a time.
//!
//! The function reads the file's bytes, padded, and the stream: every token's first byte, with
//! the file's length after the last (simdjson's sentinel index), and the tokens' last bytes. It
//! writes the nodes in preorder. Nothing here walks the words or the switch tables at run time:
//! they are the function's code.

use super::Error;
use super::native_cpu_block::{self, CompiledBlock, FRONT};
use super::native_ir::{NativeModule, StructuralOperation};
use super::stream::{Node, REFUSE, Refused};
use cranelift_codegen::ir::{AbiParam, Block, InstBuilder, JumpTableData, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};
use std::mem::offset_of;

/// A node as the function writes it: kind, field, start, end, size, and how many supertypes it
/// has (they follow, in the supertypes buffer, the ones of the nodes before it).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct NodeRecord {
    kind: u32,
    field: u32,
    start: u32,
    end: u32,
    size: u32,
    supers: u32,
}

/// The function's one argument: its inputs, its buffers, and what it reports.
#[repr(C)]
struct Context {
    bytes: *const u8,
    len: u64,
    starts: *const u32,
    nstarts: u64,
    lasts: *const u32,
    nlasts: u64,
    nodes: *mut NodeRecord,
    node_cap: u64,
    supers: *mut u32,
    super_cap: u64,
    open: *mut u32,
    returns: *mut u32,
    stack_cap: u64,
    /// A token, copied for its check: `FRONT` zero bytes, then the token, then zeros.
    scratch: *mut u8,
    /// The check's output words.
    words: *mut u64,
    /// The check's row (`native_cpu_block::Row`).
    row: [u64; 4],
    node_count: u64,
    super_count: u64,
    pc: u64,
}

/// The function: 0 the file is accepted, 1 a word refused it (`Context::pc`), 2 a buffer is full.
type Entry = unsafe extern "C" fn(*mut Context) -> u32;

/// Bytes readable past the file's end: `WORD`'s and a check's copy's loads may read past the
/// token (simdjson's `SIMDJSON_PADDING`).
const PADDING: usize = 64;

/// A compiled tier-2 program.
pub(super) struct CompiledStream {
    _module: JITModule,
    entry: Entry,
    /// The checks the function calls.
    _checks: Vec<CompiledBlock>,
    /// The function as Cranelift IR, before code generation.
    pub(super) clif: String,
}

impl CompiledStream {
    /// The nodes the words write over the stream (`starts`, `lasts`) of `bytes`, or where a word
    /// refused the file: as `stream::run`.
    pub(super) fn run(&self, bytes: &[u8], starts: &[u32], lasts: &[u32]) -> Result<Vec<Node>, Refused> {
        let mut padded = bytes.to_vec();
        padded.resize(bytes.len() + PADDING, 0);
        let mut starts = starts.to_vec();
        starts.push(bytes.len() as u32);
        let mut scratch = vec![0u8; FRONT + bytes.len() + 2 * PADDING + 16];
        let mut words = vec![0u64; bytes.len() / 64 + 3];
        let mut cap = 2 * starts.len() + 8;
        loop {
            let mut nodes = vec![NodeRecord::default(); cap];
            let mut supers = vec![0u32; cap];
            let mut open = vec![0u32; cap];
            let mut returns = vec![0u32; cap];
            let mut cx = Context {
                bytes: padded.as_ptr(),
                len: bytes.len() as u64,
                starts: starts.as_ptr(),
                nstarts: (starts.len() - 1) as u64,
                lasts: lasts.as_ptr(),
                nlasts: lasts.len() as u64,
                nodes: nodes.as_mut_ptr(),
                node_cap: cap as u64,
                supers: supers.as_mut_ptr(),
                super_cap: cap as u64,
                open: open.as_mut_ptr(),
                returns: returns.as_mut_ptr(),
                stack_cap: cap as u64,
                scratch: scratch.as_mut_ptr(),
                words: words.as_mut_ptr(),
                row: [0; 4],
                node_count: 0,
                super_count: 0,
                pc: 0,
            };
            // SAFETY: every pointer is to a buffer of the size the context gives, `bytes` with
            // `PADDING` after it, `starts` with its sentinel, `scratch` and `words` sized for the
            // longest token, the file.
            match unsafe { (self.entry)(&mut cx) } {
                0 => {
                    let mut at = 0usize;
                    return Ok(nodes[..cx.node_count as usize]
                        .iter()
                        .map(|n| {
                            let own = supers[at..at + n.supers as usize].iter().map(|&s| s as u16).collect();
                            at += n.supers as usize;
                            Node { kind: n.kind as u16, field: n.field as u16, supers: own, start: n.start, end: n.end, size: n.size }
                        })
                        .collect());
                }
                1 => return Err(Refused { pc: cx.pc as usize, why: "a compiled word refused the file" }),
                _ => cap *= 2,
            }
        }
    }
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
    supers: Variable,
    open: Variable,
    returns: Variable,
}

/// The context's fields, loaded once at the entry.
struct Inputs {
    cx: Value,
    bytes: Value,
    len: Value,
    starts: Value,
    nstarts: Value,
    lasts: Value,
    nlasts: Value,
    nodes: Value,
    node_cap: Value,
    supers: Value,
    super_cap: Value,
    open: Value,
    returns: Value,
    stack_cap: Value,
    scratch: Value,
    words: Value,
}

/// Branch to `to` (with `args`) when `cond`, and go on in a new block otherwise.
fn guard(f: &mut FunctionBuilder<'_>, cond: Value, to: Block, args: &[Value]) {
    let go_on = f.create_block();
    let args: Vec<cranelift_codegen::ir::BlockArg> = args.iter().map(|&v| v.into()).collect();
    f.ins().brif(cond, to, &args, go_on, &[]);
    f.switch_to_block(go_on);
}

/// `base[index]` for elements of `size` bytes.
fn element(f: &mut FunctionBuilder<'_>, base: Value, index: Value, size: i64) -> Value {
    let off = f.ins().imul_imm(index, size);
    f.ins().iadd(base, off)
}

/// Compile a tier-2 program's native IR. Refuses (`UnsupportedBackend`) a module with any other
/// word.
pub(super) fn compile(native: &NativeModule) -> Result<CompiledStream, Error> {
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
    let checks: Vec<CompiledBlock> = native
        .block_programs
        .first()
        .map(|b| b.checks.iter().map(native_cpu_block::compile).collect::<Result<_, _>>())
        .transpose()?
        .unwrap_or_default();
    // The states `RETURN` may land on: each `PUSH`'s target.
    let mut pushed: Vec<u16> = words
        .iter()
        .filter_map(|b| if let S::Push { target } = b.operation { Some(target) } else { None })
        .collect();
    pushed.sort_unstable();
    pushed.dedup();

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
        let field = |f: &mut FunctionBuilder<'_>, at: usize| f.ins().load(types::I64, ro, cx, at as i32);
        let i = Inputs {
            cx,
            bytes: field(&mut f, offset_of!(Context, bytes)),
            len: field(&mut f, offset_of!(Context, len)),
            starts: field(&mut f, offset_of!(Context, starts)),
            nstarts: field(&mut f, offset_of!(Context, nstarts)),
            lasts: field(&mut f, offset_of!(Context, lasts)),
            nlasts: field(&mut f, offset_of!(Context, nlasts)),
            nodes: field(&mut f, offset_of!(Context, nodes)),
            node_cap: field(&mut f, offset_of!(Context, node_cap)),
            supers: field(&mut f, offset_of!(Context, supers)),
            super_cap: field(&mut f, offset_of!(Context, super_cap)),
            open: field(&mut f, offset_of!(Context, open)),
            returns: field(&mut f, offset_of!(Context, returns)),
            stack_cap: field(&mut f, offset_of!(Context, stack_cap)),
            scratch: field(&mut f, offset_of!(Context, scratch)),
            words: field(&mut f, offset_of!(Context, words)),
        };
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
            supers: var(&mut f),
            open: var(&mut f),
            returns: var(&mut f),
        };
        f.ins().jump(blocks[0], &[]);

        let rw = MemFlags::trusted();
        // Bytes past a token: the file's padding (`PADDING`).
        let any = MemFlags::new().with_notrap();
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
                    let is_end = f.ins().icmp(IntCC::Equal, next, i.nstarts);
                    let (ended, take, after) = (f.create_block(), f.create_block(), f.create_block());
                    f.ins().brif(is_end, ended, &[], take, &[]);
                    f.switch_to_block(ended);
                    let one = f.ins().iconst(types::I64, 1);
                    f.def_var(r.at_end, one);
                    f.ins().jump(after, &[]);
                    f.switch_to_block(take);
                    let at = element(&mut f, i.starts, next, 4);
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
                        let none = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, taken, i.nlasts);
                        guard(&mut f, none, refused, &[here]);
                        let at = element(&mut f, i.lasts, taken, 4);
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
                    let p = f.ins().iadd(i.bytes, at);
                    let byte = f.ins().uload8(types::I32, ro, p, 0);
                    let table: Vec<_> = s.targets.iter().map(|&t| f.func.dfg.block_call(to(t), &[])).collect();
                    let default = f.func.dfg.block_call(refuse, &[]);
                    let jt = f.create_jump_table(JumpTableData::new(default, &table));
                    f.ins().br_table(byte, jt);
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
                    let p = f.ins().iadd(i.bytes, start);
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
                    let from = f.ins().iadd(i.bytes, start);
                    let to = f.ins().iadd_imm(i.scratch, FRONT as i64);
                    // The token, 16 bytes at a time, into the scratch after its zeros.
                    let (head, body, copied) = (f.create_block(), f.create_block(), f.create_block());
                    f.append_block_param(head, types::I64);
                    let zero = f.ins().iconst(types::I64, 0);
                    f.ins().jump(head, &[zero.into()]);
                    f.switch_to_block(head);
                    let k = f.block_params(head)[0];
                    let more = f.ins().icmp(IntCC::UnsignedLessThan, k, n);
                    f.ins().brif(more, body, &[], copied, &[]);
                    f.switch_to_block(body);
                    let src = f.ins().iadd(from, k);
                    let v = f.ins().load(types::I8X16, any, src, 0);
                    let dst = f.ins().iadd(to, k);
                    f.ins().store(rw, v, dst, 0);
                    let k16 = f.ins().iadd_imm(k, 16);
                    f.ins().jump(head, &[k16.into()]);
                    f.switch_to_block(copied);
                    // Zeros after it, over every block the check reads.
                    let z8 = f.ins().iconst(types::I8, 0);
                    let z = f.ins().splat(types::I8X16, z8);
                    let tail = f.ins().iadd(to, n);
                    for part in 0..5 {
                        f.ins().store(rw, z, tail, part * 16);
                    }
                    let blocks_n = f.ins().iadd_imm(n, i64::from(delay) + 63);
                    let blocks_n = f.ins().ushr_imm(blocks_n, 6);
                    let row = offset_of!(Context, row) as i32;
                    let front = f.ins().iconst(types::I64, FRONT as i64);
                    f.ins().store(rw, front, i.cx, row);
                    f.ins().store(rw, blocks_n, i.cx, row + 8);
                    f.ins().store(rw, n, i.cx, row + 16);
                    f.ins().store(rw, zero, i.cx, row + 24);
                    let rows = f.ins().iadd_imm(i.cx, i64::from(row));
                    let one = f.ins().iconst(types::I64, 1);
                    let callee = f.ins().iconst(ptr, address as i64);
                    f.ins().call_indirect(check_sig, callee, &[i.scratch, rows, one, i.words]);
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
                    let at = element(&mut f, i.words, j, 8);
                    let w = f.ins().load(types::I64, rw, at, 0);
                    let acc = f.ins().bor(acc, w);
                    let j1 = f.ins().iadd_imm(j, 1);
                    f.ins().jump(head, &[j1.into(), acc.into()]);
                    f.switch_to_block(done);
                    let acc = f.block_params(done)[0];
                    let fails = f.ins().icmp_imm(IntCC::NotEqual, acc, 0);
                    guard(&mut f, fails, refused, &[here]);
                }
                S::Open { kind } | S::Leaf { kind } => {
                    let leaf = matches!(block.operation, S::Leaf { .. });
                    let nn = f.use_var(r.nodes);
                    let no_room = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, nn, i.node_cap);
                    guard(&mut f, no_room, full, &[]);
                    let start = f.use_var(r.start);
                    let (end, size) = if leaf {
                        (f.use_var(r.end), f.ins().iconst(types::I64, 1))
                    } else {
                        let no = f.use_var(r.open);
                        let deep = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, no, i.stack_cap);
                        guard(&mut f, deep, full, &[]);
                        let at = element(&mut f, i.open, no, 4);
                        let nn32 = f.ins().ireduce(types::I32, nn);
                        f.ins().store(rw, nn32, at, 0);
                        let no1 = f.ins().iadd_imm(no, 1);
                        f.def_var(r.open, no1);
                        let z = f.ins().iconst(types::I64, 0);
                        (z, z)
                    };
                    let at = element(&mut f, i.nodes, nn, size_of::<NodeRecord>() as i64);
                    let k = f.ins().iconst(types::I32, i64::from(kind));
                    let z32 = f.ins().iconst(types::I32, 0);
                    f.ins().store(rw, k, at, offset_of!(NodeRecord, kind) as i32);
                    f.ins().store(rw, z32, at, offset_of!(NodeRecord, field) as i32);
                    for (v, o) in [(start, offset_of!(NodeRecord, start)), (end, offset_of!(NodeRecord, end)), (size, offset_of!(NodeRecord, size))] {
                        let v = f.ins().ireduce(types::I32, v);
                        f.ins().store(rw, v, at, o as i32);
                    }
                    f.ins().store(rw, z32, at, offset_of!(NodeRecord, supers) as i32);
                    let nn1 = f.ins().iadd_imm(nn, 1);
                    f.def_var(r.nodes, nn1);
                }
                S::Close { this } => {
                    let no = f.use_var(r.open);
                    let none = f.ins().icmp_imm(IntCC::Equal, no, 0);
                    guard(&mut f, none, refused, &[here]);
                    let no1 = f.ins().iadd_imm(no, -1);
                    f.def_var(r.open, no1);
                    let slot = element(&mut f, i.open, no1, 4);
                    let node = f.ins().uload32(rw, slot, 0);
                    let at = element(&mut f, i.nodes, node, size_of::<NodeRecord>() as i64);
                    let e = if this { f.use_var(r.end) } else { f.use_var(r.last_end) };
                    let e = f.ins().ireduce(types::I32, e);
                    f.ins().store(rw, e, at, offset_of!(NodeRecord, end) as i32);
                    let nn = f.use_var(r.nodes);
                    let size = f.ins().isub(nn, node);
                    let size = f.ins().ireduce(types::I32, size);
                    f.ins().store(rw, size, at, offset_of!(NodeRecord, size) as i32);
                }
                S::NodeField { field } | S::NodeSuper { kind: field } => {
                    let nn = f.use_var(r.nodes);
                    let none = f.ins().icmp_imm(IntCC::Equal, nn, 0);
                    guard(&mut f, none, refused, &[here]);
                    let last = f.ins().iadd_imm(nn, -1);
                    let at = element(&mut f, i.nodes, last, size_of::<NodeRecord>() as i64);
                    let v = f.ins().iconst(types::I32, i64::from(field));
                    if matches!(block.operation, S::NodeField { .. }) {
                        f.ins().store(rw, v, at, offset_of!(NodeRecord, field) as i32);
                    } else {
                        let ns = f.use_var(r.supers);
                        let no_room = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, ns, i.super_cap);
                        guard(&mut f, no_room, full, &[]);
                        let slot = element(&mut f, i.supers, ns, 4);
                        f.ins().store(rw, v, slot, 0);
                        let ns1 = f.ins().iadd_imm(ns, 1);
                        f.def_var(r.supers, ns1);
                        let o = offset_of!(NodeRecord, supers) as i32;
                        let c = f.ins().load(types::I32, rw, at, o);
                        let c = f.ins().iadd_imm(c, 1);
                        f.ins().store(rw, c, at, o);
                    }
                }
                S::Push { target } => {
                    let nr = f.use_var(r.returns);
                    let deep = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, nr, i.stack_cap);
                    guard(&mut f, deep, full, &[]);
                    let slot = element(&mut f, i.returns, nr, 4);
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
                    let slot = element(&mut f, i.returns, nr1, 4);
                    let p = f.ins().uload32(rw, slot, 0);
                    let refuse = f.create_block();
                    let mut back = Switch::new();
                    for &t in &pushed {
                        back.set_entry(u128::from(t), blocks[usize::from(t) + usize::from(again)]);
                    }
                    back.emit(&mut f, p, refuse);
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
                    let left = f.ins().icmp(IntCC::NotEqual, taken, i.nlasts);
                    let a = f.ins().bor(not_end, not_one);
                    let b = f.ins().bor(calls, left);
                    let bad = f.ins().bor(a, b);
                    guard(&mut f, bad, refused, &[here]);
                    let node = f.ins().load(types::I32, rw, i.open, 0);
                    let node = f.ins().uextend(types::I64, node);
                    let at = element(&mut f, i.nodes, node, size_of::<NodeRecord>() as i64);
                    let len = f.ins().ireduce(types::I32, i.len);
                    f.ins().store(rw, len, at, offset_of!(NodeRecord, end) as i32);
                    let nn = f.use_var(r.nodes);
                    let size = f.ins().isub(nn, node);
                    let size = f.ins().ireduce(types::I32, size);
                    f.ins().store(rw, size, at, offset_of!(NodeRecord, size) as i32);
                    f.ins().store(rw, nn, i.cx, offset_of!(Context, node_count) as i32);
                    let ns = f.use_var(r.supers);
                    f.ins().store(rw, ns, i.cx, offset_of!(Context, super_count) as i32);
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
    module.clear_context(&mut context);
    module.finalize_definitions().map_err(|_| Error::BackendFailure)?;
    let code = module.get_finalized_function(id);
    // SAFETY: `tempo_stream` was declared and defined with this signature.
    let entry = unsafe { std::mem::transmute::<*const u8, Entry>(code) };
    Ok(CompiledStream { _module: module, entry, _checks: checks, clif })
}
