//! Cranelift IR as Metal's shading language (decision 0348, stage 5): the same functions the JIT
//! runs and `clif_rust` prints as Rust, printed as MSL, so the device runs the same code.
//!
//! - **Values:** each a local of its type's unsigned C++ type, declared once (as `clif_rust`'s).
//!   Arithmetic is on the type's width, its shifts' counts masked as Cranelift's are.
//! - **Memory:** an address is a GPU address in the batch's one arena, and each access is the
//!   arena's pointer at the address's offset (`AT`). An integer cast to a pointer is never
//!   accessed: the device compiler took two such pointers to one byte as different objects and
//!   read a byte back from before its store (`!(1/0)` was true; 0348, stage 5).
//! - **The context** (each function's first parameter) is a column of the batch's table: a
//!   field is its 8-byte slot's row (0408, L2), so a SIMD group's lanes read one line. The
//!   context is only ever a field's base (`context_uses`).
//! - **Alignment:** a byte, or a context field at its size's multiple, is one access. Any other
//!   access is a helper over the aligned words around it, so no access is unaligned. The IR's
//!   `aligned` flag is the CPU's, and is not used.
//! - **Control:** `clif_rust::Shape`'s structure, printed without labels (`Shape::msl`).
//! - **Calls:** each function an MSL function, called directly; the graph has no recursion
//!   (Metal's rule), which the translation checks.
//!
//! What it does not print is an error, never a fallback: vectors, stack slots, the stack
//! pointer, traps, and more than one result.

use crate::clif_rust::{Shape, absorbed, returned_call, successors};
use cranelift_codegen::ir::{self, InstructionData, Opcode, condcodes::IntCC, types};
use std::fmt::Write;

/// A type's unsigned and signed C++ names and its bits.
fn ty(t: ir::Type) -> Result<(&'static str, &'static str, u32), String> {
    Ok(match t {
        types::I8 => ("uchar", "char", 8),
        types::I16 => ("ushort", "short", 16),
        types::I32 => ("uint", "int", 32),
        types::I64 => ("ulong", "long", 64),
        t => return Err(format!("the type {t}")),
    })
}

/// The helpers every translation carries: loads and stores at any byte, by the aligned words
/// around it.
const PRELUDE: &str = "#include <metal_stdlib>\nusing namespace metal;\n\n\
// Every address the words hold is a GPU address in the batch's one arena: each access is the\n\
// arena's pointer at the address's offset (`hb` its first byte's address), never an integer\n\
// cast to a pointer, so the compiler sees one object and orders its accesses.\n\
#define AT(T, a) (reinterpret_cast<device T*>(heap + ((a) - hb)))\n\
// Every access through a type that may alias any other: the words read and write the same\n\
// bytes at several widths (a field stored as 32 bits, read in a 64-bit word), which C++\'s\n\
// type-based alias rules would let the compiler reorder.\n\
typedef ulong __attribute__((__may_alias__)) ulong_a; typedef uint __attribute__((__may_alias__)) uint_a;\n\
typedef ushort __attribute__((__may_alias__)) ushort_a; typedef uchar __attribute__((__may_alias__)) uchar_a;\n\
typedef char __attribute__((__may_alias__)) char_a; typedef short __attribute__((__may_alias__)) short_a; typedef int __attribute__((__may_alias__)) int_a;\n\
// Loads and stores at any address: the access itself where the address is its size's multiple,\n\
// else by the aligned words around it (a 64-bit value on a 4-byte boundary as two halves).\n\
static inline ulong ld_any(device uchar* heap, ulong hb, ulong a) {\n\
  const ulong w = a & ~7ul; const uint s = uint(a & 7ul) * 8u;\n\
  const ulong lo = *AT(ulong_a, w);\n\
  if (s == 0u) return lo;\n\
  const ulong hi = *AT(ulong_a, w + 8ul);\n\
  return (lo >> s) | (hi << (64u - s));\n\
}\n\
static inline ulong ld64(device uchar* heap, ulong hb, ulong a) {\n\
  if ((a & 7ul) == 0ul) return *AT(ulong_a, a);\n\
  if ((a & 3ul) == 0ul) return ulong(*AT(uint_a, a)) | (ulong(*AT(uint_a, a + 4ul)) << 32);\n\
  return ld_any(heap, hb, a);\n\
}\n\
static inline uint ld32(device uchar* heap, ulong hb, ulong a) { return (a & 3ul) == 0ul ? *AT(uint_a, a) : uint(ld_any(heap, hb, a)); }\n\
static inline ushort ld16(device uchar* heap, ulong hb, ulong a) { return (a & 1ul) == 0ul ? *AT(ushort_a, a) : ushort(ld_any(heap, hb, a)); }\n\
static inline void st_bytes(device uchar* heap, ulong hb, ulong a, ulong x, uint n) {\n\
  device uchar_a* p = AT(uchar_a, a);\n\
  for (uint i = 0u; i < n; i++) p[i] = uchar(x >> (8u * i));\n\
}\n\
static inline void st64(device uchar* heap, ulong hb, ulong a, ulong x) {\n\
  if ((a & 7ul) == 0ul) { *AT(ulong_a, a) = x; return; }\n\
  if ((a & 3ul) == 0ul) { *AT(uint_a, a) = uint(x); *AT(uint_a, a + 4ul) = uint(x >> 32); return; }\n\
  st_bytes(heap, hb, a, x, 8u);\n\
}\n\
static inline void st32(device uchar* heap, ulong hb, ulong a, uint x) { if ((a & 3ul) == 0ul) *AT(uint_a, a) = x; else st_bytes(heap, hb, a, x, 4u); }\n\
static inline void st16(device uchar* heap, ulong hb, ulong a, ushort x) { if ((a & 1ul) == 0ul) *AT(ushort_a, a) = x; else st_bytes(heap, hb, a, x, 2u); }\n\n";

