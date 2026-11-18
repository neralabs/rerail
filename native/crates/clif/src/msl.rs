//! CLIF as Metal Shading Language (decision 0310's third backend): the tape's reads for the kernels, in place of
//! `metal_tape.metal`'s hand port.
//!
//! A read's function takes the tape (`device const uchar* t`, where the view's offsets count from: the
//! document's first byte in its batch's block) and its view as u32s (`CLIF_VIEW_SPACE const uint* v`: `thread` unless a kernel source defines it `device`, as the
//! join kernels do, which read each document's view where the batch's directory holds it; the device's
//! `DocView`), then its own parameters. In the body the tape and the view are byte offsets from those pointers, 0 where they come in, so
//! an address the read computes is an offset: a load of the view (marked aligned: its fields) reads `v`, a load of
//! the tape reads `t`, at any alignment (two aligned words shifted together where it is not aligned to its size;
//! a slab lays the document's source after its tape, so the second word lies in it).
//!
//! A relation's read (decision 0325) takes a base (`device const uchar* t`, a block's first byte, or any pointer
//! before the descriptor in its buffer) and the descriptor's offset from it (`ulong a0`), then its own parameters:
//! every address it computes is an offset from `t`, through the block's self-relative references, so an address a
//! read returns (`rows_at`) and a descriptor a read calls another of are offsets from the same `t`. Its loads are
//! aligned, and read as their type.
//!
//! MSL has no `goto` and no labelled `break`: a body is a loop over a `switch` on its block, which the Metal
//! compiler makes branches again. Each value is a local, set where a branch passes it to a block.
//!
//! A GPU computes in 32 bits; a 64-bit add or shift is several instructions. Every offset a read computes from the
//! tape or the view is below 4 GiB on the device (a document's parts are offsets from its first byte, u32 in its
//! view), so a 64-bit value whose every use reads only its low 32 bits (`low_words`: an address, a modular step to
//! another such value) is computed as a `uint`, with the same low bits, and a 64-bit load of it reads 32 bits.

use cranelift_codegen::ir::{self, InstructionData, Opcode, condcodes::IntCC, types};
use crate::reads::Family;
use std::fmt::Write;

/// MSL's unsigned type for a Cranelift integer type, and its signed one.
fn ty(t: ir::Type) -> Result<(&'static str, &'static str), String> {
    Ok(match t {
        types::I8 => ("uchar", "char"),
        types::I16 => ("ushort", "short"),
        types::I32 => ("uint", "int"),
        types::I64 => ("ulong", "long"),
        t => return Err(format!("a value of type {t}")),
    })
}

/// The loads every read's body uses: of the tape at any alignment, and of the view (`clif_view`, which a
/// kernel source may define before the reads, as the label passes' does to make the format a constant). Guarded,
/// so a kernel may include the reads of several families.
pub const HELPERS: &str = "\
#ifndef CLIF_VIEW_SPACE
#define CLIF_VIEW_SPACE thread
#endif
#ifndef CLIF_HELPERS
#define CLIF_HELPERS 1
#ifndef clif_view
#define clif_view(v,a) (v)[(a)/4u]
#endif
inline ulong clif_word(device const uchar* t,ulong a){device const ulong* w=(device const ulong*)(t+(a&~7ul));uint s=uint(a&7ul)*8u;return s==0u?w[0]:(w[0]>>s)|(w[1]<<(64u-s));}
inline uchar clif_u8(device const uchar* t,ulong a){return t[a];}
inline ushort clif_u16(device const uchar* t,ulong a){return (a&1ul)==0ul?*(device const ushort*)(t+a):ushort(clif_word(t,a));}
inline uint clif_u32(device const uchar* t,ulong a){return (a&3ul)==0ul?*(device const uint*)(t+a):uint(clif_word(t,a));}
inline ulong clif_u64(device const uchar* t,ulong a){return clif_word(t,a);}
inline ulong clif_word(device const uchar* t,uint a){device const ulong* w=(device const ulong*)(t+(a&~7u));uint s=(a&7u)*8u;return s==0u?w[0]:(w[0]>>s)|(w[1]<<(64u-s));}
inline uint clif_lo(device const uchar* t,uint a){return (a&3u)==0u?*(device const uint*)(t+a):uint(clif_word(t,a));}
inline uchar clif_u8(device const uchar* t,uint a){return t[a];}
inline ushort clif_u16(device const uchar* t,uint a){return (a&1u)==0u?*(device const ushort*)(t+a):ushort(clif_word(t,a));}
inline uint clif_u32(device const uchar* t,uint a){return clif_lo(t,a);}
inline ulong clif_u64(device const uchar* t,uint a){return clif_word(t,a);}
#endif
";

