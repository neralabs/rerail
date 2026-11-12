//! The logic's text syntax: tokens, the AST, stratification and pruning (03 §12; 05 Phase 6 row 2).
//!
//! A port of the front end of Typing-support's `rdl.py` (T, `front/rdl.py` beside this file until the port is
//! done): `tokens`, `parse`, the AST classes, `strata`/`tarjan` and `prune`, with T's grammar. Added for §12:
//! - lattices declared on any relation, `.decl r(k: t; v: L) merge @join widen k`, besides T's `.lattice`;
//! - `min` and `max` aggregates, `$m = min $v : { atom }`, besides `count`;
//! - the matcher atom `ast($D, $P, { … })`, whose matcher text is kept for the transducer lowering (§3), and which
//!   exports matcher metavariables as variables, `ast($D, $P, { … }, NAME: $n)`, each bound to what it captured (a
//!   node, or the text of a regex group or a transform); a match that leaves one unbound gives no row;
//! - JSON strings, and `.policy { … }` for lint-rule documents (`yaml`, the second syntax of B12);
//! - named errors for aggregation in recursion (unless the head is a monotone lattice) and unbounded lattices;
//! - unbounded value cycles (`cycles`, O11 option b): a recursion that creates values must be bounded by the data.
//!
//! §12's other named errors are not here: a non-interned cross-document join needs column types, and the cost gate
//! is the planner's (row 5).
//!
//! Tree structure is never a relation: `nearest($n, $s, set)` is a built-in over the tape (T's `rdl.py` docstring).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub mod caches;
pub use rerail_rr_syntax::code_rule_dsl::logic::front::name;
pub use name::Name;
pub mod cycles;
pub use rerail_rr_syntax::code_rule_dsl::logic::front::ast;
pub use ast::{AXIS, Again, AggOp, Atom, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, Literal, NEAREST, Native, OPTION_GLOB, Program, QUALIFIERS, RESERVED, Rule, SCOPE_NOT, SpanIn, Term, Value, axis, cmp_binds, globals, is_axis, is_transduced, join};

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
        // An `#include` (a `.rr` program's, passed through by `rr::translate`) is a directive the files it names
        // resolve: the text parse reads past it, as `rr::parse` does.
        if rest.starts_with("//") || rest.starts_with("#include") {
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
            // A JSON string (T's had no escapes): `Term::Str` prints as one, and matcher text embeds JSON.
            let mut end = pos + 1;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            if end >= bytes.len() {
                return Err(Error::Syntax(format!("unterminated string at {:?}", near(src, pos))));
            }
            let value = serde_json::from_str(&src[pos..=end]).map_err(|e| Error::Syntax(format!("bad string at {:?}: {e}", near(src, pos))))?;
            out.push((Tok::Str(value), start));
            pos = end + 1;
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

/// The parser's own spelling of the operator `op`, if it is one (a comparison read back from facts or the store).
pub fn operator(op: &str) -> Option<&'static str> {
    PUNCT.iter().copied().find(|p| *p == op)
}

fn near(src: &str, pos: usize) -> &str {
    let end = (pos..=src.len().min(pos + 40)).rev().find(|&i| src.is_char_boundary(i)).unwrap_or(pos);
    &src[pos..end]
}

