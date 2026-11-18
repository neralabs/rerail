//! Cranelift IR as Rust source (decision 0310: moved here from `rerail_compiler::clif_rust`): the
//! parser's, for an ahead-of-time build of the same code, each function of the JIT's, tier 1's
//! (`CompiledBlock::ir`) and tier 2's (`CompiledStream::ir`), as Cranelift is given it, a Rust
//! function, compiled by rustc and LLVM, so the two builds differ only in the compiler; and the
//! tape's reads (`reads`), for the tape crate.
//!
//! A function's graph is structured control (`Shape`); each value a local, a block's
//! parameters set where a branch goes to it. A vector is NEON's (`uint8x16_t`), each vector
//! operation its intrinsic. Cranelift's pinned register (`ctx`, `get_pinned_reg`) is a
//! parameter of every function, passed on at each call, as a method's `self`. Each function is
//! `#[inline(never)]`, so the calls are the JIT's: a function a rule, words as calls.

use cranelift_codegen::ir::{self, InstructionData, Opcode, condcodes::IntCC, types};
use std::fmt::Write;

/// A cold block's first line. `cold_path` makes the branch to it unlikely; the empty `asm!`
/// (no instruction, no memory, flags kept) keeps it a branch. Without it LLVM folded small
/// cold arms into `csel`s, so the hot way waited on the cold way's inputs: the refill's next
/// block on the block's last token's end, every block's loads after its token chain (0177,
/// "The next block waits on no token"), where the compiler's IR has a branch the core predicts.
const COLD: &str = "core::hint::cold_path(); unsafe { core::arch::asm!(\"\", options(nomem, nostack, preserves_flags)) };\n";