/// The functions (by their user name's index), as MSL: each of `entries` called from a kernel
/// of its name over a batch, one context a thread taken by ticket (decision 0027's workers):
/// `ctxs[t]` the context's address, `results[t]` its run's result.
pub fn translate(funcs: &[(u32, ir::Function)], entries: &[(u32, &str)]) -> Result<String, String> {
    let mut out = String::from("// Generated by rerail_compiler::clif_msl from the Cranelift IR. Do not edit.\n");
    out.push_str(PRELUDE);
    // Callees before callers, as C++ needs: by the call graph, which may have no cycle.
    let calls = |f: &ir::Function| -> Vec<u32> {
        f.dfg.ext_funcs.values().filter_map(|e| match &e.name {
            ir::ExternalName::User(r) => Some(f.params.user_named_funcs()[*r].index),
            _ => None,
        }).collect()
    };
    let mut order: Vec<u32> = Vec::new();
    let mut state: std::collections::HashMap<u32, u8> = Default::default();
    fn visit(id: u32, funcs: &[(u32, ir::Function)], calls: &dyn Fn(&ir::Function) -> Vec<u32>, state: &mut std::collections::HashMap<u32, u8>, order: &mut Vec<u32>) -> Result<(), String> {
        match state.get(&id) {
            Some(2) => return Ok(()),
            Some(_) => return Err(format!("u0:{id} is called in a cycle: Metal has no recursion")),
            None => {}
        }
        state.insert(id, 1);
        let f = &funcs.iter().find(|(i, _)| *i == id).ok_or_else(|| format!("no function u0:{id}"))?.1;
        for c in calls(f) {
            visit(c, funcs, calls, state, order)?;
        }
        state.insert(id, 2);
        order.push(id);
        Ok(())
    }
    for (id, _) in funcs {
        visit(*id, funcs, &calls, &mut state, &mut order)?;
    }
    for id in order {
        let f = &funcs.iter().find(|(i, _)| *i == id).expect("listed").1;
        function(&mut out, id, f).map_err(|e| format!("u0:{id}: {e}"))?;
    }
    for &(entry, n) in entries {
        let f = &funcs.iter().find(|(id, _)| *id == entry).ok_or("no entry")?.1;
        if f.signature.params.len() != 1 || f.signature.returns.len() != 1 {
            return Err(format!("the entry {n}: one parameter and one result"));
        }
        writeln!(
            out,
            "// The entry over a batch: each worker takes contexts by ticket until none is left.\n\
             kernel void {n}(device ulong* ctxs [[buffer(0)]], device ulong* results [[buffer(1)]], device atomic_uint* ticket [[buffer(2)]], constant uint& count [[buffer(3)]], device uchar* heap [[buffer(4)]], constant ulong& hb [[buffer(5)]], constant ulong& cs [[buffer(6)]]) {{\n\
             \x20 for (uint k = 0u; k < count; k++) {{\n\
             \x20   const uint t = atomic_fetch_add_explicit(ticket, 1u, memory_order_relaxed);\n\
             \x20   if (t >= count) return;\n\
             \x20   results[t] = f{entry}(0ul, heap, hb, cs, ctxs[t]);\n\
             \x20 }}\n\
             }}\n"
        )
        .unwrap();
    }
    Ok(out)
}