// T's `repr`s, which its transformed programs are compared in.
/// `scopes` with each option entry's values (`OPTION_GLOB`) added beside it, from the run's `options` as
/// `option`/`option_item` read them: a directory under `dirs`, a glob under `include` or `exclude`. An option the run
/// does not set adds nothing, and its entry still holds no path (an entry whose only globs are such reads none).
pub fn resolve_scopes(scopes: &Option<Vec<(Vec<String>, Vec<String>)>>, options: &serde_json::Value) -> Option<Vec<(Vec<String>, Vec<String>)>> {
    let mut rows = crate::code_rule_dsl::facts::repository::Rows::default();
    crate::code_rule_dsl::facts::repository::options(options, &mut rows);
    let values = |name: &str| -> Vec<String> {
        rows.option_item.iter().filter(|(n, _, _)| n == name).map(|(_, _, v)| v.clone()).chain(rows.option.iter().filter(|(n, _)| n == name).map(|(_, v)| v.clone())).collect()
    };
    let resolve = |globs: &Vec<String>| -> Vec<String> {
        let mut out = Vec::new();
        for g in globs {
            out.push(g.clone());
            let Some((form, name)) = g.strip_prefix(OPTION_GLOB).and_then(|r| r.split_once(':')) else { continue };
            for v in values(name) {
                out.push(if form == "dirs" { format!("{}/**", v.trim_end_matches('/')) } else { v });
            }
        }
        out
    };
    scopes.as_ref().map(|entries| entries.iter().map(|(within, out)| (resolve(within), resolve(out))).collect())
}

pub fn in_doc_scopes(scopes: &Option<Vec<(Vec<String>, Vec<String>)>>, path: &str) -> bool {
    let glob = |g: &String| crate::code_rule_dsl::logic::strings::test("glob", path, g);
    scopes.as_ref().is_none_or(|entries| entries.iter().any(|(within, out)| (within.is_empty() || within.iter().any(glob)) && !out.iter().any(glob)))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Syntax(String),
    /// A relation depends negatively on another in its own component.
    NotStratifiable { message: String, component: BTreeSet<String>, negated: String },
    /// An aggregate reads its own component, and the head is not a lattice the aggregate is monotone in.
    AggregationInRecursion { head: String, aggregated: String, component: BTreeSet<String> },
    /// A lattice of unbounded height with no `widen`.
    UnboundedLattice { relation: String },
    /// A demanded relation is used with its key unbound: its demand would be everything (magic sets).
    UnboundDemand { message: String, relation: String },
    /// A component creates values (`@cat`, …) the data does not bound (`cycles`): `rule` is one it could not
    /// bound.
    UnboundedValueCycle { component: BTreeSet<String>, rule: String },
    /// `.input r` where no `.decl` gives `r`'s columns (nor core.rr's, for `.rr`).
    UndeclaredInput { relation: String },
}

/// The `.rr` parser's error (`rerail_rr_syntax`) is a syntax error.
impl From<rerail_rr_syntax::Error> for Error {
    fn from(e: rerail_rr_syntax::Error) -> Self {
        match e {
            rerail_rr_syntax::Error::Syntax(m) => Error::Syntax(m),
        }
    }
}