/// The reads `funcs` of `family` (by their user name's index, each with its name) as MSL functions, `prefix` before
/// each name: declarations first, so a read may call one defined after it.
pub fn translate_reads(funcs: &[(u32, ir::Function, &str)], prefix: &str, family: Family) -> Result<String, String> {
    // A read of another family (a view read's calls of the tape's and the numbering's) is named by its place in
    // `READS`, defined in its own family's source.
    let named = |id: u32| {
        funcs.iter().find(|(f, ..)| *f == id).map(|(.., n)| format!("{prefix}{n}")).or_else(|| crate::reads::READS.get(id as usize).filter(|read| read.family != family).map(|read| format!("{prefix}{}", read.name))).ok_or(format!("a call of u0:{id}, no read"))
    };
    let mut out = String::from(HELPERS);
    let mut bodies = String::new();
    for (id, f, n) in funcs {
        // Each read in its own family (a view read's source holds the lines' reads it calls too).
        let family = crate::reads::READS.get(*id as usize).map_or(family, |read| read.family);
        let signature = signature(f, &format!("{prefix}{n}"), family)?;
        writeln!(out, "{signature};").unwrap();
        writeln!(bodies, "{signature}{{").unwrap();
        body(&mut bodies, f, &named, family).map_err(|e| format!("{n} (u0:{id}): {e}"))?;
        bodies.push_str("}\n");
    }
    out.push_str(&bodies);
    Ok(out)
}

/// The pointers a read of `family` takes, as MSL's parameters and as a call passes them on.
fn pointers(family: Family) -> (&'static str, &'static str) {
    match family {
        Family::Tape => ("device const uchar* t,CLIF_VIEW_SPACE const uint* v", "t,v"),
        Family::Relation | Family::Lines => ("device const uchar* t,ulong a0", "t"),
        Family::View => ("device const uchar* t,ulong a0,device const uchar* tt,CLIF_VIEW_SPACE const uint* v,device const uchar* ll", "t"),
    }
}

/// The 64-bit values of `f` of which every use reads only the low 32 bits (a demanded-bits pass to its fixpoint):
/// a load's address, the operand of an add, subtract, multiply, bitwise operation or left shift whose result is one,
/// a shift's amount, a reduction's operand, a select's arm that is one, a branch's argument to a block parameter that
/// is one, a call's tape and view (passed on as pointers). A comparison, a right shift, a division, a minimum, a
/// call's own argument or a return reads the whole value. The tape's reads only: a relation's (decision 0325)
/// compute every value whole.
fn low_words(f: &ir::Function, family: Family) -> std::collections::HashSet<ir::Value> {
    let dfg = &f.dfg;
    let mut low: std::collections::HashSet<ir::Value> = std::collections::HashSet::new();
    if family != Family::Tape {
        return low;
    }
    for b in f.layout.blocks() {
        for &v in dfg.block_params(b).iter().chain(f.layout.block_insts(b).flat_map(|i| dfg.inst_results(i))) {
            if dfg.value_type(v) == types::I64 {
                low.insert(v);
            }
        }
    }
    loop {
        let mut wide: Vec<ir::Value> = Vec::new();
        for b in f.layout.blocks() {
            for inst in f.layout.block_insts(b) {
                let data = &dfg.insts[inst];
                let args = dfg.inst_args(inst);
                let result_low = dfg.inst_results(inst).first().is_some_and(|r| low.contains(r));
                // Whether the use of argument `k` reads only its low 32 bits.
                let reads_low = |k: usize| -> bool {
                    match data.opcode() {
                        Opcode::Iadd | Opcode::Isub | Opcode::Imul | Opcode::Band | Opcode::Bor | Opcode::Bxor | Opcode::Bnot => result_low,
                        Opcode::IaddImm | Opcode::ImulImm | Opcode::IrsubImm | Opcode::BandImm | Opcode::BorImm | Opcode::BxorImm | Opcode::IshlImm => result_low,
                        Opcode::Ishl => k == 1 || result_low,
                        Opcode::Ushr | Opcode::Sshr => k == 1,
                        Opcode::Ireduce => true,
                        Opcode::Load | Opcode::Uload8 | Opcode::Uload16 | Opcode::Uload32 => k == 0,
                        Opcode::Select => k != 0 && result_low,
                        // A call passes the tape and the view on as `t` and `v`, not their offsets (`instruction`).
                        Opcode::Call => k < 2,
                        _ => false,
                    }
                };
                for (k, a) in args.iter().enumerate() {
                    let a = dfg.resolve_aliases(*a);
                    if low.contains(&a) && !reads_low(k) {
                        wide.push(a);
                    }
                }
                // A branch's arguments: low where the parameter they set is.
                for call in data.branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
                    let block = call.block(&dfg.value_lists);
                    for (p, a) in dfg.block_params(block).iter().zip(call.args(&dfg.value_lists)) {
                        if let ir::BlockArg::Value(a) = a {
                            let a = dfg.resolve_aliases(a);
                            if low.contains(&a) && !low.contains(p) {
                                wide.push(a);
                            }
                        }
                    }
                }
            }
        }
        if wide.is_empty() {
            return low;
        }
        for v in wide {
            low.remove(&v);
        }
    }
}

