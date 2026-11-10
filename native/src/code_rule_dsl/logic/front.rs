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
//! - unbounded value cycles (`cycles`, O11 option b): a recursion that creates values must move along the tree.
//!
//! §12's other named errors are not here: a non-interned cross-document join needs column types, and the cost gate
//! is the planner's (row 5).
//!
//! Tree structure is never a relation: `nearest($n, $s, set)` is a built-in over the tape (T's `rdl.py` docstring).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub(in crate::code_rule_dsl) mod cycles;

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
pub(in crate::code_rule_dsl) fn operator(op: &str) -> Option<&'static str> {
    PUNCT.iter().copied().find(|p| *p == op)
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
        // A head's `@dot(p, s)` names p and s too.
        let mut out = BTreeSet::new();
        for a in &self.args {
            a.vars(&mut out);
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum AggOp {
    Count,
    Min,
    Max,
    /// Soufflé's `sum`: 0 over no rows.
    Sum,
    /// `product x : {…}`: the product of `x` over the rows, 1 over no rows, saturating at i64's range (a count of
    /// combinations, as pytest's stacked parametrize marks multiply).
    Product,
    /// `core.rr`'s `join x by k sep ","`: the `over` values ordered by the `by` columns' values (ties by the value),
    /// joined by `sep`; "" over no rows.
    Join { by: Vec<String>, sep: String },
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) enum Literal {
    Atom(Atom),
    Cmp { l: Term, op: &'static str, r: Term },
    /// `$var = count { atom }`, or `$var = min $over : { atom }` (`max` likewise).
    Agg { var: String, op: AggOp, over: Option<String>, atom: Atom },
    /// `nearest($n, $a, kindset)`: `$a` is `$n`'s nearest strict ancestor of a kind in the set.
    Nearest { n: String, a: String, kindset: String },
    /// `ast($document, $node, { matcher }, NAME: $v, …)`: the matcher's text, lowered to a transducer program (§3),
    /// and the metavariables it exports, each `(NAME, v)` binding `$v` to what `$NAME` captured.
    Ast { document: String, node: String, matcher: String, captures: Vec<(String, String)> },
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
            Literal::Ast { document, node, captures, .. } => {
                out.extend([document.clone(), node.clone()]);
                out.extend(captures.iter().map(|(_, v)| v.clone()));
            }
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

// T's `repr`s, which its transformed programs are compared in.
impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Term::Var(v) => write!(f, "${v}"),
            Term::Wild => f.write_str("_"),
            Term::Int(n) => write!(f, "{n}"),
            Term::Str(s) => f.write_str(&serde_json::to_string(s).expect("a string serializes")),
            Term::Top => f.write_str("top"),
            Term::Call(name, args) => write!(f, "@{name}({})", join(args)),
        }
    }
}

impl fmt::Display for Atom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}({})", if self.neg { "!" } else { "" }, self.rel, join(&self.args))
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Atom(a) => write!(f, "{a}"),
            Literal::Cmp { l, op, r } => write!(f, "{l} {op} {r}"),
            Literal::Agg { var, op: AggOp::Count, atom, .. } => write!(f, "${var} = count {{ {atom} }}"),
            Literal::Agg { var, op: AggOp::Join { by, sep }, over, atom } => {
                let by: Vec<String> = by.iter().map(|b| format!("${b}")).collect();
                let sep = serde_json::to_string(sep).expect("a string serializes");
                write!(f, "${var} = join ${} by ({}) sep {sep} : {{ {atom} }}", over.as_deref().unwrap_or_default(), by.join(", "))
            }
            Literal::Agg { var, op, over, atom } => {
                let op = match op {
                    AggOp::Min => "min",
                    AggOp::Sum => "sum",
                    AggOp::Product => "product",
                    _ => "max",
                };
                write!(f, "${var} = {op} ${} : {{ {atom} }}", over.as_deref().unwrap_or_default())
            }
            Literal::Nearest { n, a, kindset } => write!(f, "nearest(${n}, ${a}, {kindset})"),
            Literal::Ast { document, node, matcher, captures } => {
                write!(f, "ast(${document}, ${node}, {matcher}")?;
                captures.iter().try_for_each(|(name, v)| write!(f, ", {name}: ${v}"))?;
                f.write_str(")")
            }
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} :- {}", self.head, join(&self.body))
    }
}

