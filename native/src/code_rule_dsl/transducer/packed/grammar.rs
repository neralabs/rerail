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

use bitlex::syntax::{Gram, Stmt};
use std::collections::HashMap;

/// A state of tier 2's state machine (docs/parser-generators/18-tree-as-grammar.md §2): the
/// tree section's grammar, one level per nested node. State 0 is the root's start.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct StateRow {
    /// The node kind of its level: the root's, or a nested node's.
    pub(super) level: u16,
    /// Its level may end here, after this many closes inside it: a nested level then closes its
    /// node and pops (at once, when the state has no arms); the root accepts at the end of the
    /// file. `None`: it may not end here.
    pub(super) end: Option<u8>,
}

/// What a transition does, in order: close nodes (innermost first), open nodes (outermost
/// first), then the token's own leaf.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum Act {
    /// Close the innermost open node, at the end of the token before.
    Close,
    /// Open a node of `kind` at the token's start, with a field (0: none) and supertypes
    /// (outermost first).
    Open { kind: u16, field: u16, supers: Vec<u16> },
    /// The token's leaf node, of the token's kind.
    Leaf { field: u16, supers: Vec<u16> },
}
/// A transition: from a state, on a token of a kind, its acts, to a state; `push`: a call, which
/// enters a nested level and keeps the state to return to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct ArmRow {
    pub(super) from: u16,
    pub(super) token: u16,
    pub(super) acts: Vec<Act>,
    pub(super) to: u16,
    pub(super) push: Option<u16>,
}

/// The state machine as text, a line per state and per arm, for tests and dumps:
/// `state S level K [end C]`; `arm S token K to T [push P] do ACT…`, an act `close`,
/// `open K [f F] [s K]…` or `leaf [f F] [s K]…`.
pub(super) fn text(states: &[StateRow], arms: &[ArmRow]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (s, state) in states.iter().enumerate() {
        let end = state.end.map_or(String::new(), |c| format!(" end {c}"));
        let _ = writeln!(out, "state {s} level {}{end}", state.level);
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
                Act::Leaf { field, supers } => {
                    let _ = write!(acts, " leaf{}", marks(*field, supers));
                }
            }
        }
        let push = arm.push.map_or(String::new(), |p| format!(" push {p}"));
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

