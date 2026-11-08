//! A `tree` section's nodes, by the generic rule of docs/parser-generators/13-tape-on-the-gpu.md.
//! This file has no language in it.
//!
//! - **Nodes.** A node of each declared kind starts at each bit of its start mask. Each token is
//!   a node too, unless its kind is `untoken`.
//! - **Preorder.** Nodes are ordered by start byte, then by rank. Declared kinds rank in the order
//!   they are listed, and tokens after them.
//! - **The end rule.** A node starting at byte `s` ends at the first bit `e >= s` of its end mask
//!   where the level after `e` equals the level before `s`. Without a level, at the first bit.
//! - **The columns.** `ord = C[s] + rank` and `size = C[e + 1] - ord`, where `C[p]` counts the
//!   nodes starting before byte `p`.
//!
//! Every value comes from the masks and prefix counts; no tree is built. A nesting check
//! confirms the result is a tree. A file that fails it goes to tree-sitter.

use crate::eval::{Token, TreeDecl};

/// A node, in preorder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: String,
    pub field: Option<String>,
    /// Supertype names, in declaration order.
    pub supertypes: Vec<String>,
    pub extra: bool,
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
    /// From the generic rule: `C[end] - ord`.
    pub size: usize,
}

/// A node before ordering: start, rank, kind, end (exclusive).
struct Raw {
    start: usize,
    rank: usize,
    kind: String,
    end: usize,
}

/// The nodes of `decl` and `tokens` over `n` bytes, or the reason the file goes to tree-sitter.
pub fn nodes(decl: &TreeDecl, tokens: &[Token], n: usize) -> Result<Vec<Node>, String> {
    if decl.tier2 {
        return Err("the tree section has tier-2 rules: the engine's tape writer runs them".into());
    }
    let mut raw: Vec<Raw> = Vec::new();
    for (rank, (kind, start, end, level)) in decl.nodes.iter().enumerate() {
        for s in start.ones_iter() {
            let before = level.as_ref().map(|l| if s == 0 { 0 } else { l[s - 1] });
            let mut found = None;
            for e in s..n {
                match (level, before) {
                    (Some(l), Some(b)) => {
                        if end.get(e) && l[e] == b {
                            found = Some(e);
                            break;
                        }
                        if l[e] < b {
                            break;
                        }
                    }
                    _ => {
                        if end.get(e) {
                            found = Some(e);
                            break;
                        }
                    }
                }
            }
            let e = found.ok_or_else(|| format!("node {kind} at byte {s}: no end"))?;
            raw.push(Raw { start: s, rank, kind: kind.clone(), end: e + 1 });
        }
    }
    let token_rank = decl.nodes.len();
    for t in tokens.iter().filter(|t| !decl.untoken.contains(&t.kind)) {
        raw.push(Raw { start: t.start, rank: token_rank, kind: t.kind.clone(), end: t.end });
    }
    raw.sort_by_key(|r| (r.start, r.rank));
    // A file with no node would need a zero-width root (tree-sitter's for an empty file).
    if raw.is_empty() {
        return Err("no nodes: the root would be zero-width".into());
    }
    if let Some(w) = raw.windows(2).find(|w| (w[0].start, w[0].rank) == (w[1].start, w[1].rank)) {
        return Err(format!("two nodes of one rank at byte {}: {} and {}", w[0].start, w[0].kind, w[1].kind));
    }
    // C[p]: nodes starting before byte p.
    let mut c = vec![0usize; n + 1];
    for r in &raw {
        c[r.start + 1] += 1;
    }
    for p in 1..=n {
        c[p] += c[p - 1];
    }
    let mut out = Vec::with_capacity(raw.len());
    for (ord, r) in raw.iter().enumerate() {
        if r.end <= r.start {
            return Err(format!("a zero-width {} at byte {}", r.kind, r.start));
        }
        let size = c[r.end] - ord;
        let at = r.start;
        let field = decl.facts.iter().find(|f| f.0 && f.2.contains(&r.kind) && f.3.get(at)).map(|f| f.1.clone());
        let supertypes = decl.facts.iter().filter(|f| !f.0 && f.2.contains(&r.kind) && f.3.get(at)).map(|f| f.1.clone()).collect();
        let extra = decl.extra.contains(&r.kind);
        out.push(Node { kind: r.kind.clone(), field, supertypes, extra, start: r.start, end: r.end, size });
    }
    // The nesting check: each node lies inside the nearest earlier node that contains its start,
    // and its size counts exactly the nodes that start inside it.
    let mut open: Vec<usize> = Vec::new();
    for (i, node) in out.iter().enumerate() {
        while let Some(&p) = open.last() {
            if i < p + out[p].size {
                break;
            }
            open.pop();
        }
        if let Some(&p) = open.last() {
            if node.end > out[p].end {
                return Err(format!("{} at byte {} ends after its parent {} ends", node.kind, node.start, out[p].kind));
            }
        }
        if node.size == 0 {
            return Err(format!("{} at byte {}: size 0", node.kind, node.start));
        }
        open.push(i);
    }
    Ok(out)
}