impl Error {
    /// The component and relation of a stratification failure (a negation or an aggregation inside its own
    /// component): T raised `NotStratifiable` for both, and magic sets recovers from either the same way.
    pub(in crate::code_rule_dsl) fn unstratified(&self) -> Option<(&BTreeSet<String>, &str)> {
        match self {
            Error::NotStratifiable { component, negated, .. } => Some((component, negated)),
            Error::AggregationInRecursion { component, aggregated, .. } => Some((component, aggregated)),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax(m) | Error::NotStratifiable { message: m, .. } | Error::UnboundDemand { message: m, .. } => {
                f.write_str(m)
            }
            Error::AggregationInRecursion { head, aggregated, .. } => {
                write!(f, "aggregation in recursion: {head} aggregates {aggregated} in its own component")
            }
            Error::UnboundedLattice { relation } => {
                write!(f, "unbounded lattice: {relation} has no finite height and no `widen k`")
            }
            Error::UnboundedValueCycle { component, rule } => {
                let component = component.iter().cloned().collect::<Vec<_>>().join(", ");
                write!(f, "unbounded value cycle: {{{component}}} creates values the data does not bound ({rule})")
            }
            Error::UndeclaredInput { relation } => write!(f, ".input {relation}: no .decl declares {relation}'s columns"),
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
            // A pattern set (decision 0037): `{"a*", "b"}` is `@set("a*", "b")`.
            Tok::P("{") => {
                self.i += 1;
                let items = if self.is_p("}") { Vec::new() } else { self.list(Self::term)? };
                self.p("}")?;
                return Ok(Term::Call(crate::code_rule_dsl::logic::strings::SET.to_owned(), items));
            }
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
        // Tree axes (decision 0036): `inside($n, $a, set)`, $a a strict ancestor of $n whose kind is in the set, the
        // input relation `rr_inside_<set>(n, a)`; and `has($a, $n, set)`, $n a strict descendant of $a whose kind is
        // in the set, `rr_has_<set>(n, a)`. The transducer makes both (`Lowered::transduce_nearest`).
        if (rel == "inside" || rel == "has") && matches!(self.peek_at(4), Tok::Id(_)) {
            let x = self.term()?;
            self.p(",")?;
            let y = self.term()?;
            self.p(",")?;
            let set = self.id()?;
            self.p(")")?;
            let (axis, args) = if rel == "inside" { (AXIS, vec![x, y]) } else { (DESCENDANTS, vec![y, x]) };
            return Ok(Atom { rel: format!("{axis}{set}").into(), args, neg, gated: false });
        }
        let args = self.list(Self::term)?;
        self.p(")")?;
        Ok(Atom { rel: rel.into(), args, neg, gated: false })
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
            // A failing text test (decision 0037): `!@prefix($s, "_")` is `@prefix($s, "_") = 0`.
            if self.is_p("@") {
                let l = self.term()?;
                if !matches!(&l, Term::Call(f, _) if crate::code_rule_dsl::logic::strings::is_test(f)) {
                    return self.fail("a text test after !");
                }
                return Ok(Literal::Cmp { l, op: "=", r: Term::Int(0) });
            }
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
            let mut captures = Vec::new();
            while self.is_p(",") {
                self.i += 1;
                let name = self.id()?;
                self.p(":")?;
                captures.push((name, self.var()?));
            }
            self.p(")")?;
            return Ok(Literal::Ast { document, node, matcher, captures });
        }
        if matches!(self.peek(), Tok::Id(_)) {
            return Ok(Literal::Atom(self.atom(false)?));
        }
        let l = self.term()?;
        // `$s in {…}`: one of a set (decision 0037), `@equals($s, {…})`.
        if self.is_id("in") {
            self.i += 1;
            let set = self.term()?;
            return Ok(Literal::Cmp { l: Term::Call("equals".to_owned(), vec![l, set]), op: "=", r: Term::Int(1) });
        }
        let op = match self.peek() {
            Tok::P(p @ ("=" | "!=" | "<" | "<=" | ">" | ">=")) => *p,
            // A literal test on its own (`logic::strings`): `@prefix($s, "_")` is `@prefix($s, "_") = 1`.
            _ if matches!(&l, Term::Call(f, _) if crate::code_rule_dsl::logic::strings::is_test(f)) => return Ok(Literal::Cmp { l, op: "=", r: Term::Int(1) }),
            _ => return self.fail("a comparison"),
        };
        self.i += 1;
        if op == "=" && let Term::Var(var) = &l {
            let agg = match self.peek() {
                Tok::Id(s) if s == "count" => Some(AggOp::Count),
                Tok::Id(s) if s == "min" => Some(AggOp::Min),
                Tok::Id(s) if s == "max" => Some(AggOp::Max),
                Tok::Id(s) if s == "sum" => Some(AggOp::Sum),
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
            // `.pragma "cross-rounds" "k"`, as `.rr` writes it (decision 0056).
            "pragma" => {
                let (Tok::Str(key), Tok::Str(k)) = (self.peek().clone(), self.peek_at(1).clone()) else {
                    return self.fail("a pragma's two strings");
                };
                self.i += 2;
                if key != PRAGMA || k.parse::<usize>().is_err() {
                    self.i -= 2;
                    return self.fail("`.pragma \"cross-rounds\" \"k\"`");
                } else {
                    prog.settings.push(setting(&k));
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
            "policy" => {
                let text = self.braces()?;
                let policy = serde_json::from_str(&text).map_err(|e| Error::Syntax(format!("bad .policy document: {e}")))?;
                prog.policies.push(policy);
            }
            _ => return self.fail("a directive"),
        }
        Ok(())
    }
}

pub fn parse(src: &str) -> Result<Program, Error> {
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
            prog.facts.entry(head.rel.to_string()).or_default().push(row);
            continue;
        }
        p.p(":-")?;
        let body = p.list(Parser::literal)?;
        p.p(".")?;
        prog.rules.push(Rule { head, body });
    }
    finish(prog)
}

// A test of a transform written for preorder ids (T's goldens) parses programs with their node comparisons as written,
// on its own thread.
#[cfg(any(test, feature = "test-hooks"))]
thread_local! {pub static KEEP_NODE_COMPARISONS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };}

