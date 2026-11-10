//! What is live where in a tier 1 pass's Cranelift IR, before register allocation: the
//! profile's view of why a pass spills (`rr_parser::rr_parser_profile`).
//!
//! A spill is the allocator's answer to more values live at a point than there are registers.
//! So the report counts, at each point of the function, the values live there (integer and
//! vector apart), and blames each value for the points where it is live and the count is over
//! the registers. A value with much blame is one made long before it is read, or read
//! late: moving its making or its readers, or splitting its pass, is what cuts spills.
//!
//! Each instruction the body makes for a value is marked with it (`Emit::emit` sets the source
//! location to the pass's instruction index + 1). `measure` maps that to the whole program
//! (`native_ir::Passes::origins`); the profile maps that to the Bitlex statement
//! (`bitlex_compile::Compiled::origin`). A loop's block parameters (the carried state, the
//! counts, the pointers) have no instruction: they are reported as unmarked.

use std::collections::HashMap;

use cranelift_codegen::ir::{Block, Function, Value, ValueDef};

/// The integer registers the allocator has for values on aarch64 (x0..x15, x19..x28 less the
/// few it keeps), about. A count over it is a point where something must be on the stack.
pub(super) const INT_REGS: usize = 24;
/// The vector registers (v0..v31 less its scratch), about.
pub(super) const VEC_REGS: usize = 30;

/// A pass's register pressure. Values are named by their place in the whole program, `None`
/// for unmarked ones (block parameters, the loops' glue).
#[derive(Clone, Debug, Default)]
pub(super) struct Pressure {
    /// The function's instructions (each a point).
    pub points: usize,
    pub int_peak: usize,
    pub vec_peak: usize,
    /// The points where more integer (vector) values are live than there are registers.
    pub int_over: usize,
    pub vec_over: usize,
    /// The integer values live at the hot loop's first point of peak integer pressure, with
    /// what made each.
    pub at_peak: Vec<(Option<u32>, String)>,
    /// Each value's blame, most first: the hot loop's points over the registers (integer for an
    /// integer value, vector for a vector one) where it is live; its live points there;
    /// whether a vector.
    pub blame: Vec<Blame>,
    /// The hot loop (a full block's body, `hot`'s blocks in layout order): the integer and the
    /// vector values live at each of its points, first to last, and how many of the integer
    /// ones live through it all (at its first point and its last: the loop's own values and
    /// what crosses the body).
    pub hot_int: Vec<usize>,
    pub hot_vec: Vec<usize>,
    pub hot_through: usize,
    /// The hot loop's points over the integer (vector) registers.
    pub hot_int_over: usize,
    pub hot_vec_over: usize,
}

#[derive(Clone, Debug)]
pub(super) struct Blame {
    pub origin: Option<u32>,
    /// What made it: the instruction's opcode, or `param of blockN` (a loop's parameter: the
    /// carried state, a count, a pointer).
    pub made: String,
    pub over: usize,
    pub live: usize,
    pub vector: bool,
}

