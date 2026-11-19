//! The clause being lowered, read off its file's tape in place (decision 0276, stage 6 A2). Its literals and terms
//! are positions in the tape, read where they are, as many times as the lowering reads them: a literal beside an
//! aggregate is read again for the aggregate's context, a group's alternatives for each choice.
//!
//! What the lowering states beside the written literals is a position too, or names variables only: an aggregate
//! hoisted out of a term (`v = count : { … }`) is the aggregate's node (`LitAt::Bound`), a tape operation's atom is
//! its call's node (`LitAt::Op`), a cover is a relation over variables (`LitAt::Made`). A term a hoisting moved is
//! read as the variable that stands in its place, which its literal's substitution names (`Sub`); so is a wildcard
//! the lowering names.

use std::collections::{BTreeSet, HashMap};

use super::super::from_tape::{Tape, code_body, op};
use super::super::tape::{Field, Kind};
use super::{Error, Lowerer, RELATION_ARG, TAPE_OPS, groups, test_of};
use rerail_rr_syntax::syntax::tick_vars;

/// The variables standing in place of the terms a hoisting moved (or the wildcards the lowering named), by the
/// term's node, over those of its parent. Set 0 is the clause as written.
#[derive(Clone, Debug, Default)]
pub(super) struct Sub {
    parent: u32,
    map: HashMap<u32, String>,
}

/// A literal of the clause being lowered.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum LitAt {
    /// A literal node (an atom, a negation, a comparison, a pattern, a group, or a body's disjunction), read under
    /// substitution set .1.
    Node(u32, u32),
    /// `v = t`: term node .0, hoisted out of where it stood; v stands there now (set .1 names it).
    Bound(u32, u32),
    /// A tape operation's or `@tree_digest`'s call node .0, as the atom of the relation the engine makes its value
    /// in, the value the variable set .1 names (`Lowerer::hoist`).
    Op(u32, u32),
    /// `v = t`: an aggregate's value or key that is a term, bound to a variable of its body (`Lowerer::aggregate`).
    Eq(String, At),
    /// `rel(v, …)`: a relation the lowering made (a cover), over variables.
    Made(String, Vec<String>),
}

/// A term: a node (read under a substitution set: as its variable where the set names it), a node read as written
/// at its top (a hoisted term, under the variable that stands for it), or a variable the lowering names.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum At {
    Node(u32, u32),
    Bare(u32, u32),
    Var(String),
}

/// A literal, read: its terms and alternatives are positions.
#[derive(Debug)]
pub(super) enum L {
    Atom { rel: String, args: Vec<At>, neg: bool },
    Cmp { l: At, op: &'static str, r: At },
    Pattern { subject: At, pattern: String, selector: Option<String>, neg: bool },
    Group { alts: Vec<Vec<LitAt>>, neg: bool },
}

/// A term, read.
#[derive(Debug)]
pub(super) enum T {
    Var(String),
    Wild,
    Num(i64),
    /// A decimal number, as written.
    Float(String),
    Str(String),
    Tick(String),
    Call { name: String, functor: bool, args: Vec<At> },
    Bin { op: &'static str, l: At, r: At },
    /// `-t`: `0 - t`.
    Neg(At),
    Agg { op: String, over: Option<At>, by: Vec<At>, sep: Option<String>, body: Vec<LitAt> },
    Json(String),
    Named(String, At),
}

/// The substitution set literal `l` is read under (0 for one of the lowering's own, over variables).
pub(super) fn sub_of_lit(l: &LitAt) -> u32 {
    match l {
        LitAt::Node(_, s) | LitAt::Bound(_, s) | LitAt::Op(_, s) | LitAt::Eq(_, At::Node(_, s) | At::Bare(_, s)) => *s,
        LitAt::Eq(_, At::Var(_)) | LitAt::Made(..) => 0,
    }
}

/// Literal `l`, read under substitution set `s` (one over its own).
pub(super) fn with_sub(l: &LitAt, s: u32) -> LitAt {
    match l {
        LitAt::Node(n, _) => LitAt::Node(*n, s),
        LitAt::Bound(n, _) => LitAt::Bound(*n, s),
        LitAt::Op(n, _) => LitAt::Op(*n, s),
        LitAt::Eq(v, a) => LitAt::Eq(v.clone(), with_sub_at(a.clone(), s)),
        LitAt::Made(..) => l.clone(),
    }
}

/// Term `a`, read under substitution set `s`.
pub(super) fn with_sub_at(a: At, s: u32) -> At {
    match a {
        At::Node(n, _) => At::Node(n, s),
        At::Bare(n, _) => At::Bare(n, s),
        a => a,
    }
}

/// A clause's head, read.
pub(super) struct HeadAt {
    pub modal: Option<String>,
    pub rel: String,
    pub args: Vec<At>,
}

impl Lowerer {
    /// The tape of the clause being lowered.
    pub(super) fn tape(&self) -> Tape<'_> {
        self.file.as_ref().expect("a clause being lowered").tape()
    }

