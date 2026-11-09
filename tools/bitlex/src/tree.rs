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
//! - **A whole root** (`node K whole`) spans the file from byte 0 to its end, and may hold no
//!   token: an empty file, or one of extras alone, is its root.
//!
//! A file with no parse goes to tree-sitter.

use crate::eval::{Token, TreeDecl};
use crate::syntax::Gram;
use std::collections::HashMap;

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
    /// The marks' kinds (bit `k`: mark `k`), and which are hidden.
    marks: &'a [String],
    hidden: u64,
    /// Each rule's and choice's first marks and whether it may be empty (`first_of`).
    named: HashMap<&'a str, (u64, bool)>,
}

impl<'a> Parser<'a> {
    fn mark_bit(&self, kind: &str) -> u64 {
        self.marks.iter().position(|m| m == kind).map_or(0, |k| 1 << k)
    }

    /// The marks `g` may start with, and whether it may match nothing.
    fn first(&self, g: &Gram) -> (u64, bool) {
        match g {
            Gram::Lit(k) => (self.mark_bit(k), false),
            Gram::Sym(name) => self.named.get(name.as_str()).copied().unwrap_or((self.mark_bit(name), false)),
            Gram::Seq(items) => self.first_seq(items),
            Gram::Alt(alts) => alts.iter().fold((0, false), |(b, e), a| {
                let (x, y) = self.first(a);
                (b | x, e || y)
            }),
            Gram::Opt(a) | Gram::Star(a) => (self.first(a).0, true),
            Gram::Plus(a) | Gram::Field(_, a) => self.first(a),
        }
    }

    fn first_seq(&self, items: &[Gram]) -> (u64, bool) {
        let mut bits = 0;
        for i in items {
            let (b, empty) = self.first(i);
            bits |= b;
            if !empty {
                return (bits, false);
            }
        }
        (bits, true)
    }

    /// Match `g` from token `at`: the next token, or `None`. `follow`: the marks that may come
    /// after `g` here. Nodes are appended to `out`; on a failed match the caller truncates them.
    fn matches(&mut self, g: &Gram, at: usize, follow: u64) -> Option<usize> {
        match g {
            Gram::Lit(k) => self.token(k, at, follow),
            Gram::Sym(name) => {
                let decl = self.decl;
                if let Some((kind, _, body)) = decl.rules.iter().find(|r| &r.0 == name) {
                    let me = self.out.len();
                    self.out.push(Node { kind: kind.clone(), field: None, supertypes: vec![], extra: false, start: 0, end: 0, size: 0 });
                    let next = self.matches(body, at, follow)?;
                    // Its span is its tokens', marks left out: a rule of marks alone has none.
                    let first = (at..next).find(|&i| !self.tokens[i].mark)?;
                    let last = (at..next).rev().find(|&i| !self.tokens[i].mark)?;
                    let n = self.out.len();
                    let node = &mut self.out[me];
                    node.start = self.tokens[first].start;
                    node.end = self.tokens[last].end;
                    node.size = n - me;
                    Some(next)
                } else if let Some((_, supertype, body)) = decl.choices.iter().find(|c| &c.0 == name) {
                    let me = self.out.len();
                    let next = self.matches(body, at, follow)?;
                    if let (Some(s), true) = (supertype, me < self.out.len()) {
                        self.out[me].supertypes.insert(0, s.clone());
                    }
                    Some(next)
                } else {
                    self.token(name, at, follow)
                }
            }
            Gram::Seq(items) => {
                // Each item's follow: what may start the items after it, and `follow` past them.
                let mut follows = vec![follow; items.len()];
                let mut after = follow;
                for (i, item) in items.iter().enumerate().rev() {
                    follows[i] = after;
                    let (b, empty) = self.first(item);
                    after = if empty { b | after } else { b };
                }
                let mut p = at;
                for (item, f) in items.iter().zip(follows) {
                    p = self.matches(item, p, f)?;
                }
                Some(p)
            }
            Gram::Alt(alts) => {
                let mark = self.out.len();
                for a in alts {
                    if let Some(p) = self.matches(a, at, follow) {
                        return Some(p);
                    }
                    self.out.truncate(mark);
                }
                None
            }
            Gram::Opt(a) => {
                let mark = self.out.len();
                match self.matches(a, at, follow) {
                    Some(p) => Some(p),
                    None => {
                        self.out.truncate(mark);
                        Some(at)
                    }
                }
            }
            Gram::Star(a) | Gram::Plus(a) => {
                let again = self.first(a).0 | follow;
                let mut p = if matches!(g, Gram::Plus(_)) { self.matches(a, at, again)? } else { at };
                loop {
                    let mark = self.out.len();
                    match self.matches(a, p, again) {
                        Some(q) if q > p => p = q,
                        _ => {
                            self.out.truncate(mark);
                            return Some(p);
                        }
                    }
                }
            }
            Gram::Field(f, a) => {
                let me = self.out.len();
                let next = self.matches(a, at, follow)?;
                if me < self.out.len() {
                    self.out[me].field = Some(f.clone());
                }
                Some(next)
            }
        }
    }

