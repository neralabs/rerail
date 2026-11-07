//! The tape writer on the CPU: tier 2 (decision 0049), simdjson's stage 2. Tier 1, the block
//! program, gives each row's start and end masks. The writer walks them a word at a time, and
//! does the per-token work once per token: the tree's rules, the tokens' text checks, and the
//! tape. Nothing here is language-specific: every rule is a row the Bitlex file compiled to.
//!
//! - **Starts**, in rank order at a byte, open a node of their row where its rule holds: `after`
//!   (the previous token's row) and `parent` (the innermost open node's row). Its code's facts
//!   are read then: a mask bit, another row starting at the same byte, the previous token.
//! - **Ends** close the innermost open node when its row has an end bit there. A node that ends
//!   `with` a field closes with its child of that field. A `nested` row's end must close the
//!   innermost open nested node; one deeper than its bound, or a stray end, is an error.
//! - **Tokens** (`leaf` rows) must tile the file apart from skipped bytes, one per start byte. A
//!   token is retyped by its keyword rows and checked by its row's validator as it closes.
//! - **A size** is the count of nodes started up to the end, minus `ord`.
//!
//! An error sends the file to tree-sitter. The columns are packed by the tape's one packer
//! (`TapeImage::from_written`), so the bytes are those of the same nodes parsed by tree-sitter.

use super::block::{BlockProgram, Fact, RowEnd, RowStart, Validator};
use crate::code_rule_dsl::tape::WrittenNode;

/// An open node: its slot and row.
#[derive(Clone, Copy)]
struct Open {
    ord: u32,
    row: u16,
}

