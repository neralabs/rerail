//! The shape every generated parse function must have, checked on its Cranelift IR before code
//! generation. A function that breaks it is refused, not run:
//!
//! - **No calls out.** The function calls no host Rust and no runtime helper. It may call only
//!   functions the same lowering made from the same program (`own`): a gated region or the
//!   general body, outlined so that the hot loop alone is inlined. Those are checked with no
//!   `own`, so they call nothing, and the calls make no cycle.
//! - **Every loop is a stated one.** Each cycle of the control-flow graph passes a block the
//!   lowering named as a progress point, with what it steps:
//!   - `File`: the next file of a batch;
//!   - `Block`: the next 64 bytes;
//!   - `Bits`: the next eight set bits of a mask (an extract's flattening);
//!   - `Token`: the next token of tier 1's stream;
//!   - `Round`: the next round of a fixed point, within a block (at most `block::MAX_ROUNDS`);
//!   - `Region`: the next region of a block, on the CPU (`block::Walk`, decision 0139);
//!   - `Word`: the next eight bytes of a scan for a closer past the block (a walk's `else`);
//!   - `Node`: the next record of the node table (placing the wraps at the end, and the final
//!     pass: each record's parent and extra flag);
//!   - `Return`: the next call of the call stack, popped (the words' verifier: no call is made
//!     on a cycle without a token, so the returns are at most the calls).
//!
//!   So no loop steps a byte at a time. The check is on the graph: with the named blocks taken
//!   out, it must have no cycle (a depth-first search), as the words' verifier cuts `TOKEN`'s
//!   edges.

use cranelift_codegen::flowgraph::ControlFlowGraph;
use cranelift_codegen::ir::{Block, ExternalName, Function, InstructionData, Opcode, UserExternalName};
use std::collections::HashMap;

/// What one turn of a loop steps over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    File,
    Block,
    Bits,
    Token,
    Round,
    Region,
    Word,
    Node,
    Return,
}

/// Why a function was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A call instruction, as text.
    Call(String),
    /// A loop whose header the lowering did not name.
    Loop(String),
}

/// Refuse `func` unless it calls only `own` functions and each of its cycles passes one of
/// `heads`.
pub fn check(func: &Function, heads: &[(Block, Step)], own: &[UserExternalName]) -> Result<(), Refusal> {
    check_tails(func, heads, own, &[])
}

/// `check`, and tail calls (`return_call`) of `tails` allowed too: tier 2's functions of
/// clusters, which call each other so. Their loops across functions are `acyclic`'s check.
pub fn check_tails(func: &Function, heads: &[(Block, Step)], own: &[UserExternalName], tails: &[UserExternalName]) -> Result<(), Refusal> {
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            let data = &func.dfg.insts[inst];
            let calls = matches!(data, InstructionData::Call { .. } | InstructionData::CallIndirect { .. })
                || matches!(data.opcode(), Opcode::Call | Opcode::CallIndirect | Opcode::ReturnCall | Opcode::ReturnCallIndirect | Opcode::TryCall | Opcode::TryCallIndirect);
            let named = |list: &[UserExternalName]| match data {
                InstructionData::Call { func_ref, .. } => match &func.dfg.ext_funcs[*func_ref].name {
                    ExternalName::User(r) => list.contains(&func.params.user_named_funcs()[*r]),
                    _ => false,
                },
                _ => false,
            };
            let allowed = (data.opcode() == Opcode::Call && named(own)) || (data.opcode() == Opcode::ReturnCall && named(tails));
            if calls && !allowed {
                return Err(Refusal::Call(func.dfg.display_inst(inst).to_string()));
            }
        }
    }
    let cfg = ControlFlowGraph::with_function(func);
    let named: HashMap<Block, Step> = heads.iter().copied().collect();
    // With the named blocks taken out, the graph must be acyclic: then every cycle passes one.
    // A depth-first search from every block left; 1 on the path, 2 done.
    let mut mark: HashMap<Block, u8> = HashMap::new();
    for root in func.layout.blocks().filter(|b| !named.contains_key(b)) {
        if mark.contains_key(&root) {
            continue;
        }
        let succs = |b: Block| -> Vec<Block> { cfg.succ_iter(b).filter(|s| !named.contains_key(s)).collect() };
        let mut stack: Vec<(Block, Vec<Block>)> = vec![(root, succs(root))];
        mark.insert(root, 1);
        while let Some((block, next)) = stack.last_mut() {
            match next.pop() {
                Some(n) => match mark.get(&n).copied() {
                    Some(1) => return Err(Refusal::Loop(format!("{block} -> {n}"))),
                    Some(_) => {}
                    None => {
                        mark.insert(n, 1);
                        let s = succs(n);
                        stack.push((n, s));
                    }
                },
                None => {
                    mark.insert(*block, 2);
                    stack.pop();
                }
            }
        }
    }
    Ok(())
}

