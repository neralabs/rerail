//! A `tree` section's nodes, by its grammar (docs/parser-generators/18-tree-as-grammar.md): the
//! reference. It parses the token stream one token at a time, with no language in it.
//!
//! - **Rules.** `node K = G` spans what `G` matches; the first rule is the root, and it must match
//!   every token. The root runs to the end of the file. A choice is its alternatives, each one's node in the choice's supertype.
//! - **Tokens.** A quoted token matches a token of that kind (a keyword's); a name matches a
//!   token of that kind, or a rule or a choice of that name. Every token is a leaf node.
//! - **Matching** is ordered and greedy: an alternative, an option or a repetition takes the first
//!   way that matches, and does not back out of it. For a grammar that is LL(1) per level, as the
//!   compiler requires, that is the grammar's one parse.
//!
//! - **Extras** (`extra K`) are left out of the parse, then each is placed as tree-sitter's
//!   parser places it: in the lowest node holding the tokens on either side of it, after the
//!   nodes that end before it and before those that start after it; at either end of the file,
//!   in the root (which then starts at the first extra).
//!
//! A file with no parse goes to tree-sitter.

use crate::eval::{Token, TreeDecl};
use crate::syntax::Gram;

/// A node, in preorder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: String,
    pub field: Option<String>,
    /// Supertype names, outermost choice first.
    pub supertypes: Vec<String>,
    pub extra: bool,
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
    /// The node and every node inside it.
    pub size: usize,
}

struct Parser<'a> {
    decl: &'a TreeDecl,
    tokens: &'a [Token],
    out: Vec<Node>,
}

impl Parser<'_> {
    /// Match `g` from token `at`: the next token, or `None`. Nodes are appended to `out`; on a
    /// failed match the caller truncates them.
    fn matches(&mut self, g: &Gram, at: usize) -> Option<usize> {
        match g {
            Gram::Lit(k) => self.token(k, at),
            Gram::Sym(name) => {
                if let Some((kind, _, body)) = self.decl.rules.iter().find(|r| &r.0 == name) {
                    let (kind, body) = (kind.clone(), body.clone());
                    let me = self.out.len();
                    self.out.push(Node { kind, field: None, supertypes: vec![], extra: false, start: 0, end: 0, size: 0 });
                    let next = self.matches(&body, at)?;
                    if next == at {
                        return None;
                    }
                    let n = self.out.len();
                    let node = &mut self.out[me];
                    node.start = self.tokens[at].start;
                    node.end = self.tokens[next - 1].end;
                    node.size = n - me;
                    Some(next)
                } else if let Some((_, supertype, body)) = self.decl.choices.iter().find(|c| &c.0 == name) {
                    let (supertype, body) = (supertype.clone(), body.clone());
                    let me = self.out.len();
                    let next = self.matches(&body, at)?;
                    if let (Some(s), true) = (supertype, me < self.out.len()) {
                        self.out[me].supertypes.insert(0, s);
                    }
                    Some(next)
                } else {
                    self.token(name, at)
                }
            }
            Gram::Seq(items) => {
                let mut p = at;
                for item in items {
                    p = self.matches(item, p)?;
                }
                Some(p)
            }
            Gram::Alt(alts) => {
                let mark = self.out.len();
                for a in alts {
                    if let Some(p) = self.matches(a, at) {
                        return Some(p);
                    }
                    self.out.truncate(mark);
                }
                None
            }
            Gram::Opt(a) => {
                let mark = self.out.len();
                match self.matches(a, at) {
                    Some(p) => Some(p),
                    None => {
                        self.out.truncate(mark);
                        Some(at)
                    }
                }
            }
            Gram::Star(a) => {
                let mut p = at;
                loop {
                    let mark = self.out.len();
                    match self.matches(a, p) {
                        Some(q) if q > p => p = q,
                        _ => {
                            self.out.truncate(mark);
                            return Some(p);
                        }
                    }
                }
            }
            Gram::Plus(a) => {
                let p = self.matches(a, at)?;
                self.matches(&Gram::Star(a.clone()), p)
            }
            Gram::Field(f, a) => {
                let me = self.out.len();
                let next = self.matches(a, at)?;
                if me < self.out.len() {
                    self.out[me].field = Some(f.clone());
                }
                Some(next)
            }
        }
    }

    fn token(&mut self, kind: &str, at: usize) -> Option<usize> {
        let t = self.tokens.get(at).filter(|t| t.kind == kind)?;
        let extra = self.decl.extra.contains(&t.kind);
        self.out.push(Node { kind: t.kind.clone(), field: None, supertypes: vec![], extra, start: t.start, end: t.end, size: 1 });
        Some(at + 1)
    }
}

