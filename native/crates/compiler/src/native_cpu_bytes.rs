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
use crate::native_cpu_lanes::{BodyPack, Lanes, Plan};
// The bytes' analysis this lowers (`byte_needs`), under the names the lowering's callers read.
pub use crate::byte_needs::*;
use std::rc::Rc;
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
    /// Each body's classes packed (`native_cpu_lanes::body_pack`), by its roots and extra sets:
    /// made once.
    packs: std::cell::RefCell<HashMap<(Vec<usize>, Vec<crate::class_pack::Need>), Rc<BodyPack>>>,
    /// The bytes that can be right before a token's first byte (`native_cpu_block::
    /// before_start`): each `(next, bytes)`, `bytes` where the token's first byte can be in
    /// `next`. `scalar`'s `first` folds by them what reads the byte before the token, as the
    /// lean lexer never asks whether its `"` is escaped.
    pub before_start: Option<Vec<([u64; 4], [u64; 4])>>,
    /// A split run's byte term's lanes from a token's second byte (`bit_on_run`), while a test
    /// of the token is made: a test of byte `s + j` (`1 <= j <= 16`) of exactly the term's set
    /// is the mask's lane `j - 1`, no byte read, as lean's number arm knows its digits.
    run_lanes: std::cell::Cell<Option<RunLanes>>,
    /// The refill's words the body stores in its rows (decision 0361, `native_cpu_block::
    /// refill_rows`): the outputs, and the row slot of the block's rare starts' word. The
    /// refill loads them where its block is made, as simdjson's stage 2 reads stage 1's index.
    pub rows: Option<(Vec<u16>, Option<i32>)>,
    /// Whether the body stores those words in its rows. Not where it hands every block's to the
    /// token loop (0374): the refill's cold way classifies the block's bytes again.
    pub rows_stored: bool,
}

/// A split run's byte term's mask (`Bytewise::run_split`): its lanes from `s + 1`, `stride`
/// bits apart, of the bytes in `set`; the first `known` of them hold for any token (the
/// window's 16; the byte loop's one, whether it ran a turn).
#[derive(Clone, Copy)]
pub struct RunLanes {
    pub mask: Value,
    pub stride: u32,
    pub set: [u64; 4],
    pub known: i64,
}

/// `escaped`'s carries through a window (`Bytewise::words_carried`): given in, by run, and
/// the carries out made.
/// And whether the window is its first 16 bytes alone (`Bytewise::short_word`).
#[derive(Default)]
struct Carry {
    ins: Vec<(usize, Value)>,
    outs: Vec<(usize, Value)>,
    short: bool,
    /// A short window's bits a lane (`short_stride`): its masks' positions are `stride` apart.
    stride: u32,
    /// The body's classes packed: its windows' lookups.
    pack: Option<Rc<BodyPack>>,
    /// Values known all through the window at the start (`known_if_zero`), on a gate's quick
    /// path (`words_gated`).
    known: Rc<HashMap<usize, bool>>,
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
        let can = crate::byte_needs::made_from_bytes(native, &plan);
        Bytewise { native, plan, can, carries: Vec::new(), bail: None, packs: Default::default(), before_start: None, run_lanes: Default::default(), rows: None, rows_stored: true }
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
    /// p)` to its bits for the file's bytes `p .. p + 64`: the window's code alone, for its tests to read.
    #[cfg(test)]
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

    /// The pack of the classes values `roots` read, and of `extras` (`body_pack`), made once.
    fn pack_for(&self, roots: &[usize], extras: &[crate::class_pack::Need]) -> Rc<BodyPack> {
        let mut key_roots = roots.to_vec();
        key_roots.sort_unstable();
        key_roots.dedup();
        let key = (key_roots, extras.to_vec());
        if let Some(p) = self.packs.borrow().get(&key) {
            return p.clone();
        }
        let p = Rc::new(crate::native_cpu_lanes::body_pack(self.native, &key.0, extras));
        self.packs.borrow_mut().insert(key, p.clone());
        p
    }

    /// The instructions of outputs `os`.
    fn roots(&self, os: &[u16]) -> Vec<usize> {
        os.iter().map(|&o| usize::from(self.native.outputs[usize::from(o)])).collect()
    }

    /// Output `o`'s bits for the file's bytes `p .. p + 64` (`p` at least 0), none past its end.
    pub fn word(&self, f: &mut FunctionBuilder<'_>, at: &File, o: u16, p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let x = usize::from(self.native.outputs[usize::from(o)]);
        let d = self.native.output_delays[usize::from(o)];
        let start = f.ins().iadd_imm(p, i64::from(d));
        let m = self.mask(f, at, x, start, heads);
        if self.padded(o) {
            return m;
        }
        let cut = self.cut64(f, at, p);
        f.ins().band(m, cut)
    }

    /// Whether output `o` has no bits past the file's end already: it is false where the byte
    /// its delay back is zero (`zero_kills`), and the bytes past the end are zeros, `BACK` of
    /// them (`native_cpu_block::BACK`), more than a window at a block of the file reaches. As
    /// simdjson's padded input: a line comment's `\n`, a string's quote, no cut.
    pub fn padded(&self, o: u16) -> bool {
        let x = usize::from(self.native.outputs[usize::from(o)]);
        usize::from(self.native.pad) >= pad_reach(self.native) && crate::native_cpu_lanes::zero_kills(self.native, x, u32::from(self.native.output_delays[usize::from(o)]))
    }

