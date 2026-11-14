//! Tier 1's outputs made from the file's bytes where the CPU's token loop reads them
//! (decision 0177), as the lean lexer makes its masks for the window it reads, and reads a
//! string's bytes from its opener.
//!
//! An output can be made so when its value is a function of the bytes near each position:
//! a byte-local value (`native_cpu_lanes`: byte tests, classes, `prev`s, `within`), the logic
//! and `prev`s of such values, and `escaped(a)` of one, whose carry in at a position is the
//! parity of `a`'s run that ends just before it. No other carried value is (a `toggle`, a
//! `fill`, a fixed point): those are read from the block's rows.
//!
//! Two forms, both from the program's instructions, so the same meaning as the block body:
//!
//! - **A window** (`word`): the output's 64 bits from any position, on the byte vectors
//!   (`Lanes::window`), cut to the file. An `escaped`'s carry in is one byte's test, and only
//!   where that byte is in its run, a cold loop a window at a time back to the run's start.
//! - **A bit** (`bit`): the output at one position, by scalar loads of its bytes.

use std::collections::HashMap;

use crate::block_native::{BlockInst, BlockNative};
use crate::native_cpu_block::Sym;
use crate::native_cpu_lanes::{Lanes, Plan};
use crate::native_cpu_shape::Step;
use cranelift_codegen::ir::{InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::FunctionBuilder;

/// The outputs of a program made from the bytes.
pub struct Bytewise<'a> {
    /// The program before its CPU form (`BlockNative::cpu_form` makes some outputs zero).
    pub native: &'a BlockNative,
    plan: Plan,
    /// Each instruction: made from the bytes alone.
    can: Vec<bool>,
    /// The functions of `escaped(a)`'s carry in past the byte before (`carry_fn`): each
    /// `(a, id, signature)`. Where `a` has none, its loop is inline (in that function itself).
    pub carries: Vec<(usize, cranelift_module::FuncId, cranelift_codegen::ir::Signature)>,
}

/// Where the reads are: the file's first byte and length, and the body's constants.
#[derive(Clone, Copy)]
pub struct File {
    pub bytes: Value,
    pub len: Value,
    pub nibble: Value,
    pub weights: Value,
    pub even: Value,
}

impl<'a> Bytewise<'a> {
    pub fn of(native: &'a BlockNative) -> Self {
        let plan = Plan::window(native);
        let mut can = vec![false; native.insts.len()];
        for (x, inst) in native.insts.iter().enumerate() {
            let o = |a: u16| can[usize::from(a)];
            can[x] = plan.lanes[x]
                || match *inst {
                    BlockInst::Const(_) => true,
                    BlockInst::Escaped(a) | BlockInst::Not(a) | BlockInst::Prev(a, _) => o(a),
                    BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => o(a) && o(b),
                    _ => false,
                };
        }
        Bytewise { native, plan, can, carries: Vec::new() }
    }

