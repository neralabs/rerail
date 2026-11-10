//! The tree section's grammar to tier 2's state machine (docs/parser-generators/18-tree-as-grammar.md
//! §2): a pass of the Bitlex compiler. Its output is the block program's `states` and `arms`.
//!
//! 1. **Levels.** The root rule and each `nested` rule is a level, and so is each rule or choice
//!    whose calls save enough states (`shared`). Every other rule and every choice is inlined
//!    into the levels that use it; a use of a level is a call.
//! 2. **Positions.** Glushkov's construction over each level's expression: a position per token
//!    and per call; each edge between two positions knows the wrappers (nodes, fields,
//!    supertypes) it leaves and enters.
//! 3. **States.** A state is "just took position p". Each token from it is one arm: the edge's
//!    closes, its opens (each node with the field and supertypes on the way to it), the token's
//!    leaf, the next state. An arm into a call opens the called node, pushes the call's position
//!    as the return state, and continues with the called level's first edge.
//! 4. **Checks.** Two arms of one state on one token: the grammar is not LL(1), refused.
//! 5. **Merging.** States that act alike on every token are one (Moore's refinement).

use bitlex::syntax::{Gram, Stmt};
use std::collections::HashMap;

/// A state of tier 2's state machine (docs/parser-generators/18-tree-as-grammar.md §2): the
/// tree section's grammar, one level per nested node. State 0 is the root's start.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StateRow {
    /// The node kind of its level: the root's, or a nested node's (a nested choice's name).
    pub level: u16,
    /// Its level has a node of its own: not a nested choice's, which opens none.
    pub own: bool,
    /// Its level may end here, after this many closes inside it: a nested level then closes its
    /// node and pops (at once, when the state has no arms); the root accepts at the end of the
    /// file. `None`: it may not end here.
    pub end: Option<u8>,
    /// The hidden marks its level refuses (`refuse M in L`): before a token it takes, each
    /// refuses the file.
    pub refuses: Vec<u16>,
}

/// What a transition does, in order: close nodes (innermost first), open nodes (outermost
/// first), then the token's own leaf.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Act {
    /// Close the innermost open node, at the end of the token before.
    Close,
    /// Open a node of `kind` at the token's start, with a field (0: none) and supertypes
    /// (outermost first).
    Open { kind: u16, field: u16, supers: Vec<u16> },
    /// The token's leaf node, of the token's kind, or of `kind` when it is not 0 (`A as K`).
    Leaf { field: u16, supers: Vec<u16>, kind: u16 },
    /// An extra's leaf, deferred: the nodes closed before the next node leave it out, so it goes
    /// where the next token puts it (`with_extras`).
    Extra,
    /// An extra's leaf, placed now (`with_extras`): no node of the grammar's, so never the node
    /// made last at its level, which a `wrap` or a `relabel` acts on.
    ExtraNow,
    /// The innermost open node becomes a `kind` (`retag K A`).
    Retag { kind: u16 },
    /// A node of `kind` around the node just made at this level, open (`wrap K F [as M]`): it
    /// takes that node's field, which takes `field` and, when `moved` is not 0, kind `moved`.
    Wrap { kind: u16, field: u16, moved: u16 },
    /// The node just made at this level takes `field` and, when `moved` is not 0, kind `moved`;
    /// when `inner` is not 0, the node its wrap holds first takes field `inner` (`relabel F [as M]
    /// [inner G]`).
    Relabel { field: u16, moved: u16, inner: u16 },
    /// The token's leaf joins the grammar's leaf before it, as one leaf of `kind`; the extras
    /// between are its text (`merge 'K' A`).
    Merge { kind: u16 },
    /// The innermost open node holds two leaves and nothing else: they become one leaf of
    /// `kind`, with the first one's field (`fold K`).
    Fold { kind: u16 },
    /// The node just made at this level and the nodes inside it, read by reading `reading`
    /// (`reread R`).
    Reread { reading: u16 },
    /// The token taken is its level's word, of kind `kind` (a `remember` choice's): a kind
    /// `stream_ir::REMEMBERED` names it from here, until the next.
    Remember { kind: u16 },
}
/// A transition: from a state, on a token of a kind, its acts, to a state; `push`: the calls it
/// makes, each entering a nested level and keeping the state to return to, outermost first (a
/// call at a level's first token makes one inside another).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArmRow {
    pub from: u16,
    pub token: u16,
    pub acts: Vec<Act>,
    pub to: u16,
    pub push: Vec<u16>,
}

/// The state machine as text, a line per state and per arm, for tests and dumps:
/// `state S level K [end C]`; `arm S token K to T [push P] do ACT…`, an act `close`,
/// `open K [f F] [s K]…` or `leaf [f F] [s K]…`.
pub fn text(states: &[StateRow], arms: &[ArmRow]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (s, state) in states.iter().enumerate() {
        let end = state.end.map_or(String::new(), |c| format!(" end {c}"));
        let hidden = if state.own { "" } else { " hidden" };
        let _ = writeln!(out, "state {s} level {}{hidden}{end}", state.level);
    }
    let marks = |field: u16, supers: &[u16]| {
        let mut m = String::new();
        if field != 0 {
            let _ = write!(m, " f {field}");
        }
        for s in supers {
            let _ = write!(m, " s {s}");
        }
        m
    };
    for arm in arms {
        let mut acts = String::new();
        for act in &arm.acts {
            match act {
                Act::Close => acts.push_str(" close"),
                Act::Open { kind, field, supers } => {
                    let _ = write!(acts, " open {kind}{}", marks(*field, supers));
                }
                Act::Leaf { field, supers, kind } => {
                    let as_kind = if *kind == 0 { String::new() } else { format!(" as {kind}") };
                    let _ = write!(acts, " leaf{as_kind}{}", marks(*field, supers));
                }
                Act::Wrap { kind, field, moved } => {
                    let _ = write!(acts, " wrap {kind} f {field} as {moved}");
                }
                Act::Relabel { field, moved, inner } => {
                    let _ = write!(acts, " relabel f {field} as {moved} inner {inner}");
                }
                Act::Extra => acts.push_str(" extra"),
                Act::ExtraNow => acts.push_str(" extra now"),
                Act::Retag { kind } => {
                    let _ = write!(acts, " retag {kind}");
                }
                Act::Merge { kind } => {
                    let _ = write!(acts, " merge {kind}");
                }
                Act::Fold { kind } => {
                    let _ = write!(acts, " fold {kind}");
                }
                Act::Reread { reading } => {
                    let _ = write!(acts, " reread {reading}");
                }
                Act::Remember { kind } => {
                    let _ = write!(acts, " remember {kind}");
                }
            }
        }
        let push: String = arm.push.iter().map(|p| format!(" push {p}")).collect();
        let _ = writeln!(out, "arm {} token {} to {}{push} do{acts}", arm.from, arm.token, arm.to);
    }
    out
}

/// A wrapper on the way to a position: a node, a field of the node inside it, a supertype of
/// the node inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wrap {
    Node(u16),
    Field(u16),
    Super(u16),
    /// `retag K`: entered, the innermost open node becomes a `K`.
    Retag(u16),
    /// `wrap K F [as M]`: entered, a node around the node just made at this level; left, it
    /// closes.
    Wrapped { kind: u16, field: u16, moved: u16 },
    /// `relabel F [as M]`: entered, the node just made at this level takes field `F` and kind
    /// `M`; no node, so nothing closes.
    Relabel { field: u16, moved: u16, inner: u16 },
    /// `fold K`: entered, the open node's two leaves become one of kind `K`.
    Fold(u16),
    /// `reread R`: entered, the node just made at this level is read by reading `R`.
    Reread(u16),
}

/// A level's expression, inlined; `Tok` and `Call` are positions.
enum E {
    Tok(usize),
    Call(usize),
    Seq(Vec<E>),
    Alt(Vec<E>),
    Opt(Box<E>),
    Star(Box<E>),
    Wrap(Wrap, Box<E>),
}

