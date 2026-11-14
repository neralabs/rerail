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
    /// A token's leaf (or an extra's): not a node of the grammar's, which may be empty.
    pub leaf: bool,
}

/// The token kinds a grammar may start with: names, and with `any`, every kind but some.
#[derive(Clone, Default, PartialEq)]
struct Starts {
    kinds: std::collections::BTreeSet<String>,
    /// `any`: every kind but these.
    any: Option<std::collections::BTreeSet<String>>,
}

impl Starts {
    fn of(kind: &str) -> Self {
        Starts { kinds: [kind.to_owned()].into(), any: None }
    }

    /// Token `t` (its kind, or the soft keyword it may be) starts it.
    fn has(&self, t: &Token) -> bool {
        let k = |k: &str| self.kinds.contains(k) || self.any.as_ref().is_some_and(|ex| !ex.contains(k));
        k(&t.kind) || t.soft.as_deref().is_some_and(k)
    }

    fn add(&mut self, o: &Starts) {
        self.kinds.extend(o.kinds.iter().cloned());
        self.any = match (self.any.take(), &o.any) {
            (None, x) => x.clone(),
            (Some(a), None) => Some(a),
            (Some(a), Some(b)) => Some(a.intersection(b).cloned().collect()),
        };
    }
}

struct Parser<'a> {
    decl: &'a TreeDecl,
    tokens: &'a [Token],
    out: Vec<Node>,
    /// The kinds the parse looks for ahead, a bit each: the marks (bit `k`: mark `k`), then the
    /// soft keywords. Which marks are hidden.
    looks: Vec<String>,
    hidden: u64,
    /// Each rule's and choice's first looks and whether it may be empty (`first_of`).
    named: HashMap<&'a str, (u64, bool)>,
    /// Each rule's and choice's first token kinds and whether it may be empty (`first_of`).
    named_starts: HashMap<&'a str, (Starts, bool)>,
    /// The rules' nodes open, innermost last (`retag` changes the last's kind).
    open: Vec<usize>,
    /// Each extra's first byte.
    extras: Vec<usize>,
    /// A level whose node made last went in a reread (its children in its place), and the
    /// nodes then: none is made last there until the next (as the machine's).
    dissolved: Option<(usize, usize)>,
    /// Why the file is refused, whatever else could match: as the machine, which has gone this
    /// way, refuses it (a mark a level refuses; a fold of no two leaves).
    fatal: Option<String>,
    /// The token a `remember` choice took last at this level: `as` that choice names its kind.
    /// A nested level starts with its caller's, which it gives back when it ends.
    word: Option<String>,
    /// The flags this level runs with (`with F A`): its caller's, but where a `with` or a
    /// `without` set them.
    flags: Vec<String>,
    /// The nested levels open: in one, a mark it can not take ends it, and is its caller's.
    nested: usize,
}

impl<'a> Parser<'a> {
    /// A rule, a choice or a token, by name (`Gram::Sym`).
    fn sym(&mut self, name: &String, at: usize, follow: u64) -> Option<usize> {
        let decl = self.decl;
        // A nested level is a machine's level of its own: it ends where none of its arms
        // takes the token, whatever its caller takes next. In it, only the marks and soft
        // keywords it takes count (`follow` is none); where it ends, the marks after its
        // last token are its caller's (`rewound`).
        let nested = decl.rules.iter().any(|r| &r.0 == name && r.1) || decl.nested.contains(name);
        let outer = follow;
        let follow = if nested { 0 } else { follow };

        let rewound = |p: &Self, next: usize| -> Option<usize> {
            if !nested {
                return Some(next);
            }
            let back = (at..next).rev().find(|&i| !p.tokens[i].mark).map_or(next, |i| i + 1);
            p.past_marks(back, outer)
        };
        if let Some((kind, _, body)) = decl.rules.iter().find(|r| &r.0 == name) {
            let me = self.out.len();
            self.out.push(Node { kind: kind.clone(), field: None, supertypes: vec![], extra: false, start: 0, end: 0, size: 0, leaf: false });
            self.open.push(me);
            self.nested += usize::from(nested);
            let next = self.matches(body, at, follow);
            self.nested -= usize::from(nested);
            self.open.pop();
            let next = next?;
            // Its span is its tokens', marks left out: a rule of marks alone has none.
            let first = (at..next).find(|&i| !self.tokens[i].mark)?;
            let last = (at..next).rev().find(|&i| !self.tokens[i].mark)?;
            if self.refused(name, first, last) {
                return None;
            }
            let next = rewound(self, next)?;
            let n = self.out.len();
            let node = &mut self.out[me];
            node.start = self.tokens[first].start;
            node.end = self.tokens[last].end;
            node.size = n - me;
            Some(next)
        } else if let Some((_, supertype, body)) = decl.choices.iter().find(|c| &c.0 == name) {
            let me = self.out.len();
            self.nested += usize::from(nested);
            let next = self.matches(body, at, follow);
            self.nested -= usize::from(nested);
            let next = next?;
            let first = (at..next).find(|&i| !self.tokens[i].mark);
            let last = (at..next).rev().find(|&i| !self.tokens[i].mark);
            if let (Some(first), Some(last)) = (first, last) {
                if self.refused(name, first, last) {
                    return None;
                }
            }
            let next = rewound(self, next)?;
            if let (Some(s), true) = (supertype, me < self.out.len()) {
                self.out[me].supertypes.insert(0, s.clone());
            }
            Some(next)
        } else {
            self.token(name, at, follow)
        }
    }