    /// Starts the clause at `node` of `file`: no substitution yet.
    pub(super) fn start_clause(&mut self, file: std::sync::Arc<super::super::from_tape::RrFile>) {
        self.file = Some(file);
        self.subs.clear();
        self.subs.push(Sub::default());
    }

    /// A substitution set over `parent`'s.
    pub(super) fn new_sub(&mut self, parent: u32) -> u32 {
        self.subs.push(Sub { parent, map: HashMap::new() });
        (self.subs.len() - 1) as u32
    }

    /// Variable `v` stands in place of node `n` under set `s`.
    pub(super) fn set_sub(&mut self, s: u32, n: u32, v: String) {
        self.subs[s as usize].map.insert(n, v);
    }

    /// The variable standing in place of node `n` under set `s`.
    fn sub_of(&self, mut s: u32, n: u32) -> Option<&String> {
        loop {
            let sub = &self.subs[s as usize];
            if let Some(v) = sub.map.get(&n) {
                return Some(v);
            }
            if s == 0 {
                return None;
            }
            s = sub.parent;
        }
    }

    /// Node `n`, through parentheses: a parenthesised term is the term.
    fn unparen(&self, t: &Tape<'_>, mut n: u32) -> u32 {
        while t.is(n, Kind::ParenthesizedExpression) {
            n = t.one(n, Field::Expression).expect("a parenthesised term");
        }
        n
    }

    /// The arguments of list node `list`, under set `s`.
    fn args_of(&self, t: &Tape<'_>, list: u32, s: u32) -> Vec<At> {
        t.all(list, Field::Argument).map(|a| At::Node(self.unparen(t, a), s)).collect()
    }

    /// The literals of a body or a conjunction node: a disjunction is one literal, its group.
    pub(super) fn body_at(&self, b: u32, s: u32) -> Vec<LitAt> {
        let t = self.tape();
        if t.is(b, Kind::Disjunction) { vec![LitAt::Node(b, s)] } else { t.all(b, Field::Literal).map(|l| LitAt::Node(l, s)).collect() }
    }

    /// The alternatives of a body node: each a conjunction.
    fn alternatives(&self, t: &Tape<'_>, b: u32, s: u32) -> Vec<Vec<LitAt>> {
        if t.is(b, Kind::Disjunction) {
            t.all(b, Field::Alternative).map(|c| t.all(c, Field::Literal).map(|l| LitAt::Node(l, s)).collect()).collect()
        } else {
            vec![t.all(b, Field::Literal).map(|l| LitAt::Node(l, s)).collect()]
        }
    }