    /// The runs whose `escaped` the bytes make: each `a`, once.
    pub fn escaped_runs(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.native.insts.iter().enumerate().filter_map(|(x, i)| match *i {
            BlockInst::Escaped(a) if self.can[x] => Some(usize::from(a)),
            _ => None,
        }).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// `escaped(a)`'s carry in at `pos` where `a` holds at `pos - 1`: a function of its own,
    /// `(bytes, len, pos)` to the parity of `a`'s run that ends at `pos - 1`. Cold: a run of `a`
    /// at a window's first byte.
    pub fn carry_fn(&self, f: &mut FunctionBuilder<'_>, at: &File, a: usize, pos: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let zero = f.ins().iconst(types::I64, 0);
        self.carry_loop(f, at, a, pos, zero, heads)
    }

    /// Output `o`'s window (`word`) as a function of its own, user name `u0:id`: `(bytes, len,
    /// p)` to its bits for the file's bytes `p .. p + 64`. For a backend that is given functions
    /// (`clif_rust`): tier 2's ahead-of-time source reads a mask so.
    pub fn window_function(&self, o: u16, id: u32) -> cranelift_codegen::ir::Function {
        use cranelift_codegen::ir::{AbiParam, Signature, UserFuncName};
        let mut sig = Signature::new(cranelift_codegen::isa::CallConv::SystemV);
        sig.params.extend([AbiParam::new(types::I64); 3]);
        sig.returns.push(AbiParam::new(types::I64));
        let mut func = cranelift_codegen::ir::Function::with_name_signature(UserFuncName::user(0, id), sig);
        let mut ctx = cranelift_frontend::FunctionBuilderContext::new();
        let mut f = FunctionBuilder::new(&mut func, &mut ctx);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        f.switch_to_block(entry);
        let p = f.block_params(entry).to_vec();
        let k = crate::native_cpu_block::Consts::make(&mut f);
        let file = File { bytes: p[0], len: p[1], nibble: k.nibble, weights: k.weights, even: k.even };
        let mut heads = Vec::new();
        let w = self.word(&mut f, &file, o, p[2], &mut heads);
        f.ins().return_(&[w]);
        f.seal_all_blocks();
        f.finalize();
        func
    }

    /// Whether output `o` is made from the bytes.
    pub fn can(&self, o: u16) -> bool {
        self.native.outputs.get(usize::from(o)).is_some_and(|&v| self.can[usize::from(v)])
    }

    /// Output `o`'s bits for the file's bytes `p .. p + 64` (`p` at least 0), none past its end.
    pub fn word(&self, f: &mut FunctionBuilder<'_>, at: &File, o: u16, p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let x = usize::from(self.native.outputs[usize::from(o)]);
        let d = self.native.output_delays[usize::from(o)];
        let start = f.ins().iadd_imm(p, i64::from(d));
        let m = self.mask(f, at, x, start, heads);
        // The bits of the file's bytes: `len - p` of them, at most 64.
        let rem = f.ins().isub(at.len, p);
        let zero = f.ins().iconst(types::I64, 0);
        let sixty_four = f.ins().iconst(types::I64, 64);
        let rem = f.ins().smax(rem, zero);
        let full = f.ins().icmp(IntCC::SignedGreaterThanOrEqual, rem, sixty_four);
        let one = f.ins().iconst(types::I64, 1);
        let low = f.ins().ishl(one, rem);
        let low = f.ins().iadd_imm(low, -1);
        let ones = f.ins().iconst(types::I64, -1);
        let cut = f.ins().select(full, ones, low);
        f.ins().band(m, cut)
    }

    /// Outputs `os`' bits for the file's bytes `p .. p + 64`, none past its end, as `word`'s
    /// each: one window's vectors and words shared among them, as a lexer classifies a byte
    /// once for every test that reads it.
    pub fn words(&self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Vec<Value> {
        let mut windows: HashMap<i64, Lanes<'_>> = HashMap::new();
        let mut memo = HashMap::new();
        let rem = f.ins().isub(at.len, p);
        let zero = f.ins().iconst(types::I64, 0);
        let sixty_four = f.ins().iconst(types::I64, 64);
        let rem = f.ins().smax(rem, zero);
        let full = f.ins().icmp(IntCC::SignedGreaterThanOrEqual, rem, sixty_four);
        let one = f.ins().iconst(types::I64, 1);
        let low = f.ins().ishl(one, rem);
        let low = f.ins().iadd_imm(low, -1);
        let ones = f.ins().iconst(types::I64, -1);
        let cut = f.ins().select(full, ones, low);
        os.iter()
            .map(|&o| {
                let x = usize::from(self.native.outputs[usize::from(o)]);
                // A delay `d` reads from `p + d`: `d` bytes ahead of the shared start.
                let d = i64::from(self.native.output_delays[usize::from(o)]);
                let m = self.mask_at(f, at, x, p, -d, &mut windows, &mut memo, heads);
                f.ins().band(m, cut)
            })
            .collect()
    }

    /// Output `o` at the file's byte `p`, 0 or 1 (0 past its end).
    pub fn bit(&self, f: &mut FunctionBuilder<'_>, at: &File, o: u16, p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let x = usize::from(self.native.outputs[usize::from(o)]);
        let d = self.native.output_delays[usize::from(o)];
        let pos = f.ins().iadd_imm(p, i64::from(d));
        let mut memo = HashMap::new();
        let b = self.scalar(f, at, x, pos, 0, &mut memo, heads);
        let inside = f.ins().icmp(IntCC::SignedLessThan, p, at.len);
        let inside = f.ins().uextend(types::I64, inside);
        f.ins().band(b, inside)
    }

    /// Instruction `x`'s word for stream positions `start .. start + 64`.
    fn mask(&self, f: &mut FunctionBuilder<'_>, at: &File, x: usize, start: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let mut windows: HashMap<i64, Lanes<'_>> = HashMap::new();
        let mut memo = HashMap::new();
        self.mask_at(f, at, x, start, 0, &mut windows, &mut memo, heads)
    }

    /// `x`'s word from `start - back`; the windows' vectors and the words shared.
    #[allow(clippy::too_many_arguments)]
    fn mask_at<'s>(&'s self, f: &mut FunctionBuilder<'_>, at: &File, x: usize, start: Value, back: i64, windows: &mut HashMap<i64, Lanes<'s>>, memo: &mut HashMap<(usize, i64), Value>, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        if let Some(&v) = memo.get(&(x, back)) {
            return v;
        }
        let here = |f: &mut FunctionBuilder<'_>| if back == 0 { start } else { f.ins().iadd_imm(start, -back) };
        let v = if self.plan.lanes[x] {
            let pos = here(f);
            let lanes = windows.entry(back).or_insert_with(|| {
                let base = f.ins().iadd(at.bytes, pos);
                Lanes::window(f, self.native, &self.plan, base, pos, at.len, at.nibble)
            });
            let parts: Vec<_> = (1..=4).map(|c| lanes.get(f, x, c)).collect();
            match lanes.mask(f, &parts, at.weights) {
                Sym::Known(k) => f.ins().iconst(types::I64, k as i64),
                Sym::Value(v, false) => v,
                Sym::Value(v, true) => f.ins().bnot(v),
                Sym::Slot(..) => unreachable!("a window's mask is a value"),
            }
        } else {
            match self.native.insts[x] {
                BlockInst::Const(c) => {
                    // Bit `i % 64` of the pattern at position `i`.
                    let pos = here(f);
                    let k = f.ins().iconst(types::I64, c as i64);
                    let sh = f.ins().band_imm(pos, 63);
                    f.ins().rotr(k, sh)
                }
                BlockInst::Not(a) => {
                    let a = self.mask_at(f, at, usize::from(a), start, back, windows, memo, heads);
                    f.ins().bnot(a)
                }
                BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => {
                    let p = self.mask_at(f, at, usize::from(a), start, back, windows, memo, heads);
                    let q = self.mask_at(f, at, usize::from(b), start, back, windows, memo, heads);
                    match self.native.insts[x] {
                        BlockInst::And(..) => f.ins().band(p, q),
                        BlockInst::Or(..) => f.ins().bor(p, q),
                        BlockInst::Xor(..) => f.ins().bxor(p, q),
                        _ => f.ins().band_not(p, q),
                    }
                }
                BlockInst::Prev(a, k) => {
                    // `a` `k` further back; no bits before the stream.
                    let m = self.mask_at(f, at, usize::from(a), start, back + i64::from(k), windows, memo, heads);
                    let pos = here(f);
                    let need = f.ins().irsub_imm(pos, i64::from(k));
                    let zero = f.ins().iconst(types::I64, 0);
                    let need = f.ins().smax(need, zero);
                    let ones = f.ins().iconst(types::I64, -1);
                    let cut = f.ins().ishl(ones, need);
                    let past = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, need, 64);
                    let cut = f.ins().select(past, zero, cut);
                    f.ins().band(m, cut)
                }
                BlockInst::Escaped(a) => {
                    // simdjson's find_escaped_branchless, its carry the run before the window.
                    let pos = here(f);
                    let cin = self.carry_in(f, at, usize::from(a), pos, heads);
                    // The control flow of the carry: the windows made before it are still made.
                    let run = self.mask_at(f, at, usize::from(a), start, back, windows, memo, heads);
                    let b = f.ins().band_not(run, cin);
                    let up = f.ins().ishl_imm(b, 1);
                    let follows = f.ins().bor(up, cin);
                    let odd = f.ins().band_not(b, at.even);
                    let odd_starts = f.ins().band_not(odd, follows);
                    let sequences = f.ins().iadd(odd_starts, b);
                    let invert = f.ins().ishl_imm(sequences, 1);
                    let flipped = f.ins().bxor(at.even, invert);
                    f.ins().band(flipped, follows)
                }
                _ => unreachable!("not made from the bytes"),
            }
        };
        memo.insert((x, back), v);
        v
    }