/// A fingerprint of a text (FNV-1a). The generated source carries its own (`SOURCE`): the
/// ahead-of-time build runs only where the JIT's program translates to it again.
pub fn fingerprint(clif: &str) -> u64 {
    clif.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// Rust's unsigned type for a Cranelift integer type, and its width.
fn ty(t: ir::Type) -> Result<(&'static str, &'static str, u32), String> {
    Ok(match t {
        types::I8 => ("u8", "i8", 8),
        types::I16 => ("u16", "i16", 16),
        types::I32 => ("u32", "i32", 32),
        types::I64 => ("u64", "i64", 64),
        t => return Err(format!("a value of type {t}")),
    })
}

/// A local's Rust type for a Cranelift type, and its first value.
fn local(t: ir::Type) -> Result<(&'static str, &'static str), String> {
    Ok(match t {
        types::I8X16 => ("uint8x16_t", "unsafe { core::mem::zeroed() }"),
        types::I64X2 => ("uint64x2_t", "unsafe { core::mem::zeroed() }"),
        types::I16X8 => ("uint16x8_t", "unsafe { core::mem::zeroed() }"),
        t => (ty(t)?.0, "0"),
    })
}

thread_local! {
    /// The constants that are addresses of the JIT's process (`translate`'s `addresses`).
    static ADDRESSES: std::cell::RefCell<Vec<(i64, i64, String, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The functions' style: `None` the parser's (`translate`), else the reads' (`translate_reads`), each
    /// function's inlining by its id.
    static READS: std::cell::RefCell<Option<Vec<(u32, bool)>>> = const { std::cell::RefCell::new(None) };
}

/// Whether the functions are the reads' (`translate_reads`): no pinned register; and function `id`'s
/// attribute.
fn read_style(id: u32) -> Option<&'static str> {
    READS.with(|r| r.borrow().as_ref().map(|hot| if hot.iter().any(|&(f, h)| f == id && h) { "#[inline(always)]" } else { "#[inline(never)]" }))
}

/// The tape's reads as Rust (decision 0310), for the tape crate: `funcs` by their user name's index, each
/// with whether it is hot, the JIT's mark (a hot one `#[inline(always)]`, as the JIT's pass inlines it, a
/// cold one `#[inline(never)]`); each of `entries` a safe public function of its name, over the tape's bytes
/// and its view. A load of the tape (one not marked aligned: the view's fields are) is checked against the
/// tape's bytes, as the slices a hand-written reader indexes: a corrupt tape panics where it is read. Each
/// function takes the bytes' length first, where the parser's take the pinned register. No addresses, no
/// vectors: plain Rust for every target.
pub fn translate_reads(funcs: &[(u32, ir::Function, bool)], entries: &[(u32, &str)], family: crate::reads::Family) -> Result<String, String> {
    READS.with(|r| *r.borrow_mut() = Some(funcs.iter().map(|(id, _, hot)| (*id, *hot)).collect()));
    ADDRESSES.with(|a| a.borrow_mut().clear());
    let result = (|| {
        let mut out = String::from(
            "/// A load of `T` at address `at` of the tape that starts at `base`, `len` bytes long.\n\
             #[inline(always)]\n\
             fn ld<T: Copy>(base: u64, len: u64, at: usize) -> T {\n    \
                 let off = (at as u64).wrapping_sub(base);\n    \
                 if off > len || len - off < core::mem::size_of::<T>() as u64 {\n        \
                     past();\n    \
                 }\n    \
                 // SAFETY: the load lies in the tape's bytes, as just checked.\n    \
                 unsafe { core::ptr::read_unaligned(at as *const T) }\n\
             }\n\n\
             #[cold]\n\
             #[inline(never)]\n\
             fn past() -> ! {\n    \
                 panic!(\"a read past the tape's bytes: a corrupt tape\")\n\
             }\n\n",
        );
        for (id, f, _) in funcs {
            function(&mut out, *id, f).map_err(|e| format!("u0:{id}: {e}"))?;
        }
        for &(entry, n) in entries {
            let f = &funcs.iter().find(|(id, ..)| *id == entry).ok_or("no entry")?.1;
            let types: Vec<&str> = f.signature.params.iter().map(|p| ty(p.value_type).map(|t| t.0)).collect::<Result<_, _>>()?;
            if family == crate::reads::Family::Relation {
                // A relation's read: its descriptor's address, every load an aligned word of its block (`ld`'s
                // check is the tape's alone).
                if types.first() != Some(&"u64") {
                    return Err(format!("{n}: a relation's read takes its descriptor first"));
                }
                let params: Vec<String> = types.iter().enumerate().skip(1).map(|(i, t)| format!("a{i}: {t}")).collect();
                let args: Vec<String> = (1..types.len()).map(|i| format!(", a{i}")).collect();
                let [ret] = f.signature.returns.as_slice() else { return Err(format!("{n}: a read returns one value")) };
                let ret = ty(ret.value_type)?.0;
                writeln!(
                    out,
                    "/// The read `{n}` of the relation (or rows) whose descriptor is at `desc`.\n///\n/// # Safety\n/// `desc` is a live descriptor of its kind in a block, and the read's arguments are in its bounds.\n#[inline(always)]\npub unsafe fn {n}(desc: *const u8{}{}) -> {ret} {{\n    // SAFETY: the caller's.\n    unsafe {{ f{entry}(u64::MAX, desc as u64{}) }}\n}}\n",
                    if params.is_empty() { "" } else { ", " },
                    params.join(", "),
                    args.concat()
                )
                .unwrap();
                continue;
            }
            if types.get(..2) != Some(&["u64", "u64"][..]) {
                return Err(format!("{n}: a read takes the tape and its view first"));
            }
            let params: Vec<String> = types.iter().enumerate().skip(2).map(|(i, t)| format!("a{i}: {t}")).collect();
            let args: Vec<String> = (2..types.len()).map(|i| format!(", a{i}")).collect();
            let [ret] = f.signature.returns.as_slice() else { return Err(format!("{n}: a read returns one value")) };
            let ret = ty(ret.value_type)?.0;
            writeln!(
                out,
                "/// The read `{n}` of the tape in `tape`, whose `view`'s offsets count from its first byte.\n#[inline(always)]\npub fn {n}(tape: &[u8], view: &rerail_clif::view::TapeView, {}) -> {ret} {{\n    // SAFETY: each load of the view is of its fields; each of the tape is checked against `tape` (`ld`).\n    unsafe {{ f{entry}(tape.len() as u64, tape.as_ptr() as u64, core::ptr::from_ref(view) as u64{}) }}\n}}\n",
                params.join(", "),
                args.concat()
            )
            .unwrap();
        }
        Ok(out)
    })();
    READS.with(|r| *r.borrow_mut() = None);
    result
}

/// The functions, by their user name's index (`u0:N`, the JIT's `FuncId`), as Rust: each of
/// `entries` an `extern "C"` function of its name, its parameters `u64`s, calling it with the
/// pinned register 0. Each of `addresses` (its start, its bytes): with its words, data of its
/// name (`{name}_DATA`), a constant inside it that data's address and the offset; else a
/// static of its name, which the build sets to the running process's address.
pub fn translate(funcs: &[(u32, ir::Function)], entries: &[(u32, &str)], addresses: &[(i64, i64, &str, Option<&[u64]>)]) -> Result<String, String> {
    ADDRESSES.with(|a| *a.borrow_mut() = addresses.iter().map(|&(v, len, n, d)| (v, len, n.to_owned(), d.is_some())).collect());
    let mut out = String::new();
    out.push_str("// Generated by rerail_clif::rust from the parser's Cranelift IR. Do not edit.\n");
    out.push_str("#![allow(unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_imports, clippy::all)]\n\n");
    out.push_str("use core::arch::aarch64::*;\n\n");
    for (_, _, n, data) in addresses {
        match data {
            Some(words) => {
                let list: Vec<String> = words.iter().map(|w| format!("{w:#x}")).collect();
                writeln!(out, "/// The tables the code reads, as the JIT's (`clif_rust::rust_source`).\npub static {n}_DATA: [u64; {}] = [{}];\n", words.len(), list.join(", ")).unwrap();
            }
            None => writeln!(out, "/// An address of the running process's (set before the first call).\npub static {n}: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);\n").unwrap(),
        }
    }
    let mut structured = 0;
    let mut bodies = String::new();
    for (id, f) in funcs {
        structured += usize::from(function(&mut bodies, *id, f).map_err(|e| format!("u0:{id}: {e}"))?);
    }
    // The stack pointer's read, where a function takes it (`GetStackPointer`).
    if bodies.contains("sp()") {
        out.push_str("#[inline(always)]\nfn sp() -> u64 {\n    let s: u64;\n    // SAFETY: reads the stack pointer.\n    unsafe { core::arch::asm!(\"mov {}, sp\", out(reg) s, options(nomem, nostack, preserves_flags)) };\n    s\n}\n\n");
    }
    // The wide constants of the functions' entries (`instruction`'s `Iconst`): each made once,
    // a value the function keeps, not one made again at each use.
    out.push_str(&bodies);
    writeln!(out, "/// The functions whose graph is structured control (the rest irreducible, a loop over a `match`).\npub const STRUCTURED: (usize, usize) = ({structured}, {});\n", funcs.len()).unwrap();
    for &(entry, n) in entries {
        let f = &funcs.iter().find(|(id, _)| *id == entry).ok_or("no entry")?.1;
        let params: Vec<String> = (0..f.signature.params.len()).map(|i| format!("a{i}: u64")).collect();
        let args: Vec<String> = (0..f.signature.params.len()).map(|i| format!("a{i}")).collect();
        let ret = match f.signature.returns.len() {
            0 => "",
            1 => " -> u64",
            _ => return Err("an entry with two returns".into()),
        };
        writeln!(out, "/// An entry, as the JIT's.\n///\n/// # Safety\n/// As the JIT's entry.\npub unsafe extern \"C\" fn {n}({}){ret} {{\n    unsafe {{ f{entry}(0, {}) }}\n}}\n", params.join(", "), args.join(", ")).unwrap();
    }
    Ok(out)
}

/// One function; whether its graph was structured (else a loop over a `match`).
fn function(out: &mut String, id: u32, f: &ir::Function) -> Result<bool, String> {
    let dfg = &f.dfg;
    let name = |v: ir::Value| format!("v{}", dfg.resolve_aliases(v).as_u32());
    let rets: Vec<&str> = f.signature.returns.iter().map(|p| local(p.value_type).map(|t| t.0)).collect::<Result<_, _>>()?;
    let ret_ty = match rets.len() {
        0 => "()".to_string(),
        1 => rets[0].to_string(),
        _ => format!("({})", rets.join(", ")),
    };
    let entry = f.layout.entry_block().ok_or("no entry block")?;
    let params: Vec<String> = dfg.block_params(entry).iter().map(|&v| Ok(format!("a{}: {}", v.as_u32(), local(dfg.value_type(v))?.0))).collect::<Result<_, String>>()?;
    match read_style(id) {
        Some(inline) => writeln!(out, "{inline}\nunsafe fn f{id}(len: u64, {}) -> {ret_ty} {{", params.join(", ")).unwrap(),
        None => {
            writeln!(out, "#[inline(never)]\nunsafe fn f{id}(pin: u64{}{}) -> {ret_ty} {{", if params.is_empty() { "" } else { ", " }, params.join(", ")).unwrap();
            out.push_str("    let mut pin = pin;\n");
        }
    }
    let absorbed = absorbed(f);
    // Every value a local, declared once.
    let mut declared = std::collections::BTreeMap::new();
    for b in f.layout.blocks() {
        for &v in dfg.block_params(b) {
            declared.insert(v.as_u32(), local(dfg.value_type(v))?);
        }
        for inst in f.layout.block_insts(b).filter(|i| !absorbed.contains(i)) {
            for &v in dfg.inst_results(inst) {
                declared.insert(v.as_u32(), local(dfg.value_type(v))?);
            }
        }
    }
    for (v, (t, zero)) in &declared {
        writeln!(out, "    let mut v{v}: {t} = {zero};").unwrap();
    }
    // Each stack slot a local, in words.
    for (ss, data) in f.sized_stack_slots.iter() {
        writeln!(out, "    let mut ss{}: [u64; {}] = [0; {}];", ss.as_u32(), data.size.div_ceil(8), data.size.div_ceil(8)).unwrap();
    }
    for &v in dfg.block_params(entry) {
        writeln!(out, "    v{0} = a{0};", v.as_u32()).unwrap();
    }
    let blocks: Vec<ir::Block> = std::iter::once(entry).chain(f.layout.blocks().filter(|&b| b != entry)).collect();
    let node_of: std::collections::HashMap<ir::Block, usize> = blocks.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let first: Vec<Vec<usize>> = blocks.iter().map(|&b| successors(f, b).iter().map(|t| node_of[t]).collect()).collect();
    let sizes: Vec<usize> = blocks.iter().map(|&b| f.layout.block_insts(b).count()).collect();
    if let Some(shape) = Shape::of(first, &sizes) {
        out.push_str("    let mut d: u32 = 0;\n");
        let emit = |node: usize, jump: &dyn Fn(usize) -> Result<String, String>| -> Result<String, String> {
            let mut s = String::new();
            // A cold block (`set_cold_block`) says so to rustc, as Cranelift lays it out: the
            // branch to it unlikely, its code out of the hot path, and no register kept for it
            // where the hot path wants one (`COLD`).
            if f.layout.is_cold(blocks[node]) {
                s.push_str(COLD);
            }
            for inst in f.layout.block_insts(blocks[node]).filter(|i| !absorbed.contains(i)) {
                s.push_str(&instruction(f, inst, &name, &|k, _| jump(k))?);
                s.push('\n');
            }
            Ok(s)
        };
        let body = shape.tree(&emit, 0)?;
        out.push_str(&body);
        out.push_str("}\n\n");
        return Ok(true);
    }
    // An irreducible graph: a loop over a `match` on the block.
    writeln!(out, "    let mut b: u32 = {};\n    loop {{\n        match b {{", entry.as_u32()).unwrap();
    for blk in f.layout.blocks() {
        writeln!(out, "            {} => {{", blk.as_u32()).unwrap();
        if f.layout.is_cold(blk) {
            out.push_str("                ");
            out.push_str(COLD);
        }
        for inst in f.layout.block_insts(blk).filter(|i| !absorbed.contains(i)) {
            let s = instruction(f, inst, &name, &|_, to: ir::Block| Ok(format!("b = {}; continue;", to.as_u32())))?;
            for line in s.lines() {
                writeln!(out, "                {line}").unwrap();
            }
        }
        out.push_str("            }\n");
    }
    out.push_str("            // SAFETY: every block's number is a case above.\n            _ => unsafe { core::hint::unreachable_unchecked() },\n        }\n    }\n}\n\n");
    Ok(false)
}

/// A function's graph as Rust's structured control ("Beyond Relooper", Ramsey 2022, as
/// WebAssembly's backends do): a loop header a labelled `loop`, a node two forward edges reach (a
/// merge) the code after a labelled block its branches `break` out of, any other node inline at
/// its one branch. Each branch a direct jump.
///
/// The graph's nodes are its blocks and copies of them: where an edge goes back to a node that
/// does not dominate its source (an irreducible loop, a loop entered at two places), the edge
/// gets its own copy of its target (node splitting), until none does. A copy runs the block's
/// code: each value is a local, which its last definition set, as SSA's dominance requires.
pub struct Shape {
    /// Each node's block (its place in the graph given) and its successors (nodes), in the
    /// terminator's order.
    block: Vec<Option<usize>>,
    succ: Vec<Vec<usize>>,
    /// The edges into a dispatch node (`None` its block): their target's place in its successors.
    label: std::collections::HashMap<(usize, usize), usize>,
    /// Each node's place in reverse postorder (`usize::MAX` unreachable).
    rpo: Vec<usize>,
    /// The merge nodes each node immediately dominates, the latest in the order first.
    merges: Vec<Vec<usize>>,
    header: Vec<bool>,
    merge: Vec<bool>,
}

fn successors(f: &ir::Function, b: ir::Block) -> Vec<ir::Block> {
    let Some(last) = f.layout.last_inst(b) else { return Vec::new() };
    f.dfg.insts[last].branch_destination(&f.dfg.jump_tables, &f.dfg.exception_tables).iter().map(|c| c.block(&f.dfg.value_lists)).collect()
}

/// The graph's order, dominators and loops: reverse postorder, each node's immediate dominator
/// (by place), its predecessors; or the first edge back to a node that does not dominate it.
struct Order {
    order: Vec<usize>,
    rpo: Vec<usize>,
    idom: Vec<usize>,
    preds: Vec<Vec<usize>>,
}

impl Order {
    fn of(succ: &[Vec<usize>]) -> Order {
        let n = succ.len();
        let mut post = Vec::new();
        let mut seen = vec![false; n];
        let mut stack = vec![(0usize, 0usize)];
        seen[0] = true;
        while let Some(top) = stack.last_mut() {
            let b = top.0;
            if let Some(&t) = succ[b].get(top.1) {
                top.1 += 1;
                if !seen[t] {
                    seen[t] = true;
                    stack.push((t, 0));
                }
            } else {
                post.push(b);
                stack.pop();
            }
        }
        let order: Vec<usize> = post.into_iter().rev().collect();
        let mut rpo = vec![usize::MAX; n];
        for (i, &b) in order.iter().enumerate() {
            rpo[b] = i;
        }
        let mut preds = vec![Vec::new(); n];
        for &b in &order {
            for &t in &succ[b] {
                preds[t].push(b);
            }
        }
        // Immediate dominators (Cooper, Harvey, Kennedy), by place in the order.
        let mut idom = vec![usize::MAX; order.len()];
        idom[0] = 0;
        let mut changed = true;
        while changed {
            changed = false;
            for i in 1..order.len() {
                let mut new = usize::MAX;
                for &p in &preds[order[i]] {
                    let mut q = rpo[p];
                    if idom[q] == usize::MAX {
                        continue;
                    }
                    if new == usize::MAX {
                        new = q;
                        continue;
                    }
                    let mut a = new;
                    while a != q {
                        while a > q {
                            a = idom[a];
                        }
                        while q > a {
                            q = idom[q];
                        }
                    }
                    new = a;
                }
                if idom[i] != new {
                    idom[i] = new;
                    changed = true;
                }
            }
        }
        Order { order, rpo, idom, preds }
    }

    fn dominates(&self, a: usize, mut b: usize) -> bool {
        loop {
            if a == b {
                return true;
            }
            if b == 0 {
                return false;
            }
            b = self.idom[b];
        }
    }

    /// An edge (its source node and its place in the source's successors) back to a node that
    /// does not dominate its source.
    fn irreducible(&self, succ: &[Vec<usize>]) -> Option<(usize, usize)> {
        for &x in &self.order {
            for (k, &t) in succ[x].iter().enumerate() {
                if self.rpo[t] <= self.rpo[x] && !self.dominates(self.rpo[t], self.rpo[x]) {
                    return Some((x, k));
                }
            }
        }
        None
    }
}

impl Shape {
    /// The shape of `f`: its nodes split until it is reducible, within its own size again in
    /// copies; past that, each loop with two entries given one, a dispatch.
    /// A graph's nodes' successors, the entry node 0, and each node's size (its copies' cost).
    pub fn of(first: Vec<Vec<usize>>, sizes: &[usize]) -> Option<Shape> {
        let size = |b: usize| sizes[b];
        let budget: usize = sizes.iter().sum();
        let (mut block, mut succ): (Vec<Option<usize>>, _) = ((0..first.len()).map(Some).collect(), first.clone());
        let mut copied = 0;
        let mut label = std::collections::HashMap::new();
        let order = loop {
            let order = Order::of(&succ);
            let Some((x, k)) = order.irreducible(&succ) else { break order };
            let t = succ[x][k];
            copied += block[t].map_or(0, size);
            if copied > budget {
                (block, succ, label) = Self::dispatched((0..first.len()).map(Some).collect(), first.clone())?;
                let order = Order::of(&succ);
                if order.irreducible(&succ).is_some() {
                    return None;
                }
                break order;
            }
            block.push(block[t]);
            succ.push(succ[t].clone());
            succ[x][k] = block.len() - 1;
        };
        let n = block.len();
        let (mut header, mut merge) = (vec![false; n], vec![false; n]);
        for &b in &order.order {
            let i = order.rpo[b];
            let forward = order.preds[b].iter().filter(|&&p| order.rpo[p] < i).count();
            header[b] = order.preds[b].iter().any(|&p| order.rpo[p] >= i);
            merge[b] = forward >= 2;
        }
        let mut merges = vec![Vec::new(); n];
        for &b in order.order.iter().skip(1) {
            if merge[b] {
                merges[order.order[order.idom[order.rpo[b]]]].push(b);
            }
        }
        for m in &mut merges {
            m.sort_by_key(|&b| std::cmp::Reverse(order.rpo[b]));
        }
        Some(Shape { block, succ, label, rpo: order.rpo, merges, header, merge })
    }

    /// Each loop (a strongly connected part of the graph, its header out, then the parts within,
    /// as a loop forest) entered at two nodes or more given a dispatch node, its one entry: the
    /// edges into those nodes go to the dispatch, each with its target's place (`label`), the
    /// dispatch on to it. `None` where the function's first block is one of two entries.
    #[allow(clippy::type_complexity)]
    fn dispatched(mut block: Vec<Option<usize>>, mut succ: Vec<Vec<usize>>) -> Option<(Vec<Option<usize>>, Vec<Vec<usize>>, std::collections::HashMap<(usize, usize), usize>)> {
        let mut label = std::collections::HashMap::new();
        let mut out = vec![false; block.len()];
        loop {
            let parts = strong_parts(&succ, &out);
            if parts.is_empty() {
                return Some((block, succ, label));
            }
            for part in parts {
                let inside: std::collections::HashSet<usize> = part.iter().copied().collect();
                let mut entries: Vec<usize> = Vec::new();
                for (u, next) in succ.iter().enumerate() {
                    for &t in next {
                        if inside.contains(&t) && !inside.contains(&u) && !entries.contains(&t) {
                            entries.push(t);
                        }
                    }
                }
                if inside.contains(&0) && !entries.contains(&0) {
                    entries.push(0);
                }
                if entries.len() == 1 {
                    out[entries[0]] = true;
                    continue;
                }
                if entries.contains(&0) {
                    return None;
                }
                let d = block.len();
                for (u, next) in succ.iter_mut().enumerate() {
                    for (k, t) in next.iter_mut().enumerate() {
                        if let Some(j) = entries.iter().position(|&e| e == *t) {
                            label.insert((u, k), j);
                            *t = d;
                        }
                    }
                }
                block.push(None);
                succ.push(entries);
                out.push(true);
            }
        }
    }

    /// The code of node `x` and the nodes it dominates: each node's own code by `emit`, given
    /// the node (its place in the graph given) and its branches' code (`jump(k)`, its `k`th
    /// successor's).
    pub fn tree(&self, emit: Emit<'_>, x: usize) -> Result<String, String> {
        let within = self.within(emit, x, &self.merges[x])?;
        Ok(if self.header[x] { format!("'b{x}: loop {{\n{within}}}\n") } else { within })
    }

    /// Node `x`'s `k`th edge: its dispatch's label, then a jump.
    fn transfer(&self, emit: Emit<'_>, x: usize, k: usize) -> Result<String, String> {
        let to = self.succ[x][k];
        let jump = if self.rpo[to] <= self.rpo[x] {
            format!("continue 'b{to};")
        } else if self.merge[to] {
            format!("break 'm{to};")
        } else {
            self.tree(emit, to)?
        };
        Ok(match self.label.get(&(x, k)) {
            Some(j) => format!("d = {j}; {jump}"),
            None => jump,
        })
    }

    fn within(&self, emit: Emit<'_>, x: usize, ys: &[usize]) -> Result<String, String> {
        if let Some((&y, rest)) = ys.split_first() {
            let inner = self.within(emit, x, rest)?;
            let after = self.tree(emit, y)?;
            return Ok(format!("'m{y}: {{\n{inner}}}\n{after}"));
        }
        let Some(b) = self.block[x] else {
            let mut s = String::from("match d {\n");
            for k in 0..self.succ[x].len() {
                writeln!(s, "{k} => {{ {} }}", self.transfer(emit, x, k)?).unwrap();
            }
            s.push_str("// SAFETY: a dispatch's label is one of its entries.\n_ => unsafe { core::hint::unreachable_unchecked() },\n}\n");
            return Ok(s);
        };
        emit(b, &|k| self.transfer(emit, x, k))
    }
}

/// A node's own code (`Shape::tree`): the node, and its branches' code.
pub type Emit<'a> = &'a dyn Fn(usize, &dyn Fn(usize) -> Result<String, String>) -> Result<String, String>;

/// The graph's strongly connected parts with a cycle, the nodes `out` left out (iterative
/// Tarjan).
fn strong_parts(succ: &[Vec<usize>], out: &[bool]) -> Vec<Vec<usize>> {
    let n = succ.len();
    let (mut index, mut low, mut on) = (vec![usize::MAX; n], vec![0; n], vec![false; n]);
    let (mut stack, mut parts, mut next) = (Vec::new(), Vec::new(), 0);
    for root in 0..n {
        if out[root] || index[root] != usize::MAX {
            continue;
        }
        let mut work = vec![(root, 0usize)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on[root] = true;
        while let Some(top) = work.last_mut() {
            let v = top.0;
            if let Some(&w) = succ[v].get(top.1) {
                top.1 += 1;
                if out[w] {
                    continue;
                }
                if index[w] == usize::MAX {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on[w] = true;
                    work.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                work.pop();
                if let Some(&(u, _)) = work.last() {
                    low[u] = low[u].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut part = Vec::new();
                    while let Some(w) = stack.pop() {
                        on[w] = false;
                        part.push(w);
                        if w == v {
                            break;
                        }
                    }
                    if part.len() > 1 || succ[v].contains(&v) {
                        parts.push(part);
                    }
                }
            }
        }
    }
    parts
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
        write!(s, "let t = ({}); ", args.join(", ")).unwrap();
        for (i, p) in params.iter().enumerate() {
            write!(s, "v{} = t.{i}; ", p.as_u32()).unwrap();
        }
    }
    s.push_str(&jump(k, to)?);
    s.push_str(" }");
    Ok(s)
}

fn cond(cc: IntCC, a: &str, b: &str, s: &str, u: &str) -> String {
    let (sig, op) = match cc {
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
    if sig { format!("(({a} as {s}) {op} ({b} as {s})) as u8") } else { format!("(({a} as {u}) {op} ({b} as {u})) as u8") }
}

/// Whether call `inst` is followed by a return of its results, in order, and nothing else:
/// `return f(…)`.
fn returned_call(f: &ir::Function, inst: ir::Inst) -> bool {
    let dfg = &f.dfg;
    if dfg.insts[inst].opcode() != ir::Opcode::Call {
        return false;
    }
    f.layout.next_inst(inst).is_some_and(|r| dfg.insts[r].opcode() == ir::Opcode::Return && dfg.inst_args(r) == dfg.inst_results(inst) && !dfg.inst_results(inst).is_empty())
}

fn instruction(f: &ir::Function, inst: ir::Inst, name: &dyn Fn(ir::Value) -> String, jump: &dyn Fn(usize, ir::Block) -> Result<String, String>) -> Result<String, String> {
    let dfg = &f.dfg;
    let data = &dfg.insts[inst];
    let op = data.opcode();
    let args: Vec<String> = dfg.inst_args(inst).iter().map(|&v| name(v)).collect();
    let results = dfg.inst_results(inst);
    let r = results.first().map(|&v| name(v));
    let rt = results.first().map(|&v| dfg.value_type(v));
    let at = dfg.inst_args(inst).first().map(|&v| dfg.value_type(v));
    let set = |e: String| -> String { format!("{} = {e};", r.clone().expect("a result")) };
    let (u, s, bits) = rt.and_then(|t| ty(t).ok()).unwrap_or(("u64", "i64", 64));
    let imm = || -> Result<i64, String> {
        match data {
            InstructionData::UnaryImm { imm, .. } | InstructionData::BinaryImm64 { imm, .. } | InstructionData::IntCompareImm { imm, .. } => Ok(imm.bits()),
            _ => Err(format!("{op}: no immediate")),
        }
    };
    let addr = |p: &str| -> String {
        let off = data.load_store_offset().unwrap_or(0);
        if off == 0 { format!("({p} as usize)") } else { format!("(({p} as usize).wrapping_add({off}isize as usize))") }
    };
    let vector = |t: Option<ir::Type>| t.is_some_and(|t| t.is_vector());
    if vector(rt) || (vector(at) && !matches!(op, Opcode::Brif | Opcode::Jump | Opcode::Return | Opcode::Call)) {
        return vector_instruction(f, inst, op, &args, rt, at, &set, &addr);
    }
    let slot = |ss: &ir::StackSlot, offset: i64| format!("((ss{}.as_mut_ptr() as usize).wrapping_add({offset}isize as usize))", ss.as_u32());
    Ok(match op {
        Opcode::Iconst => match ADDRESSES.with(|a| {
            let c = imm().unwrap_or(0);
            a.borrow().iter().find(|(v, len, _, _)| (*v..*v + *len).contains(&c)).map(|(v, _, n, d)| (n.clone(), c - v, *d))
        }) {
            Some((n, off, true)) => set(format!("(core::ptr::addr_of!({n}_DATA) as u64).wrapping_add({off})")),
            Some((n, 0, false)) => set(format!("{n}.load(core::sync::atomic::Ordering::Relaxed)")),
            Some((n, off, false)) => set(format!("{n}.load(core::sync::atomic::Ordering::Relaxed).wrapping_add({off})")),
            None => set(format!("{}i64 as {u}", imm()?)),
        },
        Opcode::BandNot => set(format!("{} & !{}", args[0], args[1])),
        Opcode::BorNot => set(format!("{} | !{}", args[0], args[1])),
        Opcode::BxorNot => set(format!("{} ^ !{}", args[0], args[1])),
        Opcode::Bitrev => set(format!("{}.reverse_bits()", args[0])),
        Opcode::UaddOverflow => format!("{{ let t = {}.overflowing_add({}); {} = t.0; {} = t.1 as u8; }}", args[0], args[1], name(results[0]), name(results[1])),
        Opcode::StackAddr => {
            let InstructionData::StackLoad { stack_slot, offset, .. } = data else { return Err("a stack address of no slot".into()) };
            set(format!("{} as u64", slot(stack_slot, i64::from(i32::from(*offset)))))
        }
        Opcode::StackLoad => {
            let InstructionData::StackLoad { stack_slot, offset, .. } = data else { return Err("a stack load of no slot".into()) };
            set(format!("unsafe {{ core::ptr::read_unaligned({} as *const {u}) }}", slot(stack_slot, i64::from(i32::from(*offset)))))
        }
        Opcode::StackStore => {
            let InstructionData::StackStore { stack_slot, offset, .. } = data else { return Err("a stack store of no slot".into()) };
            let vt = ty(at.ok_or("a store of nothing")?)?.0;
            format!("unsafe {{ core::ptr::write_unaligned({} as *mut {vt}, {}) }};", slot(stack_slot, i64::from(i32::from(*offset))), args[0])
        }
        Opcode::Iadd => set(format!("{}.wrapping_add({})", args[0], args[1])),
        Opcode::Isub => set(format!("{}.wrapping_sub({})", args[0], args[1])),
        Opcode::Ineg => set(format!("{}.wrapping_neg()", args[0])),
        Opcode::Imul => set(format!("{}.wrapping_mul({})", args[0], args[1])),
        Opcode::Band => set(format!("{} & {}", args[0], args[1])),
        Opcode::Bor => set(format!("{} | {}", args[0], args[1])),
        Opcode::Bxor => set(format!("{} ^ {}", args[0], args[1])),
        Opcode::Bnot => set(format!("!{}", args[0])),
        Opcode::IaddImm => set(format!("{}.wrapping_add({}i64 as {u})", args[0], imm()?)),
        Opcode::ImulImm => set(format!("{}.wrapping_mul({}i64 as {u})", args[0], imm()?)),
        // A divisor that is a constant and not zero (Cranelift's verifier refuses a zero one).
        Opcode::UdivImm => set(format!("{} / ({}i64 as {u})", args[0], imm()?)),
        Opcode::IrsubImm => set(format!("({}i64 as {u}).wrapping_sub({})", imm()?, args[0])),
        Opcode::BandImm => set(format!("{} & ({}i64 as {u})", args[0], imm()?)),
        Opcode::BorImm => set(format!("{} | ({}i64 as {u})", args[0], imm()?)),
        Opcode::BxorImm => set(format!("{} ^ ({}i64 as {u})", args[0], imm()?)),
        Opcode::Ishl => set(format!("{}.wrapping_shl({} as u32)", args[0], args[1])),
        Opcode::Ushr => set(format!("{}.wrapping_shr({} as u32)", args[0], args[1])),
        Opcode::Sshr => set(format!("(({} as {s}).wrapping_shr({} as u32)) as {u}", args[0], args[1])),
        Opcode::IshlImm => set(format!("{}.wrapping_shl({})", args[0], (imm()? as u32) % bits)),
        Opcode::UshrImm => set(format!("{}.wrapping_shr({})", args[0], (imm()? as u32) % bits)),
        Opcode::SshrImm => set(format!("(({} as {s}).wrapping_shr({})) as {u}", args[0], (imm()? as u32) % bits)),
        Opcode::Popcnt => set(format!("{}.count_ones() as {u}", args[0])),
        Opcode::Clz => set(format!("{}.leading_zeros() as {u}", args[0])),
        Opcode::Ctz => set(format!("{}.trailing_zeros() as {u}", args[0])),
        Opcode::Umin => set(format!("{}.min({})", args[0], args[1])),
        Opcode::Umax => set(format!("{}.max({})", args[0], args[1])),
        Opcode::Smin => set(format!("(({} as {s}).min({} as {s})) as {u}", args[0], args[1])),
        Opcode::Smax => set(format!("(({} as {s}).max({} as {s})) as {u}", args[0], args[1])),
        Opcode::Icmp | Opcode::IcmpImm => {
            let cc = data.cond_code().ok_or("a compare with no condition")?;
            let at = ty(dfg.value_type(dfg.inst_args(inst)[0]))?;
            let b = if op == Opcode::IcmpImm { format!("({}i64 as {})", imm()?, at.0) } else { args[1].clone() };
            set(cond(cc, &args[0], &b, at.1, at.0))
        }
        Opcode::Select => set(format!("if {} != 0 {{ {} }} else {{ {} }}", args[0], args[1], args[2])),
        Opcode::Uextend | Opcode::Ireduce | Opcode::Bitcast => set(format!("{} as {u}", args[0])),
        Opcode::Sextend => {
            let from = ty(dfg.value_type(dfg.inst_args(inst)[0]))?;
            set(format!("({} as {} as {s}) as {u}", args[0], from.1))
        }
        // A read's load of the tape: checked against its bytes (`translate_reads`).
        Opcode::Load | Opcode::Uload8 | Opcode::Uload16 | Opcode::Uload32 if read_style(u32::MAX).is_some() && !data.memflags().is_some_and(|m| m.aligned()) => {
            let base = name(dfg.block_params(f.layout.entry_block().ok_or("no entry block")?)[0]);
            let t = match op {
                Opcode::Uload8 => "u8",
                Opcode::Uload16 => "u16",
                Opcode::Uload32 => "u32",
                _ => u,
            };
            set(format!("ld::<{t}>({base}, len, {}) as {u}", addr(&args[0])))
        }
        Opcode::Load => set(format!("unsafe {{ core::ptr::read_unaligned({} as *const {u}) }}", addr(&args[0]))),
        Opcode::Uload8 => set(format!("unsafe {{ core::ptr::read_unaligned({} as *const u8) }} as {u}", addr(&args[0]))),
        Opcode::Uload16 => set(format!("unsafe {{ core::ptr::read_unaligned({} as *const u16) }} as {u}", addr(&args[0]))),
        Opcode::Uload32 => set(format!("unsafe {{ core::ptr::read_unaligned({} as *const u32) }} as {u}", addr(&args[0]))),
        Opcode::Sload8 => set(format!("unsafe {{ core::ptr::read_unaligned({} as *const i8) }} as {s} as {u}", addr(&args[0]))),
        Opcode::Sload16 => set(format!("unsafe {{ core::ptr::read_unaligned({} as *const i16) }} as {s} as {u}", addr(&args[0]))),
        Opcode::Sload32 => set(format!("unsafe {{ core::ptr::read_unaligned({} as *const i32) }} as {s} as {u}", addr(&args[0]))),
        Opcode::Store => {
            let vt = ty(dfg.value_type(dfg.inst_args(inst)[0]))?.0;
            format!("unsafe {{ core::ptr::write_unaligned({} as *mut {vt}, {}) }};", addr(&args[1]), args[0])
        }
        Opcode::Istore8 => format!("unsafe {{ core::ptr::write_unaligned({} as *mut u8, {} as u8) }};", addr(&args[1]), args[0]),
        Opcode::Istore16 => format!("unsafe {{ core::ptr::write_unaligned({} as *mut u16, {} as u16) }};", addr(&args[1]), args[0]),
        Opcode::Istore32 => format!("unsafe {{ core::ptr::write_unaligned({} as *mut u32, {} as u32) }};", addr(&args[1]), args[0]),
        Opcode::GetPinnedReg if read_style(u32::MAX).is_some() => return Err("a read takes no pinned register".into()),
        Opcode::GetPinnedReg => set("pin".to_string()),
        Opcode::SetPinnedReg => format!("pin = {};", args[0]),
        Opcode::GetStackPointer => set("sp()".to_string()),
        Opcode::Nop => String::new(),
        Opcode::Trap | Opcode::Debugtrap => "std::process::abort();".to_string(),
        Opcode::Trapz => format!("if {} == 0 {{ std::process::abort(); }}", args[0]),
        Opcode::Trapnz => format!("if {} != 0 {{ std::process::abort(); }}", args[0]),
        Opcode::Call => {
            let InstructionData::Call { func_ref, .. } = data else { return Err("a call of no function".into()) };
            let callee = match &dfg.ext_funcs[*func_ref].name {
                ir::ExternalName::User(r) => f.params.user_named_funcs()[*r].index,
                n => return Err(format!("a call of {n:?}")),
            };
            let call = if read_style(callee).is_some() {
                format!("unsafe {{ f{callee}(len, {}) }}", args.join(", "))
            } else {
                format!("unsafe {{ f{callee}(pin{}{}) }}", if args.is_empty() { "" } else { ", " }, args.join(", "))
            };
            // Its results returned as they are (`returned_call`): the call in the return, so
            // rustc hands its caller's result on (a tail call, no copy through the stack).
            if returned_call(f, inst) {
                return Ok(format!("return {call};"));
            }
            match results.len() {
                0 => format!("{call};"),
                1 => set(call),
                _ => {
                    let mut s = format!("let t = {call}; ");
                    for (i, &v) in results.iter().enumerate() {
                        write!(s, "{} = t.{i}; ", name(v)).unwrap();
                    }
                    s
                }
            }
        }
        // Made by the call before it (`returned_call`).
        Opcode::Return if f.layout.prev_inst(inst).is_some_and(|c| returned_call(f, c)) => String::new(),
        Opcode::Return => match args.len() {
            0 => "return;".to_string(),
            1 => format!("return {};", args[0]),
            _ => format!("return ({});", args.join(", ")),
        },
        Opcode::Jump => {
            let InstructionData::Jump { destination, .. } = data else { return Err("a jump with no destination".into()) };
            goto(f, destination, 0, name, jump)?
        }
        Opcode::Brif => {
            let InstructionData::Brif { blocks, .. } = data else { return Err("a branch with no blocks".into()) };
            format!("if {} != 0 {} else {}", args[0], goto(f, &blocks[0], 0, name, jump)?, goto(f, &blocks[1], 1, name, jump)?)
        }
        // A jump table: a `match`, its arms the table's (successors 1 on), the rest the default
        // (successor 0), as `successors` lists them; rustc makes it a table, or compares, again.
        // Entries that go to the same place one arm, its patterns their ranges, as a lexer's
        // `b'0'..=b'9' =>`; those that are the default's, the default's.
        Opcode::BrTable => {
            let InstructionData::BranchTable { table, .. } = data else { return Err("a jump table of nothing".into()) };
            let jt = &f.dfg.jump_tables[*table];
            let default = goto(f, &jt.default_block(), 0, name, jump)?;
            let mut arms: Vec<(String, Vec<(usize, usize)>)> = Vec::new();
            for (i, call) in jt.as_slice().iter().enumerate() {
                let to = goto(f, call, i + 1, name, jump)?;
                if to == default {
                    continue;
                }
                match arms.iter_mut().find(|a| a.0 == to) {
                    Some(a) => match a.1.last_mut() {
                        Some(r) if r.1 + 1 == i => r.1 = i,
                        _ => a.1.push((i, i)),
                    },
                    None => arms.push((to, vec![(i, i)])),
                }
            }
            let mut s = format!("match {} {{ ", args[0]);
            for (to, ranges) in &arms {
                let pats: Vec<String> = ranges.iter().map(|&(a, b)| if a == b { format!("{a}") } else { format!("{a}..={b}") }).collect();
                write!(s, "{} => {to} ", pats.join(" | ")).unwrap();
            }
            write!(s, "_ => {default} }}").unwrap();
            s
        }
        op => return Err(format!("the opcode {op}")),
    })
}

/// The `band`s each read only by compares that print as a bit test (`tested_bits`): the
/// test is the `and` too, so they are not printed.
fn absorbed(f: &ir::Function) -> std::collections::HashSet<ir::Inst> {
    let dfg = &f.dfg;
    let insts: Vec<ir::Inst> = f.layout.blocks().flat_map(|b| f.layout.block_insts(b)).collect();
    let mut tested = std::collections::HashSet::new();
    let mut other = std::collections::HashSet::new();
    for &inst in &insts {
        let in_test = matches!(dfg.insts[inst], ir::InstructionData::IntCompare { cond: IntCC::NotEqual, .. }) && dfg.value_type(dfg.inst_results(inst)[0]).is_vector() && tested_bits(f, inst).is_some();
        let mut reads: Vec<ir::Value> = dfg.inst_args(inst).iter().map(|&v| dfg.resolve_aliases(v)).collect();
        for c in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
            reads.extend(c.args(&dfg.value_lists).filter_map(|a| a.as_value()).map(|v| dfg.resolve_aliases(v)));
        }
        for v in reads {
            let ir::ValueDef::Result(d, _) = dfg.value_def(v) else { continue };
            if !matches!(dfg.insts[d], ir::InstructionData::Binary { opcode: Opcode::Band, .. }) {
                continue;
            }
            if in_test { tested.insert(d) } else { other.insert(d) };
        }
    }
    tested.retain(|d| !other.contains(d));
    // A narrowing shift's steps (`shrn`): written as one, where each is read by the next alone.
    let mut uses: std::collections::HashMap<ir::Inst, usize> = std::collections::HashMap::new();
    for &inst in &insts {
        for &v in dfg.inst_args(inst) {
            if let ir::ValueDef::Result(d, _) = dfg.value_def(dfg.resolve_aliases(v)) {
                *uses.entry(d).or_default() += 1;
            }
        }
        for c in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
            for v in c.args(&dfg.value_lists).filter_map(|a| a.as_value()) {
                if let ir::ValueDef::Result(d, _) = dfg.value_def(dfg.resolve_aliases(v)) {
                    *uses.entry(d).or_default() += 1;
                }
            }
        }
    }
    for &inst in &insts {
        if let Some((_, steps)) = narrowing_shift(f, inst) {
            // The `and`'s two reads by the narrowing (both its halves), the others one.
            let alone = steps.iter().enumerate().all(|(k, d)| uses.get(d).copied().unwrap_or(0) <= if k == 0 { 2 } else { 1 });
            if alone {
                tested.extend(steps);
            }
        }
    }
    tested
}

/// `uunarrow(k, k)` with `k = band(ushr_imm(bitcast.i16x8(v), 4), splat(0xff))`: NEON's
/// narrowing shift of `v`'s 16-bit halves by four (`shrn`, `native_cpu_block::short_mask`),
/// its halves both the result's. `v`, and the steps (`and`, shift, cast) it is written over.
fn narrowing_shift(f: &ir::Function, inst: ir::Inst) -> Option<(ir::Value, Vec<ir::Inst>)> {
    let dfg = &f.dfg;
    let def = |v: ir::Value| match dfg.value_def(dfg.resolve_aliases(v)) {
        ir::ValueDef::Result(i, _) => Some(i),
        _ => None,
    };
    let ir::InstructionData::Binary { opcode: Opcode::Uunarrow, args: [a, b] } = dfg.insts[inst] else { return None };
    if dfg.resolve_aliases(a) != dfg.resolve_aliases(b) {
        return None;
    }
    let band = def(a)?;
    let ir::InstructionData::Binary { opcode: Opcode::Band, args: [x, y] } = dfg.insts[band] else { return None };
    // The splat of 0xff and its constant.
    let is_ff = |v: ir::Value| -> Option<(ir::Inst, ir::Inst)> {
        let sp = def(v)?;
        let ir::InstructionData::Unary { opcode: Opcode::Splat, arg } = dfg.insts[sp] else { return None };
        let c = def(arg)?;
        matches!(dfg.insts[c], ir::InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } if imm.bits() & 0xffff == 0xff).then_some((sp, c))
    };
    let (shifted, (sp, c)) = match (is_ff(y), is_ff(x)) {
        (Some(k), _) => (x, k),
        (_, Some(k)) => (y, k),
        _ => return None,
    };
    let shift = def(shifted)?;
    let ir::InstructionData::BinaryImm64 { opcode: Opcode::UshrImm, arg, imm } = dfg.insts[shift] else { return None };
    if imm.bits() != 4 {
        return None;
    }
    let cast = def(arg)?;
    let ir::InstructionData::LoadNoOffset { opcode: Opcode::Bitcast, arg: v, .. } = dfg.insts[cast] else { return None };
    if dfg.value_type(v) != types::I8X16 {
        return None;
    }
    Some((dfg.resolve_aliases(v), vec![band, shift, cast, sp, c]))
}

