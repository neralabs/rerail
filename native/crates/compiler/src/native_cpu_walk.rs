//! A walk on the CPU (decision 0139): a rounds group that is regions (`block::Walk`), made by
//! walking the block's regions left to right instead of running the rounds to a fixed point.
//!
//! Each walk is a function of its own, generated from the program and called by the block's
//! body once a block (only the hot loop is inlined). It loops over the block's regions, never
//! over its bytes. At each step it finds the next opener outside a region (a trailing-zero
//! count of the openers past the resume point), then that region's closer (a trailing-zero
//! count of the closer past the opener). A region open at the block's end is carried to the
//! next block in the walk's state (`native_cpu_block::WalkState`), as a string's state is.
//!
//! An `else` asks whether the closer comes anywhere later in the file. Past the block, the walk
//! scans the bytes for it: the closer is byte tests (`block::byte_tests`), tested eight bytes a
//! word. A scan that finds one opens the region, which runs to it. A scan that finds none sets
//! the kind's flag in the state, so the file's later openers of the kind take the `else` with
//! no scan. So the scans read each byte of the file at most once.

use crate::Error;
use crate::block::ByteTest;
use crate::block_native::{BlockInst, BlockNative};
use crate::native_cpu_block::WalkState;
use crate::native_cpu_shape::Step;
use cranelift_codegen::ir::{AbiParam, Block, Function, InstBuilder, MemFlags, Signature, UserFuncName, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::Module;

/// The walks' functions of a pass: declared before the bodies are built, each built where a
/// body first calls it, and defined with the pass.
pub struct WalkCalls {
    pub(crate) ids: Vec<cranelift_module::FuncId>,
    pub(crate) signature: Signature,
    pub(crate) built: std::cell::RefCell<Vec<Option<Function>>>,
}

/// A walk's function's signature: its words' address (`layout`), the block's first byte, the
/// block's index, the stream's length.
pub fn signature(module: &JITModule) -> Signature {
    let mut signature = module.make_signature();
    for _ in 0..4 {
        signature.params.push(AbiParam::new(types::I64));
    }
    signature
}

impl WalkCalls {
    /// The walks of `native` declared in `module`, named after the pass.
    pub fn declare(module: &mut JITModule, name: &str, native: &BlockNative) -> Result<Self, Error> {
        let signature = signature(module);
        let ids = (0..native.walks.len())
            .map(|w| module.declare_function(&format!("{name}_walk{w}"), cranelift_module::Linkage::Local, &signature).map_err(|_| Error::BackendFailure))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(WalkCalls { ids, signature, built: std::cell::RefCell::new(vec![None; native.walks.len()]) })
    }
}

/// The words a walk's function reads and writes, eight bytes each: the state (in, then out),
/// each kind's opener and closer, then the values it makes (`block::Walk::made`).
pub fn layout(kinds: usize, made: usize) -> usize {
    1 + 2 * kinds + made
}

/// Value `v` of the CPU's form as byte tests, as `block::byte_tests` gives them on the program.
pub fn byte_tests(insts: &[BlockInst], v: u16) -> Option<Vec<(u8, ByteTest)>> {
    match insts[usize::from(v)] {
        BlockInst::EqByte(b) => Some(vec![(0, ByteTest::Eq(b))]),
        BlockInst::Within(d) if d <= 16 => Some(vec![(d, ByteTest::Inside)]),
        BlockInst::Prev(a, k) => {
            let inner = byte_tests(insts, a)?;
            inner.into_iter().map(|(o, t)| Some((o.checked_add(k).filter(|&o| o <= 16)?, t))).collect()
        }
        BlockInst::And(a, b) => {
            let mut out = byte_tests(insts, a)?;
            out.extend(byte_tests(insts, b)?);
            out.sort_by_key(|x| x.0);
            out.dedup();
            Some(out)
        }
        _ => None,
    }
}

/// Bits `0..n` of a word, `n` from 0 to 64.
fn low(f: &mut FunctionBuilder<'_>, n: Value) -> Value {
    let one = f.ins().iconst(types::I64, 1);
    let s = f.ins().ishl(one, n);
    let m = f.ins().iadd_imm(s, -1);
    let big = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, n, 64);
    let all = f.ins().iconst(types::I64, -1);
    f.ins().select(big, all, m)
}

/// Bits `n..64` of a word, `n` from 0 to 64 or more (none).
fn from(f: &mut FunctionBuilder<'_>, n: Value) -> Value {
    let all = f.ins().iconst(types::I64, -1);
    let s = f.ins().ishl(all, n);
    let big = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, n, 64);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().select(big, zero, s)
}

