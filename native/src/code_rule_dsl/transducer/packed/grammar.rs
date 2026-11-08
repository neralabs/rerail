//! The tree section's grammar to tier 2's state machine (docs/parser-generators/18-tree-as-grammar.md
//! §2): a pass of the Bitlex compiler. Its output is the block program's `states` and `arms`.
//!
//! 1. **Levels.** The root rule and each `nested` rule is a level. Every other rule and every
//!    choice is inlined into the levels that use it; a use of a nested rule is a call.
//! 2. **Positions.** Glushkov's construction over each level's expression: a position per token
//!    and per call; each edge between two positions knows the wrappers (nodes, fields,
//!    supertypes) it leaves and enters.
//! 3. **States.** A state is "just took position p". Each token from it is one arm: the edge's
//!    closes, its opens (each node with the field and supertypes on the way to it), the token's
//!    leaf, the next state. An arm into a call opens the called node, pushes the call's position
//!    as the return state, and continues with the called level's first edge.
//! 4. **Checks.** Two arms of one state on one token: the grammar is not LL(1), refused.
//! 5. **Merging.** States that act alike on every token are one (Moore's refinement).

use super::block::{Act, ArmRow, StateRow};
use bitlex::syntax::{Gram, Stmt};
use std::collections::HashMap;

/// A wrapper on the way to a position: a node, a field of the node inside it, a supertype of
/// the node inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wrap {
    Node(u16),
    Field(u16),
    Super(u16),
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

/// A position: the token kind it takes, or the level it calls.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pos {
    Tok(u16),
    Call(usize),
}

/// A rule of the tree section.
struct Rule {
    kind: String,
    nested: bool,
    body: Gram,
}

/// The ids of the grammar's names: kinds (token kinds, node kinds, supertypes) and fields.
pub(super) struct Ids<'a> {
    pub(super) kind: &'a dyn Fn(&str) -> Result<u16, String>,
    pub(super) field: &'a dyn Fn(&str) -> Result<u16, String>,
    /// Whether a name is a token kind the lexer makes (a token's, or a keyword's).
    pub(super) is_token: &'a dyn Fn(&str) -> bool,
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
    edges: Vec<Edge>,
}

struct Builder<'a> {
    rules: Vec<Rule>,
    choices: Vec<(String, Option<String>, Gram)>,
    ids: &'a Ids<'a>,
    positions: Vec<(usize, Pos)>,
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
                    let e = self.expand(level, &body, inlining)?;
                    inlining.pop();
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
        })
    }

    fn tok(&mut self, level: usize, name: &str) -> Result<E, String> {
        if !(self.ids.is_token)(name) {
            return Err(format!("`{name}` is no rule, choice or token kind"));
        }
        self.positions.push((level, Pos::Tok((self.ids.kind)(name)?)));
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
        if matches!(w, Wrap::Node(_)) {
            acts.push(Act::Close);
        }
    }
}

/// A state before merging: its level, the closes when its level ends after it (a pop, or the end
/// of the file at the root), and its arms: (token, acts, to, push).
struct Raw {
    level: usize,
    end: Option<u8>,
    arms: Vec<(u16, Vec<Act>, usize, Option<usize>)>,
}

/// The state machine of the tree section's rules in `stmts`.
pub(super) fn build(stmts: &[Stmt], ids: &Ids<'_>) -> Result<(Vec<StateRow>, Vec<ArmRow>), String> {
    let mut rules = Vec::new();
    let mut choices = Vec::new();
    for s in stmts {
        match s {
            Stmt::Node { kind, nested, body } => rules.push(Rule { kind: kind.clone(), nested: *nested, body: body.clone() }),
            Stmt::Choice { name, supertype, body } => choices.push((name.clone(), supertype.clone(), body.clone())),
            _ => {}
        }
    }
    if rules.is_empty() {
        return Err("the tree section has no rule".into());
    }
    let mut b = Builder { rules, choices, ids, positions: Vec::new() };
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
        if nullable(&e) {
            return Err(format!("node {} may be empty", b.rules[r].kind));
        }
        levels.push(Level { kind: (ids.kind)(&b.rules[r].kind)?, edges });
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
        push: Option<usize>,
        depth: usize,
        out: &mut Vec<(u16, Vec<Act>, usize, Option<usize>)>,
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
                Pos::Tok(kind) => {
                    acts.push(Act::Leaf { field: p.field, supers: p.supers });
                    out.push((kind, acts, q, push));
                }
                Pos::Call(rule) => {
                    let child = level_of_rule[&rule];
                    if push.is_some() {
                        return Err("a call at a level's first token inside another: not handled".into());
                    }
                    acts.push(Act::Open { kind: levels[child].kind, field: p.field, supers: p.supers });
                    arms_from(levels, positions, level_of_rule, child, None, acts, Pending::default(), Some(q), depth + 1, out)?;
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
        let prefix = if from.is_none() { vec![Act::Open { kind: levels[0].kind, field: 0, supers: vec![] }] } else { vec![] };
        arms_from(&levels, &positions, &level_of_rule, level, from, prefix, Pending::default(), None, 0, &mut arms)?;
        let arms: Vec<_> = arms.into_iter().map(|(t, a, q, push)| (t, a, state_of(q), push.map(state_of))).collect();
        let end = match from {
            None => None,
            Some(p) => levels[level].edges.iter().find(|e| e.from == Some(p) && e.to.is_none()).map(|e| {
                let mut acts = Vec::new();
                closes(&e.exits, &mut acts);
                acts.len() as u8
            }),
        };
        for (i, a) in arms.iter().enumerate() {
            if arms[..i].iter().any(|b| b.0 == a.0) {
                return Err(format!("the grammar is not LL(1): two ways on token kind {} at one place", a.0));
            }
        }
        if level != 0 && end.is_some() && !arms.is_empty() {
            return Err("a nested node that may end before a token it could also take: not handled".into());
        }
        raw.push(Raw { level, end, arms });
    }

    // Merge states that act alike (Moore's refinement), then number them from the start, in
    // the order they are reached.
    let mut class: Vec<usize> = Vec::new();
    {
        let mut keys: Vec<(usize, Option<u8>)> = Vec::new();
        for r in &raw {
            let k = (r.level, r.end);
            let c = keys.iter().position(|x| *x == k).unwrap_or_else(|| {
                keys.push(k);
                keys.len() - 1
            });
            class.push(c);
        }
        loop {
            let mut sigs: Vec<(usize, Vec<(u16, &Vec<Act>, usize, Option<usize>)>)> = Vec::new();
            let mut next = Vec::with_capacity(raw.len());
            for (i, r) in raw.iter().enumerate() {
                let mut arms: Vec<_> = r.arms.iter().map(|(t, a, q, p)| (*t, a, class[*q], p.map(|p| class[p]))).collect();
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
            if let Some(p) = p {
                queue.push_back(*p);
            }
        }
    }
    let mut states = Vec::new();
    let mut arms = Vec::new();
    for (s, &i) in order.iter().enumerate() {
        let r = &raw[i];
        states.push(StateRow { level: levels[r.level].kind, end: r.end });
        for (token, acts, q, p) in &r.arms {
            arms.push(ArmRow { from: s as u16, token: *token, acts: acts.clone(), to: number[&class[*q]], push: p.map(|p| number[&class[p]]) });
        }
    }
    Ok((states, arms))
}