/// The relation of each node's preorder key (`syntax`'s `node_order(n, k)`).
const NODE_ORDER: &str = "node_order";

/// Rules that order nodes, under node ids per definition (decision 0094 §2). An id follows preorder only inside its
/// granule, so a comparison of two nodes (`b < n`: a binding before a use) and a `min`/`max` over nodes read the
/// nodes' preorder keys instead (`node_order`): the order preorder ids gave, so the rule means what it did.
/// - `x < y`, both nodes: `node_order(x, kx), node_order(y, ky), kx < ky` (`<=`, `>`, `>=` likewise);
/// - `v = max x : { r(…x…) }`, `x` a node: `kv = max k : { r_ord(…x…, k) }, node_order(v, kv)`, where
///   `r_ord(…x…, k) :- r(…x…), node_order(x, k)`.
///
/// A variable is a node when a column it fills is declared `node`: in the head or any atom of the body.
fn order_nodes(prog: &mut Program) {
    if !crate::code_rule_dsl::facts::vtab::syntax::definition_ids() {
        return;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    if KEEP_NODE_COMPARISONS.get() {
        return;
    }
    let decls = prog.decls.clone();
    let typed = |a: &Atom, nodes: &mut BTreeSet<String>| {
        for (t, (_, ty)) in a.args.iter().zip(decls.get(a.rel.as_str()).map_or(&[][..], Vec::as_slice)) {
            if let Term::Var(v) = t
                && ty == "node"
            {
                nodes.insert(v.clone());
            }
        }
    };
    let order = |n: &str, k: &str| Literal::Atom(Atom { rel: NODE_ORDER.to_owned().into(), args: vec![Term::Var(n.to_owned()), Term::Var(k.to_owned())], neg: false, gated: false });
    let mut used = false;
    let mut helpers: Vec<(Rule, Vec<(String, String)>)> = Vec::new();
    let mut fresh = 0usize;
    for r in &mut prog.rules {
        let mut nodes = BTreeSet::new();
        typed(&r.head, &mut nodes);
        for l in &r.body {
            match l {
                Literal::Atom(a) => typed(a, &mut nodes),
                Literal::Agg { atom, .. } => typed(atom, &mut nodes),
                Literal::Nearest { n, a, .. } => {
                    nodes.insert(n.clone());
                    nodes.insert(a.clone());
                }
                _ => {}
            }
        }
        // A min or max over nodes is a node.
        for l in &r.body {
            if let Literal::Agg { var, op: AggOp::Min | AggOp::Max, over: Some(o), atom } = l {
                let mut inner = BTreeSet::new();
                typed(atom, &mut inner);
                if inner.contains(o) {
                    nodes.insert(var.clone());
                }
            }
        }
        if nodes.is_empty() {
            continue;
        }
        let mut body = Vec::with_capacity(r.body.len());
        for l in std::mem::take(&mut r.body) {
            match l {
                Literal::Cmp { l: Term::Var(x), op, r: Term::Var(y) } if matches!(op, "<" | "<=" | ">" | ">=") && nodes.contains(&x) && nodes.contains(&y) => {
                    let (kx, ky) = (format!("rr_ord{fresh}"), format!("rr_ord{}", fresh + 1));
                    fresh += 2;
                    body.extend([order(&x, &kx), order(&y, &ky), Literal::Cmp { l: Term::Var(kx), op, r: Term::Var(ky) }]);
                    used = true;
                }
                Literal::Agg { var, op: op @ (AggOp::Min | AggOp::Max), over: Some(o), atom } if {
                    let mut inner = BTreeSet::new();
                    typed(&atom, &mut inner);
                    inner.contains(&o)
                } =>
                {
                    // The rows aggregated, each with its node's key.
                    let cols = decls.get(atom.rel.as_str()).cloned().unwrap_or_default();
                    let mut head_args: Vec<Term> = Vec::new();
                    let mut head_cols: Vec<(String, String)> = Vec::new();
                    let mut agg_args: Vec<Term> = Vec::new();
                    for (i, t) in atom.args.iter().enumerate() {
                        // The node aggregated is the helper's own: only its key goes out.
                        if let Term::Var(v) = t
                            && *v != o
                            && !head_args.contains(t)
                        {
                            head_args.push(t.clone());
                            head_cols.push((format!("c{i}"), cols.get(i).map_or_else(|| "int".to_owned(), |(_, ty)| ty.clone())));
                            agg_args.push(Term::Var(v.clone()));
                        }
                    }
                    let k = format!("rr_ord{fresh}");
                    let kv = format!("rr_ord{}", fresh + 1);
                    fresh += 2;
                    let name = format!("rr_ordered{}", helpers.len());
                    head_args.push(Term::Var(k.clone()));
                    agg_args.push(Term::Var(k.clone()));
                    head_cols.push(("k".to_owned(), "int".to_owned()));
                    helpers.push((Rule { head: Atom { rel: name.clone().into(), args: head_args, neg: false, gated: false }, body: vec![Literal::Atom(atom), order(&o, &k)] }, head_cols));
                    body.push(Literal::Agg { var: kv.clone(), op, over: Some(k), atom: Atom { rel: name.into(), args: agg_args, neg: false, gated: false } });
                    body.push(order(&var, &kv));
                    used = true;
                }
                other => body.push(other),
            }
        }
        r.body = body;
    }
    for (rule, cols) in helpers {
        prog.decls.insert(rule.head.rel.to_string(), cols);
        prog.rules.push(rule);
    }
    if used {
        prog.decls.entry(NODE_ORDER.to_owned()).or_insert_with(|| vec![("n".to_owned(), "node".to_owned()), ("k".to_owned(), "int".to_owned())]);
        if !prog.inputs.iter().any(|i| i == NODE_ORDER) {
            prog.inputs.push(NODE_ORDER.to_owned());
        }
    }
}

/// The checks and additions every program gets once parsed, whatever its syntax (this one or `.rr`, `logic::rr`):
/// no computed term in a body atom, bounded lattices, the tree axes' input relations, bounded value cycles.
pub(in crate::code_rule_dsl) fn finish(mut prog: Program) -> Result<Program, Error> {
    order_nodes(&mut prog);
    for rule in &prog.rules {
        // A computed term is a value, not a pattern: an atom never matches on one (the backends would drop it).
        if let Some(a) = rule.body.iter().filter_map(|l| if let Literal::Atom(a) = l { Some(a) } else { None })
            .find(|a| a.args.iter().any(|t| matches!(t, Term::Call(..))))
        {
            return Err(Error::Syntax(format!(
                "computed term in body atom {} of the rule for {}: bind it with `v = @f(...)`",
                a.rel, rule.head.rel
            )));
        }
    }
    for (relation, decl) in &prog.lattice {
        if !decl.bounded() {
            return Err(Error::UnboundedLattice { relation: relation.clone() });
        }
    }
    // The tree axes' relations are inputs, made by the transducer.
    let axes: Vec<String> = prog.rules.iter().flat_map(|r| r.body.iter().filter_map(Literal::atom)).map(|a| a.rel.to_string()).filter(|r| is_axis(r)).collect();
    for rel in axes {
        if !prog.decls.contains_key(&rel) {
            prog.decls.insert(rel.clone(), vec![("n".to_owned(), "node".to_owned()), ("a".to_owned(), "node".to_owned())]);
            prog.inputs.push(rel);
        }
    }
    // An input's columns are its declaration's: the planner reads them.
    if let Some(relation) = prog.inputs.iter().find(|rel| !prog.decls.contains_key(*rel)) {
        return Err(Error::UndeclaredInput { relation: relation.clone() });
    }
    cycles::check(&prog)?;
    // A cache is two rules (decision 0103): a broken form refuses the program.
    caches::caches(&prog)?;
    Ok(prog)
}

/// A program to run reads only relations it declares or the transducer makes: else an error, as in Soufflé, not a
/// plan's panic. (Parsing alone does not ask it: a parse's tests read relations they never declare.) `match_all` and
/// `match_group` over a span are built-ins: the text columns make each regex's relation (`strings`).
pub fn declared(prog: &Program) -> Result<(), Error> {
    for rule in &prog.rules {
        let mut read = rule.body.iter().filter_map(|l| match l {
            Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a),
            _ => None,
        });
        // The import components' relations (`rr_cx_reach`, decision 0076) are added by the run to a program that reads
        // them: declared there.
        if let Some(a) = read.find(|a| {
            !prog.decls.contains_key(a.rel.as_str()) && !is_transduced(&a.rel) && !a.rel.starts_with("rr_cx_") && !matches!(a.rel.as_str(), "match_all" | "match_group")
        }) {
            return Err(Error::Syntax(format!("relation {} is read by the rule for {} and not declared: declare it with `.decl`", a.rel, rule.head.rel)));
        }
    }
    Ok(())
}