/// Bits `t + 1..64`, `t` from 0 to 63.
fn after(f: &mut FunctionBuilder<'_>, t: Value) -> Value {
    let all = f.ins().iconst(types::I64, -1);
    let s = f.ins().ishl(all, t);
    f.ins().ishl_imm(s, 1)
}

/// Each byte's top bit set where the byte is `b`: exact, with no carry between bytes.
fn eq_bytes(f: &mut FunctionBuilder<'_>, x: Value, b: u8) -> Value {
    const LOW7: i64 = 0x7f7f_7f7f_7f7f_7f7f;
    let y = f.ins().bxor_imm(x, (0x0101_0101_0101_0101u64.wrapping_mul(u64::from(b))) as i64);
    let t = f.ins().band_imm(y, LOW7);
    let t = f.ins().iadd_imm(t, LOW7);
    let t = f.ins().bor(t, y);
    let t = f.ins().bor_imm(t, LOW7);
    f.ins().bnot(t)
}

/// Walk `w`'s function (`layout`), named by its id: the block's regions, left to right, from
/// the state carried in; the values made, and the state out. Its loops are over the block's
/// regions (`Step::Region`) and a scan's words (`Step::Word`), checked so.
pub fn build_walk(native: &BlockNative, w: usize, calls: &WalkCalls) -> Result<Function, Error> {
    let walk = &native.walks[w];
    let kinds = walk.kinds.len();
    let made = walk.made();
    let scans: Vec<Option<Vec<(u8, ByteTest)>>> = walk
        .kinds
        .iter()
        .map(|k| k.empty.map(|_| byte_tests(&native.insts, k.closer).filter(|t| t.iter().any(|x| x.1 == ByteTest::Inside)).ok_or(Error::InvalidTarget(0))).transpose())
        .collect::<Result<_, _>>()?;
    let mut func = Function::with_name_signature(UserFuncName::user(0, calls.ids[w].as_u32()), calls.signature.clone());
    let mut ctx = FunctionBuilderContext::new();
    let mut heads: Vec<(Block, Step)> = Vec::new();
    {
        let mut f = FunctionBuilder::new(&mut func, &mut ctx);
        let flags = MemFlags::trusted();
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        f.switch_to_block(entry);
        let p = f.block_params(entry).to_vec();
        let (io, base, i, len) = (p[0], p[1], p[2], p[3]);
        let state = f.ins().load(types::I64, flags, io, 0);
        let openers: Vec<Value> = (0..kinds).map(|k| f.ins().load(types::I64, flags, io, 8 * (1 + 2 * k as i32))).collect();
        let closers: Vec<Value> = (0..kinds).map(|k| f.ins().load(types::I64, flags, io, 8 * (2 + 2 * k as i32))).collect();
        // Each value made, by its place in `made`: its word's offset in `io`.
        let out = |v: u16| 8 * (1 + 2 * kinds + made.iter().position(|&m| m == v).expect("made")) as i32;
        let zero = f.ins().iconst(types::I64, 0);
        for &v in &made[1..] {
            f.ins().store(flags, zero, io, out(v));
        }
        // OR `bits` into value `v`'s word.
        let or = |f: &mut FunctionBuilder<'_>, v: u16, bits: Value| {
            let old = f.ins().load(types::I64, flags, io, out(v));
            let new = f.ins().bor(old, bits);
            f.ins().store(flags, new, io, out(v));
        };
        let open = f.ins().band_imm(state, (1 << WalkState::SKIP) - 1);
        let skip = f.ins().ushr_imm(state, i64::from(WalkState::SKIP));
        let skip = f.ins().band_imm(skip, 3);
        // The hidden bytes the region before hides past its block.
        let hidden = low(&mut f, skip);
        f.ins().store(flags, hidden, io, out(walk.hidden));

        let head = f.create_block();
        f.append_block_param(head, types::I64);
        let exit = f.create_block();
        // (the kind open plus one, the resume place)
        f.append_block_param(exit, types::I64);
        f.append_block_param(exit, types::I64);

        // A region open at the block's start: kind `open - 1`, to its closer's first byte.
        let cont = f.create_block();
        f.ins().brif(open, cont, &[], head, &[skip.into()]);
        f.switch_to_block(cont);
        for (k, kind) in walk.kinds.iter().enumerate() {
            let this = f.create_block();
            if k + 1 < kinds {
                let next = f.create_block();
                let is = f.ins().icmp_imm(IntCC::Equal, open, k as i64 + 1);
                f.ins().brif(is, this, &[], next, &[]);
                f.switch_to_block(this);
                emit_cont(&mut f, kind, closers[k], &or, walk.hidden, head, exit, k);
                f.switch_to_block(next);
            } else {
                f.ins().jump(this, &[]);
                f.switch_to_block(this);
                emit_cont(&mut f, kind, closers[k], &or, walk.hidden, head, exit, k);
            }
        }

        // The loop over the regions: the next opener from place `r` on.
        f.switch_to_block(head);
        heads.push((head, Step::Region));
        let r = f.block_params(head)[0];
        let above = from(&mut f, r);
        let mut any = None;
        for &o in &openers {
            let m = f.ins().band(o, above);
            any = Some(match any {
                None => m,
                Some(a) => f.ins().bor(a, m),
            });
        }
        let any = any.expect("a kind");
        let found = f.create_block();
        f.ins().brif(any, found, &[], exit, &[zero.into(), r.into()]);
        f.switch_to_block(found);
        let t = f.ins().ctz(any);
        let one = f.ins().iconst(types::I64, 1);
        let bit = f.ins().ishl(one, t);
        // The first kind whose opener is at `t`.
        for (k, kind) in walk.kinds.iter().enumerate() {
            let this = f.create_block();
            if k + 1 < kinds {
                let next = f.create_block();
                let has = f.ins().band(openers[k], bit);
                f.ins().brif(has, this, &[], next, &[]);
                f.switch_to_block(this);
                emit_kind(&mut f, kind, &scans[k], closers[k], &or, walk.hidden, head, exit, k, (t, bit), (base, i, len), &mut heads);
                f.switch_to_block(next);
            } else {
                f.ins().jump(this, &[]);
                f.switch_to_block(this);
                emit_kind(&mut f, kind, &scans[k], closers[k], &or, walk.hidden, head, exit, k, (t, bit), (base, i, len), &mut heads);
            }
        }

        // The state out: the kind open, the places hidden past the block, the kinds' flags.
        f.switch_to_block(exit);
        let (open_out, r) = (f.block_params(exit)[0], f.block_params(exit)[1]);
        let past = f.ins().iadd_imm(r, -64);
        let over = f.ins().icmp_imm(IntCC::SignedGreaterThan, past, 0);
        let past = f.ins().select(over, past, zero);
        let past = f.ins().ishl_imm(past, i64::from(WalkState::SKIP));
        let now = f.ins().load(types::I64, flags, io, 0);
        let kept = f.ins().band_imm(now, !((1i64 << WalkState::NONE) - 1));
        let s = f.ins().bor(kept, open_out);
        let s = f.ins().bor(s, past);
        f.ins().store(flags, s, io, 0);
        f.ins().return_(&[]);
        f.seal_all_blocks();
        f.finalize();
    }
    crate::native_cpu_shape::check(&func, &heads, &[]).map_err(|r| Error::RefusedShape(format!("walk {w}: {r:?}")))?;
    Ok(func)
}