/// A position: the token kind it takes, or the level it calls; or a token whose leaf merges into
/// the one before it, as a leaf of `into`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pos {
    /// A token kind, its leaf's kind (0: the token's), and whether it is remembered (a
    /// `remember` choice's token: its level's word from here).
    Tok(u16, u16, bool),
    Call(usize),
    Merge { token: u16, into: u16 },
}

/// A rule of the tree section.
struct Rule {
    kind: String,
    nested: bool,
    whole: bool,
    body: Gram,
    /// A node's rule; not a nested choice's (`choice N nested`), a level with no node.
    own: bool,
    /// A nested choice's supertype.
    supertype: Option<String>,
}

/// The ids of the grammar's names: kinds (token kinds, node kinds, supertypes) and fields.
pub struct Ids<'a> {
    pub kind: &'a dyn Fn(&str) -> Result<u16, String>,
    pub field: &'a dyn Fn(&str) -> Result<u16, String>,
    /// Whether a name is a token kind the lexer makes (a token's, or a keyword's).
    pub is_token: &'a dyn Fn(&str) -> bool,
    /// Every token kind the grammar may see (marks aside): what a `balanced` leaf takes.
    pub tokens: &'a [String],
    /// A reading's id, by name (`reread R`).
    pub reading: &'a dyn Fn(&str) -> Result<u16, String>,
}

/// The level a `balanced` leaf's name: its kind, then a byte no name holds.
const BALANCED: char = '\0';

/// `g` with each `balanced K O C` as its opener (a leaf of kind `K`) and a call of a nested level
/// that merges every token into that leaf through the matching closer; each such level's `(K, O,
/// C)` goes to `levels` once.
fn desugar(g: &Gram, levels: &mut Vec<(String, String, String)>, tokens: &[String]) -> Gram {
    let mut d = |g: &Gram| desugar(g, levels, tokens);
    // `as K` and `merge K` over `any`: one alternative per token kind.
    let each = |a: Gram, f: &dyn Fn(Gram) -> Gram| match a {
        Gram::Alt(xs) => Gram::Alt(xs.into_iter().map(f).collect()),
        a => f(a),
    };
    match g {
        Gram::Lit(_) | Gram::Sym(_) => g.clone(),
        Gram::Seq(gs) => Gram::Seq(gs.iter().map(&mut d).collect()),
        Gram::Alt(gs) => Gram::Alt(gs.iter().map(&mut d).collect()),
        Gram::Opt(a) => Gram::Opt(Box::new(d(a))),
        Gram::Star(a) => Gram::Star(Box::new(d(a))),
        Gram::Plus(a) => Gram::Plus(Box::new(d(a))),
        Gram::Field(f, a) => Gram::Field(f.clone(), Box::new(d(a))),
        Gram::Retag(k, a) => Gram::Retag(k.clone(), Box::new(d(a))),
        Gram::Merge(k, a) if matches!(**a, Gram::Any(_)) => each(d(a), &|x| Gram::Merge(k.clone(), Box::new(x))),
        Gram::As(k, a) if matches!(**a, Gram::Any(_)) => each(d(a), &|x| Gram::As(k.clone(), Box::new(x))),
        Gram::Merge(k, a) => Gram::Merge(k.clone(), Box::new(d(a))),
        Gram::As(k, a) => Gram::As(k.clone(), Box::new(d(a))),
        Gram::Any(except) => Gram::Alt(tokens.iter().filter(|t| !except.contains(t)).map(|t| Gram::Sym(t.clone())).collect()),
        Gram::Wrap { kind, field, moved, body } => Gram::Wrap { kind: kind.clone(), field: field.clone(), moved: moved.clone(), body: Box::new(d(body)) },
        Gram::Relabel { field, moved, inner, body } => Gram::Relabel { field: field.clone(), moved: moved.clone(), inner: inner.clone(), body: Box::new(d(body)) },
        Gram::Fold { kind, body } => Gram::Fold { kind: kind.clone(), body: Box::new(d(body)) },
        Gram::Reread { reading, body } => Gram::Reread { reading: reading.clone(), body: Box::new(d(body)) },
        Gram::Balanced { kind, open, close } => {
            let key = (kind.clone(), open.clone(), close.clone());
            let n = levels.iter().position(|l| *l == key).unwrap_or_else(|| {
                levels.push(key);
                levels.len() - 1
            });
            Gram::Seq(vec![Gram::As(kind.clone(), Box::new(Gram::Lit(open.clone()))), Gram::Sym(format!("{kind}{BALANCED}{n}"))])
        }
    }
}

/// An edge: from a position (or the level's start), the wrappers it leaves (innermost first) and
/// enters (outermost first), to a position (or the level's end).
struct Edge {
    from: Option<usize>,
    exits: Vec<Wrap>,
    enters: Vec<Wrap>,
    to: Option<usize>,
}

struct Level {
    /// The level's node kind (its rule's).
    kind: u16,
    /// It opens a node of `kind` when called; a nested choice's does not: the call's field and
    /// supertypes (and the choice's) go to the level's first node.
    own: bool,
    supertype: Option<u16>,
    edges: Vec<Edge>,
    /// The marks it refuses (`refuse M in L`).
    refuses: Vec<u16>,
}

struct Builder<'a> {
    rules: Vec<Rule>,
    choices: Vec<(String, Option<String>, Gram)>,
    ids: &'a Ids<'a>,
    positions: Vec<(usize, Pos)>,
    /// The `remember` choices.
    remember: Vec<String>,
}

