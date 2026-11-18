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
pub use name::{List, Lists, Name, Stacks};
pub mod cycles;
pub mod template;
pub use rerail_rr_syntax::code_rule_dsl::logic::front::ast;
pub use ast::{AXIS, Again, AggOp, Atom, Branded, Class, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, Literal, NEAREST, Native, OPTION_GLOB, Program, QUALIFIERS, RESERVED, Rule, SCOPE_NOT, Shape, ShapeWait, shape_base, SpanIn, Term, Value, axis, cmp_binds, globals, is_axis, is_transduced, join};

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
        // An `#include` (a `.rerail` program's, passed through by `rr::translate`) is a directive the files it names
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
    /// `.input r` where no `.decl` gives `r`'s columns (nor core.rerail's, for `.rerail`).
    UndeclaredInput { relation: String },
}

/// The `.rerail` parser's error (`rerail_rr_syntax`) is a syntax error.
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
            Tok::Var(v) => Term::Var(v.into()),
            Tok::Str(s) => Term::Str(s.into()),
            Tok::Int(n) => Term::Int(n),
            Tok::Id(s) if s == "_" => Term::Wild,
            Tok::Id(s) if s == "top" => Term::Top,
            // A pattern set (decision 0037): `{"a*", "b"}` is `@set("a*", "b")`.
            Tok::P("{") => {
                self.i += 1;
                let items = if self.is_p("}") { Vec::new() } else { self.list(Self::term)? };
                self.p("}")?;
                return Ok(Term::Call(crate::code_rule_dsl::logic::strings::SET.to_owned().into(), items.into()));
            }
            Tok::P("@") => {
                self.i += 1;
                let f = self.id()?;
                self.p("(")?;
                let args = self.list(Self::term)?;
                self.p(")")?;
                return Ok(Term::Call(f.into(), args.into()));
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
            return Ok(Atom { rel: format!("{axis}{set}").into(), args: args.into(), neg, gated: false });
        }
        let args = self.list(Self::term)?;
        self.p(")")?;
        Ok(Atom { rel: rel.into(), args: args.into(), neg, gated: false })
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
            return Ok(Literal::Cmp { l: Term::Call("equals".into(), [l, set].into()), op: "=", r: Term::Int(1) });
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
                return Ok(Literal::Agg { var: var.to_string(), op, over, atom });
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
                prog.decls.insert(name.into(), cols.iter().map(|(c, t)| (c.into(), t.into())).collect());
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
            // `.pragma "cross-rounds" "k"` (decision 0056) is refused: bounded cross-file rounds are retired: a program's cross-file part is one fixpoint (decisions 0065, 0276 §9).
            "pragma" => {
                return self.fail("no `.pragma`: bounded cross-file rounds are retired: a program's cross-file part is one fixpoint (decisions 0065, 0276 §9)");
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
        prog.rules.push(Rule { head, body: body.into() });
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
    // A measurement, never a mode to run in: comparisons of the node ids themselves, which an edit leaves alone outside
    // the granules it touches but which are not in preorder. What a stable order key could save at most.
    if std::env::var_os("RERAIL_ORDER_RAW").is_some() {
        crate::code_rule_dsl::logic::profile::decision("node comparisons (RERAIL_ORDER_RAW)", || "node ids compared as they are, not by node_order: a measurement only".into());
        return;
    }
    let decls = prog.decls.clone();
    let typed = |a: &Atom, nodes: &mut BTreeSet<String>| {
        for (t, (_, ty)) in a.args.iter().zip(decls.get(a.rel.as_str()).map_or(&[][..], |c| c.as_slice())) {
            if let Term::Var(v) = t
                && ty == "node"
            {
                nodes.insert(v.to_string());
            }
        }
    };
    let order = |n: &str, k: &str| Literal::Atom(Atom { rel: NODE_ORDER.to_owned().into(), args: [Term::Var(n.to_owned().into()), Term::Var(k.to_owned().into())].into(), neg: false, gated: false });
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
                Literal::Cmp { l: Term::Var(x), op, r: Term::Var(y) } if matches!(op, "<" | "<=" | ">" | ">=") && nodes.contains(x.as_str()) && nodes.contains(y.as_str()) => {
                    let (kx, ky) = (format!("rr_ord{fresh}"), format!("rr_ord{}", fresh + 1));
                    fresh += 2;
                    body.extend([order(&x, &kx), order(&y, &ky), Literal::Cmp { l: Term::Var(kx.into()), op, r: Term::Var(ky.into()) }]);
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
                            head_cols.push((format!("c{i}"), cols.get(i).map_or_else(|| "int".to_owned(), |(_, ty)| ty.to_string())));
                            agg_args.push(Term::Var(v.clone()));
                        }
                    }
                    let k = format!("rr_ord{fresh}");
                    let kv = format!("rr_ord{}", fresh + 1);
                    fresh += 2;
                    let name = format!("rr_ordered{}", helpers.len());
                    head_args.push(Term::Var(k.clone().into()));
                    agg_args.push(Term::Var(k.clone().into()));
                    head_cols.push(("k".to_owned(), "int".to_owned()));
                    helpers.push((Rule { head: Atom { rel: name.clone().into(), args: head_args.into(), neg: false, gated: false }, body: [Literal::Atom(atom), order(&o, &k)].into() }, head_cols));
                    body.push(Literal::Agg { var: kv.clone(), op, over: Some(k), atom: Atom { rel: name.into(), args: agg_args.into(), neg: false, gated: false } });
                    body.push(order(&var, &kv));
                    used = true;
                }
                other => body.push(other),
            }
        }
        r.body = body.into();
    }
    for (rule, cols) in helpers {
        prog.decls.insert(rule.head.rel.clone(), cols.iter().map(|(c, t)| (c.into(), t.into())).collect());
        prog.rules.push(rule);
    }
    if used {
        prog.decls.entry(NODE_ORDER.to_owned().into()).or_insert_with(|| [("n".into(), "node".into()), ("k".into(), "int".into())].into());
        if !prog.inputs.iter().any(|i| i == NODE_ORDER) {
            prog.inputs.push(NODE_ORDER.to_owned());
        }
    }
}

