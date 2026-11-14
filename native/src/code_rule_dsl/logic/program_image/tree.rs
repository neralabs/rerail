//! A version read as today's tree (`front::Program`), by its handles: every caller not yet moved to ids reads this.
//! Each atom is made a `Name` once; lists are interned from scratch stacks, so a read allocates no list of its own.
use super::handles::{AggRef, Lit, RuleRef, TermRef, Terms};
use super::{AtomId, Cell, Version, assemble};
use crate::code_rule_dsl::logic::front::{AggOp, Atom, List, Literal, Lists, Name, Program, Rule, Term, Value};
use crate::code_rule_dsl::logic::program_facts::Facts;
use std::collections::BTreeMap;

pub struct Tree<'v> {
    v: &'v Version,
    lists: &'v Lists,
    names: Vec<Option<Name>>,
    terms: Vec<Term>,
    literals: Vec<Literal>,
}

impl<'v> Tree<'v> {
    pub fn new(v: &'v Version, lists: &'v Lists) -> Tree<'v> {
        Tree { v, lists, names: vec![None; v.atom_end() as usize], terms: Vec::new(), literals: Vec::new() }
    }
    pub fn name(&mut self, a: AtomId) -> Name {
        let v = self.v;
        self.names[a.index()].get_or_insert_with(|| Name::new(v.text(a))).clone()
    }
    fn text(&self, a: AtomId) -> String {
        self.v.text(a).to_owned()
    }

    /// Rule `r`, as the tree.
    pub fn rule(&mut self, r: RuleRef<'_>) -> Rule {
        self.terms.clear();
        self.literals.clear();
        let head = self.atom(r, r.head());
        for l in r.body() {
            let lit = self.literal(r, l.view());
            self.literals.push(lit);
        }
        let body = self.lists.literals(&self.literals);
        self.literals.clear();
        Rule { head, body }
    }
    fn var(&mut self, r: RuleRef<'_>, s: super::Slot) -> Name {
        self.name(r.var(s))
    }
    fn var_text(&self, r: RuleRef<'_>, s: super::Slot) -> String {
        self.text(r.var(s))
    }
    fn term(&mut self, r: RuleRef<'_>, t: TermRef<'_>) -> Term {
        match t {
            TermRef::Var(s) => Term::Var(self.var(r, s)),
            TermRef::Wild => Term::Wild,
            TermRef::Top => Term::Top,
            TermRef::Int(i) => Term::Int(i),
            TermRef::Str(a) => Term::Str(self.name(a)),
            TermRef::Call(f, args) => {
                let f = self.name(f);
                Term::Call(f, self.term_list(r, args))
            }
        }
    }
    fn term_list(&mut self, r: RuleRef<'_>, args: Terms<'_>) -> List<Term> {
        let start = self.terms.len();
        for t in args {
            let t = self.term(r, t);
            self.terms.push(t);
        }
        let list = self.lists.terms(&self.terms[start..]);
        self.terms.truncate(start);
        list
    }
    fn atom(&mut self, r: RuleRef<'_>, a: super::AtomRef<'_>) -> Atom {
        let rel = self.name(self.v.rel_atom(a.rel()));
        let args = self.term_list(r, a.args());
        Atom { rel, args, neg: a.neg(), gated: a.gated() }
    }
    fn literal(&mut self, r: RuleRef<'_>, l: Lit<'_>) -> Literal {
        match l {
            Lit::Atom(a) => Literal::Atom(self.atom(r, a)),
            Lit::Cmp { l, op, r: rt } => {
                let l = self.term(r, l);
                let rt = self.term(r, rt);
                Literal::Cmp { l, op: op.as_str(), r: rt }
            }
            Lit::Agg { var, op, over, atom } => {
                let op = match op {
                    AggRef::Count => AggOp::Count,
                    AggRef::Min => AggOp::Min,
                    AggRef::Max => AggOp::Max,
                    AggRef::Sum => AggOp::Sum,
                    AggRef::Product => AggOp::Product,
                    AggRef::Join { by, sep } => AggOp::Join { by: by.map(|s| self.var_text(r, s)).collect(), sep: self.text(sep) },
                };
                Literal::Agg { var: self.var_text(r, var), op, over: over.map(|s| self.var_text(r, s)), atom: self.atom(r, atom) }
            }
            Lit::Nearest { n, a, kindset } => Literal::Nearest { n: self.var_text(r, n), a: self.var_text(r, a), kindset: self.text(kindset) },
            Lit::Ast { document, node, matcher, captures } => Literal::Ast {
                document: self.var_text(r, document),
                node: self.var_text(r, node),
                matcher: self.text(matcher),
                captures: captures.map(|(c, s)| (self.text(c), self.var_text(r, s))).collect(),
            },
        }
    }
    fn rows(&mut self, f: super::version::FactsRef<'_>) -> Vec<List<Value>> {
        let mut out = Vec::with_capacity(f.rows().len());
        let mut row = Vec::new();
        for cells in f.rows() {
            row.clear();
            for c in cells {
                row.push(match c {
                    Cell::Int(i) => Value::Int(i),
                    Cell::Str(a) => Value::Str(self.name(a)),
                });
            }
            out.push(self.lists.values(&row));
        }
        out
    }

    /// The whole program.
    pub fn program(mut self) -> Result<Program, String> {
        let v = self.v;
        let mut rules = Vec::with_capacity(v.size());
        for r in v.rules() {
            rules.push(self.rule(r));
        }
        let mut decls = BTreeMap::new();
        for d in v.decls() {
            let cols: Vec<(Name, Name)> = d.cols().map(|(n, t)| (self.name(n), self.name(t))).collect();
            let name = self.name(d.name());
            decls.insert(name, self.lists.columns(&cols));
        }
        let mut facts = BTreeMap::new();
        for f in v.all_facts() {
            let rows = self.rows(f);
            facts.insert(self.text(f.name()), rows);
        }
        let mut about = Facts::new();
        for f in v.about_facts() {
            let rows = self.rows(f);
            about.insert(self.text(f.name()), rows);
        }
        let inputs = v.inputs().map(|a| self.text(a)).collect();
        let outputs = v.outputs().map(|a| self.text(a)).collect();
        assemble(rules, decls, facts, about, inputs, outputs)
    }
}