/// A vector compare `band(x, y) != 0` (either side the zero): `x` and `y`, for NEON's `vtst`.
fn tested_bits(f: &ir::Function, inst: ir::Inst) -> Option<(ir::Value, ir::Value)> {
    let dfg = &f.dfg;
    let def = |v: ir::Value| match dfg.value_def(dfg.resolve_aliases(v)) {
        ir::ValueDef::Result(i, _) => Some(i),
        _ => None,
    };
    let zero = |v: ir::Value| -> bool {
        let Some(i) = def(v) else { return false };
        match dfg.insts[i] {
            ir::InstructionData::Unary { opcode: Opcode::Splat, arg } => def(arg).is_some_and(|c| matches!(dfg.insts[c], ir::InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } if imm.bits() == 0)),
            ir::InstructionData::UnaryConst { opcode: Opcode::Vconst, constant_handle } => f.dfg.constants.get(constant_handle).as_slice().iter().all(|&b| b == 0),
            _ => false,
        }
    };
    let args = dfg.inst_args(inst);
    let (and, other) = (args[0], args[1]);
    let (and, other) = if zero(other) { (and, other) } else { (other, and) };
    if !zero(other) {
        return None;
    }
    match dfg.insts[def(and)?] {
        ir::InstructionData::Binary { opcode: Opcode::Band, args: [x, y] } => Some((dfg.resolve_aliases(x), dfg.resolve_aliases(y))),
        _ => None,
    }
}