    /// Whether `a`'s run that ends at `pos - 1` is odd, 0 or 1: `escaped(a)` at `pos`. Where
    /// `a` does not hold at `pos - 1`, none; else a window at a time back to the run's start.
    fn carry_in(&self, f: &mut FunctionBuilder<'_>, at: &File, a: usize, pos: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let (slow, join) = (f.create_block(), f.create_block());
        f.append_block_param(join, types::I64);
        let before = f.ins().iadd_imm(pos, -1);
        let mut memo = HashMap::new();
        let first = self.scalar(f, at, a, before, 0, &mut memo, heads);
        let zero = f.ins().iconst(types::I64, 0);
        f.ins().brif(first, slow, &[], join, &[zero.into()]);
        f.switch_to_block(slow);
        f.set_cold_block(slow);
        let parity = match self.carries.iter().find(|c| c.0 == a) {
            Some((_, id, sig)) => {
                let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
                let signature = f.import_signature(sig.clone());
                let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
                let call = f.ins().call(callee, &[at.bytes, at.len, pos]);
                f.inst_results(call)[0]
            }
            None => self.carry_loop(f, at, a, pos, zero, heads),
        };
        f.ins().jump(join, &[parity.into()]);
        f.switch_to_block(join);
        f.block_params(join)[0]
    }