/// The checks and additions every program gets once parsed, whatever its syntax (this one or `.rerail`, `logic::rr`):
/// no computed term in a body atom, bounded lattices, the tree axes' input relations, bounded value cycles.
pub(in crate::code_rule_dsl) fn finish(prog: Program) -> Result<Program, Error> {
    finish_given(prog, None)
}

/// `finish` of a program lowered without the rules of the `library` it reads (`rr::Text::lower_in`): its value cycles
/// are bounded by what the library's relations guarantee there (`cycles::check_given`).
pub(in crate::code_rule_dsl) fn finish_given(mut prog: Program, library: Option<&Program>) -> Result<Program, Error> {
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
        if !prog.decls.contains_key(rel.as_str()) {
            prog.decls.insert(rel.clone().into(), [("n".into(), "node".into()), ("a".into(), "node".into())].into());
            prog.inputs.push(rel);
        }
    }
    // An input's columns are its declaration's: the planner reads them.
    if let Some(relation) = prog.inputs.iter().find(|rel| !prog.decls.contains_key(rel.as_str())) {
        return Err(Error::UndeclaredInput { relation: relation.clone() });
    }
    let checking = crate::code_rule_dsl::logic::profile::span(|| "check the program's cycles and caches".to_owned());
    match library {
        Some(library) => cycles::check_given(&prog, library)?,
        None => cycles::check(&prog)?,
    };
    // A cache is two rules (decision 0103): a broken form refuses the program.
    caches::caches(&prog)?;
    drop(checking);
    Ok(prog)
}

/// A program to run reads only relations it declares or the transducer makes: else an error, as in Soufflé, not a
/// plan's panic. (Parsing alone does not ask it: a parse's tests read relations they never declare.) `match_all` and
/// `match_group` over a span are built-ins: the text columns make each regex's relation (`strings`).
pub fn declared(prog: &Program) -> Result<(), Error> {
    prog.rules.iter().try_for_each(|rule| rule_declared(prog, rule))
}