    /// Literal `l`, read.
    pub(super) fn lit(&self, l: &LitAt) -> L {
        let t = self.tape();
        match l {
            LitAt::Node(n, s) => self.lit_node(&t, *n, *s),
            LitAt::Bound(n, s) => L::Cmp { l: At::Var(self.sub_of(*s, *n).expect("a hoisted term's variable").clone()), op: "=", r: At::Bare(*n, *s) },
            LitAt::Op(n, s) => {
                let name = t.text(t.one(*n, Field::Function).expect("a call's function"));
                let mut args = t.get(*n, Field::Arguments).map_or_else(Vec::new, |a| self.args_of(&t, a, *s));
                let v = At::Var(self.sub_of(*s, *n).expect("a tape operation's variable").clone());
                let rel = match TAPE_OPS.iter().find(|(f, k, _, _)| *f == name && *k == args.len()) {
                    Some((_, _, rel, _)) => {
                        if *rel == "parse_of" {
                            args.swap(0, 1);
                        }
                        (*rel).to_owned()
                    }
                    None => "tree_digest".to_owned(),
                };
                args.push(v);
                L::Atom { rel, args, neg: false }
            }
            LitAt::Eq(v, a) => L::Cmp { l: At::Var(v.clone()), op: "=", r: a.clone() },
            LitAt::Made(rel, vars) => L::Atom { rel: rel.clone(), args: vars.iter().map(|v| At::Var(v.clone())).collect(), neg: false },
        }
    }

    fn lit_node(&self, t: &Tape<'_>, n: u32, s: u32) -> L {
        let one = |f| t.one(n, f).expect("a literal's part");
        match t.kind(n) {
            k if k == Kind::Atom as u16 => L::Atom { rel: t.text(one(Field::Relation)).to_owned(), args: self.args_of(t, one(Field::Arguments), s), neg: false },
            k if k == Kind::Negation as u16 => match self.lit_node(t, one(Field::Operand), s) {
                L::Atom { rel, args, neg } => L::Atom { rel, args, neg: !neg },
                L::Pattern { subject, pattern, selector, neg } => L::Pattern { subject, pattern, selector, neg: !neg },
                L::Group { alts, neg } => L::Group { alts, neg: !neg },
                L::Cmp { .. } => unreachable!("a negated comparison (refused by `check_clause`)"),
            },
            k if k == Kind::Comparison as u16 => L::Cmp {
                l: At::Node(self.unparen(t, one(Field::Left)), s),
                op: op(t.text(one(Field::Operator))).expect("an operator (`check_clause`)"),
                r: At::Node(self.unparen(t, one(Field::Right)), s),
            },
            k if k == Kind::PatternMatch as u16 => L::Pattern {
                subject: At::Node(self.unparen(t, one(Field::Subject)), s),
                pattern: code_body(t.text(one(Field::Pattern))).to_owned(),
                selector: t.get(n, Field::Selector).map(|x| t.string(x).expect("a string (`check_clause`)")),
                neg: false,
            },
            k if k == Kind::Group as u16 => L::Group { alts: self.alternatives(t, one(Field::Body), s), neg: false },
            k if k == Kind::Disjunction as u16 => L::Group { alts: self.alternatives(t, n, s), neg: false },
            _ => unreachable!("a literal (`check_clause`)"),
        }
    }

    /// Term `a`, read.
    pub(super) fn tv(&self, a: &At) -> T {
        match a {
            At::Var(v) => T::Var(v.clone()),
            At::Node(n, s) => match self.sub_of(*s, *n) {
                Some(v) => T::Var(v.clone()),
                None => self.term_node(*n, *s),
            },
            At::Bare(n, s) => self.term_node(*n, *s),
        }
    }