fn signature(f: &ir::Function, name: &str, family: Family) -> Result<String, String> {
    let [ret] = f.signature.returns.as_slice() else { return Err(format!("{name}: a read returns one value")) };
    let params: Vec<String> =
        f.signature.params.iter().enumerate().skip(family.pointers()).map(|(i, p)| Ok(format!("{} a{i}", ty(p.value_type)?.0))).collect::<Result<_, String>>()?;
    let more = if params.is_empty() { String::new() } else { format!(",{}", params.join(",")) };
    Ok(format!("inline {} {name}({}{more})", ty(ret.value_type)?.0, pointers(family).0))
}

fn body(out: &mut String, f: &ir::Function, named: &dyn Fn(u32) -> Result<String, String>, family: Family) -> Result<(), String> {
    let dfg = &f.dfg;
    let name = |v: ir::Value| format!("v{}", dfg.resolve_aliases(v).as_u32());
    let entry = f.layout.entry_block().ok_or("no entry block")?;
    let low = low_words(f, family);
    // A value set and read in one block only is declared where it is set, in that block's case: its life ends there,
    // which a local of the whole body (live round the loop, as far as the compiler sees) does not show. Block
    // parameters and values read in another block are the body's.
    let mut read_in: std::collections::HashMap<ir::Value, std::collections::HashSet<ir::Block>> = std::collections::HashMap::new();
    for b in f.layout.blocks() {
        for inst in f.layout.block_insts(b) {
            for &a in dfg.inst_args(inst) {
                read_in.entry(dfg.resolve_aliases(a)).or_default().insert(b);
            }
            for call in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
                for a in call.args(&dfg.value_lists) {
                    if let ir::BlockArg::Value(a) = a {
                        read_in.entry(dfg.resolve_aliases(a)).or_default().insert(b);
                    }
                }
            }
        }
    }
    let mut scoped: std::collections::HashSet<ir::Value> = std::collections::HashSet::new();
    let mut declared = std::collections::BTreeMap::new();
    for b in f.layout.blocks() {
        for &v in dfg.block_params(b) {
            declared.insert(v.as_u32(), if low.contains(&v) { "uint" } else { ty(dfg.value_type(v))?.0 });
        }
        for &v in f.layout.block_insts(b).flat_map(|i| dfg.inst_results(i)) {
            if read_in.get(&v).is_none_or(|blocks| blocks.iter().all(|&r| r == b)) {
                scoped.insert(v);
            } else {
                declared.insert(v.as_u32(), if low.contains(&v) { "uint" } else { ty(dfg.value_type(v))?.0 });
            }
        }
    }
    for (v, t) in &declared {
        writeln!(out, " {t} v{v}=0;").unwrap();
    }
    // The family's pointers are offsets from `t` (and `v`); the read's own parameters are its arguments.
    for (k, &p) in dfg.block_params(entry).iter().enumerate() {
        if family != Family::Tape && k == 0 {
            // The descriptor: its offset from `t`.
            writeln!(out, " {}=a0;", name(p)).unwrap();
        } else if k < family.pointers() {
            writeln!(out, " {}=0;", name(p)).unwrap();
        } else {
            writeln!(out, " {}=a{k};", name(p)).unwrap();
        }
    }
    writeln!(out, " uint b={};\n while(true){{switch(b){{", entry.as_u32()).unwrap();
    for block in f.layout.blocks() {
        writeln!(out, " case {}u:{{", block.as_u32()).unwrap();
        for inst in f.layout.block_insts(block) {
            let line = instruction(f, inst, &name, named, family, &low)?;
            match dfg.inst_results(inst).first() {
                // Its declaration where it is set.
                Some(&r) if scoped.contains(&r) => {
                    let t = if low.contains(&r) { "uint" } else { ty(dfg.value_type(r))?.0 };
                    writeln!(out, "  {t} {line}").unwrap();
                }
                _ => writeln!(out, "  {line}").unwrap(),
            }
        }
        out.push_str(" }\n");
    }
    out.push_str(" default:return 0;\n }}\n");
    Ok(())
}