impl Builder<'_> {
    /// `g` inlined into level `level`; `inlining` guards against a non-nested rule that uses itself.
    fn expand(&mut self, level: usize, g: &Gram, inlining: &mut Vec<String>) -> Result<E, String> {
        Ok(match g {
            Gram::Lit(text) => self.tok(level, text)?,
            Gram::Sym(name) => {
                if let Some(r) = self.rules.iter().position(|r| &r.kind == name) {
                    if self.rules[r].nested || r == 0 {
                        self.positions.push((level, Pos::Call(r)));
                        E::Call(self.positions.len() - 1)
                    } else {
                        if inlining.contains(name) {
                            return Err(format!("node {name} uses itself: mark it `nested`"));
                        }
                        inlining.push(name.clone());
                        let body = self.rules[r].body.clone();
                        let kind = (self.ids.kind)(name)?;
                        let e = self.expand(level, &body, inlining)?;
                        inlining.pop();
                        E::Wrap(Wrap::Node(kind), Box::new(e))
                    }
                } else if let Some(c) = self.choices.iter().position(|c| &c.0 == name) {
                    if inlining.contains(name) {
                        return Err(format!("choice {name} uses itself"));
                    }
                    inlining.push(name.clone());
                    let (supertype, body) = (self.choices[c].1.clone(), self.choices[c].2.clone());
                    let from = self.positions.len();
                    let e = self.expand(level, &body, inlining)?;
                    inlining.pop();
                    // A `remember` choice: each token it may take is its level's word.
                    if self.remember.contains(name) {
                        for p in from..self.positions.len() {
                            let Pos::Tok(token, leaf, _) = self.positions[p].1 else { return Err(format!("remember {name}: a choice of tokens only")) };
                            self.positions[p].1 = Pos::Tok(token, leaf, true);
                        }
                    }
                    match supertype {
                        Some(s) => E::Wrap(Wrap::Super((self.ids.kind)(&s)?), Box::new(e)),
                        None => e,
                    }
                } else {
                    self.tok(level, name)?
                }
            }
            Gram::Seq(items) => E::Seq(items.iter().map(|i| self.expand(level, i, inlining)).collect::<Result<_, _>>()?),
            Gram::Alt(alts) => E::Alt(alts.iter().map(|a| self.expand(level, a, inlining)).collect::<Result<_, _>>()?),
            Gram::Opt(a) => E::Opt(Box::new(self.expand(level, a, inlining)?)),
            Gram::Star(a) => E::Star(Box::new(self.expand(level, a, inlining)?)),
            // `A+` is `A A*`: two copies, each with positions of its own.
            Gram::Plus(a) => E::Seq(vec![self.expand(level, a, inlining)?, E::Star(Box::new(self.expand(level, a, inlining)?))]),
            Gram::Field(f, a) => E::Wrap(Wrap::Field((self.ids.field)(f)?), Box::new(self.expand(level, a, inlining)?)),
            Gram::Retag(k, a) => E::Wrap(Wrap::Retag((self.ids.kind)(k)?), Box::new(self.expand(level, a, inlining)?)),
            Gram::Wrap { kind, field, moved, body } => {
                let w = Wrap::Wrapped {
                    kind: (self.ids.kind)(kind)?,
                    field: field.as_deref().map(|f| (self.ids.field)(f)).transpose()?.unwrap_or(0),
                    moved: moved.as_deref().map(|m| self.kind(m)).transpose()?.unwrap_or(0),
                };
                E::Wrap(w, Box::new(self.expand(level, body, inlining)?))
            }
            Gram::Relabel { field, moved, inner, body } => {
                let w = Wrap::Relabel {
                    field: field.as_deref().map(|f| (self.ids.field)(f)).transpose()?.unwrap_or(0),
                    moved: moved.as_deref().map(|m| self.kind(m)).transpose()?.unwrap_or(0),
                    inner: inner.as_deref().map(|f| (self.ids.field)(f)).transpose()?.unwrap_or(0),
                };
                E::Wrap(w, Box::new(self.expand(level, body, inlining)?))
            }
            Gram::Fold { kind, body } => E::Wrap(Wrap::Fold((self.ids.kind)(kind)?), Box::new(self.expand(level, body, inlining)?)),
            Gram::Reread { reading, body } => E::Wrap(Wrap::Reread((self.ids.reading)(reading)?), Box::new(self.expand(level, body, inlining)?)),
            Gram::As(k, a) => {
                let e = self.expand(level, a, inlining)?;
                // A token, or a choice of tokens (`aggregate_word as variable`): each one's leaf.
                fn toks(e: &E, out: &mut Vec<usize>) -> bool {
                    match e {
                        E::Tok(p) => {
                            out.push(*p);
                            true
                        }
                        E::Alt(alts) => alts.iter().all(|a| toks(a, out)),
                        _ => false,
                    }
                }
                let mut ps = Vec::new();
                if !toks(&e, &mut ps) {
                    return Err(format!("`as {k}`: not a token"));
                }
                let kind = self.kind(k)?;
                if kind == crate::stream_ir::REMEMBERED {
                    return Err(format!("`as {k}` on a token: a word's kind is for a wrap's, a relabel's or a reading's node"));
                }
                for p in ps {
                    let Pos::Tok(token, _, r) = self.positions[p].1 else { return Err(format!("`as {k}`: not a token")) };
                    self.positions[p].1 = Pos::Tok(token, kind, r);
                }
                e
            }
            Gram::Balanced { kind, .. } => return Err(format!("balanced {kind}: not desugared")),
            Gram::Any(_) => return Err("any: not desugared".into()),
            Gram::Merge(k, a) => {
                let name = match &**a {
                    Gram::Lit(n) | Gram::Sym(n) if (self.ids.is_token)(n) => n,
                    _ => return Err(format!("merge {k:?}: not a token")),
                };
                self.positions.push((level, Pos::Merge { token: (self.ids.kind)(name)?, into: (self.ids.kind)(k)? }));
                E::Tok(self.positions.len() - 1)
            }
        })
    }

    /// A kind named in the grammar; a `remember` choice's name is its level's word's
    /// (`stream_ir::REMEMBERED`).
    fn kind(&self, name: &str) -> Result<u16, String> {
        if self.remember.iter().any(|r| r == name) { Ok(crate::stream_ir::REMEMBERED) } else { (self.ids.kind)(name) }
    }

    fn tok(&mut self, level: usize, name: &str) -> Result<E, String> {
        if !(self.ids.is_token)(name) {
            return Err(format!("`{name}` is no rule, choice or token kind"));
        }
        self.positions.push((level, Pos::Tok((self.ids.kind)(name)?, 0, false)));
        Ok(E::Tok(self.positions.len() - 1))
    }
}

fn nullable(e: &E) -> bool {
    match e {
        E::Tok(_) | E::Call(_) => false,
        E::Seq(items) => items.iter().all(nullable),
        E::Alt(alts) => alts.iter().any(nullable),
        E::Opt(_) | E::Star(_) => true,
        E::Wrap(_, a) => nullable(a),
    }
}

/// The positions `e` may start with, each with the wrappers entered on the way (outermost first).
fn first(e: &E) -> Vec<(usize, Vec<Wrap>)> {
    match e {
        E::Tok(p) | E::Call(p) => vec![(*p, vec![])],
        E::Seq(items) => {
            let mut out = Vec::new();
            for i in items {
                out.extend(first(i));
                if !nullable(i) {
                    break;
                }
            }
            out
        }
        E::Alt(alts) => alts.iter().flat_map(first).collect(),
        E::Opt(a) | E::Star(a) => first(a),
        E::Wrap(w, a) => first(a)
            .into_iter()
            .map(|(p, mut path)| {
                path.insert(0, *w);
                (p, path)
            })
            .collect(),
    }
}

/// The positions `e` may end with, each with the wrappers left on the way out (innermost first).
fn last(e: &E) -> Vec<(usize, Vec<Wrap>)> {
    match e {
        E::Tok(p) | E::Call(p) => vec![(*p, vec![])],
        E::Seq(items) => {
            let mut out = Vec::new();
            for i in items.iter().rev() {
                out.extend(last(i));
                if !nullable(i) {
                    break;
                }
            }
            out
        }
        E::Alt(alts) => alts.iter().flat_map(last).collect(),
        E::Opt(a) | E::Star(a) => last(a),
        E::Wrap(w, a) => last(a)
            .into_iter()
            .map(|(p, mut path)| {
                path.push(*w);
                (p, path)
            })
            .collect(),
    }
}

/// The edges inside `e`.
fn follow(e: &E, out: &mut Vec<Edge>) {
    let link = |a: &E, b: &E, out: &mut Vec<Edge>| {
        for (p, exits) in last(a) {
            for (q, enters) in first(b) {
                out.push(Edge { from: Some(p), exits: exits.clone(), enters, to: Some(q) });
            }
        }
    };
    match e {
        E::Tok(_) | E::Call(_) => {}
        E::Seq(items) => {
            for i in items {
                follow(i, out);
            }
            // Each item to every later one reached over nullable items between.
            for (i, a) in items.iter().enumerate() {
                for b in &items[i + 1..] {
                    link(a, b, out);
                    if !nullable(b) {
                        break;
                    }
                }
            }
        }
        E::Alt(alts) => alts.iter().for_each(|a| follow(a, out)),
        E::Opt(a) => follow(a, out),
        E::Star(a) => {
            follow(a, out);
            link(a, a, out);
        }
        E::Wrap(_, a) => follow(a, out),
    }
}

/// Field and supertypes waiting for the next node on a path.
#[derive(Default, Clone)]
struct Pending {
    field: u16,
    supers: Vec<u16>,
}

impl Pending {
    /// Walk `enters`, emitting an open per node; what is left applies to the position's node.
    fn enter(&mut self, enters: &[Wrap], acts: &mut Vec<Act>) {
        for w in enters {
            match *w {
                Wrap::Field(f) => self.field = f,
                Wrap::Super(s) => self.supers.push(s),
                Wrap::Retag(kind) => acts.push(Act::Retag { kind }),
                Wrap::Wrapped { kind, field, moved } => acts.push(Act::Wrap { kind, field, moved }),
                Wrap::Relabel { field, moved, inner } => acts.push(Act::Relabel { field, moved, inner }),
                Wrap::Fold(kind) => acts.push(Act::Fold { kind }),
                Wrap::Reread(reading) => acts.push(Act::Reread { reading }),
                Wrap::Node(kind) => {
                    let p = std::mem::take(self);
                    acts.push(Act::Open { kind, field: p.field, supers: p.supers });
                }
            }
        }
    }
}

fn closes(exits: &[Wrap], acts: &mut Vec<Act>) {
    for w in exits {
        if matches!(w, Wrap::Node(_) | Wrap::Wrapped { .. }) {
            acts.push(Act::Close);
        }
    }
}

