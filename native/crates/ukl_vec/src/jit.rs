//! A kernel's code tape compiled by Cranelift (decision 0567 §6, step 3): each instruction of a section (the
//! kernel's, a body's) becomes its handler's work inline, in 128-bit vectors (4 lanes of 32 bits, 2 of 64), with
//! nothing left to decode or dispatch.
//!
//! - **The state is the interpreter's** (`run::Sg`'s files, in memory), so a SIMD group's state is one whichever
//!   runs it. Within a block, a value stored to a file is taken from the register it was computed in, not loaded
//!   again (`Gen::known`); the stores still go to memory, which a stop and an interpreter's step read.
//! - **A section stops** at a barrier, a poll, the end, and a call of a section that can stop (or its return):
//!   `run`'s driver does there what the interpreter does (the call stack, the windows, the turns), then enters the
//!   next section at its resume point, through the jump table at its entry. A call of a section that cannot stop is
//!   direct, compiled code calling compiled code, through the dispatch's table of entries.
//! - **An instruction the compiled code does not do itself** (an atomic, a shuffle, a reduction, a generic opcode, a
//!   division) is one step of the interpreter, called with the SIMD group: the same code, the same results.
//! - **The bytes are the code:** it holds no address of its process (the interpreter's step and the entries are
//!   the dispatch's, its constants and jump tables are its own), so the device keeps it in its store, as every
//!   backend keeps its compiled form there, and a process loads it rather than compiling it (`compile`, `load`).

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::types::{F32, F32X4, F64, F64X2, I8, I16, I32, I32X4, I64, I64X2};
use cranelift_codegen::ir::{AbiParam, Block, ConstantData, InstBuilder, JumpTableData, MemFlags, Signature, Type, UserFuncName, Value};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::Module;

use crate::Error;
use crate::code::{CUR, I, WORDS};
use crate::k::{M, Op};
use crate::run::W;

/// A section's entry: the dispatch's state, the SIMD group, its files' windows, its frame, and the section's resume
/// point to start at. It returns 0 at a stop (the SIMD group's `pc` past the instruction that stopped it), or 1 when
/// an interpreter's step failed (its error kept in the dispatch's state).
pub type Entry = unsafe extern "C" fn(cx: *const u8, sg: *mut u8, s: *mut u64, v32: *mut u32, v64: *mut u64, mk: *mut u32, fp: u64, resume: u64) -> u64;

/// An interpreter's step of the instruction at `pc` for the SIMD group: 0, or 1 with its error kept.
pub type Step = unsafe extern "C" fn(cx: *const u8, sg: *mut u8, pc: u64) -> u64;

/// A kernel's compiled sections.
pub struct Jit {
    _module: cranelift_jit::JITModule,
    entries: Vec<Entry>,
    /// Each instruction's section and its resume point there (`u32::MAX` where none).
    resume: Vec<(u32, u32)>,
}

// SAFETY: the module's code is finalized and never written again; the entries are plain functions.
unsafe impl Send for Jit {}
// SAFETY: the same.
unsafe impl Sync for Jit {}

impl Jit {
    /// Each section's entry, in order: what a direct call reaches its callee by.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The section and its entry's resume point of instruction `pc`, one a section stops before.
    pub fn at(&self, pc: usize) -> Option<(Entry, u64)> {
        let (s, r) = *self.resume.get(pc)?;
        (r != u32::MAX).then(|| (self.entries[s as usize], u64::from(r)))
    }
}

/// The opcode of an instruction's first word, which `compile` checked first.
fn op_of(w: u32) -> I {
    // SAFETY: `compile` checks every instruction's opcode before it reads one (`I::LAST`).
    unsafe { I::of(w) }
}

/// Whether the compiled code stops at the instruction: `run`'s driver does it.
fn stops(i: I) -> bool {
    matches!(i, I::Halt | I::Call | I::Ret | I::Bar | I::Yield)
}

/// Where the compiled code reads and writes the interpreter's state: the SIMD group's instruction index
/// (`run::Sg::pc`), and the dispatch's threadgroup memory's and constant memory's addresses and its interpreter's
/// step (`run::Cx`'s). The code holds no address of its own process, so its bytes are kept (`compile`, `load`).
#[derive(Clone, Copy)]
pub struct Places {
    pub pc: i32,
    pub shared: i32,
    pub data: i32,
    pub step: i32,
    /// The SIMD group's frame and windows (`run::Sg::fp`, `win`), which a direct call sets for its callee's steps,
    /// and the dispatch's table of the sections' entries (`run::Cx::entries`).
    pub fp: i32,
    pub win: i32,
    pub entries: i32,
}

/// The kept form's first bytes, and its version: a change to the layout below changes it.
const KEPT: &[u8; 8] = b"UKLJIT01";

