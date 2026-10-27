//! The logic's text syntax: tokens, the AST, stratification and pruning (03 §12; 05 Phase 6 row 2).
//!
//! A port of the front end of Typing-support's `rdl.py` (T, `front/rdl.py` beside this file until the port is
//! done): `tokens`, `parse`, the AST classes, `strata`/`tarjan` and `prune`, with T's grammar. Added for §12:
//! - lattices declared on any relation, `.decl r(k: t; v: L) merge @join widen k`, besides T's `.lattice`;
//! - `min` and `max` aggregates, `$m = min $v : { atom }`, besides `count`;
//! - the matcher atom `ast($D, $P, { … })`, whose matcher text is kept for the transducer lowering (§3);
//! - named errors for aggregation in recursion (unless the head is a monotone lattice) and unbounded lattices.
//!
//! §12's other named errors are not here: an unbounded value cycle needs a definition of its bound (T's own programs
//! recurse through `@cat` along tree edges), a non-interned cross-document join needs column types, and the cost
//! gate is the planner's (row 5).
//!
//! Tree structure is never a relation: `nearest($n, $s, set)` is a built-in over the tape (T's `rdl.py` docstring).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A token and the byte offset where it starts.
#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Var(String),
    Str(String),
    Int(i64),
    P(&'static str),
    Id(String),
    Eof,
}

const PUNCT: [&str; 22] = [":-", "!=", "<=", ">=", "(", ")", ",", ".", ";", ":", "=", "<", ">", "!", "{", "}", "|", "@", "*", "[", "]", "-"];

/// T's `TOKEN` regex: whitespace and `//` comments skipped, then `$var`, `"string"`, `-?digits`, punctuation or an
/// identifier. T had no `*`, so its `.demand *` could not be written; it is a token here (and `[`, `]`, `-` for
/// matcher text).
fn tokens(src: &str) -> Result<Vec<(Tok, usize)>, Error> {
    let bytes = src.as_bytes();
    let (mut pos, mut out) = (0, Vec::new());
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    loop {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            return Ok(out);
        }
        let start = pos;
        let rest = &src[pos..];
        if rest.starts_with("//") {
            pos += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        let c = bytes[pos];
        let digits = |from: usize| from + bytes[from..].iter().take_while(|b| b.is_ascii_digit()).count();
        if c == b'$' && bytes.get(pos + 1).is_some_and(|&b| b.is_ascii_alphabetic() || b == b'_') {
            let end = pos + 1 + bytes[pos + 1..].iter().take_while(|&&b| ident(b)).count();
            out.push((Tok::Var(src[pos + 1..end].to_owned()), start));
            pos = end;
        } else if c == b'"' {
            let end = rest[1..].find('"').ok_or_else(|| Error::Syntax(format!("unterminated string at {:?}", near(src, pos))))?;
            out.push((Tok::Str(rest[1..1 + end].to_owned()), start));
            pos += end + 2;
        } else if c.is_ascii_digit() || (c == b'-' && bytes.get(pos + 1).is_some_and(u8::is_ascii_digit)) {
            let end = digits(pos + 1);
            let value = src[pos..end].parse().map_err(|_| Error::Syntax(format!("bad integer {:?}", &src[pos..end])))?;
            out.push((Tok::Int(value), start));
            pos = end;
        } else if let Some(p) = PUNCT.iter().find(|p| rest.starts_with(**p)) {
            out.push((Tok::P(p), start));
            pos += p.len();
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let end = pos + bytes[pos..].iter().take_while(|&&b| ident(b)).count();
            out.push((Tok::Id(src[pos..end].to_owned()), start));
            pos = end;
        } else {
            return Err(Error::Syntax(format!("bad input at {:?}", near(src, pos))));
        }
    }
}

fn near(src: &str, pos: usize) -> &str {
    let end = (pos..=src.len().min(pos + 40)).rev().find(|&i| src.is_char_boundary(i)).unwrap_or(pos);
    &src[pos..end]
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) enum Term {
    Var(String),
    Wild,
    Int(i64),
    Str(String),
    Top,
    Call(String, Vec<Term>),
}

impl Term {
    pub(in crate::code_rule_dsl) fn vars(&self, out: &mut BTreeSet<String>) {
        match self {
            Term::Var(v) => {
                out.insert(v.clone());
            }
            Term::Call(_, args) => args.iter().for_each(|a| a.vars(out)),
            _ => {}
        }
    }

