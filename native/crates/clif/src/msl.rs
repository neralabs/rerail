//! CLIF as Metal Shading Language (decision 0310's third backend): the tape's reads for the kernels, in place of
//! `metal_tape.metal`'s hand port.
//!
//! A read's function takes the tape (`device const uchar* t`, where the view's offsets count from: the
//! document's first byte in its batch's block) and its view as u32s (`thread const uint* v`, the device's `ChunkTape` past its `at`), then its own
//! parameters. In the body the tape and the view are byte offsets from those pointers, 0 where they come in, so
//! an address the read computes is an offset: a load of the view (marked aligned: its fields) reads `v`, a load of
//! the tape reads `t`, at any alignment (two aligned words shifted together where it is not aligned to its size;
//! a slab lays the document's source after its tape, so the second word lies in it).
//!
//! MSL has no `goto` and no labelled `break`: a body is a loop over a `switch` on its block, which the Metal
//! compiler makes branches again. Each value is a local, set where a branch passes it to a block.

use cranelift_codegen::ir::{self, InstructionData, Opcode, condcodes::IntCC, types};
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

/// The loads every read's body uses: of the tape at any alignment.
pub const HELPERS: &str = "\
inline ulong clif_word(device const uchar* t,ulong a){device const ulong* w=(device const ulong*)(t+(a&~7ul));uint s=uint(a&7ul)*8u;return s==0u?w[0]:(w[0]>>s)|(w[1]<<(64u-s));}
inline uchar clif_u8(device const uchar* t,ulong a){return t[a];}
inline ushort clif_u16(device const uchar* t,ulong a){return (a&1ul)==0ul?*(device const ushort*)(t+a):ushort(clif_word(t,a));}
inline uint clif_u32(device const uchar* t,ulong a){return (a&3ul)==0ul?*(device const uint*)(t+a):uint(clif_word(t,a));}
inline ulong clif_u64(device const uchar* t,ulong a){return clif_word(t,a);}
";

/// The reads `funcs` (by their user name's index, each with its name) as MSL functions, `prefix` before each
/// name: declarations first, so a read may call one defined after it.
pub fn translate_reads(funcs: &[(u32, ir::Function, &str)], prefix: &str) -> Result<String, String> {
    let named = |id: u32| funcs.iter().find(|(f, ..)| *f == id).map(|(.., n)| format!("{prefix}{n}")).ok_or(format!("a call of u0:{id}, no read"));
    let mut out = String::from(HELPERS);
    let mut bodies = String::new();
    for (id, f, n) in funcs {
        let signature = signature(f, &format!("{prefix}{n}"))?;
        writeln!(out, "{signature};").unwrap();
        writeln!(bodies, "{signature}{{").unwrap();
        body(&mut bodies, f, &named).map_err(|e| format!("{n} (u0:{id}): {e}"))?;
        bodies.push_str("}\n");
    }
    out.push_str(&bodies);
    Ok(out)
}

fn signature(f: &ir::Function, name: &str) -> Result<String, String> {
    let [ret] = f.signature.returns.as_slice() else { return Err(format!("{name}: a read returns one value")) };
    let params: Vec<String> = f.signature.params.iter().enumerate().skip(2).map(|(i, p)| Ok(format!("{} a{i}", ty(p.value_type)?.0))).collect::<Result<_, String>>()?;
    let more = if params.is_empty() { String::new() } else { format!(",{}", params.join(",")) };
    Ok(format!("inline {} {name}(device const uchar* t,thread const uint* v{more})", ty(ret.value_type)?.0))
}

fn body(out: &mut String, f: &ir::Function, named: &dyn Fn(u32) -> Result<String, String>) -> Result<(), String> {
    let dfg = &f.dfg;
    let name = |v: ir::Value| format!("v{}", dfg.resolve_aliases(v).as_u32());
    let entry = f.layout.entry_block().ok_or("no entry block")?;
    let mut declared = std::collections::BTreeMap::new();
    for b in f.layout.blocks() {
        for &v in dfg.block_params(b).iter().chain(f.layout.block_insts(b).flat_map(|i| dfg.inst_results(i))) {
            declared.insert(v.as_u32(), ty(dfg.value_type(v))?.0);
        }
    }
    for (v, t) in &declared {
        writeln!(out, " {t} v{v}=0;").unwrap();
    }
    // The tape and the view are offsets from `t` and `v`; the read's own parameters are its arguments.
    for (k, &p) in dfg.block_params(entry).iter().enumerate() {
        match k {
            0 | 1 => writeln!(out, " {}=0ul;", name(p)).unwrap(),
            _ => writeln!(out, " {}=a{k};", name(p)).unwrap(),
        }
    }
    writeln!(out, " uint b={};\n while(true){{switch(b){{", entry.as_u32()).unwrap();
    for block in f.layout.blocks() {
        writeln!(out, " case {}u:{{", block.as_u32()).unwrap();
        for inst in f.layout.block_insts(block) {
            writeln!(out, "  {}", instruction(f, inst, &name, named)?).unwrap();
        }
        out.push_str(" }\n");
    }
    out.push_str(" default:return 0;\n }}\n");
    Ok(())
}