pub(in crate::code_rule_dsl) fn join<T: fmt::Display>(items: &[T]) -> String {
    items.iter().map(T::to_string).collect::<Vec<_>>().join(", ")
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

    pub(in crate::code_rule_dsl) fn bounded(&self) -> bool {
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
    /// Logically nullary relations encoded with one physical unit column.
    pub(in crate::code_rule_dsl) nullary: BTreeSet<String>,
    pub(in crate::code_rule_dsl) lattice: BTreeMap<String, LatticeDecl>,
    pub(in crate::code_rule_dsl) facts: BTreeMap<String, Vec<Vec<Value>>>,
    pub(in crate::code_rule_dsl) rules: Vec<Rule>,
    pub(in crate::code_rule_dsl) inputs: Vec<String>,
    pub(in crate::code_rule_dsl) outputs: Vec<String>,
    pub(in crate::code_rule_dsl) kindsets: BTreeMap<String, Vec<String>>,
    /// `.demand` relations, or `*` for every derived one.
    pub(in crate::code_rule_dsl) demand: Vec<String>,
    /// Set by `magic`: demanded relations computed in full, and why.
    pub(in crate::code_rule_dsl) demand_dropped: Vec<(String, String)>,
    /// Set by `magic`: the extras its magic rules kept.
    pub(in crate::code_rule_dsl) magic_extras: Option<usize>,
    pub(in crate::code_rule_dsl) magic_report: Option<crate::code_rule_dsl::logic::magic::Report>,
    /// Gated negations (`magic::gate_negations`), by their `done_` relation, in the order they were made.
    pub(in crate::code_rule_dsl) gates: Vec<(String, crate::code_rule_dsl::logic::magic::Gate)>,
    /// `.policy { … }`: lint-rule documents, each without its matchers, which are the rules' `ast` atoms (`yaml`).
    pub(in crate::code_rule_dsl) policies: Vec<serde_json::Value>,
    /// `.rr` setting directives (`.mode block`, `.scope include "…"`, …), by name, as written (`logic::rr`).
    pub(in crate::code_rule_dsl) settings: Vec<(String, String)>,
    /// Relations the evaluator makes from their inputs' rows rather than from rules (`dfs_first`), by name. Each
    /// has one rule over its inputs, which orders it after them; its rows are the native operator's.
    pub(in crate::code_rule_dsl) natives: BTreeMap<String, Native>,
    /// The regexes `match_all` and `match` run over `source` or `src_line` spans (`core.rr`: their captures are
    /// spans), and over which: the batch makes `rx_span(p, c, j, g, m)` for them, each match's groups as spans.
    pub(in crate::code_rule_dsl) rx_spans: std::collections::BTreeSet<(SpanIn, String)>,
    pub(in crate::code_rule_dsl) ast_patterns: std::collections::BTreeSet<String>,
    pub(in crate::code_rule_dsl) doc_scopes: Option<Vec<(Vec<String>, Vec<String>)>>,
    pub(in crate::code_rule_dsl) answers: BTreeMap<String, Vec<String>>,
    /// `.scope text`'s globs: a document matching one that has no grammar of the run (`.md`, `.rst`) is read as text
    /// only, its `doc`, `source` and `src_line` and no tree, when a scope selects it. Every program's, in a combined
    /// one.
    pub(in crate::code_rule_dsl) text_globs: Vec<String>,
    /// `.decl r(…) inline`, `no_inline`, `magic`, `no_magic` (Soufflé's qualifiers, decision 0099): the program's own
    /// choice of what a relation keeps, by relation, overriding the planner's (`plan::keeps`).
    pub(in crate::code_rule_dsl) qualifiers: BTreeMap<String, BTreeSet<String>>,
    /// A session's maintained strata, each with the rules that derive it again instead (decision 0090 §4,
    /// `maintain::rewrite`): the run chooses per stratum, once its input deltas are known (`plan::run`).
    pub(in crate::code_rule_dsl) again: Vec<Again>,
}

/// A maintained stratum's other form (decision 0090 §4). Both make its relations and their deltas (`r__minus`,
/// `r__plus`), which are all that other strata read of it, so each stratum chooses alone.
#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct Again {
    /// The stratum's own relations (one fixpoint when derived again).
    pub(in crate::code_rule_dsl) scc: BTreeSet<String>,
    /// Every relation the maintained form makes for it: its relations, their pieces (`__del`, `__ins`, …) and deltas.
    pub(in crate::code_rule_dsl) owned: BTreeSet<String>,
    /// Its rules over the current inputs, then its deltas against its old rows.
    pub(in crate::code_rule_dsl) rules: Vec<Rule>,
    pub(in crate::code_rule_dsl) deltas: Vec<Rule>,
    /// The input deltas its maintained form reads, and its relations' old rows: the sizes that choose.
    pub(in crate::code_rule_dsl) reads: Vec<String>,
    pub(in crate::code_rule_dsl) olds: Vec<String>,
}

