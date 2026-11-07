//! The tape writer on the CPU (`docs/parser-generators/13-tape-on-the-gpu.md` §2–§3): a block
//! program's tree rows and its output masks for one file, written as the file's tape. No tree is
//! built and nothing is language-specific: the `node` rows say where each kind starts and ends,
//! the `code` rows which field and supertypes a node has at its start.
//!
//! The masks are read a word at a time. In each word the only loop is over its set bits, the
//! starts and ends (as the extract pass's). Records come out in preorder, which is start order
//! (byte, then rank), so a node's slot `ord` is the count of nodes started before it.
//!
//! - **A start** opens a node of its row at slot `ord`, with its code read at its byte.
//! - **An end bit** of a row closes the open nodes of that row. Without a level, that is every
//!   open one: each started at or before the bit and found no earlier end. With a level, only
//!   those whose level before their start equals the level after the bit: a stack per row and
//!   level replaces the GPU's nearest-smaller search (§3.3).
//! - **A size** is the count of nodes started up to the end, minus `ord`.
//!
//! The columns are then packed by the tape's one packer (`TapeImage::from_written`), so the bytes
//! are those of the same nodes parsed by tree-sitter.

use super::block::{BlockProgram, NodeRow};
use crate::code_rule_dsl::tape::WrittenNode;

/// A node open on a row's stack: its slot.
type Open = u32;

/// The nodes of one file, in preorder, and the supertype memberships `(node, class)`, from
/// `outputs`, the program's output masks over its `len` bytes (each cut to `len`). An error is a
/// file whose masks do not make a tree (§1.4): the file goes to tree-sitter.
pub(in crate::code_rule_dsl) fn write(program: &BlockProgram, outputs: &[Vec<u64>], len: usize) -> Result<(Vec<WrittenNode>, Vec<(u32, u16)>), String> {
    let output = |v: u16| -> Result<&[u64], String> {
        let k = program.outputs.iter().position(|&o| o == v).ok_or("a tree row's mask is not an output")?;
        Ok(&outputs[k])
    };
    let rows = &program.nodes;
    let starts: Vec<&[u64]> = rows.iter().map(|r| output(r.start)).collect::<Result<_, _>>()?;
    let ends: Vec<&[u64]> = rows.iter().map(|r| output(r.end)).collect::<Result<_, _>>()?;
    let levels: Vec<Option<(&[u64], &[u64])>> = rows
        .iter()
        .map(|r| r.level.map(|(u, d)| Ok::<_, String>((output(u)?, output(d)?))).transpose())
        .collect::<Result<_, _>>()?;
    let codes: Vec<(&super::block::CodeRow, &[u64])> = program.codes.iter().map(|c| Ok((c, output(c.when)?))).collect::<Result<_, String>>()?;
    // The level is the same count for every row that has one (the up and down masks).
    let level = levels.iter().flatten().next().copied();
    let words = len.div_ceil(64);
    let word = |m: &[u64], w: usize| m.get(w).copied().unwrap_or(0);

    let mut nodes: Vec<WrittenNode> = Vec::new();
    let mut supertypes: Vec<(u32, u16)> = Vec::new();
    // Per row, its open nodes; a leveled row's, per level before their start.
    let mut open: Vec<Vec<Vec<Open>>> = rows.iter().map(|_| vec![Vec::new()]).collect();
    // The level after the byte before the word, and its changes in the word.
    let mut depth: i64 = 0;
    for w in 0..words {
        let (up, down) = level.map_or((0, 0), |(u, d)| (word(u, w), word(d, w)));
        let mut events = 0u64;
        for r in 0..rows.len() {
            events |= word(starts[r], w) | word(ends[r], w);
        }
        events |= down;
        // The level after bit j of the word: `depth` plus the changes at and below j.
        let after = |j: u32| -> i64 {
            let below = if j == 63 { !0 } else { (1u64 << (j + 1)) - 1 };
            depth + i64::from((up & below).count_ones()) - i64::from((down & below).count_ones())
        };
        while events != 0 {
            let j = events.trailing_zeros();
            events &= events - 1;
            let at = (w * 64) as u32 + j;
            let bit = 1u64 << j;
            let before = if j == 0 { depth } else { after(j - 1) };
            let here = after(j);
            // Starts, outermost first: the rows' order is their rank.
            for (r, row) in rows.iter().enumerate() {
                if word(starts[r], w) & bit == 0 {
                    continue;
                }
                let ord = u32::try_from(nodes.len()).map_err(|_| "too many nodes")?;
                let (field, classes) = code_at(&codes, r, w, bit);
                nodes.push(WrittenNode { kind: row.kind, field, extra: row.extra, start: at, end: 0, size: 0 });
                supertypes.extend(classes.map(|c| (ord, c)));
                let stack = stack(&mut open[r], row, before)?;
                stack.push(ord);
            }
            // Ends: every open node the bit closes ends at `at`; its subtree is the nodes
            // started so far, at bytes up to `at`.
            let count = nodes.len() as u32;
            for (r, row) in rows.iter().enumerate() {
                if word(ends[r], w) & bit == 0 {
                    continue;
                }
                let stack = stack(&mut open[r], row, here)?;
                for ord in stack.drain(..) {
                    let node = &mut nodes[ord as usize];
                    node.end = at + 1;
                    node.size = count - ord;
                }
            }
            // A leveled node still open when the level drops below its start's has no end.
            for (r, row) in rows.iter().enumerate() {
                if row.level.is_some() && open[r].iter().enumerate().any(|(l, s)| l as i64 > here && !s.is_empty()) {
                    return Err(format!("node kind {} at a level above {here} has no end", row.kind));
                }
            }
        }
        depth = after(63);
    }
    if open.iter().flatten().any(|s| !s.is_empty()) {
        return Err("a node has no end".into());
    }
    if depth != 0 {
        return Err("the level does not end at zero".into());
    }
    check_nesting(&nodes)?;
    Ok((nodes, supertypes))
}