    /// `@cat` over variables only: the one builtin that can run backwards, splitting a bound result.
    pub(in crate::code_rule_dsl) fn is_cat(&self) -> bool {
        matches!(self, Term::Call(f, args) if f == "cat" && args.iter().all(|a| matches!(a, Term::Var(_))))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) struct Atom {
    pub(in crate::code_rule_dsl) rel: String,
    pub(in crate::code_rule_dsl) args: Vec<Term>,
    pub(in crate::code_rule_dsl) neg: bool,
    /// A negation held until its relation's `done_` gate passes (magic sets, row 4): not a negative edge.
    pub(in crate::code_rule_dsl) gated: bool,
}

impl Atom {
    pub(in crate::code_rule_dsl) fn vars(&self) -> BTreeSet<String> {
        self.args.iter().filter_map(|a| if let Term::Var(v) = a { Some(v.clone()) } else { None }).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum AggOp {
    Count,
    Min,
    Max,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) enum Literal {
    Atom(Atom),
    Cmp { l: Term, op: &'static str, r: Term },
    /// `$var = count { atom }`, or `$var = min $over : { atom }` (`max` likewise).
    Agg { var: String, op: AggOp, over: Option<String>, atom: Atom },
    /// `nearest($n, $a, kindset)`: `$a` is `$n`'s nearest strict ancestor of a kind in the set.
    Nearest { n: String, a: String, kindset: String },
    /// `ast($document, $node, { matcher })`: the matcher's text, lowered to a transducer program (§3).
    Ast { document: String, node: String, matcher: String },
}

impl Literal {
    pub(in crate::code_rule_dsl) fn vars(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        match self {
            Literal::Atom(a) => out = a.vars(),
            Literal::Cmp { l, r, .. } => {
                l.vars(&mut out);
                r.vars(&mut out);
            }
            Literal::Agg { var, over, atom, .. } => {
                out = atom.vars();
                out.insert(var.clone());
                out.extend(over.clone());
            }
            Literal::Nearest { n, a, .. } => out.extend([n.clone(), a.clone()]),
            Literal::Ast { document, node, .. } => out.extend([document.clone(), node.clone()]),
        }
        out
    }

    /// The atom a literal reads a relation through: itself, or an aggregate's.
    pub(in crate::code_rule_dsl) fn atom(&self) -> Option<&Atom> {
        match self {
            Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a),
            _ => None,
        }
    }
}

/// A comparison's variables once it runs, or `None` if it cannot run yet: `$x = e` assigns an unbound `$x` once
/// `e`'s variables are bound; `$m = @cat($q, $x)` with `$m` bound splits it, binding `$q` and `$x` where unbound;
/// otherwise every variable must be bound. T's `cmp_binds`.
pub(in crate::code_rule_dsl) fn cmp_binds(l: &Term, op: &str, r: &Term, bound: &BTreeSet<String>) -> Option<BTreeSet<String>> {
    let mut rv = BTreeSet::new();
    r.vars(&mut rv);
    if op == "=" && let Term::Var(x) = l && !bound.contains(x) {
        return rv.is_subset(bound).then(|| BTreeSet::from([x.clone()]));
    }
    let mut all = rv.clone();
    l.vars(&mut all);
    if all.is_subset(bound) {
        return Some(BTreeSet::new());
    }
    if op == "=" && matches!(l, Term::Var(_)) && r.is_cat() {
        return Some(rv.difference(bound).cloned().collect());
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) struct Rule {
    pub(in crate::code_rule_dsl) head: Atom,
    pub(in crate::code_rule_dsl) body: Vec<Literal>,
}

/// A relation's lattice column: the last, after `keys` key columns.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) struct LatticeDecl {
    pub(in crate::code_rule_dsl) keys: usize,
    /// The value column's type, which names the lattice (`flat`, `min`, `max`) unless `merge` does.
    pub(in crate::code_rule_dsl) ty: String,
    pub(in crate::code_rule_dsl) merge: Option<String>,
    pub(in crate::code_rule_dsl) widen: Option<u32>,
}

impl LatticeDecl {
    /// The lattice's name for `logic::native` (`declare`d when `merge` names a native join).
    pub(in crate::code_rule_dsl) fn lattice(&self) -> &str {
        self.merge.as_deref().unwrap_or(&self.ty)
    }

    fn bounded(&self) -> bool {
        self.widen.is_some() || (self.merge.is_none() && self.ty == "flat")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) enum Value {
    Int(i64),
    Str(String),
}

#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct Program {
    /// Each relation's columns: (name, type).
    pub(in crate::code_rule_dsl) decls: BTreeMap<String, Vec<(String, String)>>,
    pub(in crate::code_rule_dsl) lattice: BTreeMap<String, LatticeDecl>,
    pub(in crate::code_rule_dsl) facts: BTreeMap<String, Vec<Vec<Value>>>,
    pub(in crate::code_rule_dsl) rules: Vec<Rule>,
    pub(in crate::code_rule_dsl) inputs: Vec<String>,
    pub(in crate::code_rule_dsl) outputs: Vec<String>,
    pub(in crate::code_rule_dsl) kindsets: BTreeMap<String, Vec<String>>,
    /// `.demand` relations, or `*` for every derived one.
    pub(in crate::code_rule_dsl) demand: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) enum Error {
    Syntax(String),
    /// A relation depends negatively on another in its own component.
    NotStratifiable { message: String, component: BTreeSet<String>, negated: String },
    /// An aggregate reads its own component, and the head is not a lattice the aggregate is monotone in.
    AggregationInRecursion { head: String, aggregated: String },
    /// A lattice of unbounded height with no `widen`.
    UnboundedLattice { relation: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax(m) | Error::NotStratifiable { message: m, .. } => f.write_str(m),
            Error::AggregationInRecursion { head, aggregated } => {
                write!(f, "aggregation in recursion: {head} aggregates {aggregated} in its own component")
            }
            Error::UnboundedLattice { relation } => {
                write!(f, "unbounded lattice: {relation} has no finite height and no `widen k`")
            }
        }
    }
}

struct Parser<'s> {
    src: &'s str,
    toks: Vec<(Tok, usize)>,
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> &Tok {
        self.peek_at(0)
    }

