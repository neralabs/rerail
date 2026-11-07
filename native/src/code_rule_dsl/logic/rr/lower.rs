//! `.rr` statements to the engine's `Program`: the same program the engine plans whatever the syntax.
//!
//! Soufflé spellings map to the engine's forms:
//! - `number`, `symbol` are `int`, `sym`; `span` and `loc` are `node`;
//! - a lattice column `v: flat<>` (or `min<>`, `max<>`), or `v: L<>` for `.lattice L<base> { Lub -> @f }`, is the
//!   engine's lattice on that relation, and `widen k` after the declaration bounds it;
//! - `.pragma "magic-transform" "a, b"` is demand on `a` and `b` (`"*"`: every derived relation);
//! - the text tests `glob(p, s)`, `prefix`, `suffix`, `contains`, `word` and their `i` forms, and `match(re, s)`,
//!   are the engine's tests (`logic::strings`), pattern first; a disjunction of one test over several patterns
//!   `(glob("a", s) ; glob("b", s))` is one test of the set, and so are adjacent negations of one test;
//!   `(s = "a" ; s = "b")` is one membership test;
//! - a relation of facts given to `nearest`, `inside` or `has` is a kind set, which the transducer answers:
//!   `inside(n, a, s)` is `inside_<s>(n, a)`, and `has(a, n, s)` is `inside_rr_has_<s>(n, a)`, the set tested on n;
//!   a set of nodes, or of kinds rules derive, is read as rules over `child` instead (`Lowerer::derived_axes`),
//!   which read the set negated, so it is complete first;
//! - `@dot` is the engine's `@cat`, and `split_dot(q, m, x)` is `q = @dot(m, x)` with x dot-free (it runs either
//!   way);
//! - `count : { … }`, `min x : { … }`, `max x : { … }`, `sum x : { … }` and `join x by k sep "s" : { … }` are
//!   aggregates; a body of more than one atom, or of one built-in the lowering writes out, gets a helper relation
//!   (count and sum keep every variable: they read a multiset of rows; join reads the distinct rows of its value
//!   and keys); a value or key that is an expression is a variable the body computes; join's keys are variables of
//!   its body, ordered by text for symbols and by value for numbers, ties by the value;
//! - an aggregate inside a term (`1 + count : { … }`, in a head or a comparison) is an aggregate of its own whose
//!   variable the term uses; an aggregate on one side of a comparison gets a variable;
//! - `!(a, b)` is a helper relation of the variables it shares with the rule, negated (`Lowerer::negated`);
//!   `!(a ; b)` is one per alternative;
//! - other disjunctions are one rule per alternative.
//!
//! - `.violation V(at: loc, …)` declares V as an output, and each row of V is a finding
//!   `violation(policy, V, code, path, line)`: at a node, its document's path and its line; at a
//!   `@place(path, line)` (the loc value `-2 - (path << 32 | line)`), that path and line. The code is V,
//!   `code "X"`, or the `code {c}` column. `suppress TOOL "rule", … [over {c}]` and `fix [safe] F "title"` are kept
//!   as the settings `suppress V …` and `fix V …` for the findings layer, which applies them;
//! - a rule ending in `=> edits` defines a fix, and fixes are not run: it is left out;
//! - `.input r(IO=…, filename="…")` is an input, its parameters kept as the setting `input`;
//! - core.rr is the prelude (`rr::program` reads it before every file): the program keeps the core relations it reads
//!   (a relation read and not declared takes its declaration there), and those that are tree arithmetic lower to
//!   the base relations (`core`: `kind`, `within`, `next_sibling`, `precedes`, `earlier`, `unset`, `docof`), negated
//!   through a relation of their rows;
//! - `within_stop(n, a, stop)` is a range over the tape's preorder ids (`Lowerer::within`), and so is its negation;
//!   `!nearest(n, a, s)` is "none, or another"; `precedes_stop` walks the siblings to the next stop
//!   (`rr_next_stop_<s>`);
//! - `match("(?<x>…)", s)` binds x to the named group (`Lowerer::captures`); `+ - * / %` and Soufflé's `cat` of
//!   any arity (`concat` folded from the left), `strlen`, `to_string`, `substr`, `to_number`, `max/2` and `min/2`
//!   are functors. `to_number` reads as Soufflé's `symbol2numeric`; where Soufflé stops with an error (no digits,
//!   out of range, `x % 0`) there is no value, and so no row. `@lower` is Unicode lowercase; `@dot("", b)` is b.
//!
//! Not lowered yet, each a named error: patterns (``n ~ `…` ``), `must`/`may`, a `.violation` at a span or loc
//! that can hold a span the engine does not make (a regex capture, a token: `Lowerer::origins`), `ordered by`,
//! `mean`, `to_float` and decimal numbers (the engine has no decimal values), `dfs_first`/`dfs_first_memo`, and a
//! negated built-in with its other node unnamed (`!within_stop(n, _, s)`, `!precedes_stop(a, _, s)`,
//! `!split_dot(q, _, x)`).

mod core;
pub(in crate::code_rule_dsl) use self::core::is_core;
use self::core::UNSET;

use std::collections::{BTreeMap, BTreeSet};

use super::syntax::{Code, Fix, Head, Item, Lit, Statement, Suppress, Term as STerm, quote};
use crate::code_rule_dsl::logic::front::{AXIS, AggOp, Atom, Error, HAS, LatticeDecl, Literal, Program, Rule, Term, Value, axis};
use crate::code_rule_dsl::logic::strings::SET;

/// The text tests: their `.rr` names and the engine's.
pub(super) const TESTS: [(&str, &str); 11] = [
    ("glob", "glob"),
    ("iglob", "iglob"),
    ("prefix", "prefix"),
    ("iprefix", "iprefix"),
    ("suffix", "suffix"),
    ("isuffix", "isuffix"),
    ("contains", "contains"),
    ("icontains", "icontains"),
    ("word", "word"),
    ("iword", "iword"),
    ("match", "rx"),
];

fn test_of(rel: &str) -> Option<&'static str> {
    TESTS.iter().find(|(rr, _)| *rr == rel).map(|(_, engine)| *engine)
}

/// The whole-string regex of a pattern computed in a rule, `p`: `^(?:p)$` made as the row's pattern is.
pub(super) fn anchored(p: Term) -> Term {
    let concat = |a: Term, b: Term| Term::Call("concat".to_owned(), vec![a, b]);
    concat(Term::Str("^(?:".to_owned()), concat(p, Term::Str(")$".to_owned())))
}