/// One function.
fn function(out: &mut String, id: u32, f: &ir::Function) -> Result<(), String> {
    let dfg = &f.dfg;
    if !f.sized_stack_slots.is_empty() {
        return Err("a stack slot".into());
    }
    let ret_ty = match f.signature.returns.len() {
        0 => "void",
        1 => ty(f.signature.returns[0].value_type)?.0,
        _ => return Err("more than one result".into()),
    };
    let fields = context_fields(f);
    context_uses(f, &fields)?;
    let name = |v: ir::Value| format!("v{}", dfg.resolve_aliases(v).as_u32());
    let entry = f.layout.entry_block().ok_or("no entry block")?;
    let params: Vec<String> = dfg.block_params(entry).iter().map(|&v| Ok(format!("{} a{}", ty(dfg.value_type(v))?.0, v.as_u32()))).collect::<Result<_, String>>()?;
    writeln!(out, "static {ret_ty} f{id}(ulong pin, device uchar* heap, ulong hb, ulong cs{}{}) {{", if params.is_empty() { "" } else { ", " }, params.join(", ")).unwrap();
    let absorbed = absorbed(f);
    if !absorbed.is_empty() {
        return Err("a vector test".into());
    }
    let mut declared = std::collections::BTreeMap::new();
    for b in f.layout.blocks() {
        for &v in dfg.block_params(b) {
            declared.insert(v.as_u32(), ty(dfg.value_type(v))?.0);
        }
        for inst in f.layout.block_insts(b) {
            for &v in dfg.inst_results(inst) {
                declared.insert(v.as_u32(), ty(dfg.value_type(v))?.0);
            }
        }
    }
    for (v, t) in &declared {
        writeln!(out, "  {t} v{v} = 0;").unwrap();
    }
    for &v in dfg.block_params(entry) {
        writeln!(out, "  v{0} = a{0};", v.as_u32()).unwrap();
    }
    out.push_str("  uint d = 0u, j = 0u;\n");
    let blocks: Vec<ir::Block> = std::iter::once(entry).chain(f.layout.blocks().filter(|&b| b != entry)).collect();
    let node_of: std::collections::HashMap<ir::Block, usize> = blocks.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let first: Vec<Vec<usize>> = blocks.iter().map(|&b| successors(f, b).iter().map(|t| node_of[t]).collect()).collect();
    let sizes: Vec<usize> = blocks.iter().map(|&b| f.layout.block_insts(b).count()).collect();
    match Shape::of(first, &sizes) {
        Some(mut shape) => {
            shape.msl = true;
            let emit = |node: usize, jump: &dyn Fn(usize) -> Result<String, String>| -> Result<String, String> {
                let mut s = String::new();
                for inst in f.layout.block_insts(blocks[node]) {
                    s.push_str(&instruction(f, inst, &fields, &name, &|k, _| jump(k))?);
                    s.push('\n');
                }
                Ok(s)
            };
            // Inside one construct, so every exit's `break` has one to leave.
            let body = shape.tree(&emit, 0)?;
            writeln!(out, "do {{\n{body}}} while (false);\n  return {};\n}}\n", if ret_ty == "void" { "" } else { "0" }).unwrap();
        }
        None => {
            // An irreducible graph: a loop over a `switch` on the block.
            writeln!(out, "  uint b = {}u;\n  for (;;) {{\n  switch (b) {{", entry.as_u32()).unwrap();
            for blk in f.layout.blocks() {
                writeln!(out, "  case {}u: {{", blk.as_u32()).unwrap();
                for inst in f.layout.block_insts(blk) {
                    out.push_str(&instruction(f, inst, &fields, &name, &|_, to: ir::Block| Ok(format!("b = {}u; continue;", to.as_u32())))?);
                    out.push('\n');
                }
                out.push_str("  } break;\n");
            }
            writeln!(out, "  default: return {};\n  }}\n  }}\n}}\n", if ret_ty == "void" { "" } else { "0" }).unwrap();
        }
    }
    Ok(())
}