/// Dependency edges: head → {(dependency, negative)}; an aggregate's is negative, a gated negation's is not.
pub type Edges = BTreeMap<String, BTreeSet<(String, bool)>>;

/// The program's components, dependencies first, and its dependency edges. T's `strata`, with aggregation in
/// recursion named apart from negation, and allowed into a lattice head it is monotone in (`count`/`max` into a
/// `max` lattice, `min` into a `min` lattice).
/// Each stratum's rules (`sccs` from `strata`), in program order, found in one pass over the rules: a filter of every
/// rule per stratum costs the rules times the strata (16 k by 10 k on the Python pack, seconds of a one-file run).
pub fn rules_by_stratum<'a>(prog: &'a Program, sccs: &[BTreeSet<String>]) -> Vec<Vec<&'a Rule>> {
    let at: std::collections::HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.as_str(), i))).collect();
    let mut out: Vec<Vec<&Rule>> = vec![Vec::new(); sccs.len()];
    for r in &prog.rules {
        if let Some(&i) = at.get(r.head.rel.as_str()) {
            out[i].push(r);
        }
    }
    out
}

pub fn strata(prog: &Program) -> Result<(Vec<BTreeSet<String>>, Edges), Error> {
    let (sccs, edges, aggs) = components(prog);
    match first_unstratified(&sccs, &edges, &aggs, true).into_iter().next() {
        Some(e) => Err(e),
        None => Ok((sccs, edges)),
    }
}