    /// The cut of outputs `os`' window at `p` (`cut64`), where one of them needs it (`padded`);
    /// else all ones, an instruction no one reads.
    fn cut_for(&self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], p: Value) -> Value {
        if os.iter().all(|&o| self.padded(o)) {
            return f.ins().iconst(types::I64, -1);
        }
        self.cut64(f, at, p)
    }

    /// Outputs `os`' bits for the file's bytes `p .. p + 64`, none past its end, as `word`'s
    /// each: one window's vectors and words shared among them, as a lexer classifies a byte
    /// once for every test that reads it.
    pub fn words(&self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Vec<Value> {
        self.words_carried(f, at, os, p, &[], heads).0
    }

    /// `words`, and the words of sets `extras` the body tests besides (a byte set each, at
    /// `p .. p + 64`, none past the file's end): their classes and the outputs' packed into
    /// shared tables (`class_pack`), one lookup for all, as a lexer's window classifies a byte
    /// once for every set it asks.
    pub fn words_with(&self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], extras: &[crate::class_pack::Need], p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> (Vec<Value>, Vec<Value>) {
        let pack = self.pack_for(&self.roots(os), extras);
        let mut carry = Carry { pack: Some(pack.clone()), ..Carry::default() };
        let mut windows: HashMap<i64, Lanes<'_>> = HashMap::new();
        let mut memo = HashMap::new();
        // An extra set without the zero byte has no bits on the padding (`padded`).
        let cut = if extras.iter().all(zero_free) { self.cut_for(f, at, os, p) } else { self.cut64(f, at, p) };
        let words: Vec<Value> = os.iter().map(|&o| {
            let x = usize::from(self.native.outputs[usize::from(o)]);
            let d = i64::from(self.native.output_delays[usize::from(o)]);
            let m = self.mask_at(f, at, x, p, -d, &mut windows, &mut memo, &mut carry, heads);
            if self.padded(o) { m } else { f.ins().band(m, cut) }
        }).collect();
        let lanes = windows.entry(0).or_insert_with(|| {
            let base = f.ins().iadd(at.bytes, p);
            Lanes::window(f, self.native, &self.plan, base, p, at.len, at.nibble).packed(Some(pack.clone()))
        });
        let more: Vec<Value> = (0..extras.len()).map(|k| {
            let parts: Vec<_> = (1..=4).map(|c| lanes.extra(f, k, c)).collect();
            let m = lanes.mask(f, &parts, at.weights);
            let m = sym_value(f, m);
            if zero_free(&extras[k]) { m } else { f.ins().band(m, cut) }
        }).collect();
        (words, more)
    }

    /// The file's bytes' bits from `p`: `len - p` of them, at most 64. As the lean lexer's
    /// `block`, which pads only where 64 bytes are not left: a full window's are all its bits,
    /// one compare on its way; the tail's cut is made in a cold block of its own.
    fn cut64(&self, f: &mut FunctionBuilder<'_>, at: &File, p: Value) -> Value {
        let end = f.ins().iadd_imm(p, 64);
        let full = f.ins().icmp(IntCC::SignedLessThanOrEqual, end, at.len);
        let (tail, join) = (f.create_block(), f.create_block());
        f.append_block_param(join, types::I64);
        let ones = f.ins().iconst(types::I64, -1);
        f.ins().brif(full, join, &[ones.into()], tail, &[]);
        f.switch_to_block(tail);
        f.set_cold_block(tail);
        // Fewer than 64 left: `len - p` of them, none where `p` is past the end.
        let rem = f.ins().isub(at.len, p);
        let zero = f.ins().iconst(types::I64, 0);
        let rem = f.ins().smax(rem, zero);
        let one = f.ins().iconst(types::I64, 1);
        let low = f.ins().ishl(one, rem);
        let low = f.ins().iadd_imm(low, -1);
        f.ins().jump(join, &[low.into()]);
        f.switch_to_block(join);
        f.block_params(join)[0]
    }

    /// `words`, with `escaped(a)`'s carry in at `p` given for each `(a, carry)` of `cins`
    /// where the outputs read it at their window's start (`carried_runs`), not read back from
    /// the bytes; and each one's carry out, at `p + 64` (simdjson's add overflow), in `cins`'
    /// order: a loop of windows 64 bytes apart passes it on, as the lean lexer's reader does.
    pub fn words_carried(&self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], p: Value, cins: &[(usize, Value)], heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> (Vec<Value>, Vec<Value>) {
        let cut = self.cut_for(f, at, os, p);
        let carry = Carry { ins: cins.to_vec(), pack: Some(self.pack_for(&self.roots(os), &[])), ..Carry::default() };
        self.words_in(f, at, os, p, carry, None, cut, heads)
    }

    /// `words_carried`'s words and carries out, with `carry` given, the window at `p` the one
    /// given where it is (its loads and lookups made), and the file's cut `cut`.
    #[allow(clippy::too_many_arguments)]
    fn words_in<'s>(&'s self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], p: Value, mut carry: Carry, window: Option<Lanes<'s>>, cut: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> (Vec<Value>, Vec<Value>) {
        let mut windows: HashMap<i64, Lanes<'_>> = HashMap::new();
        if let Some(w) = window {
            windows.insert(0, w);
        }
        self.words_from(f, at, os, p, &mut carry, &mut windows, &mut HashMap::new(), cut, heads)
    }

    /// `words_in`, its windows and words made so far given.
    #[allow(clippy::too_many_arguments)]
    fn words_from<'s>(&'s self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], p: Value, carry: &mut Carry, windows: &mut HashMap<i64, Lanes<'s>>, memo: &mut HashMap<(usize, i64), Value>, cut: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> (Vec<Value>, Vec<Value>) {
        let words = os
            .iter()
            .map(|&o| {
                let x = usize::from(self.native.outputs[usize::from(o)]);
                // A delay `d` reads from `p + d`: `d` bytes ahead of the shared start.
                let d = i64::from(self.native.output_delays[usize::from(o)]);
                let m = self.mask_at(f, at, x, p, -d, windows, memo, carry, heads);
                if self.padded(o) { m } else { f.ins().band(m, cut) }
            })
            .collect();
        let outs = carry.ins.iter().map(|&(a, _)| match carry.outs.iter().find(|o| o.0 == a) {
            Some(o) => o.1,
            // A run none and its carry in none, its escapes known so (`known_if_zero`) and
            // not made: none out.
            None => {
                assert!(self.native.insts.iter().enumerate().any(|(x, i)| *i == BlockInst::Escaped(a as u16) && carry.known.get(&x) == Some(&false)), "a carried run's carry out (`carried_runs`)");
                f.ins().iconst(types::I64, 0)
            }
        }).collect();
        (words, outs)
    }

    /// Each value that a gate's zeros (`gate`) or a given carry in (`given`) may change: a
    /// gate's value, an escape of a given run, and what reads them. The others are the same
    /// on either path of `words_gated`.
    fn gated_by(&self, gate: &[usize], given: &[usize]) -> Vec<bool> {
        let mut d = vec![false; self.native.insts.len()];
        for (x, inst) in self.native.insts.iter().enumerate() {
            let o = |a: u16| d[usize::from(a)];
            d[x] = gate.contains(&x)
                || match *inst {
                    BlockInst::Escaped(a) => given.contains(&usize::from(a)) || o(a),
                    BlockInst::Not(a) | BlockInst::Prev(a, _) | BlockInst::Early(a, _) => o(a),
                    BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => o(a) || o(b),
                    _ => false,
                };
        }
        d
    }

    /// The values a gate of outputs `os`' window tests (`words_gated`): byte-local values whose
    /// zero at every byte of the window makes each output after the first, `os[1..]`, none
    /// there, and each `escaped` they read at its start whose run's carry in is given
    /// (`given`), and leaves the first (a reader's closer) not known. For each, its own value
    /// where it is byte-local (its bytes the fewest), else an `and`'s side, an `or`'s both,
    /// an escape's run. `None` where one is made none by none of them, or nothing is gated.
    pub fn zero_gate(&self, os: &[u16], given: &[usize]) -> Option<Vec<usize>> {
        fn escapes(n: &BlockNative, x: usize, out: &mut Vec<usize>) {
            match n.insts[x] {
                BlockInst::Not(a) => escapes(n, usize::from(a), out),
                BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => {
                    escapes(n, usize::from(a), out);
                    escapes(n, usize::from(b), out);
                }
                BlockInst::Escaped(a) => {
                    out.push(x);
                    escapes(n, usize::from(a), out);
                }
                _ => {}
            }
        }
        let (&first, rest) = os.split_first()?;
        if os.iter().any(|&o| self.native.output_delays[usize::from(o)] != 0) {
            return None;
        }
        let closer = usize::from(self.native.outputs[usize::from(first)]);
        let mut targets = self.roots(rest);
        for x in self.roots(os) {
            escapes(self.native, x, &mut targets);
        }
        targets.retain(|&x| !matches!(self.native.insts[x], BlockInst::Escaped(a) if !given.contains(&usize::from(a))));
        targets.sort_unstable();
        targets.dedup();
        if targets.is_empty() {
            return None;
        }
        let kills = |x: usize| known_if_zero(self.native, &[x], given).get(&closer) == Some(&false);
        let bytes = |xs: &[usize]| xs.iter().map(|&x| crate::native_cpu_lanes::byte_set(self.native, x).map_or(256, |s| s.iter().map(|w| w.count_ones()).sum())).sum::<u32>();
        // The values that make `x` none, fewest new to `have` (then fewest bytes).
        fn cover(b: &Bytewise<'_>, x: usize, given: &[usize], have: &[usize], kills: &dyn Fn(usize) -> bool, bytes: &dyn Fn(&[usize]) -> u32) -> Option<Vec<usize>> {
            if have.contains(&x) {
                return Some(Vec::new());
            }
            let leaf = !matches!(b.native.insts[x], BlockInst::Const(_)) && crate::native_cpu_lanes::byte_set(b.native, x).is_some();
            if leaf && !kills(x) {
                return Some(vec![x]);
            }
            let go = |a: u16| cover(b, usize::from(a), given, have, kills, bytes);
            match b.native.insts[x] {
                BlockInst::Const(0) => Some(Vec::new()),
                BlockInst::And(p, q) => match (go(p), go(q)) {
                    (Some(u), Some(v)) => Some(if (u.len(), bytes(&u)) <= (v.len(), bytes(&v)) { u } else { v }),
                    (u, v) => u.or(v),
                },
                BlockInst::AndNot(p, _) => go(p),
                BlockInst::Or(p, q) | BlockInst::Xor(p, q) => {
                    let (mut u, v) = (go(p)?, go(q)?);
                    u.extend(v.into_iter().filter(|y| !u.contains(y)).collect::<Vec<_>>());
                    Some(u)
                }
                BlockInst::Escaped(a) if given.contains(&usize::from(a)) => go(a),
                _ => None,
            }
        }
        let mut gate: Vec<usize> = Vec::new();
        for &t in &targets {
            let more = cover(self, t, given, &gate, &kills, &bytes)?;
            gate.extend(more);
        }
        gate.sort_unstable();
        gate.dedup();
        let k = known_if_zero(self.native, &gate, given);
        (targets.iter().all(|t| k.get(t) == Some(&false)) && k.get(&closer) != Some(&false)).then_some(gate)
    }

    /// `words_carried`, gated on the window's rare bytes (`zero_gate`): one test of them all,
    /// their bytes one set packed with the outputs' classes (`class_pack`), and of the
    /// carries in. Where none holds, the outputs are made with those values zero: an escape's
    /// math and the tests they make none fold away, as a lexer's string loop tests a chunk for
    /// a `\` or a control byte once and copies on. `then` is given each path's words and
    /// carries out, and ends its block: the code after runs on each path, its zeros constants
    /// there. With nothing to gate, `then` once, on `words_carried`'s.
    ///
    /// `prefix`: the code after reads the words only up to the first output's first bit, and
    /// no carry out. A bit at a byte is made from that byte and those before it, so the gate
    /// need hold only up to there: the first output is made assuming the gate's zeros, and
    /// the gate is tested at and below its first bit, as the lean lexer tests for a `\\` or
    /// a control byte below the quote. The quick path's words past that bit, and its carries
    /// out, are not given (`then` is given none); the whole path shares what the gate does
    /// not change.
    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    pub fn words_gated(&self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], p: Value, cins: &[(usize, Value)], prefix: bool, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>, then: &mut dyn FnMut(&mut FunctionBuilder<'_>, &[Value], &[Value], &mut Vec<(cranelift_codegen::ir::Block, Step)>)) {
        let given: Vec<usize> = cins.iter().map(|c| c.0).collect();
        let Some(gate) = self.zero_gate(os, &given) else {
            let (w, o) = self.words_carried(f, at, os, p, cins, heads);
            return then(f, &w, &o, heads);
        };
        let mut set = [0u64; 4];
        for &g in &gate {
            let s = crate::native_cpu_lanes::byte_set(self.native, g).expect("a gate's values are byte-local");
            for (w, v) in set.iter_mut().zip(s) {
                *w |= v;
            }
        }
        let pack = self.pack_for(&self.roots(os), &[crate::class_pack::Need { set, care: [u64::MAX; 4] }]);
        // The gate's bytes without the zero byte have no bits on the padding (`padded`).
        let gated_zero = set[0] & 1 != 0;
        let cut = if gated_zero { self.cut64(f, at, p) } else { self.cut_for(f, at, os, p) };
        let base = f.ins().iadd(at.bytes, p);
        let mut lanes = Lanes::window(f, self.native, &self.plan, base, p, at.len, at.nibble).packed(Some(pack.clone()));
        let parts: Vec<_> = (1..=4).map(|c| lanes.extra(f, 0, c)).collect();
        let m = lanes.mask(f, &parts, at.weights);
        let m = sym_value(f, m);
        let mut any = if gated_zero { f.ins().band(m, cut) } else { m };
        let known = Rc::new(known_if_zero(self.native, &gate, &given));
        let quick_carry = |f: &mut FunctionBuilder<'_>| {
            let zero = f.ins().iconst(types::I64, 0);
            Carry { ins: cins.iter().map(|&(a, _)| (a, zero)).collect(), pack: Some(pack.clone()), known: known.clone(), ..Carry::default() }
        };
        let (quick, whole) = (f.create_block(), f.create_block());
        if prefix {
            // The quick words first, the gate below the first output's first bit.
            let mut carry = quick_carry(f);
            let mut windows: HashMap<i64, Lanes<'_>> = HashMap::from([(0, lanes.clone().assuming(known.clone()))]);
            let mut memo = HashMap::new();
            let (w, _) = self.words_from(f, at, os, p, &mut carry, &mut windows, &mut memo, cut, heads);
            let w1 = f.ins().iadd_imm(w[0], -1);
            let below = f.ins().bxor(w[0], w1);
            any = f.ins().band(any, below);
            for &(_, c) in cins {
                any = f.ins().bor(any, c);
            }
            f.ins().brif(any, whole, &[], quick, &[]);
            f.switch_to_block(quick);
            then(f, &w, &[], heads);
            f.switch_to_block(whole);
            let by = self.gated_by(&gate, &given);
            let mut windows: HashMap<i64, Lanes<'_>> = windows.into_iter().map(|(back, l)| (back, if back == 0 { l.unassuming(&by) } else { l })).collect();
            memo.retain(|&(x, back), _| back != 0 || !by[x]);
            let mut carry = Carry { ins: cins.to_vec(), pack: Some(pack), ..Carry::default() };
            let (w, o) = self.words_from(f, at, os, p, &mut carry, &mut windows, &mut memo, cut, heads);
            return then(f, &w, &o, heads);
        }
        for &(_, c) in cins {
            any = f.ins().bor(any, c);
        }
        f.ins().brif(any, whole, &[], quick, &[]);
        f.switch_to_block(quick);
        let carry = quick_carry(f);
        let (w, o) = self.words_in(f, at, os, p, carry, Some(lanes.clone().assuming(known)), cut, heads);
        then(f, &w, &o, heads);
        f.switch_to_block(whole);
        let carry = Carry { ins: cins.to_vec(), pack: Some(pack), ..Carry::default() };
        let (w, o) = self.words_in(f, at, os, p, carry, Some(lanes), cut, heads);
        then(f, &w, &o, heads);
    }

    /// Output `o`'s bits for the file's bytes `p .. p + 16` (`p` at least 0), none past its end:
    /// `word`'s first 16, from one vector of the bytes, as a lexer reads a short token's bytes
    /// and no more.
    pub fn short_word(&self, f: &mut FunctionBuilder<'_>, at: &File, o: u16, p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        self.short_words(f, at, &[o], p, heads)[0]
    }

    /// `short_word` for each of outputs `os`, from one window's vectors shared among them, as
    /// a lexer reads a number's digits and asks of its `.` in one pass.
    ///
    /// Each mask has `short_stride(os)` bits a lane: lane `i` at bit `stride * i`.
    pub fn short_words(&self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Vec<Value> {
        let terms: Vec<(usize, i64)> = os.iter().map(|&o| (usize::from(self.native.outputs[usize::from(o)]), i64::from(self.native.output_delays[usize::from(o)]))).collect();
        self.short_terms(f, at, &terms, p, heads)
    }

    /// `short_words` of instructions at delays, `(x, d)` each: `x` at the window's byte `i`
    /// read at stream position `p + i + d`, as an output of delay `d` is. A run's split
    /// (`run_split`) so reads its byte term in the window its tests share.
    pub fn short_terms(&self, f: &mut FunctionBuilder<'_>, at: &File, terms: &[(usize, i64)], p: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Vec<Value> {
        self.short_terms_carried(f, at, terms, p, &[], heads)
    }

    /// `short_words` with the escapes' carries in given (`cins`, as `words_carried`'s): a
    /// reader's closer after an opener apart from its escapes (0358), and the bits a lane
    /// (`short_stride`).
    pub fn short_words_carried(&self, f: &mut FunctionBuilder<'_>, at: &File, os: &[u16], p: Value, cins: &[(usize, Value)], heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> (Vec<Value>, u32) {
        let terms: Vec<(usize, i64)> = os.iter().map(|&o| (usize::from(self.native.outputs[usize::from(o)]), i64::from(self.native.output_delays[usize::from(o)]))).collect();
        let xs: Vec<usize> = terms.iter().map(|t| t.0).collect();
        (self.short_terms_carried(f, at, &terms, p, cins, heads), self.short_stride_x(&xs))
    }

    /// `short_terms` with the escapes' carries in given.
    fn short_terms_carried(&self, f: &mut FunctionBuilder<'_>, at: &File, terms: &[(usize, i64)], p: Value, cins: &[(usize, Value)], heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Vec<Value> {
        let xs: Vec<usize> = terms.iter().map(|t| t.0).collect();
        let stride = self.short_stride_x(&xs);
        let mut carry = Carry { short: true, stride, ins: cins.to_vec(), pack: Some(self.pack_for(&xs, &[])), ..Carry::default() };
        let mut windows: HashMap<i64, Lanes<'_>> = HashMap::new();
        let mut memo = HashMap::new();
        let Some((&(x0, d0), rest)) = terms.split_first() else { return Vec::new() };
        let mut got = vec![self.short_x(f, at, x0, d0, p, &mut windows, &mut memo, &mut carry, heads)];
        // The first term's window (its delay's): a term of one lane's instruction read in it,
        // `lag` bytes back, its loads and compares the first's.
        for &(x, d) in rest {
            let lag = d0 - d;
            let shared = self.plan.lanes[x] && (0..=8).contains(&lag) && windows.contains_key(&-d0);
            let m = if shared {
                let lanes = windows.get_mut(&-d0).expect("the first output's window");
                let part = lanes.get_lagged(f, x, 1, lag as u8);
                match lanes.mask_short(f, part, at.weights, stride) {
                    Sym::Known(k) => f.ins().iconst(types::I64, k as i64),
                    Sym::Value(v, false) => v,
                    Sym::Value(v, true) => f.ins().bnot(v),
                    Sym::Slot(..) => unreachable!("a window's mask is a value"),
                    Sym::Unmade => unreachable!("a window's mask is made"),
                }
            } else {
                self.short_x(f, at, x, d, p, &mut windows, &mut memo, &mut carry, heads)
            };
            got.push(if shared { self.short_cut(f, at, x, d, p, m, stride) } else { m });
        }
        got
    }

    /// The bits a lane of `short_words(os)`' masks: `native_cpu_block::SHORT_STRIDE`, where
    /// every output is made of its lanes and of their lane-wise logic and `prev`s (a shift by
    /// the stride); one where a constant's pattern or an escape's run reads bit positions.
    pub fn short_stride(&self, os: &[u16]) -> u32 {
        let xs: Vec<usize> = os.iter().map(|&o| usize::from(self.native.outputs[usize::from(o)])).collect();
        self.short_stride_x(&xs)
    }

    /// `short_stride` of instructions `xs` (`short_terms`).
    pub fn short_stride_x(&self, xs: &[usize]) -> u32 {
        fn lanewise(b: &Bytewise<'_>, x: usize, seen: &mut Vec<bool>) -> bool {
            if b.plan.lanes[x] || std::mem::replace(&mut seen[x], true) {
                return true;
            }
            match b.native.insts[x] {
                BlockInst::Not(a) | BlockInst::Prev(a, _) | BlockInst::Early(a, _) => lanewise(b, usize::from(a), seen),
                BlockInst::And(a, c) | BlockInst::Or(a, c) | BlockInst::Xor(a, c) | BlockInst::AndNot(a, c) => lanewise(b, usize::from(a), seen) && lanewise(b, usize::from(c), seen),
                _ => false,
            }
        }
        let mut seen = vec![false; self.native.insts.len()];
        let all = xs.iter().all(|&x| lanewise(self, x, &mut seen));
        if all { crate::native_cpu_block::SHORT_STRIDE } else { 1 }
    }

    /// `short_x`'s cut of mask `m` (output instruction `x`, delay `d`, from `p`): none past the
    /// file's end. Where the output is false on a zero byte (`zero_kills`), none past it
    /// already: the bytes there are zeros, more than the 16 and its delay
    /// (`native_cpu_block::Batch`). Else the file's bytes' bits: `len - p`, at most 16.
    ///
    /// Either way the word is its 16 lanes' bits: at one bit a lane (`SHORT_STRIDE` off NEON)
    /// the 48 bits above them are no lane's, and a complement (a class's lane where the backend
    /// has no bit test, `Sym`'s), a `not` or a `prev`'s shift sets them; a closer's first bit
    /// found there would be past the window. Four bits a lane fill the word.
    #[allow(clippy::too_many_arguments)]
    fn short_cut(&self, f: &mut FunctionBuilder<'_>, at: &File, x: usize, d: i64, p: Value, m: Value, stride: u32) -> Value {
        if crate::native_cpu_lanes::zero_kills(self.native, x, d as u32) {
            return if 16 * stride < 64 { f.ins().band_imm(m, (1i64 << (16 * stride)) - 1) } else { m };
        }
        let rem = f.ins().isub(at.len, p);
        let zero = f.ins().iconst(types::I64, 0);
        let sixteen = f.ins().iconst(types::I64, 16);
        let rem = f.ins().smax(rem, zero);
        let rem = f.ins().smin(rem, sixteen);
        let low = crate::native_cpu_block::lanes_below(f, rem, stride, 16);
        f.ins().band(m, low)
    }

    #[allow(clippy::too_many_arguments)]
    fn short_x<'s>(&'s self, f: &mut FunctionBuilder<'_>, at: &File, x: usize, d: i64, p: Value, windows: &mut HashMap<i64, Lanes<'s>>, memo: &mut HashMap<(usize, i64), Value>, carry: &mut Carry, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let m = self.mask_at(f, at, x, p, -d, windows, memo, carry, heads);
        let stride = carry.stride;
        self.short_cut(f, at, x, d, p, m, stride)
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
                BlockInst::Early(a, k) => walk(b, usize::from(a), back - i64::from(k), seen, runs),
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

    /// A run's split, as the lean lexer reads a number: output `o` as `a | rest`, `a` its one
    /// `Or` term that is the bytes of a set at the output's own byte (`exact_at`, under its
    /// `prev`s), `rest` the others. The run of `a` from one window, `rest` asked only at that
    /// run's end, of the bytes there: lean's `while digit` then `b[end] == b'.' &&
    /// digit(b[end + 1])`, where the run's whole output reads three windows (a byte's `prev`
    /// and next). `(x, d, set)`: `a`'s instruction, its delay the output's (`short_terms`),
    /// its bytes. None where no term or every term is such.
    pub fn run_split(&self, o: u16) -> Option<(usize, i64, [u64; 4])> {
        // The term's byte, `back` before the instruction's position, and its set.
        fn at_byte(native: &BlockNative, x: usize) -> Option<(i64, [u64; 4])> {
            match native.insts[x] {
                BlockInst::Prev(a, k) => at_byte(native, usize::from(a)).map(|(b, set)| (b + i64::from(k), set)),
                _ => exact_at(native, x).filter(|n| n.lo == n.hi).map(|n| (-i64::from(n.lo), n.set)),
            }
        }
        let x = usize::from(*self.native.outputs.get(usize::from(o))?);
        let d = i64::from(self.native.output_delays[usize::from(o)]);
        let all = or_terms(self.native, x);
        let own: Vec<(usize, [u64; 4])> = all.iter().filter_map(|&y| at_byte(self.native, y).filter(|b| b.0 == d).map(|b| (y, b.1))).collect();
        match own[..] {
            [(y, set)] if all.len() > 1 => Some((y, d, set)),
            _ => None,
        }
    }

    /// `bit` at a token's first byte `s`, known in `first`, where the token's bytes from `s + 1`
    /// are a split run's term's `lanes` (`RunLanes`): a test of those bytes of the term's set is
    /// a lane of its mask, as lean's number arm asks `b[pos] == b'0'` of its digits' first.
    #[allow(clippy::too_many_arguments)]
    pub fn bit_on_run(&self, f: &mut FunctionBuilder<'_>, at: &File, o: u16, s: Value, first: Option<[u64; 4]>, lanes: RunLanes, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        self.run_lanes.set(Some(lanes));
        let b = self.bit_first(f, at, o, s, first, heads);
        self.run_lanes.set(None);
        b
    }

    /// Whether the byte at `p` is in `set`, 0 or 1, by one load (`RunForm::Bytes`' loop).
    pub fn byte_in(&self, f: &mut FunctionBuilder<'_>, at: &File, p: Value, set: [u64; 4]) -> Value {
        let b = byte_at(f, at, p);
        in_set(f, b, set)
    }

    /// Whether output `o` at a token's first byte reads only that byte and bytes a split run's
    /// lanes of `set` give (`bit_on_run`), the first `known` of them: no other byte read.
    pub fn on_run_only(&self, o: u16, set: [u64; 4], known: i64) -> bool {
        fn go(b: &Bytewise<'_>, x: usize, back: i64, d: i64, set: [u64; 4], known: i64) -> bool {
            let j = d - back;
            match b.native.insts[x] {
                BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) => j == 0 || ((1..=known).contains(&j) && exact_at(b.native, x).is_some_and(|n| n.set == set)),
                // The prefix's bytes lie before the stream, so before any token's.
                BlockInst::Within(_) | BlockInst::Const(_) | BlockInst::Prefix(..) => true,
                BlockInst::Not(a) => go(b, usize::from(a), back, d, set, known),
                BlockInst::Prev(a, k) => go(b, usize::from(a), back + i64::from(k), d, set, known),
                BlockInst::Early(a, k) => go(b, usize::from(a), back - i64::from(k), d, set, known),
                BlockInst::And(a, c) | BlockInst::Or(a, c) | BlockInst::Xor(a, c) | BlockInst::AndNot(a, c) => go(b, usize::from(a), back, d, set, known) && go(b, usize::from(c), back, d, set, known),
                _ => false,
            }
        }
        let x = usize::from(self.native.outputs[usize::from(o)]);
        let d = i64::from(self.native.output_delays[usize::from(o)]);
        go(self, x, 0, d, set, known)
    }

    /// A split run's rest (`run_split`, its byte term `a`) at the file's byte `p`, 0 or 1, by
    /// scalar loads: `a` known false there (the run of it ended), so only the other terms,
    /// and the byte at `p - 1` known in `before`. Lean's `b[end] == b'.' &&
    /// digit(b[end + 1])`, its digit test at `end` not asked again.
    #[allow(clippy::too_many_arguments)]
    pub fn rest_after(&self, f: &mut FunctionBuilder<'_>, at: &File, o: u16, a: usize, p: Value, before: [u64; 4], heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let x = usize::from(self.native.outputs[usize::from(o)]);
        let d = i64::from(self.native.output_delays[usize::from(o)]);
        let pos = f.ins().iadd_imm(p, d);
        let mut memo = HashMap::new();
        let mut v: Option<Value> = None;
        for y in or_terms(self.native, x).into_iter().filter(|&y| y != a) {
            let b = self.scalar(f, at, y, pos, 0, Some(d), Some((d + 1, before)), &mut memo, heads);
            v = Some(match v {
                Some(v) => f.ins().bor(v, b),
                None => b,
            });
        }
        let b = v.unwrap_or_else(|| f.ins().iconst(types::I64, 0));
        // Past the file's end, none (`bit_first`'s).
        if crate::native_cpu_lanes::zero_kills(self.native, x, d as u32) {
            return b;
        }
        let inside = f.ins().icmp(IntCC::SignedLessThan, p, at.len);
        let inside = f.ins().uextend(types::I64, inside);
        f.ins().band(b, inside)
    }

    /// The bytes before a split run's second end (`run_split`): the byte term's where its
    /// rest's next byte is always one of its own (lean's `.` then a digit, so its second run
    /// is never empty), else those or the rest's own byte.
    pub fn split_before_again(&self, o: u16, a: usize, set: [u64; 4]) -> [u64; 4] {
        let next = self.rest_bytes(o, a, 1);
        if (0..4).all(|k| next[k] & !set[k] == 0) {
            set
        } else {
            let here = self.rest_bytes(o, a, 0);
            std::array::from_fn(|k| set[k] | here[k])
        }
    }

    /// The bytes at offset `off` from where a split run's rest holds (`run_split`, its byte
    /// term `a`): its other terms' (`bytes_at`), together.
    pub fn rest_bytes(&self, o: u16, a: usize, off: i32) -> [u64; 4] {
        let x = usize::from(self.native.outputs[usize::from(o)]);
        let d = i32::from(self.native.output_delays[usize::from(o)]);
        or_terms(self.native, x).into_iter().filter(|&y| y != a).fold([0u64; 4], |acc, y| {
            let b = inst_bytes_at(self.native, y, off - d);
            std::array::from_fn(|k| acc[k] | b[k])
        })
    }

    /// Instruction `x`'s word for stream positions `start .. start + 64`.
    fn mask(&self, f: &mut FunctionBuilder<'_>, at: &File, x: usize, start: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        let mut windows: HashMap<i64, Lanes<'_>> = HashMap::new();
        let mut memo = HashMap::new();
        let mut carry = Carry { pack: Some(self.pack_for(&[x], &[])), ..Carry::default() };
        self.mask_at(f, at, x, start, 0, &mut windows, &mut memo, &mut carry, heads)
    }

    /// `x`'s word from `start - back`; the windows' vectors and the words shared.
    #[allow(clippy::too_many_arguments)]
    fn mask_at<'s>(&'s self, f: &mut FunctionBuilder<'_>, at: &File, x: usize, start: Value, back: i64, windows: &mut HashMap<i64, Lanes<'s>>, memo: &mut HashMap<(usize, i64), Value>, carry: &mut Carry, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        if let Some(&v) = memo.get(&(x, back)) {
            return v;
        }
        // Known all through the window (a gate's quick path): an escape so has none out.
        if back == 0
            && let Some(&k) = carry.known.get(&x)
        {
            if let BlockInst::Escaped(a) = self.native.insts[x]
                && carry.ins.iter().any(|c| c.0 == usize::from(a))
            {
                let none = f.ins().iconst(types::I64, 0);
                carry.outs.push((usize::from(a), none));
            }
            let v = f.ins().iconst(types::I64, if k { -1 } else { 0 });
            memo.insert((x, back), v);
            return v;
        }
        let here = |f: &mut FunctionBuilder<'_>| if back == 0 { start } else { f.ins().iadd_imm(start, -back) };
        let v = if self.plan.lanes[x] {
            let pos = here(f);
            let lanes = windows.entry(back).or_insert_with(|| {
                let base = f.ins().iadd(at.bytes, pos);
                Lanes::window(f, self.native, &self.plan, base, pos, at.len, at.nibble).packed(carry.pack.clone())
            });
            let m = if carry.short {
                let part = lanes.get(f, x, 1);
                lanes.mask_short(f, part, at.weights, carry.stride)
            } else {
                let parts: Vec<_> = (1..=4).map(|c| lanes.get(f, x, c)).collect();
                lanes.mask(f, &parts, at.weights)
            };
            sym_value(f, m)
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
                // `a` `k` bytes later (`BlockOp::Early`): its bits all lie in the file's first
                // `EARLY_BITS` positions, so no window cuts them, and a window from there on
                // has none. Made only in a window that holds them, behind a branch, its values
                // its own: the others' windows never see it.
                BlockInst::Early(a, k) => {
                    let pos = here(f);
                    let near = f.ins().icmp_imm(IntCC::SignedLessThan, pos, i64::from(crate::block::EARLY_BITS));
                    let (cold, join) = (f.create_block(), f.create_block());
                    f.append_block_param(join, types::I64);
                    let zero = f.ins().iconst(types::I64, 0);
                    f.ins().brif(near, cold, &[], join, &[zero.into()]);
                    f.switch_to_block(cold);
                    f.set_cold_block(cold);
                    let (mut w, mut m) = (HashMap::new(), HashMap::new());
                    let v = self.mask_at(f, at, usize::from(a), start, back - i64::from(k), &mut w, &mut m, carry, heads);
                    f.ins().jump(join, &[v.into()]);
                    f.switch_to_block(join);
                    f.block_params(join)[0]
                }
                // The prefix's positions (`BlockOp::Prefix`): the file's first `n`, where it
                // starts with the prefix's bytes, compared once (`prefix_start`). Only a window
                // that holds them has any, behind a branch, as an `early`'s.
                BlockInst::Prefix(n, w) => {
                    let pos = here(f);
                    let near = f.ins().icmp_imm(IntCC::SignedLessThan, pos, i64::from(n));
                    let (cold, join) = (f.create_block(), f.create_block());
                    f.append_block_param(join, types::I64);
                    let zero = f.ins().iconst(types::I64, 0);
                    f.ins().brif(near, cold, &[], join, &[zero.into()]);
                    f.switch_to_block(cold);
                    f.set_cold_block(cold);
                    let k = crate::native_cpu_block::prefix_start(f, n, w, at.bytes, at.len);
                    let one = f.ins().iconst(types::I64, 1);
                    let stride = i64::from(carry.stride.max(1));
                    let mut v = f.ins().iconst(types::I64, 0);
                    // Position `j`'s bit, `j - pos` lanes into the window.
                    for j in 0..i64::from(n) {
                        let d = f.ins().irsub_imm(pos, j);
                        let ge = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, d, 0);
                        let lt = f.ins().icmp_imm(IntCC::SignedLessThan, d, 64 / stride);
                        let set = f.ins().icmp_imm(IntCC::SignedGreaterThan, k, j);
                        let ok = f.ins().band(ge, lt);
                        let ok = f.ins().band(ok, set);
                        let sh = if stride == 1 { d } else { f.ins().imul_imm(d, stride) };
                        let bit = f.ins().ishl(one, sh);
                        let bit = f.ins().select(ok, bit, zero);
                        v = f.ins().bor(v, bit);
                    }
                    f.ins().jump(join, &[v.into()]);
                    f.switch_to_block(join);
                    f.block_params(join)[0]
                }
                BlockInst::Prev(a, k) => {
                    // `a` `k` further back; no bits before the stream.
                    let m = self.mask_at(f, at, usize::from(a), start, back + i64::from(k), windows, memo, carry, heads);
                    let pos = here(f);
                    let need = f.ins().irsub_imm(pos, i64::from(k));
                    let zero = f.ins().iconst(types::I64, 0);
                    let need = f.ins().smax(need, zero);
                    // A short window's positions `stride` bits apart (`short_stride`).
                    let need = if carry.stride > 1 { f.ins().imul_imm(need, i64::from(carry.stride)) } else { need };
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
        // A byte of a split run's term (`run_lanes`, `bit_on_run`): its mask's lane.
        if let (Some(rl), Some(a)) = (self.run_lanes.get(), anchor)
            && matches!(self.native.insts[x], BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_))
            && (1..=rl.known).contains(&(a - back))
            && exact_at(self.native, x).is_some_and(|n| n.set == rl.set)
        {
            let lane = (a - back - 1) * i64::from(rl.stride);
            let v = if lane == 0 { rl.mask } else { f.ins().ushr_imm(rl.mask, lane) };
            let v = f.ins().band_imm(v, 1);
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
                let b = byte_once(f, at, i, back, memo);
                in_set(f, b, s)
            }
            BlockInst::Class { table, bits } => {
                let b = byte_once(f, at, i, back, memo);
                in_set(f, b, set_of(table, bits))
            }
            BlockInst::Pair { before, table, bits } => {
                // A shared class of `bits`: each bit's two sets.
                let b = byte_once(f, at, i, back, memo);
                let p = {
                    let j = f.ins().iadd_imm(i, -1);
                    byte_once(f, at, j, back + 1, memo)
                };
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
                let b = byte_once(f, at, i, back, memo);
                let b = f.ins().ushr_imm(b, i64::from(k));
                f.ins().band_imm(b, 1)
            }
            BlockInst::PlaneBefore(k) => {
                let j = f.ins().iadd_imm(i, -1);
                let b = byte_once(f, at, j, back + 1, memo);
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
            // Whether the anchor's own byte is in the file (`i - d` is `p`): it is, where the
            // answer counts. The callers (`bit_first`, `rest_after`) cut the bit by `p < len`,
            // or `zero_kills` proves it false at a zero `p` without a `within`. A token's
            // `-` not before a digit asks no bound, as lean's `b[pos + 1]` on padded bytes.
            BlockInst::Within(d) if anchor == Some(back + i64::from(d)) => f.ins().iconst(types::I64, 1),
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
            // The prefix's positions: below the stream's start (`prefix_start`).
            BlockInst::Prefix(n, w) => {
                let k = crate::native_cpu_block::prefix_start(f, n, w, at.bytes, at.len);
                let ge = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, i, 0);
                let lt = f.ins().icmp(IntCC::SignedLessThan, i, k);
                let both = f.ins().band(ge, lt);
                f.ins().uextend(types::I64, both)
            }
            // As `mask_at`'s: none from `EARLY_BITS` on.
            BlockInst::Early(a, k) => {
                let v = self.scalar(f, at, usize::from(a), pos, back - i64::from(k), anchor, first, memo, heads);
                let near = f.ins().icmp_imm(IntCC::SignedLessThan, i, i64::from(crate::block::EARLY_BITS));
                let near = f.ins().uextend(types::I64, near);
                f.ins().band(v, near)
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
        if let Some(n) = exact_at(self.native, x).filter(|n| n.lo == n.hi && back - i64::from(n.lo) == at)
            && let Some(v) = decide(n.set) {
                return Some(v);
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
            // `escaped(a)` at the token's first byte: its run of `a` ends at the byte before,
            // none where `a` holds on no byte that can be there (`before_start`).
            BlockInst::Escaped(a) if back == at => {
                let pre = self.before_start.as_ref()?;
                let n = needs_at(self.native, usize::from(a), &mut HashMap::new())?;
                let meets = |x: &[u64; 4], y: &[u64; 4]| x.iter().zip(y).any(|(p, q)| p & q != 0);
                let none = pre.iter().filter(|(next, _)| meets(next, &set)).all(|(_, bytes)| !meets(bytes, &n.set));
                (n.lo == 0 && n.hi == 0 && none).then_some(false)
            }
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
            BlockInst::Early(a, k) => self.fold(usize::from(a), back - i64::from(k), anchor, fi),
            BlockInst::Prev(a, k) => {
                let reach = back + i64::from(k);
                let bounded = anchor.is_some_and(|d| zeros_bound(self.native, usize::from(a), reach - d));
                self.fold(usize::from(a), reach, anchor, fi).filter(|&v| !v || bounded)
            }
            _ => None,
        }
    }
}