/// A function's context: its first parameter.
fn context(f: &ir::Function) -> Option<ir::Value> {
    f.layout.entry_block().and_then(|e| f.dfg.block_params(e).first().copied())
}

/// The values that are the context plus a constant (the context itself, and `iadd_imm`s of
/// them), each with its constant.
type Fields = std::collections::HashMap<ir::Value, i64>;

fn context_fields(f: &ir::Function) -> Fields {
    let dfg = &f.dfg;
    let mut fields = Fields::new();
    let Some(c) = context(f) else { return fields };
    fields.insert(c, 0);
    loop {
        let before = fields.len();
        for b in f.layout.blocks() {
            for inst in f.layout.block_insts(b) {
                if let InstructionData::BinaryImm64 { opcode: Opcode::IaddImm, arg, imm } = dfg.insts[inst]
                    && let Some(&o) = fields.get(&dfg.resolve_aliases(arg))
                {
                    fields.insert(dfg.first_result(inst), o + imm.bits());
                }
            }
        }
        if fields.len() == before {
            return fields;
        }
    }
}

/// The context is used only as a load's or a store's base, at a field within one 8-byte slot,
/// as a callee's context (its first argument), or plus a constant (`context_fields`) for the
/// same uses: the device's table holds a context by slots, so any other use, an address made
/// from it any other way or the context as a value, is refused.
fn context_uses(f: &ir::Function, fields: &Fields) -> Result<(), String> {
    let dfg = &f.dfg;
    for b in f.layout.blocks() {
        for inst in f.layout.block_insts(b) {
            let data = &dfg.insts[inst];
            for (i, &v) in dfg.inst_args(inst).iter().enumerate() {
                let Some(&at) = fields.get(&dfg.resolve_aliases(v)) else { continue };
                let field = |n: u32| data.load_store_offset().is_some_and(|o| {
                    let o = at + i64::from(o);
                    o >= 0 && o % i64::from(n) == 0 && o % 8 + i64::from(n) <= 8
                });
                let ok = match data.opcode() {
                    Opcode::Uload8 | Opcode::Sload8 if i == 0 => field(1),
                    Opcode::Uload16 | Opcode::Sload16 if i == 0 => field(2),
                    Opcode::Uload32 | Opcode::Sload32 if i == 0 => field(4),
                    Opcode::Load if i == 0 => dfg.inst_results(inst).first().is_some_and(|&r| field(dfg.value_type(r).bytes())),
                    Opcode::Store | Opcode::Istore8 | Opcode::Istore16 | Opcode::Istore32 if i == 1 => {
                        let n = match data.opcode() {
                            Opcode::Istore8 => 1,
                            Opcode::Istore16 => 2,
                            Opcode::Istore32 => 4,
                            _ => dfg.value_type(dfg.inst_args(inst)[0]).bytes(),
                        };
                        field(n)
                    }
                    Opcode::Call => i == 0 && at == 0,
                    Opcode::IaddImm => fields.contains_key(&dfg.first_result(inst)),
                    _ => false,
                };
                if !ok {
                    return Err(format!("the context used other than as a field's base: {}", dfg.display_inst(inst)));
                }
            }
        }
    }
    Ok(())
}