/// `func`'s pressure, its marks read through `origins` (the pass's instructions' places in the
/// whole program).
pub(super) fn measure(func: &Function, origins: &[u32], hot: &[Block]) -> Pressure {
    let dfg = &func.dfg;
    let blocks: Vec<Block> = func.layout.blocks().collect();
    let succs = |b: Block| -> Vec<Block> {
        func.layout
            .last_inst(b)
            .map(|i| dfg.insts[i].branch_destination(&dfg.jump_tables, &dfg.exception_tables).iter().map(|c| c.block(&dfg.value_lists)).collect())
            .unwrap_or_default()
    };
    // Live-in sets, to a fixed point (backward dataflow; a block's parameters are defined at
    // its head, a branch's arguments used at the branch).
    let index: HashMap<Block, usize> = blocks.iter().enumerate().map(|(k, &b)| (b, k)).collect();
    let mut live_in: Vec<std::collections::HashSet<Value>> = vec![Default::default(); blocks.len()];
    let live_out = |live_in: &[std::collections::HashSet<Value>], b: Block| -> std::collections::HashSet<Value> {
        succs(b).into_iter().flat_map(|s| live_in[index[&s]].iter().copied().collect::<Vec<_>>()).collect()
    };
    loop {
        let mut changed = false;
        for (k, &b) in blocks.iter().enumerate().rev() {
            let mut live = live_out(&live_in, b);
            for inst in func.layout.block_insts(b).collect::<Vec<_>>().into_iter().rev() {
                for &r in dfg.inst_results(inst) {
                    live.remove(&r);
                }
                live.extend(dfg.inst_values(inst));
            }
            for p in dfg.block_params(b) {
                live.remove(p);
            }
            if live != live_in[k] {
                live_in[k] = live;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let origin = |v: Value| -> Option<u32> {
        match dfg.value_def(v) {
            ValueDef::Result(inst, _) => {
                let loc = func.srcloc(inst);
                if loc.is_default() || loc.bits() == 0 { None } else { origins.get(loc.bits() as usize - 1).copied() }
            }
            _ => None,
        }
    };
    let vector = |v: Value| dfg.value_type(v).is_vector();
    let made = |v: Value| -> String {
        match dfg.value_def(v) {
            ValueDef::Result(inst, _) => dfg.insts[inst].opcode().to_string(),
            ValueDef::Param(b, _) => format!("param of {b}"),
            ValueDef::Union(..) => "union".into(),
        }
    };
    // Each point's live set, walked backward a block at a time.
    let mut p = Pressure::default();
    let mut blame: HashMap<Value, (usize, usize)> = HashMap::new();
    let mut peak_set: Vec<Value> = Vec::new();
    let mut peak_ints = 0;
    let hot_blocks: Vec<Block> = blocks.iter().copied().filter(|b| hot.contains(b)).collect();
    let (mut hot_first, mut hot_last): (Vec<Value>, Vec<Value>) = Default::default();
    let mut hot_int: Vec<Vec<usize>> = vec![Vec::new(); hot_blocks.len()];
    let mut hot_vec: Vec<Vec<usize>> = vec![Vec::new(); hot_blocks.len()];
    for &b in &blocks {
        let mut live = live_out(&live_in, b);
        let h = hot_blocks.iter().position(|&x| x == b);
        let mut first_point = true;
        for inst in func.layout.block_insts(b).collect::<Vec<_>>().into_iter().rev() {
            for &r in dfg.inst_results(inst) {
                live.remove(&r);
            }
            live.extend(dfg.inst_values(inst));
            // The values live across this instruction (read here or later, made before).
            p.points += 1;
            let ints = live.iter().filter(|&&v| !vector(v)).count();
            let vecs = live.len() - ints;
            if let Some(h) = h {
                hot_int[h].push(ints);
                hot_vec[h].push(vecs);
                if first_point && h + 1 == hot_blocks.len() {
                    hot_last = live.iter().copied().filter(|&v| !vector(v)).collect();
                }
                if h == 0 {
                    hot_first = live.iter().copied().filter(|&v| !vector(v)).collect();
                }
            }
            first_point = false;
            p.int_peak = p.int_peak.max(ints);
            p.vec_peak = p.vec_peak.max(vecs);
            let (io, vo) = (ints > INT_REGS, vecs > VEC_REGS);
            p.int_over += usize::from(io);
            p.vec_over += usize::from(vo);
            // The blame and the peak's values: the hot loop's only, when there is one (the
            // first and last blocks' general body runs once a file).
            if h.is_none() && !hot_blocks.is_empty() {
                continue;
            }
            if ints > peak_ints {
                peak_ints = ints;
                peak_set = live.iter().copied().filter(|&v| !vector(v)).collect();
            }
            p.hot_int_over += usize::from(io);
            p.hot_vec_over += usize::from(vo);
            for &v in &live {
                let e = blame.entry(v).or_default();
                e.1 += 1;
                if (io && !vector(v)) || (vo && vector(v)) {
                    e.0 += 1;
                }
            }
        }
    }
    for (ints, vecs) in hot_int.iter_mut().zip(&mut hot_vec) {
        ints.reverse();
        vecs.reverse();
        p.hot_int.extend(ints.iter());
        p.hot_vec.extend(vecs.iter());
    }
    p.hot_through = hot_first.iter().filter(|v| hot_last.contains(v)).count();
    p.at_peak = peak_set.iter().map(|&v| (origin(v), made(v))).collect();
    let mut blame: Vec<Blame> = blame.into_iter().filter(|(_, (o, _))| *o > 0).map(|(v, (over, live))| Blame { origin: origin(v), made: made(v), over, live, vector: vector(v) }).collect();
    blame.sort_by_key(|b| std::cmp::Reverse(b.over));
    p.blame = blame;
    p
}