    fn term_node(&self, n: u32, s: u32) -> T {
        let t = self.tape();
        let one = |f| t.one(n, f).expect("a term's part");
        let compact = || t.text(n).split_whitespace().collect::<String>();
        match t.kind(n) {
            k if k == Kind::Variable as u16 => T::Var(t.text(n).to_owned()),
            k if k == Kind::Wildcard as u16 => T::Wild,
            k if k == Kind::Number as u16 => T::Num(compact().parse().expect("a number (`check_clause`)")),
            k if k == Kind::Float as u16 => T::Float(compact()),
            k if k == Kind::String as u16 => T::Str(t.string(n).expect("a string (`check_clause`)")),
            k if k == Kind::CodeLiteral as u16 => T::Tick(code_body(t.text(n)).to_owned()),
            k if k == Kind::JsonObject as u16 => T::Json(t.text(n).to_owned()),
            k if k == Kind::Call as u16 || k == Kind::FunctorCall as u16 => T::Call {
                name: t.text(one(Field::Function)).to_owned(),
                functor: k == Kind::FunctorCall as u16,
                args: t.get(n, Field::Arguments).map_or_else(Vec::new, |a| self.args_of(&t, a, s)),
            },
            k if k == Kind::BinaryExpression as u16 => T::Bin {
                op: op(t.text(one(Field::Operator))).expect("an operator (`check_clause`)"),
                l: At::Node(self.unparen(&t, one(Field::Left)), s),
                r: At::Node(self.unparen(&t, one(Field::Right)), s),
            },
            k if k == Kind::UnaryExpression as u16 => T::Neg(At::Node(self.unparen(&t, one(Field::Operand)), s)),
            k if k == Kind::ParenthesizedExpression as u16 => self.term_node(self.unparen(&t, n), s),
            k if k == Kind::Aggregate as u16 => T::Agg {
                // `count` is implied: the aggregate with a body alone.
                op: t.get(n, Field::Operator).map_or("count", |o| t.text(o)).to_owned(),
                over: t.get(n, Field::Value).map(|v| At::Node(self.unparen(&t, v), s)),
                by: t.all(n, Field::Key).map(|k| At::Node(self.unparen(&t, k), s)).collect(),
                sep: t.get(n, Field::Separator).map(|x| t.string(x).expect("a string (`check_clause`)")),
                body: self.body_at(one(Field::Body), s),
            },
            k if k == Kind::NamedArgument as u16 => T::Named(t.text(one(Field::Name)).to_owned(), At::Node(self.unparen(&t, one(Field::Value)), s)),
            _ => unreachable!("a term (`check_clause`)"),
        }
    }

    /// Clause node `c`'s heads.
    pub(super) fn heads_at(&self, c: u32) -> Vec<HeadAt> {
        let t = self.tape();
        t.all(c, Field::Head)
            .map(|h| HeadAt {
                modal: t.get(h, Field::Modal).map(|m| t.text(m).to_owned()),
                rel: t.text(t.one(h, Field::Relation).expect("a head's relation")).to_owned(),
                args: self.args_of(&t, t.one(h, Field::Arguments).expect("a head's arguments"), 0),
            })
            .collect()
    }

    /// Whether term `a` is a float: a decimal number, `to_float(…)`, or arithmetic on one. (A variable's type is not
    /// followed: `0.0 - f` is a float, `f - g` is not.)
    pub(super) fn is_float(&self, a: &At) -> bool {
        match self.tv(a) {
            T::Float(_) => true,
            T::Call { name, functor: false, args } => name == "to_float" || (matches!(name.as_str(), "max" | "min") && args.iter().any(|a| self.is_float(a))),
            T::Bin { l, r, .. } => self.is_float(&l) || self.is_float(&r),
            T::Neg(x) => self.is_float(&x),
            _ => false,
        }
    }

    /// Whether two terms are written the same.
    pub(super) fn same(&self, a: &At, b: &At) -> bool {
        match (self.tv(a), self.tv(b)) {
            (T::Var(x), T::Var(y)) | (T::Float(x), T::Float(y)) | (T::Str(x), T::Str(y)) | (T::Tick(x), T::Tick(y)) | (T::Json(x), T::Json(y)) => x == y,
            (T::Wild, T::Wild) => true,
            (T::Num(x), T::Num(y)) => x == y,
            (T::Call { name: n, functor: f, args: x }, T::Call { name: m, functor: g, args: y }) => n == m && f == g && x.len() == y.len() && x.iter().zip(&y).all(|(a, b)| self.same(a, b)),
            (T::Bin { op: o, l: a, r: b }, T::Bin { op: p, l: c, r: d }) => o == p && self.same(&a, &c) && self.same(&b, &d),
            (T::Neg(a), T::Neg(b)) => self.same(&a, &b),
            (T::Named(n, a), T::Named(m, b)) => n == m && self.same(&a, &b),
            // An aggregate is the same only as itself.
            (T::Agg { .. }, T::Agg { .. }) => a == b,
            _ => false,
        }
    }