/// A branch to `call`: its arguments into its block's parameters (all read first), then on.
fn goto(f: &ir::Function, call: &ir::BlockCall, k: usize, name: &dyn Fn(ir::Value) -> String, jump: &dyn Fn(usize, ir::Block) -> Result<String, String>) -> Result<String, String> {
    let dfg = &f.dfg;
    let to = call.block(&dfg.value_lists);
    let args: Vec<String> = call
        .args(&dfg.value_lists)
        .map(|a| match a {
            ir::BlockArg::Value(v) => Ok(name(v)),
            _ => Err("a block argument that is not a value".to_string()),
        })
        .collect::<Result<_, _>>()?;
    let params = dfg.block_params(to);
    let mut s = String::from("{ ");
    if params.len() == 1 {
        write!(s, "v{} = {}; ", params[0].as_u32(), args[0]).unwrap();
    } else if !params.is_empty() {
        for (i, a) in args.iter().enumerate() {
            write!(s, "const auto t{i} = {a}; ").unwrap();
        }
        for (i, p) in params.iter().enumerate() {
            write!(s, "v{} = t{i}; ", p.as_u32()).unwrap();
        }
    }
    s.push_str(&jump(k, to)?);
    s.push_str(" }");
    Ok(s)
}

/// A 64-bit constant as C++, of type `u`.
fn konst(c: i64, u: &str) -> String {
    format!("{u}({:#x}ul)", c as u64)
}