/// The nodes of `decl` over `tokens` (a file of `n` bytes), or the reason the file goes to
/// tree-sitter.
pub fn nodes(decl: &TreeDecl, tokens: &[Token], n: usize) -> Result<Vec<Node>, String> {
    let root = decl.rules.first().ok_or("the tree section has no rule")?.0.clone();
    // The extras, each with the number of other tokens before it.
    let mut extras: Vec<(usize, &Token)> = Vec::new();
    let mut plain: Vec<Token> = Vec::new();
    for t in tokens {
        if decl.extra.contains(&t.kind) {
            extras.push((plain.len(), t));
        } else {
            plain.push(t.clone());
        }
    }
    if plain.is_empty() {
        return Err("no tokens: the root would be zero-width".into());
    }
    let mut p = Parser { decl, tokens: &plain, out: Vec::new() };
    let mut out = match p.matches(&Gram::Sym(root), 0) {
        Some(next) if next == plain.len() => p.out,
        Some(next) => return Err(format!("the token at byte {} is past the grammar's end", plain[next].start)),
        None => return Err("no parse: the tokens do not match the grammar".into()),
    };
    // The root runs from its first token to the end of the file, as tree-sitter's.
    out[0].end = n;
    if extras.is_empty() {
        return Ok(out);
    }
    // Each node's parent, and each token's leaf (the leaves are the nodes of size 1, in order).
    let mut parent = vec![usize::MAX; out.len()];
    let mut open: Vec<usize> = Vec::new();
    for i in 0..out.len() {
        while let Some(&o) = open.last() {
            if i < o + out[o].size {
                break;
            }
            open.pop();
        }
        if let Some(&o) = open.last() {
            parent[i] = o;
        }
        open.push(i);
    }
    let leaves: Vec<usize> = (0..out.len()).filter(|&i| out[i].size == 1).collect();
    let ancestors = |mut i: usize| {
        let mut up = vec![i];
        while parent[i] != usize::MAX {
            i = parent[i];
            up.push(i);
        }
        up
    };
    // Where each goes, in the tree as parsed: (index to insert at, the node it is in).
    let mut places: Vec<(usize, usize)> = Vec::new();
    for &(k, _) in &extras {
        places.push(if k == 0 {
            (1, 0)
        } else if k == plain.len() {
            (out.len(), 0)
        } else {
            let (before, after) = (ancestors(leaves[k - 1]), ancestors(leaves[k]));
            let lca = *before.iter().find(|a| after.contains(a)).expect("the root holds both");
            // Before the child of the common node that holds the token after it.
            let child = after[after.iter().position(|&a| a == lca).expect("an ancestor") - 1];
            (child, lca)
        });
    }
    // Inserted from the last, so that each place is still the tree's as parsed; extras at one
    // place keep their order.
    for (&(k, t), &(at, within)) in extras.iter().zip(&places).rev() {
        let _ = k;
        for a in ancestors(within) {
            out[a].size += 1;
        }
        out.insert(at, Node { kind: t.kind.clone(), field: None, supertypes: vec![], extra: true, start: t.start, end: t.end, size: 1 });
    }
    out[0].start = out[0].start.min(extras[0].1.start);
    Ok(out)
}
