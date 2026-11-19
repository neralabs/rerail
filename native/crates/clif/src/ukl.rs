//! CLIF as UKL (decisions 0310, 0400 §3.3): the reads written once in CLIF, for every GPU's kernels. UKL is C with
//! macros for what differs between GPU languages; the backend's shim turns it into MSL for Metal or CUDA C++ for NVRTC,
//! and the library (`<ukl/...>`, decision 0400 §5) gives the loads, stores and atomics, written once over each
//! backend's intrinsics.
//!
//! A read's function takes the tape (`UKL_GLOBAL const uint8_t* t`, where the view's offsets count from: the
//! document's first byte in its batch's block) and its view as u32s (`CLIF_VIEW_SPACE const uint32_t* v`: the thread's
//! own unless a kernel source defines it `UKL_GLOBAL`, as the join kernels do, which read each document's view where the
//! batch's directory holds it; the device's `DocView`, `at` first), then its own parameters. In the body the tape and
//! the view are byte offsets from those pointers, 0 where they come in, so an address the read computes is an offset:
//! a load of the view (marked aligned: its fields) reads `v` (`clif_view`, which a kernel may define first), a load
//! of the tape reads `t` at any alignment (`ukl_ld_*`: two aligned words shifted together where it is not aligned to
//! its size; a slab lays the document's source after its tape, so the second word lies in it).
//!
//! A relation's read (decision 0325) takes a base (`UKL_GLOBAL const uint8_t* t`, a block's first byte, or any pointer
//! before the descriptor in its buffer) and the descriptor's offset from it (`uint64_t a0`), then its own parameters:
//! every address it computes is an offset from `t`, through the block's self-relative references, so an address a
//! read returns (`rows_at`) and a descriptor a read calls another of are offsets from the same `t`. Its loads are
//! aligned, and read as their type.
//!
//! No backend has `goto` or a labelled `break` (Metal has neither), so a body is written structured (`Shape`): a
//! branch as `if`/`else` with its merge after it, a loop as `while(true)` with `continue` and `break`, a block reached
//! on two paths that are not its merge's written on each. A body that takes no such shape (an irreducible loop, a loop
//! of several exits) is a loop over a `switch` on its block. A value read in another block than its own is a local of
//! the body, set where a branch passes it on; one read only in its own block is declared where it is set. (The switch
//! form cost the label kernels about 20% against hand-written reads, t41.)
//!
//! A GPU computes in 32 bits; a 64-bit add or shift is several instructions. Every offset a read computes from the
//! tape or the view is below 4 GiB on the device (a document's parts are offsets from its first byte, u32 in its
//! view), so a 64-bit value whose every use reads only its low 32 bits (`low_words`: an address, a modular step to
//! another such value) is computed as a `uint32_t`, with the same low bits, and a 64-bit load of it reads 32 bits. A
//! relation's offsets are a block's, not bounded so: its reads keep 64 bits.

use cranelift_codegen::ir::{self, InstructionData, Opcode, condcodes::IntCC, types};
use crate::reads::Family;
use std::fmt::Write;

/// The unsigned type for a Cranelift integer type, and its signed one.
fn ty(t: ir::Type) -> Result<(&'static str, &'static str), String> {
    Ok(match t {
        types::I8 => ("uint8_t", "int8_t"),
        types::I16 => ("uint16_t", "int16_t"),
        types::I32 => ("uint32_t", "int32_t"),
        types::I64 => ("uint64_t", "int64_t"),
        t => return Err(format!("a value of type {t}")),
    })
}

/// What every read's body includes: the library's loads, stores and atomics (`ukl/memory.h`), and the reads' hooks
/// (`ukl/clif.h`: the view's space and its load, which a kernel source may define before the reads).
pub const HELPERS: &str = "#pragma once\n#include <ukl/memory.h>\n#include <ukl/clif.h>\n";

/// The reads `funcs` of `family` (by their user name's index, each with its name) as UKL functions, `prefix` before
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
        let hot = crate::reads::READS.get(*id as usize).is_none_or(|read| read.hot);
        let signature = signature(f, &format!("{prefix}{n}"), family, hot)?;
        writeln!(out, "{signature};").unwrap();
        writeln!(bodies, "{signature}{{").unwrap();
        body(&mut bodies, f, &named, family).map_err(|e| format!("{n} (u0:{id}): {e}"))?;
        bodies.push_str("}\n");
    }
    out.push_str(&bodies);
    Ok(out)
}