/// Every component that a negation or a non-monotone aggregate closes a cycle in, one error each (`strata` stops at
/// the first): a caller that drops what made each cycle (demand's split, decision 0048) drops them all in one round.
pub fn unstratified(prog: &Program) -> Vec<Error> {
    let (sccs, edges, aggs) = components(prog);
    first_unstratified(&sccs, &edges, &aggs, false)
}

/// The first error of each component a negative edge closes a cycle in (`first`: only the first component's).
fn first_unstratified(sccs: &[BTreeSet<String>], edges: &Edges, aggs: &BTreeSet<(String, String)>, first: bool) -> Vec<Error> {
    let mut out = Vec::new();
    for scc in sccs {
        'scc: for v in scc {
            for (w, neg) in edges.get(v).into_iter().flatten() {
                if !*neg || !scc.contains(w) {
                    continue;
                }
                out.push(if aggs.contains(&(v.clone(), w.clone())) {
                    Error::AggregationInRecursion { head: v.clone(), aggregated: w.clone(), component: scc.clone() }
                } else {
                    Error::NotStratifiable {
                        message: format!("not stratifiable: {v} depends negatively on {w} in its own component"),
                        component: scc.clone(),
                        negated: w.clone(),
                    }
                });
                if first {
                    return out;
                }
                break 'scc;
            }
        }
    }
    out
}