fn instruction(f: &ir::Function, inst: ir::Inst, fields: &Fields, name: &dyn Fn(ir::Value) -> String, jump: &dyn Fn(usize, ir::Block) -> Result<String, String>) -> Result<String, String> {
    let dfg = &f.dfg;
    let data = &dfg.insts[inst];
    let op = data.opcode();
    let args: Vec<String> = dfg.inst_args(inst).iter().map(|&v| name(v)).collect();
    let results = dfg.inst_results(inst);
    let r = results.first().map(|&v| name(v));
    let rt = results.first().map(|&v| dfg.value_type(v));
    let at = dfg.inst_args(inst).first().map(|&v| dfg.value_type(v));
    if rt.is_some_and(|t| t.is_vector()) || at.is_some_and(|t| t.is_vector()) {
        return Err(format!("{op} on vectors"));
    }
    let set = |e: String| -> String { format!("{} = {e};", r.clone().expect("a result")) };
    let (u, s, bits) = rt.and_then(|t| ty(t).ok()).unwrap_or(("ulong", "long", 64));
    // The width arithmetic is done at: no promotion to `int` below 32 bits.
    let w = if bits == 64 { "ulong" } else { "uint" };
    let ws = if bits == 64 { "long" } else { "int" };
    let bin = |o: &str| set(format!("{u}({w}({}) {o} {w}({}))", args[0], args[1]));
    let imm = || -> Result<i64, String> {
        match data {
            InstructionData::UnaryImm { imm, .. } | InstructionData::BinaryImm64 { imm, .. } | InstructionData::IntCompareImm { imm, .. } => Ok(imm.bits()),
            _ => Err(format!("{op}: no immediate")),
        }
    };
    let bimm = |o: &str| -> Result<String, String> { Ok(set(format!("{u}({w}({}) {o} {w}({}))", args[0], konst(imm()?, u)))) };
    let addr = |p: &str| -> String {
        let off = data.load_store_offset().unwrap_or(0);
        if off == 0 { format!("ulong({p})") } else { format!("(ulong({p}) + {})", konst(i64::from(off), "ulong")) }
    };
    // A field of the context (the function's first parameter, or it plus a constant): its
    // 8-byte slot's row of the batch's table (0408, L2), `cs` bytes a row, from the context's
    // column; `context_uses` checked it lies in one slot.
    let ctx = |base: ir::Value| fields.contains_key(&dfg.resolve_aliases(base));
    let slot = |_: &str| {
        let base = dfg.inst_args(inst)[if data.opcode().can_store() { 1 } else { 0 }];
        let off = fields[&dfg.resolve_aliases(base)] + i64::from(data.load_store_offset().unwrap_or(0));
        format!("(ulong({}) + {}ul * cs + {}ul)", name(context(f).expect("a context")), off / 8, off % 8)
    };
    // Any other access is one where it is a byte; else the aligned-word helpers, because the
    // IR's `aligned` flag is the CPU's, where a 64-bit access at a 4-byte boundary is one access.
    let load = |t: &str, n: u32| -> String {
        let base = dfg.inst_args(inst)[0];
        if ctx(base) {
            return format!("*AT({t}_a, {})", slot(&args[0]));
        }
        if n == 1 {
            return format!("*AT({t}_a, {})", addr(&args[0]));
        }
        format!("{t}(ld{}(heap, hb, {}))", n * 8, addr(&args[0]))
    };
    let store = |t: &str, n: u32, x: &str| -> String {
        let base = dfg.inst_args(inst)[1];
        if ctx(base) {
            return format!("*AT({t}_a, {}) = {t}({x});", slot(&args[1]));
        }
        if n == 1 {
            return format!("*AT({t}_a, {}) = {t}({x});", addr(&args[1]));
        }
        let wide = match n { 2 => "ushort", 4 => "uint", _ => "ulong" };
        format!("st{}(heap, hb, {}, {wide}({x}));", n * 8, addr(&args[1]))
    };
    let shift = |x: &str| format!("(uint({x}) & {}u)", bits - 1);
    Ok(match op {
        Opcode::Iconst => set(konst(imm()?, u)),
        Opcode::Iadd => bin("+"),
        Opcode::Isub => bin("-"),
        Opcode::Imul => bin("*"),
        Opcode::Ineg => set(format!("{u}(-{w}({}))", args[0])),
        Opcode::Udiv => bin("/"),
        Opcode::Urem => bin("%"),
        Opcode::Sdiv => set(format!("{u}({ws}({s}({})) / {ws}({s}({})))", args[0], args[1])),
        Opcode::Srem => set(format!("{u}({ws}({s}({})) % {ws}({s}({})))", args[0], args[1])),
        Opcode::Umulhi if bits == 64 => set(format!("mulhi({}, {})", args[0], args[1])),
        Opcode::Umulhi => set(format!("{u}((ulong({}) * ulong({})) >> {bits})", args[0], args[1])),
        Opcode::Band => bin("&"),
        Opcode::Bor => bin("|"),
        Opcode::Bxor => bin("^"),
        Opcode::BandNot => set(format!("{u}({w}({}) & ~{w}({}))", args[0], args[1])),
        Opcode::BorNot => set(format!("{u}({w}({}) | ~{w}({}))", args[0], args[1])),
        Opcode::BxorNot => set(format!("{u}({w}({}) ^ ~{w}({}))", args[0], args[1])),
        Opcode::Bnot => set(format!("{u}(~{w}({}))", args[0])),
        Opcode::Bitrev => set(format!("reverse_bits({})", args[0])),
        Opcode::IaddImm => bimm("+")?,
        Opcode::ImulImm => bimm("*")?,
        Opcode::UdivImm => bimm("/")?,
        Opcode::BandImm => bimm("&")?,
        Opcode::BorImm => bimm("|")?,
        Opcode::BxorImm => bimm("^")?,
        Opcode::IrsubImm => set(format!("{u}({w}({}) - {w}({}))", konst(imm()?, u), args[0])),
        Opcode::Ishl => set(format!("{u}({w}({}) << {})", args[0], shift(&args[1]))),
        Opcode::Ushr => set(format!("{u}({w}({}) >> {})", args[0], shift(&args[1]))),
        Opcode::Sshr => set(format!("{u}({ws}({s}({})) >> {})", args[0], shift(&args[1]))),
        Opcode::IshlImm => set(format!("{u}({w}({}) << {}u)", args[0], (imm()? as u32) % bits)),
        Opcode::UshrImm => set(format!("{u}({w}({}) >> {}u)", args[0], (imm()? as u32) % bits)),
        Opcode::SshrImm => set(format!("{u}({ws}({s}({})) >> {}u)", args[0], (imm()? as u32) % bits)),
        Opcode::RotlImm => set(format!("rotate({}, {u}({}))", args[0], (imm()? as u32) % bits)),
        Opcode::Popcnt => set(format!("{u}(popcount({}))", args[0])),
        Opcode::Clz => set(format!("{u}(clz({}))", args[0])),
        Opcode::Ctz => set(format!("{u}(ctz({}))", args[0])),
        Opcode::Umin => set(format!("min({}, {})", args[0], args[1])),
        Opcode::Umax => set(format!("max({}, {})", args[0], args[1])),
        Opcode::Smin => set(format!("{u}(min({s}({}), {s}({})))", args[0], args[1])),
        Opcode::Smax => set(format!("{u}(max({s}({}), {s}({})))", args[0], args[1])),
        Opcode::UaddOverflow => format!("{0} = {2}({3} + {4}); {1} = uchar({0} < {3});", name(results[0]), name(results[1]), u, args[0], args[1]),
        Opcode::Icmp | Opcode::IcmpImm => {
            let cc = data.cond_code().ok_or("a compare with no condition")?;
            let (au, asg, _) = ty(dfg.value_type(dfg.inst_args(inst)[0]))?;
            let b = if op == Opcode::IcmpImm { konst(imm()?, au) } else { args[1].clone() };
            let (sig, o) = match cc {
                IntCC::Equal => (false, "=="),
                IntCC::NotEqual => (false, "!="),
                IntCC::SignedLessThan => (true, "<"),
                IntCC::SignedGreaterThanOrEqual => (true, ">="),
                IntCC::SignedGreaterThan => (true, ">"),
                IntCC::SignedLessThanOrEqual => (true, "<="),
                IntCC::UnsignedLessThan => (false, "<"),
                IntCC::UnsignedGreaterThanOrEqual => (false, ">="),
                IntCC::UnsignedGreaterThan => (false, ">"),
                IntCC::UnsignedLessThanOrEqual => (false, "<="),
            };
            let t = if sig { asg } else { au };
            set(format!("uchar({t}({}) {o} {t}({b}))", args[0]))
        }
        Opcode::Select => set(format!("{} != 0 ? {} : {}", args[0], args[1], args[2])),
        Opcode::Uextend | Opcode::Ireduce | Opcode::Bitcast => set(format!("{u}({})", args[0])),
        Opcode::Sextend => {
            let from = ty(dfg.value_type(dfg.inst_args(inst)[0]))?;
            set(format!("{u}({s}({}({})))", from.1, args[0]))
        }
        Opcode::Load => set(load(u, bits / 8)),
        Opcode::Uload8 => set(format!("{u}({})", load("uchar", 1))),
        Opcode::Uload16 => set(format!("{u}({})", load("ushort", 2))),
        Opcode::Uload32 => set(format!("{u}({})", load("uint", 4))),
        Opcode::Sload8 => set(format!("{u}({s}({}))", load("char", 1))),
        Opcode::Sload16 => set(format!("{u}({s}({}))", load("short", 2))),
        Opcode::Sload32 => set(format!("{u}({s}({}))", load("int", 4))),
        Opcode::Store => {
            let (vt, _, vb) = ty(dfg.value_type(dfg.inst_args(inst)[0]))?;
            store(vt, vb / 8, &args[0])
        }
        Opcode::Istore8 => store("uchar", 1, &args[0]),
        Opcode::Istore16 => store("ushort", 2, &args[0]),
        Opcode::Istore32 => store("uint", 4, &args[0]),
        Opcode::GetPinnedReg => set("pin".to_string()),
        Opcode::SetPinnedReg => format!("pin = {};", args[0]),
        Opcode::Nop => String::new(),
        Opcode::Call => {
            let InstructionData::Call { func_ref, .. } = data else { return Err("a call of no function".into()) };
            let callee = match &dfg.ext_funcs[*func_ref].name {
                ir::ExternalName::User(r) => f.params.user_named_funcs()[*r].index,
                n => return Err(format!("a call of {n:?}")),
            };
            let call = format!("f{callee}(pin, heap, hb, cs{}{})", if args.is_empty() { "" } else { ", " }, args.join(", "));
            if returned_call(f, inst) {
                return Ok(format!("return {call};"));
            }
            match results.len() {
                0 => format!("{call};"),
                1 => set(call),
                _ => return Err("a call of more than one result".into()),
            }
        }
        Opcode::Return if f.layout.prev_inst(inst).is_some_and(|c| returned_call(f, c)) => String::new(),
        Opcode::Return => match args.len() {
            0 => "return;".to_string(),
            1 => format!("return {};", args[0]),
            _ => return Err("a return of more than one value".into()),
        },
        Opcode::Jump => {
            let InstructionData::Jump { destination, .. } = data else { return Err("a jump with no destination".into()) };
            goto(f, destination, 0, name, jump)?
        }
        Opcode::Brif => {
            let InstructionData::Brif { blocks, .. } = data else { return Err("a branch with no blocks".into()) };
            format!("if ({} != 0) {} else {}", args[0], goto(f, &blocks[0], 0, name, jump)?, goto(f, &blocks[1], 1, name, jump)?)
        }
        // A jump table: a `switch`, its cases the table's (successors 1 on), the rest the
        // default (successor 0). A jump in a case breaks the `switch`, then the construct around.
        Opcode::BrTable => {
            let InstructionData::BranchTable { table, .. } = data else { return Err("a jump table of nothing".into()) };
            let jt = &f.dfg.jump_tables[*table];
            let default = goto(f, &jt.default_block(), 0, name, jump)?;
            let mut arms: Vec<(String, Vec<usize>)> = Vec::new();
            for (i, call) in jt.as_slice().iter().enumerate() {
                let to = goto(f, call, i + 1, name, jump)?;
                if to == default {
                    continue;
                }
                match arms.iter_mut().find(|a| a.0 == to) {
                    Some(a) => a.1.push(i),
                    None => arms.push((to, vec![i])),
                }
            }
            let mut sw = format!("switch ({}) {{ ", args[0]);
            for (to, keys) in &arms {
                for k in keys {
                    write!(sw, "case {k}: ").unwrap();
                }
                write!(sw, "{to} break; ").unwrap();
            }
            write!(sw, "default: {default} break; }}").unwrap();
            sw
        }
        op => return Err(format!("the opcode {op}")),
    })
}