/// For each block of `from`, the `exits` (a block that leaves the function, and where to) it
/// reaches before any of `heads`: none from a head. The graph with the heads taken out is
/// acyclic (`check`), so each block's exits are its successors' and their own, found once.
pub fn exits(func: &Function, heads: &[(Block, Step)], from: &[Block], exits: &HashMap<Block, usize>) -> Vec<Vec<usize>> {
    let cfg = ControlFlowGraph::with_function(func);
    let named: HashMap<Block, Step> = heads.iter().copied().collect();
    let mut memo: HashMap<Block, std::rc::Rc<Vec<usize>>> = HashMap::new();
    // Post-order, without recursion: a block's exits once its successors' are known.
    let mut reach = |root: Block| -> std::rc::Rc<Vec<usize>> {
        let mut stack: Vec<(Block, bool)> = vec![(root, false)];
        while let Some((b, done)) = stack.pop() {
            if memo.contains_key(&b) {
                continue;
            }
            let succs: Vec<Block> = cfg.succ_iter(b).filter(|s| !named.contains_key(s)).collect();
            if !done {
                stack.push((b, true));
                stack.extend(succs.iter().filter(|s| !memo.contains_key(s)).map(|&s| (s, false)));
                continue;
            }
            let mut out: Vec<usize> = exits.get(&b).copied().into_iter().collect();
            for s in succs {
                out.extend(memo.get(&s).map(|v| v.as_slice()).unwrap_or(&[]));
            }
            out.sort_unstable();
            out.dedup();
            memo.insert(b, std::rc::Rc::new(out));
        }
        memo.get(&root).cloned().unwrap_or_default()
    };
    from.iter().map(|&b| if named.contains_key(&b) { Vec::new() } else { reach(b).as_ref().clone() }).collect()
}

/// Functions that pass control to each other: no cycle of `edges` (an entry to the entries it
/// reaches before a loop head, `exits`), so every loop across them passes a head.
pub fn acyclic(edges: &HashMap<usize, Vec<usize>>) -> Result<(), Refusal> {
    let mut mark: HashMap<usize, u8> = HashMap::new();
    let mut roots: Vec<usize> = edges.keys().copied().collect();
    roots.sort_unstable();
    for root in roots {
        if mark.contains_key(&root) {
            continue;
        }
        let next = |x: usize| edges.get(&x).cloned().unwrap_or_default();
        let mut stack: Vec<(usize, Vec<usize>)> = vec![(root, next(root))];
        mark.insert(root, 1);
        while let Some((at, rest)) = stack.last_mut() {
            match rest.pop() {
                Some(n) => match mark.get(&n).copied() {
                    Some(1) => return Err(Refusal::Loop(format!("entry {at} -> entry {n}, across functions"))),
                    Some(_) => {}
                    None => {
                        mark.insert(n, 1);
                        let s = next(n);
                        stack.push((n, s));
                    }
                },
                None => {
                    mark.insert(*at, 2);
                    stack.pop();
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "native_cpu_shape_tests.rs"]
mod tests;
