//! A rule read the same way from a tree or from a version (work queue Q131): the checks `front::finish` runs
//! (`cycles::check`, `caches::caches`) are written once over it, and read a version's rules where they are, with no
//! tree decoded.
//!
//! ```text
//! RuleView::Tree(&rule)              a tree's rule
//! RuleView::Image(&version, rule)    a version's rule, by its handle
//! view.head()  -> AtomView           the head: relation, terms
//! view.body()  -> Vec<LitView>       the body's literals, each read now from where it is
//! ```
//!
//! A view's terms and literals are made when asked and dropped after: names are borrowed from the tree or the
//! version's strings, and nothing is kept between reads.

use super::{Literal, Rule, Term};
use crate::code_rule_dsl::logic::program_image::{
    Version,
    handles::{Lit, RuleRef, TermRef},
};
use std::collections::BTreeSet;

/// A term, as the checks read it.
#[derive(Clone, Debug, PartialEq)]
pub enum TermView<'a> {
    Var(&'a str),
    Wild,
    Top,
    Int(i64),
    Str(&'a str),
    Call(&'a str, Vec<TermView<'a>>),
    Branded(super::Branded),
}

impl<'a> TermView<'a> {
    fn of_tree(t: &'a Term) -> TermView<'a> {
        match t {
            Term::Var(v) => TermView::Var(v),
            Term::Wild => TermView::Wild,
            Term::Top => TermView::Top,
            Term::Int(n) => TermView::Int(*n),
            Term::Str(s) => TermView::Str(s),
            Term::Call(f, args) => TermView::Call(f, args.iter().map(TermView::of_tree).collect()),
            Term::Branded(b) => TermView::Branded(*b),
        }
    }

    fn of_image(v: &'a Version, r: RuleRef<'a>, t: TermRef<'a>) -> TermView<'a> {
        match t {
            TermRef::Var(s) => TermView::Var(v.text(r.var(s))),
            TermRef::Wild => TermView::Wild,
            TermRef::Top => TermView::Top,
            TermRef::Int(n) => TermView::Int(n),
            TermRef::Str(a) => TermView::Str(v.text(a)),
            TermRef::Call(f, args) => TermView::Call(v.text(f), args.map(|t| TermView::of_image(v, r, t)).collect()),
            TermRef::Branded(b) => TermView::Branded(b),
        }
    }

    /// `Term::vars`: the variables it reads, added to `out`.
    pub fn vars(&self, out: &mut BTreeSet<String>) {
        match self {
            TermView::Var(v) => {
                out.insert((*v).to_owned());
            }
            TermView::Call(_, args) => args.iter().for_each(|a| a.vars(out)),
            _ => {}
        }
    }

    /// `Term::is_cat`: `@cat` over variables only.
    pub fn is_cat(&self) -> bool {
        matches!(self, TermView::Call(f, args) if *f == "cat" && args.iter().all(|a| matches!(a, TermView::Var(_))))
    }
}

/// An atom: its relation, its terms, and whether negated.
#[derive(Clone, Debug, PartialEq)]
pub struct AtomView<'a> {
    pub rel: &'a str,
    pub args: Vec<TermView<'a>>,
    pub neg: bool,
}

impl AtomView<'_> {
    /// `Atom::vars`.
    pub fn vars(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        self.args.iter().for_each(|a| a.vars(&mut out));
        out
    }
}

/// A body literal. An aggregate's operator, `nearest`'s kind set and `ast`'s matcher are read by no check.
#[derive(Clone, Debug, PartialEq)]
pub enum LitView<'a> {
    Atom(AtomView<'a>),
    Cmp { l: TermView<'a>, op: &'static str, r: TermView<'a> },
    Agg { var: &'a str, over: Option<&'a str>, atom: AtomView<'a> },
    Nearest { n: &'a str, a: &'a str },
    /// Its document, node and captures' variables.
    Ast { vars: Vec<&'a str> },
}

impl LitView<'_> {
    /// `Literal::atom`: its own atom, or its aggregate's.
    pub fn atom(&self) -> Option<&AtomView<'_>> {
        match self {
            LitView::Atom(a) | LitView::Agg { atom: a, .. } => Some(a),
            _ => None,
        }
    }

    /// `Literal::vars`.
    pub fn vars(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        match self {
            LitView::Atom(a) => out = a.vars(),
            LitView::Cmp { l, r, .. } => {
                l.vars(&mut out);
                r.vars(&mut out);
            }
            LitView::Agg { var, over, atom } => {
                out = atom.vars();
                out.insert((*var).to_owned());
                out.extend(over.map(str::to_owned));
            }
            LitView::Nearest { n, a } => out.extend([(*n).to_owned(), (*a).to_owned()]),
            LitView::Ast { vars } => out.extend(vars.iter().map(|v| (*v).to_owned())),
        }
        out
    }
}

/// A rule of a tree or of a version.
#[derive(Clone, Copy)]
pub enum RuleView<'a> {
    Tree(&'a Rule),
    Image(&'a Version, RuleRef<'a>),
}

impl<'a> RuleView<'a> {
    /// Its head's relation.
    pub fn rel(self) -> &'a str {
        match self {
            RuleView::Tree(r) => &r.head.rel,
            RuleView::Image(v, r) => v.rel_name(r.head().rel()),
        }
    }

    pub fn head(self) -> AtomView<'a> {
        match self {
            RuleView::Tree(r) => AtomView { rel: &r.head.rel, args: r.head.args.iter().map(TermView::of_tree).collect(), neg: false },
            RuleView::Image(v, r) => AtomView { rel: v.rel_name(r.head().rel()), args: r.head().args().map(|t| TermView::of_image(v, r, t)).collect(), neg: false },
        }
    }

    pub fn body(self) -> Vec<LitView<'a>> {
        match self {
            RuleView::Tree(r) => r
                .body
                .iter()
                .map(|l| match l {
                    Literal::Atom(a) => LitView::Atom(AtomView { rel: &a.rel, args: a.args.iter().map(TermView::of_tree).collect(), neg: a.neg }),
                    Literal::Cmp { l, op, r } => LitView::Cmp { l: TermView::of_tree(l), op, r: TermView::of_tree(r) },
                    Literal::Agg { var, over, atom, .. } => LitView::Agg { var, over: over.as_deref(), atom: AtomView { rel: &atom.rel, args: atom.args.iter().map(TermView::of_tree).collect(), neg: atom.neg } },
                    Literal::Nearest { n, a, .. } => LitView::Nearest { n, a },
                    Literal::Ast { document, node, captures, .. } => LitView::Ast { vars: [document.as_str(), node.as_str()].into_iter().chain(captures.iter().map(|(_, v)| v.as_str())).collect() },
                })
                .collect(),
            RuleView::Image(v, r) => {
                let atom = |a: crate::code_rule_dsl::logic::program_image::handles::AtomRef<'a>| AtomView { rel: v.rel_name(a.rel()), args: a.args().map(|t| TermView::of_image(v, r, t)).collect(), neg: a.neg() };
                let var = |s| v.text(r.var(s));
                r.body()
                    .map(|l| match l.view() {
                        Lit::Atom(a) => LitView::Atom(atom(a)),
                        Lit::Cmp { l, op, r: rt } => LitView::Cmp { l: TermView::of_image(v, r, l), op: op.as_str(), r: TermView::of_image(v, r, rt) },
                        Lit::Agg { var: x, over, atom: a, .. } => LitView::Agg { var: var(x), over: over.map(var), atom: atom(a) },
                        Lit::Nearest { n, a, .. } => LitView::Nearest { n: var(n), a: var(a) },
                        Lit::Ast { document, node, captures, .. } => LitView::Ast { vars: [var(document), var(node)].into_iter().chain(captures.map(|(_, s)| var(s))).collect() },
                    })
                    .collect()
            }
        }
    }

    /// The rule as `Rule`'s `Display` writes it.
    pub fn shown(self) -> String {
        match self {
            RuleView::Tree(r) => r.to_string(),
            RuleView::Image(v, r) => v.show_rule(r),
        }
    }
}