    fn peek_at(&self, k: usize) -> &Tok {
        self.toks.get(self.i + k).map_or(&Tok::Eof, |t| &t.0)
    }

    fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::P(q) if *q == p)
    }

    fn is_id(&self, id: &str) -> bool {
        matches!(self.peek(), Tok::Id(s) if s == id)
    }

    fn fail<T>(&self, what: &str) -> Result<T, Error> {
        let at = self.toks.get(self.i).map_or(self.src.len(), |t| t.1);
        Err(Error::Syntax(format!("expected {what}, got {:?} near {:?}", self.peek(), near(self.src, at))))
    }

    fn p(&mut self, p: &str) -> Result<(), Error> {
        if self.is_p(p) {
            self.i += 1;
            Ok(())
        } else {
            self.fail(&format!("{p:?}"))
        }
    }

    fn id(&mut self) -> Result<String, Error> {
        match self.peek().clone() {
            Tok::Id(s) => {
                self.i += 1;
                Ok(s)
            }
            _ => self.fail("an identifier"),
        }
    }

    fn var(&mut self) -> Result<String, Error> {
        match self.peek().clone() {
            Tok::Var(s) => {
                self.i += 1;
                Ok(s)
            }
            _ => self.fail("a variable"),
        }
    }

    fn list<T>(&mut self, mut item: impl FnMut(&mut Self) -> Result<T, Error>) -> Result<Vec<T>, Error> {
        let mut out = vec![item(self)?];
        while self.is_p(",") {
            self.i += 1;
            out.push(item(self)?);
        }
        Ok(out)
    }

    fn term(&mut self) -> Result<Term, Error> {
        let t = match self.peek().clone() {
            Tok::Var(v) => Term::Var(v),
            Tok::Str(s) => Term::Str(s),
            Tok::Int(n) => Term::Int(n),
            Tok::Id(s) if s == "_" => Term::Wild,
            Tok::Id(s) if s == "top" => Term::Top,
            Tok::P("@") => {
                self.i += 1;
                let f = self.id()?;
                self.p("(")?;
                let args = self.list(Self::term)?;
                self.p(")")?;
                return Ok(Term::Call(f, args));
            }
            _ => return self.fail("a term"),
        };
        self.i += 1;
        Ok(t)
    }

    fn atom(&mut self, neg: bool) -> Result<Atom, Error> {
        let rel = self.id()?;
        self.p("(")?;
        let args = self.list(Self::term)?;
        self.p(")")?;
        Ok(Atom { rel, args, neg, gated: false })
    }

    /// The source text of a balanced `{ … }`, braces included.
    fn braces(&mut self) -> Result<String, Error> {
        let start = self.toks.get(self.i).map(|t| t.1);
        self.p("{")?;
        let mut depth = 1;
        while depth > 0 {
            match self.peek() {
                Tok::P("{") => depth += 1,
                Tok::P("}") => depth -= 1,
                Tok::Eof => return self.fail("a closing brace"),
                _ => {}
            }
            self.i += 1;
        }
        let end = self.toks[self.i - 1].1 + 1;
        Ok(self.src[start.expect("an opening brace")..end].to_owned())
    }

    fn literal(&mut self) -> Result<Literal, Error> {
        if self.is_p("!") {
            self.i += 1;
            return Ok(Literal::Atom(self.atom(true)?));
        }
        if self.is_id("nearest") && matches!(self.peek_at(1), Tok::P("(")) {
            self.i += 2;
            let n = self.var()?;
            self.p(",")?;
            let a = self.var()?;
            self.p(",")?;
            let kindset = self.id()?;
            self.p(")")?;
            return Ok(Literal::Nearest { n, a, kindset });
        }
        if self.is_id("ast") && matches!(self.peek_at(1), Tok::P("(")) {
            self.i += 2;
            let document = self.var()?;
            self.p(",")?;
            let node = self.var()?;
            self.p(",")?;
            let matcher = self.braces()?;
            self.p(")")?;
            return Ok(Literal::Ast { document, node, matcher });
        }
        if matches!(self.peek(), Tok::Id(_)) {
            return Ok(Literal::Atom(self.atom(false)?));
        }
        let l = self.term()?;
        let op = match self.peek() {
            Tok::P(p @ ("=" | "!=" | "<" | "<=" | ">" | ">=")) => *p,
            _ => return self.fail("a comparison"),
        };
        self.i += 1;
        if op == "=" && let Term::Var(var) = &l {
            let agg = match self.peek() {
                Tok::Id(s) if s == "count" => Some(AggOp::Count),
                Tok::Id(s) if s == "min" => Some(AggOp::Min),
                Tok::Id(s) if s == "max" => Some(AggOp::Max),
                _ => None,
            };
            if let Some(op) = agg.filter(|_| matches!(self.peek_at(1), Tok::P("{") | Tok::Var(_) | Tok::P(":"))) {
                self.i += 1;
                let over = if let Tok::Var(_) = self.peek() { Some(self.var()?) } else { None };
                if op != AggOp::Count && over.is_none() {
                    return self.fail("the aggregated variable");
                }
                if self.is_p(":") {
                    self.i += 1;
                }
                self.p("{")?;
                let atom = self.atom(false)?;
                self.p("}")?;
                return Ok(Literal::Agg { var: var.clone(), op, over, atom });
            }
        }
        Ok(Literal::Cmp { l, op, r: self.term()? })
    }

    fn directive(&mut self, prog: &mut Program) -> Result<(), Error> {
        let d = self.id()?;
        match d.as_str() {
            "decl" | "lattice" => {
                let name = self.id()?;
                self.p("(")?;
                let (mut cols, mut keys) = (Vec::new(), None);
                loop {
                    let c = self.id()?;
                    self.p(":")?;
                    cols.push((c, self.id()?));
                    if self.is_p(";") {
                        self.i += 1;
                        keys = Some(cols.len());
                    } else if self.is_p(",") {
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                self.p(")")?;
                let (mut merge, mut widen) = (None, None);
                if self.is_id("merge") {
                    self.i += 1;
                    self.p("@")?;
                    merge = Some(self.id()?);
                }
                if self.is_id("widen") {
                    self.i += 1;
                    widen = Some(match self.peek() {
                        Tok::Int(k) if *k >= 0 => u32::try_from(*k).unwrap_or(u32::MAX),
                        _ => return self.fail("a widening count"),
                    });
                    self.i += 1;
                }
                // T: `.lattice` makes the last column the lattice; `.decl` with `;` (or `merge`) does here too.
                if d == "lattice" || keys.is_some() || merge.is_some() {
                    let ty = cols.last().map(|c| c.1.clone()).unwrap_or_default();
                    prog.lattice.insert(name.clone(), LatticeDecl { keys: cols.len() - 1, ty, merge, widen });
                }
                prog.decls.insert(name, cols);
            }
            "input" | "output" => {
                let names = self.list(Self::id)?;
                if d == "input" { &mut prog.inputs } else { &mut prog.outputs }.extend(names);
            }
            "demand" => {
                if self.is_p("*") {
                    self.i += 1;
                    prog.demand.push("*".to_owned());
                } else {
                    let names = self.list(Self::id)?;
                    prog.demand.extend(names);
                }
            }
            "kindset" => {
                let name = self.id()?;
                self.p("=")?;
                let mut kinds = Vec::new();
                loop {
                    match self.peek().clone() {
                        Tok::Str(s) => kinds.push(s),
                        _ => return self.fail("a kind"),
                    }
                    self.i += 1;
                    if !self.is_p("|") {
                        break;
                    }
                    self.i += 1;
                }
                prog.kindsets.insert(name, kinds);
            }
            _ => return self.fail("a directive"),
        }
        Ok(())
    }
}

pub(in crate::code_rule_dsl) fn parse(src: &str) -> Result<Program, Error> {
    let mut p = Parser { src, toks: tokens(src)?, i: 0 };
    let mut prog = Program::default();
    while *p.peek() != Tok::Eof {
        if p.is_p(".") {
            p.i += 1;
            p.directive(&mut prog)?;
            continue;
        }
        let head = p.atom(false)?;
        if p.is_p(".") {
            p.i += 1;
            let row = head
                .args
                .iter()
                .map(|a| match a {
                    Term::Int(n) => Ok(Value::Int(*n)),
                    Term::Str(s) => Ok(Value::Str(s.clone())),
                    other => Err(Error::Syntax(format!("fact {} has a non-constant {other:?}", head.rel))),
                })
                .collect::<Result<_, _>>()?;
            prog.facts.entry(head.rel).or_default().push(row);
            continue;
        }
        p.p(":-")?;
        let body = p.list(Parser::literal)?;
        p.p(".")?;
        // A computed term is a value, not a pattern: an atom never matches on one (the backends would drop it).
        if let Some(a) = body.iter().filter_map(|l| if let Literal::Atom(a) = l { Some(a) } else { None })
            .find(|a| a.args.iter().any(|t| matches!(t, Term::Call(..))))
        {
            return Err(Error::Syntax(format!(
                "computed term in body atom {} of the rule for {}: bind it with `$v = @f(...)`",
                a.rel, head.rel
            )));
        }
        prog.rules.push(Rule { head, body });
    }
    for (relation, decl) in &prog.lattice {
        if !decl.bounded() {
            return Err(Error::UnboundedLattice { relation: relation.clone() });
        }
    }
    Ok(prog)
}

/// Dependency edges: head → {(dependency, negative)}; an aggregate's is negative, a gated negation's is not.
pub(in crate::code_rule_dsl) type Edges = BTreeMap<String, BTreeSet<(String, bool)>>;

/// The program's components, dependencies first, and its dependency edges. T's `strata`, with aggregation in
/// recursion named apart from negation, and allowed into a lattice head it is monotone in (`count`/`max` into a
/// `max` lattice, `min` into a `min` lattice).
pub(in crate::code_rule_dsl) fn strata(prog: &Program) -> Result<(Vec<BTreeSet<String>>, Edges), Error> {
    let derived: BTreeSet<String> = prog.rules.iter().map(|r| r.head.rel.clone()).collect();
    let mut edges = Edges::new();
    let mut aggs = Vec::new();
    for r in &prog.rules {
        for l in &r.body {
            let Some(a) = l.atom() else { continue };
            if !derived.contains(&a.rel) {
                continue;
            }
            let agg = matches!(l, Literal::Agg { .. });
            let monotone = match l {
                Literal::Agg { op, .. } => prog.lattice.get(&r.head.rel).is_some_and(|d| {
                    matches!((op, d.lattice()), (AggOp::Count | AggOp::Max, "max") | (AggOp::Min, "min"))
                }),
                _ => false,
            };
            if agg {
                aggs.push((r.head.rel.clone(), a.rel.clone(), monotone));
            }
            edges.entry(r.head.rel.clone()).or_default().insert((a.rel.clone(), (a.neg && !a.gated) || (agg && !monotone)));
        }
    }
    let sccs = tarjan(&derived, &edges);
    for scc in &sccs {
        if let Some((head, aggregated, _)) = aggs.iter().find(|(h, a, m)| !m && scc.contains(h) && scc.contains(a)) {
            return Err(Error::AggregationInRecursion { head: head.clone(), aggregated: aggregated.clone() });
        }
        for v in scc {
            for (w, neg) in edges.get(v).into_iter().flatten() {
                if *neg && scc.contains(w) {
                    return Err(Error::NotStratifiable {
                        message: format!("not stratifiable: {v} depends negatively on {w} in its own component"),
                        component: scc.clone(),
                        negated: w.clone(),
                    });
                }
            }
        }
    }
    Ok((sccs, edges))
}

/// Strongly connected components of `nodes` under `edges`, dependencies first, in the same order every run.
/// Iterative, where T's recursed (with a raised recursion limit).
pub(in crate::code_rule_dsl) fn tarjan(nodes: &BTreeSet<String>, edges: &Edges) -> Vec<BTreeSet<String>> {
    let empty = BTreeSet::new();
    let succ = |v: &str| -> Vec<&String> {
        edges.get(v).unwrap_or(&empty).iter().map(|(w, _)| w).filter(|w| nodes.contains(*w)).collect()
    };
    let (mut index, mut low) = (BTreeMap::<&str, usize>::new(), BTreeMap::<&str, usize>::new());
    let (mut on, mut stack, mut sccs, mut counter) = (BTreeSet::<&str>::new(), Vec::<&str>::new(), Vec::new(), 0);
    for root in nodes {
        if index.contains_key(root.as_str()) {
            continue;
        }
        // Frames: a node and the position of its next successor.
        let mut frames: Vec<(&str, Vec<&String>, usize)> = vec![(root, succ(root), 0)];
        index.insert(root, counter);
        low.insert(root, counter);
        counter += 1;
        stack.push(root);
        on.insert(root);
        while let Some((v, ws, at)) = frames.last_mut() {
            let v = *v;
            if let Some(&w) = ws.get(*at) {
                *at += 1;
                if !index.contains_key(w.as_str()) {
                    index.insert(w, counter);
                    low.insert(w, counter);
                    counter += 1;
                    stack.push(w);
                    on.insert(w);
                    frames.push((w, succ(w), 0));
                } else if on.contains(w.as_str()) {
                    let lw = index[w.as_str()];
                    let lv = low.get_mut(v).unwrap();
                    *lv = (*lv).min(lw);
                }
                continue;
            }
            frames.pop();
            if let Some((parent, _, _)) = frames.last() {
                let lv = low[v];
                let lp = low.get_mut(parent).unwrap();
                *lp = (*lp).min(lv);
            }
            if low[v] == index[v] {
                let mut scc = BTreeSet::new();
                loop {
                    let w = stack.pop().unwrap();
                    on.remove(w);
                    scc.insert(w.to_owned());
                    if w == v {
                        break;
                    }
                }
                sccs.push(scc);
            }
        }
    }
    sccs
}

/// Drops the rules of relations no output reads, directly or through other rules. T's `prune`.
pub(in crate::code_rule_dsl) fn prune(mut prog: Program) -> Program {
    let mut body_of: BTreeMap<&str, Vec<&Vec<Literal>>> = BTreeMap::new();
    for r in &prog.rules {
        body_of.entry(&r.head.rel).or_default().push(&r.body);
    }
    let (mut need, mut todo) = (BTreeSet::new(), prog.outputs.clone());
    while let Some(r) = todo.pop() {
        if !need.insert(r.clone()) {
            continue;
        }
        for body in body_of.get(r.as_str()).into_iter().flatten() {
            todo.extend(body.iter().filter_map(Literal::atom).map(|a| a.rel.clone()));
        }
    }
    prog.rules.retain(|r| need.contains(&r.head.rel));
    prog
}

#[cfg(test)]
mod tests;