/// The pattern `anchored` made `p` of, if `p` is one.
pub(super) fn unanchored(p: &Term) -> Option<&Term> {
    match p {
        Term::Call(f, args) if f == "concat" => match args.as_slice() {
            [Term::Str(open), Term::Call(g, inner)] if open == "^(?:" && g == "concat" => match inner.as_slice() {
                [p, Term::Str(close)] if close == ")$" => Some(p),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// The built-ins that take a relation, and its position.
pub(super) const RELATION_ARG: [(&str, &[usize]); 7] = [
    ("within_stop", &[2]),
    ("nearest", &[2]),
    ("precedes_stop", &[2]),
    ("inside", &[2]),
    ("has", &[2]),
    ("dfs_first", &[3, 4]),
    ("dfs_first_memo", &[3, 4, 5]),
];

fn not_yet<T>(what: impl std::fmt::Display) -> Result<T, Error> {
    Err(Error::Syntax(format!("not lowered yet: {what}")))
}

/// What a set given to an axis holds (`Lowerer::members`).
enum Members {
    /// Kinds known when the program is read: a kind set, which the transducer answers.
    Kinds(Vec<String>),
    /// Nodes (`block_node(n)`).
    Nodes,
    /// Kinds that rules derive from the tree (`doc_code_kind(k) :- kind(_, k), k != "comment"`).
    Derived,
}

/// A column type in the engine's names.
fn ty(t: &str) -> String {
    match t {
        "number" | "unsigned" => "int".to_owned(),
        "symbol" => "sym".to_owned(),
        "span" | "loc" => "node".to_owned(),
        other => other.to_owned(),
    }
}

#[derive(Default)]
struct Lowerer {
    prog: Program,
    /// `.lattice` declarations: name → (base type, its `Lub` functor).
    lattices: BTreeMap<String, (String, String)>,
    /// Relations given as a kind set.
    sets: BTreeSet<String>,
    /// The sets `precedes_stop` reads, each with its kinds when written as one kind (`next_stops`).
    stops: BTreeMap<String, Option<Vec<String>>>,
    /// `.violation`s, as `findings` makes their findings.
    violations: Vec<Violation>,
    /// The columns declared `span` or `loc` (both are `node` to the engine), and which: what `origins` reads of
    /// relations with no rules.
    locs: BTreeMap<(String, usize), String>,
    next: usize,
    /// The literals beside the one being lowered, in its conjunction and those enclosing it: what binds an
    /// aggregate's shared variables (`aggregate`).
    siblings: Vec<Lit>,
    /// Whether the clause being lowered binds a variable named `top` (`path_part(d, top)`); otherwise `top` is a
    /// lattice's top value (`v != top`), as the translated programs write it.
    top_is_var: bool,
    /// The relations the program derives (rules or facts): core.rr's names among them are the program's own.
    derived: BTreeSet<String>,
}

pub(super) fn lower(statements: Vec<Statement>) -> Result<Program, Error> {
    let mut l = Lowerer::default();
    let mut clauses = Vec::new();
    for (item, line) in statements {
        let at = |e: Error| match e {
            Error::Syntax(m) => Error::Syntax(format!("line {line}: {m}")),
            e => e,
        };
        match item {
            Item::Include(path) => return Err(Error::Syntax(format!("line {line}: #include {path:?} not resolved"))),
            Item::Decl { name, cols, tail } => l.decl(name, cols, &tail).map_err(at)?,
            Item::Edit { .. } | Item::Functor { .. } | Item::Type { .. } => {}
            // `.input r(IO=json, filename="…")`: r is an input whose rows come from that file (or artifact), kept
            // as the setting `input r(IO=…)` for the runner; without them r is empty.
            Item::Io { output, names, params } => {
                for (name, text) in params {
                    l.prog.settings.push((if output { "output" } else { "input" }.to_owned(), format!("{name}({text})")));
                }
                if output { &mut l.prog.outputs } else { &mut l.prog.inputs }.extend(names)
            }
            Item::Lattice { name, base, ops } => {
                let Some((_, STerm::Call { name: lub, functor: true, .. })) = ops.iter().find(|(op, _)| op == "Lub") else {
                    return Err(at(Error::Syntax(format!(".lattice {name} has no `Lub -> @f`"))));
                };
                l.lattices.insert(name, (ty(&base), lub.clone()));
            }
            Item::Pragma(words) => match words.as_slice() {
                [magic, relations] if magic == "magic-transform" => {
                    l.prog.demand.extend(relations.split(',').map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned));
                }
                _ => return not_yet(format!(".pragma {words:?}")).map_err(at),
            },
            Item::Policy(document) => l.prog.policies.push(document),
            Item::Setting { key, text } => l.prog.settings.push((key, text)),
            Item::Violation { name, cols, code, tag, suppress, fixes, .. } => l.violation(name, cols, code, tag, suppress, fixes, line).map_err(at)?,
            Item::Clause { heads, body, edits } => {
                if body.is_empty() && edits.is_empty() && let Some(rows) = facts(&heads) {
                    for (rel, row) in rows {
                        l.prog.facts.entry(rel).or_default().push(row);
                    }
                    continue;
                }
                clauses.push((heads, body, edits, line));
            }
        }
    }
    l.derived = clauses.iter().flat_map(|(heads, ..)| heads.iter().map(|h| h.rel.clone())).chain(l.prog.facts.keys().cloned()).collect();
    for (heads, body, edits, line) in clauses {
        // A rule ending in `=> edits` defines a fix; fixes are not run, so it is left out.
        if !edits.is_empty() {
            continue;
        }
        l.clause(&heads, &body).map_err(|e| match e {
            Error::Syntax(m) => Error::Syntax(format!("line {line}: {m}")),
            e => e,
        })?;
    }
    l.findings()?;
    l.within_kinds();
    // The built-ins lowering writes out (`nearest`, `within`, …) are declared in `core.rr` for the checker; they
    // are no relations of the program.
    for b in RELATION_ARG.iter().map(|(r, _)| *r).chain(["within", "earlier", "split_dot", "unset"]) {
        l.prog.decls.remove(b);
        l.prog.inputs.retain(|i| i != b);
    }
    l.declare_core()?;
    l.unused_core();
    l.kind_sets()?;
    l.unit();
    crate::code_rule_dsl::logic::front::finish(l.prog)
}

/// A clause of constant heads: its facts.
fn facts(heads: &[Head]) -> Option<Vec<(String, Vec<Value>)>> {
    heads
        .iter()
        .map(|h| {
            let row = h
                .args
                .iter()
                .map(|a| match a {
                    STerm::Num(n) => Some(Value::Int(*n)),
                    STerm::Str(s) => Some(Value::Str(s.clone())),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            h.modal.is_none().then(|| (h.rel.clone(), row))
        })
        .collect()
}

impl Lowerer {
    fn fresh(&mut self, stem: &str) -> String {
        self.next += 1;
        format!("rr_{stem}{}", self.next)
    }

    /// A rule that reads no relation (`ok(v) :- m = join …, v = @f(m).`, or an aggregate's helper of comparisons
    /// only, `{ e = "a", q = @json_str(e) }`) holds once, as in Soufflé: it reads `rr_unit(0)`, one row, since the
    /// planner starts every body from an atom.
    fn unit(&mut self) {
        const UNIT: &str = "rr_unit";
        let mut used = false;
        for rule in &mut self.prog.rules {
            // A matcher atom (`ast(…)`) or a walk reads the tree itself.
            let reads = |l: &Literal| matches!(l, Literal::Atom(a) if !a.neg) || matches!(l, Literal::Ast { .. } | Literal::Nearest { .. });
            if !rule.body.iter().any(reads) {
                rule.body.insert(0, Literal::Atom(Atom { rel: UNIT.to_owned(), args: vec![Term::Int(0)], neg: false, gated: false }));
                used = true;
            }
        }
        if used {
            self.prog.decls.insert(UNIT.to_owned(), vec![("x".to_owned(), "int".to_owned())]);
            self.prog.facts.insert(UNIT.to_owned(), vec![vec![Value::Int(0)]]);
        }
    }

    fn decl(&mut self, name: String, cols: Vec<(String, String)>, tail: &[String]) -> Result<(), Error> {
        let mut widen = None;
        let mut words = tail.iter();
        while let Some(w) = words.next() {
            match w.as_str() {
                "widen" => widen = Some(words.next().and_then(|k| k.parse::<u32>().ok()).ok_or_else(|| Error::Syntax(format!("{name}: `widen` needs a count")))?),
                other => return not_yet(format!("`{other}` on .decl {name}")),
            }
        }
        let mut out = Vec::new();
        for (i, (c, t)) in cols.iter().enumerate() {
            let Some(lattice) = t.strip_suffix("<>") else {
                if matches!(t.as_str(), "span" | "loc") {
                    self.locs.insert((name.clone(), i), t.clone());
                }
                out.push((c.clone(), ty(t)));
                continue;
            };
            if i + 1 != cols.len() {
                return Err(Error::Syntax(format!("{name}: the lattice column {c} must be the last")));
            }
            let (column, merge) = match self.lattices.get(lattice) {
                Some((base, lub)) => (base.clone(), Some(lub.clone())),
                None if matches!(lattice, "flat" | "min" | "max") => (lattice.to_owned(), None),
                None => return Err(Error::Syntax(format!("{name}: no .lattice {lattice}"))),
            };
            self.prog.lattice.insert(name.clone(), LatticeDecl { keys: i, ty: column.clone(), merge, widen });
            out.push((c.clone(), column));
        }
        if widen.is_some() && !self.prog.lattice.contains_key(&name) {
            return Err(Error::Syntax(format!("{name}: `widen` on a relation with no lattice column")));
        }
        self.prog.decls.insert(name, out);
        Ok(())
    }

    /// `.violation V(at, …)`: V is an output relation, and each of its rows is a finding (`findings`). Its
    /// `suppress` and `fix` lines are settings of the findings layer, as `.mode` is: `suppress V TOOL "rule", …
    /// [over {c}]` and `fix V [safe] F "title"`.
    #[allow(clippy::too_many_arguments)]
    fn violation(&mut self, name: String, cols: Vec<(String, String)>, code: Option<Code>, tag: Option<String>, suppress: Vec<Suppress>, fixes: Vec<Fix>, line: usize) -> Result<(), Error> {
        let Some((_, at)) = cols.first() else { return Err(Error::Syntax(format!(".violation {name} has no `at` column"))) };
        if !matches!(at.as_str(), "node" | "span" | "loc") {
            return Err(Error::Syntax(format!(".violation {name}: `at` is a node, a span or a loc, not a {at}")));
        }
        let at = at.clone();
        // `tag {t}` keeps findings at one place apart; the engine's findings are keyed by place and code, so
        // it is checked and not carried.
        let column = match &code {
            Some(Code::Column(c)) => Some(c),
            _ => None,
        };
        for c in [column, tag.as_ref()].into_iter().flatten().chain(suppress.iter().filter_map(|s| s.over.as_ref())) {
            if !cols.iter().any(|(n, _)| n == c) {
                return Err(Error::Syntax(format!(".violation {name}: no column {c}")));
            }
        }
        let code = match code {
            Some(Code::Column(c)) => FindingCode::Column(cols.iter().position(|(n, _)| *n == c).expect("a column")),
            Some(Code::Const(c)) => FindingCode::Const(c),
            None => FindingCode::Const(name.clone()),
        };
        for s in suppress {
            let over = s.over.map(|c| format!(" over {{{c}}}")).unwrap_or_default();
            self.prog.settings.push(("suppress".to_owned(), format!("{name} {} {}{over}", s.tool, s.rules.iter().map(|r| quote(r)).collect::<Vec<_>>().join(", "))));
        }
        for f in fixes {
            self.prog.settings.push(("fix".to_owned(), format!("{name} {}{} {}", if f.safe { "safe " } else { "" }, f.name, quote(&f.title))));
        }
        self.violations.push(Violation { name: name.clone(), arity: cols.len(), code, at, line });
        self.decl(name.clone(), cols, &[])?;
        self.prog.outputs.push(name);
        Ok(())
    }

    /// The findings of the `.violation`s: `violation(policy, V, code, path, line)` for each row of V (the engine's
    /// findings relation, as the converter's programs give it). A node `at` is found at its document's path and its
    /// line; a `@place(path, line)` at that path and line (`PLACE`). A `span` or `loc` column whose values can be
    /// anything else (a regex capture's span, a token: spans the engine does not make) is refused by name.
    fn findings(&mut self) -> Result<(), Error> {
        if self.violations.is_empty() {
            return Ok(());
        }
        let policy = self
            .prog
            .settings
            .iter()
            .find(|(k, _)| k == "policy")
            .and_then(|(_, text)| text.split('"').nth(1))
            .unwrap_or_default()
            .to_owned();
        self.prog.decls.insert("violation".to_owned(), [("rule", "sym"), ("clause", "sym"), ("code", "sym"), ("path", "sym"), ("line", "int")].iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
        self.prog.outputs.retain(|o| o != "violation");
        self.prog.outputs.push("violation".to_owned());
        // Without a `@place` or a `span`/`loc` at, every finding is at a node.
        let places = self.violations.iter().any(|v| v.at != "node") || self.prog.rules.iter().any(|r| r.head.args.iter().chain(r.body.iter().flat_map(literal_terms)).any(has_place));
        let origins = if places { self.origins() } else { BTreeMap::new() };
        let v = |s: &str| Term::Var(s.to_owned());
        for Violation { name, arity, code, at, line } in std::mem::take(&mut self.violations) {
            let origin = origins.get(&(name.clone(), 0)).cloned().unwrap_or_default();
            if at != "node" && let Some(why) = &origin.other {
                return Err(Error::Syntax(format!("line {line}: not lowered yet: .violation {name} at a {at} that can be a span the engine does not make ({why})")));
            }
            let code_col = match &code {
                FindingCode::Column(i) => Some(*i),
                FindingCode::Const(_) => None,
            };
            let cols: Vec<Term> = (0..arity).map(|i| if i == 0 || Some(i) == code_col { v(&format!("rr_v{i}")) } else { Term::Wild }).collect();
            let code = match code {
                FindingCode::Column(i) => v(&format!("rr_v{i}")),
                FindingCode::Const(c) => Term::Str(c),
            };
            let atom = |rel: &str, args: Vec<Term>| Literal::Atom(Atom { rel: rel.to_owned(), args, neg: false, gated: false });
            let head = |path: Term, line: Term| Atom { rel: "violation".to_owned(), args: vec![Term::Str(policy.clone()), Term::Str(name.clone()), code.clone(), path, line], neg: false, gated: false };
            if at == "node" || origin.node {
                self.prog.rules.push(Rule {
                    head: head(v("rr_p"), v("rr_l")),
                    body: vec![
                        atom(&name, cols.clone()),
                        atom("node", vec![v("rr_d"), v("rr_v0"), Term::Wild, Term::Wild, Term::Wild]),
                        atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild]),
                        atom("line", vec![v("rr_v0"), v("rr_l")]),
                    ],
                });
            }
            if origin.place {
                let of = |f: &str| Term::Call(f.to_owned(), vec![v("rr_v0")]);
                self.prog.rules.push(Rule {
                    head: head(of(PLACE_PATH), of(PLACE_LINE)),
                    body: vec![atom(&name, cols), Literal::Cmp { l: v("rr_v0"), op: "<", r: Term::Int(UNSET) }],
                });
            }
        }
        Ok(())
    }

    /// What each column of a derived relation can hold, of the kinds a `loc` has: nodes, `@place`s, and anything
    /// else (with what made it). A rule's variable holds what every positive literal binding it allows (the
    /// meet); a relation's column what any rule's head puts there (the join), to a fixpoint. A relation with no
    /// rules holds what its declared type says: a `node` column nodes, a `span` column spans, a `loc` either.
    fn origins(&self) -> BTreeMap<(String, usize), Origin> {
        let derived: BTreeSet<&str> = self.prog.rules.iter().map(|r| r.head.rel.as_str()).chain(self.prog.facts.keys().map(String::as_str)).collect();
        let mut map: BTreeMap<(String, usize), Origin> = BTreeMap::new();
        // A fact's value is a constant, but `unset`'s (core.rr's `rr_core_unset`, -1) is no value: it has no finding.
        for (rel, rows) in &self.prog.facts {
            for row in rows {
                for (c, v) in row.iter().enumerate() {
                    let o = if *v == Value::Int(UNSET) { Origin::default() } else { Origin::other(format!("a fact of {rel}")) };
                    map.entry((rel.clone(), c)).or_default().join(&o);
                }
            }
        }
        let base = |rel: &str, c: usize| -> Origin {
            match self.locs.get(&(rel.to_owned(), c)).map(String::as_str) {
                Some("span") => Origin::other(format!("{rel}, a span")),
                Some(_) => Origin { node: true, ..Origin::other(format!("{rel}, a node or a span")) },
                None => match self.prog.decls.get(rel).and_then(|cols| cols.get(c)) {
                    Some((_, t)) if t == "node" => Origin { node: true, ..Origin::default() },
                    Some((_, t)) => Origin::other(format!("a {t} of {rel}")),
                    None if rel.starts_with(AXIS) => Origin { node: true, ..Origin::default() },
                    None => Origin::other(format!("{rel}, not declared")),
                },
            }
        };
        loop {
            let mut changed = false;
            for rule in &self.prog.rules {
                let column = |rel: &str, c: usize| if derived.contains(rel) { map.get(&(rel.to_owned(), c)).cloned().unwrap_or_default() } else { base(rel, c) };
                let vars = rule_origins(&rule.body, &column);
                let heads: Vec<Origin> = rule.head.args.iter().map(|t| term_origin(t, &vars).unwrap_or_else(|| Origin::other("an unbound variable".to_owned()))).collect();
                for (c, o) in heads.into_iter().enumerate() {
                    changed |= map.entry((rule.head.rel.clone(), c)).or_default().join(&o);
                }
            }
            if !changed {
                return map;
            }
        }
    }

    fn clause(&mut self, heads: &[Head], body: &[Lit]) -> Result<(), Error> {
        fn binds_top(lits: &[Lit]) -> bool {
            lits.iter().any(|l| match l {
                Lit::Atom { neg: false, args, .. } => args.iter().any(|a| matches!(a, STerm::Var(v) if v == "top")),
                Lit::Group { alts, neg: false } => alts.iter().any(|a| binds_top(a)),
                _ => false,
            })
        }
        self.top_is_var = binds_top(body);
        let (heads, body) = self.lift_head_aggregates(heads, body);
        let (heads, body) = (heads.as_slice(), body.as_slice());
        let mut outer = Vec::new();
        heads.iter().flat_map(|h| &h.args).for_each(|a| a.vars(&mut outer));
        let bodies = self.conjunction(body, &outer.into_iter().collect())?;
        for head in heads {
            if let Some(m) = &head.modal {
                return not_yet(format!("`{m}` head {}", head.rel));
            }
            let args = head.args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
            for body in &bodies {
                self.prog.rules.push(Rule { head: Atom { rel: head.rel.clone(), args: args.clone(), neg: false, gated: false }, body: body.clone() });
            }
        }
        Ok(())
    }

    /// A conjunction's alternatives (one per choice in its disjunctions).
    /// `context`: the variables used outside the conjunction.
    fn conjunction(&mut self, lits: &[Lit], context: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        let lits = &self.lift_aggregates(lits);
        let mut acc = vec![Vec::new()];
        for (k, lit) in lits.iter().enumerate() {
            // The variables used outside this literal, which an aggregate inside it is correlated on: another
            // aggregate's body's are that aggregate's own, as in Soufflé.
            let mut outer: Vec<String> = context.iter().cloned().collect();
            lits.iter().enumerate().filter(|(j, _)| *j != k).for_each(|(_, l)| l.scope_vars(&mut outer));
            let enclosing = self.siblings.clone();
            self.siblings = lits.iter().enumerate().filter(|(j, _)| *j != k).map(|(_, l)| l.clone()).chain(enclosing.iter().cloned()).collect();
            let alts = self.literal(lit, &outer.into_iter().collect());
            self.siblings = enclosing;
            let alts = alts?;
            acc = acc.iter().flat_map(|a| alts.iter().map(move |b| a.iter().chain(b).cloned().collect::<Vec<_>>())).collect();
        }
        Ok(acc.into_iter().map(merge_negated_tests).collect())
    }

    fn literal(&mut self, lit: &Lit, outer: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        Ok(match lit {
            Lit::Group { alts, neg } => {
                if let Some(Literal::Cmp { l, op, .. }) = self.test_set(alts, *neg)? {
                    return Ok(vec![vec![Literal::Cmp { l, op, r: Term::Int(if *neg { 0 } else { 1 }) }]]);
                }
                if *neg {
                    return self.negated(alts, outer);
                }
                let mut out = Vec::new();
                for alt in alts {
                    out.extend(self.conjunction(alt, outer)?);
                }
                out
            }
            Lit::Pattern { pattern, .. } => return not_yet(format!("the pattern `{pattern}`")),
            Lit::Atom { rel, args, neg } if rel == "within_stop" => self.within(rel, args, *neg)?,
            Lit::Atom { rel, args, neg: true } if rel == "nearest" && matches!(args.as_slice(), [_, a, _] if *a != STerm::Wild) => self.not_nearest(args)?,
            Lit::Atom { rel, args, neg } if rel == "precedes_stop" => self.precedes_stop(args, *neg)?,
            Lit::Atom { rel, args, neg } if rel == "split_dot" => self.split_dot(args, *neg)?,
            Lit::Atom { rel, args, neg: false } if rel == "match" && matches!(args.as_slice(), [STerm::Str(re), _] if !groups(re).is_empty()) => self.captures(args)?,
            Lit::Atom { rel, args, neg } => {
                if let Some(literals) = self.rewrite(rel, args, *neg)? {
                    return Ok(vec![literals]);
                }
                // A computed argument (`loads(@dot(m, x))`) is bound to a variable first: the engine joins on
                // variables (`front::finish`'s computed-term check).
                let mut atom = self.atom(rel, args, *neg)?;
                let mut out = Vec::new();
                if let Literal::Atom(a) = &mut atom {
                    for t in &mut a.args {
                        if matches!(t, Term::Call(..)) {
                            let v = self.fresh("t");
                            out.push(Literal::Cmp { l: Term::Var(v.clone()), op: "=", r: std::mem::replace(t, Term::Var(v)) });
                        }
                    }
                }
                out.push(atom);
                vec![out]
            }
            Lit::Cmp { l, op, r } => self.comparison(l, op, r, outer)?,
        })
    }

    /// `within(n, a)`: n is strictly below a. A node is `d << 32 | its preorder index` and its subtree the ids
    /// `a + 1 ..= a + size` (`facts::vtab::syntax`'s `tape`), so it is a range over ids: `tape(a, _, z, _),
    /// n > a, n <= a + z`, which the tape's lookups answer for a bound `a`. `within_stop(n, a, stop)`: also no
    /// node of `stop` strictly between them. The ancestors of n are ordered by id, so that is: n has no
    /// ancestor in `stop`, or its nearest one is a or above it. Negated (a named): n is outside a's range, or
    /// (`!within_stop`) inside it with its nearest ancestor in `stop` below a.
    fn within(&mut self, rel: &str, args: &[STerm], neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        let (n, a, stop) = match args {
            [n, a] if rel == "within" => (n, a, None),
            [n, a, stop] if rel == "within_stop" => (n, a, Some(stop)),
            _ => return not_yet(format!("{}{rel}/{}", if neg { "!" } else { "" }, args.len())),
        };
        if neg && *a == STerm::Wild {
            return not_yet(format!("`!{rel}` with its ancestor unnamed"));
        }
        let (n, a) = (self.term(n)?, self.term(a)?);
        self.base("tape");
        let z = Term::Var(self.fresh("size"));
        let tape = Literal::Atom(Atom { rel: "tape".to_owned(), args: vec![a.clone(), Term::Wild, z.clone(), Term::Wild], neg: false, gated: false });
        let end = Term::Call("add".to_owned(), vec![a.clone(), z]);
        let cmp = |l: &Term, op: &'static str, r: &Term| Literal::Cmp { l: l.clone(), op, r: r.clone() };
        if neg {
            let mut out = vec![vec![tape.clone(), cmp(&n, "<=", &a)], vec![tape.clone(), cmp(&n, ">", &end)]];
            if let Some(stop) = stop {
                let set = self.set_name("within_stop", stop)?;
                let Term::Var(nv) = &n else { return Err(Error::Syntax("within_stop: the node is a variable".to_owned())) };
                let s = self.fresh("stop");
                out.push(vec![tape, cmp(&n, ">", &a), cmp(&n, "<=", &end), Literal::Nearest { n: nv.clone(), a: s.clone(), kindset: set }, cmp(&Term::Var(s), ">", &a)]);
            }
            return Ok(out);
        }
        let below = match (&n, &a) {
            // Nothing else gives n: the nodes below a are listed, following `child` down from the a's the rule
            // gives (work in proportion to the nodes found; there is no range read of ids yet).
            (Term::Var(nv), Term::Var(av)) if !self.bound_beside(nv) => {
                let ctx = self.context(std::slice::from_ref(av))?;
                let rel = self.fresh("below");
                self.prog.decls.insert(rel.clone(), vec![("a".to_owned(), "node".to_owned()), ("n".to_owned(), "node".to_owned())]);
                let v = |x: &str| Term::Var(x.to_owned());
                let atom = |r: &str, args: Vec<Term>| Literal::Atom(Atom { rel: r.to_owned(), args, neg: false, gated: false });
                let head = |args: Vec<Term>| Atom { rel: rel.clone(), args, neg: false, gated: false };
                let mut seed = ctx.clone();
                seed.args = vec![v("a")];
                self.prog.rules.push(Rule { head: head(vec![v("a"), v("n")]), body: vec![Literal::Atom(seed), atom("child", vec![v("a"), v("n"), Term::Wild])] });
                self.prog.rules.push(Rule { head: head(vec![v("a"), v("n")]), body: vec![atom(&rel, vec![v("a"), v("m")]), atom("child", vec![v("m"), v("n"), Term::Wild])] });
                vec![Literal::Atom(ctx), atom(&rel, vec![a.clone(), n.clone()])]
            }
            _ => vec![tape, cmp(&n, ">", &a), cmp(&n, "<=", &end)],
        };
        let Some(stop) = stop else { return Ok(vec![below]) };
        let set = self.set_name("within_stop", stop)?;
        let Term::Var(nv) = &n else { return Err(Error::Syntax("within_stop: the node is a variable".to_owned())) };
        let s = self.fresh("stop");
        let none = Literal::Atom(Atom { rel: format!("{AXIS}{set}"), args: vec![n.clone(), Term::Wild], neg: true, gated: false });
        let nearest = Literal::Nearest { n: nv.clone(), a: s.clone(), kindset: set };
        Ok(vec![
            below.iter().cloned().chain([none]).collect(),
            below.into_iter().chain([nearest, cmp(&Term::Var(s), "<=", &a)]).collect(),
        ])
    }


    /// `match("(?<x>…)", s)`: s matches, and each named group binds the variable of its name to its text:
    /// `x = @rx_group(s, "^(?:…)$", "x")`, which has no value when s does not match (`strings::group`).
    fn captures(&mut self, args: &[STerm]) -> Result<Vec<Vec<Literal>>, Error> {
        let [STerm::Str(re), s] = args else { unreachable!("match with a pattern") };
        let whole = Term::Str(format!("^(?:{re})$"));
        let s = self.term(s)?;
        let mut out = Vec::new();
        for g in groups(re) {
            out.push(Literal::Cmp { l: Term::Var(g.clone()), op: "=", r: Term::Call("rx_group".to_owned(), vec![s.clone(), whole.clone(), Term::Str(g)]) });
        }
        Ok(vec![out])
    }

    /// `(glob("a", s) ; glob("b", s))` as one test of the set, and `(s = "a" ; s = "b")` as one membership. `neg`: the
    /// disjunction is negated.
    fn test_set(&mut self, alts: &[Vec<Lit>], neg: bool) -> Result<Option<Literal>, Error> {
        if alts.len() < 2 || alts.iter().any(|a| a.len() != 1) {
            return Ok(None);
        }
        let one = |a: &Vec<Lit>| a[0].clone();
        let tests: Option<Vec<(&'static str, STerm, String)>> = alts
            .iter()
            .map(|a| match one(a) {
                Lit::Atom { rel, args, neg: false } => match (test_of(&rel), args.as_slice()) {
                    (Some(op), [STerm::Str(p), s]) if op != "rx" => Some((op, s.clone(), p.clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        if let Some(tests) = tests
            && tests.iter().all(|(op, s, _)| *op == tests[0].0 && *s == tests[0].1)
        {
            let s = self.term(&tests[0].1)?;
            let set = Term::Call(SET.to_owned(), tests.iter().map(|(_, _, p)| Term::Str(p.clone())).collect());
            return Ok(Some(Literal::Cmp { l: Term::Call(tests[0].0.to_owned(), vec![s, set]), op: "=", r: Term::Int(1) }));
        }
        let members: Option<Vec<(String, String)>> = alts
            .iter()
            .map(|a| match one(a) {
                Lit::Cmp { l: STerm::Var(x), op: "=", r: STerm::Str(c) } | Lit::Cmp { l: STerm::Str(c), op: "=", r: STerm::Var(x) } => Some((x, c)),
                _ => None,
            })
            .collect();
        // Only a test when something else binds x: `kind(c, k), (x = "a" ; x = "b")` gives x each value. Negated it
        // binds nothing, so it is a test (`key = cat(q, s), !(key = "a" ; key = "b")`).
        if let Some(members) = members
            && members.iter().all(|(x, _)| *x == members[0].0)
            && (neg || self.bound_beside(&members[0].0))
        {
            let set = Term::Call(SET.to_owned(), members.iter().map(|(_, c)| Term::Str(c.clone())).collect());
            return Ok(Some(Literal::Cmp { l: Term::Call("equals".to_owned(), vec![Term::Var(members[0].0.clone()), set]), op: "=", r: Term::Int(1) }));
        }
        Ok(None)
    }

    /// A set argument's relation: a relation's name (kinds or nodes, `kind_sets` tells), or one kind as its own set.
    fn set_name(&mut self, rel: &str, arg: &STerm) -> Result<String, Error> {
        match arg {
            STerm::Var(name) => {
                self.sets.insert(name.clone());
                Ok(name.clone())
            }
            STerm::Str(kind) => {
                let name = kind_set_name(kind);
                self.prog.kindsets.insert(name.clone(), vec![kind.clone()]);
                Ok(name)
            }
            _ => Err(Error::Syntax(format!("{rel}: the set is a relation of kinds or of nodes, or one kind"))),
        }
    }

    /// A base relation lowering reads (`tape`, `child`, `node`), declared, and an input, when the program has not.
    fn base(&mut self, rel: &str) {
        let cols: &[(&str, &str)] = match rel {
            "tape" => &[("n", "int"), ("par", "int"), ("size", "int"), ("depth", "int")],
            "child" => &[("p", "node"), ("c", "node"), ("i", "int")],
            "node" => &[("d", "int"), ("n", "node"), ("kind", "sym"), ("start", "int"), ("end", "int")],
            other => unreachable!("no base relation {other}"),
        };
        self.prog.decls.entry(rel.to_owned()).or_insert_with(|| cols.iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
        if !self.prog.inputs.iter().any(|i| i == rel) && !self.prog.rules.iter().any(|r| r.head.rel == rel) {
            self.prog.inputs.push(rel.to_owned());
        }
    }

    /// `!nearest(n, a, set)` with a named: a is not n's nearest ancestor in the set, so n has none, or another.
    fn not_nearest(&mut self, args: &[STerm]) -> Result<Vec<Vec<Literal>>, Error> {
        let [STerm::Var(n), a, set] = args else { return Err(Error::Syntax("nearest: the node is a variable".to_owned())) };
        let set = self.set_name("nearest", set)?;
        let a = self.term(a)?;
        let s = self.fresh("near");
        Ok(vec![
            vec![Literal::Atom(Atom { rel: format!("{AXIS}{set}"), args: vec![Term::Var(n.clone()), Term::Wild], neg: true, gated: false })],
            vec![Literal::Nearest { n: n.clone(), a: s.clone(), kindset: set }, Literal::Cmp { l: Term::Var(s), op: "!=", r: a }],
        ])
    }

    /// `split_dot(q, m, x)`: q is m.x split at its last dot, so x has no dot: `q = @cat(m, x), !contains(".", x)`
    /// (a bound q splits at every dot, `@cat` run backwards, which takes variables: a constant is bound to one
    /// first). Negated, every column named: q is not m.x, or x has a dot.
    fn split_dot(&mut self, args: &[STerm], neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        if args.len() != 3 {
            return not_yet(format!("split_dot/{}", args.len()));
        }
        if neg && args.contains(&STerm::Wild) {
            return not_yet("`!split_dot` with a column unnamed");
        }
        let (mut out, mut terms) = (Vec::new(), Vec::new());
        for a in args {
            let t = self.term(a)?;
            terms.push(match t {
                Term::Var(_) => t,
                _ if neg => t,
                Term::Wild => Term::Var(self.fresh("part")),
                constant => {
                    let v = Term::Var(self.fresh("part"));
                    out.push(Literal::Cmp { l: v.clone(), op: "=", r: constant });
                    v
                }
            });
        }
        let [q, m, x] = <[Term; 3]>::try_from(terms).expect("three columns");
        let dotted = |b: i64| Literal::Cmp { l: Term::Call("contains".to_owned(), vec![x.clone(), Term::Str(".".to_owned())]), op: "=", r: Term::Int(b) };
        let cat = Term::Call("cat".to_owned(), vec![m, x.clone()]);
        if neg {
            return Ok(vec![vec![Literal::Cmp { l: q, op: "!=", r: cat }], vec![dotted(1)]]);
        }
        out.extend([Literal::Cmp { l: q, op: "=", r: cat }, dotted(0)]);
        Ok(vec![out])
    }

    /// `precedes_stop(a, b, stop)`: a comes before b among one parent's children (`precedes`), and no child of
    /// `stop` is strictly between them. `rr_next_stop_<stop>(a, k)` is the index of the first child of `stop`
    /// after a (`next_stops`), so that is: there is none, or b is at or before it. Negated (a and b named): a is
    /// no child, b is not a's sibling or not after it, or a child of `stop` is between them.
    fn precedes_stop(&mut self, args: &[STerm], neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        let [a, b, stop] = args else { return not_yet(format!("precedes_stop/{}", args.len())) };
        if neg && (*a == STerm::Wild || *b == STerm::Wild) {
            return not_yet("`!precedes_stop` with a node unnamed");
        }
        let set = match stop {
            STerm::Var(name) => {
                self.stops.entry(name.clone()).or_insert(None);
                name.clone()
            }
            STerm::Str(kind) => {
                let name = kind_set_name(kind);
                self.stops.insert(name.clone(), Some(vec![kind.clone()]));
                name
            }
            _ => return Err(Error::Syntax("precedes_stop: the set is a relation of kinds or of nodes, or one kind".to_owned())),
        };
        let node = |t: &STerm, l: &mut Self| if *t == STerm::Wild { Ok(Term::Var(l.fresh("sib"))) } else { l.term(t) };
        let (a, b) = (node(a, self)?, node(b, self)?);
        self.base("child");
        let next = format!("rr_next_stop_{set}");
        self.prog.decls.entry(next.clone()).or_insert_with(|| vec![("a".to_owned(), "node".to_owned()), ("k".to_owned(), "int".to_owned())]);
        let [p, i, j, k] = ["p", "i", "j", "k"].map(|s| Term::Var(self.fresh(s)));
        let atom = |rel: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: rel.to_owned(), args, neg, gated: false });
        let cmp = |l: &Term, op: &'static str, r: &Term| Literal::Cmp { l: l.clone(), op, r: r.clone() };
        let pair = [atom("child", vec![p.clone(), a.clone(), i.clone()], false), atom("child", vec![p.clone(), b.clone(), j.clone()], false)];
        let with = |more: Vec<Literal>| pair.iter().cloned().chain(more).collect::<Vec<_>>();
        if neg {
            return Ok(vec![
                vec![atom("child", vec![Term::Wild, a.clone(), Term::Wild], true)],
                vec![atom("child", vec![p.clone(), a.clone(), Term::Wild], false), atom("child", vec![p.clone(), b.clone(), Term::Wild], true)],
                with(vec![cmp(&i, ">=", &j)]),
                with(vec![cmp(&i, "<", &j), atom(&next, vec![a.clone(), k.clone()], false), cmp(&k, "<", &j)]),
            ]);
        }
        Ok(vec![
            with(vec![cmp(&i, "<", &j), atom(&next, vec![a.clone(), Term::Wild], true)]),
            with(vec![cmp(&i, "<", &j), atom(&next, vec![a.clone(), k.clone()], false), cmp(&j, "<=", &k)]),
        ])
    }

    fn atom(&mut self, rel: &str, args: &[STerm], neg: bool) -> Result<Literal, Error> {
        let var = |t: &STerm| match t {
            STerm::Var(v) => Ok(v.clone()),
            _ => Err(Error::Syntax(format!("{rel}: expected a variable, got {t:?}"))),
        };
        if let Some(op) = test_of(rel) {
            let [p, s] = args else { return Err(Error::Syntax(format!("{rel} takes a pattern and a string"))) };
            if op == "rx" && let STerm::Str(re) = p && (re.contains("(?<") || re.contains("(?P<")) {
                return not_yet(format!("match with named groups, {re:?}, here (`Lowerer::captures` lowers it in a body)"));
            }
            let (s, mut p) = (self.term(s)?, self.term(p)?);
            // `match` is whole-string, as in Soufflé; the engine's `rx` searches. A pattern computed in the rule is
            // anchored as it is made (`anchored`), and tested per row (`strings::test`, each distinct pair once).
            if op == "rx" {
                p = match p {
                    Term::Str(re) => Term::Str(format!("^(?:{re})$")),
                    p => anchored(p),
                };
            }
            return Ok(Literal::Cmp { l: Term::Call(op.to_owned(), vec![s, p]), op: "=", r: Term::Int(if neg { 0 } else { 1 }) });
        }
        match (rel, args) {
            // `nearest(n, _, set)`: n has an ancestor in the set, which is `inside(n, _, set)`; and negated.
            ("nearest", [n, STerm::Wild, set]) => {
                let set = self.set_name(rel, set)?;
                return Ok(Literal::Atom(Atom { rel: format!("{AXIS}{set}"), args: vec![self.term(n)?, Term::Wild], neg, gated: false }));
            }
            ("nearest", [n, a, set]) if !neg => {
                let kindset = self.set_name(rel, set)?;
                return Ok(Literal::Nearest { n: var(n)?, a: var(a)?, kindset });
            }
            ("inside", [n, a, set]) => {
                let set = self.set_name(rel, set)?;
                return Ok(Literal::Atom(Atom { rel: format!("{AXIS}{set}"), args: vec![self.term(n)?, self.term(a)?], neg, gated: false }));
            }
            // `has(a, n, set)`: n, of the set, is strictly below a (the set is tested on n, decision 0036).
            ("has", [a, n, set]) => {
                let set = self.set_name(rel, set)?;
                return Ok(Literal::Atom(Atom { rel: format!("{HAS}{set}"), args: vec![self.term(n)?, self.term(a)?], neg, gated: false }));
            }
            ("ast", _) if !neg => return self.ast(args),
            _ => {}
        }
        if let Some((_, _)) = RELATION_ARG.iter().find(|(r, _)| *r == rel) {
            return not_yet(format!("the built-in {rel}"));
        }
        let args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
        Ok(Literal::Atom(Atom { rel: rel.to_owned(), args, neg, gated: false }))
    }

    /// `ast(d, n, { … }, NAME: v, …)`, or `ast(n, { … }, …)` with its document left unnamed.
    fn ast(&mut self, args: &[STerm]) -> Result<Literal, Error> {
        let positional: Vec<&STerm> = args.iter().filter(|a| !matches!(a, STerm::Named(..))).collect();
        let (document, node, matcher) = match positional.as_slice() {
            [STerm::Var(d), STerm::Var(n), STerm::Json(m)] => (d.clone(), n.clone(), m.clone()),
            [STerm::Var(n), STerm::Json(m)] => (self.fresh("d"), n.clone(), m.clone()),
            _ => return Err(Error::Syntax("ast takes (document, node, { … }) or (node, { … })".to_owned())),
        };
        let captures = args
            .iter()
            .filter_map(|a| match a {
                STerm::Named(name, v) => Some(match v.as_ref() {
                    STerm::Var(v) => Ok((name.clone(), v.clone())),
                    other => Err(Error::Syntax(format!("ast capture {name}: expected a variable, got {other:?}"))),
                }),
                _ => None,
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Literal::Ast { document, node, matcher, captures })
    }

    fn comparison(&mut self, l: &STerm, op: &'static str, r: &STerm, outer: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        match (l, r) {
            (STerm::Var(v), STerm::Agg { op: agg, over, by, sep, body }) | (STerm::Agg { op: agg, over, by, sep, body }, STerm::Var(v)) if op == "=" => {
                Ok(vec![self.aggregate(v, agg, over.as_deref(), by, sep, body, outer)?])
            }
            (STerm::Agg { op: agg, over, by, sep, body }, other) | (other, STerm::Agg { op: agg, over, by, sep, body }) => {
                let v = self.fresh("agg");
                let mut out = self.aggregate(&v, agg, over.as_deref(), by, sep, body, outer)?;
                let other = self.term(other)?;
                // `$v = 0` with `$v` unbound would assign it (`front::cmp_binds`), and the planner may place it
                // before the aggregate: an equality is written `0 = $v`, a test that waits for the aggregate.
                let (l, r) = if matches!(l, STerm::Agg { .. }) && op != "=" { (Term::Var(v), other) } else { (other, Term::Var(v)) };
                out.push(Literal::Cmp { l, op, r });
                Ok(vec![out])
            }
            _ => Ok(vec![vec![Literal::Cmp { l: self.term(l)?, op, r: self.term(r)? }]]),
        }
    }

    /// `!(a, b)` and `!(a ; b)`. `!(a ; b)` is `!(a), !(b)`, and each negated conjunction is a helper relation of
    /// the variables it shares with the rule (`outer`), negated: `!(option("x", "true"), afterthought(u))` is
    /// `!rr_not1(u)` with `rr_not1(u) :- option("x", "true"), afterthought(u)`. A variable only the conjunction
    /// has is its own (Soufflé's scoping: there is no such row). A shared variable it only tests is bound inside
    /// the helper by the rule's other literals (`context`), as an aggregate's is. With no variable shared, the
    /// helper has one constant column (a relation has columns).
    fn negated(&mut self, alts: &[Vec<Lit>], outer: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        let siblings = self.siblings.clone();
        let mut out = Vec::new();
        for alt in alts {
            self.siblings = siblings.clone();
            // An aggregate inside the conjunction correlates on the rule's variables too (`aggregate`'s `outer`).
            let cols: Vec<String> = shared_vars(alt).into_iter().filter(|v| outer.contains(v)).collect();
            let mut bodies = self.conjunction(alt, outer)?;
            let bound: BTreeSet<String> = body_vars(&alt.iter().filter(|l| binds_columns(l)).cloned().collect::<Vec<_>>());
            let needed: Vec<String> = cols.iter().filter(|v| !bound.contains(*v)).cloned().collect();
            if !needed.is_empty() {
                let ctx = self.context(&needed)?;
                for b in &mut bodies {
                    b.insert(0, Literal::Atom(ctx.clone()));
                }
            }
            let name = self.fresh("not");
            let (decl, args): (Vec<(String, String)>, Vec<Term>) = if cols.is_empty() {
                (vec![("c".to_owned(), "int".to_owned())], vec![Term::Int(0)])
            } else {
                (cols.iter().map(|c| (c.clone(), self.type_of(c, &bodies))).collect(), cols.iter().map(|c| Term::Var(c.clone())).collect())
            };
            self.prog.decls.insert(name.clone(), decl);
            let head = Atom { rel: name, args, neg: false, gated: false };
            for b in bodies {
                self.prog.rules.push(Rule { head: head.clone(), body: b });
            }
            out.push(Literal::Atom(Atom { neg: true, ..head }));
        }
        self.siblings = siblings;
        Ok(vec![out])
    }

    /// A relation of `vars`' values in the rule: the literals beside the one being lowered (`siblings`; their
    /// atoms and comparisons), projected on `vars`. It binds, inside a helper rule, variables its body only tests.
    fn context(&mut self, vars: &[String]) -> Result<Atom, Error> {
        let context: Vec<Lit> = std::mem::take(&mut self.siblings)
            .into_iter()
            .filter(|l| match l {
                Lit::Atom { neg, .. } => !neg,
                Lit::Cmp { l, r, .. } => !matches!(l, STerm::Agg { .. }) && !matches!(r, STerm::Agg { .. }),
                _ => false,
            })
            .collect();
        // A comparison stays only if the context binds what it reads (`k > 1` needs the count that binds k, which
        // is left out): the context may hold more rows than the rule, and the rule joins it with its own literals.
        let binds = |l: &Lit| matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none() && !matches!(rel.as_str(), "within" | "within_stop" | "earlier"));
        let mut bound = body_vars(&context.iter().filter(|l| binds(l)).cloned().collect::<Vec<_>>());
        let (atoms, mut cmps): (Vec<Lit>, Vec<Lit>) = context.into_iter().partition(|l| matches!(l, Lit::Atom { .. }));
        let mut context = atoms;
        loop {
            let ready = cmps.iter().position(|c| {
                let Lit::Cmp { l, op, r } = c else { return false };
                let reads = |t: &STerm| body_vars(&[Lit::Cmp { l: t.clone(), op: "=", r: STerm::Wild }]);
                match (l, r) {
                    (STerm::Var(x), t) | (t, STerm::Var(x)) if *op == "=" && !bound.contains(x) => reads(t).is_subset(&bound),
                    _ => reads(l).union(&reads(r)).all(|v| bound.contains(v)),
                }
            });
            let Some(k) = ready else { break };
            let c = cmps.remove(k);
            bound.extend(body_vars(std::slice::from_ref(&c)));
            context.push(c);
        }
        let ctx = self.fresh("ctx");
        let alts = self.conjunction(&context, &vars.iter().cloned().collect())?;
        let types = vars.iter().map(|c| (c.clone(), self.type_of(c, &alts))).collect();
        self.prog.decls.insert(ctx.clone(), types);
        let head = Atom { rel: ctx, args: vars.iter().map(|c| Term::Var(c.clone())).collect(), neg: false, gated: false };
        for b in alts {
            self.prog.rules.push(Rule { head: head.clone(), body: b });
        }
        Ok(head)
    }

    /// Whether a positive atom beside the literal being lowered (not a text test, `within` or `earlier`) binds `v`.
    fn bound_beside(&self, v: &str) -> bool {
        let atoms: Vec<Lit> = self
            .siblings
            .iter()
            .filter(|l| matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none() && !matches!(rel.as_str(), "within" | "within_stop" | "earlier")))
            .cloned()
            .collect();
        body_vars(&atoms).contains(v)
    }

    #[allow(clippy::too_many_arguments)]
    fn aggregate(&mut self, var: &str, agg: &str, over: Option<&STerm>, by: &[STerm], sep: &Option<String>, body: &[Lit], outer: &BTreeSet<String>) -> Result<Vec<Literal>, Error> {
        let mut op = match agg {
            "count" => AggOp::Count,
            "min" => AggOp::Min,
            "max" => AggOp::Max,
            "sum" => AggOp::Sum,
            // Its order (`by`) is set below, once each key is a variable; with no `sep`, the texts are joined as they
            // are.
            "join" => AggOp::Join { by: Vec::new(), sep: sep.clone().unwrap_or_default() },
            // Soufflé's mean is a decimal number, and the engine has none.
            "mean" => return not_yet("the aggregate mean (a decimal number)"),
            other => return not_yet(format!("the aggregate {other}")),
        };
        if !matches!(op, AggOp::Join { .. }) && (!by.is_empty() || sep.is_some()) {
            return Err(Error::Syntax(format!("`by` and `sep` are join's, not {agg}'s")));
        }
        if op != AggOp::Count && over.is_none() {
            return Err(Error::Syntax(format!("{agg} needs the variable it aggregates")));
        }
        // A value or key that is an expression is a variable the body computes: `sum strlen(x) - 1 : { … }` sums
        // v over `{ …, v = strlen(x) - 1 }`.
        let mut body = body.to_vec();
        let over = over.map(|t| self.named(t, "over", &mut body));
        let keys: Vec<String> = by.iter().map(|t| self.named(t, "key", &mut body)).collect();
        if let AggOp::Join { by, .. } = &mut op {
            by.clone_from(&keys);
        }
        // The variables the aggregate reads from its body: the aggregated one and join's order.
        let read: BTreeSet<String> = over.iter().chain(&keys).cloned().collect();
        let ours = |v: &str| !outer.contains(v);
        // One plain atom: the engine's aggregate reads it. The built-ins lowering writes out (axes, core.rr's tree
        // arithmetic, …) are no relations of the program, so a body of one of them goes through the helper.
        if let [Lit::Atom { rel, args, neg: false }] = body.as_slice()
            && test_of(rel).is_none()
            && !matches!(rel.as_str(), "nearest" | "inside" | "has" | "split_dot" | "ast" | "within_stop" | "precedes_stop")
            && !self.rewritten(rel)
        {
            let Literal::Atom(mut atom) = self.atom(rel, args, false)? else { unreachable!("a plain atom") };
            let at = |v: &str| atom.args.iter().position(|t| matches!(t, Term::Var(w) if w == v));
            // A variable of the body's own the atom repeats is an equality the engine's aggregate would not test, and
            // a key or value the atom lacks is the helper's error below.
            let repeated = atom.vars().iter().any(|v| ours(v) && atom.args.iter().filter(|t| matches!(t, Term::Var(w) if w == v)).count() > 1);
            let placed = read.iter().all(|v| at(v).is_some());
            if !repeated && placed {
                // A variable only the body uses is counted over, not grouped by: a wildcard to the engine. The
                // aggregated variable and join's order are read.
                for t in &mut atom.args {
                    if matches!(t, Term::Var(v) if ours(v) && !read.contains(v)) {
                        *t = Term::Wild;
                    }
                }
                return Ok(vec![Literal::Agg { var: var.to_owned(), op, over, atom }]);
            }
        }
        // Otherwise a helper relation of the body. `count` and `sum` range over the body's rows, a multiset of the
        // aggregated value, so they keep every variable; `min`, `max` and `join` (of distinct rows) keep those they
        // read and those the rule shares.
        let inner = body_vars(&body);
        let every = matches!(op, AggOp::Count | AggOp::Sum);
        let cols: Vec<String> = inner.iter().filter(|v| every || read.contains(*v) || outer.contains(*v)).cloned().collect();
        if let Some(v) = read.iter().find(|v| !cols.contains(v)) {
            return Err(Error::Syntax(format!("{agg}: {v} is not bound in its body")));
        }
        let name = self.fresh("agg");
        let mut bodies = self.conjunction(&body, outer)?;
        // A shared variable the body only tests (`count : { slash_at(ps, j), i < j }`) is bound inside the helper by
        // a context relation: the rule's other literals, projected on those variables.
        let bound: BTreeSet<String> = body_vars(
            &body.iter().filter(|l| binds_columns(l)).cloned().collect::<Vec<_>>(),
        );
        let needed: Vec<String> = inner.iter().filter(|v| outer.contains(*v) && !bound.contains(*v)).cloned().collect();
        if !needed.is_empty() {
            let ctx = self.context(&needed)?;
            for b in &mut bodies {
                b.insert(0, Literal::Atom(ctx.clone()));
            }
        }
        let types = cols.iter().map(|c| (c.clone(), self.type_of(c, &bodies))).collect();
        self.prog.decls.insert(name.clone(), types);
        let args: Vec<Term> = cols.iter().map(|c| Term::Var(c.clone())).collect();
        for b in bodies {
            self.prog.rules.push(Rule { head: Atom { rel: name.clone(), args: args.clone(), neg: false, gated: false }, body: b });
        }
        // The helper keeps every column (`count` counts distinct rows), but only the rule's variables group it; the
        // aggregated variable and join's order are read.
        let args = cols.iter().map(|c| if outer.contains(c) || read.contains(c) { Term::Var(c.clone()) } else { Term::Wild }).collect();
        Ok(vec![Literal::Agg { var: var.to_owned(), op, over, atom: Atom { rel: name, args, neg: false, gated: false } }])
    }

    /// An aggregate's value or key as a variable: itself, or one the body is given to compute it.
    fn named(&mut self, t: &STerm, stem: &str, body: &mut Vec<Lit>) -> String {
        if let STerm::Var(v) = t {
            return v.clone();
        }
        let v = self.fresh(stem);
        body.push(Lit::Cmp { l: STerm::Var(v.clone()), op: "=", r: t.clone() });
        v
    }

    /// The aggregates inside terms, each a comparison `rr_aggN = agg` of its own before the literal that uses it
    /// (`n = 1 + count : { … }` is `rr_agg1 = count : { … }, n = 1 + rr_agg1`), as `comparison` lowers it. An
    /// aggregate a comparison has on one side stays where it is.
    fn lift_aggregates(&mut self, lits: &[Lit]) -> Vec<Lit> {
        let mut out = Vec::new();
        for l in lits {
            let mut lifted = Vec::new();
            let l = match l {
                Lit::Atom { rel, args, neg } => Lit::Atom { rel: rel.clone(), args: args.iter().map(|a| self.lift(a, false, &mut lifted)).collect(), neg: *neg },
                Lit::Cmp { l, op, r } => {
                    let l2 = self.lift(l, true, &mut lifted);
                    let r2 = self.lift(r, !matches!(l2, STerm::Agg { .. }), &mut lifted);
                    Lit::Cmp { l: l2, op, r: r2 }
                }
                l => l.clone(),
            };
            out.extend(lifted);
            out.push(l);
        }
        out
    }

    /// A head's aggregates as `lift_aggregates` lifts them, into the body.
    fn lift_head_aggregates(&mut self, heads: &[Head], body: &[Lit]) -> (Vec<Head>, Vec<Lit>) {
        let mut body = body.to_vec();
        let heads = heads.iter().map(|h| Head { args: h.args.iter().map(|a| self.lift(a, false, &mut body)).collect(), ..h.clone() }).collect();
        (heads, body)
    }

    fn lift(&mut self, t: &STerm, top: bool, out: &mut Vec<Lit>) -> STerm {
        match t {
            STerm::Agg { .. } if top => t.clone(),
            STerm::Agg { .. } => {
                let v = self.fresh("agg");
                out.push(Lit::Cmp { l: STerm::Var(v.clone()), op: "=", r: t.clone() });
                STerm::Var(v)
            }
            STerm::Call { name, functor, args } => STerm::Call { name: name.clone(), functor: *functor, args: args.iter().map(|a| self.lift(a, false, out)).collect() },
            STerm::Bin { op, l, r } => STerm::Bin { op, l: Box::new(self.lift(l, false, out)), r: Box::new(self.lift(r, false, out)) },
            STerm::Named(n, v) => STerm::Named(n.clone(), Box::new(self.lift(v, false, out))),
            t => t.clone(),
        }
    }

    /// A helper column's type: the column it fills in the body's atoms, else the type of the aggregate or the
    /// computation that assigns it (`k = 0 - i` is a number, which a `join` orders by value).
    fn type_of(&self, v: &str, bodies: &[Vec<Literal>]) -> String {
        for literal in bodies.iter().flatten() {
            match literal {
                Literal::Atom(a) if !a.neg => {
                    for (i, t) in a.args.iter().enumerate() {
                        if *t == Term::Var(v.to_owned()) {
                            if a.rel.starts_with(AXIS) {
                                return "node".to_owned();
                            }
                            if let Some(cols) = self.prog.decls.get(&a.rel)
                                && let Some((_, ty)) = cols.get(i)
                            {
                                return ty.clone();
                            }
                        }
                    }
                }
                Literal::Nearest { n, a, .. } if n == v || a == v => return "node".to_owned(),
                Literal::Ast { document, .. } if document == v => return "int".to_owned(),
                Literal::Ast { node, captures, .. } if node == v || captures.iter().any(|(_, c)| c == v) => return "node".to_owned(),
                _ => {}
            }
        }
        use crate::code_rule_dsl::logic::plan::lower::{Domain, functor_domains};
        let number = |n: bool| if n { "int".to_owned() } else { "sym".to_owned() };
        for literal in bodies.iter().flatten() {
            match literal {
                Literal::Agg { var, op, over, atom } if var == v => {
                    return match op {
                        AggOp::Count | AggOp::Sum => "int".to_owned(),
                        AggOp::Join { .. } => "sym".to_owned(),
                        AggOp::Min | AggOp::Max => {
                            let col = atom.args.iter().position(|t| over.as_ref().is_some_and(|o| *t == Term::Var(o.clone())));
                            col.and_then(|c| self.prog.decls.get(&atom.rel)?.get(c).map(|(_, ty)| ty.clone())).unwrap_or_else(|| "int".to_owned())
                        }
                    };
                }
                Literal::Cmp { l: Term::Var(x), op: "=", r } if x == v => match r {
                    Term::Int(_) => return "int".to_owned(),
                    Term::Str(_) => return "sym".to_owned(),
                    Term::Call(f, args) if let (_, Some(d)) = functor_domains(f, args.len()) => return number(d == Domain::Int),
                    _ => {}
                },
                _ => {}
            }
        }
        "sym".to_owned()
    }

    fn term(&mut self, t: &STerm) -> Result<Term, Error> {
        Ok(match t {
            STerm::Var(v) if v == "top" && !self.top_is_var => Term::Top,
            STerm::Var(v) => Term::Var(v.clone()),
            STerm::Wild => Term::Wild,
            STerm::Num(n) => Term::Int(*n),
            STerm::Str(s) => Term::Str(s.clone()),
            STerm::Call { name, functor: true, args } => {
                let args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
                Term::Call(if name == "dot" { "cat".to_owned() } else { name.clone() }, args)
            }
            STerm::Call { name, functor: false, args } => {
                // Soufflé's functions: `cat` joins strings (the engine's `cat` joins with a dot, so it is `concat`;
                // `cat(a, b, c)` is `concat(concat(a, b), c)`). `to_number`, `max` and `min` are
                // `strings::functions`'.
                let f = match (name.as_str(), args.len()) {
                    ("cat", 2..) => "concat",
                    ("strlen" | "to_string" | "to_number", 1) => name.as_str(),
                    ("substr", 3) => "substr",
                    ("max" | "min", 2) => name.as_str(),
                    _ => return not_yet(format!("the built-in function {name}/{}", args.len())),
                };
                let mut args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?.into_iter();
                if f == "concat" {
                    let first = args.next().expect("two or more arguments");
                    return Ok(args.fold(first, |l, r| Term::Call("concat".to_owned(), vec![l, r])));
                }
                Term::Call(f.to_owned(), args.collect())
            }
            STerm::Bin { op, l, r } => {
                let f = match *op {
                    "+" => "add",
                    "-" => "sub",
                    "*" => "mul",
                    "/" => "div",
                    // C's `%`, as Soufflé's: the sign of `l`; `l % 0` has no value (`strings::functions`).
                    "%" => "mod",
                    other => return not_yet(format!("arithmetic ({other})")),
                };
                Term::Call(f.to_owned(), vec![self.term(l)?, self.term(r)?])
            }
            STerm::Agg { op, .. } => return not_yet(format!("the aggregate {op} as a term")),
            STerm::Tick(t) => return not_yet(format!("the template `{t}`")),
            STerm::Float(f) => return not_yet(format!("the decimal number {f}")),
            STerm::Json(_) | STerm::Named(..) => return Err(Error::Syntax("a JSON term or an `ast` capture outside `ast(…)`".to_owned())),
        })
    }

    /// Kind sets: the relations of facts given to `nearest`, `inside` and `has`. One read nowhere else is only a set.
    /// A set of nodes, or of kinds that rules derive, is no kind set: its axes are rules (`derived_axes`). And
    /// `precedes_stop`'s sets give `rr_next_stop_<set>` (`next_stops`).
    fn kind_sets(&mut self) -> Result<(), Error> {
        let mut derived = BTreeMap::new();
        for set in std::mem::take(&mut self.sets) {
            if self.prog.kindsets.contains_key(&set) {
                continue;
            }
            let kinds = match self.members(&set)? {
                Members::Kinds(kinds) => kinds,
                other => {
                    derived.insert(set, other);
                    continue;
                }
            };
            self.prog.kindsets.insert(set.clone(), kinds);
            // A set no rule reads is no relation of the program; one that rules read (`lfscope(k) :- fscope(k)`)
            // stays one as well.
            let read = self.prog.rules.iter().filter(|r| r.head.rel != set).any(|r| r.body.iter().filter_map(Literal::atom).any(|a| a.rel == set));
            if !read {
                self.prog.facts.remove(&set);
                self.prog.decls.remove(&set);
                self.prog.rules.retain(|r| r.head.rel != set);
            }
        }
        self.derived_axes(&derived);
        self.next_stops()
    }

    /// What a set holds: its kinds when they are known now (a kind set), else nodes or kinds its rules derive.
    fn members(&self, set: &str) -> Result<Members, Error> {
        if let Some(kinds) = self.prog.kindsets.get(set) {
            return Ok(Members::Kinds(kinds.clone()));
        }
        let column = match self.prog.decls.get(set).map(Vec::as_slice) {
            Some([(_, t)]) => Some(t.as_str()),
            _ => None,
        };
        if column == Some("node") {
            return Ok(Members::Nodes);
        }
        if let Some(kinds) = self.kinds_of(set, &mut BTreeSet::new()) {
            return Ok(Members::Kinds(kinds));
        }
        if column == Some("sym") {
            return Ok(Members::Derived);
        }
        Err(Error::Syntax(format!("the set {set} is not a relation of kinds or of nodes (one column of kinds, or of nodes)")))
    }

    /// The unary relation of nodes in `set`: the set itself when it holds nodes, else `rr_in_<set>` over `node`.
    fn member_relation(&mut self, set: &str, members: &Members) -> String {
        if let Members::Nodes = members {
            return set.to_owned();
        }
        let rel = format!("rr_in_{set}");
        if self.prog.decls.contains_key(&rel) {
            return rel;
        }
        self.base("node");
        self.prog.decls.insert(rel.clone(), vec![("x".to_owned(), "node".to_owned())]);
        let v = |s: &str| Term::Var(s.to_owned());
        let node = |k: Term| Literal::Atom(Atom { rel: "node".to_owned(), args: vec![Term::Wild, v("x"), k, Term::Wild, Term::Wild], neg: false, gated: false });
        let head = Atom { rel: rel.clone(), args: vec![v("x")], neg: false, gated: false };
        let bodies = match members {
            Members::Kinds(kinds) => kinds.iter().map(|k| vec![node(Term::Str(k.clone()))]).collect(),
            _ => vec![vec![node(v("k")), Literal::Atom(Atom { rel: set.to_owned(), args: vec![v("k")], neg: false, gated: false })]],
        };
        for body in bodies {
            self.prog.rules.push(Rule { head: head.clone(), body });
        }
        rel
    }

    /// The axes over sets of nodes, or of kinds rules derive, which the transducer cannot read from a node's kind:
    /// rules over `child`, in which each node's membership is read once. A helper reads its set negated, so the
    /// set is complete before it (a lower stratum; a set that depends on its own axis is refused by the strata).
    /// - `rr_near_<s>(n, a)`, a is n's nearest strict ancestor in s (`nearest`, and `inside(n, _, s)`): down from each
    ///   member to the next, a row per node below one;
    /// - `rr_inside_<s>(n, a)`, every strict ancestor a of n in s (`inside`): the chain of nearest ones;
    /// - `rr_has_<s>(n, a)`, n in s and a any strict ancestor of it (`has`): up from each member;
    /// - `rr_above_<s>(a)`, a has a member strictly below it (`has(a, _, s)`).
    fn derived_axes(&mut self, derived: &BTreeMap<String, Members>) {
        let mut need = BTreeSet::new();
        for rule in &mut self.prog.rules {
            for l in &mut rule.body {
                let (set, helper, args, neg) = match l {
                    Literal::Nearest { n, a, kindset } if derived.contains_key(kindset) => (kindset.clone(), "near", vec![Term::Var(n.clone()), Term::Var(a.clone())], false),
                    Literal::Atom(x) => match axis(&x.rel) {
                        Some((set, has)) if derived.contains_key(set) => {
                            let (helper, args) = match (has, x.args.as_slice()) {
                                (false, [_, Term::Wild]) => ("near", x.args.clone()),
                                (false, _) => ("inside", x.args.clone()),
                                (true, [Term::Wild, a]) => ("above", vec![a.clone()]),
                                (true, _) => ("has", x.args.clone()),
                            };
                            (set.to_owned(), helper, args, x.neg)
                        }
                        _ => continue,
                    },
                    _ => continue,
                };
                *l = Literal::Atom(Atom { rel: format!("rr_{helper}_{set}"), args, neg, gated: false });
                if helper == "inside" {
                    need.insert((set.clone(), "near"));
                }
                need.insert((set, helper));
            }
        }
        if need.is_empty() {
            return;
        }
        self.base("child");
        let v = |s: &str| Term::Var(s.to_owned());
        let atom = |rel: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: rel.to_owned(), args, neg, gated: false });
        let child = |p: &str, c: &str| atom("child", vec![v(p), v(c), Term::Wild], false);
        for (set, helper) in need {
            let member = self.member_relation(&set, &derived[&set]);
            let is = |x: &str, neg: bool| atom(&member, vec![v(x)], neg);
            let rel = format!("rr_{helper}_{set}");
            let me = |args: &[&str]| atom(&rel, args.iter().map(|a| v(a)).collect(), false);
            let (cols, bodies): (&[&str], Vec<Vec<Literal>>) = match helper {
                "near" => (&["n", "a"], vec![vec![child("a", "n"), is("a", false)], vec![child("p", "n"), is("p", true), me(&["p", "a"])]]),
                "inside" => (&["n", "a"], vec![vec![atom(&format!("rr_near_{set}"), vec![v("n"), v("a")], false)], vec![atom(&format!("rr_near_{set}"), vec![v("n"), v("b")], false), me(&["b", "a"])]]),
                "has" => (&["n", "a"], vec![vec![is("n", false), child("a", "n")], vec![me(&["n", "p"]), child("a", "p")]]),
                _ => (&["a"], vec![vec![is("n", false), child("a", "n")], vec![me(&["p"]), child("a", "p")]]),
            };
            self.prog.decls.insert(rel.clone(), cols.iter().map(|c| ((*c).to_owned(), "node".to_owned())).collect());
            let head = Atom { rel: rel.clone(), args: cols.iter().map(|c| v(c)).collect(), neg: false, gated: false };
            for body in bodies {
                self.prog.rules.push(Rule { head: head.clone(), body });
            }
        }
    }

    /// `rr_next_stop_<s>(a, k)` for each set `precedes_stop` reads: k is the index of the first child of s after
    /// a among its siblings, a walk to the right from each node (a row per node with a member after it).
    fn next_stops(&mut self) -> Result<(), Error> {
        let v = |s: &str| Term::Var(s.to_owned());
        let atom = |rel: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: rel.to_owned(), args, neg, gated: false });
        let after = |j: &str| Literal::Cmp { l: v(j), op: "=", r: Term::Call("add".to_owned(), vec![v("i"), Term::Int(1)]) };
        for (set, kinds) in std::mem::take(&mut self.stops) {
            let members = match kinds {
                Some(kinds) => Members::Kinds(kinds),
                None => self.members(&set)?,
            };
            let member = self.member_relation(&set, &members);
            let rel = format!("rr_next_stop_{set}");
            let head = Atom { rel: rel.clone(), args: vec![v("a"), v("k")], neg: false, gated: false };
            let child = |c: &str, i: &str| atom("child", vec![v("p"), v(c), v(i)], false);
            self.prog.rules.push(Rule { head: head.clone(), body: vec![child("a", "i"), after("k"), child("s", "k"), atom(&member, vec![v("s")], false)] });
            self.prog.rules.push(Rule { head, body: vec![child("a", "i"), after("j"), child("s", "j"), atom(&member, vec![v("s")], true), atom(&rel, vec![v("s"), v("k")], false)] });
        }
        Ok(())
    }

    /// The kinds of a unary relation of kinds: its facts, and the kinds of `s` for each rule `set(k) :- s(k)`.
    fn kinds_of(&self, set: &str, seen: &mut BTreeSet<String>) -> Option<Vec<String>> {
        if !seen.insert(set.to_owned()) || self.prog.inputs.iter().any(|i| i == set) || self.prog.decls.get(set).is_some_and(|c| c.len() != 1) {
            return None;
        }
        let mut kinds: Vec<String> = Vec::new();
        for row in self.prog.facts.get(set).into_iter().flatten() {
            let [Value::Str(k)] = row.as_slice() else { return None };
            kinds.push(k.clone());
        }
        for r in self.prog.rules.iter().filter(|r| r.head.rel == set) {
            let ([Term::Var(k)], [Literal::Atom(a)]) = (r.head.args.as_slice(), r.body.as_slice()) else { return None };
            if a.neg || !matches!(a.args.as_slice(), [Term::Var(v)] if v == k) {
                return None;
            }
            kinds.extend(self.kinds_of(&a.rel, seen)?);
        }
        // In the order written, each once.
        let mut once = BTreeSet::new();
        kinds.retain(|k| once.insert(k.clone()));
        Some(kinds)
    }
}

/// A regex's named groups, in order (`(?<x>…)` and `(?P<x>…)`).
fn groups(re: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = re;
    while let Some(i) = rest.find("(?<").map(|i| (i, 3)).into_iter().chain(rest.find("(?P<").map(|i| (i, 4))).min() {
        let after = &rest[i.0 + i.1..];
        let Some(end) = after.find('>') else { break };
        let name = &after[..end];
        if !name.is_empty() && !name.starts_with(['=', '!']) && !out.iter().any(|g| g == name) {
            out.push(name.to_owned());
        }
        rest = &after[end..];
    }
    out
}

/// The kind set of one kind, `"call"`: `rr_kind_call`.
fn kind_set_name(kind: &str) -> String {
    format!("rr_kind_{}", kind.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>())
}

/// The variables of literals, the relations passed to built-ins (`nearest(n, a, scope)`'s `scope`) left out.
fn body_vars(lits: &[Lit]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for l in lits {
        match l {
            Lit::Atom { rel, args, .. } => {
                let skip: &[usize] = RELATION_ARG.iter().find(|(r, _)| *r == rel).map_or(&[], |(_, k)| k);
                for (k, a) in args.iter().enumerate() {
                    if !(skip.contains(&k) && matches!(a, STerm::Var(_))) {
                        let mut vs = Vec::new();
                        a.vars(&mut vs);
                        out.extend(vs);
                    }
                }
            }
            Lit::Group { alts, .. } => alts.iter().for_each(|a| out.extend(body_vars(a))),
            other => {
                let mut vs = Vec::new();
                other.scope_vars(&mut vs);
                out.extend(vs);
            }
        }
    }
    out
}

/// `body_vars`, with the variables of aggregates inside comparisons too: what a group shares with its rule when an
/// aggregate in it is correlated on the rule's variables (`count : { b(x, _) } > 2` shares x).
fn shared_vars(lits: &[Lit]) -> BTreeSet<String> {
    let mut out = body_vars(lits);
    for l in lits {
        match l {
            Lit::Cmp { l, r, .. } => {
                let mut vs = Vec::new();
                l.vars(&mut vs);
                r.vars(&mut vs);
                out.extend(vs);
            }
            Lit::Group { alts, .. } => alts.iter().for_each(|a| out.extend(shared_vars(a))),
            _ => {}
        }
    }
    out
}

/// Whether a literal binds every variable of its columns: a positive atom of a relation, not a text test nor a
/// built-in the lowering writes out as tests or computations of variables bound elsewhere (`within`, `earlier`,
/// `split_dot`, `nearest`, `precedes_stop`, …).
fn binds_columns(l: &Lit) -> bool {
    matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none()
        && !matches!(rel.as_str(), "within" | "within_stop" | "earlier" | "split_dot" | "nearest" | "precedes_stop" | "dfs_first" | "dfs_first_memo" | "unset"))
}

/// Adjacent negations of one test of one string, `!glob("a", s), !glob("b", s)`, as one negated test of the set.
pub(super) fn merge_negated_tests(body: Vec<Literal>) -> Vec<Literal> {
    let mut out: Vec<Literal> = Vec::new();
    for literal in body {
        let negated_test = |l: &Literal| match l {
            Literal::Cmp { l: Term::Call(op, args), op: "=", r: Term::Int(0) } if args.len() == 2 && crate::code_rule_dsl::logic::strings::is_test(op) && op != "rx" && op != "irx" => Some((op.clone(), args[0].clone(), args[1].clone())),
            _ => None,
        };
        if let (Some((op, s, p)), Some(last)) = (negated_test(&literal), out.last_mut())
            && let Some((last_op, last_s, last_p)) = negated_test(last)
            && op == last_op
            && s == last_s
            && matches!(p, Term::Str(_))
        {
            let mut patterns = match last_p {
                Term::Call(f, items) if f == SET => items,
                Term::Str(_) => vec![last_p],
                _ => {
                    out.push(literal);
                    continue;
                }
            };
            patterns.push(p);
            *last = Literal::Cmp { l: Term::Call(op, vec![s, Term::Call(SET.to_owned(), patterns)]), op: "=", r: Term::Int(0) };
            continue;
        }
        out.push(literal);
    }
    out
}

// ---- violations ----

/// A `.violation`, as `Lowerer::findings` makes its findings: its relation and arity, its code, the type of its
/// `at` column (`node`, `span` or `loc`) and its line.
struct Violation {
    name: String,
    arity: usize,
    code: FindingCode,
    at: String,
    line: usize,
}

/// A finding's code: a column of the violation's row (`code {c}`), or a constant (`code "X"`, or the relation's name).
enum FindingCode {
    Column(usize),
    Const(String),
}

/// `@place(path, line)` (`core.rr`): a place with no node, as a `loc` value: `-2 - (path << 32 | line)`, `path` the
/// path's symbol (its atom) and `0 <= line < 2^32`, so no node id (`d << 32 | index`, never negative) and not `unset`
/// (-1). `@place_path(x)` and `@place_line(x)` take it apart; `plan`'s SQL computes all three, and the native set
/// engine leaves rules that call them to SQL. A place with no valid line is `unset`, which has no finding.
pub(super) const PLACE: &str = "place";
pub(super) const PLACE_PATH: &str = "place_path";
pub(super) const PLACE_LINE: &str = "place_line";

/// What a `loc` value can be (`Lowerer::origins`): a node, a `@place`, or anything else (`other`, with what made
/// it: a regex capture, a token, a number).
#[derive(Clone, Debug, Default, PartialEq)]
struct Origin {
    node: bool,
    place: bool,
    other: Option<String>,
}

impl Origin {
    fn node() -> Self {
        Origin { node: true, ..Origin::default() }
    }

    fn other(why: String) -> Self {
        Origin { other: Some(why), ..Origin::default() }
    }

    /// What either can hold; whether that is more than `self` held.
    fn join(&mut self, o: &Origin) -> bool {
        let before = (self.node, self.place, self.other.is_some());
        self.node |= o.node;
        self.place |= o.place;
        if self.other.is_none() {
            self.other.clone_from(&o.other);
        }
        before != (self.node, self.place, self.other.is_some())
    }

    /// What both can hold.
    fn meet(&self, o: &Origin) -> Origin {
        Origin { node: self.node && o.node, place: self.place && o.place, other: self.other.clone().filter(|_| o.other.is_some()) }
    }
}

fn has_place(t: &Term) -> bool {
    match t {
        Term::Call(f, args) => f == PLACE || args.iter().any(has_place),
        _ => false,
    }
}

fn literal_terms(l: &Literal) -> Vec<&Term> {
    match l {
        Literal::Atom(a) | Literal::Agg { atom: a, .. } => a.args.iter().collect(),
        Literal::Cmp { l, r, .. } => vec![l, r],
        _ => Vec::new(),
    }
}

/// What a term holds, given its rule's variables' (`rule_origins`); `None` for a variable nothing binds yet.
fn term_origin(t: &Term, vars: &BTreeMap<String, Origin>) -> Option<Origin> {
    Some(match t {
        Term::Var(v) => return vars.get(v).cloned(),
        Term::Wild => return None,
        Term::Call(f, _) if f == PLACE => Origin { place: true, ..Origin::default() },
        Term::Call(f, _) if f == "rx_group" => Origin::other("a regex capture".to_owned()),
        Term::Call(f, _) => Origin::other(format!("a value of @{f}")),
        Term::Int(UNSET) => Origin::default(),
        Term::Int(_) | Term::Str(_) | Term::Top => Origin::other("a constant".to_owned()),
    })
}

/// What each variable of a rule body can hold, `column(rel, c)` giving what a relation's column holds: the meet of
/// every positive literal binding it, then of the equalities `x = t`; a variable only an order comparison with a node
/// binds (`within`'s `n > a, n <= a + size`, a range of the tape's ids) is a node.
fn rule_origins(body: &[Literal], column: &dyn Fn(&str, usize) -> Origin) -> BTreeMap<String, Origin> {
    fn meet(vars: &mut BTreeMap<String, Origin>, v: &str, o: Origin) {
        match vars.get_mut(v) {
            Some(e) => *e = e.meet(&o),
            None => {
                vars.insert(v.to_owned(), o);
            }
        }
    }
    let mut direct = BTreeMap::new();
    for l in body {
        match l {
            Literal::Atom(a) if !a.neg => {
                for (c, t) in a.args.iter().enumerate() {
                    if let Term::Var(v) = t {
                        meet(&mut direct, v, column(&a.rel, c));
                    }
                }
            }
            Literal::Agg { var, .. } => meet(&mut direct, var, Origin::other("an aggregate".to_owned())),
            Literal::Nearest { n, a, .. } => {
                meet(&mut direct, n, Origin::node());
                meet(&mut direct, a, Origin::node());
            }
            Literal::Ast { document, node, captures, .. } => {
                meet(&mut direct, document, Origin::other("a document".to_owned()));
                meet(&mut direct, node, Origin::node());
                captures.iter().for_each(|(_, c)| meet(&mut direct, c, Origin::node()));
            }
            _ => {}
        }
    }
    let mut vars = direct.clone();
    for _ in 0..=body.len() {
        let mut next = direct.clone();
        for l in body {
            let Literal::Cmp { l, op: "=", r } = l else { continue };
            for (x, t) in [(l, r), (r, l)] {
                if let Term::Var(x) = x
                    && let Some(o) = term_origin(t, &vars)
                {
                    meet(&mut next, x, o);
                }
            }
        }
        for l in body {
            let Literal::Cmp { l, op: "<" | "<=" | ">" | ">=", r } = l else { continue };
            for (x, y) in [(l, r), (r, l)] {
                if let (Term::Var(x), Term::Var(y)) = (x, y)
                    && !next.contains_key(x)
                    && vars.get(y) == Some(&Origin::node())
                {
                    next.insert(x.clone(), Origin::node());
                }
            }
        }
        if next == vars {
            break;
        }
        vars = next;
    }
    vars
}