/// A region of kind `k` open at the block's start: closed at its closer's first bit, or open
/// through the block.
#[allow(clippy::too_many_arguments)]
fn emit_cont(f: &mut FunctionBuilder<'_>, kind: &crate::block::WalkKind, closer: Value, or: &dyn Fn(&mut FunctionBuilder<'_>, u16, Value), hidden: u16, head: Block, exit: Block, k: usize) {
    let (through, close) = (f.create_block(), f.create_block());
    f.ins().brif(closer, close, &[], through, &[]);
    f.switch_to_block(through);
    let all = f.ins().iconst(types::I64, -1);
    or(&mut *f, kind.fill, all);
    or(&mut *f, hidden, all);
    let open = f.ins().iconst(types::I64, k as i64 + 1);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().jump(exit, &[open.into(), zero.into()]);
    f.switch_to_block(close);
    let c = f.ins().ctz(closer);
    let fill = low(&mut *f, c);
    or(&mut *f, kind.fill, fill);
    let to = if kind.outside { c } else { f.ins().iadd_imm(c, i64::from(kind.bytes)) };
    let h = low(&mut *f, to);
    or(&mut *f, hidden, h);
    f.ins().jump(head, &[to.into()]);
}

/// An opener of kind `k` taken at place `t` (`bit`): its region to its closer's first bit after
/// it, or open through the block; with an `else` and no closer later in the file, the empty
/// value instead.
#[allow(clippy::too_many_arguments)]
fn emit_kind(
    f: &mut FunctionBuilder<'_>,
    kind: &crate::block::WalkKind,
    scan: &Option<Vec<(u8, ByteTest)>>,
    closer: Value,
    or: &dyn Fn(&mut FunctionBuilder<'_>, u16, Value),
    hidden: u16,
    head: Block,
    exit: Block,
    k: usize,
    (t, bit): (Value, Value),
    (base, i, len): (Value, Value, Value),
    heads: &mut Vec<(Block, Step)>,
) {
    let past = after(&mut *f, t);
    let cm = f.ins().band(closer, past);
    let (close, through) = (f.create_block(), f.create_block());
    // No closer in the block: open through it, or (an `else`) look past it first.
    let none = if scan.is_some() { f.create_block() } else { through };
    f.ins().brif(cm, close, &[], none, &[]);

    f.switch_to_block(close);
    let c = f.ins().ctz(cm);
    or(&mut *f, kind.open, bit);
    let all = f.ins().iconst(types::I64, -1);
    let start = f.ins().ishl(all, t);
    let below = low(&mut *f, c);
    let fill = f.ins().band(start, below);
    or(&mut *f, kind.fill, fill);
    let to = if kind.outside { c } else { f.ins().iadd_imm(c, i64::from(kind.bytes)) };
    let upto = low(&mut *f, to);
    let h = f.ins().band(upto, past);
    or(&mut *f, hidden, h);
    f.ins().jump(head, &[to.into()]);

    f.switch_to_block(through);
    or(&mut *f, kind.open, bit);
    let all = f.ins().iconst(types::I64, -1);
    let start = f.ins().ishl(all, t);
    or(&mut *f, kind.fill, start);
    let past2 = after(&mut *f, t);
    or(&mut *f, hidden, past2);
    let open = f.ins().iconst(types::I64, k as i64 + 1);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().jump(exit, &[open.into(), zero.into()]);

    let (Some(tests), Some((e, m))) = (scan, kind.empty) else { return };
    let flags = MemFlags::trusted();
    let empty = f.create_block();
    f.switch_to_block(none);
    // Known: no byte of the closer left in the file.
    let io_state = io_of(f);
    let state = f.ins().load(types::I64, flags, io_state, 0);
    let flag = f.ins().band_imm(state, 1i64 << (WalkState::NONE + k as u8));
    let scan_start = f.create_block();
    f.ins().brif(flag, empty, &[], scan_start, &[]);

    // The scan: places `64 (i + 1)` on, below `len + k` for the least `k` of an `Inside` test,
    // eight a word. Place `p`'s tests read the file's byte `p - k` for each `(k, test)`.
    f.switch_to_block(scan_start);
    let inside = tests.iter().filter(|x| x.1 == ByteTest::Inside).map(|x| x.0).min().expect("an inside test");
    let limit = f.ins().iadd_imm(len, i64::from(inside));
    let next = f.ins().iadd_imm(i, 1);
    let p0 = f.ins().ishl_imm(next, 6);
    let i64s = f.ins().ishl_imm(i, 6);
    let file = f.ins().isub(base, i64s);
    let word = f.create_block();
    f.append_block_param(word, types::I64);
    f.ins().jump(word, &[p0.into()]);
    f.switch_to_block(word);
    heads.push((word, Step::Word));
    let p = f.block_params(word)[0];
    let (body, missing) = (f.create_block(), f.create_block());
    let more = f.ins().icmp(IntCC::SignedLessThan, p, limit);
    f.ins().brif(more, body, &[], missing, &[]);
    f.switch_to_block(body);
    let at = f.ins().iadd(file, p);
    let mut hits = f.ins().iconst(types::I64, 0x8080_8080_8080_8080u64 as i64);
    for &(off, test) in tests {
        if let ByteTest::Eq(b) = test {
            let x = f.ins().load(types::I64, MemFlags::new().with_notrap().with_readonly(), at, -i32::from(off));
            let eq = eq_bytes(&mut *f, x, b);
            hits = f.ins().band(hits, eq);
        }
    }
    // The places below the limit: `8 (limit - p)` low bits, at most 64.
    let rem = f.ins().isub(limit, p);
    let rem = f.ins().ishl_imm(rem, 3);
    let room = low(&mut *f, rem);
    let hits = f.ins().band(hits, room);
    let on = f.ins().iadd_imm(p, 8);
    f.ins().brif(hits, through, &[], word, &[on.into()]);
    f.switch_to_block(missing);
    let state = f.ins().load(types::I64, flags, io_state, 0);
    let state = f.ins().bor_imm(state, 1i64 << (WalkState::NONE + k as u8));
    f.ins().store(flags, state, io_state, 0);
    f.ins().jump(empty, &[]);

    // The empty value at `t`; places `t + 1 ..= t + m - 1` hidden.
    f.switch_to_block(empty);
    or(&mut *f, e, bit);
    let to = f.ins().iadd_imm(t, i64::from(m));
    let upto = low(&mut *f, to);
    let h = f.ins().band(upto, past);
    or(&mut *f, hidden, h);
    f.ins().jump(head, &[to.into()]);
}

/// The words' address: the function's first parameter.
fn io_of(f: &mut FunctionBuilder<'_>) -> Value {
    let entry = f.func.layout.entry_block().expect("an entry");
    f.func.dfg.block_params(entry)[0]
}
