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
#[derive(Clone)]
pub struct Bytewise<'a> {
    /// The program before its CPU form (`BlockNative::cpu_form` makes some outputs zero).
    pub native: &'a BlockNative,
    plan: Plan,
    /// Each instruction: made from the bytes alone.
    can: Vec<bool>,
    /// The functions of `escaped(a)`'s carry in past the byte before (`carry_fn`): each
    /// `(a, id, signature)`. Where `a` has none, its loop is inline (in that function itself).
    pub carries: Vec<(usize, cranelift_module::FuncId, cranelift_codegen::ir::Signature)>,
    /// In a short function (`native_cpu_block::RuleMode::Short`): its whole function, whose
    /// call, with the function's own parameters, a rare carry's way returns instead of calling
    /// the carry's function, so the short one calls nothing on its way.
    pub bail: Option<(cranelift_module::FuncId, cranelift_codegen::ir::Signature)>,
}

/// `escaped`'s carries through a window (`Bytewise::words_carried`): given in, by run, and
/// the carries out made.
/// And whether the window is its first 16 bytes alone (`Bytewise::short_word`).
#[derive(Default)]
struct Carry {
    ins: Vec<(usize, Value)>,
    outs: Vec<(usize, Value)>,
    short: bool,
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
        Bytewise { native, plan, can, carries: Vec::new(), bail: None }
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
        self.words_carried(f, at, os, p, &[], heads).0
    }

    /// `words`, with `escaped(a)`'s carry in at `p` given for each `(a, carry)` of `cins`
    /// where the outputs read it at their window's start (`carried_runs`), not read back from
    /// the bytes; and each one's carry out, at `p + 64` (simdjson's add overflow), in `cins`'
    /// order: a loop of windows 64 bytes apart passes it on, as the lean lexer's reader does.
    pub fn words_carried(&self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], p: Value, cins: &[(usize, Value)], heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> (Vec<Value>, Vec<Value>) {
        let mut carry = Carry { ins: cins.to_vec(), outs: Vec::new(), short: false };
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
        let words = os
            .iter()
            .map(|&o| {
                let x = usize::from(self.native.outputs[usize::from(o)]);
                // A delay `d` reads from `p + d`: `d` bytes ahead of the shared start.
                let d = i64::from(self.native.output_delays[usize::from(o)]);
                let m = self.mask_at(f, at, x, p, -d, &mut windows, &mut memo, &mut carry, heads);
                f.ins().band(m, cut)
            })
            .collect();
        let outs = cins.iter().map(|&(a, _)| carry.outs.iter().find(|o| o.0 == a).expect("a carried run's carry out (`carried_runs`)").1).collect();
        (words, outs)
    }

    /// Output `o`'s bits for the file's bytes `p .. p + 16` (`p` at least 0), none past its end:
    /// `word`'s first 16, from one vector of the bytes, as a lexer reads a short token's bytes
    /// and no more.
    pub fn short_word(&self, f: &mut FunctionBuilder<'_>, at: &File, o: u16, p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        self.short_words(f, at, &[o], p, heads)[0]
    }

    /// `short_word` for each of outputs `os`, from one window's vectors shared among them, as
    /// a lexer reads a number's digits and asks of its `.` in one pass.
    pub fn short_words(&self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Vec<Value> {
        let mut carry = Carry { short: true, ..Carry::default() };
        let mut windows: HashMap<i64, Lanes<'_>> = HashMap::new();
        let mut memo = HashMap::new();
        let Some((&first, rest)) = os.split_first() else { return Vec::new() };
        let mut got = vec![self.short_in(f, at, first, p, &mut windows, &mut memo, &mut carry, heads)];
        // The first output's window (its delay's): an output of one lane's instruction read in
        // it, `lag` bytes back, its loads and compares the first's.
        let d0 = i64::from(self.native.output_delays[usize::from(first)]);
        for &o in rest {
            let x = usize::from(self.native.outputs[usize::from(o)]);
            let d = i64::from(self.native.output_delays[usize::from(o)]);
            let lag = d0 - d;
            let shared = self.plan.lanes[x] && (0..=8).contains(&lag) && windows.contains_key(&-d0);
            let m = if shared {
                let lanes = windows.get_mut(&-d0).expect("the first output's window");
                let part = lanes.get_lagged(f, x, 1, lag as u8);
                match lanes.mask(f, &[part], at.weights) {
                    Sym::Known(k) => f.ins().iconst(types::I64, k as i64),
                    Sym::Value(v, false) => v,
                    Sym::Value(v, true) => f.ins().bnot(v),
                    Sym::Slot(..) => unreachable!("a window's mask is a value"),
                }
            } else {
                self.short_in(f, at, o, p, &mut windows, &mut memo, &mut carry, heads)
            };
            got.push(if shared { self.short_cut(f, at, x, d, p, m) } else { m });
        }
        got
    }

    /// `short_in`'s cut of mask `m` (output instruction `x`, delay `d`, from `p`): none past the
    /// file's end. Where the output is false on a zero byte (`zero_kills`), none past it
    /// already: the bytes there are zeros, more than the 16 and its delay
    /// (`native_cpu_block::Batch`). Else the file's bytes' bits: `len - p`, at most 16.
    fn short_cut(&self, f: &mut FunctionBuilder<'_>, at: &File, x: usize, d: i64, p: Value, m: Value) -> Value {
        if crate::native_cpu_lanes::zero_kills(self.native, x, d as u32) {
            return m;
        }
        let rem = f.ins().isub(at.len, p);
        let zero = f.ins().iconst(types::I64, 0);
        let sixteen = f.ins().iconst(types::I64, 16);
        let rem = f.ins().smax(rem, zero);
        let rem = f.ins().smin(rem, sixteen);
        let one = f.ins().iconst(types::I64, 1);
        let low = f.ins().ishl(one, rem);
        let low = f.ins().iadd_imm(low, -1);
        f.ins().band(m, low)
    }

    #[allow(clippy::too_many_arguments)]
    fn short_in<'s>(&'s self, f: &mut FunctionBuilder<'_>, at: &File, o: u16, p: Value, windows: &mut HashMap<i64, Lanes<'s>>, memo: &mut HashMap<(usize, i64), Value>, carry: &mut Carry, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let x = usize::from(self.native.outputs[usize::from(o)]);
        let d = i64::from(self.native.output_delays[usize::from(o)]);
        let m = self.mask_at(f, at, x, p, -d, windows, memo, carry, heads);
        self.short_cut(f, at, x, d, p, m)
    }

    /// The runs whose `escaped` outputs `os` read at their window's start, no `prev` or delay
    /// before it: the carries `words_carried` takes and gives.
    pub fn carried_runs(&self, os: &[u16]) -> Vec<usize> {
        fn walk(b: &Bytewise<'_>, x: usize, back: i64, seen: &mut std::collections::HashSet<(usize, i64)>, runs: &mut Vec<usize>) {
            if !seen.insert((x, back)) || b.plan.lanes[x] {
                return;
            }
            match b.native.insts[x] {
                BlockInst::Not(a) => walk(b, usize::from(a), back, seen, runs),
                BlockInst::And(a, c) | BlockInst::Or(a, c) | BlockInst::Xor(a, c) | BlockInst::AndNot(a, c) => {
                    walk(b, usize::from(a), back, seen, runs);
                    walk(b, usize::from(c), back, seen, runs);
                }
                BlockInst::Prev(a, k) => walk(b, usize::from(a), back + i64::from(k), seen, runs),
                BlockInst::Escaped(a) => {
                    if back == 0 && !runs.contains(&usize::from(a)) {
                        runs.push(usize::from(a));
                    }
                    walk(b, usize::from(a), back, seen, runs);
                }
                _ => {}
            }
        }
        let (mut seen, mut runs) = (std::collections::HashSet::new(), Vec::new());
        for &o in os {
            let x = usize::from(self.native.outputs[usize::from(o)]);
            walk(self, x, -i64::from(self.native.output_delays[usize::from(o)]), &mut seen, &mut runs);
        }
        runs
    }

    /// `escaped(a)`'s carry in at `pos` (the parity of `a`'s run that ends at `pos - 1`), from
    /// the bytes: none where `a` does not hold at `pos - 1`.
    pub fn carry_at(&self, f: &mut FunctionBuilder<'_>, at: &File, a: usize, pos: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        self.carry_in(f, at, a, pos, heads)
    }

    /// Whether `a` never holds where output `o` does: the bytes `o` needs at its position, and
    /// those `a` needs at its own, are apart (`needs`). A reader's opener so starts no run of
    /// its escapes: the carry after it is none, as the lean lexer's reader starts with none.
    pub fn apart(&self, o: u16, a: usize) -> bool {
        let n = needs(self.native, o);
        let r = needs_at(self.native, a, &mut HashMap::new());
        match (n, r) {
            (Some(n), Some(r)) => n.lo == 0 && n.hi == 0 && r.lo == 0 && r.hi == 0 && n.set.iter().zip(r.set).all(|(x, y)| x & y == 0),
            _ => false,
        }
    }

    /// Output `o` at the file's byte `p`, 0 or 1 (0 past its end).
    pub fn bit(&self, f: &mut FunctionBuilder<'_>, at: &File, o: u16, p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        self.bit_first(f, at, o, p, None, heads)
    }

    /// `bit`, where the byte at `p` is known to be in `first` (a token's first byte, by the
    /// rule its dispatch chose): its tests there constants, as the lean lexer's `b'/'` arm asks
    /// only of the byte after.
    pub fn bit_first(&self, f: &mut FunctionBuilder<'_>, at: &File, o: u16, p: Value, first: Option<[u64; 4]>, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let x = usize::from(self.native.outputs[usize::from(o)]);
        let d = self.native.output_delays[usize::from(o)];
        let pos = f.ins().iadd_imm(p, i64::from(d));
        let mut memo = HashMap::new();
        let b = self.scalar(f, at, x, pos, 0, Some(i64::from(d)), first.map(|set| (i64::from(d), set)), &mut memo, heads);
        // Past the file's end, none: the output already says so where it is false on the zero
        // byte at `p` (`zero_kills`), the bytes after the file being zeros.
        if crate::native_cpu_lanes::zero_kills(self.native, x, u32::from(d)) {
            return b;
        }
        let inside = f.ins().icmp(IntCC::SignedLessThan, p, at.len);
        let inside = f.ins().uextend(types::I64, inside);
        f.ins().band(b, inside)
    }

    /// Instruction `x`'s word for stream positions `start .. start + 64`.
    fn mask(&self, f: &mut FunctionBuilder<'_>, at: &File, x: usize, start: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let mut windows: HashMap<i64, Lanes<'_>> = HashMap::new();
        let mut memo = HashMap::new();
        self.mask_at(f, at, x, start, 0, &mut windows, &mut memo, &mut Carry::default(), heads)
    }

    /// `x`'s word from `start - back`; the windows' vectors and the words shared.
    #[allow(clippy::too_many_arguments)]
    fn mask_at<'s>(&'s self, f: &mut FunctionBuilder<'_>, at: &File, x: usize, start: Value, back: i64, windows: &mut HashMap<i64, Lanes<'s>>, memo: &mut HashMap<(usize, i64), Value>, carry: &mut Carry, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
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
            let parts: Vec<_> = (1..=if carry.short { 1 } else { 4 }).map(|c| lanes.get(f, x, c)).collect();
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
                    let a = self.mask_at(f, at, usize::from(a), start, back, windows, memo, carry, heads);
                    f.ins().bnot(a)
                }
                BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => {
                    let p = self.mask_at(f, at, usize::from(a), start, back, windows, memo, carry, heads);
                    let q = self.mask_at(f, at, usize::from(b), start, back, windows, memo, carry, heads);
                    match self.native.insts[x] {
                        BlockInst::And(..) => f.ins().band(p, q),
                        BlockInst::Or(..) => f.ins().bor(p, q),
                        BlockInst::Xor(..) => f.ins().bxor(p, q),
                        _ => f.ins().band_not(p, q),
                    }
                }
                BlockInst::Prev(a, k) => {
                    // `a` `k` further back; no bits before the stream.
                    let m = self.mask_at(f, at, usize::from(a), start, back + i64::from(k), windows, memo, carry, heads);
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
                    let given = if back == 0 { carry.ins.iter().find(|c| c.0 == usize::from(a)).map(|c| c.1) } else { None };
                    let cin = match given {
                        Some(c) => c,
                        None => self.carry_in(f, at, usize::from(a), pos, heads),
                    };
                    // The control flow of the carry: the windows made before it are still made.
                    let run = self.mask_at(f, at, usize::from(a), start, back, windows, memo, carry, heads);
                    let b = f.ins().band_not(run, cin);
                    let up = f.ins().ishl_imm(b, 1);
                    let follows = f.ins().bor(up, cin);
                    let odd = f.ins().band_not(b, at.even);
                    let odd_starts = f.ins().band_not(odd, follows);
                    let sequences = f.ins().iadd(odd_starts, b);
                    if given.is_some() {
                        // The carry out: the add's overflow, a run odd at the window's end.
                        let over = f.ins().icmp(IntCC::UnsignedLessThan, sequences, b);
                        let over = f.ins().uextend(types::I64, over);
                        carry.outs.push((usize::from(a), over));
                    }
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
        let first = self.scalar(f, at, a, before, 0, None, None, &mut memo, heads);
        let zero = f.ins().iconst(types::I64, 0);
        f.ins().brif(first, slow, &[], join, &[zero.into()]);
        f.switch_to_block(slow);
        f.set_cold_block(slow);
        // A short function's (`bail`): its whole function's call, returned.
        if let Some((id, sig)) = &self.bail {
            let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
            let signature = f.import_signature(sig.clone());
            let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
            let entry = f.func.layout.entry_block().expect("an entry");
            // Its arguments the whole function's first (`native_cpu_block::rule_args`).
            let args = f.func.dfg.block_params(entry).to_vec();
            let call = f.ins().call(callee, &args[..sig.params.len()]);
            let got = f.inst_results(call).to_vec();
            f.ins().return_(&got);
            f.switch_to_block(join);
            return f.block_params(join)[0];
        }
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

    /// Instruction `x` at stream position `pos - back`, 0 or 1, by scalar loads. `anchor`: where
    /// `pos` is `anchor` bytes past a byte `p` of the file or the byte after it, so the bytes
    /// near it lie in the batch's layout (`native_cpu_block::Batch`: `FRONT` zeros before the
    /// file, zeros past its end as far as the program looks ahead). There a test of a position
    /// leaves out what those zeros already say: a `prev`'s bound where its operand is false on
    /// a zero byte, a `within` another conjunct is false on (`native_cpu_lanes::unguarded`).
    #[allow(clippy::too_many_arguments)]
    fn scalar(&self, f: &mut FunctionBuilder<'_>, at: &File, x: usize, pos: Value, back: i64, anchor: Option<i64>, first: Option<(i64, [u64; 4])>, memo: &mut HashMap<(usize, i64), Value>, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        if let Some(&v) = memo.get(&(x, back)) {
            return v;
        }
        // Known by the token's first byte (`first`): a constant, no byte read.
        if let Some(k) = first.and_then(|fi| self.fold(x, back, anchor, fi)) {
            let v = f.ins().iconst(types::I64, i64::from(k));
            memo.insert((x, back), v);
            return v;
        }
        let known = |y: u16, b: i64| first.and_then(|fi| self.fold(usize::from(y), b, anchor, fi));
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
                let a = self.scalar(f, at, usize::from(a), pos, back, anchor, first, memo, heads);
                f.ins().bxor_imm(a, 1)
            }
            // At or after `p`, the conjuncts but the `within`s the zeros make (`unguarded`).
            BlockInst::And(..) if let Some(kept) = anchor.filter(|&d| back <= d).and_then(|_| crate::native_cpu_lanes::unguarded(self.native, x)) => {
                // The conjuncts the first byte does not decide, from the first of them.
                let mut v: Option<Value> = None;
                for y in kept.into_iter().filter(|&y| first.and_then(|fi| self.fold(y, back, anchor, fi)) != Some(true)) {
                    let b = self.scalar(f, at, y, pos, back, anchor, first, memo, heads);
                    v = Some(match v {
                        Some(v) => f.ins().band(v, b),
                        None => b,
                    });
                }
                v.unwrap_or_else(|| f.ins().iconst(types::I64, 1))
            }
            // One side known by the first byte: the other alone, no operation.
            BlockInst::And(a, b) if known(a, back) == Some(true) || known(b, back) == Some(true) => {
                let y = if known(a, back) == Some(true) { b } else { a };
                self.scalar(f, at, usize::from(y), pos, back, anchor, first, memo, heads)
            }
            BlockInst::Or(a, b) if known(a, back) == Some(false) || known(b, back) == Some(false) => {
                let y = if known(a, back) == Some(false) { b } else { a };
                self.scalar(f, at, usize::from(y), pos, back, anchor, first, memo, heads)
            }
            BlockInst::AndNot(a, b) if known(b, back) == Some(false) => self.scalar(f, at, usize::from(a), pos, back, anchor, first, memo, heads),
            BlockInst::AndNot(a, b) if known(a, back) == Some(true) => {
                let q = self.scalar(f, at, usize::from(b), pos, back, anchor, first, memo, heads);
                f.ins().bxor_imm(q, 1)
            }
            BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => {
                let p = self.scalar(f, at, usize::from(a), pos, back, anchor, first, memo, heads);
                let q = self.scalar(f, at, usize::from(b), pos, back, anchor, first, memo, heads);
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
                let v = self.scalar(f, at, usize::from(a), pos, back + i64::from(k), anchor, first, memo, heads);
                // Its operand read at most `FRONT` bytes before `p`, false on a zero byte: the
                // zeros before the file say its bound.
                if anchor.is_some_and(|d| zeros_bound(self.native, usize::from(a), back + i64::from(k) - d)) {
                    memo.insert((x, back), v);
                    return v;
                }
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

impl Bytewise<'_> {
    /// Instruction `x`'s value at `pos - back` where the byte at `pos - fi.0` is in set `fi.1`
    /// (`scalar`'s `first`): known where the set decides it, else none.
    fn fold(&self, x: usize, back: i64, anchor: Option<i64>, fi: (i64, [u64; 4])) -> Option<bool> {
        let (at, set) = fi;
        let decide = |s: [u64; 4]| -> Option<bool> {
            let inside = (0..4).all(|k| set[k] & !s[k] == 0);
            let outside = (0..4).all(|k| set[k] & s[k] == 0);
            if inside { Some(true) } else if outside { Some(false) } else { None }
        };
        let tables = &self.native.tables;
        let class = |table: u16, bits: u8| -> [u64; 4] {
            let t = &tables[usize::from(table)];
            let mut s = [0u64; 4];
            for b in 0..256usize {
                if t.member(b as u8, bits) {
                    s[b / 64] |= 1 << (b % 64);
                }
            }
            s
        };
        let plane = |k: u8| -> [u64; 4] {
            let mut s = [0u64; 4];
            for b in 0..256usize {
                if b >> k & 1 == 1 {
                    s[b / 64] |= 1 << (b % 64);
                }
            }
            s
        };
        // A union of byte tests at one byte (`exact_at`), that byte the first: by its set whole.
        if let Some(n) = exact_at(self.native, x).filter(|n| n.lo == n.hi && back - i64::from(n.lo) == at) {
            if let Some(v) = decide(n.set) {
                return Some(v);
            }
        }
        match self.native.insts[x] {
            BlockInst::EqByte(c) if back == at => {
                let mut s = [0u64; 4];
                s[usize::from(c) / 64] |= 1 << (c % 64);
                decide(s)
            }
            BlockInst::Class { table, bits } if back == at => decide(class(table, bits)),
            BlockInst::Plane(k) if back == at => decide(plane(k)),
            BlockInst::PlaneBefore(k) if back + 1 == at => decide(plane(k)),
            BlockInst::Not(a) => self.fold(usize::from(a), back, anchor, fi).map(|v| !v),
            BlockInst::And(a, b) => match (self.fold(usize::from(a), back, anchor, fi), self.fold(usize::from(b), back, anchor, fi)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            BlockInst::Or(a, b) => match (self.fold(usize::from(a), back, anchor, fi), self.fold(usize::from(b), back, anchor, fi)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            BlockInst::Xor(a, b) => match (self.fold(usize::from(a), back, anchor, fi), self.fold(usize::from(b), back, anchor, fi)) {
                (Some(p), Some(q)) => Some(p != q),
                _ => None,
            },
            BlockInst::AndNot(a, b) => match (self.fold(usize::from(a), back, anchor, fi), self.fold(usize::from(b), back, anchor, fi)) {
                (Some(false), _) | (_, Some(true)) => Some(false),
                (Some(true), Some(false)) => Some(true),
                _ => None,
            },
            // A `prev`'s bound may make it false: only its false is known, but where the zeros
            // before the file say the bound (`scalar`'s, `zeros_bound`).
            BlockInst::Prev(a, k) => {
                let reach = back + i64::from(k);
                let bounded = anchor.is_some_and(|d| zeros_bound(self.native, usize::from(a), reach - d));
                self.fold(usize::from(a), reach, anchor, fi).filter(|&v| !v || bounded)
            }
            _ => None,
        }
    }
}

/// Whether the zeros before a file of the batch say a `prev`'s bound (`scalar`): its operand
/// `a`, read `reach` bytes before a byte of the file, is false on a zero byte, and `reach` is
/// at most `native_cpu_block::FRONT`, the zeros there.
pub fn zeros_bound(native: &crate::block_native::BlockNative, a: usize, reach: i64) -> bool {
    reach <= crate::native_cpu_block::FRONT as i64 && crate::native_cpu_lanes::zero_kills(native, a, 0)
}

/// The file's byte at `i`, 0 outside it: a load, as the batch's layout has zeros there
/// (`native_cpu_block::Batch`: `FRONT` before the file, past its end as far as the program
/// looks ahead). Further before, a `prev`'s bound says none (`scalar`).
fn byte_at(f: &mut FunctionBuilder<'_>, at: &File, i: Value) -> Value {
    let addr = f.ins().iadd(at.bytes, i);
    f.ins().uload8(types::I64, MemFlags::trusted().with_readonly(), addr, 0)
}

/// Whether byte `b` is in the 256-bit set, 0 or 1.
fn in_set(f: &mut FunctionBuilder<'_>, b: Value, set: [u64; 4]) -> Value {
    // One byte: a compare, as a lexer's `b == c`. (A run of bytes stays the set's word: as a
    // subtract and a compare in the token loop's cold tests, rustc allocated the loop's
    // registers worse, an instruction more a token on every kind.)
    if let Some((lo, _)) = crate::native_cpu_lanes::set_range(set).filter(|(lo, hi)| lo == hi) {
        let same = f.ins().icmp_imm(IntCC::Equal, b, i64::from(lo));
        return f.ins().uextend(types::I64, same);
    }
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

/// Where output `o` is its bytes exactly (`Needs` with `lo == hi`): a bit at `p` where, and only
/// where, the byte at `p + lo` is in `set`. Byte tests and their unions at one offset; a count
/// of it over a token's bytes is then the count of its bytes in the set, one vector's compares.
pub fn exact(native: &BlockNative, o: u16) -> Option<Needs> {
    let x = usize::from(*native.outputs.get(usize::from(o))?);
    let d = i32::from(native.output_delays[usize::from(o)]);
    exact_at(native, x).map(|n| n.shift(d))
}

fn exact_at(native: &BlockNative, x: usize) -> Option<Needs> {
    match native.insts[x] {
        BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) => needs_at(native, x, &mut HashMap::new()),
        BlockInst::Or(a, b) => {
            let (p, q) = (exact_at(native, usize::from(a))?, exact_at(native, usize::from(b))?);
            (p.lo == q.lo).then(|| p.union(q))
        }
        _ => None,
    }
}

/// The bytes the byte at `p + off` can be where output `o`'s bit at `p` is set: a set every
/// such byte is in (a test at that offset; an `And`'s both, an `Or`'s either, a `Prev` or an
/// `escaped` shifted). None where its instructions do not say (any byte).
pub fn output_byte_at(native: &BlockNative, o: u16, off: i32) -> Option<[u64; 4]> {
    let x = usize::from(*native.outputs.get(usize::from(o))?);
    let d = i32::from(native.output_delays[usize::from(o)]);
    // The output at `p` is its instruction at `p + d` (`needs`).
    inst_byte_at(native, x, off - d, 0)
}

/// Whether output `o` never holds where output `p` does, at the same place: each of `o`'s
/// `Or` terms either needs a byte there that `p` never has (`inst_byte_at`), or needs a value
/// (an `And`'s operand, an `AndNot`'s first) that `p` excludes (an `AndNot`'s second). A
/// string's bad-escape test, `escaped(\) & !…`, so never holds at its opener, `" & !escaped(\)`.
pub fn never_both(native: &BlockNative, o: u16, p: u16) -> bool {
    let (Some(&x), Some(&y)) = (native.outputs.get(usize::from(o)), native.outputs.get(usize::from(p))) else { return false };
    let d = native.output_delays[usize::from(o)];
    if d != native.output_delays[usize::from(p)] {
        return false;
    }
    fn needs(native: &BlockNative, x: usize, out: &mut Vec<usize>) {
        out.push(x);
        match native.insts[x] {
            BlockInst::And(a, b) => {
                needs(native, usize::from(a), out);
                needs(native, usize::from(b), out);
            }
            BlockInst::AndNot(a, _) => needs(native, usize::from(a), out),
            _ => {}
        }
    }
    fn excludes(native: &BlockNative, x: usize, out: &mut Vec<usize>) {
        match native.insts[x] {
            BlockInst::And(a, b) => {
                excludes(native, usize::from(a), out);
                excludes(native, usize::from(b), out);
            }
            BlockInst::AndNot(a, b) => {
                excludes(native, usize::from(a), out);
                out.push(usize::from(b));
            }
            _ => {}
        }
    }
    let mut not = Vec::new();
    excludes(native, usize::from(y), &mut not);
    let at = -i32::from(d);
    let py = inst_byte_at(native, usize::from(y), at, 0);
    let mut terms = Vec::new();
    let mut todo = vec![usize::from(x)];
    while let Some(t) = todo.pop() {
        match native.insts[t] {
            BlockInst::Or(a, b) => todo.extend([usize::from(a), usize::from(b)]),
            _ => terms.push(t),
        }
    }
    terms.iter().all(|&t| {
        let bytes = py.zip(inst_byte_at(native, t, at, 0)).is_some_and(|(q, r)| (0..4).all(|k| q[k] & r[k] == 0));
        let mut need = Vec::new();
        needs(native, t, &mut need);
        bytes || need.iter().any(|n| not.contains(n))
    })
}

/// The bytes a run of output `o`'s bits (`EndRule::Run`) can end with: `output_byte_at`'s at
/// the run's bytes, but an `Or`'s term whose byte after is surely one of the run's (a test of
/// that byte alone, another term) ends none, as `dot & next(digit)` in `digit | …`.
pub fn run_last_bytes(native: &BlockNative, o: u16) -> Option<[u64; 4]> {
    let x = usize::from(*native.outputs.get(usize::from(o))?);
    let d = i32::from(native.output_delays[usize::from(o)]);
    let mut terms = Vec::new();
    let mut todo = vec![x];
    while let Some(t) = todo.pop() {
        match native.insts[t] {
            BlockInst::Or(a, b) => todo.extend([usize::from(a), usize::from(b)]),
            _ => terms.push(t),
        }
    }
    // The bytes whose bit is surely set: a term that tests its own byte alone (a byte's test,
    // shifted by `Prev`s to the output's place).
    fn alone(native: &BlockNative, t: usize) -> bool {
        match native.insts[t] {
            BlockInst::EqByte(_) | BlockInst::Class { .. } => true,
            BlockInst::Prev(a, _) => alone(native, usize::from(a)),
            _ => false,
        }
    }
    let mut sure = [0u64; 4];
    for &t in &terms {
        if alone(native, t) {
            if let Some(n) = needs_at(native, t, &mut HashMap::new()).filter(|n| n.lo == -d && n.hi == -d) {
                sure = std::array::from_fn(|k| sure[k] | n.set[k]);
            }
        }
    }
    let mut set = [0u64; 4];
    for &t in &terms {
        let after = inst_byte_at(native, t, 1 - d, 0);
        if after.is_some_and(|a| (0..4).all(|k| a[k] & !sure[k] == 0)) {
            continue;
        }
        let at = inst_byte_at(native, t, -d, 0)?;
        set = std::array::from_fn(|k| set[k] | at[k]);
    }
    Some(set)
}

fn inst_byte_at(native: &BlockNative, x: usize, off: i32, depth: u32) -> Option<[u64; 4]> {
    if depth > 64 {
        return None;
    }
    let and = |p: [u64; 4], q: [u64; 4]| std::array::from_fn(|k| p[k] & q[k]);
    match native.insts[x] {
        // Both of a pair's bytes: the byte at `p` in its table's set, the one before in its
        // before's (`needs` keeps the narrower alone).
        BlockInst::Pair { before, table, bits } => {
            let set = |table: u16| {
                let c = native.tables[usize::from(table)];
                Needs::bytes(|b| c.member(b, bits), 0).set
            };
            match off {
                0 => Some(set(table)),
                -1 => Some(set(before)),
                _ => None,
            }
        }
        BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) | BlockInst::PlaneBefore(_) => {
            let n = needs_at(native, x, &mut HashMap::new())?;
            (n.lo == off && n.hi == off).then_some(n.set)
        }
        BlockInst::And(a, b) => match (inst_byte_at(native, usize::from(a), off, depth + 1), inst_byte_at(native, usize::from(b), off, depth + 1)) {
            (Some(p), Some(q)) => Some(and(p, q)),
            (p, q) => p.or(q),
        },
        BlockInst::AndNot(a, _) => inst_byte_at(native, usize::from(a), off, depth + 1),
        BlockInst::Or(a, b) => {
            let (p, q) = (inst_byte_at(native, usize::from(a), off, depth + 1)?, inst_byte_at(native, usize::from(b), off, depth + 1)?);
            Some(std::array::from_fn(|k| p[k] | q[k]))
        }
        // A bit at `p` is the operand's at `p - k`: its byte at `p + off` is the operand's at
        // `off + k`.
        BlockInst::Prev(a, k) => inst_byte_at(native, usize::from(a), off + i32::from(k), depth + 1),
        BlockInst::Escaped(a) => inst_byte_at(native, usize::from(a), off + 1, depth + 1),
        _ => None,
    }
}

/// What an output needs of the bytes, offset by offset (`needs`'s, unmerged): a bit at `p`
/// only where, for some `(d, set)`, the byte at `p + d` is in `set`. Where `needs` merges two
/// tests at two offsets into one set over both (a control byte or a `\` before, so a newline
/// before a string's quote), this keeps each set at its own offset.
pub fn needs_by_offset(native: &BlockNative, o: u16) -> Option<Vec<(i32, [u64; 4])>> {
    let x = usize::from(*native.outputs.get(usize::from(o))?);
    let d = i32::from(native.output_delays[usize::from(o)]);
    let mut memo = HashMap::new();
    by_offset(native, x, &mut memo).map(|m| m.into_iter().map(|(k, s)| (k + d, s)).collect())
}

type Offsets = std::collections::BTreeMap<i32, [u64; 4]>;

fn by_offset(native: &BlockNative, x: usize, memo: &mut HashMap<usize, Option<Offsets>>) -> Option<Offsets> {
    if let Some(n) = memo.get(&x) {
        return n.clone();
    }
    let one = |n: Needs| -> Offsets { (n.lo..=n.hi).map(|k| (k, n.set)).collect() };
    let width = |m: &Offsets| m.values().map(|s| s.iter().map(|w| w.count_ones()).sum::<u32>()).sum::<u32>();
    let shift = |m: Offsets, k: i32| -> Offsets { m.into_iter().map(|(d, s)| (d + k, s)).collect() };
    let n = match native.insts[x] {
        BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) | BlockInst::PlaneBefore(_) | BlockInst::Pair { .. } | BlockInst::Const(0) => needs_at(native, x, &mut HashMap::new()).map(one),
        BlockInst::And(a, b) => match (by_offset(native, usize::from(a), memo), by_offset(native, usize::from(b), memo)) {
            (Some(p), Some(q)) => Some(if width(&q) < width(&p) { q } else { p }),
            (p, q) => p.or(q),
        },
        BlockInst::AndNot(a, _) => by_offset(native, usize::from(a), memo),
        BlockInst::Or(a, b) => match (by_offset(native, usize::from(a), memo), by_offset(native, usize::from(b), memo)) {
            (Some(mut p), Some(q)) => {
                for (d, s) in q {
                    let e = p.entry(d).or_insert([0; 4]);
                    for (w, v) in e.iter_mut().zip(s) {
                        *w |= v;
                    }
                }
                Some(p)
            }
            _ => None,
        },
        BlockInst::Prev(a, k) => by_offset(native, usize::from(a), memo).map(|m| shift(m, -i32::from(k))),
        BlockInst::Escaped(a) => by_offset(native, usize::from(a), memo).map(|m| shift(m, -1)),
        _ => None,
    };
    memo.insert(x, n.clone());
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(bytes: &[u8]) -> [u64; 4] {
        let mut s = [0u64; 4];
        for &b in bytes {
            s[usize::from(b) / 64] |= 1 << (b % 64);
        }
        s
    }

    /// A token's first byte known to be in a set (`scalar`'s `first`) decides the tests of that
    /// byte where the set is inside or outside theirs, and no other byte's.
    #[test]
    fn a_first_byte_decides_its_tests() {
        let native = BlockNative {
            tables: Vec::new(),
            // 0 `"`, 1 `'`, 2 either, 3 `"` one back, 4 `"` then `"` (`"` at p - 1 and p).
            insts: vec![BlockInst::EqByte(b'"'), BlockInst::EqByte(b'\''), BlockInst::Or(0, 1), BlockInst::Prev(0, 1), BlockInst::And(3, 0), BlockInst::Not(1), BlockInst::AndNot(0, 1)],
            outputs: vec![2],
            extracts: Vec::new(),
            totals: Vec::new(),
            masks: Vec::new(),
            delay: 0,
            output_delays: vec![0],
            groups: Vec::new(),
            records: None,
            walks: Vec::new(),
        };
        let b = Bytewise::of(&native);
        let (quote, both) = ((0, set(b"\"")), (0, set(b"\"'")));
        assert_eq!(b.fold(0, 0, None, quote), Some(true), "`\"` at a `\"`");
        assert_eq!(b.fold(1, 0, None, quote), Some(false), "`'` at a `\"`");
        assert_eq!(b.fold(5, 0, None, quote), Some(true), "not `'` at a `\"`");
        assert_eq!(b.fold(6, 0, None, quote), Some(true), "`\"` and not `'` at a `\"`");
        assert_eq!(b.fold(0, 0, None, both), None, "`\"` at either: not decided");
        assert_eq!(b.fold(2, 0, None, both), Some(true), "either at either");
        assert_eq!(b.fold(0, 1, None, quote), None, "another byte: not decided");
        assert_eq!(b.fold(3, -1, None, quote), None, "`\"` one back at a `\"`: true but for the bound");
        assert_eq!(b.fold(3, 0, None, (1, set(b"'"))), Some(false), "`\"` one back at a `'`: false");
        assert_eq!(b.fold(4, 0, None, (1, set(b"\""))), None, "the byte at p not known");
    }
}