/// A branch to `call`'s block: its arguments set its parameters (all read before any is set), then the block.
fn goto(f: &ir::Function, call: &ir::BlockCall, name: &dyn Fn(ir::Value) -> String, low: &std::collections::HashSet<ir::Value>) -> Result<String, String> {
    let dfg = &f.dfg;
    let block = call.block(&dfg.value_lists);
    let params = dfg.block_params(block);
    let args: Vec<String> = call
        .args(&dfg.value_lists)
        .map(|a| match a {
            ir::BlockArg::Value(v) => Ok(name(v)),
            a => Err(format!("a block argument {a:?}")),
        })
        .collect::<Result<_, String>>()?;
    let mut s = String::from("{");
    for (k, (p, a)) in params.iter().zip(&args).enumerate() {
        write!(s, "{} t{k}={a};", if low.contains(p) { "uint" } else { ty(dfg.value_type(*p))?.0 }).unwrap();
    }
    for (k, p) in params.iter().enumerate() {
        write!(s, "{}=t{k};", name(*p)).unwrap();
    }
    write!(s, "b={}u;continue;}}", block.as_u32()).unwrap();
    Ok(s)
}

fn cond(cc: IntCC, a: &str, b: &str, u: &str, s: &str) -> String {
    let (op, signed) = match cc {
        IntCC::Equal => ("==", false),
        IntCC::NotEqual => ("!=", false),
        IntCC::UnsignedLessThan => ("<", false),
        IntCC::UnsignedLessThanOrEqual => ("<=", false),
        IntCC::UnsignedGreaterThan => (">", false),
        IntCC::UnsignedGreaterThanOrEqual => (">=", false),
        IntCC::SignedLessThan => ("<", true),
        IntCC::SignedLessThanOrEqual => ("<=", true),
        IntCC::SignedGreaterThan => (">", true),
        IntCC::SignedGreaterThanOrEqual => (">=", true),
    };
    let t = if signed { s } else { u };
    format!("uchar(({t})({a}){op}({t})({b}))")
}