/// The `.decl` qualifiers that choose what a relation keeps (decision 0099), as Soufflé spells them.
pub(in crate::code_rule_dsl) const QUALIFIERS: &[&str] = &["inline", "no_inline", "magic", "no_magic"];

impl Program {
    /// Whether `.decl rel(…)` carries the qualifier `word` (one of `QUALIFIERS`).
    pub(in crate::code_rule_dsl) fn qualified(&self, rel: &str, word: &str) -> bool {
        self.qualifiers.get(rel).is_some_and(|q| q.contains(word))
    }
}

/// A `.scope` entry taken from a policy option (`.scope dirs option("applicationRoots")`), as `doc_scopes` holds it
/// until the run's options are known: `rr-option:<form>:<name>`, `form` one of `dirs`, `include`, `exclude`. As a glob
/// it holds no path; `resolve_scopes` adds the option's values beside it.
pub(in crate::code_rule_dsl) const OPTION_GLOB: &str = "rr-option:";

/// `.global r, …`: relations of a policy that a combined run shares, as it shares a library's, when every policy that
/// has one defines it alike (decision 0091). `.global library r, …` names an included file's relations (`rr::read_file`
/// writes it): shared where alike, else each policy's own.
pub(in crate::code_rule_dsl) const GLOBAL: &str = "global";
/// `.global`'s first word for an included file's relations.
pub(in crate::code_rule_dsl) const LIBRARY: &str = "library";

/// A program's `.global` relations: (named by the policy, an included file's).
pub(in crate::code_rule_dsl) fn globals(settings: &[(String, String)]) -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut named, mut library) = (BTreeSet::new(), BTreeSet::new());
    for (_, text) in settings.iter().filter(|(k, _)| k == GLOBAL) {
        let (to, rest) = match text.trim().strip_prefix(LIBRARY).filter(|r| r.starts_with(char::is_whitespace)) {
            Some(rest) => (&mut library, rest),
            None => (&mut named, text.as_str()),
        };
        to.extend(rest.split(|c: char| c == ',' || c.is_whitespace()).filter(|w| !w.is_empty()).map(str::to_owned));
    }
    (named, library)
}

/// The documents out of a policy's scope (`.scope`'s `dirs`, `include`, `exclude`), as lowering derives them:
/// `rr_scope_not(d)`. In a combined run, the guards on the policy's reads negate it (decision 0091).
pub(in crate::code_rule_dsl) const SCOPE_NOT: &str = "rr_scope_not";

/// `scopes` with each option entry's values (`OPTION_GLOB`) added beside it, from the run's `options` as
/// `option`/`option_item` read them: a directory under `dirs`, a glob under `include` or `exclude`. An option the run
/// does not set adds nothing, and its entry still holds no path (an entry whose only globs are such reads none).
pub(in crate::code_rule_dsl) fn resolve_scopes(scopes: &Option<Vec<(Vec<String>, Vec<String>)>>, options: &serde_json::Value) -> Option<Vec<(Vec<String>, Vec<String>)>> {
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

pub(in crate::code_rule_dsl) fn in_doc_scopes(scopes: &Option<Vec<(Vec<String>, Vec<String>)>>, path: &str) -> bool {
    let glob = |g: &String| crate::code_rule_dsl::logic::strings::test("glob", path, g);
    scopes.as_ref().is_none_or(|entries| entries.iter().any(|(within, out)| (within.is_empty() || within.iter().any(glob)) && !out.iter().any(glob)))
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::code_rule_dsl) enum SpanIn {
    Source,
    Line,
    Comment,
    /// Nodes of this kind.
    Kind(String),
    /// The spans group `.1` of pattern `.0` captured.
    Capture(String, String),
}

/// A relation made natively from relations of the program (`Program::natives`).
#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) enum Native {
    /// `dfs_first(r, hit, first, edge, target)`; with `roots`, `dfs_first_memo` (roots in the order of their
    /// second column, one memo shared).
    DfsFirst { edge: String, target: String, roots: Option<String> },
    /// `match_all(re, s)` over symbols: `(s, g…)`, a row per non-overlapping match of `pattern` in each `s` of
    /// `input`'s one column whose `groups` all take part, each group's text (`logic::native::match_all`).
    MatchAll { input: String, pattern: String, groups: Vec<String>, positions: bool },
    /// `components(d, c, key, edge, own)`: each document's strongly connected component of `edge(d, e)` (its
    /// smallest document) and the component's Merkle key over `own(d, digest)` (`logic::native::components`).
    Components { edge: String, own: String },
}