    /// A kind written in the grammar: a `remember` choice's name is its level's word's kind.
    fn kind_of(&self, k: &str) -> Option<String> {
        if self.decl.remember.iter().any(|c| c == k) { self.word.clone() } else { Some(k.to_owned()) }
    }

    /// The node made last at the innermost open node's level: its last child; none where a
    /// reread's node went and nothing was made after (`dissolved`).
    fn last_made(&self) -> Option<usize> {
        let o = *self.open.last()?;
        if self.dissolved == Some((o, self.out.len())) {
            return None;
        }
        let mut c = None;
        let mut j = o + 1;
        while j < self.out.len() {
            c = Some(j);
            j += self.out[j].size.max(1);
        }
        c
    }

    fn look_bit(&self, kind: &str) -> u64 {
        self.looks.iter().position(|m| m == kind).map_or(0, |k| 1 << k)
    }

    /// The soft keyword the token at `at` may be taken as, its bit; 0 for none.
    fn soft_bit(&self, at: usize) -> u64 {
        self.tokens.get(at).and_then(|t| t.soft.as_deref()).map_or(0, |k| self.look_bit(k))
    }

    /// The looks `g` may start with, and whether it may match nothing.
    fn first(&self, g: &Gram) -> (u64, bool) {
        match g {
            Gram::Lit(k) => (self.look_bit(k), false),
            Gram::Sym(name) => self.named.get(name.as_str()).copied().unwrap_or((self.look_bit(name), false)),
            Gram::Seq(items) => self.first_seq(items),
            Gram::Alt(alts) => alts.iter().fold((0, false), |(b, e), a| {
                let (x, y) = self.first(a);
                (b | x, e || y)
            }),
            Gram::Opt(a) | Gram::Star(a) => (self.first(a).0, true),
            Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) | Gram::Flag { body: a, .. } | Gram::Unless { body: a, .. } => self.first(a),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } => self.first(body),
            Gram::Balanced { open, .. } => (self.look_bit(open), false),
            Gram::Any(_) => (0, false),
        }
    }

    /// The token kinds `g` may start with, and whether it may match nothing.
    fn starts(&self, g: &Gram) -> (Starts, bool) {
        match g {
            Gram::Lit(k) => (Starts::of(k), false),
            Gram::Sym(name) => self.named_starts.get(name.as_str()).cloned().unwrap_or_else(|| (Starts::of(name), false)),
            Gram::Seq(items) => {
                let mut s = Starts::default();
                for i in items {
                    let (b, empty) = self.starts(i);
                    s.add(&b);
                    if !empty {
                        return (s, false);
                    }
                }
                (s, true)
            }
            Gram::Alt(alts) => alts.iter().fold((Starts::default(), false), |(mut s, e), a| {
                let (x, y) = self.starts(a);
                s.add(&x);
                (s, e || y)
            }),
            Gram::Opt(a) | Gram::Star(a) => (self.starts(a).0, true),
            Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) | Gram::Flag { body: a, .. } | Gram::Unless { body: a, .. } => self.starts(a),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } => self.starts(body),
            Gram::Balanced { open, .. } => (Starts::of(open), false),
            Gram::Any(except) => (Starts { kinds: Default::default(), any: Some(except.iter().cloned().collect()) }, false),
        }
    }

    /// The grammar token at `at` (no mark) starts `g`: the machine, which decides by the token,
    /// takes `g` there and does not go back.
    fn predicts(&self, g: &Gram, at: usize) -> bool {
        self.tokens.get(at).is_some_and(|t| !t.mark && self.starts(g).0.has(t))
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
            // The levels `body` runs, with the flag set or clear; the caller's flags after.
            Gram::Flag { flag, set, body } => {
                let was = self.flags.clone();
                self.flags.retain(|f| f != flag);
                if *set {
                    self.flags.push(flag.clone());
                }
                let next = self.matches(body, at, follow);
                self.flags = was;
                next
            }
            // Where the flag is set, as if `body` were not there.
            Gram::Unless { flag, body } => {
                if self.flags.contains(flag) {
                    return None;
                }
                self.matches(body, at, follow)
            }
            Gram::Sym(name) => {
                // A nested level gives its caller's word back when it ends; a `remember` choice
                // makes the token it took its level's word.
                let nested = self.decl.rules.iter().any(|r| &r.0 == name && r.1) || self.decl.nested.contains(name);
                let word = self.word.clone();
                let me = self.out.len();
                let next = self.sym(name, at, follow);
                if nested {
                    self.word = word;
                } else if next.is_some() && self.decl.remember.contains(name) && me < self.out.len() {
                    self.word = Some(self.out[me].kind.clone());
                }
                next
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
                // A soft keyword: the alternatives that can start with it, where there are any.
                let soft = self.soft_bit(at);
                let keyword = soft != 0 && alts.iter().any(|a| self.first(a).0 & soft != 0);
                let mut ways: Vec<&Gram> = alts.iter().filter(|a| !keyword || self.first(a).0 & soft != 0).collect();
                // The ways the token starts, where there are any: the machine takes one of them.
                if ways.iter().any(|a| self.predicts(a, at)) {
                    ways.retain(|a| self.predicts(a, at));
                }
                for a in ways {
                    if let Some(p) = self.matches(a, at, follow) {
                        return Some(p);
                    }
                    self.out.truncate(mark);
                }
                None
            }
            Gram::Opt(a) => {
                let mark = self.out.len();
                // A soft keyword that may come after it and does not start it: past it.
                let soft = self.soft_bit(at);
                if soft != 0 && self.first(a).0 & soft == 0 && follow & soft != 0 {
                    return Some(at);
                }
                // One that starts it is its keyword: taken, as the machine takes it (no going back
                // to the word). So is any token that starts it.
                if (soft != 0 && self.first(a).0 & soft != 0) || self.predicts(a, at) {
                    return self.matches(a, at, follow);
                }
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
                    // A soft keyword that may come after it and does not start another: done.
                    let soft = self.soft_bit(p);
                    if soft != 0 && self.first(a).0 & soft == 0 && follow & soft != 0 {
                        return Some(p);
                    }
                    // One that starts another: its keyword, taken; so is any token that starts it.
                    if (soft != 0 && self.first(a).0 & soft != 0) || self.predicts(a, p) {
                        let q = self.matches(a, p, again)?;
                        if q == p {
                            return Some(p);
                        }
                        p = q;
                        continue;
                    }
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
                // The innermost field wins, as the machine's (`f: (g: A)` gives A's node `g`).
                let me = self.out.len();
                let next = self.matches(a, at, follow)?;
                if me < self.out.len() && self.out[me].field.is_none() {
                    self.out[me].field = Some(f.clone());
                }
                Some(next)
            }
            Gram::Retag(kind, a) => {
                let next = self.matches(a, at, follow)?;
                let me = *self.open.last()?;
                self.out[me].kind = self.kind_of(kind)?;
                Some(next)
            }
            Gram::As(kind, a) => {
                let me = self.out.len();
                let next = self.matches(a, at, follow)?;
                if self.out.len() != me + 1 {
                    return None;
                }
                if kind == crate::syntax::IMPLIED {
                    // A token the grammar implies (`implied`): no leaf.
                    self.out.truncate(me);
                } else {
                    self.out[me].kind = self.kind_of(kind)?;
                }
                Some(next)
            }
            Gram::Any(except) => {
                let t = self.tokens.get(at).filter(|t| !t.mark && !except.contains(&t.kind))?;
                let extra = self.decl.extra.contains(&t.kind);
                let (kind, start, end) = (t.kind.clone(), t.start, t.end);
                self.out.push(Node { kind, field: None, supertypes: vec![], extra, start, end, size: 1, leaf: true });
                self.past_marks(at + 1, follow)
            }
            Gram::Balanced { kind, open, close } => {
                // The opener's leaf, then every token to its matching closer, marks skipped:
                // one leaf of `kind`.
                let me = self.out.len();
                let mut p = self.token(open, at, 0)?;
                let mut depth = 1usize;
                loop {
                    let t = self.tokens.get(p)?;
                    if !t.mark {
                        if &t.kind == open {
                            depth += 1;
                        } else if &t.kind == close {
                            depth -= 1;
                        }
                    }
                    p += 1;
                    if depth == 0 {
                        let end = t.end;
                        self.out[me].kind = kind.clone();
                        self.out[me].end = end;
                        return self.past_marks(p, follow);
                    }
                }
            }
            Gram::Fold { kind, body } => {
                // The open node's two leaves, and no extra from its start to this token.
                let o = *self.open.last()?;
                let (a, b) = (o + 1, o + 2);
                let here = self.tokens.get(at)?.start;
                let two = self.out.len() == o + 3 && self.out[a].size == 1 && self.out[b].size == 1;
                if !two || self.extras.iter().any(|&e| e >= self.out[a].start && e < here) {
                    // Where its body matches, the machine has taken this way, and refuses.
                    let len = self.out.len();
                    if self.matches(body, at, follow).is_some() {
                        self.fatal.get_or_insert_with(|| format!("a fold of no two leaves at byte {here}"));
                    }
                    self.out.truncate(len);
                    return None;
                }
                let second = self.out.pop().expect("two leaves");
                let was = (self.out[a].kind.clone(), self.out[a].end);
                self.out[a].kind = kind.clone();
                self.out[a].end = second.end;
                let len = self.out.len();
                match self.matches(body, at, follow) {
                    Some(next) => Some(next),
                    None => {
                        self.out.truncate(len);
                        (self.out[a].kind, self.out[a].end) = was;
                        self.out.push(second);
                        None
                    }
                }
            }
            Gram::Reread { reading, body } => {
                // The node just made at this level, as a wrap finds it; read, then `body`.
                let c = self.last_made()?;
                let r = &self.decl.readings.iter().find(|r| r.0 == *reading)?.1;
                let saved: Vec<Node> = self.out[c..].to_vec();
                let was = self.dissolved;
                let goes = match &r.root {
                    Some(w) => matches!(w, crate::syntax::ReadKind::Gone),
                    None => r.kinds.iter().any(|k| k.0 == self.out[c].kind && matches!(k.1, crate::syntax::ReadKind::Gone)),
                };
                if let Err(why) = reread(&mut self.out, c, r, &self.decl.remember, self.word.as_deref()) {
                    self.out.truncate(c);
                    self.out.extend(saved);
                    // Where its body starts, the machine has taken this way, and refuses.
                    if self.predicts(body, at) {
                        let here = self.tokens[at].start;
                        self.fatal.get_or_insert_with(|| format!("{why}, read at byte {here}"));
                    }
                    return None;
                }
                // The node read went, its children in its place: none made last here.
                if goes {
                    let o = *self.open.last()?;
                    self.dissolved = Some((o, self.out.len()));
                }
                match self.matches(body, at, follow) {
                    Some(next) => Some(next),
                    None => {
                        self.out.truncate(c);
                        self.out.extend(saved);
                        self.dissolved = was;
                        None
                    }
                }
            }
            Gram::Relabel { moved: Some(m), body, .. } if m == crate::syntax::GONE => {
                // `drop`: the node just made at this level, a leaf and the last node made, goes.
                // It stays, of kind `GONE`, until the parse ends, as the machine's record does
                // (a reading names no gone node); none is made last here until the next.
                let c = self.last_made()?;
                if !self.out[c].leaf || c + 1 != self.out.len() {
                    return None;
                }
                let o = *self.open.last()?;
                let was = (std::mem::replace(&mut self.out[c].kind, crate::syntax::GONE.into()), self.dissolved);
                self.dissolved = Some((o, self.out.len()));
                let len = self.out.len();
                match self.matches(body, at, follow) {
                    Some(next) => Some(next),
                    None => {
                        self.out.truncate(len);
                        (self.out[c].kind, self.dissolved) = was;
                        None
                    }
                }
            }
            Gram::Relabel { field, moved, inner, body } => {
                // The node just made at this level, as a wrap finds it; relabelled, then `body`.
                let c = self.last_made()?;
                let was = (self.out[c].field.clone(), self.out[c].kind.clone());
                // `inner`: the node a wrap holds first, right after it; only a wrap has one.
                let held = match inner {
                    Some(g) => {
                        if self.out[c].size < 2 {
                            return None;
                        }
                        let w = self.out[c + 1].field.clone();
                        self.out[c + 1].field = Some(g.clone());
                        Some(w)
                    }
                    None => None,
                };
                if let Some(f) = field {
                    self.out[c].field = Some(f.clone());
                }
                if let Some(m) = moved {
                    self.out[c].kind = self.kind_of(m)?;
                }
                let len = self.out.len();
                match self.matches(body, at, follow) {
                    Some(next) => Some(next),
                    None => {
                        // Undone: the node as it was.
                        self.out.truncate(len);
                        (self.out[c].field, self.out[c].kind) = was;
                        if let Some(w) = held {
                            self.out[c + 1].field = w;
                        }
                        None
                    }
                }
            }
            Gram::Wrap { kind, field, moved, body } => {
                // The node just made at this level: the innermost open node's last child.
                let c = self.last_made()?;
                let was = (self.out[c].field.clone(), self.out[c].kind.clone());
                let node = Node { kind: kind.clone(), field: was.0.clone(), supertypes: vec![], extra: false, start: self.out[c].start, end: 0, size: 0, leaf: false };
                self.out.insert(c, node);
                self.out[c + 1].field = field.clone();
                if let Some(m) = moved {
                    self.out[c + 1].kind = self.kind_of(m)?;
                }
                let len = self.out.len();
                self.open.push(c);
                let r = self.matches(body, at, follow);
                self.open.pop();
                match r {
                    Some(next) => {
                        let end = (at..next).rev().find(|&i| !self.tokens[i].mark).map_or(self.out[c + 1].end, |i| self.tokens[i].end);
                        self.out[c].end = end;
                        self.out[c].size = self.out.len() - c;
                        Some(next)
                    }
                    None => {
                        // Undone: the tree as it was before the wrap.
                        self.out.truncate(len);
                        self.out.remove(c);
                        (self.out[c].field, self.out[c].kind) = was;
                        None
                    }
                }
            }
            Gram::Merge(kind, a) => {
                // The leaf just before, and the token's: one leaf.
                let me = self.out.len();
                let next = self.matches(a, at, follow)?;
                if me == 0 || self.out.len() != me + 1 || self.out[me].size != 1 || self.out[me - 1].size != 1 {
                    return None;
                }
                let end = self.out[me].end;
                self.out.pop();
                let leaf = &mut self.out[me - 1];
                leaf.kind = kind.clone();
                leaf.end = end;
                Some(next)
            }
        }
    }

    /// A mark that level `name` refuses (`refuse M in name`) between its first token and its last.
    fn refused(&mut self, name: &str, first: usize, last: usize) -> bool {
        let marks: Vec<&String> = self.decl.refuse.iter().filter(|r| r.1.iter().any(|l| l == name)).map(|r| &r.0).collect();
        let t = self.tokens[(first + 1).min(last)..last].iter().find(|t| t.mark && marks.contains(&&t.kind));
        if let Some(t) = t {
            self.fatal.get_or_insert_with(|| format!("mark {} in {name} at byte {}", t.kind, t.start));
        }
        t.is_some()
    }

    /// A token of `kind` at `at`: its leaf (a mark has none), then past the marks that `follow`
    /// does not hold. A hidden one is skipped; one not hidden fails the parse.
    fn token(&mut self, kind: &str, at: usize, follow: u64) -> Option<usize> {
        // Its own kind, or the soft keyword it is.
        let t = self.tokens.get(at).filter(|t| t.kind == kind || t.soft.as_deref() == Some(kind))?;
        if !t.mark {
            let extra = self.decl.extra.contains(&t.kind);
            self.out.push(Node { kind: kind.to_owned(), field: None, supertypes: vec![], extra, start: t.start, end: t.end, size: 1, leaf: true });
        }
        self.past_marks(at + 1, follow)
    }

    /// From `at`, past the marks the grammar can not take here (`follow`): hidden ones are
    /// skipped; one not hidden, `None`. In a nested level, one not hidden ends the level, as
    /// the machine's: its caller takes it or refuses the file (`sym`'s `rewound`).
    fn past_marks(&self, mut at: usize, follow: u64) -> Option<usize> {
        while let Some(t) = self.tokens.get(at).filter(|t| t.mark) {
            let bit = self.look_bit(&t.kind);
            if follow & bit != 0 {
                break;
            }
            if self.hidden & bit == 0 {
                return (self.nested > 0).then_some(at);
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
    for (n, _) in &names {
        p.named_starts.insert(n, (Starts::default(), false));
    }
    loop {
        let mut changed = false;
        for &(n, g) in &names {
            let f = p.first(g);
            if p.named[n] != f {
                p.named.insert(n, f);
                changed = true;
            }
            let s = p.starts(g);
            if p.named_starts[n] != s {
                p.named_starts.insert(n, s);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// Node `root` (in preorder) and the nodes inside it, read by `r`: a kind it names as `K` or
/// `K as M` stays (as `M`) and the nodes inside are read; `gone`, the node goes and the nodes
/// inside take its place; `keep`, it stays with every node inside, unread. A node read but the
/// first takes the field `r` gives its own. A kind it does not name refuses, as a first node
/// that goes does.
fn reread(out: &mut Vec<Node>, root: usize, r: &crate::syntax::Reading, remember: &[String], word: Option<&str>) -> Result<(), String> {
    use crate::syntax::ReadKind;
    let end = root + out[root].size;
    let mut gone = Vec::new();
    let mut i = root;
    while i < end {
        let what = match &r.root {
            Some(w) if i == root => Some(w.clone()),
            _ => r.kinds.iter().find(|k| k.0 == out[i].kind).map(|k| k.1.clone()),
        };
        let Some(what) = what else { return Err(format!("a {} the reading does not name", out[i].kind)) };
        if i != root {
            let to = out[i].field.as_ref().and_then(|f| r.fields.iter().find(|x| x.0 == *f)).map(|x| x.1.clone());
            if to.is_some() {
                out[i].field = to;
            }
        }
        match what {
            ReadKind::Keep => {
                i += out[i].size.max(1);
                continue;
            }
            ReadKind::Gone => gone.push(i),
            // `K as C`, `C` a `remember` choice: the level's word's kind.
            ReadKind::As(m) if remember.contains(&m) => out[i].kind = word.ok_or("a word's kind, and no word")?.to_owned(),
            ReadKind::As(m) => out[i].kind = m,
        }
        i += 1;
    }
    if gone.is_empty() {
        return Ok(());
    }
    // Each node's size, less the nodes gone inside it; then they go.
    for x in root..end {
        let (a, b) = (x, x + out[x].size);
        out[x].size -= gone.iter().filter(|&&g| g > a && g < b).count();
    }
    for &g in gone.iter().rev() {
        out.remove(g);
    }
    Ok(())
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
    let looks: Vec<String> = decl.marks.iter().chain(&decl.soft).cloned().collect();
    if looks.len() > 64 {
        return Err("more than 64 marks and soft keywords".into());
    }
    let extra_at = extras.iter().map(|e| e.1.start).collect();
    let mut p = Parser { decl, tokens: &plain, out: Vec::new(), looks, hidden, named: HashMap::new(), named_starts: HashMap::new(), open: Vec::new(), extras: extra_at, dissolved: None, fatal: None, word: None, flags: Vec::new(), nested: 0 };
    first_of(&mut p);
    // A whole root is opened at byte 0 and matched by its content, which may be empty. Marks
    // before the first token are kept where the root can start with them.
    let root_sym = Gram::Sym(root.clone());
    let matched = if decl.whole {
        let body = &decl.rules[0].2;
        p.out.push(Node { kind: root.clone(), field: None, supertypes: vec![], extra: false, start: 0, end: 0, size: 0, leaf: false });
        let first = p.first(body).0;
        p.past_marks(0, first).and_then(|at| p.matches(body, at, 0))
    } else {
        let first = p.first(&root_sym).0;
        match p.past_marks(0, first) {
            Some(at) => p.matches(&root_sym, at, 0),
            None => None,
        }
    };
    if let Some(why) = p.fatal.take() {
        return Err(why);
    }
    let mut out = match matched {
        Some(next) if next == plain.len() => p.out,
        Some(next) => return Err(format!("the token at byte {} is past the grammar's end", plain[next].start)),
        None => return Err("no parse: the tokens do not match the grammar".into()),
    };
    // The nodes dropped (`drop`) go, each node's size less those inside it.
    if out.iter().any(|x| x.kind == crate::syntax::GONE) {
        let mut before = vec![0usize; out.len() + 1];
        for (i, x) in out.iter().enumerate() {
            before[i + 1] = before[i] + usize::from(x.kind == crate::syntax::GONE);
        }
        for i in 0..out.len() {
            let e = (i + out[i].size).min(out.len());
            out[i].size -= before[e] - before[i];
        }
        out.retain(|x| x.kind != crate::syntax::GONE);
    }
    // A provisional field left: no tree of the grammar's.
    if let Some(x) = out.iter().find(|x| x.field.as_ref().is_some_and(|f| decl.provisional.contains(f))) {
        return Err(format!("a provisional field {} left at byte {}", x.field.as_deref().unwrap_or_default(), x.start));
    }
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
    let real_tokens: Vec<&Token> = plain.iter().filter(|t| !t.mark).collect();
    // The deepest node holding tokens `k - 1` and `k` (by spans: a token the grammar implies has
    // no leaf, but its node holds it), and where among its children an extra between them goes:
    // before the first that starts at or after token `k`.
    let between = |k: usize| -> (usize, usize) {
        let (a, b) = (real_tokens[k - 1], real_tokens[k]);
        let mut o = 0;
        'down: loop {
            let mut j = o + 1;
            while j < o + out[o].size {
                if out[j].start <= a.start && b.end <= out[j].end {
                    o = j;
                    continue 'down;
                }
                j += out[j].size.max(1);
            }
            break;
        }
        let mut at = o + 1;
        while at < o + out[o].size && out[at].start < b.start {
            at += out[at].size.max(1);
        }
        (at, o)
    };
    let ancestors = |mut i: usize| {
        let mut up = vec![i];
        while parent[i] != usize::MAX {
            i = parent[i];
            up.push(i);
        }
        up
    };
    // Where each goes, in the tree as parsed: (index to insert at, the node it is in).
    // An extra inside a leaf is part of its text: no node (`usize::MAX`).
    let mut places: Vec<(usize, usize)> = Vec::new();
    for &(k, _) in &extras {
        places.push(if k == 0 {
            (1, 0)
        } else if k == real {
            (out.len(), 0)
        } else {
            // Inside a leaf (a merged one): part of its text.
            let (at, within) = between(k);
            if out[within].leaf { (usize::MAX, 0) } else { (at, within) }
        });
    }
    // Inserted from the last, so that each place is still the tree's as parsed; extras at one
    // place keep their order.
    for (&(k, t), &(at, within)) in extras.iter().zip(&places).rev() {
        let _ = k;
        if at == usize::MAX {
            continue;
        }
        for a in ancestors(within) {
            out[a].size += 1;
        }
        out.insert(at, Node { kind: t.kind.clone(), field: None, supertypes: vec![], extra: true, start: t.start, end: t.end, size: 1, leaf: true });
    }
    out[0].start = out[0].start.min(extras[0].1.start);
    Ok(out)
}
