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
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            let data = &func.dfg.insts[inst];
            let calls = matches!(data, InstructionData::Call { .. } | InstructionData::CallIndirect { .. })
                || matches!(data.opcode(), Opcode::Call | Opcode::CallIndirect | Opcode::ReturnCall | Opcode::ReturnCallIndirect | Opcode::TryCall | Opcode::TryCallIndirect);
            let mine = match data {
                InstructionData::Call { func_ref, .. } => match &func.dfg.ext_funcs[*func_ref].name {
                    ExternalName::User(r) => own.contains(&func.params.user_named_funcs()[*r]),
                    _ => false,
                },
                _ => false,
            };
            if calls && !(mine && data.opcode() == Opcode::Call) {
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

#[cfg(test)]
#[path = "native_cpu_shape_tests.rs"]
mod tests;
