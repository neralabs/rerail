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
use cranelift_codegen::ir::types::{F32, F32X4, F64, F64X2, I8, I8X16, I16, I32, I32X4, I64, I64X2};
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
    /// A chunk's (a SIMD group run as chunks of its lanes, each a fiber: a cross-lane kernel below 32 lanes): its
    /// first lane (`u32`), its SIMD group's exchange area's address (`u64`, two halves of `XHALF` bytes: the lanes'
    /// values, 8 bytes a lane, then their bits, a `u32` at `XWORD`), and the half its next exchange uses (`u32`).
    pub lane0: i32,
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
/// decision, site)`, `fail(sg, what)` (which never returns), `copy(to, from, bytes)`, `zero(to, bytes)`,
/// `xbar(sg)`.
pub const H_BAR: i32 = 0;
pub const H_YIELD: i32 = 1;
pub const H_AGREE: i32 = 2;
pub const H_FAIL: i32 = 3;
pub const H_COPY: i32 = 4;
pub const H_ZERO: i32 = 5;
/// `xbar(sg)`: a chunk waits for its SIMD group's other chunks (each at the same exchange), then goes on.
pub const H_XBAR: i32 = 6;
pub const HELPERS: usize = 7;

/// An exchange area's half: the lanes' values (8 bytes a lane), then their bits at `XWORD`.
pub const XHALF: i32 = 512;
const XWORD: i32 = 256;

/// `fail`'s reasons.
pub const FAIL_DIV: i64 = 1;
pub const FAIL_REM: i64 = 2;

/// The kernel's entry: a SIMD group's state; it runs the group's lanes, chunk by chunk.
pub type Entry = unsafe extern "C" fn(sg: *mut u8);

