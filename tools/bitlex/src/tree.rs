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
    if tokens.is_empty() {
        return Err("no tokens: the root would be zero-width".into());
    }
    let mut p = Parser { decl, tokens, out: Vec::new() };
    match p.matches(&Gram::Sym(root), 0) {
        Some(next) if next == tokens.len() => {
            // The root runs from its first token to the end of the file, as tree-sitter's.
            p.out[0].end = n;
            Ok(p.out)
        }
        Some(next) => Err(format!("the token at byte {} is past the grammar's end", tokens[next].start)),
        None => Err("no parse: the tokens do not match the grammar".into()),
    }
}
