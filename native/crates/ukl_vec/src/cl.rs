//! The SSA compiled by Cranelift (decision 0567, step 3): each function at a width, its values in registers.
//!
//! - **A width** `w` (1, 2, 4, 8, 16 or 32 lanes) is how many of the SIMD group's lanes a function's code runs at
//!   once; the SIMD group's 32 run as `32 / w` chunks, one after another. At 1 the code is scalar, its varying
//!   branches real branches (the coroutines' code); wider, a lane's value is `w * bits / 128` vector registers (4 lanes
//!   of 32 bits, 2 of 64), a mask its lanes all ones or zeros.
//! - **A value is in registers**: a uniform one is a scalar (an integer normalized to 64 bits, as `ops` does; a float
//!   its own), a lanes one its parts, a mask its parts; the SSA's values are Cranelift's, blocks' parameters theirs.
//!   Memory is read and written only where the kernel does: its frame's addressed variables, its buffers.
//! - **A function is a function**: its frame pointer, its first lane, its entry's lanes and its arguments in, its result
//!   out, in registers; a call is a call.
//! - **A SIMD group is a fiber** (`fiber`): a barrier and a poll are calls of the runtime that switch to the next.
//! - What the target has no vector instruction for (a division, a shift by a varying count, a float's conversion to an
//!   integer, a gather) is done a lane at a time, each lane's the uniform code's (`u_bin` and the rest), so a value's
//!   result is the same whichever way it is made.

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::types::{F32, F32X4, F64, F64X2, I8, I8X16, I16, I16X8, I32, I32X4, I64, I64X2};
use cranelift_codegen::ir::{self, AbiParam, BlockArg, ConstantData, InstBuilder, MemFlags, Signature, StackSlotData, StackSlotKind, Type, Value};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};

use crate::Error;
use crate::k::{Builtin, M, Op};
use crate::ssa::{Func, Inst, NONE, O, Shape, Ssa, V};

type R<T> = Result<T, Error>;

/// Where the compiled code finds what it reads of the runtime's state (`exec`): offsets in a SIMD group's state and
/// in the launch's environment. The code holds no address of its process.
#[derive(Clone, Copy, Debug)]
pub struct Places {
    /// The SIMD group's: its environment's address, its frame's, its lanes (a `u32`), its arguments' (`u64`s), its
    /// lanes' ids in the group (`[[u32; 32]; 3]`).
    pub env: i32,
    pub frame: i32,
    pub live: i32,
    pub args: i32,
    pub lid: i32,
    /// Whether the code failed (`u32`, `fail`'s: the code returns from each function to the runtime).
    pub failed: i32,
    /// A SIMD group run as chunks of its lanes (a cross-lane kernel below 32 lanes): its exchange area's address
    /// (`u64`, two halves of `XHALF` bytes: the lanes' values, 8 bytes a lane, then their bits, a `u32` at `XWORD`),
    /// and the half its next exchange writes (`u32`; the chunks read the other after it).
    pub xbuf: i32,
    pub xpar: i32,
    /// The environment's: the constant memory's address, the threadgroup memory's, the grid's, group's and group
    /// id's sizes (`[u32; 3]` each), a scratch area's address (64 bytes, what a lane that does not run reads and
    /// writes), and the helpers' addresses (`H_*`).
    pub data: i32,
    pub shared: i32,
    pub grid: i32,
    pub block: i32,
    pub group: i32,
    pub scratch: i32,
    pub helpers: i32,
}

/// The runtime's helpers, by their place in the environment's table: `bar(sg, site)`, `poll(sg)`, `agree(sg, node,
/// decision, site)`, `fail(sg, what)`, `copy(to, from, bytes)`, `zero(to, bytes)`. `fail` and a failed `agree` set
/// the SIMD group's `failed`, and the code returns.
pub const H_BAR: i32 = 0;
pub const H_YIELD: i32 = 1;
pub const H_AGREE: i32 = 2;
pub const H_FAIL: i32 = 3;
pub const H_COPY: i32 = 4;
pub const H_ZERO: i32 = 5;
pub const HELPERS: usize = 6;

/// An exchange area's half: the lanes' values (8 bytes a lane), then their bits at `XWORD`.
pub const XHALF: i32 = 512;
const XWORD: i32 = 256;

/// `fail`'s reasons.
pub const FAIL_DIV: i64 = 1;
pub const FAIL_REM: i64 = 2;
/// A SIMD group's chunks at different places of its code (a decision of a chunk's own that reaches an exchange).
pub const FAIL_PLACES: i64 = 3;

/// What a SIMD group does once its chunks are all at a place between two of the code's regions (`Cg::split`), by
/// the low byte: nothing, the exchange area's halves swapped, a barrier (its site above), a poll.
const ACT_NONE: i64 = 0;
const ACT_X: i64 = 1;
const ACT_BAR: i64 = 2;
const ACT_YIELD: i64 = 3;
/// A region's number where no chunk has said which runs next, and the code's end.
const NEXT_NONE: i64 = -1;
const NEXT_END: i64 = -2;

/// The kernel's entry: a SIMD group's state; it runs the group's lanes.
pub type Entry = unsafe extern "C" fn(sg: *mut u8);

/// A kernel compiled.
pub struct Compiled {
    _module: cranelift_jit::JITModule,
    pub entry: Entry,
    pub width: u32,
    /// Whether its SIMD groups run as chunks of `width` lanes (a cross-lane kernel below 32 lanes): the kernel's
    /// code a loop over the chunks in each region between its exchanges (`Cg::split`).
    pub chunked: bool,
    /// Whether its SIMD groups wait for each other (a barrier, a poll): the runtime runs them as fibers then, and
    /// else calls each SIMD group's code in turn.
    pub waits: bool,
}

// SAFETY: the module's code is finalized and never written again.
unsafe impl Send for Compiled {}
// SAFETY: the same.
unsafe impl Sync for Compiled {}

/// A value's registers.
#[derive(Clone, Debug)]
enum Rp {
    None,
    U(Value),
    L(Vec<Value>),
    K(Vec<Value>),
}

fn isa() -> R<cranelift_codegen::isa::OwnedTargetIsa> {
    use cranelift_codegen::settings::Configurable;
    let mut flags = cranelift_codegen::settings::builder();
    // The verifier in a debug build: a value read where it is not made (a region's, `Cg::split`) fails the compile.
    let verify = if cfg!(debug_assertions) { "true" } else { "false" };
    for (k, v) in [("opt_level", "speed"), ("enable_verifier", verify), ("enable_multi_ret_implicit_sret", "true"), ("enable_probestack", "false")] {
        flags.set(k, v).map_err(|e| Error(format!("the compiler's flags: {e}")))?;
    }
    cranelift_native::builder()
        .map_err(|e| Error(format!("the compiler's target: {e}")))?
        .finish(cranelift_codegen::settings::Flags::new(flags))
        .map_err(|e| Error(format!("the compiler's target: {e}")))
}

fn mf() -> MemFlags {
    MemFlags::new().with_notrap()
}

fn le() -> MemFlags {
    MemFlags::new().with_endianness(ir::Endianness::Little)
}

/// Whether type `m`'s lanes are 64 bits.
fn wide(m: M) -> bool {
    m.bytes() == 8 && !m.is_agg()
}

/// A uniform value's type.
fn uty(m: M) -> Type {
    match m {
        M::F32 => F32,
        M::F64 => F64,
        _ => I64,
    }
}

/// `x` as type `m` holds it (`ops::norm`), a uniform integer.
fn norm_u(b: &mut FunctionBuilder, m: M, x: Value) -> Value {
    match m {
        M::B => {
            let c = b.ins().icmp_imm(IntCC::NotEqual, x, 0);
            b.ins().uextend(I64, c)
        }
        M::I8 => {
            let t = b.ins().ireduce(I8, x);
            b.ins().sextend(I64, t)
        }
        M::U8 => b.ins().band_imm(x, 0xff),
        M::I16 => {
            let t = b.ins().ireduce(I16, x);
            b.ins().sextend(I64, t)
        }
        M::U16 => b.ins().band_imm(x, 0xffff),
        M::I32 => {
            let t = b.ins().ireduce(I32, x);
            b.ins().sextend(I64, t)
        }
        M::U32 => {
            let t = b.ins().ireduce(I32, x);
            b.ins().uextend(I64, t)
        }
        _ => x,
    }
}

/// The bits of `m`'s width, as a mask.
fn width_mask(m: M) -> i64 {
    if m.bits() >= 64 { -1 } else { ((1u64 << m.bits()) - 1) as i64 }
}

/// Compiles kernel `ssa` (named `name`) at width `w`, or for `w` 0 the width its register pressure chooses
/// (`pressure`): the code, and its kept form (`load`), if its relocations are ones it can hold. A kernel with a
/// cross-lane operation below 32 lanes runs its SIMD groups as chunks (`Compiled::chunked`); one that calls a function
/// with one runs at 32 lanes only.
pub fn compile(ssa: &Ssa, name: &str, w: u32, places: &Places) -> R<(Compiled, Option<Vec<u8>>)> {
    let w = if w == 0 {
        let ps = crate::pressure::of(ssa);
        let files = crate::pressure::Files::host();
        let w = crate::pressure::width(&ps, files);
        if std::env::var("RERAIL_UKL_VEC_TIMES").is_ok() {
            let row: Vec<String> = crate::pressure::WIDTHS
                .iter()
                .enumerate()
                .map(|(k, &w)| {
                    let (v, i) = ps.iter().fold((0, 0), |(v, i), p| (v.max(p.at[k].vec + p.at[k].float), i.max(p.at[k].int)));
                    format!("w{w} {v}v {i}i {:.1}", crate::pressure::lane_cost(&ps, k, files))
                })
                .collect();
            let ops: Vec<String> = ps.iter().map(|p| format!("{:.0} lanes {:.0} each {:.0} bytes {:.0} other", p.lanes, p.scalar, p.bytes, p.other)).collect();
            eprintln!("PRESSURE {name}: {} -> w{w} ({})", row.join(", "), ops.join("; "));
        }
        // The chunks of a SIMD group take turns at the kernel's own exchanges, not at its callees'.
        if ssa.funcs[1..].iter().any(|f| f.cross) { 32 } else { w }
    } else {
        w
    };
    if !matches!(w, 1 | 2 | 4 | 8 | 16 | 32) {
        return Err(Error(format!("{name}: a width of {w} lanes")));
    }
    let chunked = ssa.funcs[0].cross && w < 32;
    if chunked && let Some(f) = ssa.funcs[1..].iter().find(|f| f.cross) {
        return Err(Error(format!("{name}: {} has a cross-lane operation: the kernel runs at 32 lanes, not {w}", f.name)));
    }
    let waits = ssa.funcs.iter().any(|f| f.insts.iter().any(|i| matches!(i.op, O::Bar | O::Yield)));
    let mut module = module()?;
    let cc = module.isa().default_call_conv();
    // Each function's convention: the platform's (its callers save what they keep), or one that keeps its callers'
    // registers (it saves what it writes), whichever saves fewer (`pressure::keeps_callers`).
    let wk = crate::pressure::WIDTHS.iter().position(|&x| x == w).expect("a width");
    let keeps = crate::pressure::keeps_callers(&crate::pressure::of(ssa), wk);
    let sigs: Vec<Signature> = ssa
        .funcs
        .iter()
        .enumerate()
        .map(|(k, f)| signature(f, k == 0, w, if keeps[k] { cranelift_codegen::isa::CallConv::PreserveAll } else { cc }))
        .collect();
    let ids: Vec<FuncId> = sigs.iter().enumerate().map(|(k, s)| module.declare_function(&format!("f{k}"), Linkage::Local, s)).collect::<Result<_, _>>().map_err(|e| Error(format!("{name}: {e}")))?;
    let mut fbc = FunctionBuilderContext::new();
    let mut ctx = module.make_context();
    // The kernel's function again for a chunk whose lanes all run (`Cg::full`), after the SSA's functions.
    let full_id = module.declare_function("full", Linkage::Local, &sigs[0]).map_err(|e| Error(format!("{name}: {e}")))?;
    let mut kept = Some(Kept::new(w | u32::from(chunked) << 16 | u32::from(waits) << 17, ssa.funcs.len() + 2));
    // A development hook: each machine instruction's SSA value, to the file `RERAIL_UKL_VEC_CL_MAP` names (appended),
    // for a profile of the compiled code: `start end kernel function value operation`.
    let map = std::env::var("RERAIL_UKL_VEC_CL_MAP").ok();
    let mut spans: Vec<(usize, Vec<(u32, u32, u32)>)> = Vec::new();
    let jobs = (0..ssa.funcs.len()).map(|k| (k, false)).chain(std::iter::once((0, true)));
    for (k, full) in jobs {
        let f = &ssa.funcs[k];
        let fid = if full { full_id } else { ids[k] };
        ctx.func.signature = sigs[k].clone();
        ctx.func.name = ir::UserFuncName::user(0, fid.as_u32());
        {
            let b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            let refs = Vec::new();
            let mut g = Cg::new(b, ssa, f, k == 0, w, places, cc, refs);
            g.full = full;
            if chunked && k == 0 {
                g.chunked = true;
                g.group_dec = crate::ssa::group_decisions(ssa, f);
            }
            g.callees = ids.iter().map(|&id| module.declare_func_in_func(id, g.b.func)).collect();
            g.function().map_err(|e| Error(format!("{name}: {}: {e}", f.name)))?;
            g.b.seal_all_blocks();
            g.b.finalize();
        }
        // A development hook: each function's CLIF and machine code, to the directory `RERAIL_UKL_VEC_ASM` names.
        let asm = std::env::var("RERAIL_UKL_VEC_ASM").ok();
        ctx.set_disasm(asm.is_some());
        if let Some(dir) = &asm {
            let _ = std::fs::write(format!("{dir}/{name}.w{w}.{k}{}.clif", if full { ".full" } else { "" }), ctx.func.display().to_string());
        }
        module.define_function(fid, &mut ctx).map_err(|e| Error(format!("{name}: {}: {e:?}", f.name)))?;
        if let (Some(dir), Some(v)) = (&asm, ctx.compiled_code().and_then(|c| c.vcode.clone())) {
            let _ = std::fs::write(format!("{dir}/{name}.w{w}.{k}{}.s", if full { ".full" } else { "" }), format!("; {}\n{v}", f.name));
            let _ = std::fs::write(format!("{dir}/{name}.w{w}.{k}{}.opt.clif", if full { ".full" } else { "" }), ctx.func.display().to_string());
        }
        kept = kept.and_then(|kp| kp.add(&ctx));
        if map.is_some() && !full {
            if let Some(cc) = ctx.compiled_code() {
                spans.push((k, cc.buffer.get_srclocs_sorted().iter().map(|l| (l.start, l.end, l.loc.bits())).collect::<Vec<_>>()));
            }
        }
        module.clear_context(&mut ctx);
    }
    // The entry: each chunk of the SIMD group's lanes that has a lane, its kernel's function.
    let mut esig = Signature::new(cc);
    esig.params.push(AbiParam::new(I64));
    let eid = module.declare_function("entry", Linkage::Local, &esig).map_err(|e| Error(format!("{name}: {e}")))?;
    ctx.func.signature = esig;
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let kernel = module.declare_func_in_func(ids[0], b.func);
        let full = module.declare_func_in_func(full_id, b.func);
        entry(&mut b, kernel, full, w, chunked, places);
        b.seal_all_blocks();
        b.finalize();
    }
    module.define_function(eid, &mut ctx).map_err(|e| Error(format!("{name}: the entry: {e:?}")))?;
    kept = kept.and_then(|kp| kp.add(&ctx));
    module.clear_context(&mut ctx);
    module.finalize_definitions().map_err(|e| Error(format!("{name}: {e}")))?;
    // SAFETY: a finalized function of `Entry`'s signature.
    let entry = unsafe { std::mem::transmute::<*const u8, Entry>(module.get_finalized_function(eid)) };
    if let Some(path) = &map {
        use std::fmt::Write as _;
        let mut out = String::new();
        for (k, ls) in &spans {
            let base = module.get_finalized_function(ids[*k]) as u64;
            let f = &ssa.funcs[*k];
            for &(a, b, v) in ls {
                let op = if (v as usize) < f.insts.len() { format!("{:?}/{}", f.insts[v as usize].op, f.insts[v as usize].sub) } else { "-".into() };
                let _ = writeln!(out, "{:x} {:x} {name} {} {v} {op}", base + u64::from(a), base + u64::from(b), f.name);
            }
        }
        if let Ok(mut fh) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            use std::io::Write as _;
            let _ = fh.write_all(out.as_bytes());
        }
    }
    Ok((Compiled { _module: module, entry, width: w, chunked, waits }, kept.map(|k| k.out)))
}

/// What the compiled code is: the host's target and features, and the compiler's own source (the SSA's, this back
/// end's, the runtime's state it reads), so a kept form is loaded only by the compiler that made it.
pub fn host() -> String {
    // FNV-1a over the sources.
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for src in [include_str!("ssa.rs"), include_str!("cl.rs"), include_str!("exec.rs"), include_str!("fiber.rs"), include_str!("pressure.rs")] {
        for &b in src.as_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    match isa() {
        Ok(isa) => format!("{} {} {h:016x}", isa.triple(), isa.isa_flags().iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")),
        Err(e) => e.0,
    }
}

/// The kept form's first bytes, and its version.
const KEPT: &[u8; 8] = b"UKLCL002";

/// The relocations a kept form holds: a call of another of its functions, each target's.
const RELOCS: [cranelift_codegen::binemit::Reloc; 10] = {
    use cranelift_codegen::binemit::Reloc::*;
    [Arm64Call, X86CallPCRel4, X86CallPLTRel4, X86PCRel4, Abs8, Abs4, X86GOTPCRel4, Aarch64AdrGotPage21, Aarch64Ld64GotLo12Nc, Aarch64AdrPrelPgHi21]
};

/// A kept form being written: its width, its functions' count, then each function's alignment, bytes and
/// relocations (offset, kind, the function it names, addend), little-endian; the entry last.
struct Kept {
    out: Vec<u8>,
}

impl Kept {
    fn new(w: u32, funcs: usize) -> Kept {
        let mut out = KEPT.to_vec();
        out.extend(w.to_le_bytes());
        out.extend((funcs as u32).to_le_bytes());
        Kept { out }
    }

    /// The function `ctx` compiled, added; none if a relocation is not one it holds.
    fn add(mut self, ctx: &cranelift_codegen::Context) -> Option<Kept> {
        let cc = ctx.compiled_code()?;
        let code = cc.buffer.data();
        self.out.extend(cc.buffer.alignment.to_le_bytes());
        self.out.extend((code.len() as u64).to_le_bytes());
        self.out.extend(code);
        let relocs = cc.buffer.relocs();
        self.out.extend((relocs.len() as u32).to_le_bytes());
        for r in relocs {
            let kind = RELOCS.iter().position(|&k| k == r.kind)? as u8;
            let index = match r.target {
                cranelift_codegen::FinalizedRelocTarget::ExternalName(ir::ExternalName::User(reff)) => ctx.func.params.user_named_funcs()[reff].index,
                _ => return None,
            };
            self.out.extend(r.offset.to_le_bytes());
            self.out.push(kind);
            self.out.extend(index.to_le_bytes());
            self.out.extend(r.addend.to_le_bytes());
        }
        Some(self)
    }
}

/// A kept form (`compile`'s) loaded: its code, linked; none if it is not one.
pub fn load(bytes: &[u8]) -> Option<Compiled> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let b = bytes.get(at..at + n)?;
        at += n;
        Some(b)
    };
    if take(8)? != KEPT {
        return None;
    }
    let u32_of = |b: &[u8]| u32::from_le_bytes(b.try_into().expect("4 bytes"));
    let wc = u32_of(take(4)?);
    let (w, chunked, waits) = (wc & 0xffff, wc >> 16 & 1 == 1, wc >> 17 & 1 == 1);
    let n = u32_of(take(4)?) as usize;
    if n == 0 {
        return None;
    }
    let mut module = module().ok()?;
    let sig = Signature::new(module.isa().default_call_conv());
    let ids: Vec<FuncId> = (0..n).map(|k| module.declare_function(&format!("f{k}"), Linkage::Local, &sig)).collect::<Result<_, _>>().ok()?;
    for &id in &ids {
        let align = u32_of(take(4)?);
        let len = u64::from_le_bytes(take(8)?.try_into().ok()?) as usize;
        let code = take(len)?.to_vec();
        let nr = u32_of(take(4)?) as usize;
        let mut relocs = Vec::with_capacity(nr);
        for _ in 0..nr {
            let offset = u32_of(take(4)?);
            let kind = *RELOCS.get(take(1)?[0] as usize)?;
            let index = u32_of(take(4)?);
            let addend = i64::from_le_bytes(take(8)?.try_into().ok()?);
            if index as usize >= n {
                return None;
            }
            relocs.push(cranelift_module::ModuleReloc { offset, kind, name: cranelift_module::ModuleRelocTarget::user(0, index), addend });
        }
        module.define_function_bytes(id, u64::from(align), &code, &relocs).ok()?;
    }
    if take(1).is_some() {
        return None;
    }
    module.finalize_definitions().ok()?;
    // SAFETY: the kept form's last function is its entry, of `Entry`'s signature.
    let entry = unsafe { std::mem::transmute::<*const u8, Entry>(module.get_finalized_function(*ids.last()?)) };
    Some(Compiled { _module: module, entry, width: w, chunked, waits })
}

/// The address space a kernel's code is in: one arena, so a call of one of its functions by another is in a direct
/// call's reach (AArch64's `bl`, 128 MB; x86-64's `call`, 2 GB). Each allocation in a place of its own (the system
/// provider's, `mmap`s) put them anywhere, and a call out of reach went elsewhere.
const ARENA: usize = 64 << 20;

fn module() -> R<cranelift_jit::JITModule> {
    let mut b = cranelift_jit::JITBuilder::with_isa(isa()?, cranelift_module::default_libcall_names());
    let arena = cranelift_jit::ArenaMemoryProvider::new_with_size(ARENA).map_err(|e| Error(format!("the compiled code's arena: {e}")))?;
    b.memory_provider(Box::new(arena));
    Ok(cranelift_jit::JITModule::new(b))
}

/// A mask's parts at width `w`: one `I32` (0 or all ones) at 1, else `I32X4`s.
fn mask_parts(w: u32) -> usize {
    if w == 1 { 1 } else { (w as usize / 4).max(1) }
}

fn mask_ty(w: u32) -> Type {
    if w == 1 { I32 } else { I32X4 }
}

/// A lanes value of `m`'s parts and their type, at width `w`.
fn parts(m: M, w: u32) -> (usize, Type) {
    let bits = if wide(m) { 64 } else { 32 };
    if w == 1 {
        return (
            1,
            match m {
                M::F32 => F32,
                M::F64 => F64,
                _ if wide(m) => I64,
                _ => I32,
            },
        );
    }
    let t = match m {
        M::F32 => F32X4,
        M::F64 => F64X2,
        _ if wide(m) => I64X2,
        _ => I32X4,
    };
    (((w * bits) as usize / 128).max(1), t)
}

/// A function's signature at width `w`: the SIMD group's state, the frame, the first lane, the entry's lanes, its
/// arguments (none for the kernel's, which reads them from the state); its result.
fn signature(f: &Func, kernel: bool, w: u32, cc: cranelift_codegen::isa::CallConv) -> Signature {
    let mut s = Signature::new(cc);
    for _ in 0..3 {
        s.params.push(AbiParam::new(I64));
    }
    for _ in 0..mask_parts(w) {
        s.params.push(AbiParam::new(mask_ty(w)));
    }
    if !kernel {
        for &(m, sh) in &f.args {
            push_shape(&mut s.params, m, sh, w);
        }
    }
    let (m, sh) = f.ret;
    push_shape(&mut s.returns, m, sh, w);
    s
}

fn push_shape(to: &mut Vec<AbiParam>, m: M, sh: Shape, w: u32) {
    match sh {
        Shape::Uniform => to.push(AbiParam::new(uty(m))),
        Shape::Lanes => {
            let (n, t) = parts(m, w);
            to.extend(std::iter::repeat_n(AbiParam::new(t), n));
        }
        Shape::Mask => to.extend(std::iter::repeat_n(AbiParam::new(mask_ty(w)), mask_parts(w))),
        Shape::None => {}
    }
}