/// An instruction on vectors: NEON's intrinsic for it (`uint8x16_t` for `i8x16`).
#[allow(clippy::too_many_arguments)]
fn vector_instruction(f: &ir::Function, inst: ir::Inst, op: Opcode, args: &[String], rt: Option<ir::Type>, at: Option<ir::Type>, set: &dyn Fn(String) -> String, addr: &dyn Fn(&str) -> String) -> Result<String, String> {
    let dfg = &f.dfg;
    let data = &dfg.insts[inst];
    let is = |t: Option<ir::Type>, w: ir::Type| t == Some(w);
    let bytes16 = |b: &[u8]| -> Result<String, String> {
        if b.len() != 16 {
            return Err(format!("a vector constant of {} bytes", b.len()));
        }
        Ok(format!("unsafe {{ core::mem::transmute::<[u8; 16], uint8x16_t>({b:?}) }}"))
    };
    let u = |e: String| set(format!("unsafe {{ {e} }}"));
    Ok(match op {
        Opcode::Band if is(rt, types::I8X16) => u(format!("vandq_u8({}, {})", args[0], args[1])),
        Opcode::Isub if is(rt, types::I8X16) => u(format!("vsubq_u8({}, {})", args[0], args[1])),
        Opcode::Iadd if is(rt, types::I8X16) => u(format!("vaddq_u8({}, {})", args[0], args[1])),
        Opcode::Bor if is(rt, types::I8X16) => u(format!("vorrq_u8({}, {})", args[0], args[1])),
        Opcode::Bxor if is(rt, types::I8X16) => u(format!("veorq_u8({}, {})", args[0], args[1])),
        Opcode::Bnot if is(rt, types::I8X16) => u(format!("vmvnq_u8({})", args[0])),
        Opcode::BandNot if is(rt, types::I8X16) => u(format!("vbicq_u8({}, {})", args[0], args[1])),
        Opcode::BorNot if is(rt, types::I8X16) => u(format!("vornq_u8({}, {})", args[0], args[1])),
        Opcode::Icmp if is(at, types::I8X16) => {
            let (a, b) = (&args[0], &args[1]);
            let sg = |x: &str| format!("vreinterpretq_s8_u8({x})");
            u(match data.cond_code().ok_or("a compare with no condition")? {
                IntCC::Equal => format!("vceqq_u8({a}, {b})"),
                // Bits of an `and` against zero: NEON's bit test, one instruction.
                IntCC::NotEqual => match tested_bits(f, inst) {
                    Some((x, y)) => format!("vtstq_u8(v{}, v{})", x.as_u32(), y.as_u32()),
                    None => format!("vmvnq_u8(vceqq_u8({a}, {b}))"),
                },
                IntCC::UnsignedGreaterThan => format!("vcgtq_u8({a}, {b})"),
                IntCC::UnsignedGreaterThanOrEqual => format!("vcgeq_u8({a}, {b})"),
                IntCC::UnsignedLessThan => format!("vcltq_u8({a}, {b})"),
                IntCC::UnsignedLessThanOrEqual => format!("vcleq_u8({a}, {b})"),
                IntCC::SignedGreaterThan => format!("vcgtq_s8({}, {})", sg(a), sg(b)),
                IntCC::SignedGreaterThanOrEqual => format!("vcgeq_s8({}, {})", sg(a), sg(b)),
                IntCC::SignedLessThan => format!("vcltq_s8({}, {})", sg(a), sg(b)),
                IntCC::SignedLessThanOrEqual => format!("vcleq_s8({}, {})", sg(a), sg(b)),
            })
        }
        // A vector's pairs with themselves (a bitmask's reduction): both halves the same, so
        // written as the 64-bit `addp` of its halves, which LLVM keeps one `addp` (it turns a
        // `vpaddq_u8(a, a)` into `uzp1`, `uzp2`, `orr`).
        Opcode::IaddPairwise if is(rt, types::I8X16) && args[0] == args[1] => {
            u(format!("{{ let d = vpadd_u8(vget_low_u8({a}), vget_high_u8({a})); vcombine_u8(d, d) }}", a = args[0]))
        }
        Opcode::IaddPairwise if is(rt, types::I8X16) => u(format!("vpaddq_u8({}, {})", args[0], args[1])),
        Opcode::Swizzle if is(rt, types::I8X16) => u(format!("vqtbl1q_u8({}, {})", args[0], args[1])),
        Opcode::Shuffle if is(rt, types::I8X16) => {
            let InstructionData::Shuffle { imm, .. } = data else { return Err("a shuffle with no mask".into()) };
            let mask = dfg.immediates.get(*imm).ok_or("a shuffle's mask")?.as_slice();
            // Sixteen bytes in a row of the pair, from byte `n`: an `ext`, one instruction
            // and no table (as the lean lexer's `vextq_u8::<1>` makes a block's next bytes).
            match mask.first().filter(|&&n| n < 16 && mask.iter().enumerate().all(|(k, &m)| usize::from(m) == usize::from(n) + k)) {
                Some(&n) => u(format!("vextq_u8::<{n}>({}, {})", args[0], args[1])),
                None => u(format!("vqtbl2q_u8(uint8x16x2_t({}, {}), {})", args[0], args[1], bytes16(mask)?)),
            }
        }
        Opcode::Splat if is(rt, types::I8X16) => u(format!("vdupq_n_u8({} as u8)", args[0])),
        Opcode::Vconst if is(rt, types::I8X16) => {
            let InstructionData::UnaryConst { constant_handle, .. } = data else { return Err("a vector constant of nothing".into()) };
            set(bytes16(dfg.constants.get(*constant_handle).as_slice())?)
        }
        Opcode::Load if is(rt, types::I8X16) => u(format!("vld1q_u8({} as *const u8)", addr(&args[0]))),
        Opcode::Store if is(at, types::I8X16) => format!("unsafe {{ vst1q_u8({} as *mut u8, {}) }};", addr(&args[1]), args[0]),
        Opcode::Bitcast if is(rt, types::I64X2) && is(at, types::I8X16) => u(format!("vreinterpretq_u64_u8({})", args[0])),
        Opcode::Bitcast if is(rt, types::I8X16) && is(at, types::I64X2) => u(format!("vreinterpretq_u8_u64({})", args[0])),
        Opcode::Bitcast if is(rt, types::I16X8) && is(at, types::I8X16) => u(format!("vreinterpretq_u16_u8({})", args[0])),
        Opcode::Bitcast if is(rt, types::I8X16) && is(at, types::I16X8) => u(format!("vreinterpretq_u8_u16({})", args[0])),
        Opcode::Band if is(rt, types::I16X8) => u(format!("vandq_u16({}, {})", args[0], args[1])),
        Opcode::Splat if is(rt, types::I16X8) => u(format!("vdupq_n_u16({} as u16)", args[0])),
        Opcode::UshrImm if is(rt, types::I16X8) => {
            let InstructionData::BinaryImm64 { imm, .. } = data else { return Err("a shift by nothing".into()) };
            match imm.bits() & 15 {
                0 => set(args[0].clone()),
                n => u(format!("vshrq_n_u16::<{n}>({})", args[0])),
            }
        }
        // A narrowing shift (`narrowing_shift`): `shrn`, one instruction; else each half
        // narrowed.
        Opcode::Uunarrow if is(rt, types::I8X16) => match narrowing_shift(f, inst) {
            Some((v, _)) => u(format!("{{ let d = vshrn_n_u16::<4>(vreinterpretq_u16_u8(v{})); vcombine_u8(d, d) }}", v.as_u32())),
            None => u(format!("vcombine_u8(vqmovn_u16({}), vqmovn_u16({}))", args[0], args[1])),
        },
        Opcode::Extractlane if is(at, types::I64X2) => {
            let InstructionData::BinaryImm8 { imm, .. } = data else { return Err("a lane of nothing".into()) };
            u(format!("vgetq_lane_u64::<{imm}>({})", args[0]))
        }
        Opcode::UshrImm if is(rt, types::I8X16) => {
            let InstructionData::BinaryImm64 { imm, .. } = data else { return Err("a shift by nothing".into()) };
            match imm.bits() & 7 {
                0 => set(args[0].clone()),
                n => u(format!("vshrq_n_u8::<{n}>({})", args[0])),
            }
        }
        op => return Err(format!("the vector opcode {op} ({rt:?} of {at:?})")),
    })
}