    /// The variables in term `a`, patterns' and templates' `$X` included.
    pub(super) fn term_vars(&self, a: &At, out: &mut Vec<String>) {
        match self.tv(a) {
            T::Var(v) => out.push(v),
            T::Tick(x) => out.extend(tick_vars(&x)),
            T::Call { args, .. } => args.iter().for_each(|a| self.term_vars(a, out)),
            T::Bin { l, r, .. } => {
                self.term_vars(&l, out);
                self.term_vars(&r, out);
            }
            T::Neg(x) | T::Named(_, x) => self.term_vars(&x, out),
            T::Agg { over, by, body, .. } => {
                over.iter().for_each(|o| self.term_vars(o, out));
                by.iter().for_each(|b| self.term_vars(b, out));
                body.iter().for_each(|l| self.lit_vars(l, out));
            }
            T::Wild | T::Num(_) | T::Float(_) | T::Str(_) | T::Json(_) => {}
        }
    }

    /// The variables of term `a`'s own scope: none inside an aggregate.
    fn term_scope_vars(&self, a: &At, out: &mut Vec<String>) {
        match self.tv(a) {
            T::Agg { .. } => {}
            T::Call { args, .. } => args.iter().for_each(|a| self.term_scope_vars(a, out)),
            T::Bin { l, r, .. } => {
                self.term_scope_vars(&l, out);
                self.term_scope_vars(&r, out);
            }
            T::Neg(x) | T::Named(_, x) => self.term_scope_vars(&x, out),
            _ => self.term_vars(a, out),
        }
    }

    /// The variables in literal `l`.
    pub(super) fn lit_vars(&self, l: &LitAt, out: &mut Vec<String>) {
        match self.lit(l) {
            L::Atom { args, .. } => args.iter().for_each(|a| self.term_vars(a, out)),
            L::Cmp { l, r, .. } => {
                self.term_vars(&l, out);
                self.term_vars(&r, out);
            }
            L::Pattern { subject, pattern, .. } => {
                self.term_vars(&subject, out);
                out.extend(tick_vars(&pattern));
            }
            L::Group { alts, .. } => alts.iter().flatten().for_each(|l| self.lit_vars(l, out)),
        }
    }

    /// The variables of literal `l`'s own scope: not those only inside an aggregate's body, which are that body's own
    /// unless they appear outside it too (Soufflé's scoping).
    pub(super) fn scope_vars(&self, l: &LitAt, out: &mut Vec<String>) {
        match self.lit(l) {
            L::Atom { args, .. } => args.iter().for_each(|a| self.term_scope_vars(a, out)),
            L::Cmp { l, r, .. } => {
                self.term_scope_vars(&l, out);
                self.term_scope_vars(&r, out);
            }
            L::Group { alts, .. } => alts.iter().flatten().for_each(|l| self.scope_vars(l, out)),
            L::Pattern { .. } => self.lit_vars(l, out),
        }
    }

    /// The variables `lits` bind or read in their own scope: an atom's arguments (not a relation argument's name),
    /// `match`'s named groups, and the scope variables of the rest.
    pub(super) fn body_vars(&self, lits: &[LitAt]) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for l in lits {
            match self.lit(l) {
                L::Atom { rel, args, .. } => {
                    // `match` and `match_all` bind their named groups, which no argument names.
                    if matches!(rel.as_str(), "match" | "match_all")
                        && let Some(T::Str(re)) = args.first().map(|a| self.tv(a))
                    {
                        out.extend(groups(&re));
                    }
                    let skip: &[usize] = RELATION_ARG.iter().find(|(r, _)| *r == rel).map_or(&[], |(_, k)| k);
                    for (k, a) in args.iter().enumerate() {
                        if !(skip.contains(&k) && matches!(self.tv(a), T::Var(_))) {
                            let mut vs = Vec::new();
                            self.term_vars(a, &mut vs);
                            out.extend(vs);
                        }
                    }
                }
                L::Group { alts, .. } => alts.iter().for_each(|a| out.extend(self.body_vars(a))),
                _ => {
                    let mut vs = Vec::new();
                    self.scope_vars(l, &mut vs);
                    out.extend(vs);
                }
            }
        }
        out
    }

    /// The variables of one literal (`body_vars`).
    pub(super) fn vars_of(&self, l: &LitAt) -> BTreeSet<String> {
        self.body_vars(std::slice::from_ref(l))
    }