/// A branch to `call`'s block: its arguments set its parameters (all read before any is set), then the block.
fn goto(f: &ir::Function, call: &ir::BlockCall, name: &dyn Fn(ir::Value) -> String) -> Result<String, String> {
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
        write!(s, "{} t{k}={a};", ty(dfg.value_type(*p))?.0).unwrap();
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

fn instruction(f: &ir::Function, inst: ir::Inst, name: &dyn Fn(ir::Value) -> String, named: &dyn Fn(u32) -> Result<String, String>) -> Result<String, String> {
    let dfg = &f.dfg;
    let data = &dfg.insts[inst];
    let op = data.opcode();
    let args: Vec<String> = dfg.inst_args(inst).iter().map(|&v| name(v)).collect();
    let results = dfg.inst_results(inst);
    let r = results.first().map(|&v| name(v));
    let rt = results.first().map(|&v| dfg.value_type(v));
    let at = dfg.inst_args(inst).first().map(|&v| dfg.value_type(v));
    let set = |e: String| -> String { format!("{}={e};", r.clone().expect("a result")) };
    let (u, s) = rt.and_then(|t| ty(t).ok()).unwrap_or(("ulong", "long"));
    let (au, as_) = at.and_then(|t| ty(t).ok()).unwrap_or(("ulong", "long"));
    let bits = rt.map_or(64, |t| t.bits());
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
        format!("{u}(({t})({a}){op}(uint({b})&{}u))", bits - 1)
    };
    let addr = |p: &str| -> String {
        let off = data.load_store_offset().unwrap_or(0);
        if off == 0 { format!("{p}") } else { format!("({p}+ulong(long({off})))") }
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
        Opcode::Ushr => set(shift(&args[0], &args[1], ">>", false)),
        Opcode::Sshr => set(shift(&args[0], &args[1], ">>", true)),
        Opcode::IaddImm => set(format!("{}+{}", args[0], k(imm()?))),
        Opcode::ImulImm => set(format!("{}*{}", args[0], k(imm()?))),
        Opcode::IrsubImm => set(format!("{}-{}", k(imm()?), args[0])),
        Opcode::BandImm => set(format!("{}&{}", args[0], k(imm()?))),
        Opcode::BorImm => set(format!("{}|{}", args[0], k(imm()?))),
        Opcode::BxorImm => set(format!("{}^{}", args[0], k(imm()?))),
        Opcode::IshlImm => set(shift(&args[0], &(imm()? as u64 & 63).to_string(), "<<", false)),
        Opcode::UshrImm => set(shift(&args[0], &(imm()? as u64 & 63).to_string(), ">>", false)),
        Opcode::SshrImm => set(shift(&args[0], &(imm()? as u64 & 63).to_string(), ">>", true)),
        Opcode::Umin => set(format!("min({},{})", args[0], args[1])),
        Opcode::Umax => set(format!("max({},{})", args[0], args[1])),
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
            if aligned {
                // The view: its u32 fields.
                if size != 32 {
                    return Err(format!("a load of the view of {size} bits"));
                }
                set(format!("{u}(v[{}/4ul])", addr(&args[0])))
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
            // A read's call passes its tape and view on: the callee's offsets count from the same pointers.
            let more: Vec<&String> = args.iter().skip(2).collect();
            let call = format!("{}(t,v{}{})", named(callee)?, if more.is_empty() { "" } else { "," }, more.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(","));
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
            goto(f, destination, name)?
        }
        Opcode::Brif => {
            let InstructionData::Brif { blocks, .. } = data else { return Err("a branch with no blocks".into()) };
            format!("if({}!=0){}else{}", args[0], goto(f, &blocks[0], name)?, goto(f, &blocks[1], name)?)
        }
        op => return Err(format!("the opcode {op}")),
    })
}