/// The 64-bit values of `f` of which every use reads only the low 32 bits (a demanded-bits pass to its fixpoint):
/// a load's address, the operand of an add, subtract, multiply, bitwise operation or left shift whose result is one,
/// a shift's amount, a reduction's operand, a select's arm that is one, a branch's argument to a block parameter that
/// is one, a call's tape and view (passed on as pointers). A comparison, a right shift, a division, a minimum, a
/// call's own argument or a return reads the whole value.
fn low_words(f: &ir::Function, family: Family) -> std::collections::HashSet<ir::Value> {
    let dfg = &f.dfg;
    let mut low: std::collections::HashSet<ir::Value> = std::collections::HashSet::new();
    if family == Family::Relation {
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
                        // A load of the tape (read-only, below 4 GiB of it); a cache's (decision 0353 step 4b) may lie
                        // anywhere from it.
                        Opcode::Load | Opcode::Uload8 | Opcode::Uload16 | Opcode::Uload32 => k == 0 && data.memflags().is_some_and(|m| m.readonly()),
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

/// The pointers a read of `family` takes, as its parameters and as a call passes them on.
fn pointers(family: Family) -> (&'static str, &'static str) {
    match family {
        Family::Tape => ("UKL_GLOBAL const uint8_t* t,CLIF_VIEW_SPACE const uint32_t* v", "t,v"),
        Family::Relation | Family::Lines => ("UKL_GLOBAL const uint8_t* t,uint64_t a0", "t"),
        Family::View => ("UKL_GLOBAL const uint8_t* t,uint64_t a0,UKL_GLOBAL const uint8_t* tt,CLIF_VIEW_SPACE const uint32_t* v,UKL_GLOBAL const uint8_t* ll", "t"),
    }
}

/// A read's declaration: `UKL_INLINE` where it is hot, `UKL_COLD` (a call) where it is marked cold, as the JIT's
/// inlining pass and the Rust backend treat it (decision 0310).
fn signature(f: &ir::Function, name: &str, family: Family, hot: bool) -> Result<String, String> {
    let [ret] = f.signature.returns.as_slice() else { return Err(format!("{name}: a read returns one value")) };
    let params: Vec<String> =
        f.signature.params.iter().enumerate().skip(family.pointers()).map(|(i, p)| Ok(format!("{} a{i}", ty(p.value_type)?.0))).collect::<Result<_, String>>()?;
    let more = if params.is_empty() { String::new() } else { format!(",{}", params.join(",")) };
    let qualifier = if hot { "UKL_INLINE" } else { "UKL_COLD" };
    Ok(format!("{qualifier} {} {name}({}{more})", ty(ret.value_type)?.0, pointers(family).0))
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
            declared.insert(v.as_u32(), if low.contains(&v) { "uint32_t" } else { ty(dfg.value_type(v))?.0 });
        }
        for &v in f.layout.block_insts(b).flat_map(|i| dfg.inst_results(i)) {
            if read_in.get(&v).is_none_or(|blocks| blocks.iter().all(|&r| r == b)) {
                scoped.insert(v);
            } else {
                declared.insert(v.as_u32(), if low.contains(&v) { "uint32_t" } else { ty(dfg.value_type(v))?.0 });
            }
        }
    }
    for (v, t) in &declared {
        writeln!(out, " {t} v{v}=0;").unwrap();
    }
    // The family's pointers are offsets from `t` (and `v`): a relation's descriptor is its offset from `t`. The
    // read's own parameters are its arguments.
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
    // A block's instructions but its branch, each value declared where it is set when only its block reads it.
    let straight = |out: &mut String, block: ir::Block, indent: &str| -> Result<(), String> {
        for inst in f.layout.block_insts(block) {
            if dfg.insts[inst].opcode().is_terminator() {
                continue;
            }
            let line = instruction(f, inst, &name, named, &low, family)?;
            match dfg.inst_results(inst).first() {
                Some(&r) if scoped.contains(&r) => {
                    let t = if low.contains(&r) { "uint32_t" } else { ty(dfg.value_type(r))?.0 };
                    writeln!(out, "{indent}{t} {line}").unwrap();
                }
                _ => writeln!(out, "{indent}{line}").unwrap(),
            }
        }
        Ok(())
    };
    // Structured: branches as `if`, loops as `while` (`Shape`). A body whose blocks do not take that shape keeps the
    // loop over a switch.
    if let Some(shape) = Shape::of(f) {
        let mut text = String::new();
        let mut loops = Vec::new();
        if shape.emit(f, entry, None, &mut loops, &mut text, 1, &straight, &|call| assign(f, call, &name, &low))?.is_some() {
            out.push_str(&text);
            return Ok(());
        }
    }
    writeln!(out, " uint32_t b={};\n while(true){{switch(b){{", entry.as_u32()).unwrap();
    for block in f.layout.blocks() {
        writeln!(out, " case {}u:{{", block.as_u32()).unwrap();
        straight(out, block, "  ")?;
        let last = f.layout.last_inst(block).ok_or("an empty block")?;
        writeln!(out, "  {}", instruction(f, last, &name, named, &low, family)?).unwrap();
        out.push_str(" }\n");
    }
    out.push_str(" default:return 0;\n }}\n");
    Ok(())
}