/// A window's mask, its value.
fn sym_value(f: &mut FunctionBuilder<'_>, s: Sym) -> Value {
    match s {
        Sym::Known(k) => f.ins().iconst(types::I64, k as i64),
        Sym::Value(v, false) => v,
        Sym::Value(v, true) => f.ins().bnot(v),
        Sym::Slot(..) => unreachable!("a window's mask is a value"),
        Sym::Unmade => unreachable!("a window's mask is made"),
    }
}

/// The file's byte at `i`, 0 outside it: a load, as the batch's layout has zeros there
/// (`native_cpu_block::Batch`: `FRONT` before the file, past its end as far as the program
/// looks ahead). Further before, a `prev`'s bound says none (`scalar`).
fn byte_at(f: &mut FunctionBuilder<'_>, at: &File, i: Value) -> Value {
    let addr = f.ins().iadd(at.bytes, i);
    f.ins().uload8(types::I64, MemFlags::trusted().with_readonly(), addr, 0)
}

/// The byte at `i`, `back` before a scalar test's position, read once among its tests
/// (`scalar`'s memo, under a key no instruction has), as a lexer reads `b[end]` once.
fn byte_once(f: &mut FunctionBuilder<'_>, at: &File, i: Value, back: i64, memo: &mut HashMap<(usize, i64), Value>) -> Value {
    if let Some(&v) = memo.get(&(usize::MAX, back)) {
        return v;
    }
    let v = byte_at(f, at, i);
    memo.insert((usize::MAX, back), v);
    v
}