/// A state before merging: its level, the closes when its level ends after it (a pop, or the end
/// of the file at the root), and its arms: (token, acts, to, push).
struct Raw {
    level: usize,
    end: Option<u8>,
    arms: Vec<(u16, Vec<Act>, usize, Vec<usize>)>,
}

/// The tree section's rules, desugared: the nodes (the root first), the choices inlined where
/// used, the nested choices, and each `balanced` level's `(K, O, C)`.
fn gather(stmts: &[Stmt], tokens: &[String]) -> (Vec<Rule>, Vec<(String, Option<String>, Gram)>, Vec<Rule>, Vec<(String, String, String)>) {
    let mut rules = Vec::new();
    let mut choices = Vec::new();
    let mut nested_choices = Vec::new();
    let mut balanced = Vec::new();
    for s in stmts {
        match s {
            Stmt::Node { kind, nested, whole, body } => rules.push(Rule { kind: kind.clone(), nested: *nested, whole: *whole, body: desugar(body, &mut balanced, tokens), own: true, supertype: None }),
            Stmt::Choice { name, supertype, nested: true, body } => nested_choices.push(Rule { kind: name.clone(), nested: true, whole: false, body: desugar(body, &mut balanced, tokens), own: false, supertype: supertype.clone() }),
            Stmt::Choice { name, supertype, body, .. } => choices.push((name.clone(), supertype.clone(), desugar(body, &mut balanced, tokens))),
            _ => {}
        }
    }
    (rules, choices, nested_choices, balanced)
}

/// The fewest states a call must save for a rule to be shared by calls (`shared`).
pub const SHARED_STATES: usize = 8;

/// The states a rule's calls save: its inner positions once instead of at each of its `uses`,
/// less a state for each call. An inner position is one the rule may go on from (its state's
/// arms are the rule's own); a position it may end with is a state at each place either way
/// (its arms are the place's), so a choice of single tokens saves nothing.
pub fn saved_states(uses: usize, inner: usize) -> usize {
    (uses.saturating_sub(1) * inner).saturating_sub(uses)
}

/// The rules and choices not marked `nested` that become calls, as if they were: those whose
/// calls save `SHARED_STATES` states or more (`saved_states`), the rules they use decided first
/// (a call is one position). Inlined, each place has a copy of the rule's inner states; called,
/// its level has them once, and each use pays a push and a return when it runs: a few states
/// saved are not worth that. The rest stay inlined; so do those that may match no token (a call
/// takes one), those a refusing level uses (its marks would be the callee's, `refusing`), and
/// those whose first token merges into the leaf before them.
fn shared(rules: &[Rule], choices: &[(String, Option<String>, Gram)], levels: &[Rule], stmts: &[Stmt]) -> Vec<String> {
    #[derive(Default)]
    struct Facts {
        uses: HashMap<String, usize>,
        size: HashMap<String, usize>,
        called: HashMap<String, bool>,
        /// Inlined for good: what a refusing level reaches.
        kept: Vec<String>,
    }
    let body = |name: &str| -> Option<&Gram> {
        rules.iter().find(|r| r.kind == name).map(|r| &r.body).or_else(|| choices.iter().find(|c| c.0 == name).map(|c| &c.2))
    };
    let fixed = |name: &str| rules.iter().position(|r| r.kind == name).is_some_and(|r| r == 0 || rules[r].nested);
    // Uses: each place a body names it (a `+`'s twice: its body is expanded twice).
    fn count(g: &Gram, times: usize, uses: &mut HashMap<String, usize>) {
        match g {
            Gram::Sym(n) => *uses.entry(n.clone()).or_default() += times,
            Gram::Seq(gs) | Gram::Alt(gs) => gs.iter().for_each(|g| count(g, times, uses)),
            Gram::Plus(a) => count(a, 2 * times, uses),
            Gram::Opt(a) | Gram::Star(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => count(a, times, uses),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } => count(body, times, uses),
            Gram::Lit(_) | Gram::Any(_) | Gram::Balanced { .. } => {}
        }
    }
    let mut f = Facts::default();
    for g in rules.iter().chain(levels).map(|r| &r.body).chain(choices.iter().map(|c| &c.2)) {
        count(g, 1, &mut f.uses);
    }
    // A name's positions where it is used: one for a call or a token, its body's inlined.
    fn size(g: &Gram, f: &mut Facts, ctx: &dyn Fn(&str) -> (Option<Gram>, bool), stack: &mut Vec<String>) -> usize {
        match g {
            Gram::Lit(_) | Gram::Merge(..) | Gram::Any(_) | Gram::Balanced { .. } => 1,
            Gram::Sym(n) => match ctx(n) {
                (Some(b), false) if !stack.contains(n) => {
                    if decide(n, &b, f, ctx, stack) { 1 } else { f.size[n] }
                }
                _ => 1,
            },
            Gram::Seq(gs) | Gram::Alt(gs) => gs.iter().map(|g| size(g, f, ctx, stack)).sum(),
            Gram::Plus(a) => 2 * size(a, f, ctx, stack),
            Gram::Opt(a) | Gram::Star(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::As(_, a) => size(a, f, ctx, stack),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } => size(body, f, ctx, stack),
        }
    }
    // Whether `n` may match no token, and whether its first token merges into the leaf before.
    fn empty(g: &Gram, ctx: &dyn Fn(&str) -> (Option<Gram>, bool), called: &HashMap<String, bool>, stack: &mut Vec<String>) -> bool {
        match g {
            Gram::Lit(_) | Gram::Merge(..) | Gram::Any(_) | Gram::Balanced { .. } => false,
            Gram::Sym(n) => match ctx(n) {
                (Some(b), false) if !stack.contains(n) && !called.get(n).copied().unwrap_or(false) => {
                    stack.push(n.clone());
                    let e = empty(&b, ctx, called, stack);
                    stack.pop();
                    e
                }
                _ => false,
            },
            Gram::Seq(gs) => gs.iter().all(|g| empty(g, ctx, called, stack)),
            Gram::Alt(gs) => gs.iter().any(|g| empty(g, ctx, called, stack)),
            Gram::Opt(_) | Gram::Star(_) => true,
            Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::As(_, a) => empty(a, ctx, called, stack),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } => empty(body, ctx, called, stack),
        }
    }
    fn merges_first(g: &Gram, ctx: &dyn Fn(&str) -> (Option<Gram>, bool), called: &HashMap<String, bool>, stack: &mut Vec<String>) -> bool {
        match g {
            Gram::Merge(..) => true,
            Gram::Lit(_) | Gram::Any(_) | Gram::Balanced { .. } => false,
            Gram::Sym(n) => match ctx(n) {
                (Some(b), false) if !stack.contains(n) && !called.get(n).copied().unwrap_or(false) => {
                    stack.push(n.clone());
                    let m = merges_first(&b, ctx, called, stack);
                    stack.pop();
                    m
                }
                _ => false,
            },
            Gram::Seq(gs) => {
                for g in gs {
                    if merges_first(g, ctx, called, stack) {
                        return true;
                    }
                    if !empty(g, ctx, called, stack) {
                        return false;
                    }
                }
                false
            }
            Gram::Alt(gs) => gs.iter().any(|g| merges_first(g, ctx, called, stack)),
            Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::As(_, a) => merges_first(a, ctx, called, stack),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } => merges_first(body, ctx, called, stack),
        }
    }
    // The positions `g` may end with, the rules it uses decided.
    fn lasts(g: &Gram, ctx: &dyn Fn(&str) -> (Option<Gram>, bool), called: &HashMap<String, bool>, stack: &mut Vec<String>) -> usize {
        match g {
            Gram::Lit(_) | Gram::Merge(..) | Gram::Any(_) | Gram::Balanced { .. } => 1,
            Gram::Sym(n) => match ctx(n) {
                (Some(b), false) if !stack.contains(n) && !called.get(n).copied().unwrap_or(false) => {
                    stack.push(n.clone());
                    let k = lasts(&b, ctx, called, stack);
                    stack.pop();
                    k
                }
                _ => 1,
            },
            Gram::Seq(gs) => {
                let mut k = 0;
                for g in gs.iter().rev() {
                    k += lasts(g, ctx, called, stack);
                    if !empty(g, ctx, called, stack) {
                        break;
                    }
                }
                k
            }
            Gram::Alt(gs) => gs.iter().map(|g| lasts(g, ctx, called, stack)).sum(),
            Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::As(_, a) => lasts(a, ctx, called, stack),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } => lasts(body, ctx, called, stack),
        }
    }
    fn decide(n: &str, b: &Gram, f: &mut Facts, ctx: &dyn Fn(&str) -> (Option<Gram>, bool), stack: &mut Vec<String>) -> bool {
        if let Some(&c) = f.called.get(n) {
            return c;
        }
        stack.push(n.to_owned());
        let s = size(b, f, ctx, stack);
        let inner = s.saturating_sub(lasts(b, ctx, &f.called, stack));
        let e = empty(b, ctx, &f.called, stack) || merges_first(b, ctx, &f.called, stack);
        stack.pop();
        let c = !e && !f.kept.iter().any(|k| k == n) && saved_states(f.uses.get(n).copied().unwrap_or(0), inner) >= SHARED_STATES;
        f.size.insert(n.to_owned(), s);
        f.called.insert(n.to_owned(), c);
        c
    }
    let ctx = |n: &str| -> (Option<Gram>, bool) { (body(n).cloned(), fixed(n)) };
    // What a refusing level reaches by inlining, before any call: inlined there for good.
    let refusing: Vec<&str> = stmts.iter().filter_map(|s| if let Stmt::Refuse { levels, .. } = s { Some(levels.iter().map(String::as_str)) } else { None }).flatten().collect();
    fn reach(g: &Gram, ctx: &dyn Fn(&str) -> (Option<Gram>, bool), out: &mut Vec<String>) {
        match g {
            Gram::Sym(n) => {
                if let (Some(b), false) = ctx(n)
                    && !out.contains(n)
                {
                    out.push(n.clone());
                    reach(&b, ctx, out);
                }
            }
            Gram::Seq(gs) | Gram::Alt(gs) => gs.iter().for_each(|g| reach(g, ctx, out)),
            Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => reach(a, ctx, out),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } => reach(body, ctx, out),
            Gram::Lit(_) | Gram::Any(_) | Gram::Balanced { .. } => {}
        }
    }
    // What takes a remembered token, by inlining: inlined for good, as a call's level would
    // give its caller's word back when it ends.
    let remember: Vec<&String> = stmts.iter().filter_map(|s| if let Stmt::Remember(n) = s { Some(n.iter()) } else { None }).flatten().collect();
    for r in rules.iter().map(|r| r.kind.clone()).chain(choices.iter().map(|c| c.0.clone())) {
        let mut reached = Vec::new();
        if let Some(b) = body(&r) {
            reach(b, &ctx, &mut reached);
        }
        if remember.contains(&&r) || reached.iter().any(|n| remember.contains(&n)) {
            f.kept.push(r);
        }
    }
    for l in &refusing {
        if let Some(b) = rules.iter().chain(levels).find(|r| r.kind == *l).map(|r| &r.body) {
            reach(b, &ctx, &mut f.kept);
        }
    }
    let names: Vec<String> = rules.iter().skip(1).filter(|r| !r.nested).map(|r| r.kind.clone()).chain(choices.iter().map(|c| c.0.clone())).collect();
    for n in &names {
        if let Some(b) = body(n).cloned() {
            decide(n, &b, &mut f, &ctx, &mut Vec::new());
        }
    }
    names.into_iter().filter(|n| f.called.get(n).copied().unwrap_or(false)).collect()
}