impl Native {
    /// Rewrite producer inputs with the same relation mapping as the rest of its program.
    pub(in crate::code_rule_dsl) fn rename_reads(&mut self, rename: impl Fn(&str) -> String) {
        match self {
            Native::DfsFirst { edge, target, roots } => {
                *edge = rename(edge);
                *target = rename(target);
                if let Some(roots) = roots { *roots = rename(roots); }
            }
            Native::MatchAll { input, .. } => *input = rename(input),
            Native::Components { edge, own } => {
                *edge = rename(edge);
                *own = rename(own);
            }
        }
    }
    /// The relations it reads.
    pub(in crate::code_rule_dsl) fn reads(&self) -> Vec<&str> {
        match self {
            Native::DfsFirst { edge, target, roots } => [Some(edge.as_str()), Some(target.as_str()), roots.as_deref()].into_iter().flatten().collect(),
            Native::MatchAll { input, .. } => vec![input.as_str()],
            Native::Components { edge, own } => vec![edge.as_str(), own.as_str()],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) enum Error {
    Syntax(String),
    /// A relation depends negatively on another in its own component.
    NotStratifiable { message: String, component: BTreeSet<String>, negated: String },
    /// An aggregate reads its own component, and the head is not a lattice the aggregate is monotone in.
    AggregationInRecursion { head: String, aggregated: String, component: BTreeSet<String> },
    /// A lattice of unbounded height with no `widen`.
    UnboundedLattice { relation: String },
    /// A demanded relation is used with its key unbound: its demand would be everything (magic sets).
    UnboundDemand { message: String, relation: String },
    /// A component creates values (`@cat`, …) without moving along the tree (`cycles`): `rule` is one it
    /// could not bound.
    UnboundedValueCycle { component: BTreeSet<String>, rule: String },
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
                write!(f, "unbounded value cycle: {{{component}}} creates values without moving along the tree ({rule})")
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
            return Ok(Atom { rel: format!("{axis}{set}"), args, neg, gated: false });
        }
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
                if key != crate::code_rule_dsl::typing::cross_rounds::PRAGMA || k.parse::<usize>().is_err() {
                    self.i -= 2;
                    return self.fail("`.pragma \"cross-rounds\" \"k\"`");
                } else {
                    prog.settings.push(crate::code_rule_dsl::typing::cross_rounds::setting(&k));
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
        prog.rules.push(Rule { head, body });
    }
    finish(prog)
}

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
    let decls = prog.decls.clone();
    let typed = |a: &Atom, nodes: &mut BTreeSet<String>| {
        for (t, (_, ty)) in a.args.iter().zip(decls.get(&a.rel).map_or(&[][..], Vec::as_slice)) {
            if let Term::Var(v) = t
                && ty == "node"
            {
                nodes.insert(v.clone());
            }
        }
    };
    let order = |n: &str, k: &str| Literal::Atom(Atom { rel: NODE_ORDER.to_owned(), args: vec![Term::Var(n.to_owned()), Term::Var(k.to_owned())], neg: false, gated: false });
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
                    let cols = decls.get(&atom.rel).cloned().unwrap_or_default();
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
                    helpers.push((Rule { head: Atom { rel: name.clone(), args: head_args, neg: false, gated: false }, body: vec![Literal::Atom(atom), order(&o, &k)] }, head_cols));
                    body.push(Literal::Agg { var: kv.clone(), op, over: Some(k), atom: Atom { rel: name, args: agg_args, neg: false, gated: false } });
                    body.push(order(&var, &kv));
                    used = true;
                }
                other => body.push(other),
            }
        }
        r.body = body;
    }
    for (rule, cols) in helpers {
        prog.decls.insert(rule.head.rel.clone(), cols);
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
    let axes: Vec<String> = prog.rules.iter().flat_map(|r| r.body.iter().filter_map(Literal::atom)).map(|a| a.rel.clone()).filter(|r| is_axis(r)).collect();
    for rel in axes {
        if !prog.decls.contains_key(&rel) {
            prog.decls.insert(rel.clone(), vec![("n".to_owned(), "node".to_owned()), ("a".to_owned(), "node".to_owned())]);
            prog.inputs.push(rel);
        }
    }
    cycles::check(&prog)?;
    Ok(prog)
}