/// `declared` for one rule: every relation it reads is declared in `prog`.
pub fn rule_declared(prog: &Program, rule: &Rule) -> Result<(), Error> {
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
    // And with the columns it declares: a read of another arity names some other relation (a rule's own `field(n)`
    // beside the core `field(p, f, c)`), which no plan can join.
    let atoms = rule.body.iter().filter_map(|l| match l {
        Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a),
        _ => None,
    });
    for a in std::iter::once(&rule.head).chain(atoms) {
        if let Some(cols) = prog.decls.get(a.rel.as_str())
            && cols.len() != a.args.len()
        {
            return Err(Error::Syntax(format!(
                "relation {} is declared with {} columns and read with {} in the rule for {}",
                a.rel,
                cols.len(),
                a.args.len(),
                rule.head.rel
            )));
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
    let sccs = components_of(prog)?;
    Ok((sccs, edges(prog)))
}

/// The program's components, dependencies first (`strata` without its edges): an error if a negation or a
/// non-monotone aggregate closes a cycle. What most callers of `strata` read, made without the edges by name.
pub fn components_of(prog: &Program) -> Result<Vec<BTreeSet<String>>, Error> {
    Graph::of(prog).components(&prog.together)
}

/// `components_of` of a dependency graph read from a version (`program_image::analyses::components_by_name`): the
/// derived relations' names, sorted; each one's dependencies by their places among them, with whether negative, sorted
/// and each once; the non-monotone aggregation edges; and the sets run as one component. The components, their order
/// and the error are the tree's.
pub fn components_of_graph(names: Vec<&str>, succ: Vec<Vec<(u32, bool)>>, aggs: rustc_hash::FxHashSet<(u32, u32)>, together: &[BTreeSet<String>]) -> Result<Vec<BTreeSet<String>>, Error> {
    Graph { names, succ, aggs }.components(together)
}

/// Every component that a negation or a non-monotone aggregate closes a cycle in, one error each (`strata` stops at
/// the first): a caller that drops what made each cycle (demand's split, decision 0048) drops them all in one round.
pub fn unstratified(prog: &Program) -> Vec<Error> {
    let g = Graph::of(prog);
    let sccs = g.together(&prog.together);
    g.first_unstratified(&sccs, false)
}

/// The dependency graph of `strata` on numbers: the derived relations in their order, each one's dependencies among
/// them (with whether negative, sorted as `Edges` sorts them), and the non-monotone aggregation edges. The search, the
/// stratification check and `together`'s rings run on it; the edges by name are made only for a caller that reads
/// them (`edges`). Each edge as a pair of names in a sorted map was most of the strata's cost.
struct Graph<'p> {
    names: Vec<&'p str>,
    succ: Vec<Vec<(u32, bool)>>,
    aggs: rustc_hash::FxHashSet<(u32, u32)>,
}