#[cfg(test)]
mod tests {
    use cranelift_codegen::ir::{AbiParam, Function, InstBuilder, MemFlags, Signature, UserFuncName, types::I64};
    use cranelift_codegen::isa::CallConv;
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

    /// A function of the context: `body` given the builder and the context, returning a value.
    fn of_context(body: impl FnOnce(&mut FunctionBuilder<'_>, cranelift_codegen::ir::Value) -> cranelift_codegen::ir::Value) -> Function {
        let mut sig = Signature::new(CallConv::Tail);
        sig.params.push(AbiParam::new(I64));
        sig.returns.push(AbiParam::new(I64));
        let mut f = Function::with_name_signature(UserFuncName::user(0, 0), sig);
        let mut ctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut f, &mut ctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let c = b.block_params(entry)[0];
        let r = body(&mut b, c);
        b.ins().return_(&[r]);
        b.seal_all_blocks();
        b.finalize();
        f
    }

    /// A context field, directly or through the context plus a constant, is its slot's row of
    /// the batch's table (0408, L2): 8 bytes on and 4 bytes into it is row 1 and byte 4.
    #[test]
    fn a_context_field_is_its_slots_row() {
        let f = of_context(|b, c| {
            let at = b.ins().iadd_imm(c, 8);
            let x = b.ins().uload32(MemFlags::trusted(), at, 4);
            let y = b.ins().load(I64, MemFlags::trusted(), c, 16);
            b.ins().iadd(x, y)
        });
        let msl = super::translate(&[(0, f)], &[(0, "k")]).unwrap();
        assert!(msl.contains("(ulong(v0) + 1ul * cs + 4ul)"), "{msl}");
        assert!(msl.contains("(ulong(v0) + 2ul * cs + 0ul)"), "{msl}");
    }

    /// Any other use of the context is refused: an address made from it by a value, the context
    /// as a value, or a field across two slots.
    #[test]
    fn the_context_as_a_value_is_refused() {
        let made = of_context(|b, c| {
            let at = b.ins().iadd(c, c);
            b.ins().load(I64, MemFlags::trusted(), at, 0)
        });
        let returned = of_context(|_, c| c);
        let across = of_context(|b, c| b.ins().load(I64, MemFlags::trusted(), c, 4));
        for f in [made, returned, across] {
            let e = super::translate(&[(0, f)], &[(0, "k")]).unwrap_err();
            assert!(e.contains("the context used other than as a field's base"), "{e}");
        }
    }
}