/// A program to run reads only relations it declares or the transducer makes: else an error, as in Soufflé, not a
/// plan's panic. (Parsing alone does not ask it: a parse's tests read relations they never declare.)
pub(in crate::code_rule_dsl) fn declared(prog: &Program) -> Result<(), Error> {
    for rule in &prog.rules {
        let mut read = rule.body.iter().filter_map(|l| match l {
            Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a),
            _ => None,
        });
        // The import components' relations (`rr_cx_reach`, decision 0076) are added by the run to a program that reads
        // them: declared there.
        if let Some(a) = read.find(|a| !prog.decls.contains_key(&a.rel) && !is_transduced(&a.rel) && !a.rel.starts_with("rr_cx_")) {
            return Err(Error::Syntax(format!("relation {} is read by the rule for {} and not declared: declare it with `.decl`", a.rel, rule.head.rel)));
        }
    }
    Ok(())
}

/// The prefix of the relations the engine names itself (helpers, axes, places): a rule file may not declare one
/// (`rr::parse_file`), so a rule's relation never collides with them.
pub(in crate::code_rule_dsl) const RESERVED: &str = "rr_";

/// The prefix of a tree axis's relation: `rr_inside_<set>(n, a)`, a an ancestor of n whose kind is in the set
/// (decision 0036).
pub(in crate::code_rule_dsl) const AXIS: &str = "rr_inside_";

/// The prefix of the descendant axis's relation: `rr_has_<set>(n, a)`, n of a kind in the set and a any strict
/// ancestor of n (`core.rr`'s `has(a, n, set)`: the set tests the descendant).
pub(in crate::code_rule_dsl) const DESCENDANTS: &str = "rr_has_";

/// The prefix of a `nearest(n, a, set)` literal's relation: `rr_nearest_<set>(n, a)` (`plan::lower`).
pub(in crate::code_rule_dsl) const NEAREST: &str = "rr_nearest_";

/// Whether `rel` is a tree axis's relation, which the transducer makes: `(n, a)` rows.
pub(in crate::code_rule_dsl) fn is_axis(rel: &str) -> bool {
    rel.starts_with(AXIS) || rel.starts_with(DESCENDANTS)
}

/// Whether the transducer makes `rel`: a tree axis's relation or a `nearest` one, `(n, a)` rows.
pub(in crate::code_rule_dsl) fn is_transduced(rel: &str) -> bool {
    is_axis(rel) || rel.starts_with(NEAREST)
}

/// Dependency edges: head → {(dependency, negative)}; an aggregate's is negative, a gated negation's is not.
pub(in crate::code_rule_dsl) type Edges = BTreeMap<String, BTreeSet<(String, bool)>>;

/// The program's components, dependencies first, and its dependency edges. T's `strata`, with aggregation in
/// recursion named apart from negation, and allowed into a lattice head it is monotone in (`count`/`max` into a
/// `max` lattice, `min` into a `min` lattice).
pub(in crate::code_rule_dsl) fn strata(prog: &Program) -> Result<(Vec<BTreeSet<String>>, Edges), Error> {
    let mut derived: BTreeSet<String> = prog.rules.iter().map(|r| r.head.rel.clone()).collect();
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
            if !derived.contains(&a.rel) {
                continue;
            }
            let agg = match l {
                Literal::Agg { op, .. } => !prog.lattice.get(&r.head.rel).is_some_and(|d| {
                    matches!((op, d.lattice()), (AggOp::Count | AggOp::Max, "max") | (AggOp::Min, "min"))
                }),
                _ => false,
            };
            if agg && !a.gated {
                aggs.insert((r.head.rel.clone(), a.rel.clone()));
            }
            edges.entry(r.head.rel.clone()).or_default().insert((a.rel.clone(), (a.neg || agg) && !a.gated));
        }
    }
    let sccs = tarjan(&derived, &edges);
    for scc in &sccs {
        for v in scc {
            for (w, neg) in edges.get(v).into_iter().flatten() {
                if !*neg || !scc.contains(w) {
                    continue;
                }
                if aggs.contains(&(v.clone(), w.clone())) {
                    return Err(Error::AggregationInRecursion { head: v.clone(), aggregated: w.clone(), component: scc.clone() });
                }
                return Err(Error::NotStratifiable {
                    message: format!("not stratifiable: {v} depends negatively on {w} in its own component"),
                    component: scc.clone(),
                    negated: w.clone(),
                });
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

/// The relations `roots` read, directly or through other rules, the roots included (`prune`'s, from the outputs).
pub(in crate::code_rule_dsl) fn needed(prog: &Program, roots: Vec<String>) -> BTreeSet<String> {
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
            todo.extend(body.iter().filter_map(Literal::atom).map(|a| a.rel.clone()));
        }
    }
    need
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
        if let Some(native) = prog.natives.get(&r) {
            todo.extend(native.reads().into_iter().map(str::to_owned));
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
#[cfg(test)]
mod data_layer_census_tests;
#[cfg(test)]
mod data_layer_tests;
pub(in crate::code_rule_dsl) mod yaml;