/// The rules and choices of a tree section that become calls (`shared`), for tests.
#[cfg(test)]
pub(crate) fn shared_names(stmts: &[Stmt], tokens: &[String]) -> Vec<String> {
    let (rules, choices, levels, _) = gather(stmts, tokens);
    shared(&rules, &choices, &levels, stmts)
}

/// The state machine of the tree section's rules in `stmts`.
/// With a whole root (`node K whole`), the root is open before the first token (the words open
/// it at byte 0, `bitlex_tier2::lower`): state 0's arms do not open it, and state 0 ends the file
/// when the root may be empty. The third value says so.
pub fn build(stmts: &[Stmt], ids: &Ids<'_>) -> Result<(Vec<StateRow>, Vec<ArmRow>, bool), String> {
    let (mut rules, mut choices, mut nested_choices, balanced) = gather(stmts, ids.tokens);
    // The rules shared by calls (`shared`): levels of their own, as if marked `nested`.
    let calls = shared(&rules, &choices, &nested_choices, stmts);
    for r in rules.iter_mut().skip(1) {
        r.nested |= calls.contains(&r.kind);
    }
    for (name, supertype, body) in std::mem::take(&mut choices) {
        if calls.contains(&name) {
            nested_choices.push(Rule { kind: name, nested: true, whole: false, body, own: false, supertype });
        } else {
            choices.push((name, supertype, body));
        }
    }
    // Each `balanced K O C`'s level: any token but `O` or `C` merged into the leaf, an inner `O`
    // merged and its own pair taken by the level again, then the closer merged; no node of its own.
    for (n, (kind, open, close)) in balanced.iter().enumerate() {
        let name = format!("{kind}{BALANCED}{n}");
        let merge = |t: &str| Gram::Merge(kind.clone(), Box::new(Gram::Sym(t.to_string())));
        let mut alts: Vec<Gram> = ids.tokens.iter().filter(|t| *t != open && *t != close).map(|t| merge(t)).collect();
        alts.push(Gram::Seq(vec![merge(open), Gram::Sym(name.clone())]));
        let body = Gram::Seq(vec![Gram::Star(Box::new(Gram::Alt(alts))), merge(close)]);
        nested_choices.push(Rule { kind: name, nested: true, whole: false, body, own: false, supertype: None });
    }
    if rules.is_empty() {
        return Err("the tree section has no rule".into());
    }
    // After the nodes: the root is the first node.
    rules.extend(nested_choices);
    if rules.iter().skip(1).any(|r| r.whole) || (rules[0].whole && rules[0].nested) {
        return Err("only the root may be whole, and it is not nested".into());
    }
    let whole = rules[0].whole;
    let mut root_empty = false;
    let remember: Vec<String> = stmts.iter().filter_map(|s| if let Stmt::Remember(n) = s { Some(n.iter().cloned()) } else { None }).flatten().collect();
    let mut b = Builder { rules, choices, ids, positions: Vec::new(), remember };
    // The levels: the root (rule 0) and each nested rule.
    let level_rules: Vec<usize> = (0..b.rules.len()).filter(|&r| r == 0 || b.rules[r].nested).collect();
    let mut levels = Vec::new();
    for (l, &r) in level_rules.iter().enumerate() {
        let body = b.rules[r].body.clone();
        let e = b.expand(l, &body, &mut vec![b.rules[r].kind.clone()])?;
        let mut edges = Vec::new();
        follow(&e, &mut edges);
        for (q, enters) in first(&e) {
            edges.push(Edge { from: None, exits: vec![], enters, to: Some(q) });
        }
        for (p, exits) in last(&e) {
            edges.push(Edge { from: Some(p), exits, enters: vec![], to: None });
        }
        // The root may be empty: a file without tokens goes to tree-sitter (its start state has
        // no end), as the reference's root is never zero-width. A nested node may not.
        if l != 0 && nullable(&e) {
            return Err(format!("node {} may be empty", b.rules[r].kind));
        }
        if l == 0 {
            root_empty = nullable(&e);
        }
        let supertype = b.rules[r].supertype.as_deref().map(ids.kind).transpose()?;
        // A `balanced` level's kind is its leaf's.
        let kind = b.rules[r].kind.split(BALANCED).next().unwrap_or_default();
        let mut refuses = Vec::new();
        for s in stmts {
            if let Stmt::Refuse { mark, levels: names } = s
                && names.contains(&b.rules[r].kind)
            {
                if l == 0 {
                    return Err(format!("refuse {mark} in {}: the root is no nested level", b.rules[r].kind));
                }
                refuses.push((ids.kind)(mark)?);
            }
        }
        // A shared choice's level (`shared`) makes no node: where its name is no kind of the
        // grammar's, an id of its own from the top, apart from every kind, names the level.
        let id = if !b.rules[r].own && calls.contains(&b.rules[r].kind) { (ids.kind)(kind).unwrap_or(u16::MAX - l as u16) } else { (ids.kind)(kind)? };
        levels.push(Level { kind: id, own: b.rules[r].own, supertype, edges, refuses });
    }
    // Each level a `refuse` names: a nested one.
    for s in stmts {
        if let Stmt::Refuse { mark, levels: names } = s
            && let Some(n) = names.iter().find(|n| !level_rules.iter().any(|&r| &b.rules[r].kind == *n))
        {
            return Err(format!("refuse {mark} in {n}: no nested level of that name"));
        }
    }
    let level_of_rule: HashMap<usize, usize> = level_rules.iter().enumerate().map(|(l, &r)| (r, l)).collect();
    let positions = b.positions;

    // The arms of a level's start or position `from`, on each token: follow a call into its
    // level's start until a token.
    fn arms_from(
        levels: &[Level],
        positions: &[(usize, Pos)],
        level_of_rule: &HashMap<usize, usize>,
        level: usize,
        from: Option<usize>,
        prefix: Vec<Act>,
        pending: Pending,
        push: Vec<usize>,
        depth: usize,
        out: &mut Vec<(u16, Vec<Act>, usize, Vec<usize>)>,
    ) -> Result<(), String> {
        if depth > 16 {
            return Err("calls nest at the first token without end".into());
        }
        for edge in levels[level].edges.iter().filter(|e| e.from == from) {
            let Some(q) = edge.to else { continue };
            let mut acts = prefix.clone();
            closes(&edge.exits, &mut acts);
            let mut p = pending.clone();
            p.enter(&edge.enters, &mut acts);
            match positions[q].1 {
                Pos::Tok(kind, leaf, remembered) => {
                    acts.push(Act::Leaf { field: p.field, supers: p.supers, kind: leaf });
                    if remembered {
                        acts.push(Act::Remember { kind });
                    }
                    out.push((kind, acts, q, push.clone()));
                }
                Pos::Merge { token, into } => {
                    if p.field != 0 || !p.supers.is_empty() {
                        return Err("a field or a supertype on a merged leaf".into());
                    }
                    if acts.iter().any(|a| matches!(a, Act::Close | Act::Open { .. })) {
                        return Err("a merge after a node opens or closes: it joins the leaf just before".into());
                    }
                    acts.push(Act::Merge { kind: into });
                    out.push((token, acts, q, push.clone()));
                }
                Pos::Call(rule) => {
                    let child = level_of_rule[&rule];
                    // A call at the first token of a level entered by a call: both are made,
                    // the outer one first, so the inner one returns first.
                    let mut calls = push.clone();
                    calls.push(q);
                    // A nested choice opens no node: the call's field and supertypes go to its
                    // first node, as an inlined choice's do.
                    let inner = if levels[child].own {
                        acts.push(Act::Open { kind: levels[child].kind, field: p.field, supers: p.supers });
                        Pending::default()
                    } else {
                        p.supers.extend(levels[child].supertype);
                        p
                    };
                    arms_from(levels, positions, level_of_rule, child, None, acts, inner, calls, depth + 1, out)?;
                }
            }
        }
        Ok(())
    }

    // Raw states: 0 is the root's start, then one per position.
    let mut raw: Vec<Raw> = Vec::new();
    let state_of = |p: usize| p + 1;
    for from in std::iter::once(None).chain((0..positions.len()).map(Some)) {
        let level = from.map_or(0, |p| positions[p].0);
        let mut arms = Vec::new();
        // The root's start opens the root node first.
        let prefix = if from.is_none() && !whole { vec![Act::Open { kind: levels[0].kind, field: 0, supers: vec![] }] } else { vec![] };
        arms_from(&levels, &positions, &level_of_rule, level, from, prefix, Pending::default(), Vec::new(), 0, &mut arms)?;
        let arms: Vec<_> = arms.into_iter().map(|(t, a, q, push)| (t, a, state_of(q), push.into_iter().map(state_of).collect::<Vec<_>>())).collect();
        let end = match from {
            // A whole root that may be empty ends at its start: the file has no token.
            None => (whole && root_empty).then_some(0),
            Some(p) => levels[level].edges.iter().find(|e| e.from == Some(p) && e.to.is_none()).map(|e| {
                let mut acts = Vec::new();
                closes(&e.exits, &mut acts);
                acts.len() as u8
            }),
        };
        for (i, a) in arms.iter().enumerate() {
            if arms[..i].iter().any(|b| b.0 == a.0) {
                let name = ids.tokens.iter().find(|t| (ids.kind)(t).ok() == Some(a.0)).cloned().unwrap_or_else(|| format!("kind {}", a.0));
                let rule = &b.rules[level_rules[level]].kind;
                return Err(format!("the grammar is not LL(1): two ways on {name} at one place in {rule}'s level"));
            }
        }
        // A nested level that may end here and has arms takes a token an arm takes, and ends
        // (returns to switch on it again) before any other. As the reference, which tries what
        // may come before what follows; where that fails later the reference may go back and
        // the words refuse: the file goes to tree-sitter, never to a different tree.
        raw.push(Raw { level, end, arms });
    }

    // Merge states that act alike (Moore's refinement), then number them from the start, in
    // the order they are reached. States of different levels act alike when their arms do and
    // they end alike: the root's apart, a level with a node of its own apart from one without
    // (its end closes it), and the marks refused the same.
    let mut class: Vec<usize> = Vec::new();
    {
        let mut keys: Vec<(bool, bool, Option<u8>, &[u16])> = Vec::new();
        for r in &raw {
            let k = (r.level == 0, levels[r.level].own, r.end, levels[r.level].refuses.as_slice());
            let c = keys.iter().position(|x| *x == k).unwrap_or_else(|| {
                keys.push(k);
                keys.len() - 1
            });
            class.push(c);
        }
        loop {
            let mut sigs: Vec<(usize, Vec<(u16, &Vec<Act>, usize, Vec<usize>)>)> = Vec::new();
            let mut next = Vec::with_capacity(raw.len());
            for (i, r) in raw.iter().enumerate() {
                let mut arms: Vec<_> = r.arms.iter().map(|(t, a, q, p)| (*t, a, class[*q], p.iter().map(|&p| class[p]).collect::<Vec<_>>())).collect();
                arms.sort_by_key(|a| a.0);
                let sig = (class[i], arms);
                let c = sigs.iter().position(|s| *s == sig).unwrap_or_else(|| {
                    sigs.push(sig.clone());
                    sigs.len() - 1
                });
                next.push(c);
            }
            let stable = next.iter().max() == class.iter().max();
            class = next;
            if stable {
                break;
            }
        }
    }
    // Number the classes in the order they are reached from the start.
    let mut number: HashMap<usize, u16> = HashMap::new();
    let mut order: Vec<usize> = Vec::new();
    let mut queue = std::collections::VecDeque::from([0usize]);
    while let Some(i) = queue.pop_front() {
        if number.contains_key(&class[i]) {
            continue;
        }
        number.insert(class[i], order.len() as u16);
        order.push(i);
        for (_, _, q, p) in &raw[i].arms {
            queue.push_back(*q);
            queue.extend(p.iter().copied());
        }
    }
    // Levels that share a state are one level from here on: a state that ends returns to the
    // states any of them returns to (`bitlex_masks`'s modes follow returns by level).
    let mut joined: Vec<usize> = (0..levels.len()).collect();
    fn find(joined: &mut [usize], l: usize) -> usize {
        let mut r = l;
        while joined[r] != r {
            r = joined[r];
        }
        joined[l] = r;
        r
    }
    let mut first_of: HashMap<usize, usize> = HashMap::new();
    for (i, r) in raw.iter().enumerate() {
        let other = *first_of.entry(class[i]).or_insert(r.level);
        let (a, b) = (find(&mut joined, other), find(&mut joined, r.level));
        if a != b {
            joined[a.max(b)] = a.min(b);
        }
    }
    let mut states = Vec::new();
    let mut arms = Vec::new();
    for (s, &i) in order.iter().enumerate() {
        let r = &raw[i];
        let level = find(&mut joined, r.level);
        states.push(StateRow { level: levels[level].kind, own: levels[r.level].own, end: r.end, refuses: levels[r.level].refuses.clone() });
        for (token, acts, q, p) in &r.arms {
            arms.push(ArmRow { from: s as u16, token: *token, acts: acts.clone(), to: number[&class[*q]], push: p.iter().map(|&p| number[&class[p]]).collect() });
        }
    }
    refusing(&states, &arms)?;
    Ok((states, arms, whole))
}