/// Whether byte `b` is in the 256-bit set, 0 or 1.
fn in_set(f: &mut FunctionBuilder<'_>, b: Value, set: [u64; 4]) -> Value {
    // One byte: a compare, as a lexer's `b == c`. A run of bytes: a subtract and a compare,
    // as lean's `is_ascii_digit` (0177, "A number's run ends as lean's": the set's word was
    // kept for a count, an instruction a token, and is not lean's source).
    if let Some((lo, hi)) = crate::native_cpu_lanes::set_range(set) {
        if lo == hi {
            let same = f.ins().icmp_imm(IntCC::Equal, b, i64::from(lo));
            return f.ins().uextend(types::I64, same);
        }
        let off = f.ins().iadd_imm(b, -i64::from(lo));
        let inside = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, off, i64::from(hi - lo));
        return f.ins().uextend(types::I64, inside);
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
            newlines: None,
            delay: 0,
            output_delays: vec![0],
            groups: Vec::new(),
            records: None,
            walks: Vec::new(),
            pad: crate::native_cpu_block::BACK as u16,
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

    /// A program of two classes spread over two tables (as products are allocated in order of
    /// first use), and a third set the body tests besides: one window looks them all up in
    /// one table (`class_pack`), two nibble lookups a 16 bytes, and each word is its set's.
    #[test]
    fn a_windows_classes_share_one_lookup() {
        use crate::block::ClassTable;
        use crate::class_pack::{Need, has};
        use cranelift_codegen::ir::{AbiParam, Opcode};
        use cranelift_module::{Linkage, Module};
        let mut t0 = ClassTable::default();
        let mut t1 = ClassTable::default();
        let put = |t: &mut ClassTable, bit: u8, hi: &[usize], lo: std::ops::RangeInclusive<usize>, also: &[usize]| {
            for &h in hi {
                t.hi[h] |= 1 << bit;
            }
            for l in lo.chain(also.iter().copied()) {
                t.lo[l] |= 1 << bit;
            }
        };
        // Table 0: whitespace in bits 0 and 1, six products of other classes after them.
        put(&mut t0, 0, &[2], 0..=0, &[]);
        put(&mut t0, 1, &[0], 9..=10, &[13]);
        for bit in 2..8u8 {
            put(&mut t0, bit, &[8 + usize::from(bit)], 0..=3, &[]);
        }
        // Table 1: a word's bytes, digits, letters and `_`.
        put(&mut t1, 0, &[3], 0..=9, &[]);
        put(&mut t1, 1, &[4, 6], 1..=15, &[]);
        put(&mut t1, 2, &[5, 7], 0..=10, &[]);
        put(&mut t1, 3, &[5], 15..=15, &[]);
        let native = BlockNative {
            tables: vec![t0, t1],
            insts: vec![BlockInst::Class { table: 0, bits: 0b11 }, BlockInst::Class { table: 1, bits: 0b1111 }],
            outputs: vec![0, 1],
            extracts: Vec::new(),
            totals: Vec::new(),
            masks: Vec::new(),
            newlines: None,
            delay: 0,
            output_delays: vec![0, 0],
            groups: Vec::new(),
            records: None,
            walks: Vec::new(),
            pad: crate::native_cpu_block::BACK as u16,
        };
        let space = crate::native_cpu_lanes::byte_set(&native, 0).expect("a class");
        let word = crate::native_cpu_lanes::byte_set(&native, 1).expect("a class");
        let mut rare = [0u64; 4];
        for b in b"\"#/".iter().copied().chain(b'0'..=b'9').chain(0xc0..=0xff) {
            rare[usize::from(b >> 6)] |= 1 << (b & 63);
        }
        let need = Need { set: rare, care: std::array::from_fn(|k| !space[k]) };
        let b = Bytewise::of(&native);
        let mut module = cranelift_jit::JITModule::new(crate::native_cpu_codegen::tier1_builder().expect("a JIT"));
        let mut sig = module.make_signature();
        sig.params.extend([AbiParam::new(types::I64); 4]);
        let id = module.declare_function("window", Linkage::Local, &sig).expect("declared");
        let mut ctx = module.make_context();
        ctx.func.signature = sig;
        let mut fctx = cranelift_frontend::FunctionBuilderContext::new();
        {
            let mut f = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let entry = f.create_block();
            f.append_block_params_for_function_params(entry);
            f.switch_to_block(entry);
            let p = f.block_params(entry).to_vec();
            let k = crate::native_cpu_block::Consts::make(&mut f);
            let file = File { bytes: p[0], len: p[1], nibble: k.nibble, weights: k.weights, even: k.even };
            let mut heads = Vec::new();
            let (w, more) = b.words_with(&mut f, &file, &[0, 1], &[need], p[2], &mut heads);
            for (i, v) in w.iter().chain(&more).enumerate() {
                f.ins().store(MemFlags::trusted(), *v, p[3], 8 * i as i32);
            }
            f.ins().return_(&[]);
            f.seal_all_blocks();
            f.finalize();
        }
        let swizzles = ctx.func.layout.blocks().flat_map(|bk| ctx.func.layout.block_insts(bk)).filter(|&i| ctx.func.dfg.insts[i].opcode() == Opcode::Swizzle).count();
        assert_eq!(swizzles, 8, "one table's two lookups for each of four 16 bytes:\n{}", ctx.func.display());
        module.define_function(id, &mut ctx).expect("defined");
        module.finalize_definitions().expect("finalized");
        let run: extern "C" fn(*const u8, i64, i64, *mut u64) = unsafe { std::mem::transmute(module.get_finalized_function(id)) };
        let alphabet: Vec<u8> = b" \t\n\r\"#/_09azAZ_.,;-+$\x0b\x0c\x7f\x80\xbf\xc0\xff".to_vec();
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for round in 0..300 {
            let n = 1 + (round % 90);
            // 16 bytes before and 80 after the file, zeros, as a window may read.
            let mut buf = vec![0u8; 16 + n + 80];
            for byte in &mut buf[16..16 + n] {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *byte = alphabet[(x % alphabet.len() as u64) as usize];
            }
            let p = (round % 3).min(n - 1);
            let mut out = [0u64; 3];
            run(buf[16..].as_ptr(), n as i64, p as i64, out.as_mut_ptr());
            for i in 0..64usize {
                let at = p + i;
                let byte = if at < n { buf[16 + at] } else { continue };
                let bit = |w: u64| w >> i & 1 == 1;
                assert_eq!(bit(out[0]), has(&space, byte), "space at {i} ({byte:#04x})");
                assert_eq!(bit(out[1]), has(&word, byte), "word at {i} ({byte:#04x})");
                if !has(&space, byte) {
                    assert_eq!(bit(out[2]), has(&rare, byte), "rare at {i} ({byte:#04x})");
                }
            }
            assert_eq!(out.iter().map(|w| if n - p >= 64 { 0 } else { w >> (n - p) }).fold(0, |a, w| a | w), 0, "none past the file");
        }
    }

    /// A program of a closer `|` that `^` escapes, and tests besides it: a digit, a `~` after an
    /// `@`, and (`blocked`) a `|` after a `#`.
    fn gated_program() -> BlockNative {
        let mut digits = crate::block::ClassTable::default();
        digits.hi[3] = 1;
        for l in 0..=9 {
            digits.lo[l] = 1;
        }
        BlockNative {
            tables: vec![digits],
            // 0 `|`, 1 `^`, 2 escaped by `^`, 3 the closer, 4 a digit, 5 `@`, 6 `~`,
            // 7 `@` one back, 8 `~` after `@`, 9 `#`, 10 `#` one back, 11 `|` after `#`.
            insts: vec![
                BlockInst::EqByte(b'|'),
                BlockInst::EqByte(b'^'),
                BlockInst::Escaped(1),
                BlockInst::AndNot(0, 2),
                BlockInst::Class { table: 0, bits: 1 },
                BlockInst::EqByte(b'@'),
                BlockInst::EqByte(b'~'),
                BlockInst::Prev(5, 1),
                BlockInst::And(7, 6),
                BlockInst::EqByte(b'#'),
                BlockInst::Prev(9, 1),
                BlockInst::And(10, 0),
            ],
            outputs: vec![3, 4, 8, 11],
            extracts: Vec::new(),
            totals: Vec::new(),
            masks: Vec::new(),
            newlines: None,
            delay: 0,
            output_delays: vec![0; 4],
            groups: Vec::new(),
            records: None,
            walks: Vec::new(),
            pad: crate::native_cpu_block::BACK as u16,
        }
    }

    /// Where a window's rare bytes are none, its tests and its escapes are known zero: the gate
    /// is those bytes' values (an escape's run, a test's byte, the byte a test needs besides a
    /// byte before), never one whose zero would make the closer none, and none where a test
    /// is not made zero by any.
    #[test]
    fn a_gate_is_the_rare_bytes_the_tests_read() {
        let native = gated_program();
        let b = Bytewise::of(&native);
        assert_eq!(b.zero_gate(&[0, 1, 2], &[1]), Some(vec![1, 4, 6]), "`^`, a digit, `~`");
        assert_eq!(b.zero_gate(&[0], &[1]), Some(vec![1]), "the escapes alone: `^`");
        assert_eq!(b.zero_gate(&[0], &[]), None, "the escape's carry not given: nothing to gate");
        assert_eq!(b.zero_gate(&[0, 3], &[1]), None, "a `|` after `#`: only `|`'s zero, the closer's, makes it none");
        assert_eq!(b.zero_gate(&[2, 1], &[]), Some(vec![4]), "a digit, the closer `~` after `@`");
        let k = known_if_zero(&native, &[1, 4, 6], &[1]);
        assert_eq!((k.get(&2), k.get(&4), k.get(&8)), (Some(&false), Some(&false), Some(&false)));
        assert_eq!(k.get(&3), None, "the closer, `|` where it is: not known");
        assert_eq!(known_if_zero(&native, &[1], &[]).get(&2), None, "an escape whose carry in may be one");
    }

    /// A short window's masks (`short_words`) are the window's words' first 16 bits, a lane
    /// `short_stride` bits apart (NEON's narrowing shift: four), its lanes counted by
    /// `lanes_set` and cut by `lanes_below`: on a lane output, `prev`s and their logic (four
    /// on NEON), and on an escape's (one: its run's sums read bit positions).
    #[test]
    fn a_short_windows_lanes_are_its_words_bits() {
        use cranelift_codegen::ir::AbiParam;
        use cranelift_module::{Linkage, Module};
        let native = gated_program();
        let b = Bytewise::of(&native);
        let os = [0u16, 1, 2, 3];
        let wide = crate::native_cpu_block::SHORT_STRIDE;
        assert_eq!(b.short_stride(&[1]), wide, "a class's lanes");
        assert_eq!(b.short_stride(&[2, 3]), wide, "`prev`s and their logic");
        assert_eq!(b.short_stride(&[0]), 1, "an escape's run");
        assert_eq!(b.short_stride(&[1, 0]), 1, "a window with an escape's");
        let mut module = cranelift_jit::JITModule::new(crate::native_cpu_codegen::tier1_builder().expect("a JIT"));
        let mut sig = module.make_signature();
        sig.params.extend([AbiParam::new(types::I64); 4]);
        let id = module.declare_function("short", Linkage::Local, &sig).expect("declared");
        let mut ctx = module.make_context();
        ctx.func.signature = sig;
        let mut fctx = cranelift_frontend::FunctionBuilderContext::new();
        {
            let mut f = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let entry = f.create_block();
            f.append_block_params_for_function_params(entry);
            f.switch_to_block(entry);
            let p = f.block_params(entry).to_vec();
            let k = crate::native_cpu_block::Consts::make(&mut f);
            let file = File { bytes: p[0], len: p[1], nibble: k.nibble, weights: k.weights, even: k.even };
            let (at, out) = (p[2], p[3]);
            let mut heads = Vec::new();
            // Each output: its short mask, its word, its lanes set, its first five lanes.
            for (i, &o) in os.iter().enumerate() {
                let st = b.short_stride(&[o]);
                let m = b.short_word(&mut f, &file, o, at, &mut heads);
                let w = b.word(&mut f, &file, o, at, &mut heads);
                let set = crate::native_cpu_block::lanes_set(&mut f, m, st);
                let five = f.ins().iconst(types::I64, 5);
                let below = crate::native_cpu_block::lanes_below(&mut f, five, st, 5);
                let below = f.ins().band(m, below);
                for (j, v) in [m, w, set, below].into_iter().enumerate() {
                    f.ins().store(MemFlags::trusted(), v, out, 8 * (4 * i + j) as i32);
                }
            }
            f.ins().return_(&[]);
            f.seal_all_blocks();
            f.finalize();
        }
        module.define_function(id, &mut ctx).expect("defined");
        module.finalize_definitions().expect("finalized");
        let run: extern "C" fn(*const u8, i64, i64, *mut u64) = unsafe { std::mem::transmute(module.get_finalized_function(id)) };
        let alphabet = b"ab|^5@~#|9";
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for round in 0..300 {
            let n = 1 + (round % 90);
            let mut buf = vec![0u8; 16 + n + 80];
            for byte in &mut buf[16..16 + n] {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *byte = alphabet[(x >> 8) as usize % alphabet.len()];
            }
            let at = (x % n as u64) as i64;
            let mut got = [0u64; 16];
            run(buf[16..].as_ptr(), n as i64, at, got.as_mut_ptr());
            for (i, &o) in os.iter().enumerate() {
                let st = b.short_stride(&[o]);
                let [m, w, set, below] = [got[4 * i], got[4 * i + 1], got[4 * i + 2], got[4 * i + 3]];
                let lane = |v: u64, j: u32| v >> (st * j) & 1;
                for j in 0..16 {
                    assert_eq!(lane(m, j), w >> j & 1, "round {round}, output {o}, lane {j}");
                    if st > 1 {
                        let nib = m >> (st * j) & ((1 << st) - 1);
                        assert!(nib == 0 || nib == (1 << st) - 1, "a lane's bits all alike");
                    }
                }
                if 16 * st < 64 {
                    assert_eq!(m >> (16 * st), 0, "round {round}, output {o}: no bit past the window's 16 lanes");
                }
                assert_eq!(set, u64::from((!w).trailing_zeros().min(16)), "round {round}, output {o}: the lanes set");
                assert_eq!((0..16).filter(|&j| lane(below, j) == 1).count(), (0..5).filter(|&j| w >> j & 1 == 1).count(), "the first five lanes");
            }
        }
    }

    /// An output false on a zero byte (`padded`) is not cut at all, as simdjson's padded
    /// input: its window never reads the file's length, the bytes past it zeros. One true
    /// there keeps its cut. The padding is the compiler's input (`BlockNative::pad`): with less
    /// than a window reaches (`pad_reach`), every window is cut.
    #[test]
    fn a_padded_windows_word_is_not_cut() {
        let reads_len = |b: &Bytewise<'_>, o: u16| {
            let func = b.window_function(o, 0);
            let entry = func.layout.entry_block().expect("an entry");
            let len = func.dfg.block_params(entry)[1];
            func.layout.blocks().flat_map(|bl| func.layout.block_insts(bl)).any(|i| func.dfg.inst_values(i).any(|v| v == len))
        };
        let native = padded_program();
        let reach = pad_reach(&native);
        assert!(reach <= crate::native_cpu_block::BACK, "the default pads every window");
        for (pad, want) in [(crate::native_cpu_block::BACK, [true, true, true, true, false]), (reach, [true, true, true, true, false]), (reach - 1, [false; 5]), (0, [false; 5])] {
            let native = BlockNative { pad: pad as u16, ..native.clone() };
            let b = Bytewise::of(&native);
            assert_eq!((0..5).map(|o| b.padded(o)).collect::<Vec<_>>(), want, "padding {pad}");
            for o in 0..5u16 {
                assert_eq!(reads_len(&b, o), !b.padded(o), "padding {pad}, output {o}");
            }
        }
    }

    /// `gated_program`, and an output true on a zero byte (4: not `|`): its bits past the
    /// file's end are the padding's, cut.
    fn padded_program() -> BlockNative {
        let mut native = gated_program();
        native.insts.push(BlockInst::Not(0));
        native.outputs.push(12);
        native.output_delays.push(0);
        native
    }

    /// A window's cut to the file's end (`cut64`) is its tail's alone, as the lean lexer's
    /// `block` pads only where 64 bytes are not left: a full window's word is its mask, one
    /// compare on its way; the tail's cut in a cold block, no select. Its bits are the file's
    /// bytes' alone, whatever lies past its padding.
    #[test]
    fn a_full_windows_word_is_not_cut() {
        let native = padded_program();
        let b = Bytewise::of(&native);
        // An output true on a zero byte (`padded` not): no `prev`'s cut of its own.
        assert!(!b.padded(4));
        let func = b.window_function(4, 0);
        let dfg = &func.dfg;
        let mut shifts = 0;
        for block in func.layout.blocks() {
            for inst in func.layout.block_insts(block) {
                let op = dfg.insts[inst].opcode();
                assert_ne!(op, cranelift_codegen::ir::Opcode::Select, "no select: {}", func.display());
                if op == cranelift_codegen::ir::Opcode::Ishl {
                    shifts += 1;
                    assert!(func.layout.is_cold(block), "the cut's shift is the tail's: {}", func.display());
                }
            }
        }
        assert_eq!(shifts, 1, "one cut: {}", func.display());
        // Every output's word, whatever the bytes past the program's padding: the default's,
        // and none (the application's choice, every window cut on the rare branch).
        windows_past_the_end(native.clone());
        windows_past_the_end(BlockNative { pad: 0, ..native });
    }

    /// Each output's window of `native` (`window_function`) over a file whose `pad` bytes past
    /// its end are zeros and the rest text: the file's bytes' bits alone.
    fn windows_past_the_end(native: BlockNative) {
        use cranelift_module::{Linkage, Module};
        let b = Bytewise::of(&native);
        let mut module = cranelift_jit::JITModule::new(crate::native_cpu_codegen::tier1_builder().expect("a JIT"));
        let mut runs = Vec::new();
        for o in 0..5u16 {
            let mut ctx = module.make_context();
            let mut func = b.window_function(o, 0);
            let id = module.declare_function(&format!("w{o}"), Linkage::Local, &func.signature).expect("declared");
            func.name = cranelift_codegen::ir::UserFuncName::user(0, id.as_u32());
            ctx.func = func;
            module.define_function(id, &mut ctx).expect("defined");
            runs.push(id);
        }
        module.finalize_definitions().expect("finalized");
        let runs: Vec<extern "C" fn(*const u8, i64, i64) -> u64> = runs.iter().map(|&id| unsafe { std::mem::transmute(module.get_finalized_function(id)) }).collect();
        let alphabet = b"ab|^5@~#|9";
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for round in 0..300 {
            let n = 1 + (round % 150);
            let pad = 16 + n + usize::from(native.pad);
            let (mut zeros, mut text) = (vec![0u8; pad + 160], vec![0u8; pad + 160]);
            for i in 16..zeros.len() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let c = alphabet[(x >> 8) as usize % alphabet.len()];
                if i < 16 + n || i >= pad {
                    text[i] = c;
                }
                if i < 16 + n {
                    zeros[i] = c;
                }
            }
            let p = (x % (n as u64 + 8)) as i64;
            for (o, run) in runs.iter().enumerate() {
                let (z, t) = (run(zeros[16..].as_ptr(), n as i64, p), run(text[16..].as_ptr(), n as i64, p));
                assert_eq!(t, z, "round {round}, output {o}, at {p} of {n}");
                if o == 1 {
                    let want = (0..64).filter(|&j| p + j < n as i64 && zeros[16 + (p + j) as usize].is_ascii_digit()).fold(0u64, |w, j| w | 1 << j);
                    assert_eq!(z, want, "round {round}, the digits at {p} of {n}");
                }
            }
        }
    }

    /// A window's words gated on its rare bytes (`words_gated`) are its words: on each path,
    /// with the carries out, on bytes where the rare ones are some and where they are none.
    #[test]
    fn the_gated_words_are_the_words() {
        gated_words(false);
        gated_words(true);
    }

    /// `the_gated_words_are_the_words` with or without `prefix`.
    fn gated_words(prefix: bool) {
        use cranelift_codegen::ir::AbiParam;
        use cranelift_module::{Linkage, Module};
        let native = gated_program();
        let b = Bytewise::of(&native);
        let os = [0u16, 1, 2];
        assert_eq!(b.carried_runs(&os), vec![1]);
        let mut module = cranelift_jit::JITModule::new(crate::native_cpu_codegen::tier1_builder().expect("a JIT"));
        let mut sig = module.make_signature();
        sig.params.extend([AbiParam::new(types::I64); 5]);
        let id = module.declare_function("gated", Linkage::Local, &sig).expect("declared");
        let mut ctx = module.make_context();
        ctx.func.signature = sig;
        let mut fctx = cranelift_frontend::FunctionBuilderContext::new();
        {
            let mut f = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let entry = f.create_block();
            f.append_block_params_for_function_params(entry);
            f.switch_to_block(entry);
            let p = f.block_params(entry).to_vec();
            let k = crate::native_cpu_block::Consts::make(&mut f);
            let file = File { bytes: p[0], len: p[1], nibble: k.nibble, weights: k.weights, even: k.even };
            let mut heads = Vec::new();
            let (out, cin) = (p[3], p[4]);
            let (w, o) = b.words_carried(&mut f, &file, &os, p[2], &[(1, cin)], &mut heads);
            for (i, v) in w.iter().chain(&o).enumerate() {
                f.ins().store(MemFlags::trusted(), *v, out, 8 * i as i32);
            }
            let mut path = 0i64;
            b.words_gated(&mut f, &file, &os, p[2], &[(1, cin)], prefix, &mut heads, &mut |f, w, o, _| {
                path += 1;
                for (i, v) in w.iter().chain(o).enumerate() {
                    f.ins().store(MemFlags::trusted(), *v, out, 8 * (4 + i) as i32);
                }
                let n = f.ins().iconst(types::I64, path);
                f.ins().store(MemFlags::trusted(), n, out, 64);
                f.ins().return_(&[]);
            });
            assert_eq!(path, 2, "a quick path and a whole one");
            f.seal_all_blocks();
            f.finalize();
        }
        // The escape's carry out (its add's overflow, `icmp ult`): made on the whole path and
        // by the words not gated, not on the quick one.
        let overflows = ctx.func.layout.blocks().flat_map(|bk| ctx.func.layout.block_insts(bk)).filter(|&i| matches!(ctx.func.dfg.insts[i], cranelift_codegen::ir::InstructionData::IntCompare { cond: IntCC::UnsignedLessThan, .. })).count();
        assert_eq!(overflows, 2, "prefix {prefix}: {}", ctx.func.display());
        module.define_function(id, &mut ctx).expect("defined");
        module.finalize_definitions().expect("finalized");
        let run: extern "C" fn(*const u8, i64, i64, *mut u64, i64) = unsafe { std::mem::transmute(module.get_finalized_function(id)) };
        let (common, rare) = (b"ab |c ".as_slice(), b"^5~@#".as_slice());
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut paths = [0usize; 3];
        for round in 0..400 {
            let n = 1 + (round % 120);
            let mut buf = vec![0u8; 16 + n + 80];
            for byte in &mut buf[16..16 + n] {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                // Rare bytes in a round in four, one a 16 there.
                *byte = if round % 4 == 0 && x.is_multiple_of(16) { rare[(x >> 8) as usize % rare.len()] } else { common[(x >> 8) as usize % common.len()] };
            }
            let p = (round % 5).min(n - 1);
            let mut out = [0u64; 9];
            run(buf[16..].as_ptr(), n as i64, p as i64, out.as_mut_ptr(), (round / 7 % 2) as i64);
            if prefix && out[8] == 1 {
                // The quick path: the words up to the closer's first bit, no carry out.
                let below = out[0] ^ out[0].wrapping_sub(1);
                let cut: Vec<u64> = out[..3].iter().map(|w| w & below).collect();
                let quick: Vec<u64> = out[4..7].iter().map(|w| w & below).collect();
                assert_eq!(cut, quick, "round {round}: the words up to the closer, gated and not");
            } else {
                assert_eq!(out[..4], out[4..8], "round {round}: words and carry out, gated and not (prefix {prefix})");
            }
            paths[out[8] as usize] += 1;
        }
        assert!(paths[1] > 0 && paths[2] > 0, "both paths taken: {paths:?}");
    }
}