/// The nodes of one file, in preorder, and the supertype memberships `(node, class)`, from
/// `outputs`, the program's output masks over `source` (each cut to its length). An error is a
/// file whose tokens do not make a tree: the file goes to tree-sitter.
pub(in crate::code_rule_dsl) fn write(program: &BlockProgram, outputs: &[Vec<u64>], source: &[u8]) -> Result<(Vec<WrittenNode>, Vec<(u32, u16)>), String> {
    let output = |v: u16| -> Result<&[u64], String> {
        let k = program.outputs.iter().position(|&o| o == v).ok_or("a tree row's mask is not an output")?;
        Ok(&outputs[k])
    };
    let rows = &program.nodes;
    let starts: Vec<Option<&[u64]>> = rows
        .iter()
        .map(|r| match r.start {
            RowStart::At(v) => output(v).map(Some),
            RowStart::FirstToken => Ok(None),
        })
        .collect::<Result<_, _>>()?;
    let ends: Vec<Option<&[u64]>> = rows
        .iter()
        .map(|r| match r.end {
            RowEnd::At(v) => output(v).map(Some),
            _ => Ok(None),
        })
        .collect::<Result<_, _>>()?;
    let skip = program.skip.map(output).transpose()?;
    let len = source.len();
    let words = len.div_ceil(64);
    let word = |m: &[u64], w: usize| m.get(w).copied().unwrap_or(0);
    let bit_of = |m: &[u64], at: usize| word(m, at / 64) >> (at % 64) & 1 == 1;
    // The bytes `from..to` are all skipped.
    let skipped = |from: usize, to: usize| (from..to).all(|i| skip.is_some_and(|s| bit_of(s, i)));
    let mut codes: Vec<Vec<&super::block::CodeRow>> = vec![Vec::new(); rows.len()];
    for c in &program.codes {
        codes[usize::from(c.node)].push(c);
    }

    let mut nodes: Vec<WrittenNode> = Vec::new();
    let mut supertypes: Vec<(u32, u16)> = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut depth = 0usize;
    let mut first_open = false;
    // The previous token's row, and the byte after the last token.
    let mut prev_leaf: Option<u16> = None;
    let mut leaf_end = 0usize;
    for w in 0..words {
        let mut events = 0u64;
        for r in 0..rows.len() {
            events |= starts[r].map_or(0, |m| word(m, w)) | ends[r].map_or(0, |m| word(m, w));
        }
        while events != 0 {
            let j = events.trailing_zeros();
            events &= events - 1;
            let at = w * 64 + j as usize;
            let bit = 1u64 << j;
            let starts_here = |r: usize| starts[r].is_some_and(|m| word(m, w) & bit != 0);
            // The first token opens the rows that start with it, before anything at its byte.
            if !first_open && (0..rows.len()).any(|r| rows[r].leaf && starts_here(r)) {
                first_open = true;
                if !skipped(0, at) {
                    return Err(format!("byte {} is in no token", (0..at).find(|&i| !skipped(i, i + 1)).unwrap_or(0)));
                }
                for (r, row) in rows.iter().enumerate().filter(|(_, row)| row.start == RowStart::FirstToken) {
                    open(&mut nodes, &mut stack, &mut supertypes, r, row.kind, at, 0, &[]);
                }
            }
            // Starts, outermost first: the rows' order is their rank.
            let mut here: Vec<u16> = Vec::new();
            let mut leaf_here = None;
            for (r, row) in rows.iter().enumerate() {
                if !starts_here(r) {
                    continue;
                }
                if row.leaf {
                    if leaf_here.is_some() {
                        return Err(format!("two tokens start at byte {at}"));
                    }
                    if !skipped(leaf_end, at) {
                        return Err(format!("byte {leaf_end} is in no token"));
                    }
                    leaf_here = Some(r as u16);
                }
                if !row.after.is_empty() && !prev_leaf.is_some_and(|p| row.after.contains(&p)) {
                    continue;
                }
                if row.parent.is_some_and(|p| stack.last().is_none_or(|o| o.row != p)) {
                    continue;
                }
                if row.nested.is_some() {
                    depth += 1;
                    if row.nested.is_some_and(|d| depth > usize::from(d)) {
                        return Err(format!("nested deeper than {} at byte {at}", row.nested.unwrap_or(0)));
                    }
                }
                let holds = |c: &&&super::block::CodeRow| match &c.when {
                    Fact::At(v) => output(*v).is_ok_and(|m| word(m, w) & bit != 0),
                    Fact::StartOf(s) => here.contains(s),
                    Fact::NotStartOf(s) => !here.contains(s),
                    Fact::After(rs) => prev_leaf.is_some_and(|p| rs.contains(&p)),
                    Fact::Always => true,
                };
                let held: Vec<&super::block::CodeRow> = codes[r].iter().filter(holds).copied().collect();
                let field = held.iter().find(|c| c.field).map_or(0, |c| c.id);
                let classes: Vec<u16> = held.iter().filter(|c| !c.field).map(|c| c.id).collect();
                open(&mut nodes, &mut stack, &mut supertypes, r, row.kind, at, field, &classes);
                here.push(r as u16);
            }
            if leaf_here.is_some() {
                prev_leaf = leaf_here;
            }
            // Ends: close the innermost open node while its row ends here, and each node that
            // ends with the child just closed.
            let ends_here = |r: usize| ends[r].is_some_and(|m| word(m, w) & bit != 0);
            let mut closed: Vec<u16> = Vec::new();
            while let Some(&top) = stack.last() {
                let r = usize::from(top.row);
                if !ends_here(r) || closed.contains(&top.row) && rows[r].nested.is_some() {
                    break;
                }
                let mut node = close(&mut nodes, &mut stack, program, source, top, at, &mut depth)?;
                closed.push(top.row);
                if rows[r].leaf {
                    leaf_end = at + 1;
                }
                while let Some(&parent) = stack.last() {
                    match rows[usize::from(parent.row)].end {
                        RowEnd::With(f) if f == node.field && node.field != 0 => node = close(&mut nodes, &mut stack, program, source, parent, at, &mut depth)?,
                        _ => break,
                    }
                }
            }
            // A nested row's end that closed nothing: a stray or misnested close.
            for (r, row) in rows.iter().enumerate() {
                if row.nested.is_some() && ends_here(r) && !closed.contains(&(r as u16)) {
                    return Err(format!("a close of row {r} at byte {at} matches no open node"));
                }
            }
        }
    }
    if !first_open {
        return Err("no tokens: the root would be zero-width".into());
    }
    if !skipped(leaf_end, len) {
        return Err(format!("byte {leaf_end} is in no token"));
    }
    // The nodes that end with the last token.
    while let Some(&top) = stack.last() {
        if rows[usize::from(top.row)].end != RowEnd::LastToken {
            return Err(format!("node kind {} has no end", rows[usize::from(top.row)].kind));
        }
        close(&mut nodes, &mut stack, program, source, top, leaf_end - 1, &mut depth)?;
    }
    check_nesting(&nodes)?;
    Ok((nodes, supertypes))
}