/// A row's stack for nodes at `level` (one stack for a row without a level).
fn stack<'a>(open: &'a mut Vec<Vec<Open>>, row: &NodeRow, level: i64) -> Result<&'a mut Vec<Open>, String> {
    if row.level.is_none() {
        return Ok(&mut open[0]);
    }
    let l = usize::try_from(level).map_err(|_| "the level goes below zero")?;
    if open.len() <= l {
        open.resize_with(l + 1, Vec::new);
    }
    Ok(&mut open[l])
}

/// The field (the first that holds, else 0) and supertypes of a node of row `r` starting at
/// `bit` of word `w`.
fn code_at<'a>(codes: &'a [(&super::block::CodeRow, &[u64])], r: usize, w: usize, bit: u64) -> (u16, impl Iterator<Item = u16> + 'a) {
    let holds = move |c: &&(&super::block::CodeRow, &[u64])| usize::from(c.0.node) == r && c.1.get(w).is_some_and(|&x| x & bit != 0);
    let field = codes.iter().filter(holds).find(|c| c.0.field).map_or(0, |c| c.0.id);
    (field, codes.iter().filter(holds).filter(|c| !c.0.field).map(|c| c.0.id))
}

/// Each node inside the nearest earlier node whose subtree holds it, and ending by its end
/// (§1.4: a node that ends after its parent is an error); one root.
fn check_nesting(nodes: &[WrittenNode]) -> Result<(), String> {
    if nodes.is_empty() {
        return Err("no nodes: the root would be zero-width".into());
    }
    if nodes[0].size as usize != nodes.len() {
        return Err("more than one root".into());
    }
    let mut open: Vec<usize> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        while let Some(&p) = open.last() {
            if i < p + nodes[p].size as usize {
                break;
            }
            open.pop();
        }
        if let Some(&p) = open.last()
            && node.end > nodes[p].end
        {
            return Err(format!("node kind {} at byte {} ends after its parent", node.kind, node.start));
        }
        open.push(i);
    }
    Ok(())
}

impl BlockProgram {
    /// The tape of `source` (decision 0046): the interpreter's masks, then the writer. `None`
    /// when the file goes to tree-sitter: its fallback output (`fallback`) has a bit, or its
    /// masks make no tree.
    pub(in crate::code_rule_dsl) fn tape(
        &self,
        source: &str,
        fallback: usize,
        grammar: &crate::code_rule_dsl::tape::Grammar,
    ) -> Result<Option<crate::code_rule_dsl::tape::TapeImage>, String> {
        tape_of(self, &self.interpret(source.as_bytes()), fallback, grammar, source)
    }
}

/// The tape of `source` from a program's outputs over it, by any executor.
pub(in crate::code_rule_dsl) fn tape_of(
    program: &BlockProgram,
    outputs: &[Vec<u64>],
    fallback: usize,
    grammar: &crate::code_rule_dsl::tape::Grammar,
    source: &str,
) -> Result<Option<crate::code_rule_dsl::tape::TapeImage>, String> {
    if outputs[fallback].iter().any(|&w| w != 0) {
        return Ok(None);
    }
    let Ok((nodes, supertypes)) = write(program, outputs, source.len()) else {
        return Ok(None);
    };
    crate::code_rule_dsl::tape::TapeImage::from_written(&nodes, supertypes, grammar, source, &|| false).map(Some)
}