    /// The parity of `a`'s run that ends at `pos - 1`, a window at a time back to its start.
    fn carry_loop(&self, f: &mut FunctionBuilder<'_>, at: &File, a: usize, pos: Value, zero: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let (head, join) = (f.create_block(), f.create_block());
        f.append_block_param(head, types::I64);
        f.append_block_param(head, types::I64);
        f.append_block_param(join, types::I64);
        f.ins().jump(head, &[pos.into(), zero.into()]);
        // The run's bits from `e - 1` down, in the window that ends there.
        f.switch_to_block(head);
        f.set_cold_block(head);
        heads.push((head, Step::Word));
        let (e, parity) = (f.block_params(head)[0], f.block_params(head)[1]);
        let b = f.ins().iadd_imm(e, -64);
        let b = f.ins().smax(b, zero);
        let n = f.ins().isub(e, b);
        let m = self.mask(f, at, a, b, heads);
        let up = f.ins().irsub_imm(n, 64);
        let m = f.ins().ishl(m, up);
        let inv = f.ins().bnot(m);
        let ones = f.ins().clz(inv);
        let ones = f.ins().umin(ones, n);
        let odd = f.ins().band_imm(ones, 1);
        let parity = f.ins().bxor(parity, odd);
        let whole = f.ins().icmp(IntCC::Equal, ones, n);
        let more = f.ins().icmp_imm(IntCC::SignedGreaterThan, b, 0);
        let on = f.ins().band(whole, more);
        let (go, done) = (f.create_block(), f.create_block());
        f.ins().brif(on, go, &[], done, &[]);
        f.switch_to_block(go);
        f.set_cold_block(go);
        f.ins().jump(head, &[b.into(), parity.into()]);
        f.switch_to_block(done);
        f.set_cold_block(done);
        f.ins().jump(join, &[parity.into()]);
        f.switch_to_block(join);
        f.block_params(join)[0]
    }