/// Open a node of row `r` at byte `at`.
#[allow(clippy::too_many_arguments)]
fn open(nodes: &mut Vec<WrittenNode>, stack: &mut Vec<Open>, supertypes: &mut Vec<(u32, u16)>, r: usize, kind: u16, at: usize, field: u16, classes: &[u16]) {
    let ord = nodes.len() as u32;
    nodes.push(WrittenNode { kind, field, extra: false, start: at as u32, end: 0, size: 0 });
    supertypes.extend(classes.iter().map(|&c| (ord, c)));
    stack.push(Open { ord, row: r as u16 });
}

/// Close the innermost open node `top` at byte `at`: its end and size; a token's keyword and
/// validator.
fn close(nodes: &mut [WrittenNode], stack: &mut Vec<Open>, program: &BlockProgram, source: &[u8], top: Open, at: usize, depth: &mut usize) -> Result<WrittenNode, String> {
    stack.pop();
    let row = &program.nodes[usize::from(top.row)];
    if row.nested.is_some() {
        *depth -= 1;
    }
    let count = nodes.len() as u32;
    let node = &mut nodes[top.ord as usize];
    node.end = at as u32 + 1;
    node.size = count - top.ord;
    node.extra = row.extra;
    if row.leaf {
        let text = &source[node.start as usize..node.end as usize];
        if let Some(k) = program.keywords.iter().find(|k| k.node == top.row && k.word == text) {
            node.kind = k.kind;
        }
        if let Some((kind, v)) = row.check
            && node.kind == kind
            && !valid(v, text)
        {
            return Err(format!("{} {:?} at byte {} fails its check", v.name(), String::from_utf8_lossy(text), node.start));
        }
    }
    Ok(node.clone())
}

/// A built-in validator over a token's text (tier 2's lowering).
pub(super) fn valid(v: Validator, text: &[u8]) -> bool {
    match v {
        Validator::JsonNumber => json_number(text),
    }
}

/// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE]-?[0-9]+)?`.
fn json_number(t: &[u8]) -> bool {
    let digits = |t: &[u8], i: usize| t[i..].iter().take_while(|b| b.is_ascii_digit()).count();
    let mut i = usize::from(t.first() == Some(&b'-'));
    match t.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => i += digits(t, i),
        _ => return false,
    }
    if t.get(i) == Some(&b'.') {
        let n = digits(t, i + 1);
        if n == 0 {
            return false;
        }
        i += 1 + n;
    }
    if matches!(t.get(i), Some(b'e' | b'E')) {
        i += 1;
        if t.get(i) == Some(&b'-') {
            i += 1;
        }
        let n = digits(t, i);
        if n == 0 {
            return false;
        }
        i += n;
    }
    i == t.len()
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
    /// when the file goes to tree-sitter: a fallback output (`fallbacks`) has a bit, or its
    /// tokens make no tree.
    pub(in crate::code_rule_dsl) fn tape(
        &self,
        source: &str,
        fallbacks: &[u16],
        grammar: &crate::code_rule_dsl::tape::Grammar,
    ) -> Result<Option<crate::code_rule_dsl::tape::TapeImage>, String> {
        tape_of(self, &self.interpret(source.as_bytes()), fallbacks, grammar, source)
    }
}

/// The tape of `source` from a program's outputs over it, by any executor. `fallbacks`: the
/// fallback's outputs, one per delay (`bitlex_compile::Compiled::fallbacks`).
pub(in crate::code_rule_dsl) fn tape_of(
    program: &BlockProgram,
    outputs: &[Vec<u64>],
    fallbacks: &[u16],
    grammar: &crate::code_rule_dsl::tape::Grammar,
    source: &str,
) -> Result<Option<crate::code_rule_dsl::tape::TapeImage>, String> {
    if fallbacks.iter().any(|&k| outputs[usize::from(k)].iter().any(|&w| w != 0)) {
        return Ok(None);
    }
    let Ok((nodes, supertypes)) = write(program, outputs, source.as_bytes()) else {
        return Ok(None);
    };
    crate::code_rule_dsl::tape::TapeImage::from_written(&nodes, supertypes, grammar, source, &|| false).map(Some)
}