/// The program's components (dependencies first), its dependency edges, and its non-monotone aggregation edges.
fn components(prog: &Program) -> (Vec<BTreeSet<String>>, Edges, BTreeSet<(String, String)>) {
    let mut derived: BTreeSet<String> = prog.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut edges = Edges::new();
    // An answer is an edge: each produced relation reads its request (decision 0065 §2a).
    for (request, produced) in &prog.answers {
        for p in produced {
            derived.insert(p.clone());
            edges.entry(p.clone()).or_default().insert((request.clone(), false));
        }
    }
    for (rel, native) in &prog.natives {
        if !derived.contains(rel) { continue; }
        edges.entry(rel.clone()).or_default().extend(native.reads().into_iter()
            .filter(|read| derived.contains(*read)).map(|read| (read.to_owned(), false)));
    }
    // Aggregation edges that are not monotone: negative, and named apart when they close a cycle.
    let mut aggs = BTreeSet::new();
    for r in &prog.rules {
        for l in &r.body {
            let Some(a) = l.atom() else { continue };
            if !derived.contains(a.rel.as_str()) {
                continue;
            }
            let agg = match l {
                Literal::Agg { op, .. } => !prog.lattice.get(r.head.rel.as_str()).is_some_and(|d| {
                    matches!((op, d.lattice()), (AggOp::Count | AggOp::Max, "max") | (AggOp::Min, "min"))
                }),
                _ => false,
            };
            if agg && !a.gated {
                aggs.insert((r.head.rel.to_string(), a.rel.to_string()));
            }
            edges.entry(r.head.rel.to_string()).or_default().insert((a.rel.to_string(), (a.neg || agg) && !a.gated));
        }
    }
    let sccs = together(&derived, &edges, &prog.together);
    (sccs, edges, aggs)
}

/// The sets of `prog.together` that are not each one component as `strata` makes them (`together`, below).
pub fn apart(prog: &Program) -> Vec<usize> {
    let (sccs, _, _) = components(prog);
    (0..prog.together.len()).filter(|&i| {
        let t = &prog.together[i];
        !sccs.iter().any(|s| s == t)
    }).collect()
}

/// The components, each set of `together` one of them (decision 0101 §3): its relations joined in a ring for the
/// search only, so the edges stay the rules' own. A set that would take in a relation outside it (one read by a
/// member and reading another) is left apart.
fn together(derived: &BTreeSet<String>, edges: &Edges, sets: &[BTreeSet<String>]) -> Vec<BTreeSet<String>> {
    let mut sets: Vec<Vec<&String>> = sets.iter().map(|t| t.iter().filter(|r| derived.contains(*r)).collect::<Vec<_>>()).filter(|t| t.len() > 1).collect();
    loop {
        if sets.is_empty() {
            return tarjan(derived, edges);
        }
        let mut ring = edges.clone();
        for t in &sets {
            for (i, r) in t.iter().enumerate() {
                ring.entry((*r).clone()).or_default().insert((t[(i + 1) % t.len()].clone(), false));
            }
        }
        let sccs = tarjan(derived, &ring);
        let n = sets.len();
        sets.retain(|t| sccs.iter().find(|s| s.contains(t[0])).is_some_and(|s| s.len() == t.len()));
        if sets.len() == n {
            return sccs;
        }
    }
}