    /// Instruction `x` at stream position `pos - back`, 0 or 1, by scalar loads.
    #[allow(clippy::too_many_arguments)]
    fn scalar(&self, f: &mut FunctionBuilder<'_>, at: &File, x: usize, pos: Value, back: i64, memo: &mut HashMap<(usize, i64), Value>, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        if let Some(&v) = memo.get(&(x, back)) {
            return v;
        }
        let i = if back == 0 { pos } else { f.ins().iadd_imm(pos, -back) };
        let tables = &self.native.tables;
        let set_of = |table: u16, bits: u8| -> [u64; 4] {
            let t = &tables[usize::from(table)];
            let mut s = [0u64; 4];
            for b in 0..256usize {
                if t.member(b as u8, bits) {
                    s[b / 64] |= 1 << (b % 64);
                }
            }
            s
        };
        let v = match self.native.insts[x] {
            BlockInst::EqByte(c) => {
                let mut s = [0u64; 4];
                s[usize::from(c) / 64] |= 1 << (c % 64);
                let b = byte_at(f, at, i);
                in_set(f, b, s)
            }
            BlockInst::Class { table, bits } => {
                let b = byte_at(f, at, i);
                in_set(f, b, set_of(table, bits))
            }
            BlockInst::Pair { before, table, bits } => {
                // A shared class of `bits`: each bit's two sets.
                let (b, p) = (byte_at(f, at, i), {
                    let j = f.ins().iadd_imm(i, -1);
                    byte_at(f, at, j)
                });
                let mut acc = f.ins().iconst(types::I64, 0);
                for k in (0..8).filter(|k| bits >> k & 1 == 1) {
                    let x = in_set(f, p, set_of(before, 1 << k));
                    let y = in_set(f, b, set_of(table, 1 << k));
                    let both = f.ins().band(x, y);
                    acc = f.ins().bor(acc, both);
                }
                acc
            }
            BlockInst::Plane(k) => {
                let b = byte_at(f, at, i);
                let b = f.ins().ushr_imm(b, i64::from(k));
                f.ins().band_imm(b, 1)
            }
            BlockInst::PlaneBefore(k) => {
                let j = f.ins().iadd_imm(i, -1);
                let b = byte_at(f, at, j);
                let b = f.ins().ushr_imm(b, i64::from(k));
                f.ins().band_imm(b, 1)
            }
            BlockInst::Const(c) => {
                let sh = f.ins().band_imm(i, 63);
                let k = f.ins().iconst(types::I64, c as i64);
                let bit = f.ins().ushr(k, sh);
                let bit = f.ins().band_imm(bit, 1);
                let ok = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, i, 0);
                let ok = f.ins().uextend(types::I64, ok);
                f.ins().band(bit, ok)
            }
            BlockInst::Within(d) => {
                let lo = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, i, i64::from(d));
                let end = f.ins().iadd_imm(at.len, i64::from(d));
                let hi = f.ins().icmp(IntCC::SignedLessThan, i, end);
                let both = f.ins().band(lo, hi);
                f.ins().uextend(types::I64, both)
            }
            BlockInst::Not(a) => {
                let a = self.scalar(f, at, usize::from(a), pos, back, memo, heads);
                f.ins().bxor_imm(a, 1)
            }
            BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => {
                let p = self.scalar(f, at, usize::from(a), pos, back, memo, heads);
                let q = self.scalar(f, at, usize::from(b), pos, back, memo, heads);
                match self.native.insts[x] {
                    BlockInst::And(..) => f.ins().band(p, q),
                    BlockInst::Or(..) => f.ins().bor(p, q),
                    BlockInst::Xor(..) => f.ins().bxor(p, q),
                    _ => {
                        let nq = f.ins().bxor_imm(q, 1);
                        f.ins().band(p, nq)
                    }
                }
            }
            BlockInst::Prev(a, k) => {
                let v = self.scalar(f, at, usize::from(a), pos, back + i64::from(k), memo, heads);
                let ok = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, i, i64::from(k));
                let ok = f.ins().uextend(types::I64, ok);
                f.ins().band(v, ok)
            }
            BlockInst::Escaped(a) => self.carry_in(f, at, usize::from(a), i, heads),
            _ => unreachable!("not made from the bytes"),
        };
        memo.insert((x, back), v);
        v
    }
}