fn instruction(
    f: &ir::Function,
    inst: ir::Inst,
    name: &dyn Fn(ir::Value) -> String,
    named: &dyn Fn(u32) -> Result<String, String>,
    family: Family,
    low: &std::collections::HashSet<ir::Value>,
) -> Result<String, String> {
    let dfg = &f.dfg;
    let data = &dfg.insts[inst];
    let op = data.opcode();
    let results = dfg.inst_results(inst);
    // A low value (`low_words`) is a `uint`. A modular step to one is computed in 32 bits, from its operands' low
    // words; any other is computed whole, and its low word kept (a `uint` local takes it).
    let result_low = results.first().is_some_and(|r| low.contains(r));
    let modular = matches!(
        op,
        Opcode::Iadd
            | Opcode::Isub
            | Opcode::Imul
            | Opcode::Band
            | Opcode::Bor
            | Opcode::Bxor
            | Opcode::Bnot
            | Opcode::IaddImm
            | Opcode::ImulImm
            | Opcode::IrsubImm
            | Opcode::BandImm
            | Opcode::BorImm
            | Opcode::BxorImm
            | Opcode::Ishl
            | Opcode::IshlImm
            | Opcode::Select
    );
    let args: Vec<String> = dfg
        .inst_args(inst)
        .iter()
        .map(|&v| if result_low && modular && !low.contains(&dfg.resolve_aliases(v)) && dfg.value_type(v) == types::I64 { format!("uint({})", name(v)) } else { name(v) })
        .collect();
    let r = results.first().map(|&v| name(v));
    let rt = results.first().map(|&v| dfg.value_type(v));
    let at = dfg.inst_args(inst).first().map(|&v| dfg.value_type(v));
    let set = |e: String| -> String { format!("{}={e};", r.clone().expect("a result")) };
    let (u, s) = if result_low { ("uint", "int") } else { rt.and_then(|t| ty(t).ok()).unwrap_or(("ulong", "long")) };
    let (au, as_) = at.and_then(|t| ty(t).ok()).unwrap_or(("ulong", "long"));
    let bits = if result_low { 32 } else { rt.map_or(64, |t| t.bits()) };
    let imm = || -> Result<i64, String> {
        match data {
            InstructionData::UnaryImm { imm, .. } | InstructionData::BinaryImm64 { imm, .. } | InstructionData::IntCompareImm { imm, .. } => Ok(imm.bits()),
            _ => Err(format!("{op}: no immediate")),
        }
    };
    // An immediate as a value of the result's type (its low bits).
    let k = |x: i64| format!("{u}({}ul)", x as u64);
    let shift = |a: &str, b: &str, op: &str, signed: bool| {
        let t = if signed { s } else { u };
        if result_low && op == "<<" {
            // The low word of a 64-bit left shift: 0 past 31.
            return format!("((uint({b})&63u)<32u?uint({a})<<(uint({b})&31u):0u)");
        }
        format!("{u}(({t})({a}){op}(uint({b})&{}u))", bits - 1)
    };
    let addr = |p: &str| -> String {
        let off = data.load_store_offset().unwrap_or(0);
        let address_low = dfg.inst_args(inst).first().is_some_and(|a| low.contains(&dfg.resolve_aliases(*a)));
        if off == 0 {
            format!("{p}")
        } else if address_low {
            format!("({p}+uint({}u))", off as i64 as u64 as u32)
        } else {
            format!("({p}+ulong(long({off})))")
        }
    };
    Ok(match op {
        Opcode::Iconst => set(k(imm()?)),
        Opcode::Iadd => set(format!("{}+{}", args[0], args[1])),
        Opcode::Isub => set(format!("{}-{}", args[0], args[1])),
        Opcode::Imul => set(format!("{}*{}", args[0], args[1])),
        Opcode::Udiv => set(format!("{}/{}", args[0], args[1])),
        Opcode::Urem => set(format!("{}%{}", args[0], args[1])),
        Opcode::Band => set(format!("{}&{}", args[0], args[1])),
        Opcode::Bor => set(format!("{}|{}", args[0], args[1])),
        Opcode::Bxor => set(format!("{}^{}", args[0], args[1])),
        Opcode::Bnot => set(format!("~{}", args[0])),
        Opcode::Ishl => set(shift(&args[0], &args[1], "<<", false)),
        Opcode::Ushr | Opcode::Sshr if result_low => {
            let (wu, ws) = ty(at.unwrap_or(types::I64))?;
            let t = if op == Opcode::Sshr { ws } else { wu };
            set(format!("uint(({t})({}){}(uint({})&63u))", name(dfg.inst_args(inst)[0]), ">>", args[1]))
        }
        Opcode::Ushr => set(shift(&args[0], &args[1], ">>", false)),
        Opcode::Sshr => set(shift(&args[0], &args[1], ">>", true)),
        Opcode::IaddImm => set(format!("{}+{}", args[0], k(imm()?))),
        Opcode::ImulImm => set(format!("{}*{}", args[0], k(imm()?))),
        Opcode::IrsubImm => set(format!("{}-{}", k(imm()?), args[0])),
        Opcode::BandImm => set(format!("{}&{}", args[0], k(imm()?))),
        Opcode::BorImm => set(format!("{}|{}", args[0], k(imm()?))),
        Opcode::BxorImm => set(format!("{}^{}", args[0], k(imm()?))),
        Opcode::IshlImm => set(shift(&args[0], &(imm()? as u64 & 63).to_string(), "<<", false)),
        Opcode::UshrImm | Opcode::SshrImm if result_low => {
            let (wu, ws) = ty(at.unwrap_or(types::I64))?;
            let t = if op == Opcode::SshrImm { ws } else { wu };
            set(format!("uint(({t})({})>>{}u)", name(dfg.inst_args(inst)[0]), imm()? as u64 & 63))
        }
        Opcode::UshrImm => set(shift(&args[0], &(imm()? as u64 & 63).to_string(), ">>", false)),
        Opcode::SshrImm => set(shift(&args[0], &(imm()? as u64 & 63).to_string(), ">>", true)),
        Opcode::Clz => set(format!("{u}(clz({}))", args[0])),
        Opcode::Ctz => set(format!("{u}(ctz({}))", args[0])),
        Opcode::Popcnt => set(format!("{u}(popcount({}))", args[0])),
        Opcode::Umin => set(format!("min({},{})", args[0], args[1])),
        Opcode::Umax => set(format!("max({},{})", args[0], args[1])),
        Opcode::Smin => set(format!("{u}(min({s}({}),{s}({})))", args[0], args[1])),
        Opcode::Smax => set(format!("{u}(max({s}({}),{s}({})))", args[0], args[1])),
        Opcode::Icmp | Opcode::IcmpImm => {
            let cc = match data {
                InstructionData::IntCompare { cond, .. } | InstructionData::IntCompareImm { cond, .. } => *cond,
                _ => return Err("a comparison of no condition".into()),
            };
            let b = if op == Opcode::IcmpImm { format!("{au}({}ul)", imm()? as u64) } else { args[1].clone() };
            set(cond(cc, &args[0], &b, au, as_))
        }
        Opcode::Select => set(format!("{}!=0?{}:{}", args[0], args[1], args[2])),
        Opcode::Uextend | Opcode::Ireduce => set(format!("{u}({})", args[0])),
        Opcode::Sextend => set(format!("{u}({s}({as_}({})))", args[0])),
        Opcode::Load | Opcode::Uload8 | Opcode::Uload16 | Opcode::Uload32 => {
            let aligned = data.memflags().is_some_and(|m| m.aligned());
            let size = match op {
                Opcode::Uload8 => 8,
                Opcode::Uload16 => 16,
                Opcode::Uload32 => 32,
                _ => bits,
            };
            if family == Family::View && !aligned {
                return Err("a view read's load of the tape: the tape's reads make them".into());
            }
            if aligned && family != Family::Tape {
                // The block's own words, at their alignment.
                let t = match size {
                    8 => "uchar",
                    16 => "ushort",
                    32 => "uint",
                    _ => "ulong",
                };
                set(format!("{u}(*(device const {t}*)(t+{}))", addr(&args[0])))
            } else if aligned {
                // The view: its u32 fields.
                if size != 32 {
                    return Err(format!("a load of the view of {size} bits"));
                }
                set(format!("{u}(clif_view(v,{}))", addr(&args[0])))
            } else if result_low && size == 64 {
                // The low word of a little-endian 64-bit load is the 32-bit load at its address.
                set(format!("uint(clif_u32(t,{}))", addr(&args[0])))
            } else {
                set(format!("{u}(clif_u{size}(t,{}))", addr(&args[0])))
            }
        }
        Opcode::Call => {
            let InstructionData::Call { func_ref, .. } = data else { return Err("a call of no function".into()) };
            let callee = match &dfg.ext_funcs[*func_ref].name {
                ir::ExternalName::User(n) => f.params.user_named_funcs()[*n].index,
                n => return Err(format!("a call of {n:?}")),
            };
            // A read's call passes its pointers on: the callee's offsets count from the same ones. (A relation's read
            // passes `t` and the callee's descriptor, an offset from it.)
            // A view read's: the tape's reads take its tape (`tt`, `v`), the numbering's the descriptor it computed.
            let to = crate::reads::READS.get(callee as usize).map_or(family, |read| read.family);
            let more: Vec<&String> = args.iter().skip(to.pointers()).collect();
            let pointers = match (family, to) {
                (Family::View, Family::Tape) => "tt,v".to_owned(),
                (Family::View, Family::Lines) => "ll,0".to_owned(),
                (Family::View, Family::View) => format!("t,{},tt,v,ll", args[0]),
                (_, Family::Relation) => format!("t,{}", args[0]),
                (Family::Tape, _) => pointers(family).1.to_owned(),
                (from, to) => return Err(format!("a {from:?} read's call of a {to:?} read")),
            };
            let call = format!("{}({pointers}{}{})", named(callee)?, if more.is_empty() { "" } else { "," }, more.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(","));
            match results.len() {
                0 => format!("{call};"),
                1 => set(call),
                _ => return Err("a call of two results".into()),
            }
        }
        Opcode::Return => match args.len() {
            1 => format!("return {};", args[0]),
            _ => return Err("a read returns one value".into()),
        },
        Opcode::Jump => {
            let InstructionData::Jump { destination, .. } = data else { return Err("a jump with no destination".into()) };
            goto(f, destination, name, low)?
        }
        Opcode::Brif => {
            let InstructionData::Brif { blocks, .. } = data else { return Err("a branch with no blocks".into()) };
            format!("if({}!=0){}else{}", args[0], goto(f, &blocks[0], name, low)?, goto(f, &blocks[1], name, low)?)
        }
        op => return Err(format!("the opcode {op}")),
    })
}