/// A body's blocks as structured code can take them: each branch's merge (its immediate post-dominator), and each
/// loop's header with its exits. A loop's header is a block a back edge reaches (in a depth-first order from the
/// entry) and that dominates the edge's source; a back edge to a block that does not dominate it leaves the body to
/// the switch. A loop of several exits sets which it leaves by (`e<header>`), and the text after the loop takes
/// each exit to the loop's merge.
struct Shape {
    /// Each block's immediate post-dominator (`None`: every path from it returns before any block is common).
    merge: std::collections::HashMap<ir::Block, Option<ir::Block>>,
    /// Each loop's header and its exits.
    loops: std::collections::HashMap<ir::Block, Vec<ir::Block>>,
}

/// The emitted text's bound, in bytes: a body whose duplicated blocks pass it keeps the switch.
const STRUCTURED_MAX: usize = 1 << 16;

impl Shape {
    fn of(f: &ir::Function) -> Option<Shape> {
        use std::collections::{BTreeSet, HashMap, HashSet};
        let dfg = &f.dfg;
        let blocks: Vec<ir::Block> = f.layout.blocks().collect();
        let mut succs: HashMap<ir::Block, Vec<ir::Block>> = HashMap::new();
        for &b in &blocks {
            let last = f.layout.last_inst(b)?;
            let to = dfg.insts[last].branch_destination(&dfg.jump_tables, &dfg.exception_tables).iter().map(|c| c.block(&dfg.value_lists)).collect();
            succs.insert(b, to);
        }
        let entry = f.layout.entry_block()?;
        // Dominators (iterative sets: a read has few blocks).
        let all: BTreeSet<ir::Block> = blocks.iter().copied().collect();
        let mut preds: HashMap<ir::Block, Vec<ir::Block>> = HashMap::new();
        for (&b, to) in &succs {
            for &t in to {
                preds.entry(t).or_default().push(b);
            }
        }
        let mut dom: HashMap<ir::Block, BTreeSet<ir::Block>> = blocks.iter().map(|&b| (b, if b == entry { BTreeSet::from([b]) } else { all.clone() })).collect();
        loop {
            let mut changed = false;
            for &b in &blocks {
                if b == entry {
                    continue;
                }
                let mut new: Option<BTreeSet<ir::Block>> = None;
                for p in preds.get(&b).into_iter().flatten() {
                    new = Some(match new {
                        None => dom[p].clone(),
                        Some(n) => n.intersection(&dom[p]).copied().collect(),
                    });
                }
                let mut new = new.unwrap_or_default();
                new.insert(b);
                if new != dom[&b] {
                    dom.insert(b, new);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // Back edges, by a depth-first walk from the entry: each must reach a block dominating its source.
        let mut back: Vec<(ir::Block, ir::Block)> = Vec::new();
        let (mut on, mut seen) = (HashSet::new(), HashSet::new());
        let mut stack = vec![(entry, 0usize)];
        on.insert(entry);
        seen.insert(entry);
        while let Some((b, k)) = stack.pop() {
            if let Some(&t) = succs[&b].get(k) {
                stack.push((b, k + 1));
                if on.contains(&t) {
                    back.push((b, t));
                } else if seen.insert(t) {
                    on.insert(t);
                    stack.push((t, 0));
                }
            } else {
                on.remove(&b);
            }
        }
        let mut loops = HashMap::new();
        for &(_, h) in &back {
            if loops.contains_key(&h) {
                continue;
            }
            let sources: Vec<ir::Block> = back.iter().filter(|(_, t)| *t == h).map(|(s, _)| *s).collect();
            if sources.iter().any(|s| !dom[s].contains(&h)) {
                return None;
            }
            let mut body: HashSet<ir::Block> = HashSet::from([h]);
            let mut work = sources.clone();
            while let Some(b) = work.pop() {
                if body.insert(b) {
                    work.extend(preds.get(&b).into_iter().flatten().copied());
                }
            }
            let exits: BTreeSet<ir::Block> = body.iter().flat_map(|b| succs[b].iter().copied()).filter(|t| !body.contains(t)).collect();
            if exits.is_empty() {
                return None;
            }
            loops.insert(h, exits.into_iter().collect());
        }
        // Post-dominators, a return's block reaching none.
        let mut pdom: HashMap<ir::Block, BTreeSet<ir::Block>> = blocks.iter().map(|&b| (b, if succs[&b].is_empty() { BTreeSet::from([b]) } else { all.clone() })).collect();
        loop {
            let mut changed = false;
            for &b in blocks.iter().rev() {
                if succs[&b].is_empty() {
                    continue;
                }
                let mut new: Option<BTreeSet<ir::Block>> = None;
                for s in &succs[&b] {
                    new = Some(match new {
                        None => pdom[s].clone(),
                        Some(n) => n.intersection(&pdom[s]).copied().collect(),
                    });
                }
                let mut new = new.unwrap_or_default();
                new.insert(b);
                if new != pdom[&b] {
                    pdom.insert(b, new);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let merge = blocks
            .iter()
            .map(|&b| {
                let strict = pdom[&b].len() - 1;
                (b, pdom[&b].iter().copied().find(|&d| d != b && pdom[&d].len() == strict))
            })
            .collect();
        Some(Shape { merge, loops })
    }

    /// Emits `block` and what follows it up to `stop` (not emitted): `None` where the text passes its bound.
    #[allow(clippy::too_many_arguments)]
    fn emit(
        &self,
        f: &ir::Function,
        block: ir::Block,
        stop: Option<ir::Block>,
        loops: &mut Vec<(ir::Block, Vec<ir::Block>)>,
        out: &mut String,
        depth: usize,
        straight: &dyn Fn(&mut String, ir::Block, &str) -> Result<(), String>,
        assign: &dyn Fn(&ir::BlockCall) -> Result<String, String>,
    ) -> Result<Option<()>, String> {
        if out.len() > STRUCTURED_MAX || depth > 64 {
            return Ok(None);
        }
        let indent = " ".repeat(depth);
        // A loop's header not yet entered: the loop, then its exits.
        if let Some(exits) = self.loops.get(&block)
            && loops.last().is_none_or(|(h, _)| *h != block)
        {
            let h = block.as_u32();
            if exits.len() > 1 {
                writeln!(out, "{indent}uint32_t e{h}=0u;").unwrap();
            }
            writeln!(out, "{indent}while(true){{").unwrap();
            loops.push((block, exits.clone()));
            let inner = self.emit(f, block, None, loops, out, depth + 1, straight, assign)?;
            loops.pop();
            if inner.is_none() {
                return Ok(None);
            }
            writeln!(out, "{indent}}}").unwrap();
            let [exit] = exits[..] else {
                // Each exit to the loop's merge, then the merge.
                let merge = self.merge[&block].filter(|m| Some(*m) != stop && !loops.iter().any(|(h, x)| m == h || x.contains(m)));
                let to = merge.or(stop);
                for (k, &x) in exits.iter().enumerate() {
                    if Some(x) == to {
                        continue;
                    }
                    writeln!(out, "{indent}if(e{h}=={k}u){{").unwrap();
                    if self.emit(f, x, to, loops, out, depth + 1, straight, assign)?.is_none() {
                        return Ok(None);
                    }
                    writeln!(out, "{indent}}}").unwrap();
                }
                return match merge {
                    Some(m) => self.emit(f, m, stop, loops, out, depth, straight, assign),
                    None => Ok(Some(())),
                };
            };
            return if Some(exit) == stop { Ok(Some(())) } else { self.emit(f, exit, stop, loops, out, depth, straight, assign) };
        }
        straight(out, block, &indent)?;
        let dfg = &f.dfg;
        let last = f.layout.last_inst(block).ok_or("an empty block")?;
        match &dfg.insts[last] {
            InstructionData::Jump { destination, .. } => self.edge(f, destination, stop, loops, out, depth, straight, assign),
            InstructionData::Brif { blocks, arg, .. } => {
                let merge = self.merge[&block];
                let c = format!("v{}", dfg.resolve_aliases(*arg).as_u32());
                writeln!(out, "{indent}if({c}!=0){{").unwrap();
                if self.edge(f, &blocks[0], merge, loops, out, depth + 1, straight, assign)?.is_none() {
                    return Ok(None);
                }
                writeln!(out, "{indent}}}else{{").unwrap();
                if self.edge(f, &blocks[1], merge, loops, out, depth + 1, straight, assign)?.is_none() {
                    return Ok(None);
                }
                writeln!(out, "{indent}}}").unwrap();
                match merge {
                    // The merge of a loop's branch is its header or exit: the loop's own text reaches it.
                    Some(m) if Some(m) != stop && !loops.iter().any(|(h, x)| m == *h || x.contains(&m)) => self.emit(f, m, stop, loops, out, depth, straight, assign),
                    _ => Ok(Some(())),
                }
            }
            _ if dfg.insts[last].opcode() == Opcode::Return => {
                let [v] = dfg.inst_args(last) else { return Err("a read returns one value".into()) };
                writeln!(out, "{indent}return v{};", dfg.resolve_aliases(*v).as_u32()).unwrap();
                Ok(Some(()))
            }
            data => Err(format!("the branch {:?}", data.opcode())),
        }
    }

    /// A branch to `call`'s block: its parameters set, then the innermost loop's `continue` or `break`, nothing at
    /// `stop` (the text after reaches it), or the block.
    #[allow(clippy::too_many_arguments)]
    fn edge(
        &self,
        f: &ir::Function,
        call: &ir::BlockCall,
        stop: Option<ir::Block>,
        loops: &mut Vec<(ir::Block, Vec<ir::Block>)>,
        out: &mut String,
        depth: usize,
        straight: &dyn Fn(&mut String, ir::Block, &str) -> Result<(), String>,
        assign: &dyn Fn(&ir::BlockCall) -> Result<String, String>,
    ) -> Result<Option<()>, String> {
        let indent = " ".repeat(depth);
        let set = assign(call)?;
        if !set.is_empty() {
            writeln!(out, "{indent}{set}").unwrap();
        }
        let to = call.block(&f.dfg.value_lists);
        if let Some((h, x)) = loops.last() {
            if to == *h {
                writeln!(out, "{indent}continue;").unwrap();
                return Ok(Some(()));
            }
            if let Some(k) = x.iter().position(|&x| x == to) {
                if x.len() > 1 {
                    writeln!(out, "{indent}e{}={k}u;", h.as_u32()).unwrap();
                }
                writeln!(out, "{indent}break;").unwrap();
                return Ok(Some(()));
            }
        }
        if Some(to) == stop {
            return Ok(Some(()));
        }
        // A branch out of an outer loop from an inner one has no `break` to take.
        if loops.iter().rev().skip(1).any(|(h, x)| to == *h || x.contains(&to)) {
            return Ok(None);
        }
        self.emit(f, to, stop, loops, out, depth, straight, assign)
    }
}

/// A branch's arguments set its block's parameters, all read before any is set (empty for none).
fn assign(f: &ir::Function, call: &ir::BlockCall, name: &dyn Fn(ir::Value) -> String, low: &std::collections::HashSet<ir::Value>) -> Result<String, String> {
    let dfg = &f.dfg;
    let params = dfg.block_params(call.block(&dfg.value_lists));
    if params.is_empty() {
        return Ok(String::new());
    }
    let args: Vec<String> = call
        .args(&dfg.value_lists)
        .map(|a| match a {
            ir::BlockArg::Value(v) => Ok(name(v)),
            a => Err(format!("a block argument {a:?}")),
        })
        .collect::<Result<_, String>>()?;
    let mut s = String::from("{");
    for (k, (p, a)) in params.iter().zip(&args).enumerate() {
        write!(s, "{} t{k}={a};", if low.contains(p) { "uint32_t" } else { ty(dfg.value_type(*p))?.0 }).unwrap();
    }
    for (k, p) in params.iter().enumerate() {
        write!(s, "{}=t{k};", name(*p)).unwrap();
    }
    s.push('}');
    Ok(s)
}

/// A branch to `call`'s block in the switch form: its parameters set (`assign`), then the block.
fn goto(f: &ir::Function, call: &ir::BlockCall, name: &dyn Fn(ir::Value) -> String, low: &std::collections::HashSet<ir::Value>) -> Result<String, String> {
    Ok(format!("{{{}b={}u;continue;}}", assign(f, call, name, low)?, call.block(&f.dfg.value_lists).as_u32()))
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
    format!("((uint8_t)(({t})({a}){op}({t})({b})))")
}

fn instruction(f: &ir::Function, inst: ir::Inst, name: &dyn Fn(ir::Value) -> String, named: &dyn Fn(u32) -> Result<String, String>, low: &std::collections::HashSet<ir::Value>, family: Family) -> Result<String, String> {
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
        .map(|&v| if result_low && modular && !low.contains(&dfg.resolve_aliases(v)) && dfg.value_type(v) == types::I64 { format!("((uint32_t)({}))", name(v)) } else { name(v) })
        .collect();
    let r = results.first().map(|&v| name(v));
    let rt = results.first().map(|&v| dfg.value_type(v));
    let at = dfg.inst_args(inst).first().map(|&v| dfg.value_type(v));
    let set = |e: String| -> String { format!("{}={e};", r.clone().expect("a result")) };
    let (u, s) = if result_low { ("uint32_t", "int32_t") } else { rt.and_then(|t| ty(t).ok()).unwrap_or(("uint64_t", "int64_t")) };
    let (au, as_) = at.and_then(|t| ty(t).ok()).unwrap_or(("uint64_t", "int64_t"));
    let bits = if result_low { 32 } else { rt.map_or(64, |t| t.bits()) };
    let imm = || -> Result<i64, String> {
        match data {
            InstructionData::UnaryImm { imm, .. } | InstructionData::BinaryImm64 { imm, .. } | InstructionData::IntCompareImm { imm, .. } => Ok(imm.bits()),
            _ => Err(format!("{op}: no immediate")),
        }
    };
    // An immediate as a value of the result's type (its low bits).
    let k = |x: i64| format!("(({u})({}ul))", x as u64);
    let shift = |a: &str, b: &str, op: &str, signed: bool| {
        let t = if signed { s } else { u };
        if result_low && op == "<<" {
            // The low word of a 64-bit left shift: 0 past 31.
            return format!("((((uint32_t)({b}))&63u)<32u?((uint32_t)({a}))<<(((uint32_t)({b}))&31u):0u)");
        }
        format!("(({u})(({t})({a}){op}(((uint32_t)({b}))&{}u)))", bits - 1)
    };
    let addr = |p: &str| -> String {
        let off = data.load_store_offset().unwrap_or(0);
        let address_low = dfg.inst_args(inst).first().is_some_and(|a| low.contains(&dfg.resolve_aliases(*a)));
        if off == 0 {
            p.to_string()
        } else if address_low {
            format!("({p}+((uint32_t)({}u)))", off as i64 as u64 as u32)
        } else {
            format!("({p}+((uint64_t)(((int64_t)({off})))))")
        }
    };
    // A cache's address (decision 0353 step 4b): 64 bits, its offset added as a signed one.
    let wide_addr = |p: &str| -> String {
        let off = data.load_store_offset().unwrap_or(0);
        if off == 0 { format!("((uint64_t)({p}))") } else { format!("(((uint64_t)({p}))+((uint64_t)(((int64_t)({off})))))") }
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
            set(format!("((uint32_t)(({t})({}){}(((uint32_t)({}))&63u)))", name(dfg.inst_args(inst)[0]), ">>", args[1]))
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
            set(format!("((uint32_t)(({t})({})>>{}u))", name(dfg.inst_args(inst)[0]), imm()? as u64 & 63))
        }
        Opcode::UshrImm => set(shift(&args[0], &(imm()? as u64 & 63).to_string(), ">>", false)),
        Opcode::SshrImm => set(shift(&args[0], &(imm()? as u64 & 63).to_string(), ">>", true)),
        // The library's bit counts, by the operand's width.
        Opcode::Clz | Opcode::Popcnt | Opcode::Ctz => {
            let f = match op {
                Opcode::Clz => "clz",
                Opcode::Popcnt => "popcount",
                _ => "ctz",
            };
            let w = if at == Some(types::I64) { 64 } else { 32 };
            set(format!("(({u})(ukl_{f}{w}({})))", args[0]))
        }
        Opcode::Umin => set(format!("ukl_min({},{})", args[0], args[1])),
        Opcode::Umax => set(format!("ukl_max({},{})", args[0], args[1])),
        Opcode::Smin => set(format!("(({u})(ukl_min((({s})({})),(({s})({})))))", args[0], args[1])),
        Opcode::Smax => set(format!("(({u})(ukl_max((({s})({})),(({s})({})))))", args[0], args[1])),
        Opcode::Icmp | Opcode::IcmpImm => {
            let cc = match data {
                InstructionData::IntCompare { cond, .. } | InstructionData::IntCompareImm { cond, .. } => *cond,
                _ => return Err("a comparison of no condition".into()),
            };
            let b = if op == Opcode::IcmpImm { format!("(({au})({}ul))", imm()? as u64) } else { args[1].clone() };
            set(cond(cc, &args[0], &b, au, as_))
        }
        Opcode::Select => set(format!("{}!=0?{}:{}", args[0], args[1], args[2])),
        Opcode::Uextend | Opcode::Ireduce => set(format!("(({u})({}))", args[0])),
        Opcode::Sextend => set(format!("(({u})((({s})((({as_})({}))))))", args[0])),
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
            let readonly = data.memflags().is_some_and(|m| m.readonly());
            if aligned && family != Family::Tape {
                // The block's own words, at their alignment.
                let t = match size {
                    8 => "uint8_t",
                    16 => "uint16_t",
                    32 => "uint32_t",
                    _ => "uint64_t",
                };
                set(format!("(({u})(*(UKL_GLOBAL const {t}*)(t+{})))", addr(&args[0])))
            } else if aligned {
                // The view: its u32 fields.
                if size != 32 {
                    return Err(format!("a load of the view of {size} bits"));
                }
                set(format!("(({u})(clif_view(v,{})))", addr(&args[0])))
            } else if !readonly {
                // A cache's (decision 0353 step 4b), which other threads write: coherent across the device.
                set(format!("(({u})(ukl_ldc_u{size}(clif_coherent(t),{})))", wide_addr(&args[0])))
            } else if result_low && size == 64 {
                // The low word of a little-endian 64-bit load is the 32-bit load at its address.
                set(format!("((uint32_t)(ukl_ld_u32(t,{})))", addr(&args[0])))
            } else {
                set(format!("(({u})(ukl_ld_u{size}(t,{})))", addr(&args[0])))
            }
        }
        // A cache's stores, atomics and fences (decision 0353 step 4b): of the tape's family, at offsets from `t`.
        Opcode::Store | Opcode::Istore8 | Opcode::Istore16 | Opcode::Istore32 => {
            let bits = match op {
                Opcode::Istore8 => 8,
                Opcode::Istore16 => 16,
                Opcode::Istore32 => 32,
                _ => dfg.value_type(dfg.inst_args(inst)[0]).bits(),
            };
            format!("ukl_stc_u{bits}(clif_written(t),{},((uint64_t)({})));", wide_addr(&args[1]), args[0])
        }
        Opcode::AtomicLoad => set(format!("ukl_atomic_load_at(t,{})", wide_addr(&args[0]))),
        Opcode::AtomicStore => format!("ukl_atomic_store_at(t,{},((uint32_t)({})));", wide_addr(&args[1]), args[0]),
        Opcode::AtomicRmw => {
            let InstructionData::AtomicRmw { op: ir::AtomicRmwOp::Add, .. } = data else { return Err("an atomic other than an add".into()) };
            set(format!("ukl_atomic_add_at(t,{},((uint32_t)({})))", wide_addr(&args[0]), args[1]))
        }
        Opcode::AtomicCas => set(format!("ukl_atomic_cas_at(t,{},((uint32_t)({})),((uint32_t)({})))", wide_addr(&args[0]), args[1], args[2])),
        Opcode::Fence => "ukl_fence_device();".to_owned(),
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