/// Strongly connected components of `nodes` under `edges`, dependencies first, in the same order every run.
/// Iterative, where T's recursed (with a raised recursion limit).
pub fn tarjan(nodes: &BTreeSet<String>, edges: &Edges) -> Vec<BTreeSet<String>> {
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

/// The relations `roots` read, directly or through other rules, the roots included (`prune`'s, from the outputs).
pub fn needed(prog: &Program, roots: Vec<String>) -> BTreeSet<String> {
    let mut body_of: BTreeMap<&str, Vec<&Vec<Literal>>> = BTreeMap::new();
    for r in &prog.rules {
        body_of.entry(&r.head.rel).or_default().push(&r.body);
    }
    let (mut need, mut todo) = (BTreeSet::new(), roots);
    while let Some(r) = todo.pop() {
        if !need.insert(r.clone()) {
            continue;
        }
        if let Some(native) = prog.natives.get(&r) {
            todo.extend(native.reads().into_iter().map(str::to_owned));
        }
        for body in body_of.get(r.as_str()).into_iter().flatten() {
            todo.extend(body.iter().filter_map(Literal::atom).map(|a| a.rel.to_string()));
        }
    }
    need
}

/// Drops the rules of relations no output reads, directly or through other rules. T's `prune`.
pub fn prune(mut prog: Program) -> Program {
    let mut body_of: BTreeMap<&str, Vec<&Vec<Literal>>> = BTreeMap::new();
    for r in &prog.rules {
        body_of.entry(&r.head.rel).or_default().push(&r.body);
    }
    let (mut need, mut todo) = (BTreeSet::new(), prog.outputs.clone());
    while let Some(r) = todo.pop() {
        if !need.insert(r.clone()) {
            continue;
        }
        if let Some(native) = prog.natives.get(&r) {
            todo.extend(native.reads().into_iter().map(str::to_owned));
        }
        for body in body_of.get(r.as_str()).into_iter().flatten() {
            todo.extend(body.iter().filter_map(Literal::atom).map(|a| a.rel.to_string()));
        }
    }
    prog.rules.retain(|r| need.contains(r.head.rel.as_str()));
    // Native metadata owns real dependency edges. A dead producer must not keep its demands
    // alive in the file-local split or cause them to be exported across every chunk.
    prog.natives.retain(|rel, _| need.contains(rel));
    prog
}

// cratify: `tests` is rerail-logic's (it mounts it)
#[cfg(test)]
pub mod data_layer_census_tests;
// cratify: `data_layer_tests` is rerail-logic's (it mounts it)
pub mod yaml;

/// The pragma by which a program asks for bounded rounds (decision 0056): `.pragma "cross-rounds" "4"`. A typing
/// program declares it: its cross-file recursion is over interface summaries, cut at `k` file crossings, with the
/// pending round when it also declares `unconverged`. A program that does not declare it (a rule) has its
/// cross-file part run as one fixpoint, which no chain length cuts.
pub const PRAGMA: &str = "cross-rounds";

/// The program setting `.pragma "cross-rounds" "k"` lowers to (from `.rr`, or the `.rdl` copies' same line).
pub(crate) fn setting(k: &str) -> (String, String) {
    ("pragma".to_owned(), format!("\"{PRAGMA}\" \"{k}\""))
}

/// A base relation's document column, including raw text views.
pub fn document_column(relation: &str) -> Option<usize> {
    match relation {
        "node" | "doc" | "pkg" | "path_part" | "file_name" | "syntax_error" | "cpython_syntax_error" |
        "source" | "src_line" | "src_line_lf" | "tape_line" | "split_line" | "lf_line" | "line_count" | "parsed"
        // Decision 0055's: a guest and its host are one file (`corpus::guests` attributes its rows to the host).
        | "doc_role" | "doc_root" | "tape_of" | "guest" | "host_node" => Some(0),
        "docof" => Some(1),
        _ => None,
    }
}

pub use crate::code_rule_dsl::facts::core::PYTHON_CONTEXTS;