/// The host's target and the features its code is compiled for: what a kept form's key names.
pub fn host() -> String {
    match isa() {
        Ok(isa) => format!("{} {}", isa.triple(), isa.isa_flags().iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")),
        Err(e) => e.0,
    }
}

fn isa() -> Result<cranelift_codegen::isa::OwnedTargetIsa, Error> {
    use cranelift_codegen::settings::Configurable;
    let mut flags = cranelift_codegen::settings::builder();
    for (k, v) in [("opt_level", "speed"), ("enable_verifier", "false")] {
        flags.set(k, v).map_err(|e| Error(format!("the JIT's flags: {e}")))?;
    }
    cranelift_native::builder()
        .map_err(|e| Error(format!("the JIT's target: {e}")))?
        .finish(cranelift_codegen::settings::Flags::new(flags))
        .map_err(|e| Error(format!("the JIT's target: {e}")))
}

/// The code's sections (the kernel's at 0, a body's at each call's target), each with its resume points: its start,
/// and past each instruction it stops at.
fn sections(code: &[u32]) -> Result<Vec<(usize, usize, Vec<usize>)>, Error> {
    let n = code.len() / WORDS;
    let at = |i: usize, w: usize| code[i * WORDS + w];
    if let Some(i) = (0..n).find(|&i| (at(i, 0) & 0xff) as u8 > I::LAST) {
        return Err(Error(format!("the code's instruction {i} has no opcode ({:#x})", at(i, 0))));
    }
    let mut starts: Vec<usize> = (0..n).filter(|&i| op_of(at(i, 0)) == I::Call).map(|i| at(i, 1) as usize).collect();
    starts.push(0);
    starts.sort_unstable();
    starts.dedup();
    if starts.last().is_some_and(|&s| s >= n) {
        return Err(Error("a call's target past the code".into()));
    }
    Ok(starts
        .iter()
        .enumerate()
        .map(|(k, &lo)| {
            let hi = starts.get(k + 1).copied().unwrap_or(n);
            let mut points = vec![lo];
            points.extend((lo..hi).filter(|&i| matches!(op_of(at(i, 0)), I::Call | I::Bar | I::Yield) && i + 1 < hi).map(|i| i + 1));
            (lo, hi, points)
        })
        .collect())
}

/// A module for the sections, and their signature.
fn module() -> Result<(cranelift_jit::JITModule, Signature), Error> {
    let module = cranelift_jit::JITModule::new(cranelift_jit::JITBuilder::with_isa(isa()?, cranelift_module::default_libcall_names()));
    let ptr = module.target_config().pointer_type();
    let mut sig = Signature::new(module.isa().default_call_conv());
    for _ in 0..8 {
        sig.params.push(AbiParam::new(ptr));
    }
    sig.returns.push(AbiParam::new(I64));
    Ok((module, sig))
}

/// The sections' functions, finalized, as the compiled form.
fn finish(mut module: cranelift_jit::JITModule, ids: &[cranelift_module::FuncId], secs: &[(usize, usize, Vec<usize>)], n: usize) -> Result<Jit, Error> {
    module.finalize_definitions().map_err(|e| Error(format!("the JIT: {e}")))?;
    // SAFETY: each is a finalized function of the sections' signature, which `Entry` is.
    let entries = ids.iter().map(|&id| unsafe { std::mem::transmute::<*const u8, Entry>(module.get_finalized_function(id)) }).collect();
    let mut resume = vec![(0u32, u32::MAX); n];
    for (k, (_, _, points)) in secs.iter().enumerate() {
        for (r, &p) in points.iter().enumerate() {
            resume[p] = (k as u32, r as u32);
        }
    }
    Ok(Jit { _module: module, entries, resume })
}

/// Kernel `name`'s `code` (its instructions), compiled; and its kept form (`load`'s), each section's code, when none
/// of it needs relocating (else `None`).
pub fn compile(name: &str, code: &[u32], table: &[u32], places: Places) -> Result<(Jit, Option<Vec<u8>>), Error> {
    let n = code.len() / WORDS;
    let secs = sections(code)?;
    // The sections that can stop (a barrier, a poll, the end, or a call of one that can): a call of any other is
    // direct, compiled code calling compiled code; a call of one that can stops, for `run`'s driver to make.
    let of_start: std::collections::HashMap<usize, usize> = secs.iter().enumerate().map(|(k, s)| (s.0, k)).collect();
    let mut can_stop: Vec<bool> = secs.iter().map(|&(lo, hi, _)| (lo..hi).any(|i| matches!(op_of(code[i * WORDS]), I::Bar | I::Yield | I::Halt))).collect();
    loop {
        let mut more = false;
        for (k, &(lo, hi, _)) in secs.iter().enumerate() {
            if !can_stop[k] && (lo..hi).any(|i| op_of(code[i * WORDS]) == I::Call && can_stop[of_start[&(code[i * WORDS + 1] as usize)]]) {
                (can_stop[k], more) = (true, true);
            }
        }
        if !more {
            break;
        }
    }
    let calls = Calls { of_start: &of_start, can_stop: &can_stop, table };
    let (mut module, sig) = module()?;
    let ptr = module.target_config().pointer_type();
    let mut step_sig = Signature::new(module.isa().default_call_conv());
    for _ in 0..3 {
        step_sig.params.push(AbiParam::new(ptr));
    }
    step_sig.returns.push(AbiParam::new(I64));
    let mut kept = Some(KEPT.to_vec());
    kept.as_mut().expect("the kept form").extend((secs.len() as u32).to_le_bytes());
    let mut ids = Vec::new();
    let mut fctx = FunctionBuilderContext::new();
    // A measurement's: each instruction's code, by address (`RERAIL_UKL_VEC_JIT_MAP`, a file to append to).
    let map = std::env::var("RERAIL_UKL_VEC_JIT_MAP").ok();
    let mut spans: Vec<(usize, u32, u32, u32)> = Vec::new();
    let mut ctx = module.make_context();
    for (k, (lo, hi, points)) in secs.iter().enumerate() {
        let id = module.declare_function(&format!("section{k}"), cranelift_module::Linkage::Local, &sig).map_err(|e| Error(format!("the JIT: {e}")))?;
        ctx.func.signature = sig.clone();
        ctx.func.name = UserFuncName::user(0, id.as_u32());
        {
            let b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let mut g = Gen::new(b, code, *lo, *hi, points, &step_sig, &sig, places, calls)?;
            g.section()?;
            g.b.seal_all_blocks();
            g.b.finalize();
        }
        module.define_function(id, &mut ctx).map_err(|e| Error(format!("the JIT, section {k} at {lo}: {e:?}")))?;
        let cc = ctx.compiled_code().expect("a compiled function");
        if map.is_some() {
            for l in cc.buffer.get_srclocs_sorted() {
                if !l.loc.is_default() {
                    spans.push((k, l.start, l.end, l.loc.bits()));
                }
            }
        }
        if !cc.buffer.relocs().is_empty() {
            kept = None;
        }
        if let Some(out) = &mut kept {
            let bytes = cc.code_buffer();
            out.extend(cc.buffer.alignment.to_le_bytes());
            out.extend((bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        module.clear_context(&mut ctx);
        ids.push(id);
    }
    let jit = finish(module, &ids, &secs, n)?;
    if let Some(path) = map {
        use std::io::Write;
        let mut out = String::new();
        for (k, a, b, i) in spans {
            let base = jit.entries[k] as usize;
            let w = &code[i as usize * WORDS..][..WORDS];
            out.push_str(&format!("{:x} {:x} {:?} {name}:{i} {} {} {} {} {}\n", base + a as usize, base + b as usize, op_of(w[0]), w[0] >> 8, w[1], w[2], w[3], w[4]));
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(out.as_bytes());
        }
    }
    Ok((jit, kept))
}

/// The compiled form of `code` from its kept form (`compile`'s), made on this host (`host`): `None` if the bytes are
/// not one of `code`'s sections.
pub fn load(code: &[u32], bytes: &[u8]) -> Option<Jit> {
    let secs = sections(code).ok()?;
    let mut at = bytes.strip_prefix(KEPT.as_slice())?;
    fn take<'x>(at: &mut &'x [u8], n: usize) -> Option<&'x [u8]> {
        let (a, rest) = (at.get(..n)?, at.get(n..)?);
        *at = rest;
        Some(a)
    }
    let word = |at: &mut &[u8]| take(at, 4).map(|w| u32::from_le_bytes(w.try_into().expect("four bytes")));
    if word(&mut at)? as usize != secs.len() {
        return None;
    }
    let (mut module, sig) = module().ok()?;
    let mut ids = Vec::new();
    for k in 0..secs.len() {
        let (align, len) = (word(&mut at)?, word(&mut at)? as usize);
        let body = take(&mut at, len)?;
        let id = module.declare_function(&format!("section{k}"), cranelift_module::Linkage::Local, &sig).ok()?;
        module.define_function_bytes(id, u64::from(align), body, &[]).ok()?;
        ids.push(id);
    }
    if !at.is_empty() {
        return None;
    }
    finish(module, &ids, &secs, code.len() / WORDS).ok()
}

/// The bytes of a 32-bit register (32 lanes), and of a 64-bit one.
const R32: i32 = (W * 4) as i32;
const R64: i32 = (W * 8) as i32;

/// What a call needs of the other sections: each one's index by its start, whether it can stop, and the table of
/// what a call from each function advances by (`code::TABLE`).
#[derive(Clone, Copy)]
struct Calls<'a> {
    of_start: &'a std::collections::HashMap<usize, usize>,
    can_stop: &'a [bool],
    table: &'a [u32],
}

struct Gen<'a, 'b> {
    b: FunctionBuilder<'b>,
    code: &'a [u32],
    lo: usize,
    hi: usize,
    /// The block of each instruction that starts one.
    blocks: Vec<Option<Block>>,
    fail: Block,
    step_sig: cranelift_codegen::ir::SigRef,
    places: Places,
    calls: Calls<'a>,
    entry_sig: cranelift_codegen::ir::SigRef,
    cx: Value,
    sg: Value,
    s: Value,
    v32: Value,
    v64: Value,
    mk: Value,
    fp: Value,
    /// The values the block has stored to (or loaded from) the files, by file and place: a later load in the block
    /// takes the value. The stores still go to memory, so a stop and an interpreter's step see the files as they are;
    /// a new block, or a step, starts it again.
    known: std::collections::HashMap<(u8, i32), Value>,
    /// The lanes of a mask value in a chunk (`lanes4`, `lanes2`), by the value, the chunk and the lanes' width.
    lanes_of: std::collections::HashMap<(Value, usize, bool), Value>,
}

/// The files, as `Gen::known` keys them.
const FS: u8 = 0;
const FV32: u8 = 1;
const FV64: u8 = 2;
const FM: u8 = 3;

fn mf() -> MemFlags {
    MemFlags::new().with_notrap()
}

impl<'a, 'b> Gen<'a, 'b> {
    #[allow(clippy::too_many_arguments)]
    fn new(mut b: FunctionBuilder<'b>, code: &'a [u32], lo: usize, hi: usize, points: &[usize], step_sig: &Signature, sig: &Signature, places: Places, calls: Calls<'a>) -> Result<Self, Error> {
        let at = |i: usize, w: usize| code[i * WORDS + w];
        // The blocks: at each resume point, each jump's target, and past each jump and stop.
        let mut lead = vec![false; hi - lo];
        for &p in points {
            lead[p - lo] = true;
        }
        for i in lo..hi {
            let op = op_of(at(i, 0));
            let (targets, past): (Vec<usize>, usize) = match op {
                I::Jmp => (vec![at(i, 1) as usize], i + 1),
                I::Jzm | I::Jzs | I::Jnzs => (vec![at(i, 2) as usize], i + 1),
                // A switch's labels follow it: the code goes on past them.
                I::Switch => {
                    let n = at(i, 2) as usize;
                    let mut t: Vec<usize> = (0..n).map(|e| at(i + 1 + e, 3) as usize).collect();
                    t.push(at(i, 3) as usize);
                    (t, i + 1 + n)
                }
                _ if stops(op) => (vec![], i + 1),
                _ => continue,
            };
            for t in targets {
                if !(lo..hi).contains(&t) {
                    return Err(Error(format!("the code's instruction {i} jumps to {t}, out of its section {lo}..{hi}")));
                }
                lead[t - lo] = true;
            }
            if past < hi {
                lead[past - lo] = true;
            }
        }
        let blocks = lead.iter().map(|&l| l.then(|| b.create_block())).collect();
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let ps = b.block_params(entry).to_vec();
        let fail = b.create_block();
        let step_sig = b.import_signature(step_sig.clone());
        let entry_sig = b.import_signature(sig.clone());
        let mut g = Gen { b, code, lo, hi, blocks, fail, step_sig, places, calls, entry_sig, cx: ps[0], sg: ps[1], s: ps[2], v32: ps[3], v64: ps[4], mk: ps[5], fp: ps[6], known: Default::default(), lanes_of: Default::default() };
        // The entry: to the resume point asked for.
        let calls: Vec<_> = points.iter().map(|&p| g.call_of(p)).collect();
        let bad = g.b.func.dfg.block_call(fail, &[]);
        let jt = g.b.create_jump_table(JumpTableData::new(bad, &calls));
        let r = g.b.ins().ireduce(I32, ps[7]);
        g.b.ins().br_table(r, jt);
        g.b.switch_to_block(fail);
        let one = g.b.ins().iconst(I64, 1);
        g.b.ins().return_(&[one]);
        Ok(g)
    }

    fn block(&self, i: usize) -> Block {
        self.blocks[i - self.lo].expect("a block's instruction")
    }

    fn call_of(&mut self, i: usize) -> cranelift_codegen::ir::BlockCall {
        let blk = self.block(i);
        self.b.func.dfg.block_call(blk, &[])
    }

    fn w(&self, i: usize, k: usize) -> u32 {
        self.code[i * WORDS + k]
    }

    fn section(&mut self) -> Result<(), Error> {
        let mut open = false;
        for i in self.lo..self.hi {
            if let Some(blk) = self.blocks[i - self.lo] {
                if open {
                    self.b.ins().jump(blk, &[]);
                }
                self.enter(blk);
                open = true;
            }
            if !open {
                // Unreachable: past a jump or stop, and no block starts here.
                continue;
            }
            open = self.inst(i)?;
        }
        if open {
            // A section ends at its `Halt` or `Ret`; code that falls off it is the code's error.
            self.b.ins().jump(self.fail, &[]);
        }
        Ok(())
    }

    /// Block `blk` from here: what the block before knew is forgotten.
    fn enter(&mut self, blk: Block) {
        self.b.switch_to_block(blk);
        self.known.clear();
        self.lanes_of.clear();
    }

    fn base(&self, f: u8) -> Value {
        [self.s, self.v32, self.v64, self.mk][f as usize]
    }

    /// The value of type `t` at `off` in file `f`: the block's, else loaded.
    fn ld(&mut self, f: u8, t: Type, off: i32) -> Value {
        if let Some(&x) = self.known.get(&(f, off)) {
            let have = self.b.func.dfg.value_type(x);
            if have == t {
                return x;
            }
            if have.bits() == 128 && t.bits() == 128 {
                let flags = MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little);
                return self.b.ins().bitcast(t, flags, x);
            }
        }
        let p = self.base(f);
        let x = self.b.ins().load(t, mf(), p, off);
        self.known.insert((f, off), x);
        x
    }

    fn st(&mut self, f: u8, off: i32, x: Value) {
        let p = self.base(f);
        self.b.ins().store(mf(), x, p, off);
        self.known.insert((f, off), x);
    }

    /// What the block knows of register `r` of a vector file (`bytes` a register) forgotten: lanes written apart.
    fn forget(&mut self, f: u8, r: u32, bytes: i32) {
        let (lo, hi) = (r as i32 * bytes, (r as i32 + 1) * bytes);
        self.known.retain(|&(g, o), _| g != f || !(lo..hi).contains(&o));
    }

    fn ld_s(&mut self, r: u32) -> Value {
        self.ld(FS, I64, (r * 8) as i32)
    }

    fn st_s(&mut self, r: u32, x: Value) {
        self.st(FS, (r * 8) as i32, x);
    }

    fn ld_m(&mut self, r: u32) -> Value {
        self.ld(FM, I32, (r * 4) as i32)
    }

    fn st_m(&mut self, r: u32, x: Value) {
        self.st(FM, (r * 4) as i32, x);
    }

    /// Chunk `k` of a 32-bit register (lanes `4k..4k+4`), as `t` (I32X4 or F32X4).
    fn ld32(&mut self, r: u32, k: usize, t: Type) -> Value {
        self.ld(FV32, t, r as i32 * R32 + 16 * k as i32)
    }

    fn st32(&mut self, r: u32, k: usize, x: Value) {
        self.st(FV32, r as i32 * R32 + 16 * k as i32, x);
    }

    /// Chunk `k` of a 64-bit register (lanes `2k, 2k+1`).
    fn ld64(&mut self, r: u32, k: usize, t: Type) -> Value {
        self.ld(FV64, t, r as i32 * R64 + 16 * k as i32)
    }

    fn st64(&mut self, r: u32, k: usize, x: Value) {
        self.st(FV64, r as i32 * R64 + 16 * k as i32, x);
    }

    fn vconst(&mut self, t: Type, bytes: Vec<u8>) -> Value {
        let h = self.b.func.dfg.constants.insert(ConstantData::from(bytes));
        self.b.ins().vconst(t, h)
    }

    fn c32x4(&mut self, x: [u32; 4]) -> Value {
        self.vconst(I32X4, x.iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    fn c64x2(&mut self, x: [u64; 2]) -> Value {
        self.vconst(I64X2, x.iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    /// The lanes of `cur` in chunk `k` of 4 lanes, each all ones or zeros: `cur` splatted once, then its bits.
    fn lanes4(&mut self, cur: Value, k: usize) -> Value {
        if let Some(&x) = self.lanes_of.get(&(cur, k, false)) {
            return x;
        }
        let v = match self.lanes_of.get(&(cur, usize::MAX, false)) {
            Some(&v) => v,
            None => {
                let v = self.b.ins().splat(I32X4, cur);
                self.lanes_of.insert((cur, usize::MAX, false), v);
                v
            }
        };
        let l = 4 * k as u32;
        let sel = self.c32x4([1 << l, 1 << (l + 1), 1 << (l + 2), 1 << (l + 3)]);
        let x = self.b.ins().band(v, sel);
        let x = self.b.ins().icmp(IntCC::Equal, x, sel);
        self.lanes_of.insert((cur, k, false), x);
        x
    }

    /// The lanes of `cur` in chunk `k` of 2 lanes.
    fn lanes2(&mut self, cur: Value, k: usize) -> Value {
        if let Some(&x) = self.lanes_of.get(&(cur, k, true)) {
            return x;
        }
        let v = match self.lanes_of.get(&(cur, usize::MAX, true)) {
            Some(&v) => v,
            None => {
                let wide = self.b.ins().uextend(I64, cur);
                let v = self.b.ins().splat(I64X2, wide);
                self.lanes_of.insert((cur, usize::MAX, true), v);
                v
            }
        };
        let l = 2 * k as u64;
        let sel = self.c64x2([1 << l, 1 << (l + 1)]);
        let x = self.b.ins().band(v, sel);
        let x = self.b.ins().icmp(IntCC::Equal, x, sel);
        self.lanes_of.insert((cur, k, true), x);
        x
    }

    /// `full()` if all 32 lanes run, else `part()`, then on: what the block knew before is known after (the two
    /// write only what `forget` names, besides memory outside the files).
    fn if_full(&mut self, forget: Option<(u8, u32, i32)>, full: impl FnOnce(&mut Self), part: impl FnOnce(&mut Self)) {
        let cur = self.cur();
        let all = self.b.ins().icmp_imm(IntCC::Equal, cur, -1);
        let (f, p, join) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(all, f, &[], p, &[]);
        let (known, lanes) = (self.known.clone(), self.lanes_of.clone());
        self.b.switch_to_block(f);
        full(self);
        self.b.ins().jump(join, &[]);
        (self.known, self.lanes_of) = (known.clone(), lanes.clone());
        self.b.switch_to_block(p);
        part(self);
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(join);
        (self.known, self.lanes_of) = (known, lanes);
        if let Some((file, r, bytes)) = forget {
            self.forget(file, r, bytes);
        }
    }

    /// Vector register `c`'s running lanes stored to the variable at `at` (32 values of 4 bytes, or of 8 if `wide`).
    fn blend_store(&mut self, at: Value, c: u32, wide: bool) {
        let cur = self.cur();
        if wide {
            for k in 0..W / 2 {
                let sel = self.lanes2(cur, k);
                let x = self.ld64(c, k, I64X2);
                let o = self.b.ins().load(I64X2, mf(), at, 16 * k as i32);
                let r = self.b.ins().bitselect(sel, x, o);
                self.b.ins().store(mf(), r, at, 16 * k as i32);
            }
        } else {
            for k in 0..W / 4 {
                let sel = self.lanes4(cur, k);
                let x = self.ld32(c, k, I32X4);
                let o = self.b.ins().load(I32X4, mf(), at, 16 * k as i32);
                let r = self.b.ins().bitselect(sel, x, o);
                self.b.ins().store(mf(), r, at, 16 * k as i32);
            }
        }
    }

    /// `n` bytes (at most 64) from `from` to `to`, as a move: every load before the stores.
    fn copy_bytes(&mut self, to: Value, from: Value, n: u32) {
        let mut parts = Vec::new();
        let mut at = 0u32;
        for (size, t) in [(16u32, I32X4), (8, I64), (4, I32), (2, cranelift_codegen::ir::types::I16), (1, cranelift_codegen::ir::types::I8)] {
            while n - at >= size {
                parts.push((at as i32, self.b.ins().load(t, mf(), from, at as i32)));
                at += size;
            }
        }
        for (o, x) in parts {
            self.b.ins().store(mf(), x, to, o);
        }
    }

    /// The lanes of 32-bit register `r` that are not 0, as a mask.
    fn tomask(&mut self, r: u32) -> Value {
        let mut m = self.b.ins().iconst(I32, 0);
        let zero = self.c32x4([0; 4]);
        for k in 0..W / 4 {
            let x = self.ld32(r, k, I32X4);
            let t = self.b.ins().icmp(IntCC::NotEqual, x, zero);
            let bits = self.b.ins().vhigh_bits(I32, t);
            let bits = if k == 0 { bits } else { self.b.ins().ishl_imm(bits, 4 * k as i64) };
            m = self.b.ins().bor(m, bits);
        }
        m
    }

    fn cur(&mut self) -> Value {
        self.ld_m(CUR)
    }

    /// 32-bit lanes: `o = f(x, y)` a chunk at a time.
    fn vbin32(&mut self, i: usize, t: Type, f: impl Fn(&mut FunctionBuilder, Value, Value) -> Value) {
        let (a, x, y) = (self.w(i, 1), self.w(i, 2), self.w(i, 3));
        for k in 0..W / 4 {
            let (p, q) = (self.ld32(x, k, t), self.ld32(y, k, t));
            let o = f(&mut self.b, p, q);
            self.st32(a, k, o);
        }
    }

    fn vbin64(&mut self, i: usize, t: Type, f: impl Fn(&mut FunctionBuilder, Value, Value) -> Value) {
        let (a, x, y) = (self.w(i, 1), self.w(i, 2), self.w(i, 3));
        for k in 0..W / 2 {
            let (p, q) = (self.ld64(x, k, t), self.ld64(y, k, t));
            let o = f(&mut self.b, p, q);
            self.st64(a, k, o);
        }
    }

    fn vun32(&mut self, i: usize, f: impl Fn(&mut FunctionBuilder, Value) -> Value) {
        let (a, x) = (self.w(i, 1), self.w(i, 2));
        for k in 0..W / 4 {
            let p = self.ld32(x, k, I32X4);
            let o = f(&mut self.b, p);
            self.st32(a, k, o);
        }
    }

    fn vun64(&mut self, i: usize, f: impl Fn(&mut FunctionBuilder, Value) -> Value) {
        let (a, x) = (self.w(i, 1), self.w(i, 2));
        for k in 0..W / 2 {
            let p = self.ld64(x, k, I64X2);
            let o = f(&mut self.b, p);
            self.st64(a, k, o);
        }
    }

    /// Lane by lane, as scalars: `o[l] = f(x[l], y[l])`, the operands of `tx` (I32, I64, F32, F64) from their file,
    /// the result of `to` (I32 or I64) to its.
    fn lanes(&mut self, i: usize, tx: Type, to: Type, f: impl Fn(&mut FunctionBuilder, Value, Value) -> Value) {
        let (a, x, y) = (self.w(i, 1), self.w(i, 2), self.w(i, 3));
        let (xf, xs) = if tx.bytes() == 8 { (self.v64, R64) } else { (self.v32, R32) };
        let (of, os) = if to.bytes() == 8 { (self.v64, R64) } else { (self.v32, R32) };
        self.forget(if to.bytes() == 8 { FV64 } else { FV32 }, a, os);
        for l in 0..W as i32 {
            let n = tx.bytes() as i32;
            let p = self.b.ins().load(tx, mf(), xf, x as i32 * xs + l * n);
            let q = if y == x { p } else { self.b.ins().load(tx, mf(), xf, y as i32 * xs + l * n) };
            let o = f(&mut self.b, p, q);
            self.b.ins().store(mf(), o, of, a as i32 * os + l * to.bytes() as i32);
        }
    }

    /// `x` as type `m` holds it (`ops::norm`).
    fn norm(&mut self, m: M, x: Value) -> Value {
        let ins = self.b.ins();
        match m {
            M::B => {
                let t = ins.icmp_imm(IntCC::NotEqual, x, 0);
                self.b.ins().uextend(I64, t)
            }
            M::I8 | M::I16 | M::I32 => {
                let t = [I8, I16, I32][[M::I8, M::I16, M::I32].iter().position(|&n| n == m).unwrap_or(0)];
                let n = ins.ireduce(t, x);
                self.b.ins().sextend(I64, n)
            }
            M::U8 => ins.band_imm(x, 0xff),
            M::U16 => ins.band_imm(x, 0xffff),
            M::U32 | M::F32 => ins.band_imm(x, 0xffff_ffff),
            _ => x,
        }
    }

    /// The value of `m` at `p + off` (`ops::load`).
    fn load_m(&mut self, m: M, p: Value, off: i32) -> Value {
        let x = match m.bytes() {
            1 => self.b.ins().uload8(I64, mf(), p, off),
            2 => self.b.ins().uload16(I64, mf(), p, off),
            4 => self.b.ins().uload32(mf(), p, off),
            _ => return self.b.ins().load(I64, mf(), p, off),
        };
        self.norm(m, x)
    }

    /// `v` of `m` stored at `p + off` (`ops::store`).
    fn store_m(&mut self, m: M, p: Value, off: i32, v: Value) {
        match m.bytes() {
            1 => {
                let v = self.norm(m, v);
                self.b.ins().istore8(mf(), v, p, off)
            }
            2 => self.b.ins().istore16(mf(), v, p, off),
            4 => self.b.ins().istore32(mf(), v, p, off),
            _ => self.b.ins().store(mf(), v, p, off),
        };
    }

    /// A float of `m`'s bits in `x`, and back.
    fn float_of(&mut self, m: M, x: Value) -> Value {
        if m == M::F32 {
            let n = self.b.ins().ireduce(I32, x);
            self.b.ins().bitcast(F32, MemFlags::new(), n)
        } else {
            self.b.ins().bitcast(F64, MemFlags::new(), x)
        }
    }

    fn bits_of(&mut self, f: Value) -> Value {
        if self.b.func.dfg.value_type(f) == F32 {
            let n = self.b.ins().bitcast(I32, MemFlags::new(), f);
            self.b.ins().uextend(I64, n)
        } else {
            self.b.ins().bitcast(I64, MemFlags::new(), f)
        }
    }

    /// Whether `x` of `m` is not zero, as 0 or 1 (`ops::truth`).
    fn truth(&mut self, m: M, x: Value) -> Value {
        let t = if m.float() {
            let f = self.float_of(m, x);
            let z = if m == M::F32 { self.b.ins().f32const(0.0) } else { self.b.ins().f64const(0.0) };
            self.b.ins().fcmp(FloatCC::NotEqual, f, z)
        } else {
            let n = self.norm(m, x);
            self.b.ins().icmp_imm(IntCC::NotEqual, n, 0)
        };
        self.b.ins().uextend(I64, t)
    }

    /// `a op b` on values of `m` (`ops::bin`), or `None` for what the interpreter does (a division, which can fail).
    fn sbin(&mut self, op: Op, m: M, a: Value, b: Value) -> Option<Value> {
        if m.float() {
            if !matches!(m, M::F32 | M::F64) {
                return None;
            }
            let (x, y) = (self.float_of(m, a), self.float_of(m, b));
            let ins = self.b.ins();
            let r = match op {
                Op::Add => ins.fadd(x, y),
                Op::Sub => ins.fsub(x, y),
                Op::Mul => ins.fmul(x, y),
                Op::Div => ins.fdiv(x, y),
                Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    let cc = match op {
                        Op::Eq => FloatCC::Equal,
                        Op::Ne => FloatCC::NotEqual,
                        Op::Lt => FloatCC::LessThan,
                        Op::Le => FloatCC::LessThanOrEqual,
                        Op::Gt => FloatCC::GreaterThan,
                        _ => FloatCC::GreaterThanOrEqual,
                    };
                    let t = ins.fcmp(cc, x, y);
                    return Some(self.b.ins().uextend(I64, t));
                }
                _ => return None,
            };
            return Some(self.bits_of(r));
        }
        let bits = i64::from(m.bits().max(1));
        let r = match op {
            Op::Add => self.b.ins().iadd(a, b),
            Op::Sub => self.b.ins().isub(a, b),
            Op::Mul => self.b.ins().imul(a, b),
            Op::And => self.b.ins().band(a, b),
            Op::Or => self.b.ins().bor(a, b),
            Op::Xor => self.b.ins().bxor(a, b),
            Op::Shl | Op::Shr => {
                let n = self.b.ins().band_imm(b, bits - 1);
                if op == Op::Shl {
                    self.b.ins().ishl(a, n)
                } else if m.signed() {
                    self.b.ins().sshr(a, n)
                } else {
                    let x = self.norm(m, a);
                    self.b.ins().ushr(x, n)
                }
            }
            Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                let (x, y) = (self.norm(m, a), self.norm(m, b));
                let s = m.signed();
                let cc = match op {
                    Op::Eq => IntCC::Equal,
                    Op::Ne => IntCC::NotEqual,
                    Op::Lt if s => IntCC::SignedLessThan,
                    Op::Lt => IntCC::UnsignedLessThan,
                    Op::Le if s => IntCC::SignedLessThanOrEqual,
                    Op::Le => IntCC::UnsignedLessThanOrEqual,
                    Op::Gt if s => IntCC::SignedGreaterThan,
                    Op::Gt => IntCC::UnsignedGreaterThan,
                    Op::Ge if s => IntCC::SignedGreaterThanOrEqual,
                    _ => IntCC::UnsignedGreaterThanOrEqual,
                };
                let t = self.b.ins().icmp(cc, x, y);
                return Some(self.b.ins().uextend(I64, t));
            }
            _ => return None,
        };
        Some(self.norm(m, r))
    }

    /// `op a` on a value of `m` (`ops::un`).
    fn sun(&mut self, op: Op, m: M, a: Value) -> Value {
        match op {
            Op::Neg if m == M::F32 => {
                let r = self.b.ins().bxor_imm(a, 0x8000_0000);
                self.b.ins().band_imm(r, 0xffff_ffff)
            }
            Op::Neg if m == M::F64 => self.b.ins().bxor_imm(a, i64::MIN),
            Op::Neg => {
                let r = self.b.ins().ineg(a);
                self.norm(m, r)
            }
            Op::Not => {
                let r = self.b.ins().bnot(a);
                self.norm(m, r)
            }
            Op::LNot => {
                let t = self.truth(m, a);
                self.b.ins().bxor_imm(t, 1)
            }
            _ => a,
        }
    }

    /// `a` of `from` converted to `to` (`ops::cast`), or `None` for a type it does not take.
    fn scast(&mut self, from: M, to: M, a: Value) -> Option<Value> {
        let scalar = |m: M| m.0 <= M::PTR.0 && m != M::VOID;
        if !scalar(from) || !scalar(to) {
            return None;
        }
        if from == to {
            return Some(a);
        }
        if to == M::B {
            return Some(self.truth(from, a));
        }
        Some(match (from.float(), to.float()) {
            (false, false) => {
                let x = self.norm(from, a);
                self.norm(to, x)
            }
            (true, true) => {
                let f = self.float_of(from, a);
                let g = if to == M::F32 { self.b.ins().fdemote(F32, f) } else { self.b.ins().fpromote(F64, f) };
                self.bits_of(g)
            }
            (false, true) => {
                let v = self.norm(from, a);
                let f = if from.signed() { self.b.ins().fcvt_from_sint(F64, v) } else { self.b.ins().fcvt_from_uint(F64, v) };
                let f = if to == M::F32 { self.b.ins().fdemote(F32, f) } else { f };
                self.bits_of(f)
            }
            (true, false) => {
                let f = self.float_of(from, a);
                let f = if from == M::F32 { self.b.ins().fpromote(F64, f) } else { f };
                let x = if to.signed() { self.b.ins().fcvt_to_sint_sat(I64, f) } else { self.b.ins().fcvt_to_uint_sat(I64, f) };
                self.norm(to, x)
            }
        })
    }

    /// The first running lane (`run::first`): lane 0 of no lanes.
    fn first(&mut self) -> Value {
        let cur = self.cur();
        let z = self.b.ins().ctz(cur);
        let l = self.b.ins().band_imm(z, (W - 1) as i64);
        self.b.ins().uextend(I64, l)
    }

    /// `body(l)` for each running lane `l` (an `I64`), in order: a loop over the mask's bits.
    fn each_lane(&mut self, body: impl Fn(&mut Self, Value)) {
        let cur = self.cur();
        let (head, work, done) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.append_block_param(head, I32);
        self.b.ins().jump(head, &[cur.into()]);
        self.enter(head);
        let m = self.b.block_params(head)[0];
        self.b.ins().brif(m, work, &[], done, &[]);
        self.enter(work);
        let z = self.b.ins().ctz(m);
        let l = self.b.ins().uextend(I64, z);
        body(self, l);
        let less = self.b.ins().iadd_imm(m, -1);
        let rest = self.b.ins().band(m, less);
        self.b.ins().jump(head, &[rest.into()]);
        self.enter(done);
    }

    /// The place of lane `l` of a register of `bytes` lanes in the file at `base`, register `r`.
    fn lane_at(&mut self, base: Value, r: u32, bytes: i64, l: Value) -> Value {
        let off = self.b.ins().imul_imm(l, bytes);
        let p = self.b.ins().iadd(base, off);
        self.b.ins().iadd_imm(p, i64::from(r) * W as i64 * bytes)
    }

    /// A call of the section at instruction `to` (one that cannot stop) from function `row` (its table's row): the
    /// callee's frame and windows past the caller's (`code::TABLE`), the five masks into it and back, and the SIMD
    /// group's frame and windows set to the callee's while it runs, for its interpreter's steps.
    fn direct_call(&mut self, to: usize, row: usize) {
        use crate::code::TABLE;
        let r: [u32; TABLE] = self.calls.table[row * TABLE..][..TABLE].try_into().expect("a table row");
        let fp = self.b.ins().iadd_imm(self.fp, i64::from(r[0]));
        let s = self.b.ins().iadd_imm(self.s, i64::from(r[1]) * 8);
        let v32 = self.b.ins().iadd_imm(self.v32, i64::from(r[2]) * i64::from(R32));
        let v64 = self.b.ins().iadd_imm(self.v64, i64::from(r[3]) * i64::from(R64));
        let mk = self.b.ins().iadd_imm(self.mk, i64::from(r[4]) * 4);
        for m in 0..5 {
            let x = self.ld_m(m);
            self.b.ins().store(mf(), x, mk, 4 * m as i32);
        }
        let (pf, pw) = (self.places.fp, self.places.win);
        let old_fp = self.b.ins().load(I64, mf(), self.sg, pf);
        let old: Vec<Value> = (0..4).map(|w| self.b.ins().load(I64, mf(), self.sg, pw + 8 * w)).collect();
        self.b.ins().store(mf(), fp, self.sg, pf);
        for (w, &o) in old.iter().enumerate() {
            let x = self.b.ins().iadd_imm(o, i64::from(r[1 + w]));
            self.b.ins().store(mf(), x, self.sg, pw + 8 * w as i32);
        }
        let table = self.b.ins().load(I64, mf(), self.cx, self.places.entries);
        let k = self.calls.of_start[&to];
        let entry = self.b.ins().load(I64, mf(), table, 8 * k as i32);
        let zero = self.b.ins().iconst(I64, 0);
        let call = self.b.ins().call_indirect(self.entry_sig, entry, &[self.cx, self.sg, s, v32, v64, mk, fp, zero]);
        let res = self.b.inst_results(call)[0];
        self.b.ins().store(mf(), old_fp, self.sg, pf);
        for (w, &o) in old.iter().enumerate() {
            self.b.ins().store(mf(), o, self.sg, pw + 8 * w as i32);
        }
        let next = self.b.create_block();
        self.b.ins().brif(res, self.fail, &[], next, &[]);
        self.enter(next);
        for m in 0..5 {
            let x = self.b.ins().load(I32, mf(), mk, 4 * m as i32);
            self.st_m(m, x);
        }
    }

    /// An interpreter's step of instruction `i`.
    fn fallback(&mut self, i: usize) {
        let pc = self.b.ins().iconst(I64, i as i64);
        let step = self.b.ins().load(I64, mf(), self.cx, self.places.step);
        let call = self.b.ins().call_indirect(self.step_sig, step, &[self.cx, self.sg, pc]);
        let r = self.b.inst_results(call)[0];
        let next = self.b.create_block();
        self.b.ins().brif(r, self.fail, &[], next, &[]);
        self.enter(next);
    }

    /// Instruction `i`: whether the code goes on past it.
    fn inst(&mut self, i: usize) -> Result<bool, Error> {
        self.b.set_srcloc(cranelift_codegen::ir::SourceLoc::new(i as u32));
        let w0 = self.w(i, 0);
        let (a, b, c) = (self.w(i, 1), self.w(i, 2), self.w(i, 3));
        let op = op_of(w0);
        let f1 = (w0 >> 8) & 0xff;
        let shr = |b: &mut FunctionBuilder, x: Value| b.ins().ushr_imm(x, 31);
        match op {
            I::Call if !self.calls.can_stop[self.calls.of_start[&(a as usize)]] => {
                self.direct_call(a as usize, b as usize);
            }
            _ if stops(op) => {
                let next = self.b.ins().iconst(I64, i as i64 + 1);
                self.b.ins().store(mf(), next, self.sg, self.places.pc);
                let zero = self.b.ins().iconst(I64, 0);
                self.b.ins().return_(&[zero]);
                return Ok(false);
            }
            I::Switch => {
                let x = self.ld_s(a);
                let mut sw = cranelift_frontend::Switch::new();
                for e in 0..b as usize {
                    let l = u64::from(self.w(i + 1 + e, 1)) | u64::from(self.w(i + 1 + e, 2)) << 32;
                    let t = self.block(self.w(i + 1 + e, 3) as usize);
                    sw.set_entry(u128::from(l), t);
                }
                let otherwise = self.block(c as usize);
                sw.emit(&mut self.b, x, otherwise);
                return Ok(false);
            }
            I::Label => return Err(Error(format!("the code's instruction {i}, a switch's label, is reached"))),
            I::Jmp => {
                let t = self.block(a as usize);
                self.b.ins().jump(t, &[]);
                return Ok(false);
            }
            I::Jzm | I::Jzs | I::Jnzs => {
                let x = if op == I::Jzm { self.ld_m(a) } else { self.ld_s(a) };
                let (t, next) = (self.block(b as usize), self.block(i + 1));
                if t == next {
                    self.b.ins().jump(t, &[]);
                } else if op == I::Jnzs {
                    self.b.ins().brif(x, t, &[], next, &[]);
                } else {
                    self.b.ins().brif(x, next, &[], t, &[]);
                }
                return Ok(false);
            }
            I::MAnd | I::MAndN => {
                let m = self.tomask(c);
                let m = if op == I::MAndN { self.b.ins().bnot(m) } else { m };
                let x = self.ld_m(b);
                let o = self.b.ins().band(x, m);
                self.st_m(a, o);
            }
            I::MSet => {
                let x = self.ld_m(b);
                self.st_m(a, x);
            }
            I::MOr => {
                let (x, y) = (self.ld_m(a), self.ld_m(b));
                let o = self.b.ins().bor(x, y);
                self.st_m(a, o);
            }
            I::MClr => {
                let z = self.b.ins().iconst(I32, 0);
                self.st_m(a, z);
            }
            I::MAndNM => {
                let (x, y) = (self.ld_m(b), self.ld_m(c));
                let o = self.b.ins().band_not(x, y);
                self.st_m(a, o);
            }
            I::MLive | I::MRejoin | I::MExit | I::MResume | I::MSwEnd => {
                // `CUR = base & !(the exits')`.
                let (base, off): (u32, &[u32]) = match op {
                    I::MLive => (CUR, &[1, 2, 3, 4]),
                    I::MRejoin => (a, &[1, 3, 4]),
                    I::MExit => (a, &[3, 4]),
                    I::MResume => (a, &[4]),
                    _ => (a, &[2, 3, 4]),
                };
                let mut x = self.ld_m(base);
                for &m in off {
                    let y = self.ld_m(m);
                    x = self.b.ins().band_not(x, y);
                }
                self.st_m(CUR, x);
            }
            I::SConst => {
                let x = self.b.ins().iconst(I64, (u64::from(b) | u64::from(c) << 32) as i64);
                self.st_s(a, x);
            }
            I::SMov => {
                let x = self.ld_s(b);
                self.st_s(a, x);
            }
            I::Bcast32 => {
                let x = self.ld_s(b);
                let x = self.b.ins().ireduce(I32, x);
                let v = self.b.ins().splat(I32X4, x);
                for k in 0..W / 4 {
                    self.st32(a, k, v);
                }
            }
            I::Bcast64 => {
                let x = self.ld_s(b);
                let v = self.b.ins().splat(I64X2, x);
                for k in 0..W / 2 {
                    self.st64(a, k, v);
                }
            }
            I::LdL if (M(f1).bytes() == 8 && c == 8) || (M(f1).bytes() == 4 && c == 4) => {
                // A whole scalar variable's 32 values: one copy.
                let at = self.b.ins().iadd_imm(self.fp, i64::from(b));
                let wide = c == 8;
                for k in 0..W * c as usize / 16 {
                    let x = self.b.ins().load(I32X4, mf(), at, 16 * k as i32);
                    if wide {
                        self.st64(a, k, x);
                    } else {
                        self.st32(a, k, x);
                    }
                }
            }
            I::StL if (M(f1).bytes() == 8 && b == 8) || (M(f1).bytes() == 4 && b == 4) => {
                let at = self.b.ins().iadd_imm(self.fp, i64::from(a));
                let wide = b == 8;
                let chunks = if wide { W / 2 } else { W / 4 };
                self.if_full(
                    None,
                    |g| {
                        for k in 0..chunks {
                            let x = if wide { g.ld64(c, k, I64X2) } else { g.ld32(c, k, I32X4) };
                            g.b.ins().store(mf(), x, at, 16 * k as i32);
                        }
                    },
                    |g| g.blend_store(at, c, wide),
                );
            }
            I::VMovM32 | I::VMovM64 => {
                let wide = op == I::VMovM64;
                let (f, bytes, chunks) = if wide { (FV64, R64, W / 2) } else { (FV32, R32, W / 4) };
                self.if_full(
                    Some((f, a, bytes)),
                    |g| {
                        for k in 0..chunks {
                            let x = if wide { g.ld64(b, k, I64X2) } else { g.ld32(b, k, I32X4) };
                            if wide {
                                g.st64(a, k, x);
                            } else {
                                g.st32(a, k, x);
                            }
                        }
                    },
                    |g| {
                        let cur = g.cur();
                        for k in 0..chunks {
                            let sel = if wide { g.lanes2(cur, k) } else { g.lanes4(cur, k) };
                            let t = if wide { I64X2 } else { I32X4 };
                            let (x, o) = if wide { (g.ld64(b, k, t), g.ld64(a, k, t)) } else { (g.ld32(b, k, t), g.ld32(a, k, t)) };
                            let r = g.b.ins().bitselect(sel, x, o);
                            if wide {
                                g.st64(a, k, r);
                            } else {
                                g.st32(a, k, r);
                            }
                        }
                    },
                );
            }
            I::VCopy if c <= 64 => {
                self.each_lane(|g, l| {
                    let (pt, pf) = (g.lane_at(g.v64, a, 8, l), g.lane_at(g.v64, b, 8, l));
                    let (to, from) = (g.b.ins().load(I64, mf(), pt, 0), g.b.ins().load(I64, mf(), pf, 0));
                    g.copy_bytes(to, from, c);
                });
            }
            I::VZero if b <= 64 => {
                self.each_lane(|g, l| {
                    let pt = g.lane_at(g.v64, a, 8, l);
                    let to = g.b.ins().load(I64, mf(), pt, 0);
                    let z = g.b.ins().iconst(I64, 0);
                    let mut at = 0u32;
                    while b - at >= 8 {
                        g.b.ins().store(mf(), z, to, at as i32);
                        at += 8;
                    }
                    while at < b {
                        g.b.ins().istore8(mf(), z, to, at as i32);
                        at += 1;
                    }
                });
            }
            I::LAddr => {
                let at = self.b.ins().iadd_imm(self.fp, i64::from(b));
                let base = self.b.ins().splat(I64X2, at);
                let c = u64::from(c);
                for k in 0..W / 2 {
                    let l = 2 * k as u64;
                    let off = self.c64x2([l * c, (l + 1) * c]);
                    let o = self.b.ins().iadd(base, off);
                    self.st64(a, k, o);
                }
            }
            I::VLane => {
                for k in 0..W / 4 {
                    let l = 4 * k as u32;
                    let o = self.c32x4([l, l + 1, l + 2, l + 3]);
                    self.st32(a, k, o);
                }
            }

            I::Zx8 | I::Zx16 => {
                let m = if op == I::Zx8 { 0xff } else { 0xffff };
                let mask = self.c32x4([m; 4]);
                self.vun32(i, |b, x| b.ins().band(x, mask));
            }
            I::Sx8 | I::Sx16 => {
                let n = if op == I::Sx8 { 24 } else { 16 };
                self.vun32(i, |b, x| {
                    let y = b.ins().ishl_imm(x, n);
                    b.ins().sshr_imm(y, n)
                });
            }
            I::Truth32 | I::LNot32 => {
                let zero = self.c32x4([0; 4]);
                let cc = if op == I::Truth32 { IntCC::NotEqual } else { IntCC::Equal };
                self.vun32(i, |b, x| {
                    let t = b.ins().icmp(cc, x, zero);
                    b.ins().ushr_imm(t, 31)
                });
            }
            I::LNot64 => {
                let zero = self.c64x2([0; 2]);
                self.vun64(i, |b, x| {
                    let t = b.ins().icmp(IntCC::Equal, x, zero);
                    b.ins().ushr_imm(t, 63)
                });
            }
            I::Zext | I::Sext => {
                for k in 0..W / 4 {
                    let x = self.ld32(b, k, I32X4);
                    let (lo, hi) = if op == I::Zext { (self.b.ins().uwiden_low(x), self.b.ins().uwiden_high(x)) } else { (self.b.ins().swiden_low(x), self.b.ins().swiden_high(x)) };
                    self.st64(a, 2 * k, lo);
                    self.st64(a, 2 * k + 1, hi);
                }
            }
            I::Truth64 | I::Trunc => {
                let truth = op == I::Truth64;
                self.lanes(i, I64, I32, |b, x, _| {
                    if truth {
                        let t = b.ins().icmp_imm(IntCC::NotEqual, x, 0);
                        b.ins().uextend(I32, t)
                    } else {
                        b.ins().ireduce(I32, x)
                    }
                });
            }
            I::Neg32 => self.vun32(i, |b, x| b.ins().ineg(x)),
            I::Not32 => self.vun32(i, |b, x| b.ins().bnot(x)),
            I::NegF32 => {
                let sign = self.c32x4([0x8000_0000; 4]);
                self.vun32(i, |b, x| b.ins().bxor(x, sign));
            }
            I::Neg64 => self.vun64(i, |b, x| b.ins().ineg(x)),
            I::Not64 => self.vun64(i, |b, x| b.ins().bnot(x)),
            I::NegF64 => {
                let sign = self.c64x2([1 << 63; 2]);
                self.vun64(i, |b, x| b.ins().bxor(x, sign));
            }
            I::Popcount32 => self.lanes(i, I32, I32, |b, x, _| b.ins().popcnt(x)),

            I::Add32 => self.vbin32(i, I32X4, |b, x, y| b.ins().iadd(x, y)),
            I::Sub32 => self.vbin32(i, I32X4, |b, x, y| b.ins().isub(x, y)),
            I::Mul32 => self.vbin32(i, I32X4, |b, x, y| b.ins().imul(x, y)),
            I::And32 => self.vbin32(i, I32X4, |b, x, y| b.ins().band(x, y)),
            I::Or32 => self.vbin32(i, I32X4, |b, x, y| b.ins().bor(x, y)),
            I::Xor32 => self.vbin32(i, I32X4, |b, x, y| b.ins().bxor(x, y)),
            I::Shl32 => self.lanes(i, I32, I32, |b, x, y| b.ins().ishl(x, y)),
            I::ShrU32 => self.lanes(i, I32, I32, |b, x, y| b.ins().ushr(x, y)),
            I::ShrI32 => self.lanes(i, I32, I32, |b, x, y| b.ins().sshr(x, y)),
            I::Eq32 | I::Ne32 | I::LtU32 | I::LtI32 | I::LeU32 | I::LeI32 => {
                let cc = int_cc(op);
                self.vbin32(i, I32X4, |b, x, y| {
                    let t = b.ins().icmp(cc, x, y);
                    shr(b, t)
                });
            }
            I::MinU32 => self.vbin32(i, I32X4, |b, x, y| b.ins().umin(x, y)),
            I::MinI32 => self.vbin32(i, I32X4, |b, x, y| b.ins().smin(x, y)),
            I::MaxU32 => self.vbin32(i, I32X4, |b, x, y| b.ins().umax(x, y)),
            I::MaxI32 => self.vbin32(i, I32X4, |b, x, y| b.ins().smax(x, y)),

            I::Add64 => self.vbin64(i, I64X2, |b, x, y| b.ins().iadd(x, y)),
            I::Sub64 => self.vbin64(i, I64X2, |b, x, y| b.ins().isub(x, y)),
            I::And64 => self.vbin64(i, I64X2, |b, x, y| b.ins().band(x, y)),
            I::Or64 => self.vbin64(i, I64X2, |b, x, y| b.ins().bor(x, y)),
            I::Xor64 => self.vbin64(i, I64X2, |b, x, y| b.ins().bxor(x, y)),
            I::Mul64 => self.lanes(i, I64, I64, |b, x, y| b.ins().imul(x, y)),
            I::Shl64 => self.lanes(i, I64, I64, |b, x, y| b.ins().ishl(x, y)),
            I::ShrU64 => self.lanes(i, I64, I64, |b, x, y| b.ins().ushr(x, y)),
            I::ShrI64 => self.lanes(i, I64, I64, |b, x, y| b.ins().sshr(x, y)),
            I::Eq64 | I::Ne64 | I::LtU64 | I::LtI64 | I::LeU64 | I::LeI64 => {
                let cc = int_cc(op);
                self.lanes(i, I64, I32, |b, x, y| {
                    let t = b.ins().icmp(cc, x, y);
                    b.ins().uextend(I32, t)
                });
            }
            I::MinU64 | I::MinI64 | I::MaxU64 | I::MaxI64 => {
                let cc = match op {
                    I::MinU64 => IntCC::UnsignedLessThan,
                    I::MinI64 => IntCC::SignedLessThan,
                    I::MaxU64 => IntCC::UnsignedGreaterThan,
                    _ => IntCC::SignedGreaterThan,
                };
                self.lanes(i, I64, I64, |b, x, y| {
                    let t = b.ins().icmp(cc, x, y);
                    b.ins().select(t, x, y)
                });
            }

            I::AddF32 => self.vbin32(i, F32X4, |b, x, y| b.ins().fadd(x, y)),
            I::SubF32 => self.vbin32(i, F32X4, |b, x, y| b.ins().fsub(x, y)),
            I::MulF32 => self.vbin32(i, F32X4, |b, x, y| b.ins().fmul(x, y)),
            I::DivF32 => self.vbin32(i, F32X4, |b, x, y| b.ins().fdiv(x, y)),
            I::EqF32 | I::NeF32 | I::LtF32 | I::LeF32 => {
                let cc = float_cc(op);
                self.vbin32(i, F32X4, |b, x, y| {
                    let t = b.ins().fcmp(cc, x, y);
                    shr(b, t)
                });
            }
            I::AddF64 => self.vbin64(i, F64X2, |b, x, y| b.ins().fadd(x, y)),
            I::SubF64 => self.vbin64(i, F64X2, |b, x, y| b.ins().fsub(x, y)),
            I::MulF64 => self.vbin64(i, F64X2, |b, x, y| b.ins().fmul(x, y)),
            I::DivF64 => self.vbin64(i, F64X2, |b, x, y| b.ins().fdiv(x, y)),
            I::EqF64 | I::NeF64 | I::LtF64 | I::LeF64 => {
                let cc = float_cc(op);
                self.lanes(i, F64, I32, |b, x, y| {
                    let t = b.ins().fcmp(cc, x, y);
                    b.ins().uextend(I32, t)
                });
            }
            I::SShared | I::SGlobal => {
                let off = if op == I::SShared { self.places.shared } else { self.places.data };
                let base = self.b.ins().load(I64, mf(), self.cx, off);
                let x = self.b.ins().iadd_imm(base, i64::from(b));
                self.st_s(a, x);
            }
            I::SBin if f1 <= Op::LNot as u32 && scalar_m(M((w0 >> 16) & 0xff)) => {
                let (op, m) = (Op::of(f1), M((w0 >> 16) & 0xff));
                let (x, y) = (self.ld_s(b), self.ld_s(c));
                match self.sbin(op, m, x, y) {
                    Some(r) => self.st_s(a, r),
                    None => self.fallback(i),
                }
            }
            I::SUn if f1 <= Op::LNot as u32 && scalar_m(M((w0 >> 16) & 0xff)) => {
                let (op, m) = (Op::of(f1), M((w0 >> 16) & 0xff));
                let x = self.ld_s(b);
                let r = self.sun(op, m, x);
                self.st_s(a, r);
            }
            I::SCast => {
                let x = self.ld_s(b);
                match self.scast(M(f1), M((w0 >> 16) & 0xff), x) {
                    Some(r) => self.st_s(a, r),
                    None => self.fallback(i),
                }
            }
            I::SLoad if scalar_m(M(f1)) => {
                let p = self.ld_s(b);
                let r = self.load_m(M(f1), p, 0);
                self.st_s(a, r);
            }
            I::SStore if scalar_m(M(f1)) => {
                let (p, v) = (self.ld_s(a), self.ld_s(b));
                self.store_m(M(f1), p, 0, v);
            }
            I::LdLS if scalar_m(M(f1)) => {
                let l = self.first();
                let off = self.b.ins().imul_imm(l, i64::from(c));
                let p = self.b.ins().iadd(self.fp, off);
                let r = self.load_m(M(f1), p, b as i32);
                self.st_s(a, r);
            }
            I::Extract32 | I::Extract64 => {
                let l = self.first();
                let r = if op == I::Extract32 {
                    let p = self.lane_at(self.v32, b, 4, l);
                    let x = self.b.ins().uload32(mf(), p, 0);
                    self.norm(M(f1), x)
                } else {
                    let p = self.lane_at(self.v64, b, 8, l);
                    self.b.ins().load(I64, mf(), p, 0)
                };
                self.st_s(a, r);
            }
            I::LdL if scalar_m(M(f1)) => {
                // A variable's lanes apart (a field, a narrow type): each lane's, all 32.
                let m = M(f1);
                let at = self.b.ins().iadd_imm(self.fp, i64::from(b));
                if m.bytes() == 8 {
                    self.forget(FV64, a, R64);
                } else {
                    self.forget(FV32, a, R32);
                }
                for l in 0..W as i32 {
                    let x = self.load_m(m, at, l * c as i32);
                    if m.bytes() == 8 {
                        self.b.ins().store(mf(), x, self.v64, a as i32 * R64 + 8 * l);
                    } else {
                        self.b.ins().istore32(mf(), x, self.v32, a as i32 * R32 + 4 * l);
                    }
                }
            }
            I::StL if scalar_m(M(f1)) => {
                let m = M(f1);
                let at = self.b.ins().iadd_imm(self.fp, i64::from(a));
                self.each_lane(|g, l| {
                    let x = if m.bytes() == 8 {
                        let p = g.lane_at(g.v64, c, 8, l);
                        g.b.ins().load(I64, mf(), p, 0)
                    } else {
                        let p = g.lane_at(g.v32, c, 4, l);
                        g.b.ins().uload32(mf(), p, 0)
                    };
                    let off = g.b.ins().imul_imm(l, i64::from(b));
                    let p = g.b.ins().iadd(at, off);
                    g.store_m(m, p, 0, x);
                });
            }
            I::VLoad if scalar_m(M(f1)) => {
                let m = M(f1);
                self.each_lane(|g, l| {
                    let pa = g.lane_at(g.v64, b, 8, l);
                    let p = g.b.ins().load(I64, mf(), pa, 0);
                    let x = g.load_m(m, p, 0);
                    if m.bytes() == 8 {
                        let o = g.lane_at(g.v64, a, 8, l);
                        g.b.ins().store(mf(), x, o, 0);
                    } else {
                        let o = g.lane_at(g.v32, a, 4, l);
                        g.b.ins().istore32(mf(), x, o, 0);
                    }
                });
            }
            I::VStore if scalar_m(M(f1)) => {
                let m = M(f1);
                self.each_lane(|g, l| {
                    let pa = g.lane_at(g.v64, a, 8, l);
                    let p = g.b.ins().load(I64, mf(), pa, 0);
                    let x = if m.bytes() == 8 {
                        let q = g.lane_at(g.v64, b, 8, l);
                        g.b.ins().load(I64, mf(), q, 0)
                    } else {
                        let q = g.lane_at(g.v32, b, 4, l);
                        g.b.ins().uload32(mf(), q, 0)
                    };
                    g.store_m(m, p, 0, x);
                });
            }
            _ => self.fallback(i),
        }
        Ok(true)
    }
}

/// Whether `m` is a scalar's type (not an aggregate's, not void).
fn scalar_m(m: M) -> bool {
    m != M::VOID && m.0 <= M::PTR.0
}

fn int_cc(op: I) -> IntCC {
    match op {
        I::Eq32 | I::Eq64 => IntCC::Equal,
        I::Ne32 | I::Ne64 => IntCC::NotEqual,
        I::LtU32 | I::LtU64 => IntCC::UnsignedLessThan,
        I::LtI32 | I::LtI64 => IntCC::SignedLessThan,
        I::LeU32 | I::LeU64 => IntCC::UnsignedLessThanOrEqual,
        _ => IntCC::SignedLessThanOrEqual,
    }
}

fn float_cc(op: I) -> FloatCC {
    match op {
        I::EqF32 | I::EqF64 => FloatCC::Equal,
        I::NeF32 | I::NeF64 => FloatCC::NotEqual,
        I::LtF32 | I::LtF64 => FloatCC::LessThan,
        _ => FloatCC::LessThanOrEqual,
    }
}