#[cfg(test)]
mod tests {
    use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature, types::I64};
    use cranelift_codegen::isa::CallConv;
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

    /// `entry` branches into a loop of `a` and `b` at either: a loop with two entries (no
    /// block dominates the other), so the graph is irreducible.
    fn two_entries() -> cranelift_codegen::ir::Function {
        let mut sig = Signature::new(CallConv::Tail);
        sig.params.push(AbiParam::new(I64));
        sig.returns.push(AbiParam::new(I64));
        let mut f = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, 0), sig);
        let mut ctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut f, &mut ctx);
        let (entry, x, y, done) = (b.create_block(), b.create_block(), b.create_block(), b.create_block());
        b.append_block_params_for_function_params(entry);
        for blk in [x, y, done] {
            b.append_block_param(blk, I64);
        }
        b.switch_to_block(entry);
        let n = b.block_params(entry)[0];
        let odd = b.ins().band_imm(n, 1);
        b.ins().brif(odd, x, &[n.into()], y, &[n.into()]);
        b.switch_to_block(x);
        let v = b.block_params(x)[0];
        let v = b.ins().iadd_imm(v, -1);
        b.ins().jump(y, &[v.into()]);
        b.switch_to_block(y);
        let v = b.block_params(y)[0];
        let v = b.ins().iadd_imm(v, -1);
        let more = b.ins().icmp_imm(cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThan, v, 0);
        b.ins().brif(more, x, &[v.into()], done, &[v.into()]);
        b.switch_to_block(done);
        let v = b.block_params(done)[0];
        b.ins().return_(&[v]);
        b.seal_all_blocks();
        b.finalize();
        f
    }

    #[test]
    fn a_loop_with_two_entries_is_structured() {
        let src = super::translate(&[(0, two_entries())], &[(0, "entry")], &[]).expect("translates");
        assert!(src.contains("STRUCTURED: (usize, usize) = (1, 1)"), "{src}");
        assert!(!src.contains("match b {"), "no loop over the blocks: {src}");
        assert!(src.contains("loop {"), "{src}");
    }

    /// A call whose results are returned as they are is the return's (`returned_call`):
    /// `return f(…)`, so rustc hands its caller's result on, a tail call, where a copy through
    /// a temporary kept the caller's frame. (Moved from `rr_aot`'s generated source, where no
    /// function returns a call since the short forms are the loop's arms.)
    #[test]
    fn a_returned_call_is_the_return() {
        let callee = two_entries();
        let mut sig = Signature::new(CallConv::Tail);
        sig.params.push(AbiParam::new(I64));
        sig.returns.push(AbiParam::new(I64));
        let mut f = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, 1), sig.clone());
        let mut ctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut f, &mut ctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let n = b.block_params(entry)[0];
        let name = b.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, 0));
        let signature = b.import_signature(sig);
        let r = b.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
        let call = b.ins().call(r, &[n]);
        let got = b.inst_results(call).to_vec();
        b.ins().return_(&got);
        b.seal_all_blocks();
        b.finalize();
        let src = super::translate(&[(0, callee), (1, f)], &[(1, "outer")], &[]).expect("translates");
        assert!(src.contains("return unsafe { f0("), "no call returned as it is: {src}");
        assert!(!src.contains("let t = unsafe { f0("), "a call's results copied into a return: {src}");
    }

    /// A vector's bits against a mask, compared with zero for any set (`(x & m) != 0`), is
    /// one bit test (`vtstq_u8`), either operand order, and its `and` is not printed besides;
    /// a compare with zero for none is not.
    #[test]
    fn bits_against_zero_are_one_test() {
        use cranelift_codegen::ir::condcodes::IntCC;
        use cranelift_codegen::ir::types::{I8, I8X16};
        for (cc, zero_first) in [(IntCC::NotEqual, false), (IntCC::NotEqual, true), (IntCC::Equal, false)] {
            let mut sig = Signature::new(CallConv::Tail);
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
            let mut f = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, 0), sig);
            let mut ctx = FunctionBuilderContext::new();
            let mut b = FunctionBuilder::new(&mut f, &mut ctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let p = b.block_params(entry)[0];
            let x = b.ins().load(I8X16, cranelift_codegen::ir::MemFlags::trusted(), p, 0);
            let m = b.ins().iconst(I8, 6);
            let m = b.ins().splat(I8X16, m);
            let z = b.ins().iconst(I8, 0);
            let z = b.ins().splat(I8X16, z);
            let and = b.ins().band(x, m);
            let t = if zero_first { b.ins().icmp(cc, z, and) } else { b.ins().icmp(cc, and, z) };
            b.ins().store(cranelift_codegen::ir::MemFlags::trusted(), t, p, 0);
            b.ins().return_(&[p]);
            b.seal_all_blocks();
            b.finalize();
            let src = super::translate(&[(0, f)], &[(0, "entry")], &[]).expect("translates");
            assert_eq!(src.contains("vtstq_u8"), cc == IntCC::NotEqual, "{cc:?}, zero first {zero_first}: {src}");
            // The test is the `and` too: it is not printed besides.
            assert_eq!(src.contains("vandq_u8"), cc != IntCC::NotEqual, "{cc:?}, zero first {zero_first}: {src}");
        }
    }
}