/// A level that refuses a mark (`refuse M in L`) refuses it between its first token and its
/// last, as the reference does: a state of it that may end takes no token (else its end would
/// test the marks after its last); it takes no arm on the mark; and each level it calls refuses
/// the mark too.
fn refusing(states: &[StateRow], arms: &[ArmRow]) -> Result<(), String> {
    for a in arms {
        let s = &states[usize::from(a.from)];
        if s.refuses.is_empty() {
            continue;
        }
        if s.end.is_some() {
            return Err(format!("state {}: a level that refuses a mark may end only after its last token", a.from));
        }
        if s.refuses.contains(&a.token) {
            return Err(format!("state {}: a level takes a mark it refuses", a.from));
        }
        for &q in a.push.iter().chain([&a.to]) {
            if !s.refuses.iter().all(|m| states[usize::from(q)].refuses.contains(m)) {
                return Err(format!("state {}: a level that refuses a mark calls one that does not", a.from));
            }
        }
    }
    Ok(())
}

/// The extras (`extra K`): tokens the grammar does not name, which may come between any two
/// others. Each goes where tree-sitter's parser puts it: in the lowest node holding the tokens
/// on either side, after the nodes that end before it. A state's arms close those nodes when
/// the next token comes; so an extra arm closes them first, writes the extra's leaf, and goes on
/// in a copy of the state without those closes. That needs every way on from the state to close
/// the same number. Where they differ, the extra's place depends on the token after it: the
/// arm writes the leaf deferred (`Act::Extra`), the closes the next token makes leave it out,
/// and the state stays as it is (docs/parser-generators/24-rr-and-bitlex-plan.md §2.3). A
/// nested level that may end where it could go on has no extra arm.
///
/// A state takes extras only where `apart(token, extra)` for each of its tokens: tier 2 tells a
/// token's row by its first byte, so an extra there must not begin as one of the state's tokens
/// (inside a string, say, where tier 1 makes no comment).
///
/// In state 0 nothing is open: the extra arm opens the root first (so the root starts at the
/// extra) and goes on in a copy of state 0 whose arms do not open it.
pub fn with_extras(
    mut states: Vec<StateRow>,
    mut arms: Vec<ArmRow>,
    extras: &[u16],
    apart: &dyn Fn(u16, u16) -> bool,
    whole: bool,
    marks: &[u16],
) -> Result<(Vec<StateRow>, Vec<ArmRow>), String> {
    if extras.is_empty() {
        return Ok((states, arms));
    }
    if let Some(a) = arms.iter().find(|a| extras.contains(&a.token)) {
        return Err(format!("extra kind {} is a token of the grammar too", a.token));
    }
    if arms.iter().any(|a| a.to == 0 || a.push.contains(&0)) {
        return Err("extras with the root's start state entered again: not compiled yet".into());
    }
    let root = states[0].level;
    let closes = |acts: &[Act]| acts.iter().take_while(|a| **a == Act::Close).count();
    let mut new_arms: Vec<ArmRow> = Vec::new();
    // With a whole root, state 0 is like any other: the root is open already.
    let mut s = if whole { 0u16 } else { 1u16 };
    if !whole {
    // State 0: its arms all open the root first.
    let first: Vec<&ArmRow> = arms.iter().filter(|a| a.from == 0).collect();
    let open_root = match first.first().and_then(|a| a.acts.first()) {
        Some(o @ Act::Open { kind, .. }) if *kind == root && first.iter().all(|a| a.acts.first() == Some(o)) => o.clone(),
        _ => return Err("extras where state 0's arms do not all open the root: not compiled yet".into()),
    };
    let zero_takes = first.iter().all(|a| extras.iter().all(|&k| apart(a.token, k)));
    let opened = states.len() as u16;
    states.push(states[0].clone());
    for a in first.iter().map(|a| (*a).clone()).collect::<Vec<_>>() {
        arms.push(ArmRow { from: opened, acts: a.acts[1..].to_vec(), ..a });
    }
    if zero_takes {
        for &k in extras {
            new_arms.push(ArmRow { from: 0, token: k, acts: vec![open_root.clone(), Act::ExtraNow], to: opened, push: Vec::new() });
        }
    }
    }
    // Every other state, and the copies made on the way.
    let mut copies: HashMap<(u16, usize), u16> = HashMap::new();
    while usize::from(s) < states.len() {
        let mine: Vec<ArmRow> = arms.iter().filter(|a| a.from == s).cloned().collect();
        let nested = states[usize::from(s)].level != root;
        let end = states[usize::from(s)].end;
        // A nested state with no arms ends its level at once, taking no token.
        if mine.is_empty() && nested {
            s += 1;
            continue;
        }
        if !mine.iter().all(|a| extras.iter().all(|&k| apart(a.token, k))) {
            s += 1;
            continue;
        }
        // A nested state that may end: the extra's leaf is deferred, so the closes of the token
        // after it (an arm's, or the level's end) leave it out, and it goes where that token
        // puts it.
        if nested && end.is_some() {
            for &k in extras {
                new_arms.push(ArmRow { from: s, token: k, acts: vec![Act::Extra], to: s, push: Vec::new() });
            }
            s += 1;
            continue;
        }
        let mut ways: Vec<usize> = mine.iter().map(|a| closes(&a.acts)).collect();
        ways.extend(end.map(usize::from));
        // A mark's arm takes no token: the nodes closed before the next token are its closes and
        // then the next state's (`None`: past a call or a return, not followed).
        let through = |s: u16| -> Option<Vec<usize>> {
            let mut out = Vec::new();
            let mut work = vec![(s, 0usize, 0usize)];
            while let Some((at, before, depth)) = work.pop() {
                if depth > marks.len() {
                    return None;
                }
                let state = &states[usize::from(at)];
                let here: Vec<&ArmRow> = arms.iter().filter(|a| a.from == at).collect();
                if state.level != root && (here.is_empty() || state.end.is_some()) {
                    return None;
                }
                out.extend(state.end.map(|e| before + usize::from(e)));
                for a in here {
                    let c = before + closes(&a.acts);
                    if marks.contains(&a.token) {
                        if !a.push.is_empty() {
                            return None;
                        }
                        work.push((a.to, c, depth + 1));
                    } else {
                        out.push(c);
                    }
                }
            }
            Some(out)
        };
        let marked = mine.iter().any(|a| marks.contains(&a.token));
        // A merge joins the leaf before the extras: they must be pending, to be its text.
        let merges = mine.iter().any(|a| a.acts.iter().any(|x| matches!(x, Act::Merge { .. })));
        let early = !merges && if marked { through(s).is_some_and(|w| w.iter().all(|&x| x == 0)) } else { ways.iter().all(|&w| w == ways[0]) };
        let m = if marked { 0 } else { ways[0] };
        if early {
            let to = if m == 0 {
                s
            } else {
                *copies.entry((s, m)).or_insert_with(|| {
                    let c = states.len() as u16;
                    let st = &states[usize::from(s)];
                    states.push(StateRow { level: st.level, own: st.own, end: end.map(|e| e - m as u8), refuses: st.refuses.clone() });
                    for a in &mine {
                        arms.push(ArmRow { from: c, acts: a.acts[m..].to_vec(), ..a.clone() });
                    }
                    c
                })
            };
            let mut acts = vec![Act::Close; m];
            acts.push(Act::ExtraNow);
            for &k in extras {
                new_arms.push(ArmRow { from: s, token: k, acts: acts.clone(), to, push: Vec::new() });
            }
        } else {
            for &k in extras {
                new_arms.push(ArmRow { from: s, token: k, acts: vec![Act::Extra], to: s, push: Vec::new() });
            }
        }
        s += 1;
    }
    arms.extend(new_arms);
    arms.sort_by_key(|a| a.from);
    Ok((states, arms))
}