/// A kernel compiled.
pub struct Compiled {
    _module: cranelift_jit::JITModule,
    pub entry: Entry,
    pub width: u32,
    /// Whether its SIMD groups run as chunks of `width` lanes, a fiber each (a cross-lane kernel below 32 lanes).
    pub chunked: bool,
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
    for (k, v) in [("opt_level", "speed"), ("enable_verifier", "false"), ("enable_multi_ret_implicit_sret", "true"), ("enable_probestack", "false")] {
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
/// cross-lane operation below 32 lanes runs its SIMD groups as chunks (`Compiled::chunked`).
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
            eprintln!("PRESSURE {name}: {} -> w{w}", row.join(", "));
        }
        w
    } else {
        w
    };
    if !matches!(w, 1 | 2 | 4 | 8 | 16 | 32) {
        return Err(Error(format!("{name}: a width of {w} lanes")));
    }
    let chunked = ssa.funcs[0].cross && w < 32;
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
    let mut kept = Some(Kept::new(w | u32::from(chunked) << 16, ssa.funcs.len() + 2));
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
            if chunked && f.cross {
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
    Ok((Compiled { _module: module, entry, width: w, chunked }, kept.map(|k| k.out)))
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
const KEPT: &[u8; 8] = b"UKLCL001";

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
    let (w, chunked) = (wc & 0xffff, wc >> 16 == 1);
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
    Some(Compiled { _module: module, entry, width: w, chunked })
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
/// called if it has one; `chunked`, the one chunk the state's first lane (`lane0`) names.
fn entry(b: &mut FunctionBuilder, kernel: ir::FuncRef, full: ir::FuncRef, w: u32, chunked: bool, pl: &Places) {
    let e = b.create_block();
    b.append_block_params_for_function_params(e);
    b.switch_to_block(e);
    let sg = b.block_params(e)[0];
    let live = b.ins().uload32(mf(), sg, pl.live);
    let fp = b.ins().load(I64, mf(), sg, pl.frame);
    if chunked {
        let lane0 = b.ins().uload32(mf(), sg, pl.lane0);
        let shifted = b.ins().ushr(live, lane0);
        let bits = b.ins().band_imm(shifted, (1i64 << w) - 1);
        let body = b.create_block();
        let done = b.create_block();
        b.ins().brif(bits, body, &[], done, &[]);
        b.switch_to_block(body);
        call_kernel(b, kernel, full, bits, w, sg, fp, lane0);
        b.ins().jump(done, &[]);
        b.switch_to_block(done);
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

/// A lanes value whose lane `j` is `first + j * step` (`first` an `I64`): in its type's bits, or (`exact`) as an
/// integer, so a cast that widens it keeps it.
#[derive(Clone, Copy)]
struct Steps {
    first: Value,
    step: i64,
    exact: bool,
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
    /// Splats and 64-bit masks made in the block being made.
    splats: HashMap<(V, u32), Vec<Value>>,
    masks64: HashMap<V, Vec<Value>>,
    fail: Option<ir::Block>,
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
            splats: HashMap::new(),
            masks64: HashMap::new(),
            fail: None,
            entry_bits: dummy,
            helper_sigs: HashMap::new(),
            chunked: false,
            group_dec: Vec::new(),
            cur_block: 0,
            full: false,
            ones: std::collections::HashSet::new(),
        }
    }

    fn function(&mut self) -> R<()> {
        let f = self.f;
        let w = self.w;
        self.blocks = f.blocks.iter().map(|_| self.b.create_block()).collect();
        let e = self.blocks[0];
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
        // A SIMD group of 32 lanes at 32 is one chunk, its first lane 0.
        self.lane0 = if w == 32 { self.b.ins().iconst(I64, 0) } else { ps[2] };
        let mp = mask_parts(w);
        let entry_mask: Vec<Value> = if self.full {
            // Every lane of the chunk: as `mask_of_bits` of all its lanes' bits.
            self.ones.insert(self.entry_v);
            if w == 1 {
                vec![self.b.ins().iconst(I32, -1)]
            } else {
                let lanes: Vec<u8> = (0..4u32).flat_map(|j| if j < w { [0xffu8; 4] } else { [0u8; 4] }).collect();
                let c = self.b.func.dfg.constants.insert(lanes.into());
                let v = self.b.ins().vconst(I32X4, c);
                vec![v; mp]
            }
        } else {
            ps[3..3 + mp].to_vec()
        };
        self.env = self.b.ins().load(I64, mf(), self.sg, self.pl.env);
        self.entry_bits = self.mask_bits(&entry_mask);
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
        let order = crate::ssa::rpo(f);
        for &bk in &order {
            if bk != 0 {
                self.b.switch_to_block(self.blocks[bk]);
            }
            self.splats.clear();
            self.masks64.clear();
            self.cur_block = bk;
            let blk = &f.blocks[bk];
            for &x in &blk.insts {
                let i = *f.inst(x);
                self.b.set_srcloc(ir::SourceLoc::new(x));
                let r = match i.op {
                    O::Entry => Rp::K(entry_mask.clone()),
                    O::Arg if !self.kernel => args[i.a as usize].clone(),
                    _ => self.inst(x, &i)?,
                };
                self.vals[x as usize] = r;
            }
        }
        if let Some(fb) = self.fail {
            let _ = fb;
        }
        Ok(())
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
    // exchange area's next half, waits for the others (`xbar`), and reads the whole SIMD group's.

    /// The exchange area's next half (the chunk's next exchange uses the other).
    fn x_half(&mut self) -> Value {
        let par = self.b.ins().uload32(mf(), self.sg, self.pl.xpar);
        let next = self.b.ins().bxor_imm(par, 1);
        self.b.ins().istore32(mf(), next, self.sg, self.pl.xpar);
        let buf = self.b.ins().load(I64, mf(), self.sg, self.pl.xbuf);
        let off = self.b.ins().imul_imm(par, i64::from(XHALF));
        self.b.ins().iadd(buf, off)
    }

    fn x_wait(&mut self) {
        let sg = self.sg;
        self.helper(H_XBAR, 1, 0, &[sg]);
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
            let on8 = self.b.ins().uextend(I8, on);
            have = self.b.ins().bor(have, on8);
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

    /// A branch to the kernel's failure (`fail(sg, what)`, which does not return) if `c` is true.
    fn fail_if(&mut self, c: Value, what: i64) {
        let fail = self.b.create_block();
        let go = self.b.create_block();
        let c = self.cond(c);
        self.b.ins().brif(c, fail, &[], go, &[]);
        self.b.switch_to_block(fail);
        let wv = self.b.ins().iconst(I64, what);
        let sg = self.sg;
        self.helper(H_FAIL, 2, 0, &[sg, wv]);
        self.b.ins().trap(ir::TrapCode::user(1).expect("a trap code"));
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

    /// `a op x`'s steps, when `a` or `x` has them and the other is uniform (a sum) or a constant (a product, a shift).
    fn stepped(&mut self, op: Op, m: M, a: V, x: V) -> Option<Steps> {
        if m.float() || m.bytes() < 4 {
            return None;
        }
        let konst = |g: &Self, v: V| g.konst(v).map(|k| k as i64);
        // A 64-bit value's bits are its integer's, mod 2^64 as the addresses made of it; a 32-bit one's only while
        // nothing has added or multiplied.
        let exact = m.bytes() == 8;
        match op {
            Op::Add => {
                for (s, u) in [(a, x), (x, a)] {
                    if let (Some(st), Rp::U(u)) = (self.steps.get(&s).copied(), &self.vals[u as usize]) {
                        let u = *u;
                        let first = self.b.ins().iadd(st.first, u);
                        return Some(Steps { first, step: st.step, exact: exact && st.exact });
                    }
                }
                None
            }
            Op::Mul | Op::Shl => {
                for (s, c) in [(a, x), (x, a)] {
                    if op == Op::Shl && s != a {
                        break;
                    }
                    if let (Some(st), Some(k)) = (self.steps.get(&s).copied(), konst(self, c)) {
                        let k = if op == Op::Shl { 1i64.checked_shl(k as u32)? } else { k };
                        let first = self.b.ins().imul_imm(st.first, k);
                        return Some(Steps { first, step: st.step.wrapping_mul(k), exact: exact && st.exact });
                    }
                }
                None
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

    /// The address of lane 0 when `addr`'s lanes are `bytes` apart: its steps'.
    fn consecutive(&self, addr: V, bytes: u32) -> Option<Value> {
        let st = self.steps.get(&addr)?;
        (st.step == i64::from(bytes) && (bytes == 4 || bytes == 8)).then_some(st.first)
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

    fn store_lanes(&mut self, m: M, addr: V, x: V, mask: V) {
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
                    Rp::U(self.u_bin(op, om, a, b))
                } else {
                    if let Some(st) = self.stepped(op, om, i.a, i.b) {
                        self.steps.insert(x, st);
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
                    Rp::U(self.u_cast(from, m, a))
                } else {
                    if let Some(st) = self.steps.get(&i.a).copied() {
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
                Rp::U(a) => {
                    let a = self.guarded(a, i.b);
                    Rp::U(self.load_u(m, a, 0))
                }
                _ => Rp::L(self.load_lanes(m, i.a, i.b)),
            },
            O::Store => {
                match (self.vals[i.a as usize].clone(), self.vals[i.b as usize].clone()) {
                    (Rp::U(a), Rp::U(v)) => {
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
                    self.steps.insert(x, Steps { first, step: 1, exact: true });
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
                    if native {
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
                let pl = self.lanes(i.b, M::U32);
                let (bits, base) = if self.chunked {
                    // The SIMD group's lanes, from the exchange area.
                    let at = self.x_half();
                    self.x_put_lanes(at, &px, m);
                    self.x_put_bits(at, &k);
                    self.x_wait();
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
                    let word = self.b.ins().load(I32, mf(), at, XWORD);
                    let lead = self.x_lead(word);
                    let some = self.b.ins().icmp_imm(IntCC::NotEqual, word, 0);
                    let mine = self.b.ins().icmp(IntCC::Equal, lead, self.lane0);
                    let go = self.b.ins().band(some, mine);
                    let (does, join) = (self.b.create_block(), self.b.create_block());
                    let rt = uty(m);
                    self.b.append_block_param(join, rt);
                    let none = self.konst_u(m, 0);
                    self.b.ins().brif(go, does, &[], join, &[BlockArg::Value(none)]);
                    self.b.switch_to_block(does);
                    let p = self.u(i.a);
                    let xv = if i.b != NONE { self.u(i.b) } else { self.konst_u(m, 0) };
                    let yv = if i.c != NONE { self.u(i.c) } else { self.konst_u(m, 0) };
                    let r = self.atomic_u(bi, m, p, xv, yv);
                    self.b.ins().jump(join, &[BlockArg::Value(r)]);
                    self.b.switch_to_block(join);
                    let r = self.b.block_params(join)[0];
                    // The leader's result: each chunk's own at its place, the leader's read.
                    let at2 = self.x_half();
                    let r64 = if rt == I64 { r } else { self.b.ins().bitcast(if rt == F32 { I32 } else { I64 }, MemFlags::new(), r) };
                    let r64 = if self.b.func.dfg.value_type(r64) == I32 { self.b.ins().uextend(I64, r64) } else { r64 };
                    let off = self.b.ins().imul_imm(self.lane0, 8);
                    let mine_at = self.b.ins().iadd(at2, off);
                    self.b.ins().store(mf(), r64, mine_at, 0);
                    self.x_wait();
                    let off = self.b.ins().imul_imm(lead, 8);
                    let theirs = self.b.ins().iadd(at2, off);
                    let r = self.b.ins().load(I64, mf(), theirs, 0);
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
                self.helper(H_AGREE, 4, 0, &[sg, node, d, site]);
                if m == M::VOID { Rp::None } else { Rp::U(d) }
            }
            O::Bar => {
                let site = self.b.ins().iconst(I64, i64::from(i.a));
                let sg = self.sg;
                self.helper(H_BAR, 2, 0, &[sg, site]);
                Rp::None
            }
            // At the group's width its lanes run in step: what one wrote, the others read after it; a chunk waits for
            // the others.
            O::SgBar => {
                if self.chunked {
                    self.x_wait();
                }
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
                let mut vs = Vec::new();
                for (&a, &p) in targs.iter().zip(&params) {
                    let pi = self.f.inst(p);
                    match pi.shape {
                        Shape::Uniform => vs.push(self.u(a)),
                        Shape::Lanes => vs.extend(self.lanes(a, pi.m)),
                        Shape::Mask => vs.extend(self.mask(a)),
                        Shape::None => {}
                    }
                }
                let ba: Vec<BlockArg> = vs.into_iter().map(BlockArg::Value).collect();
                self.b.ins().jump(self.blocks[to], &ba);
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