/// The file's byte at `i`, 0 outside it. `i` is at least `-FRONT`: the load is inside the
/// zeros before the file (`native_cpu_block::Batch`).
fn byte_at(f: &mut FunctionBuilder<'_>, at: &File, i: Value) -> Value {
    let inside = f.ins().icmp(IntCC::UnsignedLessThan, i, at.len);
    let addr = f.ins().iadd(at.bytes, i);
    let b = f.ins().uload8(types::I64, MemFlags::trusted().with_readonly(), addr, 0);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().select(inside, b, zero)
}

/// Whether byte `b` is in the 256-bit set, 0 or 1.
fn in_set(f: &mut FunctionBuilder<'_>, b: Value, set: [u64; 4]) -> Value {
    let zero = f.ins().iconst(types::I64, 0);
    let used: Vec<usize> = (0..4).filter(|&w| set[w] != 0).collect();
    let word = match used.as_slice() {
        [] => zero,
        [w] => {
            let hi = f.ins().ushr_imm(b, 6);
            let same = f.ins().icmp_imm(IntCC::Equal, hi, *w as i64);
            let x = f.ins().iconst(types::I64, set[*w] as i64);
            f.ins().select(same, x, zero)
        }
        _ => {
            let hi = f.ins().ushr_imm(b, 6);
            let mut x = f.ins().iconst(types::I64, set[0] as i64);
            for (w, &s) in set.iter().enumerate().skip(1) {
                let same = f.ins().icmp_imm(IntCC::Equal, hi, w as i64);
                let y = f.ins().iconst(types::I64, s as i64);
                x = f.ins().select(same, y, x);
            }
            x
        }
    };
    let bit = f.ins().ushr(word, b);
    f.ins().band_imm(bit, 1)
}

/// What an output needs of the bytes (`needs`): a bit at `p` only where some byte from
/// `p + lo` to `p + hi` is in `set` (256 bits). An over-approximation, from the program's
/// instructions: a test that can hold only so is skipped where the bytes say it cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Needs {
    pub set: [u64; 4],
    pub lo: i32,
    pub hi: i32,
}

impl Needs {
    fn bytes(f: impl Fn(u8) -> bool, at: i32) -> Needs {
        let mut set = [0u64; 4];
        for b in 0..=255u8 {
            if f(b) {
                set[usize::from(b >> 6)] |= 1 << (b & 63);
            }
        }
        Needs { set, lo: at, hi: at }
    }

    fn shift(self, k: i32) -> Needs {
        Needs { lo: self.lo + k, hi: self.hi + k, ..self }
    }

    /// Its cost as a filter: the bytes it lets through times the places it looks at.
    fn width(&self) -> u32 {
        self.set.iter().map(|w| w.count_ones()).sum::<u32>() * (self.hi - self.lo + 1) as u32
    }