    /// The variables of term `a`.
    pub(super) fn term_var_set(&self, a: &At) -> BTreeSet<String> {
        let mut vs = Vec::new();
        self.term_vars(a, &mut vs);
        vs.into_iter().collect()
    }

    /// `body_vars`, with the variables of aggregates inside comparisons too: what a group shares with its rule when
    /// an aggregate in it is correlated on the rule's variables (`count : { b(x, _) } > 2` shares x).
    pub(super) fn shared_vars(&self, lits: &[LitAt]) -> BTreeSet<String> {
        let mut out = self.body_vars(lits);
        for l in lits {
            match self.lit(l) {
                L::Cmp { l, r, .. } => {
                    let mut vs = Vec::new();
                    self.term_vars(&l, &mut vs);
                    self.term_vars(&r, &mut vs);
                    out.extend(vs);
                }
                L::Group { alts, .. } => alts.iter().for_each(|a| out.extend(self.shared_vars(a))),
                _ => {}
            }
        }
        out
    }

    /// Whether a literal binds every variable of its columns: a positive atom of a relation, not a text test nor a
    /// built-in the lowering writes out as tests or computations of variables bound elsewhere (`within`, `earlier`,
    /// `split_dot`, `nearest`, `precedes_stop`, …).
    pub(super) fn binds_columns(&self, l: &LitAt) -> bool {
        matches!(self.lit(l), L::Atom { rel, neg: false, .. } if test_of(&rel).is_none()
            && !matches!(rel.as_str(), "within" | "within_stop" | "earlier" | "split_dot" | "nearest" | "precedes_stop" | "dfs_first" | "dfs_first_memo" | "unset"))
    }

    /// Whether `l` is a positive atom of `rel`'s kind: a binding atom (not a text test, not `earlier`).
    pub(super) fn binding_atom(&self, l: &LitAt) -> bool {
        matches!(self.lit(l), L::Atom { rel, neg: false, .. } if test_of(&rel).is_none() && rel != "earlier")
    }

    /// The relations `lits` read, inside groups and aggregates too.
    pub(super) fn lit_rels(&self, lits: &[LitAt], out: &mut BTreeSet<String>) {
        for l in lits {
            match self.lit(l) {
                L::Atom { rel, args, .. } => {
                    out.insert(rel);
                    args.iter().for_each(|a| self.term_rels(a, out));
                }
                L::Cmp { l, r, .. } => {
                    self.term_rels(&l, out);
                    self.term_rels(&r, out);
                }
                L::Group { alts, .. } => alts.iter().for_each(|a| self.lit_rels(a, out)),
                L::Pattern { .. } => {}
            }
        }
    }

    fn term_rels(&self, a: &At, out: &mut BTreeSet<String>) {
        match self.tv(a) {
            T::Agg { over, by, body, .. } => {
                self.lit_rels(&body, out);
                over.iter().for_each(|t| self.term_rels(t, out));
                by.iter().for_each(|t| self.term_rels(t, out));
            }
            T::Named(_, t) => self.term_rels(&t, out),
            _ => {}
        }
    }

    /// What the lowering reads of clause `c` can be read: no negated comparison, each operator and number one it
    /// knows, each string a string. Checked once, as the clause's walk starts.
    pub(super) fn check_clause(&self, c: u32) -> Result<(), Error> {
        let t = self.tape();
        let bad = |what: String| Err(Error::Syntax(format!("tape: {what}")));
        for n in c..c + t.size(c) {
            let k = t.kind(n);
            if t.extra(n) {
                continue;
            }
            if k == Kind::Negation as u16 && t.get(n, Field::Operand).is_some_and(|o| t.is(o, Kind::Comparison)) {
                return bad("a negated comparison".to_owned());
            }
            if k == Kind::Comparison as u16 || k == Kind::BinaryExpression as u16 {
                op(t.text(t.one(n, Field::Operator)?))?;
            }
            if k == Kind::Number as u16 && t.text(n).split_whitespace().collect::<String>().parse::<i64>().is_err() {
                return bad(format!("bad number {:?}", t.text(n)));
            }
            if k == Kind::String as u16 {
                t.string(n)?;
            }
        }
        Ok(())
    }
}