/// The entry's body: for each chunk of `w` lanes, its lanes of the SIMD group's (`live`), the kernel's function
/// called if it has one; `chunked`, the kernel's function once, which runs the chunks itself (`Cg::split`): the full
/// one where every lane of the SIMD group runs.
fn entry(b: &mut FunctionBuilder, kernel: ir::FuncRef, full: ir::FuncRef, w: u32, chunked: bool, pl: &Places) {
    let e = b.create_block();
    b.append_block_params_for_function_params(e);
    b.switch_to_block(e);
    let sg = b.block_params(e)[0];
    let live = b.ins().uload32(mf(), sg, pl.live);
    let fp = b.ins().load(I64, mf(), sg, pl.frame);
    if chunked {
        // Its first lane and its mask parameters unread: each chunk's are the function's own.
        let zero = b.ins().iconst(I64, 0);
        let is_full = b.ins().icmp_imm(IntCC::Equal, live, 0xffff_ffff);
        let (fb, pb, join) = (b.create_block(), b.create_block(), b.create_block());
        b.ins().brif(is_full, fb, &[], pb, &[]);
        for (blk, f) in [(fb, full), (pb, kernel)] {
            b.switch_to_block(blk);
            let bits32 = b.ins().ireduce(I32, live);
            let mut args = vec![sg, fp, zero];
            args.extend(mask_of_bits(b, bits32, w));
            b.ins().call(f, &args);
            b.ins().jump(join, &[]);
        }
        b.switch_to_block(join);
        b.ins().return_(&[]);
        return;
    }
    let head = b.create_block();
    let body = b.create_block();
    let next = b.create_block();
    let done = b.create_block();
    b.append_block_param(head, I64);
    let zero = b.ins().iconst(I64, 0);
    b.ins().jump(head, &[BlockArg::Value(zero)]);
    b.switch_to_block(head);
    let lane0 = b.block_params(head)[0];
    let shifted = b.ins().ushr(live, lane0);
    let bits = if w == 32 { shifted } else { b.ins().band_imm(shifted, (1i64 << w) - 1) };
    b.ins().brif(bits, body, &[], next, &[]);
    b.switch_to_block(body);
    call_kernel(b, kernel, full, bits, w, sg, fp, lane0);
    b.ins().jump(next, &[]);
    b.switch_to_block(next);
    let l = b.ins().iadd_imm(lane0, i64::from(w));
    let more = b.ins().icmp_imm(IntCC::UnsignedLessThan, l, 32);
    b.ins().brif(more, head, &[BlockArg::Value(l)], done, &[]);
    b.switch_to_block(done);
    b.ins().return_(&[]);
}

/// A chunk's call of the kernel's function, its lanes `bits` (an `I64`, not 0): the full one (`Cg::full`) when every
/// lane of the chunk runs, else the one of its mask.
#[allow(clippy::too_many_arguments)]
fn call_kernel(b: &mut FunctionBuilder, kernel: ir::FuncRef, full: ir::FuncRef, bits: Value, w: u32, sg: Value, fp: Value, lane0: Value) {
    let all = if w == 32 { 0xffff_ffff } else { (1i64 << w) - 1 };
    let is_full = b.ins().icmp_imm(IntCC::Equal, bits, all);
    let (fb, pb, join) = (b.create_block(), b.create_block(), b.create_block());
    b.ins().brif(is_full, fb, &[], pb, &[]);
    for (blk, f) in [(fb, full), (pb, kernel)] {
        b.switch_to_block(blk);
        let bits32 = b.ins().ireduce(I32, bits);
        let mask = mask_of_bits(b, bits32, w);
        let mut args = vec![sg, fp, lane0];
        args.extend(mask);
        b.ins().call(f, &args);
        b.ins().jump(join, &[]);
    }
    b.switch_to_block(join);
}

/// A mask's parts from its lanes' bits (`I32`, bit `j` lane `j` of the chunk).
fn mask_of_bits(b: &mut FunctionBuilder, bits: Value, w: u32) -> Vec<Value> {
    if w == 1 {
        let one = b.ins().band_imm(bits, 1);
        return vec![b.ins().ineg(one)];
    }
    let v = b.ins().splat(I32X4, bits);
    (0..mask_parts(w))
        .map(|p| {
            let l = 4 * p as u32;
            let sel = vconst32(b, [1 << l, 1 << (l + 1), 1 << (l + 2), 1 << (l + 3)]);
            let x = b.ins().band(v, sel);
            b.ins().icmp(IntCC::Equal, x, sel)
        })
        .collect()
}

/// A lanes value whose lane `j` is `first + j * step` (`first` a 64-bit sum, `Lin`): in its type's bits, or
/// (`exact`) as an integer, so a cast that widens it keeps it. `group`, where known: its value at the SIMD group's
/// lane 0, so the SIMD group's lane `l` (0 to 31) is `group + l * step` (the first dimension's thread id: its group
/// lane 0's a multiple of 32, `exec::run`'s).
#[derive(Clone)]
struct Steps {
    first: Lin,
    step: i64,
    exact: bool,
    group: Option<Lin>,
}

/// A 64-bit integer as a sum: of `I64` values each times a constant, each with the loop depth of the block that makes
/// it, and a constant. Made (`Cg::sum`) in order of depth, so the part a loop does not change is one value its loop
/// can be left with (Cranelift hoists it), and the loop adds only what changes (an address: a pointer and its
/// counter's multiple, not the counter's sum with the invariant terms, widened and scaled each time round).
#[derive(Clone, Debug, Default)]
struct Lin {
    terms: Vec<(Value, i64, u32)>,
    k: i64,
}

impl Lin {
    fn leaf(v: Value, depth: u32) -> Lin {
        Lin { terms: vec![(v, 1, depth)], k: 0 }
    }

    fn konst(k: i64) -> Lin {
        Lin { terms: Vec::new(), k }
    }

    fn plus(mut self, o: Lin) -> Lin {
        for t in o.terms {
            match self.terms.iter_mut().find(|x| x.0 == t.0) {
                Some(x) => x.1 = x.1.wrapping_add(t.1),
                None => self.terms.push(t),
            }
        }
        self.k = self.k.wrapping_add(o.k);
        self.terms.retain(|t| t.1 != 0);
        self
    }

    fn times(mut self, c: i64) -> Lin {
        for t in &mut self.terms {
            t.1 = t.1.wrapping_mul(c);
        }
        self.k = self.k.wrapping_mul(c);
        self.terms.retain(|t| t.1 != 0);
        self
    }

    /// The power of two every value of the sum is a multiple of (its log): its constants' least.
    fn align(&self) -> u32 {
        self.terms.iter().map(|t| t.1.trailing_zeros()).chain([self.k.trailing_zeros()]).min().unwrap_or(64)
    }

    /// The sum over `2^k`, where `align` is at least `k`.
    fn over(mut self, k: u32) -> Lin {
        for t in &mut self.terms {
            t.1 >>= k;
        }
        self.k >>= k;
        self
    }
}

fn vconst32(b: &mut FunctionBuilder, x: [u32; 4]) -> Value {
    let bytes: Vec<u8> = x.iter().flat_map(|v| v.to_le_bytes()).collect();
    let c = b.func.dfg.constants.insert(ConstantData::from(bytes));
    b.ins().vconst(I32X4, c)
}

fn vconst64(b: &mut FunctionBuilder, x: [u64; 2]) -> Value {
    let bytes: Vec<u8> = x.iter().flat_map(|v| v.to_le_bytes()).collect();
    let c = b.func.dfg.constants.insert(ConstantData::from(bytes));
    b.ins().vconst(I64X2, c)
}

/// A function's code generator.
struct Cg<'a, 'b> {
    b: FunctionBuilder<'b>,
    ssa: &'a Ssa,
    f: &'a Func,
    kernel: bool,
    w: u32,
    pl: &'a Places,
    cc: cranelift_codegen::isa::CallConv,
    callees: Vec<ir::FuncRef>,
    blocks: Vec<ir::Block>,
    vals: Vec<Rp>,
    sg: Value,
    fp: Value,
    lane0: Value,
    env: Value,
    /// The function's entry's lanes (its `Entry` value), whose stores need no blend.
    entry_v: V,
    /// A scratch slot: a lanes value stored to read one lane by a dynamic index.
    slot: Option<ir::StackSlot>,
    /// A comparison's mask, before it is made a `bool`'s lanes (`MaskOf` of it takes it).
    raw: HashMap<V, Vec<Value>>,
    /// The frame variables' addresses (`FrameAddr`): base and stride.
    frame_of: HashMap<V, (u32, u32)>,
    /// The lanes values that are their chunk's first lane's value and a constant step per lane (`Steps`): a thread's
    /// id in its group's first dimension, and the sums, products and casts of it, to the addresses made of them.
    steps: HashMap<V, Steps>,
    /// Stores of 1- or 2-byte lanes some bytes apart, held until their bytes are whole elements (`hold_bytes`).
    held: Vec<(M, V, V, V)>,
    /// The blocks' immediate dominators, and the bounds the branches give: a value at most the bound in the blocks
    /// the branch's side dominates (`bound`).
    idom: Vec<usize>,
    below: HashMap<V, Vec<(u64, usize)>>,
    /// A counted loop's counter `k` (a header's parameter, from a constant up by 1 while `k + 1 < N`): its block
    /// parameter is `k - N` (`counted`: it and `N`), so the loop runs while that is not 0 (`count_tests`: the test
    /// of `k + 1 < N`, the counter's).
    counted: HashMap<V, (Value, i64)>,
    count_tests: HashMap<V, V>,
    /// Each value's block, and each block's loop depth (the natural loops it is in), for `Lin`'s order.
    block_of: Vec<u32>,
    depth: Vec<u32>,
    preds: Vec<Vec<usize>>,
    /// Splats and 64-bit masks made in the block being made.
    splats: HashMap<(V, u32), Vec<Value>>,
    masks64: HashMap<V, Vec<Value>>,
    /// The function's entry's lanes as bits (`mask_bits`).
    entry_bits: Value,
    helper_sigs: HashMap<usize, ir::SigRef>,
    /// Whether the function runs a chunk of its SIMD group's lanes, as a fiber of its own (`Compiled::chunked`), and
    /// then which blocks' branches are the group's (`ssa::group_decisions`); the block being made.
    chunked: bool,
    group_dec: Vec<bool>,
    cur_block: usize,
    /// The kernel's function for a chunk whose lanes all run: its entry's mask all lanes, a constant.
    full: bool,
    /// The masks known to have every lane of the chunk (`all`): the entry's, where the function is `full`, and the
    /// masks made of them.
    ones: std::collections::HashSet<V>,
    /// A chunked kernel's regions (`split`): each block's at its start (`regions`), the blocks a region starts at
    /// (a join of different ones), and the values live at each block's end.
    lab: Vec<Option<crate::ssa::Lab>>,
    resume: Vec<bool>,
    live_out: Vec<Vec<bool>>,
    /// The regions made: their numbers, their first blocks, the values each reads again (or makes again) at its
    /// start, and those now in place of the values they were where the region being made is not their first.
    reg_ids: HashMap<crate::ssa::Lab, usize>,
    reg_blocks: Vec<ir::Block>,
    over: Vec<Vec<(V, Rp, Option<Steps>)>>,
    applied: Vec<(V, Rp, Option<Steps>, Option<Vec<Value>>)>,
    region: usize,
    /// The values kept across regions: each one's place in the save area (a chunk's at `16 * parts * chunk` past
    /// it), its form (`Rp`'s: 0 uniform, 1 lanes, 2 a mask) and its parts' types; the area's bytes, its slot and
    /// address; the slot of the region to run, the next, and what is done between (`ACT_*`).
    saved: HashMap<V, (i32, u8, Vec<ir::Type>)>,
    save_size: u32,
    save_slot: Option<ir::StackSlot>,
    save_base: Value,
    st: Option<ir::StackSlot>,
    /// The chunk being run (`I64`), and the block that runs the next.
    chunk: Value,
    next: Option<ir::Block>,
    /// The instruction being made, and the regions it has ended (an atomic two).
    cur_inst: V,
    split_n: u8,
    err: Option<Error>,
}

impl<'a, 'b> Cg<'a, 'b> {
    #[allow(clippy::too_many_arguments)]
    fn new(b: FunctionBuilder<'b>, ssa: &'a Ssa, f: &'a Func, kernel: bool, w: u32, pl: &'a Places, cc: cranelift_codegen::isa::CallConv, callees: Vec<ir::FuncRef>) -> Self {
        let entry_v = (0..f.insts.len() as V).find(|&x| f.inst(x).op == O::Entry).unwrap_or(NONE);
        let dummy = Value::from_u32(0);
        Cg {
            b,
            ssa,
            f,
            kernel,
            w,
            pl,
            cc,
            callees,
            blocks: Vec::new(),
            vals: vec![Rp::None; f.insts.len()],
            sg: dummy,
            fp: dummy,
            lane0: dummy,
            env: dummy,
            entry_v,
            slot: None,
            raw: HashMap::new(),
            frame_of: HashMap::new(),
            steps: HashMap::new(),
            held: Vec::new(),
            idom: Vec::new(),
            below: HashMap::new(),
            counted: HashMap::new(),
            count_tests: HashMap::new(),
            block_of: Vec::new(),
            depth: Vec::new(),
            preds: Vec::new(),
            splats: HashMap::new(),
            masks64: HashMap::new(),
            entry_bits: dummy,
            helper_sigs: HashMap::new(),
            chunked: false,
            group_dec: Vec::new(),
            cur_block: 0,
            full: false,
            ones: std::collections::HashSet::new(),
            lab: Vec::new(),
            resume: Vec::new(),
            live_out: Vec::new(),
            reg_ids: HashMap::new(),
            reg_blocks: Vec::new(),
            over: Vec::new(),
            applied: Vec::new(),
            region: 0,
            saved: HashMap::new(),
            save_size: 0,
            save_slot: None,
            save_base: dummy,
            st: None,
            chunk: dummy,
            next: None,
            cur_inst: NONE,
            split_n: 0,
            err: None,
        }
    }