    /// Whether byte `b` is in the set.
    pub fn has(&self, b: u8) -> bool {
        self.set[usize::from(b >> 6)] >> (b & 63) & 1 == 1
    }

    /// The union of two needs: a bit of either's.
    pub fn union(self, o: Needs) -> Needs {
        let mut set = self.set;
        for (s, w) in set.iter_mut().zip(o.set) {
            *s |= w;
        }
        Needs { set, lo: self.lo.min(o.lo), hi: self.hi.max(o.hi) }
    }

    /// The set as runs of bytes, `(first, last)` each, in order.
    pub fn ranges(&self) -> Vec<(u8, u8)> {
        let mut v: Vec<(u8, u8)> = Vec::new();
        for b in 0..=255u8 {
            if !self.has(b) {
                continue;
            }
            match v.last_mut() {
                Some(r) if r.1.checked_add(1) == Some(b) => r.1 = b,
                _ => v.push((b, b)),
            }
        }
        v
    }
}

/// What output `o` needs of the bytes (`Needs`), where its instructions say: byte tests, their
/// logic, `prev`s and `escaped` (a bit only after a byte of its run). None where it can hold
/// whatever the bytes near it (a negation, a carried value, a position).
pub fn needs(native: &BlockNative, o: u16) -> Option<Needs> {
    let x = usize::from(*native.outputs.get(usize::from(o))?);
    let d = i32::from(native.output_delays[usize::from(o)]);
    let mut memo = HashMap::new();
    // The output at `p` is its instruction at `p + d` (`word`'s `start`).
    needs_at(native, x, &mut memo).map(|n| n.shift(d))
}

fn needs_at(native: &BlockNative, x: usize, memo: &mut HashMap<usize, Option<Needs>>) -> Option<Needs> {
    if let Some(&n) = memo.get(&x) {
        return n;
    }
    let t = |table: u16, bits: u8| {
        let c = native.tables[usize::from(table)];
        move |b: u8| c.member(b, bits)
    };
    let n = match native.insts[x] {
        BlockInst::EqByte(c) => Some(Needs::bytes(|b| b == c, 0)),
        BlockInst::Class { table, bits } => Some(Needs::bytes(t(table, bits), 0)),
        BlockInst::Plane(k) => Some(Needs::bytes(|b| b >> k & 1 == 1, 0)),
        BlockInst::PlaneBefore(k) => Some(Needs::bytes(|b| b >> k & 1 == 1, -1)),
        BlockInst::Pair { before, table, bits } => {
            let (a, b) = (Needs::bytes(t(table, bits), 0), Needs::bytes(t(before, bits), -1));
            Some(if b.width() < a.width() { b } else { a })
        }
        BlockInst::Const(0) => Some(Needs { set: [0; 4], lo: 0, hi: 0 }),
        BlockInst::And(a, b) => match (needs_at(native, usize::from(a), memo), needs_at(native, usize::from(b), memo)) {
            (Some(p), Some(q)) => Some(if q.width() < p.width() { q } else { p }),
            (p, q) => p.or(q),
        },
        BlockInst::AndNot(a, _) => needs_at(native, usize::from(a), memo),
        BlockInst::Or(a, b) => match (needs_at(native, usize::from(a), memo), needs_at(native, usize::from(b), memo)) {
            (Some(p), Some(q)) => {
                let mut set = p.set;
                for (s, w) in set.iter_mut().zip(q.set) {
                    *s |= w;
                }
                Some(Needs { set, lo: p.lo.min(q.lo), hi: p.hi.max(q.hi) })
            }
            _ => None,
        },
        BlockInst::Prev(a, k) => needs_at(native, usize::from(a), memo).map(|n| n.shift(-i32::from(k))),
        BlockInst::Escaped(a) => needs_at(native, usize::from(a), memo).map(|n| n.shift(-1)),
        _ => None,
    };
    memo.insert(x, n);
    n
}