    /// A token of `kind` at `at`: its leaf (a mark has none), then past the marks that `follow`
    /// does not hold. A hidden one is skipped; one not hidden fails the parse.
    fn token(&mut self, kind: &str, at: usize, follow: u64) -> Option<usize> {
        let t = self.tokens.get(at).filter(|t| t.kind == kind)?;
        if !t.mark {
            let extra = self.decl.extra.contains(&t.kind);
            self.out.push(Node { kind: t.kind.clone(), field: None, supertypes: vec![], extra, start: t.start, end: t.end, size: 1 });
        }
        self.past_marks(at + 1, follow)
    }

    /// From `at`, past the marks the grammar can not take here (`follow`): hidden ones are
    /// skipped; one not hidden, `None`.
    fn past_marks(&self, mut at: usize, follow: u64) -> Option<usize> {
        while let Some(t) = self.tokens.get(at).filter(|t| t.mark) {
            let bit = self.mark_bit(&t.kind);
            if follow & bit != 0 {
                break;
            }
            if self.hidden & bit == 0 {
                return None;
            }
            at += 1;
        }
        Some(at)
    }
}

/// Each rule's and choice's first marks and whether it may match nothing: a fixpoint, as rules
/// may use each other.
fn first_of<'a>(p: &mut Parser<'a>) {
    let names: Vec<(&'a str, &'a Gram)> =
        p.decl.rules.iter().map(|r| (r.0.as_str(), &r.2)).chain(p.decl.choices.iter().map(|c| (c.0.as_str(), &c.2))).collect();
    for (n, _) in &names {
        p.named.insert(n, (0, false));
    }
    loop {
        let mut changed = false;
        for &(n, g) in &names {
            let f = p.first(g);
            if p.named[n] != f {
                p.named.insert(n, f);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// The nodes of `decl` over `tokens` (a file of `n` bytes), or the reason the file goes to
/// tree-sitter.
pub fn nodes(decl: &TreeDecl, tokens: &[Token], n: usize) -> Result<Vec<Node>, String> {
    let root = decl.rules.first().ok_or("the tree section has no rule")?.0.clone();
    // The extras, each with the number of other tokens before it.
    // The extras, each with the number of other tokens before it (marks left out: they make no
    // leaf).
    let mut extras: Vec<(usize, &Token)> = Vec::new();
    let mut plain: Vec<Token> = Vec::new();
    let mut real = 0;
    for t in tokens {
        if decl.extra.contains(&t.kind) {
            extras.push((real, t));
        } else {
            real += usize::from(!t.mark);
            plain.push(t.clone());
        }
    }
    if real == 0 && !decl.whole {
        return Err("no tokens: the root would be zero-width".into());
    }
    let hidden = decl.marks.iter().enumerate().filter(|(_, m)| decl.hidden.contains(m)).fold(0u64, |h, (k, _)| h | 1 << k);
    if decl.marks.len() > 64 {
        return Err("more than 64 marks".into());
    }
    let mut p = Parser { decl, tokens: &plain, out: Vec::new(), marks: &decl.marks, hidden, named: HashMap::new() };
    first_of(&mut p);
    // A whole root is opened at byte 0 and matched by its content, which may be empty. Marks
    // before the first token are kept where the root can start with them.
    let root_sym = Gram::Sym(root.clone());
    let matched = if decl.whole {
        let body = &decl.rules[0].2;
        p.out.push(Node { kind: root.clone(), field: None, supertypes: vec![], extra: false, start: 0, end: 0, size: 0 });
        let first = p.first(body).0;
        p.past_marks(0, first).and_then(|at| p.matches(body, at, 0))
    } else {
        let first = p.first(&root_sym).0;
        match p.past_marks(0, first) {
            Some(at) => p.matches(&root_sym, at, 0),
            None => None,
        }
    };
    let mut out = match matched {
        Some(next) if next == plain.len() => p.out,
        Some(next) => return Err(format!("the token at byte {} is past the grammar's end", plain[next].start)),
        None => return Err("no parse: the tokens do not match the grammar".into()),
    };
    // The root runs from its first token (or byte 0, when whole) to the end of the file.
    out[0].end = n;
    if decl.whole {
        out[0].start = 0;
        out[0].size = out.len();
    }
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
        } else if k == real {
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