    fn function(&mut self) -> R<()> {
        let f = self.f;
        let w = self.w;
        self.blocks = f.blocks.iter().map(|_| self.b.create_block()).collect();
        // A chunked kernel's entry is its chunk loop's (`chunk_loop`), and its first region its first block.
        let e = if self.chunked { self.b.create_block() } else { self.blocks[0] };
        self.b.append_block_params_for_function_params(e);
        // Each block's parameters, by their shapes.
        for (k, blk) in f.blocks.iter().enumerate() {
            for &p in &blk.params {
                let i = f.inst(p);
                let r = match i.shape {
                    Shape::Uniform => Rp::U(self.b.append_block_param(self.blocks[k], uty(i.m))),
                    Shape::Lanes => {
                        let (n, t) = parts(i.m, w);
                        Rp::L((0..n).map(|_| self.b.append_block_param(self.blocks[k], t)).collect())
                    }
                    Shape::Mask => Rp::K((0..mask_parts(w)).map(|_| self.b.append_block_param(self.blocks[k], mask_ty(w))).collect()),
                    Shape::None => Rp::None,
                };
                self.vals[p as usize] = r;
            }
        }
        self.b.switch_to_block(e);
        let ps: Vec<Value> = self.b.block_params(e).to_vec();
        self.sg = ps[0];
        self.fp = ps[1];
        self.env = self.b.ins().load(I64, mf(), self.sg, self.pl.env);
        // A SIMD group of 32 lanes at 32 is one chunk, its first lane 0.
        self.lane0 = if w == 32 { self.b.ins().iconst(I64, 0) } else { ps[2] };
        let mp = mask_parts(w);
        if self.full {
            // Every lane of the chunk: as `mask_of_bits` of all its lanes' bits.
            self.ones.insert(self.entry_v);
        }
        let mut dispatch = None;
        let entry_mask: Vec<Value> = if self.chunked {
            let (mask, d) = self.chunk_loop();
            dispatch = Some(d);
            mask
        } else if self.full {
            self.all_mask()
        } else {
            ps[3..3 + mp].to_vec()
        };
        if !self.chunked {
            self.entry_bits = self.mask_bits(&entry_mask);
        }
        let mut at = 3 + mp;
        let mut args = Vec::new();
        if !self.kernel {
            for &(m, sh) in &f.args {
                let r = match sh {
                    Shape::Uniform => {
                        at += 1;
                        Rp::U(ps[at - 1])
                    }
                    Shape::Lanes => {
                        let (n, _) = parts(m, w);
                        at += n;
                        Rp::L(ps[at - n..at].to_vec())
                    }
                    _ => Rp::None,
                };
                args.push(r);
            }
        }
        self.idom = crate::ssa::idoms(f);
        self.bounds_of_branches();
        self.loops();
        if self.chunked {
            self.regions();
            let b0 = self.blocks[0];
            self.reg_blocks.push(b0);
            self.over.push(Vec::new());
            self.reg_ids.insert(crate::ssa::Lab::Start, 0);
        } else {
            self.resume = vec![false; f.blocks.len()];
        }
        self.counted_loops();
        let order = crate::ssa::rpo(f);
        for &bk in &order {
            if self.chunked {
                self.start_block(bk);
            } else if bk != 0 {
                self.b.switch_to_block(self.blocks[bk]);
            }
            for &p in &f.blocks[bk].params {
                if let Some(&(n, k)) = self.counted.get(&p) {
                    self.vals[p as usize] = Rp::U(self.b.ins().iadd_imm(n, k));
                }
            }
            self.splats.clear();
            self.masks64.clear();
            self.cur_block = bk;
            let blk = &f.blocks[bk];
            for &x in &blk.insts {
                let i = *f.inst(x);
                (self.cur_inst, self.split_n) = (x, 0);
                self.b.set_srcloc(ir::SourceLoc::new(x));
                // Held stores go before anything that reads or writes memory, or leaves the block.
                let pure = matches!(
                    i.op,
                    O::Const | O::Arg | O::Entry | O::NoLanes | O::Undef | O::Param | O::Bin | O::Un | O::Cast | O::Select
                        | O::First | O::MaskOf | O::MAnd | O::MOr | O::MAndNot | O::Any | O::Global | O::Shared
                        | O::FrameAddr | O::Lid | O::Gid | O::Lane | O::GroupInfo | O::LaneOp
                );
                if !pure && i.op != O::Store {
                    self.release_held();
                }
                let r = match i.op {
                    O::Entry => Rp::K(entry_mask.clone()),
                    O::Arg if !self.kernel => args[i.a as usize].clone(),
                    _ => self.inst(x, &i)?,
                };
                self.vals[x as usize] = r;
            }
        }
        if let Some(d) = dispatch {
            // The region every chunk runs next: the regions' first blocks by their numbers.
            self.b.switch_to_block(d);
            let rs = self.b.ins().stack_load(I32, self.st.expect("a chunked kernel's state"), 0);
            let mut sw = cranelift_frontend::Switch::new();
            for (k, &blk) in self.reg_blocks.iter().enumerate().skip(1) {
                sw.set_entry(k as u128, blk);
            }
            sw.emit(&mut self.b, rs, self.reg_blocks[0]);
            let slot = self.save_slot.expect("a chunked kernel's save area");
            self.b.func.sized_stack_slots[slot].size = self.save_size.max(16);
        }
        match self.err.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    // Values.

    fn u(&self, x: V) -> Value {
        match &self.vals[x as usize] {
            Rp::U(v) => *v,
            r => panic!("value {x} ({:?}) is not uniform: {r:?}", self.f.inst(x).op),
        }
    }

    fn m_of(&self, x: V) -> M {
        self.f.inst(x).m
    }

    /// `x`'s lanes as type `m`'s parts (a uniform value splatted).
    fn lanes(&mut self, x: V, m: M) -> Vec<Value> {
        match self.vals[x as usize].clone() {
            Rp::L(p) => p,
            Rp::U(u) => {
                let key = (x, m.0);
                if let Some(p) = self.splats.get(&key) {
                    return p.clone();
                }
                let p = self.splat(u, m);
                self.splats.insert(key, p.clone());
                p
            }
            Rp::K(k) => k,
            Rp::None => panic!("value {x} ({:?}) has no registers", self.f.inst(x).op),
        }
    }

    /// Uniform `u` (of `m`) in every lane.
    fn splat(&mut self, u: Value, m: M) -> Vec<Value> {
        let (n, t) = parts(m, self.w);
        let s = self.lane_scalar(u, m);
        let v = if self.w == 1 { s } else { self.b.ins().splat(t, s) };
        vec![v; n]
    }

    /// A uniform value as a lane's scalar (an integer of 32 bits or fewer truncated to 32).
    fn lane_scalar(&mut self, u: Value, m: M) -> Value {
        if m.float() || wide(m) || m.is_agg() { u } else { self.b.ins().ireduce(I32, u) }
    }

    /// A lane's scalar as a uniform value.
    fn uniform_of(&mut self, s: Value, m: M) -> Value {
        if m.float() || wide(m) {
            return s;
        }
        if m.signed() { self.b.ins().sextend(I64, s) } else { self.b.ins().uextend(I64, s) }
    }

    /// Whether mask `x` has every lane of the chunk (`ones`).
    fn all(&self, x: V) -> bool {
        x != NONE && self.ones.contains(&x)
    }

    /// If mask `x` is `lane == u`'s under every lane (`Func::lane_is`), `u` uniform: `u`.
    fn lane_test(&self, x: V) -> Option<V> {
        let i = *self.f.inst(x);
        match i.op {
            O::MaskOf => self.f.lane_is(i.a),
            O::MAnd if self.all(i.a) => self.lane_test(i.b),
            O::MAnd if self.all(i.b) => self.lane_test(i.a),
            _ => None,
        }
    }

    fn mask(&self, x: V) -> Vec<Value> {
        match &self.vals[x as usize] {
            Rp::K(k) => k.clone(),
            r => panic!("value {x} ({:?}) is not a mask: {r:?}", self.f.inst(x).op),
        }
    }

    /// Mask `x` for type `m`'s lanes: its parts, widened to 64-bit lanes for a 64-bit type.
    fn mask_for(&mut self, x: V, m: M) -> Vec<Value> {
        let k = self.mask(x);
        if !wide(m) || self.w == 1 {
            if self.w == 1 && wide(m) {
                return vec![self.b.ins().sextend(I64, k[0])];
            }
            return k;
        }
        if let Some(p) = self.masks64.get(&x) {
            return p.clone();
        }
        let (n, _) = parts(m, self.w);
        let p: Vec<Value> = (0..n).map(|j| if j % 2 == 0 { self.b.ins().swiden_low(k[j / 2]) } else { self.b.ins().swiden_high(k[j / 2]) }).collect();
        self.masks64.insert(x, p.clone());
        p
    }

    /// A mask's lanes as bits of an `I32` (bit `j`: lane `j` of the chunk).
    fn mask_bits(&mut self, k: &[Value]) -> Value {
        if self.w == 1 {
            return self.b.ins().band_imm(k[0], 1);
        }
        let mut acc = None;
        for (p, &x) in k.iter().enumerate() {
            let h = self.b.ins().vhigh_bits(I32, x);
            let h = if p == 0 { h } else { self.b.ins().ishl_imm(h, 4 * p as i64) };
            acc = Some(match acc {
                None => h,
                Some(a) => self.b.ins().bor(a, h),
            });
        }
        let a = acc.expect("a mask's parts");
        if self.w < 4 { self.b.ins().band_imm(a, (1i64 << self.w) - 1) } else { a }
    }

    /// Whether mask `k` has a lane, a uniform `bool`.
    fn any(&mut self, k: &[Value]) -> Value {
        let c = if self.w == 1 {
            self.b.ins().icmp_imm(IntCC::NotEqual, k[0], 0)
        } else if self.w < 4 {
            let bits = self.mask_bits(k);
            self.b.ins().icmp_imm(IntCC::NotEqual, bits, 0)
        } else {
            let mut a = k[0];
            for &x in &k[1..] {
                a = self.b.ins().bor(a, x);
            }
            self.b.ins().vany_true(a)
        };
        self.b.ins().uextend(I64, c)
    }

    fn konst_u(&mut self, m: M, x: u64) -> Value {
        match m {
            M::F32 => self.b.ins().f32const(f32::from_bits(x as u32)),
            M::F64 => self.b.ins().f64const(f64::from_bits(x)),
            _ => self.b.ins().iconst(I64, crate::ops::norm(m, x) as i64),
        }
    }

    fn zero_lanes(&mut self, m: M) -> Vec<Value> {
        let z = self.konst_u(m, 0);
        self.splat(z, m)
    }

    fn zero_mask(&mut self) -> Vec<Value> {
        let w = self.w;
        if w == 1 {
            return vec![self.b.ins().iconst(I32, 0)];
        }
        let z = vconst32(&mut self.b, [0; 4]);
        vec![z; mask_parts(w)]
    }

    /// Lane `j` of parts `p` (of `m`).
    fn extract(&mut self, p: &[Value], m: M, j: u32) -> Value {
        if self.w == 1 {
            return p[0];
        }
        let per = if wide(m) { 2 } else { 4 };
        self.b.ins().extractlane(p[(j / per) as usize], (j % per) as u8)
    }

    /// Parts of `m` with lane `j` of each set to `xs[j]` (lane scalars).
    fn assemble(&mut self, m: M, xs: &[Value]) -> Vec<Value> {
        if self.w == 1 {
            return vec![xs[0]];
        }
        let (n, t) = parts(m, self.w);
        let per = if wide(m) { 2 } else { 4 };
        let mut out = Vec::with_capacity(n);
        for p in 0..n {
            let first = xs[(p as u32 * per) as usize];
            let mut v = self.b.ins().splat(t, first);
            for l in 1..per {
                let j = p as u32 * per + l;
                if (j as usize) < xs.len() {
                    v = self.b.ins().insertlane(v, xs[j as usize], l as u8);
                }
            }
            out.push(v);
        }
        out
    }

    /// Mask lane `j` as a `bool` (`I8`).
    /// A uniform address `a` that the lanes of `mask` read or write: the scratch area's if none of them is in this
    /// chunk (another chunk's are), so a chunk with no lane touches nothing.
    fn guarded(&mut self, a: Value, mask: V) -> Value {
        if mask == NONE || (mask == self.entry_v && self.kernel) || self.all(mask) {
            return a;
        }
        let k = self.mask(mask);
        let any = self.any(&k);
        let any = self.cond(any);
        let scratch = self.b.ins().load(I64, mf(), self.env, self.pl.scratch);
        self.b.ins().select(any, a, scratch)
    }

    /// The scratch slot (32 lanes of 8 bytes), made at its first use.
    fn slot(&mut self) -> ir::StackSlot {
        *self.slot.get_or_insert_with(|| self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 32 * 8, 4)))
    }

    fn mask_lane(&mut self, k: &[Value], j: u32) -> Value {
        let x = if self.w == 1 { k[0] } else { self.b.ins().extractlane(k[(j / 4) as usize], (j % 4) as u8) };
        self.b.ins().icmp_imm(IntCC::NotEqual, x, 0)
    }

    /// The number of lanes a value has at this width.
    fn n_lanes(&self) -> u32 {
        self.w
    }

    // The runtime.

    fn helper(&mut self, h: i32, params: usize, rets: usize, args: &[Value]) -> Vec<Value> {
        let sig = *self.helper_sigs.entry(params * 8 + rets).or_insert_with(|| {
            let mut s = Signature::new(self.cc);
            for _ in 0..params {
                s.params.push(AbiParam::new(I64));
            }
            for _ in 0..rets {
                s.returns.push(AbiParam::new(I64));
            }
            self.b.import_signature(s)
        });
        let at = self.b.ins().load(I64, mf(), self.env, self.pl.helpers + 8 * h);
        let call = self.b.ins().call_indirect(sig, at, args);
        self.b.inst_results(call).to_vec()
    }


    // A chunk's exchanges with its SIMD group's other chunks (`chunked`): it writes its lanes' part of the
    // exchange area's half, its region ends (`x_wait`), and once every chunk has written its part, the SIMD group
    // swaps the halves and each chunk reads the whole SIMD group's in the next region.

    /// The exchange area's half the chunks write.
    fn x_half(&mut self) -> Value {
        let par = self.b.ins().uload32(mf(), self.sg, self.pl.xpar);
        self.x_at(par)
    }

    /// The half the chunks wrote, after `x_wait`.
    fn x_read(&mut self) -> Value {
        let par = self.b.ins().uload32(mf(), self.sg, self.pl.xpar);
        let was = self.b.ins().bxor_imm(par, 1);
        self.x_at(was)
    }

    fn x_at(&mut self, par: Value) -> Value {
        let buf = self.b.ins().load(I64, mf(), self.sg, self.pl.xbuf);
        let off = self.b.ins().imul_imm(par, i64::from(XHALF));
        self.b.ins().iadd(buf, off)
    }

    fn x_wait(&mut self) {
        self.split(ACT_X);
    }

    // A chunked kernel's regions (decision 0567, step 3). The kernel's function runs its SIMD group's chunks
    // itself: a region of its code (from its start, from an exchange or barrier, from a join of different regions)
    // runs for each chunk in turn, up to the next region's start, where the chunk keeps the values live there in the
    // save area (its slot of each); once every chunk is there, the SIMD group does what is done between (`ACT_*`), and
    // runs the next region for each chunk, which reads them back. No chunk runs on a stack of its own.

    /// The chunk loop around the regions, at the function's entry: its state, each chunk's first lane and lanes,
    /// and between the regions' turns what the SIMD group does. Its mask of the chunk's lanes, and the block that
    /// goes to the region to run (`dispatch`, made last).
    fn chunk_loop(&mut self) -> (Vec<Value>, ir::Block) {
        let w = self.w;
        let st = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 3));
        let save = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 4));
        (self.st, self.save_slot) = (Some(st), Some(save));
        self.save_base = self.b.ins().stack_addr(I64, save, 0);
        let (zero, none, zero64) = (self.b.ins().iconst(I32, 0), self.b.ins().iconst(I32, NEXT_NONE), self.b.ins().iconst(I64, 0));
        self.b.ins().stack_store(zero, st, 0);
        self.b.ins().stack_store(none, st, 4);
        self.b.ins().stack_store(zero64, st, 8);
        let [round, each, body, disp, next, end] = std::array::from_fn(|_| self.b.create_block());
        self.b.append_block_param(each, I64);
        self.b.append_block_param(next, I64);
        self.b.ins().jump(round, &[]);
        self.b.switch_to_block(round);
        let c0 = self.b.ins().iconst(I64, 0);
        self.b.ins().jump(each, &[BlockArg::Value(c0)]);
        self.b.switch_to_block(each);
        let c = self.b.block_params(each)[0];
        self.chunk = c;
        self.lane0 = self.b.ins().ishl_imm(c, i64::from(w.trailing_zeros()));
        let bits = if self.full {
            self.b.ins().jump(body, &[]);
            None
        } else {
            // A chunk with no lane of the SIMD group's has nothing to run, and nothing to give an exchange.
            let live = self.b.ins().uload32(mf(), self.sg, self.pl.live);
            let sh = self.b.ins().ushr(live, self.lane0);
            let bits = self.b.ins().band_imm(sh, (1i64 << w) - 1);
            self.b.ins().brif(bits, body, &[], next, &[BlockArg::Value(c)]);
            Some(bits)
        };
        self.b.switch_to_block(body);
        let mask = match bits {
            None => self.all_mask(),
            Some(bits) => {
                let b32 = self.b.ins().ireduce(I32, bits);
                mask_of_bits(&mut self.b, b32, w)
            }
        };
        self.entry_bits = self.mask_bits(&mask);
        self.b.ins().jump(disp, &[]);
        self.b.switch_to_block(next);
        let c = self.b.block_params(next)[0];
        let c1 = self.b.ins().iadd_imm(c, 1);
        let more = self.b.ins().icmp_imm(IntCC::UnsignedLessThan, c1, i64::from(32 / w));
        self.b.ins().brif(more, each, &[BlockArg::Value(c1)], end, &[]);
        self.b.switch_to_block(end);
        let nx = self.b.ins().stack_load(I32, st, 4);
        let act = self.b.ins().stack_load(I64, st, 8);
        let a = self.b.ins().band_imm(act, 0xff);
        let [swap, not_x, bar, not_bar, poll, fin, ret, again] = std::array::from_fn(|_| self.b.create_block());
        let is = self.b.ins().icmp_imm(IntCC::Equal, a, ACT_X);
        self.b.ins().brif(is, swap, &[], not_x, &[]);
        self.b.switch_to_block(swap);
        let par = self.b.ins().uload32(mf(), self.sg, self.pl.xpar);
        let par = self.b.ins().bxor_imm(par, 1);
        self.b.ins().istore32(mf(), par, self.sg, self.pl.xpar);
        self.b.ins().jump(fin, &[]);
        self.b.switch_to_block(not_x);
        let is = self.b.ins().icmp_imm(IntCC::Equal, a, ACT_BAR);
        self.b.ins().brif(is, bar, &[], not_bar, &[]);
        self.b.switch_to_block(bar);
        let site = self.b.ins().ushr_imm(act, 8);
        let sg = self.sg;
        self.helper(H_BAR, 2, 0, &[sg, site]);
        self.b.ins().jump(fin, &[]);
        self.b.switch_to_block(not_bar);
        let is = self.b.ins().icmp_imm(IntCC::Equal, a, ACT_YIELD);
        self.b.ins().brif(is, poll, &[], fin, &[]);
        self.b.switch_to_block(poll);
        self.helper(H_YIELD, 1, 0, &[sg]);
        self.b.ins().jump(fin, &[]);
        self.b.switch_to_block(fin);
        let done = self.b.ins().icmp_imm(IntCC::Equal, nx, NEXT_END);
        self.b.ins().brif(done, ret, &[], again, &[]);
        self.b.switch_to_block(ret);
        self.b.ins().return_(&[]);
        self.b.switch_to_block(again);
        self.b.ins().stack_store(nx, st, 0);
        self.b.ins().stack_store(none, st, 4);
        self.b.ins().stack_store(zero64, st, 8);
        self.b.ins().jump(round, &[]);
        self.next = Some(next);
        (mask, disp)
    }

    /// The mask of every lane of the chunk, a constant.
    fn all_mask(&mut self) -> Vec<Value> {
        if self.w == 1 {
            vec![self.b.ins().iconst(I32, -1)]
        } else {
            let lanes: Vec<u8> = (0..4u32).flat_map(|j| if j < self.w { [0xffu8; 4] } else { [0u8; 4] }).collect();
            let c = self.b.func.dfg.constants.insert(lanes.into());
            let v = self.b.ins().vconst(I32X4, c);
            vec![v; mask_parts(self.w)]
        }
    }

    /// Each block's region at its start, the blocks that start one (`ssa::regions`), and the values live at each
    /// block's end.
    fn regions(&mut self) {
        (self.lab, self.resume) = crate::ssa::regions(self.f, &self.group_dec);
        self.live_out = crate::ssa::live_out(self.f);
    }

    fn remade(&self, x: V) -> bool {
        crate::ssa::remade(self.f, x)
    }

    /// The values live before instruction `at` of block `b` (its end for `at` past the last).
    fn live_before(&self, b: usize, at: usize) -> Vec<bool> {
        let mut l = self.live_out[b].clone();
        for &y in self.f.blocks[b].insts[at..].iter().rev() {
            l[y as usize] = false;
            for u in crate::ssa::reads(self.f, y) {
                l[u as usize] = true;
            }
        }
        l
    }

    /// The values a region kept: those of `live` with registers (the function's lanes are the chunk loop's).
    fn kept(&self, live: &[bool]) -> Vec<V> {
        (0..live.len() as V)
            .filter(|&v| live[v as usize] && v != self.entry_v && self.f.inst(v).shape != Shape::None && !matches!(self.vals[v as usize], Rp::None))
            .collect()
    }

    /// The values a region starting at block `b` keeps: those live at its start but its parameters.
    fn live_in(&self, b: usize) -> Vec<V> {
        let mut l = self.live_before(b, 0);
        for &p in &self.f.blocks[b].params {
            l[p as usize] = false;
        }
        self.kept(&l)
    }

    /// The values a region starting after instruction `x` keeps: those live after it, and its operands (its part
    /// after the region's start reads them).
    fn live_after(&self, x: V) -> Vec<V> {
        let b = self.block_of[x as usize] as usize;
        let at = self.f.blocks[b].insts.iter().position(|&y| y == x).expect("in its block");
        let mut l = self.live_before(b, at + 1);
        for u in crate::ssa::reads(self.f, x) {
            l[u as usize] = true;
        }
        l[x as usize] = false;
        self.kept(&l)
    }

    /// Region `lab`'s number, made at its first use (its first block, its values).
    fn reg(&mut self, lab: crate::ssa::Lab) -> usize {
        if let Some(&k) = self.reg_ids.get(&lab) {
            return k;
        }
        let k = self.reg_blocks.len();
        let blk = self.b.create_block();
        self.reg_blocks.push(blk);
        self.over.push(Vec::new());
        self.reg_ids.insert(lab, k);
        k
    }

    /// The chunk's region ends: region `k` runs next (or none, `NEXT_END`), after what the SIMD group does then
    /// (`act`). Every chunk names the same one.
    fn leave(&mut self, k: i64, act: i64) {
        let st = self.st.expect("a chunked kernel's state");
        let kv = self.b.ins().iconst(I32, k);
        let prev = self.b.ins().stack_load(I32, st, 4);
        let named = self.b.ins().icmp_imm(IntCC::NotEqual, prev, NEXT_NONE);
        let other = self.b.ins().icmp(IntCC::NotEqual, prev, kv);
        let bad = self.b.ins().band(named, other);
        self.fail_if(bad, FAIL_PLACES);
        self.b.ins().stack_store(kv, st, 4);
        let a = self.b.ins().iconst(I64, act);
        self.b.ins().stack_store(a, st, 8);
        let (next, c) = (self.next.expect("the chunk loop"), self.chunk);
        self.b.ins().jump(next, &[BlockArg::Value(c)]);
    }

    /// The region ends at the instruction being made, and the next starts after it (its part after the exchange,
    /// what follows): `act` done between.
    fn split(&mut self, act: i64) {
        let x = self.cur_inst;
        let lab = crate::ssa::Lab::Split(x, self.split_n);
        self.split_n += 1;
        let live = self.live_after(x);
        let k = self.reg(lab);
        self.save(&live);
        self.leave(k as i64, act);
        self.b.switch_to_block(self.reg_blocks[k]);
        self.begin(k, &live);
    }

    /// The values `live` kept in the save area, the chunk's slot of each.
    fn save(&mut self, live: &[V]) {
        for &v in live {
            if self.remade(v) {
                continue;
            }
            let (kind, vs) = match &self.vals[v as usize] {
                Rp::U(x) => (0, vec![*x]),
                Rp::L(p) => (1, p.clone()),
                Rp::K(p) => (2, p.clone()),
                Rp::None => continue,
            };
            self.save_parts(v, kind, &vs);
        }
    }

    fn save_parts(&mut self, v: V, kind: u8, vs: &[Value]) {
        let tys: Vec<ir::Type> = vs.iter().map(|&x| self.b.func.dfg.value_type(x)).collect();
        let n = vs.len() as u32;
        let off = match self.saved.get(&v) {
            Some((o, k, t)) => {
                if *k != kind || *t != tys {
                    self.err.get_or_insert(Error(format!("value {v} kept as {kind} {tys:?}, and before as {k} {t:?}")));
                }
                *o
            }
            None => {
                let o = self.save_size as i32;
                self.save_size += 16 * n * (32 / self.w);
                self.saved.insert(v, (o, kind, tys));
                o
            }
        };
        let base = self.save_at(n);
        for (j, &x) in vs.iter().enumerate() {
            self.b.ins().store(mf(), x, base, off + 16 * j as i32);
        }
    }

    /// The chunk's place in the save area of a value of `n` parts.
    fn save_at(&mut self, n: u32) -> Value {
        let o = self.b.ins().imul_imm(self.chunk, i64::from(16 * n));
        self.b.ins().iadd(self.save_base, o)
    }

    /// Value `v` read back from the save area.
    fn reload(&mut self, v: V) -> Option<Rp> {
        let Some((off, kind, tys)) = self.saved.get(&v).cloned() else {
            self.err.get_or_insert(Error(format!("value {v} ({:?}) is read in a region no region kept it for", self.f.inst(v).op)));
            return None;
        };
        let base = self.save_at(tys.len() as u32);
        let vs: Vec<Value> = tys.iter().enumerate().map(|(j, &t)| self.b.ins().load(t, mf(), base, off + 16 * j as i32)).collect();
        Some(match kind {
            0 => Rp::U(vs[0]),
            1 => Rp::L(vs),
            _ => Rp::K(vs),
        })
    }

    /// Region `k` starts (in its first block): the values `live` read back, or made again.
    fn begin(&mut self, k: usize, live: &[V]) {
        self.undo();
        self.region = k;
        self.splats.clear();
        self.masks64.clear();
        for &v in live {
            if self.remade(v) {
                self.remake(v);
            } else if let Some(rp) = self.reload(v) {
                let old_st = self.steps.remove(&v);
                let old_raw = self.raw.remove(&v);
                let old = std::mem::replace(&mut self.vals[v as usize], rp.clone());
                self.applied.push((v, old, old_st, old_raw));
                self.over[k].push((v, rp, None));
            }
        }
    }

    /// Value `v` made again in the region starting (`remade`), its operands first.
    fn remake(&mut self, v: V) {
        let k = self.region;
        if self.over[k].iter().any(|(x, ..)| *x == v) {
            return;
        }
        for u in crate::ssa::reads(self.f, v) {
            self.remake(u);
        }
        let i = *self.f.inst(v);
        let old_st = self.steps.remove(&v);
        let old_raw = self.raw.remove(&v);
        match self.inst(v, &i) {
            Ok(rp) => {
                let old = std::mem::replace(&mut self.vals[v as usize], rp.clone());
                self.applied.push((v, old, old_st, old_raw));
                let st = self.steps.get(&v).cloned();
                self.over[k].push((v, rp, st));
            }
            Err(e) => {
                self.err.get_or_insert(e);
            }
        }
    }

    /// The values the region being made holds in place of others' put back.
    fn undo(&mut self) {
        while let Some((v, old, st, raw)) = self.applied.pop() {
            self.vals[v as usize] = old;
            match st {
                Some(s) => self.steps.insert(v, s),
                None => self.steps.remove(&v),
            };
            match raw {
                Some(r) => self.raw.insert(v, r),
                None => self.raw.remove(&v),
            };
        }
    }

    /// Region `k`'s values in place, to make another of its blocks.
    fn enter(&mut self, k: usize) {
        if self.region == k {
            return;
        }
        self.undo();
        self.region = k;
        self.splats.clear();
        self.masks64.clear();
        for (v, rp, st) in self.over[k].clone() {
            let old_st = self.steps.remove(&v);
            let old_raw = self.raw.remove(&v);
            if let Some(st) = st {
                self.steps.insert(v, st);
            }
            let old = std::mem::replace(&mut self.vals[v as usize], rp);
            self.applied.push((v, old, old_st, old_raw));
        }
    }

    /// Block `b` begins: in its region's values, or as a region of its own, which reads back its values and its
    /// parameters.
    fn start_block(&mut self, b: usize) {
        if self.resume[b] {
            let k = self.reg(crate::ssa::Lab::Join(b));
            self.b.switch_to_block(self.reg_blocks[k]);
            let live = self.live_in(b);
            self.begin(k, &live);
            let mut args = Vec::new();
            for &p in &self.f.blocks[b].params.clone() {
                match self.reload(p) {
                    Some(Rp::U(x)) => args.push(x),
                    Some(Rp::L(xs) | Rp::K(xs)) => args.extend(xs),
                    _ => {}
                }
            }
            let ba: Vec<BlockArg> = args.into_iter().map(BlockArg::Value).collect();
            self.b.ins().jump(self.blocks[b], &ba);
        } else {
            let lab = self.lab[b].expect("a reached block's region");
            let k = self.reg_ids[&lab];
            self.enter(k);
        }
        self.b.switch_to_block(self.blocks[b]);
    }

    /// The function's return where its SIMD group failed (`fail`): its results 0.
    fn ret_failed(&mut self) {
        let rets: Vec<ir::Type> = self.b.func.signature.returns.iter().map(|a| a.value_type).collect();
        let vs: Vec<Value> = rets
            .into_iter()
            .map(|t| {
                if t.is_vector() {
                    let c = self.b.func.dfg.constants.insert(vec![0u8; 16].into());
                    self.b.ins().vconst(t, c)
                } else if t == F32 {
                    self.b.ins().f32const(0.0)
                } else if t == F64 {
                    self.b.ins().f64const(0.0)
                } else {
                    self.b.ins().iconst(t, 0)
                }
            })
            .collect();
        self.b.ins().return_(&vs);
    }

    /// A return where a helper or a function called failed the SIMD group.
    fn check_failed(&mut self) {
        let failed = self.b.ins().uload32(mf(), self.sg, self.pl.failed);
        let (bad, go) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(failed, bad, &[], go, &[]);
        self.b.switch_to_block(bad);
        self.ret_failed();
        self.b.switch_to_block(go);
        self.splats.clear();
        self.masks64.clear();
    }

    /// The chunk's lanes of mask `k` as its lanes' bits of the half's word.
    fn x_put_bits(&mut self, at: Value, k: &[Value]) {
        let bits = self.mask_bits(k);
        let word = self.b.ins().load(I32, mf(), at, XWORD);
        let l0 = self.b.ins().ireduce(I32, self.lane0);
        let ours = self.b.ins().iconst(I32, ((1u64 << self.w) - 1) as i64);
        let ours = self.b.ins().ishl(ours, l0);
        let keep = self.b.ins().band_not(word, ours);
        let mine = self.b.ins().ishl(bits, l0);
        let word = self.b.ins().bor(keep, mine);
        self.b.ins().store(mf(), word, at, XWORD);
    }

    /// The chunk's lanes of a value of `m` (parts `p`) at their lanes' places of the half (4 or 8 bytes a lane).
    fn x_put_lanes(&mut self, at: Value, p: &[Value], m: M) {
        let size: i64 = if wide(m) { 8 } else { 4 };
        let off = self.b.ins().imul_imm(self.lane0, size);
        let base = self.b.ins().iadd(at, off);
        let per = 16 / size as u32;
        if self.w >= per {
            for (k, &v) in p.iter().enumerate() {
                self.b.ins().store(mf(), v, base, 16 * k as i32);
            }
        } else {
            for j in 0..self.w {
                let v = self.extract(p, m, j);
                self.b.ins().store(mf(), v, base, (j as i64 * size) as i32);
            }
        }
    }

    /// The SIMD group's decision `v` (uniform) under mask `mask`: the value of the chunk that has the group's first
    /// lane of the mask (none in any chunk: the first chunk's), so every chunk takes it.
    fn x_leader(&mut self, v: Value, mask: V) -> Value {
        let k = if mask == NONE { self.mask(self.entry_v) } else { self.mask(mask) };
        let at = self.x_half();
        let t = self.b.func.dfg.value_type(v);
        let v64 = if t == I64 { v } else if t.is_float() { self.b.ins().bitcast(if t == F32 { I32 } else { I64 }, MemFlags::new(), v) } else { self.b.ins().uextend(I64, v) };
        let v64 = if self.b.func.dfg.value_type(v64) == I32 { self.b.ins().uextend(I64, v64) } else { v64 };
        let off = self.b.ins().imul_imm(self.lane0, 8);
        let mine = self.b.ins().iadd(at, off);
        self.b.ins().store(mf(), v64, mine, 0);
        self.x_put_bits(at, &k);
        self.x_wait();
        let at = self.x_read();
        let word = self.b.ins().load(I32, mf(), at, XWORD);
        let lead = self.x_lead(word);
        let off = self.b.ins().imul_imm(lead, 8);
        let theirs = self.b.ins().iadd(at, off);
        let r = self.b.ins().load(I64, mf(), theirs, 0);
        if t == I64 {
            r
        } else if t.is_float() {
            let r = if t == F32 { self.b.ins().ireduce(I32, r) } else { r };
            self.b.ins().bitcast(t, MemFlags::new(), r)
        } else {
            self.b.ins().ireduce(t, r)
        }
    }

    /// The first lane of the chunk that has the group's first lane of `word`'s (0 for none), an `I64`.
    fn x_lead(&mut self, word: Value) -> Value {
        let none = self.b.ins().icmp_imm(IntCC::Equal, word, 0);
        let first = self.b.ins().ctz(word);
        let first = self.b.ins().band_imm(first, !(i64::from(self.w) - 1) & 31);
        let zero = self.b.ins().iconst(I32, 0);
        let first = self.b.ins().select(none, zero, first);
        self.b.ins().uextend(I64, first)
    }

    /// Whether the SIMD group has a lane of mask `mask` (each chunk's lanes of it), a uniform `bool`.
    fn x_any(&mut self, mask: V) -> Value {
        let k = self.mask(mask);
        let at = self.x_half();
        self.x_put_bits(at, &k);
        self.x_wait();
        let at = self.x_read();
        let word = self.b.ins().load(I32, mf(), at, XWORD);
        let c = self.b.ins().icmp_imm(IntCC::NotEqual, word, 0);
        self.b.ins().uextend(I64, c)
    }

    /// The SIMD group's lanes of mask `mask`, their bits (each chunk's lanes of it), an `I32`.
    fn x_bits(&mut self, mask: V) -> Value {
        let k = self.mask(mask);
        let at = self.x_half();
        self.x_put_bits(at, &k);
        self.x_wait();
        let at = self.x_read();
        self.b.ins().load(I32, mf(), at, XWORD)
    }

    /// A branch's decision `c` (of block `cur_block`'s `Br` or `Switch`, its mask `mask`), the SIMD group's where
    /// the function runs a chunk and what it branches to reaches the group's operations: a lane of a mask any of the
    /// group's chunks has; an agreed one or a constant as it is; any other value the leading chunk's.
    fn decided(&mut self, c: V, mask: V) -> Value {
        let v = self.u(c);
        if !self.chunked || !self.group_dec[self.cur_block] {
            return v;
        }
        let i = *self.f.inst(c);
        match i.op {
            O::Any => self.x_any(i.a),
            O::Agree | O::Const | O::AnyGroup => v,
            _ => self.x_leader(v, mask),
        }
    }

    /// A SIMD group reduction (`bi`: a sum, a minimum or a maximum) of `n` lanes of `m`, each lane's `(runs, value)`
    /// from `lane`: the running lanes' sum, or their least or greatest from the first running lane's (0 for none).
    fn fold(&mut self, bi: Builtin, m: M, n: u32, lane: &mut dyn FnMut(&mut Self, u32) -> (Value, Value)) -> Value {
        if bi == Builtin::SubGroupReduceAdd {
            let mut acc = self.konst_u(m, 0);
            for l in 0..n {
                let (on, v) = lane(self, l);
                let sum = self.u_bin(Op::Add, m, acc, v);
                acc = self.b.ins().select(on, sum, acc);
            }
            return acc;
        }
        let mut have = self.b.ins().iconst(I8, 0);
        let mut best = self.konst_u(m, 0);
        for l in 0..n {
            let (on, v) = lane(self, l);
            let less = self.u_bin(Op::Lt, m, v, best);
            let better = if bi == Builtin::SubGroupReduceMin { less } else { self.b.ins().bxor_imm(less, 1) };
            let better = self.b.ins().icmp_imm(IntCC::NotEqual, better, 0);
            let none = self.b.ins().icmp_imm(IntCC::Equal, have, 0);
            let take = self.b.ins().bor(none, better);
            let take = self.b.ins().band(take, on);
            best = self.b.ins().select(take, v, best);
            // `on` an `I8` (a comparison's).
            have = self.b.ins().bor(have, on);
        }
        best
    }

    /// An integer SIMD group reduction (`bi`) of lanes `px` (of `m`) under mask `mask`, as vector operations: the
    /// lanes that do not run made the operation's identity (0 for a sum, the type's greatest for a minimum, its least
    /// for a maximum), the parts combined pairwise, then the last part's lanes across (`horizontal`); 0 for no lane.
    fn reduce_lanes(&mut self, bi: Builtin, m: M, mask: V, px: Vec<Value>) -> Value {
        let t = self.b.func.dfg.value_type(px[0]);
        let (min, max) = (bi == Builtin::SubGroupReduceMin, bi == Builtin::SubGroupReduceMax);
        let parts: Vec<Value> = if self.all(mask) {
            px
        } else {
            let k = self.mask_for(mask, m);
            if !(min || max) {
                px.iter().zip(&k).map(|(&p, &c)| self.b.ins().band(p, c)).collect()
            } else {
                let lane_bits = if wide(m) { 64 } else { 32 };
                let id: i64 = match (min, m.signed()) {
                    (true, false) => if lane_bits == 64 { -1 } else { 0xffff_ffff },
                    (true, true) => if lane_bits == 64 { i64::MAX } else { i64::from(i32::MAX) },
                    (false, false) => 0,
                    (false, true) => if lane_bits == 64 { i64::MIN } else { i64::from(i32::MIN) },
                };
                let s = self.b.ins().iconst(if lane_bits == 64 { I64 } else { I32 }, if lane_bits == 64 { id } else { id as i32 as i64 });
                let id = self.b.ins().splat(t, s);
                px.iter().zip(&k).map(|(&p, &c)| self.b.ins().bitselect(c, p, id)).collect()
            }
        };
        let op = |g: &mut Self, a: Value, b: Value| match (min, max, m.signed()) {
            (true, _, true) => g.b.ins().smin(a, b),
            (true, _, false) => g.b.ins().umin(a, b),
            (_, true, true) => g.b.ins().smax(a, b),
            (_, true, false) => g.b.ins().umax(a, b),
            _ => g.b.ins().iadd(a, b),
        };
        let mut level = parts;
        while level.len() > 1 {
            level = level.chunks(2).map(|c| if c.len() == 2 { op(self, c[0], c[1]) } else { c[0] }).collect();
        }
        let v = if min || max {
            self.horizontal(level[0], &op)
        } else {
            // A sum's: pairwise sums (AArch64's `addp`, x86-64's `phaddd`), two for 32-bit lanes, one for 64-bit ones.
            let mut v = level[0];
            for _ in 0..if t == I64X2 { 1 } else { 2 } {
                v = self.b.ins().iadd_pairwise(v, v);
            }
            v
        };
        let lane = self.b.ins().extractlane(v, 0);
        let r = if t == I64X2 { lane } else if m.signed() { self.b.ins().sextend(I64, lane) } else { self.b.ins().uextend(I64, lane) };
        let r = norm_u(&mut self.b, m, r);
        if (min || max) && !self.all(mask) {
            // No lane running: 0.
            let k = self.mask(mask);
            let any = self.any(&k);
            let any = self.cond(any);
            let zero = self.b.ins().iconst(I64, 0);
            return self.b.ins().select(any, r, zero);
        }
        r
    }

    /// Vector `v`'s lanes combined by `op` into its lane 0: its halves, then (32-bit lanes) its quarters, each a
    /// shuffle of the vector by itself (AArch64's `ext`, x86-64's `pshufd`).
    fn horizontal(&mut self, v: Value, op: &dyn Fn(&mut Self, Value, Value) -> Value) -> Value {
        let t = self.b.func.dfg.value_type(v);
        let as_bytes = |g: &mut Self, x: Value| g.b.ins().bitcast(I8X16, le(), x);
        let mut v = v;
        let steps: &[[u8; 16]] = if t == I64X2 {
            &[[8, 9, 10, 11, 12, 13, 14, 15, 0, 1, 2, 3, 4, 5, 6, 7]]
        } else {
            &[[8, 9, 10, 11, 12, 13, 14, 15, 0, 1, 2, 3, 4, 5, 6, 7], [4, 5, 6, 7, 0, 1, 2, 3, 12, 13, 14, 15, 8, 9, 10, 11]]
        };
        for s in steps {
            let bv = as_bytes(self, v);
            let imm = self.b.func.dfg.immediates.push(ConstantData::from(s.to_vec()));
            let sh = self.b.ins().shuffle(bv, bv, imm);
            let sh = self.b.ins().bitcast(t, le(), sh);
            v = op(self, v, sh);
        }
        v
    }

    /// A branch to the kernel's failure (`fail(sg, what)`, then a return) if `c` is true.
    fn fail_if(&mut self, c: Value, what: i64) {
        let fail = self.b.create_block();
        let go = self.b.create_block();
        let c = self.cond(c);
        self.b.ins().brif(c, fail, &[], go, &[]);
        self.b.switch_to_block(fail);
        let wv = self.b.ins().iconst(I64, what);
        let sg = self.sg;
        self.helper(H_FAIL, 2, 0, &[sg, wv]);
        self.ret_failed();
        self.b.switch_to_block(go);
        self.splats.clear();
        self.masks64.clear();
    }

    /// A uniform `bool` as a branch's condition: the comparison it was made from, if it was.
    fn cond(&self, v: Value) -> Value {
        let dfg = &self.b.func.dfg;
        if let ir::ValueDef::Result(inst, 0) = dfg.value_def(v) {
            if let ir::InstructionData::Unary { opcode: ir::Opcode::Uextend, arg } = dfg.insts[inst] {
                if dfg.value_type(arg) == I8 {
                    return arg;
                }
            }
        }
        v
    }

    // Uniform operations (and a lane's, a lane at a time).

    fn truth_u(&mut self, m: M, a: Value) -> Value {
        let c = match m {
            M::F32 => {
                let z = self.b.ins().f32const(0.0);
                self.b.ins().fcmp(FloatCC::NotEqual, a, z)
            }
            M::F64 => {
                let z = self.b.ins().f64const(0.0);
                self.b.ins().fcmp(FloatCC::NotEqual, a, z)
            }
            _ => self.b.ins().icmp_imm(IntCC::NotEqual, a, 0),
        };
        self.b.ins().uextend(I64, c)
    }

    /// `a / x` or `a % x` (uniform values of `m`), the kernel failed on a divisor of 0 if `check` (else it is not 0).
    fn u_div(&mut self, op: Op, m: M, a: Value, x: Value, check: bool) -> Value {
        if check {
            let z = self.b.ins().icmp_imm(IntCC::Equal, x, 0);
            self.fail_if(z, if op == Op::Div { FAIL_DIV } else { FAIL_REM });
        }
        let b = &mut self.b;
        let r = if m.signed() {
            let neg1 = b.ins().icmp_imm(IntCC::Equal, x, -1);
            let one = b.ins().iconst(I64, 1);
            let safe = b.ins().select(neg1, one, x);
            if op == Op::Div {
                let q = b.ins().sdiv(a, safe);
                let n = b.ins().ineg(a);
                b.ins().select(neg1, n, q)
            } else {
                let r = b.ins().srem(a, safe);
                let z = b.ins().iconst(I64, 0);
                b.ins().select(neg1, z, r)
            }
        } else if op == Op::Div {
            b.ins().udiv(a, x)
        } else {
            b.ins().urem(a, x)
        };
        norm_u(&mut self.b, m, r)
    }

    fn u_bin(&mut self, op: Op, m: M, a: Value, x: Value) -> Value {
        let b = &mut self.b;
        if m.float() {
            let r = match op {
                Op::Add => b.ins().fadd(a, x),
                Op::Sub => b.ins().fsub(a, x),
                Op::Mul => b.ins().fmul(a, x),
                Op::Div => b.ins().fdiv(a, x),
                _ => {
                    let cc = match op {
                        Op::Eq => FloatCC::Equal,
                        Op::Ne => FloatCC::NotEqual,
                        Op::Lt => FloatCC::LessThan,
                        Op::Le => FloatCC::LessThanOrEqual,
                        Op::Gt => FloatCC::GreaterThan,
                        _ => FloatCC::GreaterThanOrEqual,
                    };
                    let c = b.ins().fcmp(cc, a, x);
                    return b.ins().uextend(I64, c);
                }
            };
            return r;
        }
        let bits = i64::from(m.bits().max(1));
        let signed = m.signed();
        let r = match op {
            Op::Add => b.ins().iadd(a, x),
            Op::Sub => b.ins().isub(a, x),
            Op::Mul => b.ins().imul(a, x),
            Op::Div | Op::Rem => return self.u_div(op, m, a, x, true),
            Op::Shl => {
                let s = b.ins().band_imm(x, bits - 1);
                b.ins().ishl(a, s)
            }
            Op::Shr => {
                let s = b.ins().band_imm(x, bits - 1);
                if signed { b.ins().sshr(a, s) } else { b.ins().ushr(a, s) }
            }
            Op::And => b.ins().band(a, x),
            Op::Or => b.ins().bor(a, x),
            Op::Xor => b.ins().bxor(a, x),
            _ => {
                let cc = match (op, signed) {
                    (Op::Eq, _) => IntCC::Equal,
                    (Op::Ne, _) => IntCC::NotEqual,
                    (Op::Lt, true) => IntCC::SignedLessThan,
                    (Op::Lt, false) => IntCC::UnsignedLessThan,
                    (Op::Le, true) => IntCC::SignedLessThanOrEqual,
                    (Op::Le, false) => IntCC::UnsignedLessThanOrEqual,
                    (Op::Gt, true) => IntCC::SignedGreaterThan,
                    (Op::Gt, false) => IntCC::UnsignedGreaterThan,
                    (Op::Ge, true) => IntCC::SignedGreaterThanOrEqual,
                    _ => IntCC::UnsignedGreaterThanOrEqual,
                };
                let c = b.ins().icmp(cc, a, x);
                return b.ins().uextend(I64, c);
            }
        };
        norm_u(&mut self.b, m, r)
    }

    fn u_un(&mut self, op: Op, m: M, a: Value) -> Value {
        match op {
            Op::Neg if m.float() => self.b.ins().fneg(a),
            Op::Neg => {
                let r = self.b.ins().ineg(a);
                norm_u(&mut self.b, m, r)
            }
            Op::Not => {
                let r = self.b.ins().bnot(a);
                norm_u(&mut self.b, m, r)
            }
            Op::LNot => {
                let t = self.truth_u(m, a);
                self.b.ins().bxor_imm(t, 1)
            }
            _ => a,
        }
    }

    fn u_cast(&mut self, from: M, to: M, a: Value) -> Value {
        if from == to {
            return a;
        }
        if to == M::B {
            return self.truth_u(from, a);
        }
        let b = &mut self.b;
        match (from.float(), to.float()) {
            (false, false) => norm_u(b, to, a),
            (true, true) => {
                if to == M::F32 {
                    b.ins().fdemote(F32, a)
                } else {
                    b.ins().fpromote(F64, a)
                }
            }
            (false, true) => {
                let f = if from.signed() { b.ins().fcvt_from_sint(F64, a) } else { b.ins().fcvt_from_uint(F64, a) };
                if to == M::F32 { b.ins().fdemote(F32, f) } else { f }
            }
            (true, false) => {
                let f = if from == M::F32 { b.ins().fpromote(F64, a) } else { a };
                let i = if to.signed() { b.ins().fcvt_to_sint_sat(I64, f) } else { b.ins().fcvt_to_uint_sat(I64, f) };
                norm_u(b, to, i)
            }
        }
    }

    /// A value's bits as a uniform integer (a float's), of `m`'s width.
    fn bits_u(&mut self, m: M, a: Value) -> Value {
        match m {
            M::F32 => {
                let i = self.b.ins().bitcast(I32, MemFlags::new(), a);
                self.b.ins().uextend(I64, i)
            }
            M::F64 => self.b.ins().bitcast(I64, MemFlags::new(), a),
            _ => a,
        }
    }

    fn u_laneop(&mut self, bi: Builtin, m: M, from: M, a: Value, x: Value) -> Value {
        match bi {
            Builtin::Popcount => {
                let v = self.b.ins().band_imm(a, width_mask(m));
                self.b.ins().popcnt(v)
            }
            Builtin::Clz => {
                let v = self.b.ins().band_imm(a, width_mask(m));
                let c = self.b.ins().clz(v);
                self.b.ins().iadd_imm(c, -(64 - i64::from(m.bits())))
            }
            Builtin::MulHi => {
                if m.bits() == 64 {
                    if m.signed() { self.b.ins().smulhi(a, x) } else { self.b.ins().umulhi(a, x) }
                } else {
                    let p = self.b.ins().imul(a, x);
                    let s = i64::from(m.bits());
                    let r = if m.signed() { self.b.ins().sshr_imm(p, s) } else { self.b.ins().ushr_imm(p, s) };
                    norm_u(&mut self.b, m, r)
                }
            }
            Builtin::As => {
                let bits = self.bits_u(from, a);
                let v = self.b.ins().band_imm(bits, width_mask(from));
                match m {
                    M::F32 => {
                        let i = self.b.ins().ireduce(I32, v);
                        self.b.ins().bitcast(F32, MemFlags::new(), i)
                    }
                    M::F64 => self.b.ins().bitcast(F64, MemFlags::new(), v),
                    _ => norm_u(&mut self.b, m, v),
                }
            }
            Builtin::Min | Builtin::Max => {
                let less = self.u_bin(Op::Lt, m, a, x);
                let (p, q) = if bi == Builtin::Min { (a, x) } else { (x, a) };
                self.b.ins().select(less, p, q)
            }
            _ => a,
        }
    }

    /// Each lane's `f` of its operands' lanes (uniform values, as the uniform code takes them): the result's lanes of
    /// `rm`.
    fn per_lane(&mut self, ops: &[(Vec<Value>, M)], rm: M, f: &mut dyn FnMut(&mut Self, &[Value]) -> Value) -> Vec<Value> {
        let mut out = Vec::new();
        for j in 0..self.n_lanes() {
            let mut xs = Vec::with_capacity(ops.len());
            for (p, m) in ops {
                let s = self.extract(p, *m, j);
                xs.push(self.uniform_of(s, *m));
            }
            let r = f(self, &xs);
            out.push(self.lane_scalar(r, rm));
        }
        self.assemble(rm, &out)
    }

    // Lanes' operations.

    /// Lanes of an integer type of 32 bits or fewer made as `m` holds them.
    fn norm32(&mut self, m: M, v: Value) -> Value {
        let b = &mut self.b;
        let t = b.func.dfg.value_type(v);
        let k = |b: &mut FunctionBuilder, x: i64| if t.is_vector() { let c = b.ins().iconst(I32, x); b.ins().splat(t, c) } else { b.ins().iconst(t, x) };
        match m {
            M::I8 => {
                let s = b.ins().ishl_imm(v, 24);
                b.ins().sshr_imm(s, 24)
            }
            M::U8 => {
                let c = k(b, 0xff);
                b.ins().band(v, c)
            }
            M::I16 => {
                let s = b.ins().ishl_imm(v, 16);
                b.ins().sshr_imm(s, 16)
            }
            M::U16 => {
                let c = k(b, 0xffff);
                b.ins().band(v, c)
            }
            M::B => {
                let z = k(b, 0);
                let c = b.ins().icmp(IntCC::NotEqual, v, z);
                let one = k(b, 1);
                if t.is_vector() { b.ins().band(c, one) } else { b.ins().uextend(t, c) }
            }
            _ => v,
        }
    }

    /// A comparison's 32-bit lanes' mask as `bool` lanes (0 or 1).
    fn bools_of(&mut self, k: &[Value]) -> Vec<Value> {
        if self.w == 1 {
            return vec![self.b.ins().band_imm(k[0], 1)];
        }
        k.iter().map(|&x| self.b.ins().ushr_imm(x, 31)).collect()
    }

    /// 64-bit lanes' masks (`I64X2`s, or an `I64` at width 1) as 32-bit lanes' (`mask_parts`): each lane's low half.
    fn narrow_mask(&mut self, p: &[Value]) -> Vec<Value> {
        self.low_halves(p)
    }

    /// 64-bit lanes (`I64X2`s, or an `I64` at width 1) as their low 32 bits' (`I32X4`s, or an `I32`), in order.
    fn low_halves(&mut self, p: &[Value]) -> Vec<Value> {
        if self.w == 1 {
            return vec![self.b.ins().ireduce(I32, p[0])];
        }
        let imm = self.b.func.dfg.immediates.push(ConstantData::from(vec![0u8, 1, 2, 3, 8, 9, 10, 11, 16, 17, 18, 19, 24, 25, 26, 27]));
        (0..mask_parts(self.w))
            .map(|k| {
                let lo = p[(2 * k).min(p.len() - 1)];
                let hi = p[(2 * k + 1).min(p.len() - 1)];
                let lo = self.b.ins().bitcast(I8X16, le(), lo);
                let hi = self.b.ins().bitcast(I8X16, le(), hi);
                let s = self.b.ins().shuffle(lo, hi, imm);
                self.b.ins().bitcast(I32X4, le(), s)
            })
            .collect()
    }

    /// A comparison of lanes: its 32-bit lanes' mask.
    fn cmp_lanes(&mut self, op: Op, m: M, a: &[Value], x: &[Value]) -> Vec<Value> {
        let w1 = self.w == 1;
        let mut out = Vec::new();
        for (&p, &q) in a.iter().zip(x) {
            let r = if m.float() {
                let cc = match op {
                    Op::Eq => FloatCC::Equal,
                    Op::Ne => FloatCC::NotEqual,
                    Op::Lt => FloatCC::LessThan,
                    Op::Le => FloatCC::LessThanOrEqual,
                    Op::Gt => FloatCC::GreaterThan,
                    _ => FloatCC::GreaterThanOrEqual,
                };
                self.b.ins().fcmp(cc, p, q)
            } else {
                let signed = m.signed();
                let cc = match (op, signed) {
                    (Op::Eq, _) => IntCC::Equal,
                    (Op::Ne, _) => IntCC::NotEqual,
                    (Op::Lt, true) => IntCC::SignedLessThan,
                    (Op::Lt, false) => IntCC::UnsignedLessThan,
                    (Op::Le, true) => IntCC::SignedLessThanOrEqual,
                    (Op::Le, false) => IntCC::UnsignedLessThanOrEqual,
                    (Op::Gt, true) => IntCC::SignedGreaterThan,
                    (Op::Gt, false) => IntCC::UnsignedGreaterThan,
                    (Op::Ge, true) => IntCC::SignedGreaterThanOrEqual,
                    _ => IntCC::UnsignedGreaterThanOrEqual,
                };
                self.b.ins().icmp(cc, p, q)
            };
            let r = if w1 {
                let t = if wide(m) { I64 } else { I32 };
                self.b.ins().bmask(t, r)
            } else if m.float() {
                // A float comparison's lanes are its type's integers.
                r
            } else {
                r
            };
            out.push(r);
        }
        if wide(m) { self.narrow_mask(&out) } else { out }
    }

    /// The bounds each branch on an unsigned comparison with a constant gives the value compared, in the blocks its
    /// sides dominate (`below`).
    fn bounds_of_branches(&mut self) {
        let f = self.f;
        for blk in &f.blocks {
            let t = *f.inst(*blk.insts.last().expect("a terminator"));
            if t.op != O::Br {
                continue;
            }
            let mut c = t.a;
            while f.inst(c).op == O::Cast && f.inst(c).shape == Shape::Uniform {
                c = f.inst(c).a;
            }
            let i = *f.inst(c);
            if i.op != O::Bin || i.shape != Shape::Uniform {
                continue;
            }
            let om = M(i.d);
            if om.float() || om.signed() || om.is_agg() {
                continue;
            }
            let (a, b) = (self.konst(i.a), self.konst(i.b));
            // `x < k` (`k > x`) where true, `x >= k` (`k <= x`) where false: `x` at most `k - 1`; with `<=`, `k`.
            let fact = match (Op::of(u32::from(i.sub)), a, b) {
                (Op::Lt, None, Some(k)) => k.checked_sub(1).map(|k| (i.a, k, t.b)),
                (Op::Le, None, Some(k)) => Some((i.a, k, t.b)),
                (Op::Gt, Some(k), None) => k.checked_sub(1).map(|k| (i.b, k, t.b)),
                (Op::Ge, Some(k), None) => Some((i.b, k, t.b)),
                (Op::Ge, None, Some(k)) => k.checked_sub(1).map(|k| (i.a, k, t.c)),
                (Op::Gt, None, Some(k)) => Some((i.a, k, t.c)),
                (Op::Le, Some(k), None) => k.checked_sub(1).map(|k| (i.b, k, t.c)),
                (Op::Lt, Some(k), None) => Some((i.b, k, t.c)),
                _ => None,
            };
            if let Some((x, k, side)) = fact {
                self.below.entry(x).or_default().push((k, side as usize));
            }
        }
    }

    /// `counted` and `count_tests`: each loop header's parameter `k` that its loop's one entry passes a constant
    /// `k0`, its one latch `k + 1`, where the latch is the side of a branch on `k + 1 < N` that runs again, `N` a
    /// constant over `k0`: in the loop `k < N`, so `k + 1 < N` is `k + 1 != N`, `(k - N) + 1 != 0`.
    fn counted_loops(&mut self) {
        let f = self.f;
        let int = |m: M| !m.float() && !m.is_agg() && !m.signed() && m.bytes() >= 4;
        let through = |mut c: V| {
            while f.inst(c).op == O::Cast && f.inst(c).shape == Shape::Uniform && int(f.inst(c).m) && int(M(f.inst(c).b)) {
                c = f.inst(c).a;
            }
            c
        };
        for h in 0..f.blocks.len() {
            let ps = &self.preds[h];
            // A loop whose body ends a chunked kernel's region starts one at its header: its counter is read back.
            if ps.len() != 2 || self.resume[h] {
                continue;
            }
            let (pre, latch) = if self.dominates(h, ps[1]) && !self.dominates(h, ps[0]) { (ps[0], ps[1]) } else if self.dominates(h, ps[0]) && !self.dominates(h, ps[1]) { (ps[1], ps[0]) } else { continue };
            let (tp, tl) = (*f.inst(*f.blocks[pre].insts.last().expect("a terminator")), *f.inst(*f.blocks[latch].insts.last().expect("a terminator")));
            if tp.op != O::Jump || tl.op != O::Jump || self.preds[latch].len() != 1 {
                continue;
            }
            let tb = *f.inst(*f.blocks[self.preds[latch][0]].insts.last().expect("a terminator"));
            if tb.op != O::Br || tb.b as usize != latch {
                continue;
            }
            // The test's 0 or 1, through casts (each keeps 0 and 1).
            let mut test = tb.a;
            while f.inst(test).op == O::Cast && f.inst(test).shape == Shape::Uniform {
                test = f.inst(test).a;
            }
            let ti = *f.inst(test);
            if ti.op != O::Bin || ti.shape != Shape::Uniform || !int(M(ti.d)) || Op::of(u32::from(ti.sub)) != Op::Lt {
                continue;
            }
            let Some(n) = self.konst(ti.b).and_then(|n| i64::try_from(n).ok()) else { continue };
            for (j, &p) in f.blocks[h].params.iter().enumerate() {
                let pi = f.inst(p);
                if pi.shape != Shape::Uniform || !int(pi.m) {
                    continue;
                }
                let (a0, a1) = (f.args_of(&tp)[j], f.args_of(&tl)[j]);
                let Some(k0) = self.konst(a0).and_then(|k| i64::try_from(k).ok()) else { continue };
                let xi = *f.inst(a1);
                let one = |v: V| self.konst(v) == Some(1);
                let next = xi.op == O::Bin && xi.shape == Shape::Uniform && Op::of(u32::from(xi.sub)) == Op::Add && M(xi.d) == pi.m
                    && ((xi.a == p && one(xi.b)) || (xi.b == p && one(xi.a)));
                if next && through(ti.a) == a1 && k0 < n {
                    let Rp::U(v) = self.vals[p as usize] else { continue };
                    self.counted.insert(p, (v, n));
                    self.count_tests.insert(test, p);
                }
            }
        }
    }

    /// `block_of` and `depth`: for each back edge (a jump to a block that dominates it), its natural loop's blocks
    /// one deeper.
    fn loops(&mut self) {
        let f = self.f;
        let nb = f.blocks.len();
        self.block_of = vec![0; f.insts.len()];
        let mut preds = vec![Vec::new(); nb];
        for (b, blk) in f.blocks.iter().enumerate() {
            for &x in blk.params.iter().chain(&blk.insts) {
                self.block_of[x as usize] = b as u32;
            }
            for s in f.succs(b) {
                preds[s as usize].push(b);
            }
        }
        self.depth = vec![0; nb];
        for b in 0..nb {
            for h in f.succs(b) {
                let h = h as usize;
                if !self.dominates(h, b) {
                    continue;
                }
                let mut body = vec![false; nb];
                body[h] = true;
                let mut work = vec![b];
                while let Some(x) = work.pop() {
                    if !body[x] {
                        body[x] = true;
                        work.extend(&preds[x]);
                    }
                }
                for (x, &inside) in body.iter().enumerate() {
                    self.depth[x] += u32::from(inside);
                }
            }
        }
        self.preds = preds;
    }

    /// Uniform integer `x` as a sum (`Lin`): through sums, products and shifts by constants, and casts that keep its
    /// integer, of 64 bits (whose sums wrap as the addresses made of them) or of 32 that cannot wrap (`bound`).
    fn lin(&mut self, x: V) -> Lin {
        self.lin_at(x, 16)
    }

    fn lin_at(&mut self, x: V, depth: u32) -> Lin {
        let i = *self.f.inst(x);
        if let Some(&(n, k)) = self.counted.get(&x) {
            return Lin::leaf(n, self.depth[self.block_of[x as usize] as usize]).plus(Lin::konst(k));
        }
        let unsigned = |m: M| !m.float() && !m.is_agg() && !m.signed() && m.bytes() >= 4;
        let whole = |g: &Self, m: M| m.bytes() == 8 || g.bound(x).is_some();
        if depth > 0 && i.shape == Shape::Uniform {
            let d = depth - 1;
            match i.op {
                O::Const if unsigned(i.m) => return Lin::konst(self.konst(x).unwrap_or(0) as i64),
                O::Bin if unsigned(M(i.d)) && whole(self, M(i.d)) => {
                    let ka = self.konst(i.a).map(|k| k as i64);
                    let kb = self.konst(i.b).map(|k| k as i64);
                    match Op::of(u32::from(i.sub)) {
                        Op::Add => return self.lin_at(i.a, d).plus(self.lin_at(i.b, d)),
                        Op::Mul if kb.is_some() => return self.lin_at(i.a, d).times(kb.unwrap_or(0)),
                        Op::Mul if ka.is_some() => return self.lin_at(i.b, d).times(ka.unwrap_or(0)),
                        Op::Shl if kb.is_some_and(|k| (0..63).contains(&k)) => return self.lin_at(i.a, d).times(1i64 << kb.unwrap_or(0)),
                        _ => {}
                    }
                }
                O::Cast if unsigned(i.m) && unsigned(M(i.b)) && (i.m.bytes() >= M(i.b).bytes() || self.fits_in(i.a, i.m)) => {
                    return self.lin_at(i.a, d);
                }
                _ => {}
            }
        }
        let depth = self.depth[self.block_of[x as usize] as usize];
        Lin::leaf(self.u(x), depth)
    }

    /// The sum's value: its terms in order of depth, the constant after the outermost ones.
    fn sum(&mut self, l: &Lin) -> Value {
        let mut terms = l.terms.clone();
        terms.sort_by_key(|t| t.2);
        let mut acc: Option<Value> = None;
        let mut k_done = l.k == 0;
        let outer = terms.first().map_or(0, |t| t.2);
        for (v, c, d) in terms {
            if !k_done && d > outer {
                acc = Some(self.plus_imm(acc, l.k));
                k_done = true;
            }
            let t = if c == 1 { v } else if c > 0 && (c as u64).is_power_of_two() { self.b.ins().ishl_imm(v, i64::from(c.trailing_zeros())) } else { self.b.ins().imul_imm(v, c) };
            acc = Some(match acc {
                Some(a) => self.b.ins().iadd(a, t),
                None => t,
            });
        }
        if !k_done {
            acc = Some(self.plus_imm(acc, l.k));
        }
        acc.unwrap_or_else(|| self.b.ins().iconst(I64, 0))
    }

    fn plus_imm(&mut self, acc: Option<Value>, k: i64) -> Value {
        match acc {
            Some(a) => self.b.ins().iadd_imm(a, k),
            None => self.b.ins().iconst(I64, k),
        }
    }

    /// A uniform address: its sum (`lin`, `sum`).
    fn addr_u(&mut self, a: V) -> Value {
        let l = self.lin(a);
        self.sum(&l)
    }

    fn dominates(&self, a: usize, mut b: usize) -> bool {
        loop {
            if a == b {
                return true;
            }
            if b == 0 || self.idom[b] == usize::MAX {
                return false;
            }
            b = self.idom[b];
        }
    }

    /// The most an unsigned integer value `x` can be, in the block being made (every lane's, for lanes): `None` where
    /// its operations may wrap, or nothing bounds it.
    /// Whether unsigned `x`'s values all fit in `m`.
    fn fits_in(&self, x: V, m: M) -> bool {
        m.bytes() >= 8 || self.bound(x).is_some_and(|b| b >> (8 * m.bytes()) == 0)
    }

    fn bound(&self, x: V) -> Option<u64> {
        self.bound_at(x, 12, self.cur_block)
    }

    /// The same at the end of block `at`; a block's parameter's, the most its jumps pass it (a loop's counter: its
    /// start, and its next value where the loop's test of it branches back).
    fn bound_at(&self, x: V, depth: u32, at: usize) -> Option<u64> {
        let i = *self.f.inst(x);
        let max = |m: M| if m.bytes() >= 8 { u64::MAX } else { (1u64 << (8 * m.bytes())) - 1 };
        let int = |m: M| !m.float() && !m.is_agg() && !m.signed() && m.bytes() > 0;
        let fact = self.below.get(&x).and_then(|fs| fs.iter().filter(|&&(_, b)| self.dominates(b, at)).map(|&(k, _)| k).min());
        if depth == 0 || !int(i.m) || fact.is_some() {
            return fact;
        }
        if i.op == O::Param {
            let b = self.block_of[x as usize] as usize;
            let k = self.f.blocks[b].params.iter().position(|&p| p == x)?;
            let mut most = 0;
            for &p in &self.preds[b] {
                let t = self.f.inst(*self.f.blocks[p].insts.last()?);
                if t.op != O::Jump {
                    return None;
                }
                most = most.max(self.bound_at(*self.f.args_of(t).get(k)?, depth - 1, p)?);
            }
            return Some(most);
        }
        let d = depth - 1;
        let fits = |v: u128, m: M| (v <= u128::from(max(m))).then_some(v as u64);
        let of = match i.op {
            O::Const => self.konst(x),
            O::Cast => {
                let from = M(i.b);
                if int(from) {
                    self.bound_at(i.a, d, at).and_then(|a| fits(u128::from(a), i.m))
                } else {
                    // A signed constant: its value, where the target holds it.
                    self.konst(i.a).and_then(|k| fits(u128::from(k), i.m))
                }
            }
            O::Bin => {
                let om = M(i.d);
                let (a, b) = (self.bound_at(i.a, d, at), self.bound_at(i.b, d, at));
                match Op::of(u32::from(i.sub)) {
                    _ if !int(om) => None,
                    Op::Add => fits(u128::from(a?) + u128::from(b?), om),
                    Op::Mul => fits(u128::from(a?) * u128::from(b?), om),
                    Op::Shl => self.konst(i.b).filter(|&k| k < 64).and_then(|k| fits(u128::from(a?) << k, om)),
                    Op::Shr => self.konst(i.b).filter(|&k| k < 64).map(|k| a.unwrap_or(max(om)) >> k),
                    Op::Div => self.konst(i.b).filter(|&k| k > 0).map(|k| a.unwrap_or(max(om)) / k),
                    Op::Rem => self.konst(i.b).filter(|&k| k > 0).map(|k| k - 1),
                    Op::And => Some(a.unwrap_or(max(om)).min(b.unwrap_or(max(om)))),
                    Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => Some(1),
                    _ => None,
                }
            }
            // A thread's id in its group: under its dimension's threads (`exec::run`: at most `MAX_DIM`).
            O::Lid => Some(u64::from(crate::exec::MAX_DIM) - 1),
            _ => None,
        };
        match (of, fact) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// `a op x`'s steps, when `a` or `x` has them and the other is uniform (a sum) or a constant (a product, a shift).
    fn stepped(&mut self, op: Op, m: M, a: V, x: V, r: V) -> Option<Steps> {
        if m.float() || m.bytes() < 4 {
            return None;
        }
        let konst = |g: &Self, v: V| g.konst(v).map(|k| k as i64);
        // A 64-bit value's bits are its integer's, mod 2^64 as the addresses made of it; a 32-bit one's where no lane
        // can wrap (`bound`).
        let exact = m.bytes() == 8 || self.bound(r).is_some();
        match op {
            Op::Add => {
                for (s, u) in [(a, x), (x, a)] {
                    if let (Some(st), Rp::U(_)) = (self.steps.get(&s).cloned(), &self.vals[u as usize]) {
                        let lu = self.lin(u);
                        let first = st.first.plus(lu.clone());
                        let group = st.group.map(|g| g.plus(lu));
                        return Some(Steps { first, step: st.step, exact: exact && st.exact, group });
                    }
                }
                None
            }
            Op::Mul | Op::Shl => {
                for (s, c) in [(a, x), (x, a)] {
                    if op == Op::Shl && s != a {
                        break;
                    }
                    if let (Some(st), Some(k)) = (self.steps.get(&s).cloned(), konst(self, c)) {
                        let k = if op == Op::Shl { 1i64.checked_shl(k as u32)? } else { k };
                        let group = st.group.map(|g| g.times(k));
                        return Some(Steps { first: st.first.times(k), step: st.step.wrapping_mul(k), exact: exact && st.exact, group });
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// A lanes operation the SIMD group's lanes' steps answer, or one by such a value (`Steps`'s `group`):
    /// - `v >> k`, where `v`'s group lane 0 is a multiple of `2^k` and its lanes' steps add under `2^k`: the group's
    ///   one value, `group >> k` (a row's word of bits, at its lanes' index over 32);
    /// - `v & (2^k - 1)`, the same `v`: each lane's own steps, `l * step` (a lane's bit in that word);
    /// - of 32-bit lanes, a shift by such a stepped `s` (its first lane's `u`, its steps `t`), under the lanes' bits:
    ///   `x << s` as `(x << u)` times each lane's `2^(j t)`; `(x >> s) << K` (`s` at most `K`) as `(x >> u)` times
    ///   `2^(K - j t)`, its bits under `K` cleared; `(x >> s) & 1` as `(x >> u)` times `2^(31 - j t)`, over `2^31`.
    ///   (Cranelift's vectors shift by one count; a multiply is one instruction.)
    fn group_bin(&mut self, op: Op, m: M, a: V, b: V, x: V) -> Option<Vec<Value>> {
        if m.float() || m.signed() || m.is_agg() || m.bytes() < 4 || self.w == 1 {
            return None;
        }
        let kb = self.konst(b);
        let bits = m.bits();
        // `v`'s group, where its lanes' steps stay under `2^k` and its lane 0 is a multiple of it.
        let grouped = |g: &Self, v: V, k: u32| {
            let st = g.steps.get(&v)?;
            let gr = st.group.clone()?;
            (st.exact && st.step >= 0 && k < 63 && (st.step as i128) * 31 < (1i128 << k) && gr.align() >= k).then(|| (gr, st.step))
        };
        // A lanes shift count with steps, all under `limit`: its first lane's count (an `I64`) and its steps.
        let counts = |g: &mut Self, s: V, limit: u64| -> Option<(Value, i64)> {
            let st = g.steps.get(&s)?.clone();
            if !st.exact || st.step < 0 || g.bound(s)? >= limit {
                return None;
            }
            Some((g.sum(&st.first), st.step))
        };
        let (n, _) = parts(m, self.w);
        // Each register's lanes' `f(j)`, `j` the lane in the chunk.
        let lanes_of = |g: &mut Self, f: &dyn Fn(u32) -> u32| -> Vec<Value> {
            (0..n as u32).map(|p| vconst32(&mut g.b, [f(4 * p), f(4 * p + 1), f(4 * p + 2), f(4 * p + 3)])).collect()
        };
        match (op, kb) {
            (Op::Shr, Some(k)) if k > 0 && k < 32 && bits == 32 => {
                let (gr, _) = grouped(self, a, k as u32)?;
                let r = gr.over(k as u32);
                // The sum's terms over `2^k` agree with the value under `2^(64 - k)`: its 32 bits are the value.
                let u = self.sum(&r);
                let u = self.b.ins().band_imm(u, 0xffff_ffff);
                self.steps.insert(x, Steps { first: r.clone(), step: 0, exact: true, group: Some(r) });
                Some(self.splat(u, m))
            }
            (Op::And, Some(mask)) if mask > 0 && (mask + 1).is_power_of_two() && mask < u64::from(u32::MAX) => {
                let k = (mask + 1).trailing_zeros();
                let (_, step) = grouped(self, a, k)?;
                if bits != 32 {
                    return None;
                }
                let lane0 = self.lane0;
                let f = self.b.ins().imul_imm(lane0, step);
                let first = Lin::leaf(lane0, 0).times(step);
                self.steps.insert(x, Steps { first, step, exact: true, group: Some(Lin::konst(0)) });
                let s = self.splat(f, m);
                let c = lanes_of(self, &|j| (j as i64 * step) as u32);
                Some(s.iter().zip(&c).map(|(&s, &c)| self.b.ins().iadd(s, c)).collect())
            }
            (Op::Shl, None) if bits == 32 => {
                let (u, t) = counts(self, b, 32)?;
                if (self.w as i64 - 1) * t >= 32 {
                    return None;
                }
                let pa = self.lanes(a, m);
                let c = lanes_of(self, &|j| 1u32 << (j as i64 * t));
                let u = self.b.ins().ireduce(I32, u);
                Some(pa.iter().zip(&c).map(|(&p, &c)| { let y = self.b.ins().ishl(p, u); self.b.ins().imul(y, c) }).collect())
            }
            (Op::Shl | Op::And, Some(kk)) if bits == 32 && (op == Op::Shl && kk < 32 || op == Op::And && kk == 1) => {
                let ai = *self.f.inst(a);
                if ai.op != O::Bin || ai.shape != Shape::Lanes || Op::of(u32::from(ai.sub)) != Op::Shr || M(ai.d) != m {
                    return None;
                }
                let top = if op == Op::Shl { kk as i64 } else { 31 };
                let (u, t) = counts(self, ai.b, top as u64 + 1)?;
                if (self.w as i64 - 1) * t > top {
                    return None;
                }
                let px = self.lanes(ai.a, m);
                let c = lanes_of(self, &|j| 1u32 << (top - j as i64 * t));
                let u = self.b.ins().ireduce(I32, u);
                let ys: Vec<Value> = px.iter().zip(&c).map(|(&p, &c)| { let y = self.b.ins().ushr(p, u); self.b.ins().imul(y, c) }).collect();
                Some(if op == Op::Shl {
                    let hi = self.b.ins().iconst(I32, i64::from(u32::MAX << top));
                    let hv = self.b.ins().splat(I32X4, hi);
                    ys.into_iter().map(|y| self.b.ins().band(y, hv)).collect()
                } else {
                    ys.into_iter().map(|y| self.b.ins().ushr_imm(y, 31)).collect()
                })
            }
            _ => None,
        }
    }

    /// The constant Cranelift value `c` is (an `iconst`, through extensions), if it is one.
    fn known(&self, c: Value) -> Option<i64> {
        let dfg = &self.b.func.dfg;
        let ir::ValueDef::Result(inst, _) = dfg.value_def(c) else { return None };
        match dfg.insts[inst] {
            ir::InstructionData::UnaryImm { opcode: ir::Opcode::Iconst, imm } => Some(imm.bits()),
            ir::InstructionData::Unary { opcode: ir::Opcode::Uextend | ir::Opcode::Sextend, arg } => self.known(arg),
            // A narrowing that keeps the constant.
            ir::InstructionData::Unary { opcode: ir::Opcode::Ireduce, arg } => {
                let k = self.known(arg)?;
                (k >= 0 && (k as u64) >> (dfg.value_type(c).bits() - 1) == 0).then_some(k)
            }
            ir::InstructionData::IntCompareImm { opcode: ir::Opcode::IcmpImm, cond, arg, imm } => {
                let k = self.known(arg)?;
                use ir::condcodes::IntCC::*;
                match cond {
                    Equal => Some(i64::from(k == imm.bits())),
                    NotEqual => Some(i64::from(k != imm.bits())),
                    _ => None,
                }
            }
            // A test of two constants (both at least 0, where its signed and unsigned answers agree).
            ir::InstructionData::IntCompare { opcode: ir::Opcode::Icmp, cond, args } => {
                let (a, b) = (self.known(args[0])?, self.known(args[1])?);
                if a < 0 || b < 0 {
                    return None;
                }
                use ir::condcodes::IntCC::*;
                Some(i64::from(match cond {
                    Equal => a == b,
                    NotEqual => a != b,
                    SignedLessThan | UnsignedLessThan => a < b,
                    SignedLessThanOrEqual | UnsignedLessThanOrEqual => a <= b,
                    SignedGreaterThan | UnsignedGreaterThan => a > b,
                    SignedGreaterThanOrEqual | UnsignedGreaterThanOrEqual => a >= b,
                }))
            }
            _ => None,
        }
    }

    /// The integer constant `x` is, through uniform integer casts that keep it.
    fn konst(&self, x: V) -> Option<u64> {
        let i = self.f.inst(x);
        match i.op {
            O::Const => Some(crate::ops::norm(i.m, u64::from(i.a) | u64::from(i.b) << 32)),
            O::Cast if i.shape == Shape::Uniform && !i.m.float() && !M(i.b).float() => {
                let k = self.konst(i.a)?;
                (crate::ops::norm(i.m, k) == k).then_some(k)
            }
            _ => None,
        }
    }

    /// Lanes of `m` (an integer of 4 or 8 bytes) whose lane `j` is `first + j` (`first` an `I64`).
    fn iota(&mut self, first: Value, m: M) -> Vec<Value> {
        let w = self.w;
        if w == 1 {
            return vec![if wide(m) { first } else { self.b.ins().ireduce(I32, first) }];
        }
        let (n, t) = parts(m, w);
        let f = if wide(m) { first } else { self.b.ins().ireduce(I32, first) };
        let s = self.b.ins().splat(t, f);
        (0..n as u32)
            .map(|p| {
                let o = if wide(m) { vconst64(&mut self.b, [2 * p as u64, 2 * p as u64 + 1]) } else { vconst32(&mut self.b, [4 * p, 4 * p + 1, 4 * p + 2, 4 * p + 3]) };
                self.b.ins().iadd(s, o)
            })
            .collect()
    }

    /// Lanes of 4 or 8 bytes in memory from `at` on, lane `j` at `at + j * bytes`, read as whole registers (a width
    /// under a register's lanes: lane by lane, so nothing past the chunk's is read).
    fn load_contiguous(&mut self, m: M, at: Value) -> Vec<Value> {
        let w = self.w;
        let (n, t) = parts(m, w);
        let per = if wide(m) { 2 } else { 4 };
        if w < per {
            let xs: Vec<Value> = (0..w)
                .map(|j| {
                    let u = self.load_u(m, at, (j * m.bytes()) as i32);
                    self.lane_scalar(u, m)
                })
                .collect();
            return if w == 1 { xs } else { self.assemble(m, &xs) };
        }
        (0..n).map(|p| self.b.ins().load(t, mf(), at, 16 * p as i32)).collect()
    }

    /// Lanes of 1 or 2 bytes (`m`) at `addr`'s steps, `st` bytes apart (`st` 1 to 16), every lane's read, at 4 lanes
    /// or more: the bytes the lanes read, `span` of them from lane 0's, in windows of 16 (or under 16 bytes, two
    /// loads of 8 or 4 at its ends), each window only bytes of the span, so nothing past the lanes' is read; each 4
    /// lanes' one `swizzle` (AArch64's `tbl`) of their bytes from a window into their 32-bit lanes, the others 0.
    fn load_bytes(&mut self, m: M, addr: V) -> Option<Vec<Value>> {
        let w = self.w;
        let eb = i64::from(m.bytes());
        if !matches!(eb, 1 | 2) || m.float() || m.is_agg() || m == M::B || w < 4 {
            return None;
        }
        let st = self.steps.get(&addr)?.clone();
        let s = st.step;
        if s < eb || s > 16 {
            return None;
        }
        let at = self.sum(&st.first);
        let span = (i64::from(w) - 1) * s + eb;
        let little = MemFlags::new().with_endianness(ir::Endianness::Little);
        // A window holding bytes `lo..hi` of the span: its value and each byte's place in it.
        let mut windows: Vec<(i64, Value)> = Vec::new();
        let (n, _) = parts(m, w);
        let mut out = Vec::with_capacity(n);
        for p in 0..n as i64 {
            let (lo, hi) = (4 * p * s, (4 * p + 3) * s + eb);
            let (v, place): (Value, Box<dyn Fn(i64) -> i64>) = if span >= 16 {
                let mut o = lo / 16 * 16;
                if hi > o + 16 {
                    o = lo;
                }
                let o = o.min(span - 16);
                let v = match windows.iter().find(|x| x.0 == o) {
                    Some(&(_, v)) => v,
                    None => {
                        let v = self.b.ins().load(I8X16, mf(), at, o as i32);
                        windows.push((o, v));
                        v
                    }
                };
                (v, Box::new(move |b| b - o))
            } else {
                let (h, ty, vt) = if span >= 8 { (8, I64, I64X2) } else { (4, I32, I32X4) };
                let a1 = span - h;
                let v = match windows.first() {
                    Some(&(_, v)) => v,
                    None => {
                        let x0 = self.b.ins().load(ty, mf(), at, 0);
                        let mut v = self.b.ins().scalar_to_vector(vt, x0);
                        if a1 > 0 {
                            let x1 = self.b.ins().load(ty, mf(), at, a1 as i32);
                            v = self.b.ins().insertlane(v, x1, 1);
                        }
                        let v = self.b.ins().bitcast(I8X16, little, v);
                        windows.push((0, v));
                        v
                    }
                };
                (v, Box::new(move |b| if b < h { b } else { b - a1 + h }))
            };
            let mut idx = [0xffu8; 16];
            for i in 0..4i64 {
                for c in 0..eb {
                    idx[(4 * i + c) as usize] = place(lo + i * s + c) as u8;
                }
            }
            let ic = self.b.func.dfg.constants.insert(ConstantData::from(idx.to_vec()));
            let iv = self.b.ins().vconst(I8X16, ic);
            let r = self.b.ins().swizzle(v, iv);
            let r = self.b.ins().bitcast(I32X4, little, r);
            out.push(if m.signed() {
                let k = 32 - 8 * eb;
                let r = self.b.ins().ishl_imm(r, k);
                self.b.ins().sshr_imm(r, k)
            } else {
                r
            });
        }
        Some(out)
    }

    /// The address of lane 0 when `addr`'s lanes are `bytes` apart: its steps'.
    fn consecutive(&mut self, addr: V, bytes: u32) -> Option<Value> {
        let st = self.steps.get(&addr)?.clone();
        (st.step == i64::from(bytes) && (bytes == 4 || bytes == 8)).then(|| self.sum(&st.first))
    }

    fn bin_lanes(&mut self, op: Op, m: M, a: V, x: V, mask: V) -> (Vec<Value>, Option<Vec<Value>>) {
        let pa = self.lanes(a, m);
        if matches!(op, Op::Div | Op::Rem) && !m.float() {
            return (self.div_lanes(op, m, pa, x, mask), None);
        }
        if op.compares() {
            let px = self.lanes(x, m);
            let k = self.cmp_lanes(op, m, &pa, &px);
            return (self.bools_of(&k), Some(k));
        }
        // A shift by a uniform count: the vector's.
        if matches!(op, Op::Shl | Op::Shr) && !m.float() {
            if let Rp::U(c) = self.vals[x as usize] {
                let bits = i64::from(m.bits().max(1));
                let s = self.b.ins().band_imm(c, bits - 1);
                let s = if wide(m) { s } else { self.b.ins().ireduce(I32, s) };
                let r: Vec<Value> = pa
                    .iter()
                    .map(|&p| match (op, m.signed()) {
                        (Op::Shl, _) => self.b.ins().ishl(p, s),
                        (_, true) => self.b.ins().sshr(p, s),
                        _ => self.b.ins().ushr(p, s),
                    })
                    .collect();
                let r = if wide(m) { r } else { r.into_iter().map(|v| self.norm32(m, v)).collect() };
                return (r, None);
            }
        }
        // A product by a constant power of two (an index scaled to its element's bytes): a shift. AArch64 has no
        // 64-bit lanes' multiply, so Cranelift's is nine instructions.
        if op == Op::Mul && !m.float() {
            let ci = *self.f.inst(x);
            let k = (ci.op == O::Const).then(|| u64::from(ci.a) | u64::from(ci.b) << 32);
            if let Some(k) = k.filter(|k| k.is_power_of_two()) {
                let sh = k.trailing_zeros();
                let r: Vec<Value> = pa.iter().map(|&p| self.b.ins().ishl_imm(p, i64::from(sh))).collect();
                let r = if wide(m) { r } else { r.into_iter().map(|v| self.norm32(m, v)).collect() };
                return (r, None);
            }
        }
        let native = match op {
            Op::Add | Op::Sub | Op::Mul | Op::And | Op::Or | Op::Xor => true,
            Op::Div => m.float(),
            _ => false,
        };
        if native {
            let px = self.lanes(x, m);
            let r: Vec<Value> = pa
                .iter()
                .zip(&px)
                .map(|(&p, &q)| match (op, m.float()) {
                    (Op::Add, true) => self.b.ins().fadd(p, q),
                    (Op::Sub, true) => self.b.ins().fsub(p, q),
                    (Op::Mul, true) => self.b.ins().fmul(p, q),
                    (Op::Div, true) => self.b.ins().fdiv(p, q),
                    (Op::Add, _) => self.b.ins().iadd(p, q),
                    (Op::Sub, _) => self.b.ins().isub(p, q),
                    (Op::Mul, _) => self.b.ins().imul(p, q),
                    (Op::And, _) => self.b.ins().band(p, q),
                    (Op::Or, _) => self.b.ins().bor(p, q),
                    _ => self.b.ins().bxor(p, q),
                })
                .collect();
            let r = if wide(m) || m.float() { r } else { r.into_iter().map(|v| self.norm32(m, v)).collect() };
            return (r, None);
        }
        let px = self.lanes(x, m);
        let r = self.per_lane(&[(pa, m), (px, m)], m, &mut |g, xs| g.u_bin(op, m, xs[0], xs[1]));
        (r, None)
    }

    /// An integer division's lanes (`pa` by `x`): the kernel failed if a lane of `mask` divides by 0, each lane's
    /// quotient or remainder the uniform code's, a lane that does not run dividing by 1.
    fn div_lanes(&mut self, op: Op, m: M, pa: Vec<Value>, x: V, mask: V) -> Vec<Value> {
        let what = if op == Op::Div { FAIL_DIV } else { FAIL_REM };
        let km = self.mask(mask);
        let any = self.any(&km);
        if let Rp::U(d) = self.vals[x as usize] {
            // One divisor: one check.
            let z = self.b.ins().icmp_imm(IntCC::Equal, d, 0);
            let z = self.b.ins().uextend(I64, z);
            let bad = self.b.ins().band(z, any);
            self.fail_if(bad, what);
            // A constant power of two: a mask or a shift of the lanes, unsigned.
            let i = *self.f.inst(x);
            let k = (i.op == O::Const).then(|| u64::from(i.a) | u64::from(i.b) << 32);
            if let Some(k) = k.filter(|&k| k.is_power_of_two() && !m.signed() && !wide(m) && !m.float() && self.w > 1) {
                let r: Vec<Value> = pa
                    .iter()
                    .map(|&p| {
                        if op == Op::Rem {
                            let c = self.b.ins().iconst(I32, (k - 1) as i64);
                            let c = self.b.ins().splat(I32X4, c);
                            self.b.ins().band(p, c)
                        } else {
                            let c = self.b.ins().iconst(I32, i64::from(k.trailing_zeros()));
                            self.b.ins().ushr(p, c)
                        }
                    })
                    .collect();
                return r;
            }
            let one = self.b.ins().iconst(I64, 1);
            let zc = self.b.ins().icmp_imm(IntCC::Equal, d, 0);
            let safe = self.b.ins().select(zc, one, d);
            if self.vec_div(m) {
                let q = self.splat(safe, m);
                return self.div_f64(op, m, &pa, &q);
            }
            let r = self.per_lane(&[(pa, m)], m, &mut |g, xs| g.u_div(op, m, xs[0], safe, false));
            return r;
        }
        let px = self.lanes(x, m);
        let z = self.zero_lanes(m);
        let eq = self.cmp_lanes(Op::Eq, m, &px, &z);
        let bad: Vec<Value> = eq.iter().zip(&km).map(|(&e, &k)| self.b.ins().band(e, k)).collect();
        let anybad = self.any(&bad);
        self.fail_if(anybad, what);
        let one = self.b.ins().iconst(I64, 1);
        let ones = self.splat(one, m);
        let px = self.select_lanes(mask, m, px, ones);
        if self.vec_div(m) {
            return self.div_f64(op, m, &pa, &px);
        }
        self.per_lane(&[(pa, m), (px, m)], m, &mut |g, xs| g.u_div(op, m, xs[0], xs[1], false))
    }

    /// Whether lanes of `m` divide as vectors (`div_f64`): integers of 32 bits or fewer, on AArch64 (whose vector
    /// conversions of 64-bit lanes Cranelift has), at a width of two lanes or more.
    fn vec_div(&self, m: M) -> bool {
        cfg!(target_arch = "aarch64") && self.w > 1 && !wide(m) && !m.float() && !m.is_agg()
    }

    /// Lanes `pa` divided by `px` (no lane by 0), integers of 32 bits or fewer: each pair of lanes as 64-bit floats,
    /// whose quotient, truncated, is the integer's (an operand below 2^32 is exact, and the quotient's rounding error
    /// below 2^-21 / divisor, less than its distance to the next integer); the remainder `a - q * x`. The quotient's
    /// low 32 bits wrap as the integer's (`i32::MIN / -1`).
    fn div_f64(&mut self, op: Op, m: M, pa: &[Value], px: &[Value]) -> Vec<Value> {
        let signed = m.signed();
        let mut out = Vec::new();
        for (&a, &x) in pa.iter().zip(px) {
            let mut q64 = Vec::new();
            for high in [false, true] {
                let widen = |b: &mut FunctionBuilder, v: Value| match (signed, high) {
                    (true, false) => b.ins().swiden_low(v),
                    (true, true) => b.ins().swiden_high(v),
                    (false, false) => b.ins().uwiden_low(v),
                    (false, true) => b.ins().uwiden_high(v),
                };
                let fa = widen(&mut self.b, a);
                let fa = self.b.ins().fcvt_from_sint(F64X2, fa);
                let fx = widen(&mut self.b, x);
                let fx = self.b.ins().fcvt_from_sint(F64X2, fx);
                let fq = self.b.ins().fdiv(fa, fx);
                q64.push(self.b.ins().fcvt_to_sint_sat(I64X2, fq));
            }
            let imm = self.b.func.dfg.immediates.push(ConstantData::from(vec![0u8, 1, 2, 3, 8, 9, 10, 11, 16, 17, 18, 19, 24, 25, 26, 27]));
            let lo = self.b.ins().bitcast(I8X16, le(), q64[0]);
            let hi = self.b.ins().bitcast(I8X16, le(), q64[1]);
            let q = self.b.ins().shuffle(lo, hi, imm);
            let q = self.b.ins().bitcast(I32X4, le(), q);
            let r = if op == Op::Div {
                q
            } else {
                let p = self.b.ins().imul(q, x);
                self.b.ins().isub(a, p)
            };
            out.push(self.norm32(m, r));
        }
        out
    }

    fn cast_lanes(&mut self, from: M, to: M, a: V) -> Vec<Value> {
        let p = self.lanes(a, from);
        if from == to {
            return p;
        }
        let int = !from.float() && !to.float();
        let w = self.w;
        if int && to != M::B {
            match (wide(from), wide(to)) {
                (false, false) => return p.into_iter().map(|v| self.norm32(to, v)).collect(),
                (true, true) => return p,
                (false, true) => {
                    if w == 1 {
                        return vec![if from.signed() { self.b.ins().sextend(I64, p[0]) } else { self.b.ins().uextend(I64, p[0]) }];
                    }
                    let (n, _) = parts(to, w);
                    return (0..n)
                        .map(|j| {
                            let s = p[j / 2];
                            match (j % 2 == 0, from.signed()) {
                                (true, true) => self.b.ins().swiden_low(s),
                                (true, false) => self.b.ins().uwiden_low(s),
                                (false, true) => self.b.ins().swiden_high(s),
                                (false, false) => self.b.ins().uwiden_high(s),
                            }
                        })
                        .collect();
                }
                (true, false) => {
                    let t = self.low_halves(&p);
                    return t.into_iter().map(|v| self.norm32(to, v)).collect();
                }
            }
        }
        if to == M::B && !from.float() {
            let z = self.zero_lanes(from);
            let k = self.cmp_lanes(Op::Ne, from, &p, &z);
            self.raw.insert(a, k.clone());
            return self.bools_of(&k);
        }
        self.per_lane(&[(p, from)], to, &mut |g, xs| g.u_cast(from, to, xs[0]))
    }

    /// Lanes `a` in mask `k`'s lanes, `x` elsewhere.
    fn select_lanes(&mut self, k: V, m: M, a: Vec<Value>, x: Vec<Value>) -> Vec<Value> {
        if self.all(k) {
            return a;
        }
        let mk = self.mask_for(k, m);
        let w1 = self.w == 1;
        a.iter()
            .zip(&x)
            .zip(&mk)
            .map(|((&p, &q), &c)| {
                if w1 {
                    let t = self.b.ins().icmp_imm(IntCC::NotEqual, c, 0);
                    self.b.ins().select(t, p, q)
                } else if m.float() {
                    let t = self.b.func.dfg.value_type(p);
                    let c = self.b.ins().bitcast(t, le(), c);
                    self.b.ins().bitselect(c, p, q)
                } else {
                    self.b.ins().bitselect(c, p, q)
                }
            })
            .collect()
    }

    /// The lanes value `x`'s lane at dynamic index `idx` (an `I32`), as a uniform value.
    fn lane_at(&mut self, x: &[Value], m: M, idx: Value) -> Value {
        if self.w == 1 {
            return self.uniform_of(x[0], m);
        }
        for (k, &p) in x.iter().enumerate() {
            let sl = self.slot();
            self.b.ins().stack_store(p, sl, 16 * k as i32);
        }
        let sl = self.slot();
        let base = self.b.ins().stack_addr(I64, sl, 0);
        let size = if wide(m) { 8 } else { 4 };
        let i = self.b.ins().uextend(I64, idx);
        let off = self.b.ins().imul_imm(i, size);
        let at = self.b.ins().iadd(base, off);
        let (_, t) = parts(m, 1);
        let s = self.b.ins().load(t, mf(), at, 0);
        self.uniform_of(s, m)
    }

    // Memory.

    /// The value of `m` at uniform address `a`, uniform.
    fn load_u(&mut self, m: M, a: Value, off: i32) -> Value {
        let b = &mut self.b;
        match m {
            M::B => {
                let x = b.ins().uload8(I64, mf(), a, off);
                norm_u(b, M::B, x)
            }
            M::I8 => b.ins().sload8(I64, mf(), a, off),
            M::U8 => b.ins().uload8(I64, mf(), a, off),
            M::I16 => b.ins().sload16(I64, mf(), a, off),
            M::U16 => b.ins().uload16(I64, mf(), a, off),
            M::I32 => b.ins().sload32(mf(), a, off),
            M::U32 => b.ins().uload32(mf(), a, off),
            M::F32 => b.ins().load(F32, mf(), a, off),
            M::F64 => b.ins().load(F64, mf(), a, off),
            _ => b.ins().load(I64, mf(), a, off),
        }
    }

    fn store_u(&mut self, m: M, a: Value, x: Value, off: i32) {
        let b = &mut self.b;
        match m.bytes() {
            1 => {
                b.ins().istore8(mf(), x, a, off);
            }
            2 => {
                b.ins().istore16(mf(), x, a, off);
            }
            4 if m == M::F32 => {
                b.ins().store(mf(), x, a, off);
            }
            4 => {
                b.ins().istore32(mf(), x, a, off);
            }
            _ => {
                b.ins().store(mf(), x, a, off);
            }
        }
    }

    /// The first lane's address of a frame variable (`FrameAddr`'s `base`, `stride`) in this chunk.
    fn frame_start(&mut self, base: u32, stride: u32) -> Value {
        let o = self.b.ins().imul_imm(self.lane0, i64::from(stride));
        let a = self.b.ins().iadd(self.fp, o);
        self.b.ins().iadd_imm(a, i64::from(base))
    }

    fn load_lanes(&mut self, m: M, addr: V, mask: V) -> Vec<Value> {
        let w = self.w;
        if let Some(&(base, stride)) = self.frame_of.get(&addr) {
            if stride == m.bytes() {
                let at = self.frame_start(base, stride);
                let (n, t) = parts(m, w);
                if w == 1 {
                    let u = self.load_u(m, at, 0);
                    return vec![self.lane_scalar(u, m)];
                }
                return (0..n)
                    .map(|p| match m.bytes() {
                        4 | 8 => self.b.ins().load(t, mf(), at, 16 * p as i32),
                        2 => {
                            if m.signed() { self.b.ins().sload16x4(mf(), at, 8 * p as i32) } else { self.b.ins().uload16x4(mf(), at, 8 * p as i32) }
                        }
                        _ => {
                            let h = if m.signed() { self.b.ins().sload8x8(mf(), at, 4 * p as i32) } else { self.b.ins().uload8x8(mf(), at, 4 * p as i32) };
                            let v = if m.signed() { self.b.ins().swiden_low(h) } else { self.b.ins().uwiden_low(h) };
                            if m == M::B { self.norm32(M::B, v) } else { v }
                        }
                    })
                    .collect();
            }
            // A field of each lane's variable: in the frame, every lane's readable.
            let at = self.frame_start(base, stride);
            let mut xs = Vec::new();
            for j in 0..w {
                let u = self.load_u(m, at, (j * stride) as i32);
                xs.push(self.lane_scalar(u, m));
            }
            return self.assemble(m, &xs);
        }
        let k = self.mask(mask);
        let full = (mask == self.entry_v && self.kernel) || self.all(mask);
        if full {
            if let Some(at) = self.consecutive(addr, m.bytes()) {
                return self.load_contiguous(m, at);
            }
            if let Some(r) = self.load_bytes(m, addr) {
                return r;
            }
            // Every lane's address one: its value, in every lane.
            if let Some(st) = self.steps.get(&addr).filter(|st| st.step == 0).cloned() {
                let at = self.sum(&st.first);
                let u = self.load_u(m, at, 0);
                return self.splat(u, m);
            }
        }
        if w > 1 {
            if let Some((base, off, from)) = self.based(addr) {
                // A uniform base and each lane's 32-bit offset: a load of base and the offset, extended, a lane; a
                // lane that does not run reads the first running lane's place (none running: the scratch area's).
                let po = self.lanes(off, from);
                let (base, po) = if full {
                    (base, po)
                } else {
                    let bits = self.mask_bits(&k);
                    let none = self.b.ins().icmp_imm(IntCC::Equal, bits, 0);
                    let idx = self.b.ins().ctz(bits);
                    let idx = self.b.ins().band_imm(idx, i64::from(w - 1));
                    let first = self.lane_at(&po, from, idx);
                    let first = self.b.ins().ireduce(I32, first);
                    let zero = self.b.ins().iconst(I32, 0);
                    let first = self.b.ins().select(none, zero, first);
                    let scratch = self.b.ins().load(I64, mf(), self.env, self.pl.scratch);
                    let base = self.b.ins().select(none, scratch, base);
                    let sp = self.b.ins().splat(I32X4, first);
                    let po: Vec<Value> = po.iter().zip(&k).map(|(&o, &km)| self.b.ins().bitselect(km, o, sp)).collect();
                    (base, po)
                };
                return self.gather(m, &mut |g, j| {
                    let o = g.b.ins().extractlane(po[(j / 4) as usize], (j % 4) as u8);
                    let o = if from.signed() { g.b.ins().sextend(I64, o) } else { g.b.ins().uextend(I64, o) };
                    g.b.ins().iadd(base, o)
                });
            }
        }
        let pa = self.lanes(addr, M::PTR);
        let pa = if full || w == 1 {
            pa
        } else {
            // A lane that does not run reads the scratch area: the addresses selected as vectors, two lanes a part.
            let scratch = self.b.ins().load(I64, mf(), self.env, self.pl.scratch);
            let ss = self.b.ins().splat(I64X2, scratch);
            pa.iter()
                .enumerate()
                .map(|(p, &a)| {
                    let km = k[p / 2];
                    let wide_m = if p % 2 == 0 { self.b.ins().swiden_low(km) } else { self.b.ins().swiden_high(km) };
                    self.b.ins().bitselect(wide_m, a, ss)
                })
                .collect()
        };
        if w == 1 && !full {
            let scratch = self.b.ins().load(I64, mf(), self.env, self.pl.scratch);
            let on = self.mask_lane(&k, 0);
            let a = self.b.ins().select(on, pa[0], scratch);
            let u = self.load_u(m, a, 0);
            return vec![self.lane_scalar(u, m)];
        }
        self.gather(m, &mut |g, j| g.extract(&pa, M::PTR, j))
    }

    /// If lanes address `addr` is a uniform base plus each lane's 32-bit integer, extended: the base, the integer's
    /// value and its type.
    fn based(&mut self, addr: V) -> Option<(Value, V, M)> {
        let i = *self.f.inst(addr);
        if i.op != O::Bin || Op::of(u32::from(i.sub)) != Op::Add {
            return None;
        }
        for (u, o) in [(i.a, i.b), (i.b, i.a)] {
            let Rp::U(base) = self.vals[u as usize] else { continue };
            let c = *self.f.inst(o);
            if c.op != O::Cast || c.shape != Shape::Lanes {
                continue;
            }
            let from = M(c.b);
            if from.bytes() == 4 && !from.float() && !from.is_agg() && matches!(self.vals[c.a as usize], Rp::L(_)) {
                return Some((base, c.a, from));
            }
        }
        None
    }

    /// The parts of `m` whose lane `j` is the value of `m` at the address `addr(j)` gives, each part made as its lanes
    /// are loaded (no lane's value kept past its part's).
    fn gather(&mut self, m: M, addr: &mut dyn FnMut(&mut Self, u32) -> Value) -> Vec<Value> {
        if self.w == 1 {
            let a = addr(self, 0);
            let u = self.load_u(m, a, 0);
            return vec![self.lane_scalar(u, m)];
        }
        let (n, t) = parts(m, self.w);
        let per = if wide(m) { 2 } else { 4 };
        let mut out = Vec::with_capacity(n);
        for p in 0..n as u32 {
            let mut v = None;
            for l in 0..per {
                let j = p * per + l;
                if j >= self.w {
                    break;
                }
                let a = addr(self, j);
                let u = self.load_u(m, a, 0);
                let x = self.lane_scalar(u, m);
                v = Some(match v {
                    None => self.b.ins().splat(t, x),
                    Some(v) => self.b.ins().insertlane(v, x, l as u8),
                });
            }
            out.push(v.expect("a part's lanes"));
        }
        out
    }

    /// Lanes of 1 or 2 bytes (`m`) stored at `addr`'s steps, `st` bytes apart (`st` 1 to 16), every lane's, at 4
    /// lanes or more: as their bytes. At `st` the bytes' (contiguous), the span's bytes at once; else held, with
    /// the stores before it at the same lanes' addresses but their constant (other bytes of the same elements),
    /// until the held ones' bytes are whole elements (a 3-byte field's 3 stores), then the span's bytes at once;
    /// a store that is not one of them lets the held ones go as they are.
    fn hold_bytes(&mut self, m: M, addr: V, x: V, mask: V) -> bool {
        let eb = i64::from(m.bytes());
        let full = (mask == self.entry_v && self.kernel) || self.all(mask);
        if !matches!(eb, 1 | 2) || m.float() || m.is_agg() || m == M::B || self.w < 4 || !full {
            return false;
        }
        let Some(st) = self.steps.get(&addr).cloned() else { return false };
        if st.step < eb || st.step > 16 {
            return false;
        }
        let key = |l: &Lin| {
            let mut t: Vec<(u32, i64)> = l.terms.iter().map(|t| (t.0.as_u32(), t.1)).collect();
            t.sort_unstable();
            t
        };
        let joins = self.held.first().is_some_and(|&(hm, ha, _, hk)| {
            let h = &self.steps[&ha];
            hm.bytes() == m.bytes() && hk == mask && h.step == st.step && key(&h.first) == key(&st.first)
        });
        if !joins {
            self.release_held();
        }
        self.held.push((m, addr, x, mask));
        // Each held store's bytes from the least's: whole elements when they are `0..step` once each.
        let ks: Vec<i64> = self.held.iter().map(|h| self.steps[&h.1].first.k).collect();
        let k0 = *ks.iter().min().expect("one held");
        let mut cover = vec![0u32; st.step as usize];
        for &k in &ks {
            for c in 0..eb {
                match cover.get_mut((k - k0 + c) as usize) {
                    Some(n) => *n += 1,
                    None => return true,
                }
            }
        }
        if cover.iter().all(|&n| n == 1) {
            let held = std::mem::take(&mut self.held);
            let first = Lin { k: k0, ..st.first.clone() };
            let at = self.sum(&first);
            let items: Vec<(i64, Vec<Value>)> = held.iter().map(|h| (self.steps[&h.1].first.k - k0, self.lanes(h.2, h.0))).collect();
            self.store_span(at, st.step, eb, &items);
        }
        true
    }

    /// The held stores, each as it is.
    fn release_held(&mut self) {
        for (m, a, x, k) in std::mem::take(&mut self.held) {
            self.store_lanes_each(m, a, x, k);
        }
    }

    /// The bytes of lanes `s` bytes apart from `at`: lane `j`'s `eb` bytes of item `(off, parts)` at
    /// `at + j * s + off`, the items' bytes every byte of the `w * s`; stored 16 at a time (the last 16 at the
    /// span's end, again the bytes before them; under 16 bytes, pieces of 8, 4, 2, 1), each 16 a shuffle of the
    /// registers holding them.
    fn store_span(&mut self, at: Value, s: i64, eb: i64, items: &[(i64, Vec<Value>)]) {
        let w = i64::from(self.w);
        let span = w * s;
        let little = MemFlags::new().with_endianness(ir::Endianness::Little);
        // Each byte of the span: its register (an item's part) and its byte there.
        let mut srcs: Vec<Value> = Vec::new();
        let mut src_of: Vec<(usize, u8)> = vec![(usize::MAX, 0); span as usize];
        for (off, ps) in items {
            let base = srcs.len();
            srcs.extend(ps.iter().map(|&p| self.b.ins().bitcast(I8X16, little, p)));
            for j in 0..w {
                for c in 0..eb {
                    src_of[(j * s + off + c) as usize] = (base + (j / 4) as usize, (4 * (j % 4) + c) as u8);
                }
            }
        }
        let chunk = |g: &mut Self, from: i64| -> Value {
            let want: Vec<(usize, u8)> = (0..16).map(|b| src_of.get((from + b) as usize).copied().unwrap_or((usize::MAX, 0))).collect();
            let mut order: Vec<usize> = Vec::new();
            for &(r, _) in &want {
                if r != usize::MAX && !order.contains(&r) {
                    order.push(r);
                }
            }
            // The first two registers' bytes, then each next one's into what is made so far.
            let pick = |want: &[(usize, u8)], a: Option<usize>, b: usize| -> [u8; 16] {
                let mut idx = [0u8; 16];
                for (n, &(r, x)) in want.iter().enumerate() {
                    idx[n] = if Some(r) == a { x } else if r == b { 16 + x } else { n as u8 };
                }
                idx
            };
            let shuf = |g: &mut Self, x: Value, y: Value, idx: [u8; 16]| {
                let im = g.b.func.dfg.immediates.push(ConstantData::from(idx.to_vec()));
                g.b.ins().shuffle(x, y, im)
            };
            let mut acc = {
                let a = order[0];
                let b = *order.get(1).unwrap_or(&a);
                let mut idx = pick(&want, Some(a), b);
                for (n, &(r, x)) in want.iter().enumerate() {
                    if r == a {
                        idx[n] = x;
                    } else if r != b {
                        idx[n] = 0;
                    }
                }
                shuf(g, srcs[a], srcs[b], idx)
            };
            for &r in order.iter().skip(2) {
                let idx = pick(&want, None, r);
                acc = shuf(g, acc, srcs[r], idx);
            }
            acc
        };
        if span >= 16 {
            let mut from = 0;
            while from < span {
                let o = from.min(span - 16);
                let v = chunk(self, o);
                self.b.ins().store(mf(), v, at, o as i32);
                from += 16;
            }
        } else {
            let v = chunk(self, 0);
            let mut done = 0;
            for (size, vt) in [(8, I64X2), (4, I32X4), (2, I16X8), (1, I8X16)] {
                if span - done >= size {
                    let lanes = self.b.ins().bitcast(vt, little, v);
                    let e = self.b.ins().extractlane(lanes, (done / size) as u8);
                    self.b.ins().store(mf(), e, at, done as i32);
                    done += size;
                }
            }
            debug_assert_eq!(done, span);
        }
    }

    fn store_lanes(&mut self, m: M, addr: V, x: V, mask: V) {
        if self.hold_bytes(m, addr, x, mask) {
            return;
        }
        self.store_lanes_each(m, addr, x, mask);
    }

    fn store_lanes_each(&mut self, m: M, addr: V, x: V, mask: V) {
        let w = self.w;
        if let Some(&(base, stride)) = self.frame_of.get(&addr) {
            if stride == m.bytes() && (m.bytes() == 4 || m.bytes() == 8) && w > 1 {
                let at = self.frame_start(base, stride);
                let px = self.lanes(x, m);
                let (_, t) = parts(m, w);
                let full = mask == self.entry_v || self.all(mask);
                let px = if full {
                    px
                } else {
                    let old: Vec<Value> = (0..px.len()).map(|p| self.b.ins().load(t, mf(), at, 16 * p as i32)).collect();
                    self.select_lanes(mask, m, px, old)
                };
                // At a width under a register's lanes, its lanes past the chunk's are the next chunk's: kept.
                let lanes_per = if wide(m) { 2 } else { 4 };
                if w < lanes_per {
                    for j in 0..w {
                        let s = self.extract(&px, m, j);
                        self.b.ins().store(mf(), s, at, (j * m.bytes()) as i32);
                    }
                } else {
                    for (p, v) in px.iter().enumerate() {
                        self.b.ins().store(mf(), *v, at, 16 * p as i32);
                    }
                }
                return;
            }
        }
        if (mask == self.entry_v && self.kernel) || self.all(mask) {
            if let Some(at) = self.consecutive(addr, m.bytes()) {
                let px = self.lanes(x, m);
                if w == 1 || w < if wide(m) { 2 } else { 4 } {
                    for j in 0..w {
                        let s = self.extract(&px, m, j);
                        self.b.ins().store(mf(), s, at, (j * m.bytes()) as i32);
                    }
                } else {
                    for (p, v) in px.iter().enumerate() {
                        self.b.ins().store(mf(), *v, at, 16 * p as i32);
                    }
                }
                return;
            }
        }
        let k = self.mask(mask);
        let pa = self.lanes(addr, M::PTR);
        let px = self.lanes(x, m);
        let scratch = self.b.ins().load(I64, mf(), self.env, self.pl.scratch);
        for j in 0..w {
            let a = self.extract(&pa, M::PTR, j);
            let on = self.mask_lane(&k, j);
            let a = self.b.ins().select(on, a, scratch);
            let s = self.extract(&px, m, j);
            let u = self.uniform_of(s, m);
            self.store_u(m, a, u, 0);
        }
    }

    /// `body(j)` for each lane `j` of mask `k` that runs, in order.
    fn each_lane(&mut self, k: &[Value], body: &mut dyn FnMut(&mut Self, u32)) {
        for j in 0..self.w {
            let on = self.mask_lane(k, j);
            let go = self.b.create_block();
            let next = self.b.create_block();
            self.b.ins().brif(on, go, &[], next, &[]);
            self.b.switch_to_block(go);
            self.splats.clear();
            self.masks64.clear();
            body(self, j);
            self.b.ins().jump(next, &[]);
            self.b.switch_to_block(next);
            self.splats.clear();
            self.masks64.clear();
        }
    }

    /// `n` bytes from `from` to `to` (at most 64 inline, else the runtime's).
    fn copy_bytes(&mut self, to: Value, from: Value, n: u32) {
        if n > 64 {
            let nv = self.b.ins().iconst(I64, i64::from(n));
            self.helper(H_COPY, 3, 0, &[to, from, nv]);
            return;
        }
        let mut parts = Vec::new();
        let mut at = 0u32;
        for (size, t) in [(16u32, I8X16), (8, I64), (4, I32), (2, I16), (1, I8)] {
            while n - at >= size {
                parts.push((at as i32, self.b.ins().load(t, mf(), from, at as i32)));
                at += size;
            }
        }
        for (o, x) in parts {
            self.b.ins().store(mf(), x, to, o);
        }
    }

    fn zero_bytes(&mut self, to: Value, n: u32) {
        if n > 64 {
            let nv = self.b.ins().iconst(I64, i64::from(n));
            self.helper(H_ZERO, 2, 0, &[to, nv]);
            return;
        }
        let z = self.b.ins().iconst(I64, 0);
        let mut at = 0u32;
        while n - at >= 8 {
            self.b.ins().store(mf(), z, to, at as i32);
            at += 8;
        }
        while at < n {
            self.b.ins().istore8(mf(), z, to, at as i32);
            at += 1;
        }
    }

    /// One lane's atomic at `ptr` (uniform values): the old value.
    fn atomic_u(&mut self, bi: Builtin, m: M, ptr: Value, x: Value, y: Value) -> Value {
        let old = self.load_u(m, ptr, 0);
        let new = match bi {
            Builtin::AtomicAdd => self.b.ins().iadd(old, x),
            Builtin::AtomicSub => self.b.ins().isub(old, x),
            Builtin::AtomicAnd => self.b.ins().band(old, x),
            Builtin::AtomicOr => self.b.ins().bor(old, x),
            Builtin::AtomicXor => self.b.ins().bxor(old, x),
            Builtin::AtomicXchg => x,
            Builtin::AtomicInc => self.b.ins().iadd_imm(old, 1),
            Builtin::AtomicDec => self.b.ins().iadd_imm(old, -1),
            Builtin::AtomicMin | Builtin::AtomicMax => {
                let less = self.u_bin(Op::Lt, m, x, old);
                let (p, q) = if bi == Builtin::AtomicMin { (x, old) } else { (old, x) };
                self.b.ins().select(less, p, q)
            }
            _ => {
                let nx = norm_u(&mut self.b, m, x);
                let eq = self.b.ins().icmp(IntCC::Equal, old, nx);
                self.b.ins().select(eq, y, old)
            }
        };
        self.store_u(m, ptr, new, 0);
        old
    }

    // The instructions.

    fn inst(&mut self, x: V, i: &Inst) -> R<Rp> {
        let w = self.w;
        let m = i.m;
        Ok(match i.op {
            O::Const => Rp::U(self.konst_u(m, u64::from(i.a) | u64::from(i.b) << 32)),
            O::Arg => {
                // The kernel's: its arguments, normalized `u64`s in the state.
                let at = self.b.ins().load(I64, mf(), self.sg, self.pl.args);
                let v = self.b.ins().load(I64, mf(), at, 8 * i.a as i32);
                Rp::U(match m {
                    M::F32 => {
                        let t = self.b.ins().ireduce(I32, v);
                        self.b.ins().bitcast(F32, MemFlags::new(), t)
                    }
                    M::F64 => self.b.ins().bitcast(F64, MemFlags::new(), v),
                    _ => v,
                })
            }
            O::Entry => unreachable!("the entry's lanes are the function's"),
            O::NoLanes => Rp::K(self.zero_mask()),
            O::Undef => Rp::U(self.konst_u(m, 0)),
            O::Param => unreachable!("a parameter is its block's"),
            O::Bin => {
                let op = Op::of(u32::from(i.sub));
                let om = M(i.d);
                if i.shape == Shape::Uniform {
                    let (a, b) = (self.u(i.a), self.u(i.b));
                    // A 32-bit unsigned sum or product no value can wrap (`bound`): the 64-bit one, already its
                    // normalized value.
                    let whole = matches!(op, Op::Add | Op::Mul) && !om.float() && !om.signed() && om.bytes() == 4 && self.bound(x).is_some();
                    if whole {
                        return Ok(Rp::U(if op == Op::Add { self.b.ins().iadd(a, b) } else { self.b.ins().imul(a, b) }));
                    }
                    Rp::U(self.u_bin(op, om, a, b))
                } else {
                    if let Some(st) = self.stepped(op, om, i.a, i.b, x) {
                        self.steps.insert(x, st);
                    }
                    if let Some(r) = self.group_bin(op, om, i.a, i.b, x) {
                        return Ok(Rp::L(r));
                    }
                    let (r, k) = self.bin_lanes(op, om, i.a, i.b, i.c);
                    if let Some(k) = k {
                        self.raw.insert(x, k);
                    }
                    Rp::L(r)
                }
            }
            O::Un => {
                let op = Op::of(u32::from(i.sub));
                let om = M(i.d);
                if i.shape == Shape::Uniform {
                    let a = self.u(i.a);
                    Rp::U(self.u_un(op, om, a))
                } else {
                    let p = self.lanes(i.a, om);
                    let r = match (op, om.float()) {
                        (Op::Neg, true) => p.iter().map(|&v| self.b.ins().fneg(v)).collect(),
                        (Op::Neg, false) => p.iter().map(|&v| self.b.ins().ineg(v)).collect::<Vec<_>>().into_iter().map(|v| self.norm32_if(om, v)).collect(),
                        (Op::Not, false) => p.iter().map(|&v| self.b.ins().bnot(v)).collect::<Vec<_>>().into_iter().map(|v| self.norm32_if(om, v)).collect(),
                        (Op::LNot, false) => {
                            let z = self.zero_lanes(om);
                            let k = self.cmp_lanes(Op::Eq, om, &p, &z);
                            self.raw.insert(x, k.clone());
                            self.bools_of(&k)
                        }
                        _ => self.per_lane(&[(p, om)], m, &mut |g, xs| g.u_un(op, om, xs[0])),
                    };
                    Rp::L(r)
                }
            }
            O::Cast => {
                let from = M(i.b);
                if i.shape == Shape::Uniform {
                    let a = self.u(i.a);
                    let int = |t: M| !t.float() && !t.is_agg() && !t.signed() && t != M::B && t.bytes() > 0;
                    // A narrowing of an integer the target holds: its normalized `I64` already.
                    if int(from) && int(m) && self.fits_in(i.a, m) {
                        Rp::U(a)
                    } else {
                        Rp::U(self.u_cast(from, m, a))
                    }
                } else {
                    if let Some(st) = self.steps.get(&i.a).cloned() {
                        let int = |t: M| !t.float() && !t.is_agg() && t != M::B && t.bytes() >= 4;
                        if int(from) && int(m) && (m.bytes() <= from.bytes() || st.exact || from.bytes() == 8) {
                            // A narrowing keeps the bits, a widening the integer (when it is one).
                            let exact = st.exact && (m.bytes() >= from.bytes() || self.f.inst(i.a).op == O::Lid);
                            self.steps.insert(x, Steps { exact, ..st });
                        }
                    }
                    let r = self.cast_lanes(from, m, i.a);
                    if let Some(k) = self.raw.get(&i.a).cloned().filter(|_| m == M::B && from != M::B) {
                        self.raw.insert(x, k);
                    }
                    Rp::L(r)
                }
            }
            O::Select => {
                let a = self.lanes(i.b, m);
                let b = self.lanes(i.c, m);
                Rp::L(self.select_lanes(i.a, m, a, b))
            }
            O::First => {
                let mk = self.mask(i.b);
                match self.vals[i.a as usize].clone() {
                    Rp::U(u) => Rp::U(u),
                    Rp::L(p) => {
                        // A chunk whose mask has no lane (another chunk's has): its entry's first lane, whose value
                        // the code computed; no lane uses it.
                        let bits = self.mask_bits(&mk);
                        let eb = self.entry_bits;
                        let some = self.b.ins().icmp_imm(IntCC::NotEqual, bits, 0);
                        let bits = self.b.ins().select(some, bits, eb);
                        let idx = self.b.ins().ctz(bits);
                        let idx = self.b.ins().band_imm(idx, i64::from(w - 1));
                        Rp::U(self.lane_at(&p, m, idx))
                    }
                    r => return Err(Error(format!("a first lane of {r:?}"))),
                }
            }
            O::MaskOf => {
                if let Some(k) = self.raw.get(&i.a) {
                    return Ok(Rp::K(k.clone()));
                }
                match self.vals[i.a as usize].clone() {
                    Rp::U(u) => {
                        let t = self.b.ins().icmp_imm(IntCC::NotEqual, u, 0);
                        let s = self.b.ins().bmask(I32, t);
                        Rp::K(if w == 1 { vec![s] } else { vec![self.b.ins().splat(I32X4, s); mask_parts(w)] })
                    }
                    Rp::L(p) => {
                        let am = self.m_of(i.a);
                        let z = self.zero_lanes(am);
                        Rp::K(self.cmp_lanes(Op::Ne, am, &p, &z))
                    }
                    r => return Err(Error(format!("a mask of {r:?}"))),
                }
            }
            O::MAnd if self.all(i.a) || self.all(i.b) => {
                let o = if self.all(i.a) { i.b } else { i.a };
                if self.all(o) {
                    self.ones.insert(x);
                }
                self.vals[o as usize].clone()
            }
            O::MOr if self.all(i.a) || self.all(i.b) => {
                self.ones.insert(x);
                self.vals[if self.all(i.a) { i.a } else { i.b } as usize].clone()
            }
            O::MAnd | O::MOr | O::MAndNot => {
                let (a, b) = (self.mask(i.a), self.mask(i.b));
                Rp::K(
                    a.iter()
                        .zip(&b)
                        .map(|(&p, &q)| match i.op {
                            O::MAnd => self.b.ins().band(p, q),
                            O::MOr => self.b.ins().bor(p, q),
                            _ => self.b.ins().band_not(p, q),
                        })
                        .collect(),
                )
            }
            O::AnyGroup if self.chunked => Rp::U(self.x_any(i.a)),
            O::Any | O::AnyGroup if self.all(i.a) => Rp::U(self.b.ins().iconst(I64, 1)),
            O::Any if self.lane_test(i.a).is_some() => {
                // Whether lane `u` is in the chunk: `u - lane0 < w`.
                let uv = self.lane_test(i.a).expect("a lane's test");
                if self.w == 32 {
                    // A SIMD group's one chunk: a lane under 32 runs.
                    if let Some(k) = self.konst(uv) {
                        return Ok(Rp::U(self.b.ins().iconst(I64, i64::from(k < 32))));
                    }
                }
                let u = self.u(uv);
                let j = self.b.ins().isub(u, self.lane0);
                let c = self.b.ins().icmp_imm(IntCC::UnsignedLessThan, j, i64::from(self.w));
                Rp::U(self.b.ins().uextend(I64, c))
            }
            O::Any | O::AnyGroup => {
                let k = self.mask(i.a);
                Rp::U(self.any(&k))
            }
            O::Bits if self.chunked => {
                let bits = self.x_bits(i.a);
                Rp::U(self.b.ins().uextend(I64, bits))
            }
            O::Bits => {
                let k = self.mask(i.a);
                let bits = self.mask_bits(&k);
                Rp::U(self.b.ins().uextend(I64, bits))
            }
            O::Global | O::Shared => {
                let base = self.b.ins().load(I64, mf(), self.env, if i.op == O::Global { self.pl.data } else { self.pl.shared });
                Rp::U(self.b.ins().iadd_imm(base, i64::from(i.a)))
            }
            O::FrameAddr => {
                self.frame_of.insert(x, (i.a, i.b));
                let start = self.frame_start(i.a, i.b);
                if w == 1 {
                    Rp::L(vec![start])
                } else {
                    let s = self.b.ins().splat(I64X2, start);
                    let stride = u64::from(i.b);
                    let (n, _) = parts(M::PTR, w);
                    Rp::L(
                        (0..n)
                            .map(|p| {
                                let l = 2 * p as u64;
                                let o = vconst64(&mut self.b, [l * stride, (l + 1) * stride]);
                                self.b.ins().iadd(s, o)
                            })
                            .collect(),
                    )
                }
            }
            O::Load => match self.vals[i.a as usize].clone() {
                Rp::U(_) => {
                    let a = self.addr_u(i.a);
                    let a = self.guarded(a, i.b);
                    Rp::U(self.load_u(m, a, 0))
                }
                _ => Rp::L(self.load_lanes(m, i.a, i.b)),
            },
            O::Store => {
                match (self.vals[i.a as usize].clone(), self.vals[i.b as usize].clone()) {
                    (Rp::U(_), Rp::U(v)) => {
                        let a = self.addr_u(i.a);
                        let a = self.guarded(a, i.c);
                        self.store_u(m, a, v, 0)
                    }
                    (Rp::U(a), Rp::L(p)) => {
                        // Each running lane's in order: the last one's stays.
                        let a = self.guarded(a, i.c);
                        let k = self.mask(i.c);
                        let bits = self.mask_bits(&k);
                        let c = self.b.ins().clz(bits);
                        let idx = self.b.ins().irsub_imm(c, 31);
                        let idx = self.b.ins().band_imm(idx, i64::from(w - 1));
                        let v = self.lane_at(&p, m, idx);
                        self.store_u(m, a, v, 0);
                    }
                    _ => self.store_lanes(m, i.a, i.b, i.c),
                }
                Rp::None
            }
            O::Copy | O::Zero => {
                let n = m.0;
                let k = self.mask(if i.op == O::Copy { i.c } else { i.b });
                let to = self.lanes(i.a, M::PTR);
                let from = if i.op == O::Copy { self.lanes(i.b, M::PTR) } else { Vec::new() };
                let copy = i.op == O::Copy;
                self.each_lane(&k, &mut |g, j| {
                    let t = g.extract(&to, M::PTR, j);
                    if copy {
                        let f = g.extract(&from, M::PTR, j);
                        g.copy_bytes(t, f, n);
                    } else {
                        g.zero_bytes(t, n);
                    }
                });
                Rp::None
            }
            O::Lid | O::Gid if i.shape == Shape::Uniform => {
                // The SIMD group's (a dimension past the first): its first lane's.
                let dim = self.u(i.a);
                let off = self.b.ins().imul_imm(dim, 4 * 32);
                let row = self.b.ins().iadd(self.sg, off);
                let l4 = self.b.ins().imul_imm(self.lane0, 4);
                let at = self.b.ins().iadd(row, l4);
                let mut x = self.b.ins().uload32(mf(), at, self.pl.lid);
                if i.op == O::Gid {
                    let d4 = self.b.ins().imul_imm(dim, 4);
                    let e = self.b.ins().iadd(self.env, d4);
                    let g = self.b.ins().uload32(mf(), e, self.pl.group);
                    let bl = self.b.ins().uload32(mf(), e, self.pl.block);
                    let gb = self.b.ins().imul(g, bl);
                    x = self.b.ins().iadd(x, gb);
                }
                Rp::U(norm_u(&mut self.b, m, x))
            }
            O::Lid | O::Gid => {
                // The first dimension's: the chunk's first lane's and the lane (`exec::run` takes only groups whose
                // SIMD groups' threads are consecutive in it).
                if self.konst(i.a) == Some(0) {
                    let l4 = self.b.ins().imul_imm(self.lane0, 4);
                    let at = self.b.ins().iadd(self.sg, l4);
                    let mut first = self.b.ins().uload32(mf(), at, self.pl.lid);
                    if i.op == O::Gid {
                        let g = self.b.ins().uload32(mf(), self.env, self.pl.group);
                        let bl = self.b.ins().uload32(mf(), self.env, self.pl.block);
                        let gb = self.b.ins().imul(g, bl);
                        first = self.b.ins().iadd(first, gb);
                    }
                    let d = self.depth[self.cur_block];
                    // The SIMD group's lane 0's: a multiple of 32 (its lanes are 32 consecutive threads from one).
                    let group = (i.op == O::Lid).then(|| {
                        let l0 = self.b.ins().uload32(mf(), self.sg, self.pl.lid);
                        let q = self.b.ins().ushr_imm(l0, 5);
                        Lin::leaf(q, 0).times(32)
                    });
                    self.steps.insert(x, Steps { first: Lin::leaf(first, d), step: 1, exact: true, group });
                    return Ok(Rp::L(self.iota(first, m)));
                }
                let dim = self.u(i.a);
                let lt3 = self.b.ins().icmp_imm(IntCC::UnsignedLessThan, dim, 3);
                let zero = self.b.ins().iconst(I64, 0);
                let d = self.b.ins().select(lt3, dim, zero);
                let off = self.b.ins().imul_imm(d, 4 * 32);
                let row = self.b.ins().iadd(self.sg, off);
                let l4 = self.b.ins().imul_imm(self.lane0, 4);
                let at = self.b.ins().iadd(row, l4);
                let base = if i.op == O::Gid {
                    let d4 = self.b.ins().imul_imm(d, 4);
                    let e = self.b.ins().iadd(self.env, d4);
                    let g = self.b.ins().uload32(mf(), e, self.pl.group);
                    let bl = self.b.ins().uload32(mf(), e, self.pl.block);
                    Some(self.b.ins().imul(g, bl))
                } else {
                    None
                };
                let mut xs: Vec<Value> = if w == 1 {
                    vec![self.b.ins().load(I32, mf(), at, self.pl.lid)]
                } else {
                    (0..mask_parts(w)).map(|p| self.b.ins().load(I32X4, mf(), at, self.pl.lid + 16 * p as i32)).collect()
                };
                // Past the third dimension: 0.
                let lt = self.b.ins().bmask(I32, lt3);
                let ltv = if w == 1 { lt } else { self.b.ins().splat(I32X4, lt) };
                xs = xs.into_iter().map(|v| self.b.ins().band(v, ltv)).collect();
                let wide_m = wide(m);
                let mut r = if wide_m {
                    if w == 1 {
                        vec![self.b.ins().uextend(I64, xs[0])]
                    } else {
                        let (n, _) = parts(m, w);
                        (0..n).map(|j| if j % 2 == 0 { self.b.ins().uwiden_low(xs[j / 2]) } else { self.b.ins().uwiden_high(xs[j / 2]) }).collect()
                    }
                } else {
                    xs
                };
                if let Some(base) = base {
                    let base = self.b.ins().select(lt3, base, zero);
                    let sp = self.splat(base, if wide_m { M::U64 } else { M::U32 });
                    r = r.iter().zip(&sp).map(|(&a, &b)| self.b.ins().iadd(a, b)).collect();
                }
                Rp::L(r)
            }
            O::Lane => {
                let l = self.b.ins().ireduce(I32, self.lane0);
                if w == 1 {
                    Rp::L(vec![l])
                } else {
                    let s = self.b.ins().splat(I32X4, l);
                    Rp::L(
                        (0..mask_parts(w))
                            .map(|p| {
                                let b0 = 4 * p as u32;
                                let o = vconst32(&mut self.b, [b0, b0 + 1, b0 + 2, b0 + 3]);
                                self.b.ins().iadd(s, o)
                            })
                            .collect(),
                    )
                }
            }
            O::GroupInfo => {
                let bi = Builtin::of(u32::from(i.sub));
                let dim = self.u(i.a);
                let lt3 = self.b.ins().icmp_imm(IntCC::UnsignedLessThan, dim, 3);
                let zero = self.b.ins().iconst(I64, 0);
                let d = self.b.ins().select(lt3, dim, zero);
                let d4 = self.b.ins().imul_imm(d, 4);
                let e = self.b.ins().iadd(self.env, d4);
                let (off, or) = match bi {
                    Builtin::LocalSize => (self.pl.block, 1),
                    Builtin::GroupId => (self.pl.group, 0),
                    _ => (self.pl.grid, 1),
                };
                let v = self.b.ins().uload32(mf(), e, off);
                let v = if bi == Builtin::GlobalSize {
                    let bl = self.b.ins().uload32(mf(), e, self.pl.block);
                    self.b.ins().imul(v, bl)
                } else {
                    v
                };
                let orv = self.b.ins().iconst(I64, or);
                let r = self.b.ins().select(lt3, v, orv);
                Rp::U(norm_u(&mut self.b, m, r))
            }
            O::LaneOp if Builtin::of(u32::from(i.sub)) == Builtin::Fma => {
                // `a * b + c` rounded once: Cranelift's `fma` (AArch64's `fmla`, by an element where an operand is
                // uniform).
                if i.shape == Shape::Uniform {
                    let (a, b, c) = (self.u(i.a), self.u(i.b), self.u(i.d));
                    Rp::U(self.b.ins().fma(a, b, c))
                } else {
                    let (pa, pb, pc) = (self.lanes(i.a, m), self.lanes(i.b, m), self.lanes(i.d, m));
                    Rp::L((0..pa.len()).map(|p| self.b.ins().fma(pa[p], pb[p], pc[p])).collect())
                }
            }
            O::LaneOp => {
                let bi = Builtin::of(u32::from(i.sub));
                let from = M(i.c);
                let second = if i.b != NONE { self.m_of(i.b) } else { from };
                if i.shape == Shape::Uniform {
                    let a = self.u(i.a);
                    let b = if i.b != NONE { self.u(i.b) } else { a };
                    Rp::U(self.u_laneop(bi, m, from, a, b))
                } else {
                    let pa = self.lanes(i.a, from);
                    let native = matches!((bi, m), (Builtin::Min | Builtin::Max, M::I32 | M::U32)) && i.b != NONE && w > 1;
                    let same = |x: M| !x.is_agg() && (x.float() || x.bytes() >= 4) && x.bytes() == from.bytes();
                    if bi == Builtin::As && same(m) && same(from) && matches!(from.bytes(), 4 | 8) {
                        // The bits as another type of their width: each register's bits, as they are.
                        let (_, t) = parts(m, w);
                        let flags = MemFlags::new().with_endianness(ir::Endianness::Little);
                        Rp::L(pa.iter().map(|&p| self.b.ins().bitcast(t, flags, p)).collect())
                    } else if native {
                        let pb = self.lanes(i.b, m);
                        Rp::L(
                            pa.iter()
                                .zip(&pb)
                                .map(|(&p, &q)| match (bi, m) {
                                    (Builtin::Min, M::I32) => self.b.ins().smin(p, q),
                                    (Builtin::Min, _) => self.b.ins().umin(p, q),
                                    (Builtin::Max, M::I32) => self.b.ins().smax(p, q),
                                    _ => self.b.ins().umax(p, q),
                                })
                                .collect(),
                        )
                    } else {
                        let mut ops = vec![(pa, from)];
                        if i.b != NONE {
                            let pb = self.lanes(i.b, second);
                            ops.push((pb, second));
                        }
                        Rp::L(self.per_lane(&ops, m, &mut |g, xs| {
                            let b = if xs.len() > 1 { xs[1] } else { xs[0] };
                            g.u_laneop(bi, m, from, xs[0], b)
                        }))
                    }
                }
            }
            O::Shuffle => {
                let bi = Builtin::of(u32::from(i.sub));
                let k = self.mask(i.c);
                let px = self.lanes(i.a, m);
                let (bits, base) = if self.chunked {
                    // The SIMD group's lanes, from the exchange area.
                    let at = self.x_half();
                    self.x_put_lanes(at, &px, m);
                    self.x_put_bits(at, &k);
                    self.x_wait();
                    let at = self.x_read();
                    (self.b.ins().load(I32, mf(), at, XWORD), at)
                } else {
                    let bits = self.mask_bits(&k);
                    for (p, &v) in px.iter().enumerate() {
                        let sl = self.slot();
                        self.b.ins().stack_store(v, sl, 16 * p as i32);
                    }
                    let sl = self.slot();
                    (bits, self.b.ins().stack_addr(I64, sl, 0))
                };
                // The operands as the region after the exchange has them.
                let (px, pl) = (self.lanes(i.a, m), self.lanes(i.b, M::U32));
                let l0 = if self.chunked { Some(self.b.ins().ireduce(I32, self.lane0)) } else { None };
                let size = if wide(m) { 8 } else { 4 };
                let (_, lt) = parts(m, 1);
                let mut out = Vec::new();
                for l in 0..w {
                    let idx = self.extract(&pl, M::U32, l);
                    let own = self.extract(&px, m, l);
                    let lv = match l0 {
                        Some(l0) => self.b.ins().iadd_imm(l0, i64::from(l)),
                        None => self.b.ins().iconst(I32, i64::from(l)),
                    };
                    let (src, inrange, keep) = match bi {
                        Builtin::SubGroupShuffle => (self.b.ins().band_imm(idx, 31), None, false),
                        Builtin::SubGroupShuffleXor => {
                            let s = self.b.ins().bxor(lv, idx);
                            (self.b.ins().band_imm(s, 31), None, false)
                        }
                        Builtin::SubGroupShuffleUp => {
                            let ok = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, idx, lv);
                            let s = self.b.ins().isub(lv, idx);
                            let s = self.b.ins().band_imm(s, 31);
                            (s, Some(ok), true)
                        }
                        _ => {
                            let s = self.b.ins().iadd(lv, idx);
                            let ok = self.b.ins().icmp_imm(IntCC::UnsignedLessThan, s, 32);
                            let s = self.b.ins().band_imm(s, 31);
                            (s, Some(ok), true)
                        }
                    };
                    let on = self.b.ins().ushr(bits, src);
                    let on = self.b.ins().band_imm(on, 1);
                    let on = self.b.ins().icmp_imm(IntCC::NotEqual, on, 0);
                    let ok = match inrange {
                        Some(r) => self.b.ins().band(r, on),
                        None => on,
                    };
                    let s64 = self.b.ins().uextend(I64, src);
                    let off = self.b.ins().imul_imm(s64, size);
                    let at = self.b.ins().iadd(base, off);
                    let v = self.b.ins().load(lt, mf(), at, 0);
                    let other = if keep {
                        own
                    } else if lt.is_float() {
                        if lt == F32 { self.b.ins().f32const(0.0) } else { self.b.ins().f64const(0.0) }
                    } else {
                        self.b.ins().iconst(lt, 0)
                    };
                    out.push(self.b.ins().select(ok, v, other));
                }
                Rp::L(self.assemble(m, &out))
            }
            O::Reduce => {
                let bi = Builtin::of(u32::from(i.sub));
                let k = self.mask(i.b);
                let px = self.lanes(i.a, m);
                if self.chunked {
                    // The SIMD group's lanes, from the exchange area, folded in each chunk alike.
                    let at = self.x_half();
                    self.x_put_lanes(at, &px, m);
                    self.x_put_bits(at, &k);
                    self.x_wait();
                    let at = self.x_read();
                    let word = self.b.ins().load(I32, mf(), at, XWORD);
                    let size = if wide(m) { 8 } else { 4 };
                    let (_, lt) = parts(m, 1);
                    Rp::U(self.fold(bi, m, 32, &mut |g, l| {
                        let on = g.b.ins().ushr_imm(word, i64::from(l));
                        let on = g.b.ins().band_imm(on, 1);
                        let on = g.b.ins().icmp_imm(IntCC::NotEqual, on, 0);
                        let s = g.b.ins().load(lt, mf(), at, (l * size) as i32);
                        (on, g.uniform_of(s, m))
                    }))
                } else if self.w >= 4 && !m.float() && (bi == Builtin::SubGroupReduceAdd || !wide(m)) {
                    Rp::U(self.reduce_lanes(bi, m, i.b, px))
                } else {
                    let w = self.w;
                    Rp::U(self.fold(bi, m, w, &mut |g, l| {
                        let on = g.mask_lane(&k, l);
                        let s = g.extract(&px, m, l);
                        (on, g.uniform_of(s, m))
                    }))
                }
            }
            O::Atomic => {
                let bi = Builtin::of(u32::from(i.sub));
                if i.shape == Shape::Uniform && self.chunked {
                    // Once for the SIMD group: by the chunk that has its first lane of the mask, whose result every
                    // chunk reads.
                    let k = self.mask(i.d);
                    let at = self.x_half();
                    self.x_put_bits(at, &k);
                    self.x_wait();
                    let at = self.x_read();
                    let word = self.b.ins().load(I32, mf(), at, XWORD);
                    let lead = self.x_lead(word);
                    let some = self.b.ins().icmp_imm(IntCC::NotEqual, word, 0);
                    let mine = self.b.ins().icmp(IntCC::Equal, lead, self.lane0);
                    let go = self.b.ins().band(some, mine);
                    let rt = uty(m);
                    let [does, skip, zero, keep] = std::array::from_fn(|_| self.b.create_block());
                    self.b.ins().brif(go, does, &[], skip, &[]);
                    self.b.switch_to_block(does);
                    let p = self.u(i.a);
                    let xv = if i.b != NONE { self.u(i.b) } else { self.konst_u(m, 0) };
                    let yv = if i.c != NONE { self.u(i.c) } else { self.konst_u(m, 0) };
                    let r = self.atomic_u(bi, m, p, xv, yv);
                    // The leader's result at the half's first place, for every chunk; with no lane of the mask,
                    // every chunk's 0.
                    let r64 = if rt == I64 { r } else { self.b.ins().bitcast(if rt == F32 { I32 } else { I64 }, MemFlags::new(), r) };
                    let r64 = if self.b.func.dfg.value_type(r64) == I32 { self.b.ins().uextend(I64, r64) } else { r64 };
                    let at2 = self.x_half();
                    self.b.ins().store(mf(), r64, at2, 0);
                    self.b.ins().jump(keep, &[]);
                    self.b.switch_to_block(skip);
                    self.b.ins().brif(some, keep, &[], zero, &[]);
                    self.b.switch_to_block(zero);
                    let at2 = self.x_half();
                    let z = self.b.ins().iconst(I64, 0);
                    self.b.ins().store(mf(), z, at2, 0);
                    self.b.ins().jump(keep, &[]);
                    self.b.switch_to_block(keep);
                    self.x_wait();
                    let at2 = self.x_read();
                    let r = self.b.ins().load(I64, mf(), at2, 0);
                    Rp::U(if rt == I64 {
                        r
                    } else {
                        let r = if rt == F32 { self.b.ins().ireduce(I32, r) } else { r };
                        self.b.ins().bitcast(rt, MemFlags::new(), r)
                    })
                } else if i.shape == Shape::Uniform {
                    let p = self.u(i.a);
                    let xv = if i.b != NONE { self.u(i.b) } else { self.konst_u(m, 0) };
                    let yv = if i.c != NONE { self.u(i.c) } else { self.konst_u(m, 0) };
                    Rp::U(self.atomic_u(bi, m, p, xv, yv))
                } else {
                    let k = self.mask(i.d);
                    let pa = self.lanes(i.a, M::PTR);
                    let px = if i.b != NONE { Some(self.lanes(i.b, m)) } else { None };
                    let py = if i.c != NONE { Some(self.lanes(i.c, m)) } else { None };
                    let slot = self.slot();
                    let size = if wide(m) { 8 } else { 4 };
                    // Each running lane's in order, its old value to the slot.
                    let z = self.zero_lanes(m);
                    for (p, &v) in z.iter().enumerate() {
                        self.b.ins().stack_store(v, slot, 16 * p as i32);
                    }
                    self.each_lane(&k, &mut |g, j| {
                        let a = g.extract(&pa, M::PTR, j);
                        let xv = match &px {
                            Some(p) => {
                                let s = g.extract(p, m, j);
                                g.uniform_of(s, m)
                            }
                            None => g.konst_u(m, 0),
                        };
                        let yv = match &py {
                            Some(p) => {
                                let s = g.extract(p, m, j);
                                g.uniform_of(s, m)
                            }
                            None => g.konst_u(m, 0),
                        };
                        let old = g.atomic_u(bi, m, a, xv, yv);
                        let s = g.lane_scalar(old, m);
                        g.b.ins().stack_store(s, slot, (j * size) as i32);
                    });
                    let (n, t) = parts(m, w);
                    Rp::L(if w == 1 {
                        let (_, lt) = parts(m, 1);
                        vec![self.b.ins().stack_load(lt, slot, 0)]
                    } else {
                        (0..n).map(|p| self.b.ins().stack_load(t, slot, 16 * p as i32)).collect()
                    })
                }
            }
            O::Agree => {
                // A chunk's decision is its SIMD group's (the leading chunk's) before the groups compare theirs.
                let d = self.u(i.c);
                let d = if self.chunked { self.x_leader(d, i.d) } else { d };
                let node = self.b.ins().iconst(I64, i64::from(i.a));
                let site = self.b.ins().iconst(I64, i64::from(i.b));
                let sg = self.sg;
                if self.chunked {
                    // Once for the SIMD group: by its first chunk with a lane.
                    let live = self.b.ins().uload32(mf(), sg, self.pl.live);
                    let first = self.b.ins().ctz(live);
                    let first = self.b.ins().band_imm(first, !(i64::from(self.w) - 1));
                    let mine = self.b.ins().icmp(IntCC::Equal, first, self.lane0);
                    let (call, join) = (self.b.create_block(), self.b.create_block());
                    self.b.ins().brif(mine, call, &[], join, &[]);
                    self.b.switch_to_block(call);
                    self.helper(H_AGREE, 4, 0, &[sg, node, d, site]);
                    self.b.ins().jump(join, &[]);
                    self.b.switch_to_block(join);
                } else {
                    self.helper(H_AGREE, 4, 0, &[sg, node, d, site]);
                }
                self.check_failed();
                if m == M::VOID { Rp::None } else { Rp::U(d) }
            }
            // A chunked kernel's barrier and poll are its SIMD group's, once its chunks are all at them.
            O::Bar if self.chunked => {
                self.split(ACT_BAR | i64::from(i.a) << 8);
                Rp::None
            }
            O::Bar => {
                let site = self.b.ins().iconst(I64, i64::from(i.a));
                let sg = self.sg;
                self.helper(H_BAR, 2, 0, &[sg, site]);
                Rp::None
            }
            // At the group's width its lanes run in step: what one wrote, the others read after it; a chunk's region
            // ends, and the next starts once the others have run theirs.
            O::SgBar => {
                if self.chunked {
                    self.split(ACT_NONE);
                }
                Rp::None
            }
            O::Yield if self.chunked => {
                self.split(ACT_YIELD);
                Rp::None
            }
            O::Yield => {
                let sg = self.sg;
                self.helper(H_YIELD, 1, 0, &[sg]);
                Rp::None
            }
            O::Call => {
                let callee = &self.ssa.funcs[i.a as usize];
                // The frame's advance: past the body the call is in (the extra word after its arguments).
                let region = self.f.extra[(i.b + i.c) as usize];
                let fp = self.b.ins().iadd_imm(self.fp, i64::from(region) * 32);
                let mut args = vec![self.sg, fp, self.lane0];
                args.extend(self.mask(i.d));
                let vals: Vec<V> = self.f.args_of(i).to_vec();
                for (k, &v) in vals.iter().enumerate() {
                    let (am, sh) = callee.args[k];
                    match sh {
                        Shape::Uniform => args.push(self.u(v)),
                        _ => args.extend(self.lanes(v, am)),
                    }
                }
                let fr = self.callees[i.a as usize];
                let call = self.b.ins().call(fr, &args);
                let res = self.b.inst_results(call).to_vec();
                self.check_failed();
                match callee.ret.1 {
                    Shape::Uniform => Rp::U(res[0]),
                    Shape::Lanes => Rp::L(res),
                    _ => Rp::None,
                }
            }
            O::Jump => {
                let to = i.a as usize;
                let targs: Vec<V> = self.f.args_of(i).to_vec();
                let params: Vec<V> = self.f.blocks[to].params.clone();
                let mut each = Vec::new();
                for (&a, &p) in targs.iter().zip(&params) {
                    let pi = self.f.inst(p);
                    let (kind, vs) = match pi.shape {
                        Shape::Uniform if self.counted.contains_key(&p) => {
                            let k = self.counted[&p].1;
                            let l = self.lin(a).plus(Lin::konst(-k));
                            (0, vec![self.sum(&l)])
                        }
                        Shape::Uniform => (0, vec![self.u(a)]),
                        Shape::Lanes => (1, self.lanes(a, pi.m)),
                        Shape::Mask => (2, self.mask(a)),
                        Shape::None => continue,
                    };
                    each.push((p, kind, vs));
                }
                if self.resume[to] {
                    // A block that starts a region: the values it reads, and its parameters, kept for it.
                    let k = self.reg(crate::ssa::Lab::Join(to));
                    let live = self.live_in(to);
                    self.save(&live);
                    for (p, kind, vs) in &each {
                        self.save_parts(*p, *kind, vs);
                    }
                    self.leave(k as i64, ACT_NONE);
                    return Ok(Rp::None);
                }
                let ba: Vec<BlockArg> = each.into_iter().flat_map(|(_, _, vs)| vs).map(BlockArg::Value).collect();
                self.b.ins().jump(self.blocks[to], &ba);
                Rp::None
            }
            O::Br if self.count_tests.contains_key(&{
                let mut c = i.a;
                while self.f.inst(c).op == O::Cast && self.f.inst(c).shape == Shape::Uniform {
                    c = self.f.inst(c).a;
                }
                c
            }) => {
                // A counted loop's test: its counter's next value (`k + 1 - N`) not 0.
                let mut c = i.a;
                while self.f.inst(c).op == O::Cast && self.f.inst(c).shape == Shape::Uniform {
                    c = self.f.inst(c).a;
                }
                let t = *self.f.inst(c);
                let k = self.counted[&self.count_tests[&c]].1;
                let l = self.lin(t.a).plus(Lin::konst(-k));
                let n = self.sum(&l);
                self.b.ins().brif(n, self.blocks[i.b as usize], &[], self.blocks[i.c as usize], &[]);
                Rp::None
            }
            O::Br => {
                let c = self.decided(i.a, i.d);
                let c = self.cond(c);
                match self.known(c) {
                    // A known decision: a jump (Cranelift's mid-end keeps a branch on a constant).
                    Some(k) => {
                        let t = if k != 0 { i.b } else { i.c };
                        self.b.ins().jump(self.blocks[t as usize], &[]);
                    }
                    None => {
                        self.b.ins().brif(c, self.blocks[i.b as usize], &[], self.blocks[i.c as usize], &[]);
                    }
                }
                Rp::None
            }
            O::Switch => {
                let xv = self.decided(i.a, i.d);
                let mut sw = cranelift_frontend::Switch::new();
                for (val, t) in self.f.labels(i) {
                    sw.set_entry(u128::from(val), self.blocks[t as usize]);
                }
                sw.emit(&mut self.b, xv, self.blocks[self.f.default_of(i) as usize]);
                Rp::None
            }
            O::Ret | O::Halt if self.chunked => {
                self.leave(NEXT_END, ACT_NONE);
                Rp::None
            }
            O::Ret => {
                let (rm, sh) = self.f.ret;
                let vs: Vec<Value> = match sh {
                    Shape::Uniform => vec![self.u(i.a)],
                    Shape::Lanes => self.lanes(i.a, rm),
                    _ => vec![],
                };
                self.b.ins().return_(&vs);
                Rp::None
            }
            O::Halt => {
                self.b.ins().return_(&[]);
                Rp::None
            }
        })
    }

    fn norm32_if(&mut self, m: M, v: Value) -> Value {
        if wide(m) || m.float() { v } else { self.norm32(m, v) }
    }
}