/// A `check`'s grammar over a token's bytes to a byte state machine (doc 18 §4): its names are
/// byte classes, its literals bytes. Glushkov's positions, each a set of bytes; then a state per
/// set of positions reached (subset construction), the arms from one state on disjoint bytes.
/// State 0 is the start. The token's arm runs it over the token's bytes, as simdjson's stage 2
/// parses a number; a byte with no arm, or an end outside an accepting state, refuses.
pub(super) fn byte_check(g: &Gram, classes: &HashMap<String, Vec<(u32, u32)>>) -> Result<(Vec<bool>, Vec<(u16, [u64; 4], u16)>), String> {
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
            Gram::Field(..) => return Err("check: a field in a byte grammar".into()),
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

/// A check's byte machine as straight-line operations over the token's bytes (doc 18 §4,
/// decision 0086): no loop over the bytes, as simdjson's `parse_number` has none. Bit `i` is the
/// token's byte `i`; `within` is the token's bytes. A token of any length runs it 64 bytes at a
/// time, `prev` and `fill` carrying from word to word, as tier 1 runs over a file. The machine is on one path, so per state:
///
/// - `e`: where it enters the state (the token's first byte for the start);
/// - a self-loop on bytes `L` keeps it there over a run: `q = fill(e, ~prev(L, 1))`, and it
///   leaves at `x = q & ~L`, the first byte not in `L` (or the token's end);
/// - without one, `x = e`;
/// - each other arm on bytes `B` enters its state at `prev(x & B, 1)`.
///
/// The output fails where some `x` has a byte no arm takes, or is the end outside an accepting
/// state; it is one byte late (`delay 1`), so a failure at the end is the last byte's. A cycle through two or more states would need a loop over the bytes: refused.
pub(super) fn check_program(accept: &[bool], arms: &[(u16, [u64; 4], u16)]) -> Result<super::block::BlockProgram, String> {
    use super::block::{BlockOp, BlockProgram, ClassTable};
    let n = accept.len();
    let mut p = BlockProgram::default();
    let mut seen: HashMap<BlockOp, u16> = HashMap::new();
    let mut op = |p: &mut BlockProgram, o: BlockOp| -> u16 {
        *seen.entry(o).or_insert_with(|| {
            p.ops.push(o);
            (p.ops.len() - 1) as u16
        })
    };
    // A byte set: one `eq`, or a class of its own table (products of nibble sets, eight a table).
    let mut sets: HashMap<[u64; 4], u16> = HashMap::new();
    let mut set_op = |p: &mut BlockProgram, op: &mut dyn FnMut(&mut BlockProgram, BlockOp) -> u16, s: [u64; 4]| -> Result<u16, String> {
        if let Some(&v) = sets.get(&s) {
            return Ok(v);
        }
        let bytes: Vec<u8> = (0..=255u8).filter(|&b| s[usize::from(b / 64)] >> (b % 64) & 1 == 1).collect();
        let v = if bytes.len() == 1 {
            op(p, BlockOp::Eq(bytes[0]))
        } else {
            let mut products: Vec<(u16, u16)> = Vec::new();
            for h in 0..16u8 {
                let lo = bytes.iter().filter(|&&b| b >> 4 == h).fold(0u16, |a, &b| a | 1 << (b & 15));
                if lo == 0 {
                    continue;
                }
                match products.iter_mut().find(|x| x.1 == lo) {
                    Some(x) => x.0 |= 1 << h,
                    None => products.push((1 << h, lo)),
                }
            }
            if products.len() > 8 {
                return Err("a byte set of more than eight nibble products".into());
            }
            let mut t = ClassTable::default();
            for (k, &(hi, lo)) in products.iter().enumerate() {
                for i in 0..16 {
                    if hi >> i & 1 == 1 {
                        t.hi[i] |= 1 << k;
                    }
                    if lo >> i & 1 == 1 {
                        t.lo[i] |= 1 << k;
                    }
                }
            }
            p.tables.push(t);
            let bits = ((1u16 << products.len()) - 1) as u8;
            op(p, BlockOp::Class { table: (p.tables.len() - 1) as u16, bits })
        };
        sets.insert(s, v);
        Ok(v)
    };
    // The states in an order where every arm but a self-loop goes forward.
    let mut order = Vec::new();
    let mut indegree = vec![0usize; n];
    for a in arms.iter().filter(|a| a.0 != a.2) {
        indegree[usize::from(a.2)] += 1;
    }
    let mut ready: Vec<usize> = (0..n).filter(|&s| indegree[s] == 0).collect();
    while let Some(s) = ready.pop() {
        order.push(s);
        for a in arms.iter().filter(|a| usize::from(a.0) == s && a.0 != a.2) {
            let t = usize::from(a.2);
            indegree[t] -= 1;
            if indegree[t] == 0 {
                ready.push(t);
            }
        }
    }
    if order.len() != n || order.first() != Some(&0) {
        return Err("its state machine has a cycle through two or more states, which needs a loop over the bytes: refused".into());
    }
    let within = op(&mut p, BlockOp::Within(0));
    let shifted_within = op(&mut p, BlockOp::Prev(within, 1));
    let end = op(&mut p, BlockOp::AndNot(shifted_within, within));
    let mut enter: Vec<Option<u16>> = vec![None; n];
    // The token's first byte: not a `const`, which would start it again in every 64 bytes.
    enter[0] = Some(op(&mut p, BlockOp::AndNot(within, shifted_within)));
    // Failures inside the token, and at its end; both brought to one byte late, so the end's is
    // the last byte's (the output's delay is 1).
    let mut fail_in: Option<u16> = None;
    let mut fail_end: Option<u16> = None;
    let join = |p: &mut BlockProgram, op: &mut dyn FnMut(&mut BlockProgram, BlockOp) -> u16, acc: &mut Option<u16>, v: u16| {
        *acc = Some(match *acc {
            Some(a) => op(p, BlockOp::Or(a, v)),
            None => v,
        });
    };
    for &s in &order {
        let Some(e) = enter[s] else { continue };
        let own: Vec<&(u16, [u64; 4], u16)> = arms.iter().filter(|a| usize::from(a.0) == s).collect();
        let x = match own.iter().find(|a| usize::from(a.2) == s) {
            Some(lp) => {
                let l = set_op(&mut p, &mut op, lp.1)?;
                let l = op(&mut p, BlockOp::And(l, within));
                let stay = op(&mut p, BlockOp::Prev(l, 1));
                let leave = op(&mut p, BlockOp::Not(stay));
                let q = op(&mut p, BlockOp::Fill(e, leave));
                op(&mut p, BlockOp::AndNot(q, l))
            }
            None => e,
        };
        let mut taken: Option<u16> = None;
        for a in own.iter().filter(|a| usize::from(a.2) != s) {
            let b = set_op(&mut p, &mut op, a.1)?;
            let at = op(&mut p, BlockOp::And(x, b));
            let at = op(&mut p, BlockOp::And(at, within));
            let next = op(&mut p, BlockOp::Prev(at, 1));
            let t = usize::from(a.2);
            enter[t] = Some(match enter[t] {
                Some(v) => op(&mut p, BlockOp::Or(v, next)),
                None => next,
            });
            taken = Some(match taken {
                Some(v) => op(&mut p, BlockOp::Or(v, b)),
                None => b,
            });
        }
        // A byte no arm takes, inside the token; the end, outside an accepting state.
        let inside = op(&mut p, BlockOp::And(x, within));
        let bad = match taken {
            Some(t) => op(&mut p, BlockOp::AndNot(inside, t)),
            None => inside,
        };
        join(&mut p, &mut op, &mut fail_in, bad);
        if !accept[s] {
            let at_end = op(&mut p, BlockOp::And(x, end));
            join(&mut p, &mut op, &mut fail_end, at_end);
        }
    }
    let late = op(&mut p, BlockOp::Prev(fail_in.expect("the start state"), 1));
    let fail = match fail_end {
        Some(e) => op(&mut p, BlockOp::Or(late, e)),
        None => late,
    };
    p.outputs = vec![fail];
    p.delay = 1;
    p.output_delays = vec![1];
    p.verify().map_err(|_| "its operations do not verify".to_owned())?;
    Ok(p)
}