impl<'p> Graph<'p> {
    fn of(prog: &'p Program) -> Graph<'p> {
        let mut names: Vec<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
        for produced in prog.answers.values() {
            names.extend(produced.iter().map(String::as_str));
        }
        names.sort_unstable();
        names.dedup();
        let id: rustc_hash::FxHashMap<&str, u32> = names.iter().enumerate().map(|(i, n)| (*n, i as u32)).collect();
        let mut succ: Vec<Vec<(u32, bool)>> = vec![Vec::new(); names.len()];
        // An answer is an edge: each produced relation reads its request (decision 0065 §2a).
        for (request, produced) in &prog.answers {
            if let Some(&r) = id.get(request.as_str()) {
                for p in produced {
                    succ[id[p.as_str()] as usize].push((r, false));
                }
            }
        }
        for (rel, native) in &prog.natives {
            let Some(&h) = id.get(rel.as_str()) else { continue };
            succ[h as usize].extend(native.reads().into_iter().filter_map(|read| id.get(read).map(|&r| (r, false))));
        }
        // Aggregation edges that are not monotone: negative, and named apart when they close a cycle.
        let mut aggs = rustc_hash::FxHashSet::default();
        for r in &prog.rules {
            let h = id[r.head.rel.as_str()];
            for l in &r.body {
                let Some(a) = l.atom() else { continue };
                let Some(&d) = id.get(a.rel.as_str()) else { continue };
                let agg = match l {
                    Literal::Agg { op, .. } => !prog.lattice.get(r.head.rel.as_str()).is_some_and(|d| {
                        matches!((op, d.lattice()), (AggOp::Count | AggOp::Max, "max") | (AggOp::Min, "min"))
                    }),
                    _ => false,
                };
                if agg && !a.gated {
                    aggs.insert((h, d));
                }
                succ[h as usize].push((d, (a.neg || agg) && !a.gated));
            }
        }
        for s in &mut succ {
            s.sort_unstable();
            s.dedup();
        }
        Graph { names, succ, aggs }
    }

    /// The components, each set of `sets` one of them (decision 0101 §3): its relations joined in a ring for the
    /// search only, so the edges stay the rules' own. A set that would take in a relation outside it (one read by a
    /// member and reading another) is left apart.
    fn together(&self, sets: &[BTreeSet<String>]) -> Vec<Vec<u32>> {
        let id = |r: &str| self.names.binary_search(&r).ok().map(|i| i as u32);
        let mut sets: Vec<Vec<u32>> = sets.iter().map(|t| t.iter().filter_map(|r| id(r)).collect::<Vec<_>>()).filter(|t| t.len() > 1).collect();
        loop {
            if sets.is_empty() {
                return tarjan_ids(&self.succ);
            }
            let mut ring = self.succ.clone();
            for t in &sets {
                for (i, &r) in t.iter().enumerate() {
                    let e = (t[(i + 1) % t.len()], false);
                    let s = &mut ring[r as usize];
                    if let Err(at) = s.binary_search(&e) {
                        s.insert(at, e);
                    }
                }
            }
            let sccs = tarjan_ids(&ring);
            let n = sets.len();
            sets.retain(|t| sccs.iter().find(|s| s.contains(&t[0])).is_some_and(|s| s.len() == t.len()));
            if sets.len() == n {
                return sccs;
            }
        }
    }

    /// The components, dependencies first, `together`'s sets each one (`together`): an error if a negation or a
    /// non-monotone aggregate closes a cycle.
    fn components(&self, together: &[BTreeSet<String>]) -> Result<Vec<BTreeSet<String>>, Error> {
        let sccs = self.together(together);
        match self.first_unstratified(&sccs, true).into_iter().next() {
            Some(e) => Err(e),
            None => Ok(self.named(&sccs)),
        }
    }

    fn named(&self, sccs: &[Vec<u32>]) -> Vec<BTreeSet<String>> {
        sccs.iter().map(|scc| scc.iter().map(|&r| self.names[r as usize].to_owned()).collect()).collect()
    }

    /// The first error of each component a negative edge closes a cycle in (`first`: only the first component's).
    fn first_unstratified(&self, sccs: &[Vec<u32>], first: bool) -> Vec<Error> {
        let mut at = vec![usize::MAX; self.names.len()];
        for (i, scc) in sccs.iter().enumerate() {
            for &r in scc {
                at[r as usize] = i;
            }
        }
        let mut out = Vec::new();
        for (i, scc) in sccs.iter().enumerate() {
            // In the order of names, as the components by name are read.
            let mut members = scc.clone();
            members.sort_unstable();
            'scc: for &v in &members {
                for &(w, neg) in &self.succ[v as usize] {
                    if !neg || at[w as usize] != i {
                        continue;
                    }
                    let component: BTreeSet<String> = scc.iter().map(|&r| self.names[r as usize].to_owned()).collect();
                    let (vn, wn) = (self.names[v as usize].to_owned(), self.names[w as usize].to_owned());
                    unstratified_trace(&vn, &wn, &component);
                    out.push(if self.aggs.contains(&(v, w)) {
                        Error::AggregationInRecursion { head: vn, aggregated: wn, component }
                    } else {
                        Error::NotStratifiable { message: format!("not stratifiable: {vn} depends negatively on {wn} in its own component"), component, negated: wn }
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
}

/// `RERAIL_STRATA_COUNTS` (a diagnostic): a refused component's members, which the error's message leaves out.
pub fn unstratified_trace(head: &str, dep: &str, component: &BTreeSet<String>) {
    if std::env::var_os("RERAIL_STRATA_COUNTS").is_some() {
        eprintln!("RERAIL_STRATA_COUNTS unstratified (magic sets may recover): {head} -> {dep} in a component of {}: {}", component.len(), component.iter().cloned().collect::<Vec<_>>().join(" "));
    }
}

/// The dependency edges by name (`strata`'s second part): head → {(dependency, negative)}.
fn edges(prog: &Program) -> Edges {
    let mut derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let mut edges = Edges::new();
    for (request, produced) in &prog.answers {
        for p in produced {
            derived.insert(p.as_str());
            edges.entry(p.clone()).or_default().insert((request.clone(), false));
        }
    }
    for (rel, native) in &prog.natives {
        if !derived.contains(rel.as_str()) { continue; }
        edges.entry(rel.clone()).or_default().extend(native.reads().into_iter()
            .filter(|read| derived.contains(*read)).map(|read| (read.to_owned(), false)));
    }
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
            edges.entry(r.head.rel.to_string()).or_default().insert((a.rel.to_string(), (a.neg || agg) && !a.gated));
        }
    }
    edges
}

/// The sets of `prog.together` that are not each one component as `strata` makes them.
pub fn apart(prog: &Program) -> Vec<usize> {
    let g = Graph::of(prog);
    let sccs = g.named(&g.together(&prog.together));
    (0..prog.together.len()).filter(|&i| {
        let t = &prog.together[i];
        !sccs.iter().any(|s| s == t)
    }).collect()
}

/// `apart` over a graph given by its names (sorted) and each name's reads (sorted, each with whether negative): for
/// a program held as a version, whose rules are not trees.
pub fn apart_of_graph(names: Vec<&str>, succ: Vec<Vec<(u32, bool)>>, together: &[BTreeSet<String>]) -> Vec<usize> {
    let g = Graph { names, succ, aggs: rustc_hash::FxHashSet::default() };
    let sccs = g.named(&g.together(together));
    (0..together.len()).filter(|&i| !sccs.iter().any(|s| *s == together[i])).collect()
}

/// Strongly connected components on numbers (`tarjan`), dependencies first; each component's members in the order
/// they leave the stack.
fn tarjan_ids(succ: &[Vec<(u32, bool)>]) -> Vec<Vec<u32>> {
    const NONE: u32 = u32::MAX;
    let n = succ.len();
    let (mut index, mut low, mut on) = (vec![NONE; n], vec![0u32; n], vec![false; n]);
    let (mut stack, mut sccs, mut counter) = (Vec::<u32>::new(), Vec::new(), 0u32);
    for root in 0..n as u32 {
        if index[root as usize] != NONE {
            continue;
        }
        let mut frames: Vec<(u32, usize)> = vec![(root, 0)];
        index[root as usize] = counter;
        low[root as usize] = counter;
        counter += 1;
        stack.push(root);
        on[root as usize] = true;
        while let Some((v, at)) = frames.last_mut() {
            let v = *v;
            if let Some(&(w, _)) = succ[v as usize].get(*at) {
                *at += 1;
                if index[w as usize] == NONE {
                    index[w as usize] = counter;
                    low[w as usize] = counter;
                    counter += 1;
                    stack.push(w);
                    on[w as usize] = true;
                    frames.push((w, 0));
                } else if on[w as usize] {
                    low[v as usize] = low[v as usize].min(index[w as usize]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent as usize] = low[parent as usize].min(low[v as usize]);
            }
            if low[v as usize] == index[v as usize] {
                let mut scc = Vec::new();
                loop {
                    let w = stack.pop().unwrap();
                    on[w as usize] = false;
                    scc.push(w);
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

/// Strongly connected components of `nodes` under `edges`, dependencies first, in the same order every run.
/// Iterative, where T's recursed (with a raised recursion limit).
///
/// The nodes are numbered in their order and each edge looked up once, so the search runs on numbers: it looked up
/// each node's index, low link and stack mark by name at every step, a string comparison per level of three maps (the
/// strata are asked of every program a batch prepares, by some thirty passes). Roots and successors are taken in the
/// same order as by name, so the components and their order are the same.
pub fn tarjan(nodes: &BTreeSet<String>, edges: &Edges) -> Vec<BTreeSet<String>> {
    let names: Vec<&String> = nodes.iter().collect();
    let id: rustc_hash::FxHashMap<&str, u32> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i as u32)).collect();
    // Each node's successors among the nodes, in the edges' order.
    let succ: Vec<Vec<u32>> = names
        .iter()
        .map(|v| edges.get(v.as_str()).into_iter().flatten().filter_map(|(w, _)| id.get(w.as_str()).copied()).collect())
        .collect();
    const NONE: u32 = u32::MAX;
    let n = names.len();
    let (mut index, mut low, mut on) = (vec![NONE; n], vec![0u32; n], vec![false; n]);
    let (mut stack, mut sccs, mut counter) = (Vec::<u32>::new(), Vec::new(), 0u32);
    for root in 0..n as u32 {
        if index[root as usize] != NONE {
            continue;
        }
        // Frames: a node and the position of its next successor.
        let mut frames: Vec<(u32, usize)> = vec![(root, 0)];
        index[root as usize] = counter;
        low[root as usize] = counter;
        counter += 1;
        stack.push(root);
        on[root as usize] = true;
        while let Some((v, at)) = frames.last_mut() {
            let v = *v;
            if let Some(&w) = succ[v as usize].get(*at) {
                *at += 1;
                if index[w as usize] == NONE {
                    index[w as usize] = counter;
                    low[w as usize] = counter;
                    counter += 1;
                    stack.push(w);
                    on[w as usize] = true;
                    frames.push((w, 0));
                } else if on[w as usize] {
                    low[v as usize] = low[v as usize].min(index[w as usize]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent as usize] = low[parent as usize].min(low[v as usize]);
            }
            if low[v as usize] == index[v as usize] {
                let mut scc = BTreeSet::new();
                loop {
                    let w = stack.pop().unwrap();
                    on[w as usize] = false;
                    scc.insert(names[w as usize].clone());
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
    reach(prog, roots, false).0
}

/// `needed`, and which rules are kept: every rule of a needed relation. With `gated` (`prune`), a request rule
/// (`Program::asked_by`, decision 0174) only once a relation whose readers it serves is needed, or its relation is a
/// root. `needed` reads every request rule: a stage that computes requests names them as its roots. A request rule is read through its
/// attribute's answer, which every reader reads; without its readers' gate an unreached reader's request would keep
/// its body's relations alive, and lowering a program whole would prune to more than lowering it without its
/// unreached clauses (`dropping_unreached_clauses_prunes_to_the_same_program`).
fn reach(prog: &Program, roots: Vec<String>, gated: bool) -> (BTreeSet<String>, Vec<bool>) {
    // Each request rule's readers, by its place among its relation's rules; a relation whose rules no longer match
    // the record (rules added or dropped since lowering) keeps them ungated.
    let mut gate: Vec<Option<&BTreeSet<String>>> = vec![None; prog.rules.len()];
    for (req, readers) in prog.asked_by.iter().filter(|(req, _)| gated && !roots.contains(req)) {
        let at: Vec<usize> = prog.rules.iter().enumerate().filter(|(_, r)| r.head.rel == req.as_str()).map(|(i, _)| i).collect();
        if at.len() == readers.len() {
            for (i, by) in at.into_iter().zip(readers) {
                gate[i] = Some(by);
            }
        }
    }
    let mut body_of: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, r) in prog.rules.iter().enumerate() {
        body_of.entry(&r.head.rel).or_default().push(i);
    }
    let mut kept = vec![false; prog.rules.len()];
    let (mut need, mut todo) = (BTreeSet::new(), roots);
    loop {
        while let Some(r) = todo.pop() {
            if !need.insert(r.clone()) {
                continue;
            }
            if let Some(native) = prog.natives.get(&r) {
                todo.extend(native.reads().into_iter().map(str::to_owned));
            }
            for &i in body_of.get(r.as_str()).into_iter().flatten() {
                if gate[i].is_none() {
                    kept[i] = true;
                    todo.extend(prog.rules[i].body.iter().filter_map(Literal::atom).map(|a| a.rel.to_string()));
                }
            }
        }
        // The request rules whose relation and one of whose readers are needed: their bodies are read, which may
        // need more readers.
        for (i, by) in gate.iter().enumerate() {
            if let Some(by) = by
                && !kept[i]
                && need.contains(prog.rules[i].head.rel.as_str())
                && by.iter().any(|b| need.contains(b))
            {
                kept[i] = true;
                todo.extend(prog.rules[i].body.iter().filter_map(Literal::atom).map(|a| a.rel.to_string()));
            }
        }
        if todo.is_empty() {
            return (need, kept);
        }
    }
}

/// Drops the rules of relations no output reads, directly or through other rules, and the request rules none of
/// whose readers is read (`reach`). T's `prune`. Facts stay: the engine reads some by name, read by no rule (`env`'s
/// settings, `corpus::World`; `unconverged`, the pending round).
pub fn prune(mut prog: Program) -> Program {
    let (need, kept) = reach(&prog, prog.outputs.clone(), true);
    // The record follows the rules kept: each request relation's kept rules, in order, with their readers.
    let gated: BTreeMap<String, Vec<bool>> = prog.asked_by.keys().map(|req| {
        let mine: Vec<bool> = prog.rules.iter().zip(&kept).filter(|(r, _)| r.head.rel == req.as_str()).map(|(_, k)| *k).collect();
        (req.clone(), mine)
    }).collect();
    for (req, readers) in prog.asked_by.iter_mut() {
        if let Some(mine) = gated.get(req) && mine.len() == readers.len() {
            let mut it = mine.iter();
            readers.retain(|_| *it.next().unwrap());
        }
    }
    let mut keep = kept.into_iter();
    prog.rules.retain(|_| keep.next().unwrap());
    prog.asked_by.retain(|req, readers| !readers.is_empty() || prog.rules.iter().any(|r| r.head.rel == req.as_str()));
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

/// The pragma by which a program asked for bounded rounds (decision 0056), refused: bounded cross-file rounds are retired: a program's cross-file part is one fixpoint (decisions 0065, 0276 §9).
pub const PRAGMA: &str = "cross-rounds";

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