/// The arms on marks (tokens of no width) write no leaf: a mark makes no node. A field or a
/// supertype on one would name a node that is not there.
pub fn without_mark_leaves(arms: &mut [ArmRow], marks: &[u16]) -> Result<(), String> {
    for a in arms.iter_mut().filter(|a| marks.contains(&a.token)) {
        let mut acts = Vec::with_capacity(a.acts.len());
        for act in std::mem::take(&mut a.acts) {
            match act {
                Act::Leaf { field: 0, supers, kind: 0 } if supers.is_empty() => {}
                Act::Leaf { .. } => return Err(format!("mark kind {} with a field or a supertype: a mark makes no node", a.token)),
                other => acts.push(other),
            }
        }
        a.acts = acts;
    }
    Ok(())
}

/// A `check`'s grammar over a token's bytes to a byte state machine (doc 18 §4): its names are
/// byte classes, its literals bytes. Glushkov's positions, each a set of bytes; then a state per
/// set of positions reached (subset construction), the arms from one state on disjoint bytes.
/// State 0 is the start. The compiler makes a mask per state, over every byte of the file at
/// once (`bitlex_compile`); a byte with no arm, or an end outside an accepting state, refuses.
pub fn byte_check(g: &Gram, classes: &HashMap<String, Vec<(u32, u32)>>) -> Result<(Vec<bool>, Vec<(u16, [u64; 4], u16)>), String> {
    fn set(ranges: &[(u32, u32)]) -> [u64; 4] {
        let mut s = [0u64; 4];
        for &(lo, hi) in ranges {
            for b in lo..=hi.min(255) {
                s[(b / 64) as usize] |= 1 << (b % 64);
            }
        }
        s
    }
    fn expand(g: &Gram, classes: &HashMap<String, Vec<(u32, u32)>>, sets: &mut Vec<[u64; 4]>) -> Result<E, String> {
        let pos = |s: [u64; 4], sets: &mut Vec<[u64; 4]>| {
            sets.push(s);
            E::Tok(sets.len() - 1)
        };
        Ok(match g {
            Gram::Lit(text) if !text.is_empty() => E::Seq(text.bytes().map(|b| pos(set(&[(u32::from(b), u32::from(b))]), sets)).collect()),
            Gram::Lit(_) => return Err("check: an empty literal".into()),
            Gram::Sym(name) => pos(set(classes.get(name).ok_or_else(|| format!("check: `{name}` is no class"))?), sets),
            Gram::Seq(items) => E::Seq(items.iter().map(|i| expand(i, classes, sets)).collect::<Result<_, _>>()?),
            Gram::Alt(alts) => E::Alt(alts.iter().map(|a| expand(a, classes, sets)).collect::<Result<_, _>>()?),
            Gram::Opt(a) => E::Opt(Box::new(expand(a, classes, sets)?)),
            Gram::Star(a) => E::Star(Box::new(expand(a, classes, sets)?)),
            Gram::Plus(a) => E::Seq(vec![expand(a, classes, sets)?, E::Star(Box::new(expand(a, classes, sets)?))]),
            Gram::Field(..) | Gram::Retag(..) | Gram::Merge(..) | Gram::Wrap { .. } | Gram::Relabel { .. } | Gram::Fold { .. } | Gram::Reread { .. } | Gram::Balanced { .. } | Gram::Any(_) | Gram::As(..) => return Err("check: a tree's notation in a byte grammar".into()),
        })
    }
    let mut sets = Vec::new();
    let e = expand(g, classes, &mut sets)?;
    let mut edges = Vec::new();
    follow(&e, &mut edges);
    let starts: Vec<usize> = first(&e).into_iter().map(|p| p.0).collect();
    let ends: Vec<usize> = last(&e).into_iter().map(|p| p.0).collect();
    // A state: a sorted set of positions; `None` in it is the start.
    let mut states: Vec<Vec<Option<usize>>> = vec![vec![None]];
    let mut accept = vec![nullable(&e)];
    let mut arms = Vec::new();
    let mut at = 0;
    while at < states.len() {
        let next: Vec<usize> = {
            let mut n: Vec<usize> = Vec::new();
            for p in &states[at] {
                match p {
                    None => n.extend(&starts),
                    Some(p) => n.extend(edges.iter().filter(|e| e.from == Some(*p)).filter_map(|e| e.to)),
                }
            }
            n.sort_unstable();
            n.dedup();
            n
        };
        // Split the bytes the next positions take into blocks that take the same positions.
        for b in 0..=255u32 {
            let bit = |s: &[u64; 4]| s[(b / 64) as usize] >> (b % 64) & 1 == 1;
            let to: Vec<Option<usize>> = next.iter().copied().filter(|&q| bit(&sets[q])).map(Some).collect();
            if to.is_empty() {
                continue;
            }
            let t = match states.iter().position(|s| *s == to) {
                Some(t) => t,
                None => {
                    accept.push(to.iter().any(|q| q.is_some_and(|q| ends.contains(&q))));
                    states.push(to);
                    states.len() - 1
                }
            };
            let (from, to) = (at as u16, t as u16);
            match arms.iter_mut().find(|a: &&mut (u16, [u64; 4], u16)| a.0 == from && a.2 == to) {
                Some(a) => a.1[(b / 64) as usize] |= 1 << (b % 64),
                None => {
                    let mut s = [0u64; 4];
                    s[(b / 64) as usize] |= 1 << (b % 64);
                    arms.push((from, s, to));
                }
            }
        }
        at += 1;
        if states.len() > 256 {
            return Err("check: more than 256 states".into());
        }
    }
    Ok(merge(&accept, &arms))
}

/// A byte machine with the states that act alike merged (Moore's refinement), numbered from the
/// start in the order they are reached.
fn merge(accept: &[bool], arms: &[(u16, [u64; 4], u16)]) -> (Vec<bool>, Vec<(u16, [u64; 4], u16)>) {
    let n = accept.len();
    let target = |s: usize, b: u32| arms.iter().find(|a| usize::from(a.0) == s && a.1[(b / 64) as usize] >> (b % 64) & 1 == 1).map(|a| usize::from(a.2));
    let mut class: Vec<usize> = accept.iter().map(|&a| usize::from(a)).collect();
    loop {
        let mut sigs: Vec<(usize, Vec<Option<usize>>)> = Vec::new();
        let next: Vec<usize> = (0..n)
            .map(|s| {
                let sig = (class[s], (0..256).map(|b| target(s, b).map(|t| class[t])).collect());
                sigs.iter().position(|x| *x == sig).unwrap_or_else(|| {
                    sigs.push(sig);
                    sigs.len() - 1
                })
            })
            .collect();
        let stable = sigs.len() == class.iter().collect::<std::collections::HashSet<_>>().len();
        class = next;
        if stable {
            break;
        }
    }
    // Number the classes from the start, in the order the arms reach them.
    let mut number: HashMap<usize, u16> = HashMap::new();
    let mut order = vec![0usize];
    number.insert(class[0], 0);
    let mut at = 0;
    while at < order.len() {
        let here = order[at];
        for a in arms.iter().filter(|a| usize::from(a.0) == here) {
            let t = usize::from(a.2);
            if !number.contains_key(&class[t]) {
                number.insert(class[t], order.len() as u16);
                order.push(t);
            }
        }
        at += 1;
    }
    let mut out: Vec<(u16, [u64; 4], u16)> = Vec::new();
    for (k, &s) in order.iter().enumerate() {
        for a in arms.iter().filter(|a| usize::from(a.0) == s) {
            let to = number[&class[usize::from(a.2)]];
            match out.iter_mut().find(|o| usize::from(o.0) == k && o.2 == to) {
                Some(o) => (0..4).for_each(|w| o.1[w] |= a.1[w]),
                None => out.push((k as u16, a.1, to)),
            }
        }
    }
    (order.iter().map(|&s| accept[s]).collect(), out)
}

