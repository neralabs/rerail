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
//!   `inside(n, a, s)` is `rr_inside_<s>(n, a)`, and `has(a, n, s)` is `rr_has_<s>(n, a)`, the set tested on n; a
//!   set of nodes is read as climbs up the tree (`Lowerer::node_axis`), and a set of kinds rules derive as rules
//!   over `child` (`Lowerer::derived_axes`), which read the set negated, so it is complete first;
//! - `@dot` is the engine's `@cat`, and `split_dot(q, m, x)` is `q = @dot(m, x)` with x dot-free (it runs either
//!   way);
//! - `count : { … }`, `min x : { … }`, `max x : { … }`, `sum x : { … }` and `join x by k sep "s" : { … }` are
//!   aggregates; a body of more than one atom, or of one built-in the lowering writes out, gets a helper relation
//!   (count and sum keep every variable: they read a multiset of rows; join reads the distinct rows of its value
//!   and keys); a value or key that is an expression is a variable the body computes; join's keys are variables of
//!   its body, ordered by text for symbols and by value for numbers, ties by the value;
//! - an aggregate inside a term (`1 + count : { … }`, in a head or a comparison) is a variable bound beside it
//!   (`Lowerer::hoist`); an aggregate on one side of a comparison gets a variable;
//! - `!(a, b)` is a helper relation of the variables it shares with the rule, negated
//!   (`Lowerer::negated_conjunction`); `!(a ; b)` is one per alternative;
//! - other disjunctions are one rule per alternative.
//!
//! - `.violation V(at: loc, …)` declares V as an output, and each row of V is a finding
//!   `violation(policy, V, code, path, line)`: at a node, its document's path and its line; at a span, its `docof`
//!   document and its line; a head `V(@place(path, line), …)` writes V's place relation, and a `@place(path, line)`
//!   value elsewhere (the loc value `-2 - (path << 32 | line)`) is found at that path and line. The code is V,
//!   `code "X"`, or the `code {c}` column. `suppress TOOL "rule", … [over {c}]`, `code {c} in "re"` and
//!   `fix [safe] F "title"` are kept as the settings `suppress V …`, `code_in V …` and `fix V …` for the findings
//!   layer, which applies them;
//! - a rule ending in `=> edits` defines a fix, and fixes are not run: it is left out;
//! - `.input r(IO=…, filename="…")` is an input, its parameters kept as the setting `input`;
//! - core.rr is the prelude (`rr::program` reads it before every file): the program keeps the core relations it reads
//!   (a relation read and not declared takes its declaration there); `next_sibling`, `precedes`, `earlier` and
//!   `syntax_error` lower to the relations the engine has (`core`), negated through a relation of their rows;
//!   `kind` and T's `docof` over nodes are projections of `node`, and core.rr's `docof(x: loc, d)` an input;
//! - `within(n, a)` is the engine's relation read off the tree, and `within_stop(n, a, stop)` that less a stop
//!   between (`Lowerer::within`, decision 0050), and so is its negation; `!nearest(n, a, s)` is a helper relation;
//!   `precedes_stop(a, b, S)` is a relation over `child` (`Lowerer::precedes_stop`);
//! - `unset(x)` is the empty value -1, whose text is "", and `!unset(x)` is `x != -1`;
//! - `match("(?<x>…)", s)` binds x to the named group (`Lowerer::captures`); `match_all` and `match_group` are the
//!   text layer's relations (`strings::as_relations`); `+ - * / %` and Soufflé's `cat` of any arity (`concat`
//!   folded from the left), `strlen`, `to_string`, `substr`, `to_number`, `to_float`, `max/2` and `min/2` are
//!   functors. `to_number` reads as Soufflé's `symbol2numeric`; where Soufflé stops with an error (no digits, out
//!   of range, `x % 0`) there is no value, and so no row. `@lower` is Unicode lowercase; `@dot("", b)` is b; a
//!   decimal number is its float's key (`float_key`).
//!
//! Not lowered yet, each a named error: patterns (``n ~ `…` ``), `ordered by`, `mean` (a decimal number),
//! `dfs_first`/`dfs_first_memo`, and a negated built-in with its other node unnamed (`!within_stop(n, _, s)`,
//! `!split_dot(q, _, x)`).

mod core;
pub(in crate::code_rule_dsl) use self::core::is_core;

use std::collections::{BTreeMap, BTreeSet};

use super::syntax::{Code, Fix, Head, Item, Lit, Statement, Suppress, Term as STerm, quote};
use crate::code_rule_dsl::logic::front::{AXIS, AggOp, Atom, DESCENDANTS, Error, LatticeDecl, Literal, Program, RESERVED, Rule, Term, Value, axis};
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
    /// Kinds that rules derive from the tree (`doc_code_kind(k) :- kind(_, k), k != "comment"`).
    Derived,
}

/// A column type in the engine's names.
fn ty(t: &str) -> String {
    match t {
        "number" | "unsigned" => "int".to_owned(),
        "symbol" => "sym".to_owned(),
        "span" | "loc" => "node".to_owned(),
        // A float column holds floats' keys (`float_key`).
        "float" => "int".to_owned(),
        other => other.to_owned(),
    }
}

/// A `.violation V(at, …)`: its relation, its columns, its code, and whether its `at` is a node (else a span or a
/// place); `places`: whether a rule gives it a `@place(path, line)`, kept in its place relation (`place_relation`).
struct Violation {
    name: String,
    arity: usize,
    code: FindingCode,
    node: bool,
    places: bool,
}

/// A finding's code: the violation's name, a column's value, or a constant.
enum FindingCode {
    Name,
    Column(usize),
    Constant(String),
}

/// The relation of the `@place(path, line)` rows of a violation V: `(path, line, V's other columns)`.
fn place_relation(v: &str) -> String {
    format!("rr_place_{v}")
}

#[derive(Default)]
struct Lowerer {
    prog: Program,
    /// `.lattice` declarations: name → (base type, its `Lub` functor).
    lattices: BTreeMap<String, (String, String)>,
    /// Relations given as a kind set.
    sets: BTreeSet<String>,
    /// `.violation`s, by relation.
    violations: Vec<Violation>,
    next: usize,
    /// The literals beside the one being lowered, in its conjunction and those enclosing it: what binds an
    /// aggregate's shared variables (`aggregate`).
    siblings: Vec<Lit>,
    /// Whether the clause being lowered binds a variable named `top` (`path_part(d, top)`); otherwise `top` is a
    /// lattice's top value (`v != top`), as the translated programs write it.
    top_is_var: bool,
    /// Whether `docof` is declared over `loc`s, as `core.rr` declares it: an input that spans have rows in
    /// (decision 0046), not the projection of `node`.
    docof_of_locs: bool,
    /// The relations declared `must, may`, and the rules a `must` or `may` head wrote (by index).
    three: BTreeSet<String>,
    modal: BTreeMap<usize, String>,
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
            Item::Violation { name, cols, code, code_in, tag, suppress, fixes, .. } => l.violation(name, cols, code, code_in, tag, suppress, fixes).map_err(at)?,
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
    l.three_valued();
    l.findings();
    l.declare_core()?;
    l.projections();
    // The built-ins lowering writes out (`nearest`, `within`, …) are declared in `core.rr` for the checker; they
    // are no relations of the program.
    // `within` stays where a rule reads it: the lowering made it the engine's relation (decision 0050).
    let reads_within = l.prog.rules.iter().flat_map(|r| &r.body).filter_map(Literal::atom).any(|a| a.rel == crate::code_rule_dsl::facts::vtab::syntax::WITHIN);
    for b in RELATION_ARG.iter().map(|(r, _)| *r).chain(["within", "earlier", "split_dot", "unset"]).filter(|b| !(reads_within && *b == "within")) {
        l.prog.decls.remove(b);
        l.prog.inputs.retain(|i| i != b);
    }
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

    fn decl(&mut self, name: String, cols: Vec<(String, String)>, tail: &[String]) -> Result<(), Error> {
        let mut widen = None;
        let mut words = tail.iter();
        while let Some(w) = words.next() {
            match w.as_str() {
                // `must, may`: three-valued (`three_valued`).
                "must" | "may" | "," => {
                    self.three.insert(name.clone());
                }
                "widen" => widen = Some(words.next().and_then(|k| k.parse::<u32>().ok()).ok_or_else(|| Error::Syntax(format!("{name}: `widen` needs a count")))?),
                other => return not_yet(format!("`{other}` on .decl {name}")),
            }
        }
        if name == "docof" {
            self.docof_of_locs = cols.first().is_some_and(|(_, t)| t == "loc");
        }
        let mut out = Vec::new();
        for (i, (c, t)) in cols.iter().enumerate() {
            let Some(lattice) = t.strip_suffix("<>") else {
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

    /// `.decl r(…) must, may` (core.rr 3): r is computed twice. Its may side, which keeps the name, holds what is
    /// proven or unknown: every rule of r. Its must side (`rr_must_r`) holds what is proven: the rules with no
    /// prefix or `must`. A relation that reads a three-valued one is three-valued too, with every rule on both
    /// sides. In a must side's rule a positive atom of a three-valued relation reads its must side and a negated
    /// one its may side; a may side's rule reads the other way. A finding is the may side: a definite one and a
    /// possible one have the same key, and the possible tag is not made yet.
    fn three_valued(&mut self) {
        if self.three.is_empty() {
            return;
        }
        let must = |rel: &str| format!("{RESERVED}must_{rel}");
        let mut three = self.three.clone();
        loop {
            let before = three.len();
            for rule in &self.prog.rules {
                if rule.body.iter().any(|l| l.atom().is_some_and(|a| three.contains(&a.rel))) {
                    three.insert(rule.head.rel.clone());
                }
            }
            if three.len() == before {
                break;
            }
        }
        let side = |body: &[Literal], must_side: bool| -> Vec<Literal> {
            body.iter()
                .cloned()
                .map(|mut l| {
                    if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = &mut l
                        && three.contains(&a.rel)
                        && (a.neg != must_side)
                    {
                        a.rel = must(&a.rel);
                    }
                    l
                })
                .collect()
        };
        let mut rules = Vec::new();
        for (i, rule) in std::mem::take(&mut self.prog.rules).into_iter().enumerate() {
            if !three.contains(&rule.head.rel) {
                rules.push(rule);
                continue;
            }
            let modal = self.modal.get(&i).map(String::as_str);
            if modal != Some("may") {
                let mut head = rule.head.clone();
                head.rel = must(&head.rel);
                rules.push(Rule { head, body: side(&rule.body, true) });
            }
            rules.push(Rule { head: rule.head.clone(), body: side(&rule.body, false) });
        }
        self.prog.rules = rules;
        for rel in &three {
            if let Some(cols) = self.prog.decls.get(rel).cloned() {
                self.prog.decls.insert(must(rel), cols);
            }
            if let Some(rows) = self.prog.facts.get(rel).cloned() {
                self.prog.facts.insert(must(rel), rows);
            }
        }
    }

    /// `core.rr`'s `kind(n, k)` is a projection of `node`, and so is `docof(n, d)` as T's programs declare it (over
    /// nodes): rules, which the planner reads from `node` in place, not inputs, where a rule reads them (the prelude
    /// declares both for every program). `core.rr`'s `docof(x: loc, d)` stays an input, for spans have a document
    /// too. (`precedes` is core.rr's tree arithmetic, `core::rewrite`.)
    fn projections(&mut self) {
        let v = |s: &str| Term::Var(s.to_owned());
        let read: BTreeSet<String> = self.prog.rules.iter().flat_map(|r| r.body.iter().filter_map(Literal::atom)).map(|a| a.rel.clone()).collect();
        for (rel, args) in [("kind", [Term::Wild, v("n"), v("k"), Term::Wild, Term::Wild]), ("docof", [v("d"), v("n"), Term::Wild, Term::Wild, Term::Wild])] {
            if !read.contains(rel) || !self.prog.inputs.iter().any(|i| i == rel) || (rel == "docof" && (self.docof_of_locs || !self.prog.decls.contains_key("node"))) {
                continue;
            }
            // `kind` is always `node`'s column: a program that reads it reads `node`.
            self.base("node");
            self.prog.inputs.retain(|i| i != rel);
            let head = if rel == "kind" { vec![v("n"), v("k")] } else { vec![v("n"), v("d")] };
            // First, kind before docof: where T's programs wrote them.
            let at = usize::from(rel == "docof" && self.prog.rules.first().is_some_and(|r| r.head.rel == "kind"));
            self.prog.rules.insert(at, Rule {
                head: Atom { rel: rel.to_owned(), args: head, neg: false, gated: false },
                body: vec![Literal::Atom(Atom { rel: "node".to_owned(), args: args.to_vec(), neg: false, gated: false })],
            });
        }
    }

    /// `.violation V(at, …)`: V is an output relation, and each of its rows is a finding (`findings`). Its
    /// `suppress` and `fix` lines and `code {c} in "re"` are settings of the findings layer, as `.mode` is:
    /// `suppress V tool "rule", … [over {c}]`, `fix V [safe] F "title"` and `code_in V "re"`.
    #[allow(clippy::too_many_arguments)]
    fn violation(&mut self, name: String, cols: Vec<(String, String)>, code: Option<Code>, code_in: Option<String>, tag: Option<String>, suppress: Vec<Suppress>, fixes: Vec<Fix>) -> Result<(), Error> {
        let Some((_, at)) = cols.first() else { return Err(Error::Syntax(format!(".violation {name} has no `at` column"))) };
        if !matches!(at.as_str(), "node" | "span" | "loc") {
            return Err(Error::Syntax(format!(".violation {name}: `at` is a node, a span or a loc, not a {at}")));
        }
        let node = at == "node";
        // `tag {t}` keeps findings at one place apart; the engine's findings are keyed by place and code, so
        // it is checked and not carried.
        let column = |c: &String| cols.iter().position(|(n, _)| n == c).ok_or_else(|| Error::Syntax(format!(".violation {name}: no column {c}")));
        if let Some(t) = &tag {
            column(t)?;
        }
        let code = match code {
            None => FindingCode::Name,
            Some(Code::Column(c)) => FindingCode::Column(column(&c)?),
            Some(Code::Constant(c)) => FindingCode::Constant(c),
        };
        for s in suppress {
            let rules = s.rules.iter().map(|r| quote(r)).collect::<Vec<_>>().join(", ");
            let over = s.over.map(|c| format!(" over {{{c}}}")).unwrap_or_default();
            self.prog.settings.push(("suppress".to_owned(), format!("{name} {} {rules}{over}", s.tool)));
        }
        for f in fixes {
            self.prog.settings.push(("fix".to_owned(), format!("{name} {}{} {}", if f.safe { "safe " } else { "" }, f.name, quote(&f.title))));
        }
        if let Some(re) = code_in {
            self.prog.settings.push(("code_in".to_owned(), format!("{name} {}", quote(&re))));
        }
        self.violations.push(Violation { name: name.clone(), arity: cols.len(), code, node, places: false });
        self.decl(name.clone(), cols, &[])?;
        self.prog.outputs.push(name);
        Ok(())
    }

    /// The findings of the `.violation`s: `violation(policy, V, code, path, line)` for each row of V, at its first
    /// column's line (the engine's findings relation, as the converter's programs give it). A node's document is
    /// `node`'s column; a span's is `docof` (`core.rr`), and so is its line. A `@place(path, line)` row is read from
    /// V's place relation; a span or loc `at` that holds a `@place` value made elsewhere (`PLACE`) is found at its
    /// path and line.
    fn findings(&mut self) {
        if self.violations.is_empty() {
            return;
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
        let v = |s: &str| Term::Var(s.to_owned());
        let atom = |rel: &str, args: Vec<Term>| Literal::Atom(Atom { rel: rel.to_owned(), args, neg: false, gated: false });
        // Whether a rule makes a `@place` value: only then can a span or loc `at` hold one.
        let place_values = self.prog.rules.iter().any(|r| r.head.args.iter().chain(r.body.iter().flat_map(literal_terms)).any(has_place));
        for f in std::mem::take(&mut self.violations) {
            let code_col = match f.code {
                FindingCode::Column(i) => Some(i),
                _ => None,
            };
            let code = match &f.code {
                FindingCode::Name => Term::Str(f.name.clone()),
                FindingCode::Column(i) => v(&format!("rr_v{i}")),
                FindingCode::Constant(c) => Term::Str(c.clone()),
            };
            let head = Atom { rel: "violation".to_owned(), args: vec![Term::Str(policy.clone()), Term::Str(f.name.clone()), code, v("rr_p"), v("rr_l")], neg: false, gated: false };
            let cols: Vec<Term> = (0..f.arity).map(|i| if i == 0 || Some(i) == code_col { v(&format!("rr_v{i}")) } else { Term::Wild }).collect();
            let at = if f.node {
                vec![atom("node", vec![v("rr_d"), v("rr_v0"), Term::Wild, Term::Wild, Term::Wild]), atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild]), atom("line", vec![v("rr_v0"), v("rr_l")])]
            } else {
                vec![atom("docof", vec![v("rr_v0"), v("rr_d")]), atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild]), atom("line", vec![v("rr_v0"), v("rr_l")])]
            };
            self.prog.rules.push(Rule { head: head.clone(), body: std::iter::once(atom(&f.name, cols.clone())).chain(at).collect() });
            if !f.node && place_values {
                let of = |g: &str| Term::Call(g.to_owned(), vec![v("rr_v0")]);
                let mut found = head.clone();
                found.args[3] = of(PLACE_PATH);
                found.args[4] = of(PLACE_LINE);
                self.prog.rules.push(Rule { head: found, body: vec![atom(&f.name, cols.clone()), Literal::Cmp { l: v("rr_v0"), op: "<", r: Term::Int(UNSET) }] });
            }
            if f.places {
                let mut args = vec![v("rr_p"), v("rr_l")];
                args.extend(cols.into_iter().skip(1));
                self.prog.rules.push(Rule { head, body: vec![atom(&place_relation(&f.name), args)] });
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
        // An aggregate in a head's argument (`cc(f, 1 + count : { … })`) is a variable the body binds.
        let mut heads = heads.to_vec();
        let mut body = body.to_vec();
        for a in heads.iter_mut().flat_map(|h| &mut h.args) {
            self.hoist(a, &mut body, true);
        }
        let heads = heads.as_slice();
        let mut outer = Vec::new();
        heads.iter().flat_map(|h| &h.args).for_each(|a| a.vars(&mut outer));
        let bodies = self.conjunction(&body, &outer.into_iter().collect())?;
        for head in heads {
            if let Some(m) = &head.modal
                && !self.three.contains(&head.rel)
            {
                return Err(Error::Syntax(format!("`{m}` head {}: the relation is not declared `must, may`", head.rel)));
            }
            // `V(@place(path, line), …)` for a `.violation V`: a row of V's place relation.
            let mut rel = head.rel.clone();
            let mut written = head.args.clone();
            if let Some(STerm::Call { name, functor: true, args: place }) = head.args.first()
                && name == "place"
                && let Some(k) = self.violations.iter().position(|v| v.name == head.rel)
            {
                let [path, line] = place.as_slice() else { return Err(Error::Syntax("@place takes a path and a line".to_owned())) };
                if !self.violations[k].places {
                    self.violations[k].places = true;
                    let mut cols = vec![("path".to_owned(), "sym".to_owned()), ("line".to_owned(), "int".to_owned())];
                    cols.extend(self.prog.decls[&head.rel].iter().skip(1).cloned());
                    self.prog.decls.insert(place_relation(&head.rel), cols);
                }
                rel = place_relation(&head.rel);
                written = [path.clone(), line.clone()].into_iter().chain(head.args.iter().skip(1).cloned()).collect();
            }
            let args = written.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
            for body in &bodies {
                let (args, body) = unset(args.clone(), body.clone())?;
                if let Some(m) = &head.modal {
                    self.modal.insert(self.prog.rules.len(), m.clone());
                }
                self.prog.rules.push(Rule { head: Atom { rel: rel.clone(), args, neg: false, gated: false }, body });
            }
        }
        Ok(())
    }

    /// A conjunction's alternatives (one per choice in its disjunctions).
    /// `context`: the variables used outside the conjunction.
    fn conjunction(&mut self, lits: &[Lit], context: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        // An aggregate inside a term (`n = count : { … } + 1`) is a variable bound beside it.
        let mut hoisted = Vec::new();
        for lit in lits {
            let mut lit = lit.clone();
            match &mut lit {
                Lit::Atom { args, .. } => args.iter_mut().for_each(|a| self.hoist(a, &mut hoisted, true)),
                Lit::Cmp { l, r, .. } => {
                    // `count : { … } = count : { … }`: the right one gets a variable.
                    let both = matches!((&*l, &*r), (STerm::Agg { .. }, STerm::Agg { .. }));
                    self.hoist(l, &mut hoisted, false);
                    self.hoist(r, &mut hoisted, both);
                }
                Lit::Pattern { .. } | Lit::Group { .. } => {}
            }
            hoisted.push(lit);
        }
        let lits = hoisted.as_slice();
        let mut acc = vec![Vec::new()];
        for (k, lit) in lits.iter().enumerate() {
            // The variables used outside this literal, which an aggregate inside it is correlated on.
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

    /// Each aggregate inside `t` (and `t` itself when `whole`) as a fresh variable, bound by `v = aggregate` in `out`.
    fn hoist(&mut self, t: &mut STerm, out: &mut Vec<Lit>, whole: bool) {
        match t {
            STerm::Agg { .. } if whole => {
                let v = STerm::Var(self.fresh("agg"));
                let agg = std::mem::replace(t, v.clone());
                out.push(Lit::Cmp { l: v, op: "=", r: agg });
            }
            // `@tree_digest(n)` reads the tree: a variable bound by the engine's `tree_digest(n, h)`.
            STerm::Call { name, functor: true, args } if name == "tree_digest" && args.len() == 1 => {
                self.hoist(&mut args[0], out, true);
                let n = args[0].clone();
                let v = STerm::Var(self.fresh("digest"));
                *t = v.clone();
                out.push(Lit::Atom { rel: "tree_digest".to_owned(), args: vec![n, v], neg: false });
                if !self.prog.inputs.iter().any(|i| i == "tree_digest") {
                    self.prog.inputs.push("tree_digest".to_owned());
                    self.prog.decls.insert("tree_digest".to_owned(), vec![("n".to_owned(), "node".to_owned()), ("s".to_owned(), "sym".to_owned())]);
                }
            }
            STerm::Call { args, .. } => args.iter_mut().for_each(|a| self.hoist(a, out, true)),
            STerm::Bin { l, r, .. } => {
                self.hoist(l, out, true);
                self.hoist(r, out, true);
            }
            _ => {}
        }
    }

    fn literal(&mut self, lit: &Lit, outer: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        Ok(match lit {
            Lit::Group { alts, neg } => {
                if let Some(Literal::Cmp { l, op, .. }) = self.test_set(alts, *neg)? {
                    return Ok(vec![vec![Literal::Cmp { l, op, r: Term::Int(if *neg { 0 } else { 1 }) }]]);
                }
                // `!(a, b)`: no row of the conjunction, through a helper relation of the variables it shares with the
                // rule; `!(a ; b)` is `!a, !b`.
                if *neg {
                    let mut out = Vec::new();
                    for alt in alts {
                        out.push(self.negated_conjunction(alt, outer)?);
                    }
                    return Ok(vec![out]);
                }
                let mut out = Vec::new();
                for alt in alts {
                    out.extend(self.conjunction(alt, outer)?);
                }
                out
            }
            Lit::Pattern { pattern, .. } => return not_yet(format!("the pattern `{pattern}`")),
            Lit::Atom { rel, args, neg } if matches!(rel.as_str(), "nearest" | "inside" | "has" | "within_stop") && args.len() == 3 && self.node_set(&args[2]).is_some() => {
                self.node_axis(rel, args, *neg, outer)?
            }
            Lit::Atom { rel, args, neg } if matches!(rel.as_str(), "within" | "within_stop") => self.within(rel, args, *neg)?,
            Lit::Atom { rel, args, neg } if rel == "split_dot" => self.split_dot(args, *neg)?,
            Lit::Atom { rel, args, neg: false } if rel == "match" && matches!(args.as_slice(), [STerm::Str(re), _] if !groups(re).is_empty()) => self.captures(args)?,
            Lit::Atom { rel, args, neg } => {
                // core.rr's tree arithmetic (`earlier`, `precedes`, …) as the relations the engine has (`core`).
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

    /// `!(l, …)`: a helper relation of the conjunction, over the variables it shares with the rule (an aggregate's
    /// inside it too: `shared_vars`), bound inside it by a context relation where the conjunction only tests them,
    /// negated. A variable only the conjunction has is its own (Soufflé's scoping).
    fn negated_conjunction(&mut self, lits: &[Lit], outer: &BTreeSet<String>) -> Result<Literal, Error> {
        let shared: Vec<String> = shared_vars(lits).into_iter().filter(|v| outer.contains(v)).collect();
        let bound: BTreeSet<String> = body_vars(&lits.iter().filter(|l| binds_columns(l)).cloned().collect::<Vec<_>>());
        let needed: Vec<String> = shared.iter().filter(|v| !bound.contains(*v)).cloned().collect();
        let siblings = self.siblings.clone();
        let mut bodies = self.conjunction(lits, &shared.iter().cloned().collect())?;
        self.siblings = siblings.clone();
        if !needed.is_empty() {
            let ctx = self.context(&needed)?;
            for b in &mut bodies {
                b.insert(0, Literal::Atom(ctx.clone()));
            }
        }
        self.siblings = siblings;
        self.negate(bodies, &shared)
    }

    /// A helper relation `rr_not<k>(shared)` of the bodies, negated; `rr_not<k>(0)` when nothing is shared.
    fn negate(&mut self, bodies: Vec<Vec<Literal>>, shared: &[String]) -> Result<Literal, Error> {
        let name = self.fresh("not");
        let (cols, args): (Vec<(String, String)>, Vec<Term>) = if shared.is_empty() {
            (vec![("unit".to_owned(), "int".to_owned())], vec![Term::Int(0)])
        } else {
            (shared.iter().map(|c| (c.clone(), self.type_of(c, &bodies))).collect(), shared.iter().map(|c| Term::Var(c.clone())).collect())
        };
        self.prog.decls.insert(name.clone(), cols);
        let head = Atom { rel: name, args, neg: false, gated: false };
        for body in bodies {
            self.prog.rules.push(Rule { head: head.clone(), body });
        }
        Ok(Literal::Atom(Atom { neg: true, ..head }))
    }

    /// `precedes_stop(a, b, S)` as a relation `rr_prec_<S>(a, b)`: a and b are children of one parent, a first,
    /// and no child between them is in S (a kind, a relation of kinds, or a relation of nodes).
    fn precedes_stop(&mut self, set: &STerm) -> String {
        let label = match set {
            STerm::Str(k) | STerm::Var(k) => k.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>(),
            _ => "set".to_owned(),
        };
        let name = format!("{RESERVED}prec_{label}");
        if self.prog.decls.contains_key(&name) {
            return name;
        }
        let v = |s: &str| Term::Var(s.to_owned());
        let atom = |rel: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: rel.to_owned(), args, neg, gated: false });
        let cmp = |l: &str, r: &str| Literal::Cmp { l: v(l), op: "<", r: v(r) };
        let member = match (set, self.node_set(set)) {
            (_, Some(nodes)) => vec![atom(&nodes, vec![v("s")], false)],
            (STerm::Str(k), _) => vec![atom("kind", vec![v("s"), Term::Str(k.clone())], false)],
            (STerm::Var(kinds), _) => vec![atom("kind", vec![v("s"), v("sk")], false), atom(kinds, vec![v("sk")], false)],
            _ => vec![],
        };
        // after(a, k): a later sibling of a at index k is in S; first(a, j): the first such.
        let after = format!("{RESERVED}prec_after_{label}");
        let first = format!("{RESERVED}prec_first_{label}");
        let node_int = vec![("a".to_owned(), "node".to_owned()), ("k".to_owned(), "int".to_owned())];
        self.prog.decls.insert(after.clone(), node_int.clone());
        self.prog.decls.insert(first.clone(), node_int);
        self.prog.decls.insert(name.clone(), vec![("a".to_owned(), "node".to_owned()), ("b".to_owned(), "node".to_owned())]);
        let head = |rel: &str, args: Vec<Term>| Atom { rel: rel.to_owned(), args, neg: false, gated: false };
        let mut body = vec![atom("child", vec![v("p"), v("a"), v("i")], false), atom("child", vec![v("p"), v("s"), v("k")], false), cmp("i", "k")];
        body.extend(member);
        self.prog.rules.push(Rule { head: head(&after, vec![v("a"), v("k")]), body });
        self.prog.rules.push(Rule {
            head: head(&first, vec![v("a"), v("j")]),
            body: vec![
                atom(&after, vec![v("a"), Term::Wild], false),
                Literal::Agg { var: "j".to_owned(), op: AggOp::Min, over: Some("k".to_owned()), atom: head(&after, vec![v("a"), v("k")]) },
            ],
        });
        let pair = || vec![atom("child", vec![v("p"), v("a"), v("i")], false), atom("child", vec![v("p"), v("b"), v("l")], false), cmp("i", "l")];
        let mut none = pair();
        none.push(atom(&after, vec![v("a"), Term::Wild], true));
        self.prog.rules.push(Rule { head: head(&name, vec![v("a"), v("b")]), body: none });
        let mut upto = pair();
        upto.push(atom(&first, vec![v("a"), v("j")], false));
        upto.push(Literal::Cmp { l: v("l"), op: "<=", r: v("j") });
        self.prog.rules.push(Rule { head: head(&name, vec![v("a"), v("b")]), body: upto });
        name
    }

    /// A set argument that names a relation of nodes (`.decl block_node(n: node)`), not of kinds.
    fn node_set(&self, set: &STerm) -> Option<String> {
        let STerm::Var(name) = set else { return None };
        let cols = self.prog.decls.get(name)?;
        (cols.len() == 1 && cols[0].1 == "node").then(|| name.clone())
    }

    /// `tape(n, par, size, depth)`, read by the tree's ranges and climbs.
    fn need_tape(&mut self) {
        self.prog.decls.entry("tape".to_owned()).or_insert_with(|| ["n", "par", "size", "depth"].iter().map(|c| ((*c).to_owned(), "int".to_owned())).collect());
        if !self.prog.inputs.iter().any(|i| i == "tape") {
            self.prog.inputs.push("tape".to_owned());
        }
    }

    /// A climb up the tree from the nodes `seed` gives (in its variable `x`): `rr_up<k>(n, a)` for each strict
    /// ancestor a of n, up to and including the first node of `stop` where one is given. Its work is each seed's
    /// path up (to the stop), from the tape's parent column.
    fn climb(&mut self, seed: Vec<Literal>, x: &str, stop: Option<&str>) -> String {
        self.need_tape();
        let rel = self.fresh("up");
        self.prog.decls.insert(rel.clone(), vec![("n".to_owned(), "node".to_owned()), ("a".to_owned(), "node".to_owned())]);
        let v = |s: &str| Term::Var(s.to_owned());
        let atom = |r: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: r.to_owned(), args, neg, gated: false });
        let up = |p: &str, q: &str| atom("tape", vec![v(p), v(q), Term::Wild, Term::Wild], false);
        let real = |q: &str| Literal::Cmp { l: v(q), op: ">=", r: Term::Int(0) };
        let mut first = seed;
        first.extend([up(x, "rr_p"), real("rr_p")]);
        self.prog.rules.push(Rule { head: Atom { rel: rel.clone(), args: vec![v(x), v("rr_p")], neg: false, gated: false }, body: first });
        let mut next = vec![atom(&rel, vec![v("rr_n"), v("rr_p")], false)];
        if let Some(stop) = stop {
            next.push(atom(stop, vec![v("rr_p")], true));
        }
        next.extend([up("rr_p", "rr_q"), real("rr_q")]);
        self.prog.rules.push(Rule { head: Atom { rel: rel.clone(), args: vec![v("rr_n"), v("rr_q")], neg: false, gated: false }, body: next });
        rel
    }

    /// The nodes a climb starts from: the values the rule gives `n` (`context`), or every node.
    fn climb_seed(&mut self, n: &str) -> Result<Vec<Literal>, Error> {
        if self.bound_beside(n) {
            let siblings = self.siblings.clone();
            let ctx = self.context(&[n.to_owned()]);
            self.siblings = siblings;
            return Ok(vec![Literal::Atom(ctx?)]);
        }
        Ok(vec![Literal::Atom(Atom { rel: "node".to_owned(), args: vec![Term::Wild, Term::Var(n.to_owned()), Term::Wild, Term::Wild, Term::Wild], neg: false, gated: false })])
    }

    /// The axes over a set of nodes S (`core.rr`: a set may be a relation of nodes), as climbs up the tree:
    /// - `inside(n, a, S)`: a is an ancestor of n, and S(a);
    /// - `nearest(n, a, S)`: the climb from n stops at the first node of S, which is a;
    /// - `has(a, n, S)`: the climb from each node of S, n, passes a;
    /// - `within_stop(n, a, S)`: n is below a, and the climb from n meets no node of S before a.
    ///
    /// Negated, each is a helper relation (`negate`).
    fn node_axis(&mut self, rel: &str, args: &[STerm], neg: bool, outer: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        let set = self.node_set(&args[2]).expect("a node set");
        let atom = |r: &str, args: Vec<Term>| Literal::Atom(Atom { rel: r.to_owned(), args, neg: false, gated: false });
        let named = |me: &mut Self, t: &STerm| -> Result<String, Error> {
            match t {
                STerm::Var(v) => Ok(v.clone()),
                STerm::Wild => Ok(me.fresh("w")),
                other => Err(Error::Syntax(format!("{rel}: expected a variable, got {other:?}"))),
            }
        };
        let (first, second) = (named(self, &args[0])?, named(self, &args[1])?);
        let v = |s: &str| Term::Var(s.to_owned());
        let positive = match rel {
            "inside" | "nearest" => {
                let (n, a) = (first.clone(), second.clone());
                let seed = self.climb_seed(&n)?;
                let up = self.climb(seed, &n, (rel == "nearest").then_some(set.as_str()));
                vec![vec![atom(&up, vec![v(&n), v(&a)]), atom(&set, vec![v(&a)])]]
            }
            "has" => {
                let (a, n) = (first.clone(), second.clone());
                let up = self.climb(vec![atom(&set, vec![v(&n)])], &n, None);
                vec![vec![atom(&up, vec![v(&n), v(&a)])]]
            }
            _ => {
                let (n, a) = (first.clone(), second.clone());
                let below = self.within("within", &[STerm::Var(n.clone()), STerm::Var(a.clone())], false)?;
                let seed = self.climb_seed(&n)?;
                let up = self.climb(seed, &n, Some(&set));
                let s = self.fresh("stop");
                let hit = vec![atom(&up, vec![v(&n), v(&s)]), atom(&set, vec![v(&s)])];
                let none = self.negate(vec![hit.clone()], std::slice::from_ref(&n))?;
                let mut out = Vec::new();
                for b in below {
                    out.push(b.iter().cloned().chain([none.clone()]).collect());
                    out.push(b.into_iter().chain(hit.iter().cloned()).chain([Literal::Cmp { l: v(&s), op: "<=", r: v(&a) }]).collect());
                }
                out
            }
        };
        if !neg {
            return Ok(positive);
        }
        let shared: Vec<String> = [&first, &second].into_iter().filter(|x| outer.contains(*x)).cloned().collect();
        Ok(vec![vec![self.negate(positive, &shared)?]])
    }

    /// `within(n, a)`: n is strictly below a, the engine's relation read off the tree (decision 0050): a's
    /// subtree for a bound `a`, n's ancestors for a bound `n`, made per node looked up. No range over ids and no
    /// climb through `child` (`core.rr`: tape arithmetic, never Datalog recursion). `within_stop(n, a, stop)`: also
    /// no node of `stop` strictly between them. The ancestors of n are ordered by id, so that is: n has no
    /// ancestor in `stop`, or its nearest one is a or above it. Negated (a named): n is not below a, or it is with
    /// its nearest ancestor in `stop` below a.
    fn within(&mut self, rel: &str, args: &[STerm], neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        let (n, a, stop) = match args {
            [n, a] if rel == "within" => (n, a, None),
            [n, a, stop] if rel == "within_stop" => (n, a, Some(stop)),
            _ => return not_yet(format!("{}{rel}/{}", if neg { "!" } else { "" }, args.len())),
        };
        if neg && stop.is_some() && *a == STerm::Wild {
            return not_yet(format!("`!{rel}` with its ancestor unnamed"));
        }
        let (n, a) = (self.term(n)?, self.term(a)?);
        let within = crate::code_rule_dsl::facts::vtab::syntax::WITHIN;
        self.prog.decls.insert(within.to_owned(), vec![("n".to_owned(), "node".to_owned()), ("a".to_owned(), "node".to_owned())]);
        if !self.prog.inputs.iter().any(|i| i == within) {
            self.prog.inputs.push(within.to_owned());
        }
        let below = |neg: bool| vec![Literal::Atom(Atom { rel: within.to_owned(), args: vec![n.clone(), a.clone()], neg, gated: false })];
        let cmp = |l: &Term, op: &'static str, r: &Term| Literal::Cmp { l: l.clone(), op, r: r.clone() };
        let Some(stop) = stop else { return Ok(vec![below(neg)]) };
        let set = self.set_name("within_stop", stop)?;
        let Term::Var(nv) = &n else { return Err(Error::Syntax("within_stop: the node is a variable".to_owned())) };
        let s = self.fresh("stop");
        let nearest = Literal::Nearest { n: nv.clone(), a: s.clone(), kindset: set.clone() };
        if neg {
            return Ok(vec![below(true), below(false).into_iter().chain([nearest, cmp(&Term::Var(s), ">", &a)]).collect()]);
        }
        let none = Literal::Atom(Atom { rel: format!("{AXIS}{set}"), args: vec![n.clone(), Term::Wild], neg: true, gated: false });
        Ok(vec![below(false).into_iter().chain([none]).collect(), below(false).into_iter().chain([nearest, cmp(&Term::Var(s), "<=", &a)]).collect()])
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

    /// A set argument's relation: a relation's name (of kinds, `kind_sets` tells, or of nodes, `node_set`), or one
    /// kind as its own set.
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
            _ => Err(Error::Syntax(format!("{rel}: the set is a relation of kinds or one kind"))),
        }
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
            // `!nearest(n, s, set)` with s named: not that nearest, through a helper relation over (n, s).
            ("nearest", [n, a, set]) => {
                let kindset = self.set_name(rel, set)?;
                let (n, a) = (var(n)?, var(a)?);
                let shared = [n.clone(), a.clone()];
                return self.negate(vec![vec![Literal::Nearest { n, a, kindset }]], &shared);
            }
            // `inside(n, a, set)`: a is an ancestor of n in the set; `has(a, n, set)`: n is a descendant of a in the
            // set. Both are `(n, a)` rows the transducer makes.
            ("inside" | "has", [x, y, set]) => {
                let set = self.set_name(rel, set)?;
                let (x, y) = (self.term(x)?, self.term(y)?);
                let (axis, args) = if rel == "inside" { (AXIS, vec![x, y]) } else { (DESCENDANTS, vec![y, x]) };
                return Ok(Literal::Atom(Atom { rel: format!("{axis}{set}"), args, neg, gated: false }));
            }
            ("ast", _) if !neg => return self.ast(args),
            ("precedes_stop", [a, b, set]) => {
                let rel = self.precedes_stop(set);
                return Ok(Literal::Atom(Atom { rel, args: vec![self.term(a)?, self.term(b)?], neg, gated: false }));
            }
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
        // A float compared with a whole number: the number as a float (`to_float(t) > 86400`).
        let as_float = |t: &STerm| match t {
            STerm::Num(n) => STerm::Float(format!("{n}.0")),
            t => t.clone(),
        };
        let (l, r) = if is_float(l) { (l.clone(), as_float(r)) } else if is_float(r) { (as_float(l), r.clone()) } else { (l.clone(), r.clone()) };
        let (l, r) = (&l, &r);
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
        let binds = |l: &Lit| matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none() && rel != "earlier");
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

    /// Whether a positive atom beside the literal being lowered (not a text test or `earlier`) binds `v`. `within`
    /// and `within_stop` bind both ends: they are the engine's relation (decision 0050).
    fn bound_beside(&self, v: &str) -> bool {
        let atoms: Vec<Lit> = self
            .siblings
            .iter()
            .filter(|l| matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none() && rel != "earlier"))
            .cloned()
            .collect();
        body_vars(&atoms).contains(v)
    }

    #[allow(clippy::too_many_arguments)]
    fn aggregate(&mut self, var: &str, agg: &str, over: Option<&STerm>, by: &[STerm], sep: &Option<String>, body: &[Lit], outer: &BTreeSet<String>) -> Result<Vec<Literal>, Error> {
        // A value or key that is a term (`sum strlen(x) - strlen(a)`) is a variable the body binds.
        let mut body = body.to_vec();
        let mut variable = |t: &STerm, me: &mut Self| match t {
            STerm::Var(v) => v.clone(),
            t => {
                let v = me.fresh("val");
                body.push(Lit::Cmp { l: STerm::Var(v.clone()), op: "=", r: t.clone() });
                v
            }
        };
        let over = over.map(|t| variable(t, self));
        let by: Vec<String> = by.iter().map(|t| variable(t, self)).collect();
        let body = body.as_slice();
        let op = match agg {
            "count" => AggOp::Count,
            "min" => AggOp::Min,
            "max" => AggOp::Max,
            "sum" => AggOp::Sum,
            // With no `sep`, the texts are joined as they are.
            "join" => AggOp::Join { by: by.clone(), sep: sep.clone().unwrap_or_default() },
            // Soufflé's mean is a decimal number, and the engine has none.
            "mean" => return not_yet("the aggregate mean (a decimal number)"),
            other => return not_yet(format!("the aggregate {other}")),
        };
        if (!by.is_empty() || sep.is_some()) && agg != "join" {
            return Err(Error::Syntax(format!("{agg} takes no `by` or `sep`")));
        }
        if op != AggOp::Count && over.is_none() {
            return Err(Error::Syntax(format!("{agg} needs the variable it aggregates")));
        }
        // The body's own variables the aggregate reads: its value, and a join's keys.
        let own = |v: &String| over.as_ref() == Some(v) || by.contains(v);
        let ours = |v: &String| !outer.contains(v);
        // One plain atom: the engine's aggregate reads it. The built-ins lowering writes out (axes, core.rr's tree
        // arithmetic, …) are no relations of the program, so a body of one of them goes through the helper.
        if let [Lit::Atom { rel, args, neg: false }] = body
            && test_of(rel).is_none()
            && !matches!(rel.as_str(), "nearest" | "inside" | "has" | "split_dot" | "ast" | "within" | "within_stop" | "precedes_stop")
            && !self.rewritten(rel)
        {
            let Literal::Atom(mut atom) = self.atom(rel, args, false)? else { unreachable!("a plain atom") };
            let at = |v: &String| atom.args.iter().position(|t| matches!(t, Term::Var(w) if w == v));
            // A variable of the body's own the atom repeats is an equality the engine's aggregate would not test, and
            // a key or value the atom lacks is the helper's error below.
            let repeated = atom.vars().iter().any(|v| ours(v) && atom.args.iter().filter(|t| matches!(t, Term::Var(w) if w == v)).count() > 1);
            let placed = over.iter().chain(&by).all(|v| at(v).is_some());
            if !repeated && placed {
                // A variable only the body uses is counted over, not grouped by: a wildcard to the engine.
                for t in &mut atom.args {
                    if matches!(t, Term::Var(v) if ours(v) && !own(v)) {
                        *t = Term::Wild;
                    }
                }
                return Ok(vec![Literal::Agg { var: var.to_owned(), op, over, atom }]);
            }
        }
        // Otherwise a helper relation of the body. `count` and `sum` range over the body's rows, a multiset of the
        // aggregated value, so they keep every variable; `min`, `max` and `join` (of distinct rows) keep those they
        // read (the aggregated one and a join's keys) and those the rule shares.
        let inner = body_vars(body);
        let cols: Vec<String> = inner.iter().filter(|v| matches!(op, AggOp::Count | AggOp::Sum) || own(v) || outer.contains(*v)).cloned().collect();
        if let Some(v) = over.iter().chain(&by).find(|v| !cols.contains(v)) {
            return Err(Error::Syntax(format!("{agg}: {v} is not bound in its body")));
        }
        let name = self.fresh("agg");
        let mut bodies = self.conjunction(body, outer)?;
        // A shared variable the body only tests (`count : { slash_at(ps, j), i < j }`) is bound inside the helper by
        // a context relation: the rule's other literals, projected on those variables.
        let bound: BTreeSet<String> = body_vars(&body.iter().filter(|l| binds_columns(l)).cloned().collect::<Vec<_>>());
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
        // The helper keeps every column (`count` counts distinct rows), but only the rule's variables group it.
        let args = cols.iter().map(|c| if outer.contains(c) || own(c) { Term::Var(c.clone()) } else { Term::Wild }).collect();
        Ok(vec![Literal::Agg { var: var.to_owned(), op, over, atom: Atom { rel: name, args, neg: false, gated: false } }])
    }

    /// A helper column's type: the column it fills in the body's atoms, else the type of the aggregate or the
    /// computation that assigns it (`k = 0 - i` is a number, which a `join` orders by value).
    fn type_of(&self, v: &str, bodies: &[Vec<Literal>]) -> String {
        for literal in bodies.iter().flatten() {
            match literal {
                Literal::Atom(a) if !a.neg => {
                    for (i, t) in a.args.iter().enumerate() {
                        if *t == Term::Var(v.to_owned()) {
                            if crate::code_rule_dsl::logic::front::is_axis(&a.rel) {
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
                // A computed value: a number from arithmetic and the functions of numbers, else a symbol.
                Literal::Cmp { l: Term::Var(x), op: "=", r: r @ (Term::Int(_) | Term::Call(..)) } if x == v => {
                    let number = match r {
                        Term::Int(_) => true,
                        Term::Call(f, _) => {
                            matches!(f.as_str(), "add" | "sub" | "mul" | "div" | "mod" | "max" | "min" | "strlen" | "to_number" | "to_float" | "count_matches")
                                || f.starts_with("float_") && f != "float_str"
                        }
                        _ => false,
                    };
                    return if number { "int" } else { "sym" }.to_owned();
                }
                Literal::Cmp { l: Term::Var(x), op: "=", r: Term::Str(_) } if x == v => return "sym".to_owned(),
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
                Literal::Ast { document, .. } if document == v => return "int".to_owned(),
                Literal::Ast { node, captures, .. } if node == v || captures.iter().any(|(_, c)| c == v) => return "node".to_owned(),
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
                // Soufflé's functions: `cat` joins strings (the engine's `cat` joins with a dot, so it is `concat`).
                // `to_string` of a float writes the float (`float_str`); `max` and `min` of two are SQL's.
                let f = match (name.as_str(), args.len()) {
                    ("cat", n) if n >= 2 => "concat",
                    ("to_string", 1) if is_float(&args[0]) => "float_str",
                    ("strlen" | "to_string" | "to_number" | "to_float", 1) => name.as_str(),
                    ("substr", 3) => "substr",
                    ("max" | "min", 2) => name.as_str(),
                    _ => return not_yet(format!("the built-in function {name}/{}", args.len())),
                };
                let args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
                // `cat(a, b, c)` is `cat(cat(a, b), c)`.
                if f == "concat" {
                    let mut args = args.into_iter();
                    let first = args.next().expect("two or more");
                    return Ok(args.fold(first, |acc, a| Term::Call("concat".to_owned(), vec![acc, a])));
                }
                Term::Call(f.to_owned(), args)
            }
            STerm::Bin { op, l, r } => {
                // Float arithmetic is on floats' keys (`strings::functions::float_key`), by functions of them.
                let float = is_float(l) || is_float(r);
                let f = match *op {
                    "+" if float => "float_add",
                    "-" if float => "float_sub",
                    "*" if float => "float_mul",
                    "/" if float => "float_div",
                    "+" => "add",
                    "-" => "sub",
                    "*" => "mul",
                    "/" => "div",
                    "%" if !float => "mod",
                    other => return not_yet(format!("arithmetic ({other})")),
                };
                Term::Call(f.to_owned(), vec![self.term(l)?, self.term(r)?])
            }
            STerm::Agg { op, .. } => return not_yet(format!("the aggregate {op} as a term")),
            STerm::Tick(t) => return not_yet(format!("the template `{t}`")),
            // A decimal number is its float's key, which orders as the floats do.
            STerm::Float(f) => Term::Int(float_key(f.parse::<f64>().map_err(|_| Error::Syntax(format!("bad decimal number {f}")))?)),
            STerm::Json(_) | STerm::Named(..) => return Err(Error::Syntax("a JSON term or an `ast` capture outside `ast(…)`".to_owned())),
        })
    }

    /// Kind sets: the relations of facts given to `nearest`, `inside` and `has`. One read nowhere else is only a set.
    /// A set of kinds that rules derive is no kind set: its axes are rules (`derived_axes`). (A set of nodes is read
    /// as climbs where it is given, `node_axis`.)
    fn kind_sets(&mut self) -> Result<(), Error> {
        let mut derived = BTreeSet::new();
        for set in std::mem::take(&mut self.sets) {
            if self.prog.kindsets.contains_key(&set) {
                continue;
            }
            let kinds = match self.members(&set)? {
                Members::Kinds(kinds) => kinds,
                Members::Derived => {
                    derived.insert(set);
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
        Ok(())
    }

    /// What a set holds: its kinds when they are known now (a kind set), else kinds its rules (or an input) give.
    fn members(&self, set: &str) -> Result<Members, Error> {
        if let Some(kinds) = self.kinds_of(set, &mut BTreeSet::new()) {
            return Ok(Members::Kinds(kinds));
        }
        if matches!(self.prog.decls.get(set).map(Vec::as_slice), Some([(_, t)]) if t == "sym") {
            return Ok(Members::Derived);
        }
        Err(Error::Syntax(format!("the kind set {set} is not a relation of kinds (one column of facts, or rules `{set}(k) :- s(k)` over such sets)")))
    }

    /// The axes over sets of kinds rules derive, which the transducer cannot read from a node's kind: rules over
    /// `child`, in which each node's membership (`rr_in_<s>(x) :- node(_, x, k, _, _), s(k)`) is read once. A helper
    /// reads its set negated, so the set is complete before it (a lower stratum; a set that depends on its own axis
    /// is refused by the strata).
    /// - `rr_near_<s>(n, a)`, a is n's nearest strict ancestor in s (`nearest`, and `inside(n, _, s)`): down from each
    ///   member to the next, a row per node below one;
    /// - `rr_anc_<s>(n, a)`, every strict ancestor a of n in s (`inside`): the chain of nearest ones;
    /// - `rr_desc_<s>(n, a)`, n in s and a any strict ancestor of it (`has`): up from each member;
    /// - `rr_above_<s>(a)`, a has a member strictly below it (`has(a, _, s)`).
    fn derived_axes(&mut self, derived: &BTreeSet<String>) {
        let mut need = BTreeSet::new();
        for rule in &mut self.prog.rules {
            for l in &mut rule.body {
                let (set, helper, args, neg) = match l {
                    Literal::Nearest { n, a, kindset } if derived.contains(kindset) => (kindset.clone(), "near", vec![Term::Var(n.clone()), Term::Var(a.clone())], false),
                    Literal::Atom(x) => match axis(&x.rel) {
                        Some((set, has)) if derived.contains(set) => {
                            let (helper, args) = match (has, x.args.as_slice()) {
                                (false, [_, Term::Wild]) => ("near", x.args.clone()),
                                (false, _) => ("anc", x.args.clone()),
                                (true, [Term::Wild, a]) => ("above", vec![a.clone()]),
                                (true, _) => ("desc", x.args.clone()),
                            };
                            (set.to_owned(), helper, args, x.neg)
                        }
                        _ => continue,
                    },
                    _ => continue,
                };
                *l = Literal::Atom(Atom { rel: format!("rr_{helper}_{set}"), args, neg, gated: false });
                if helper == "anc" {
                    need.insert((set.clone(), "near"));
                }
                need.insert((set, helper));
            }
        }
        if need.is_empty() {
            return;
        }
        for rel in ["child", "node"] {
            self.base(rel);
        }
        let v = |s: &str| Term::Var(s.to_owned());
        let atom = |rel: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: rel.to_owned(), args, neg, gated: false });
        let child = |p: &str, c: &str| atom("child", vec![v(p), v(c), Term::Wild], false);
        for (set, helper) in need {
            let member = format!("rr_in_{set}");
            if !self.prog.decls.contains_key(&member) {
                self.prog.decls.insert(member.clone(), vec![("x".to_owned(), "node".to_owned())]);
                let body = vec![atom("node", vec![Term::Wild, v("x"), v("k"), Term::Wild, Term::Wild], false), atom(&set, vec![v("k")], false)];
                self.prog.rules.push(Rule { head: Atom { rel: member.clone(), args: vec![v("x")], neg: false, gated: false }, body });
            }
            let is = |x: &str, neg: bool| atom(&member, vec![v(x)], neg);
            let rel = format!("rr_{helper}_{set}");
            let me = |args: &[&str]| atom(&rel, args.iter().map(|a| v(a)).collect(), false);
            let (cols, bodies): (&[&str], Vec<Vec<Literal>>) = match helper {
                "near" => (&["n", "a"], vec![vec![child("a", "n"), is("a", false)], vec![child("p", "n"), is("p", true), me(&["p", "a"])]]),
                "anc" => (&["n", "a"], vec![vec![atom(&format!("rr_near_{set}"), vec![v("n"), v("a")], false)], vec![atom(&format!("rr_near_{set}"), vec![v("n"), v("b")], false), me(&["b", "a"])]]),
                "desc" => (&["n", "a"], vec![vec![is("n", false), child("a", "n")], vec![me(&["n", "p"]), child("a", "p")]]),
                _ => (&["a"], vec![vec![is("n", false), child("a", "n")], vec![me(&["p"]), child("a", "p")]]),
            };
            self.prog.decls.insert(rel.clone(), cols.iter().map(|c| ((*c).to_owned(), "node".to_owned())).collect());
            let head = Atom { rel: rel.clone(), args: cols.iter().map(|c| v(c)).collect(), neg: false, gated: false };
            for body in bodies {
                self.prog.rules.push(Rule { head: head.clone(), body });
            }
        }
    }

    /// A base relation lowering reads (`child`, `node`), declared, and an input, when the program has not.
    fn base(&mut self, rel: &str) {
        let cols: &[(&str, &str)] = match rel {
            "child" => &[("p", "node"), ("c", "node"), ("i", "int")],
            "node" => &[("d", "int"), ("n", "node"), ("kind", "sym"), ("start", "int"), ("end", "int")],
            other => unreachable!("no base relation {other}"),
        };
        self.prog.decls.entry(rel.to_owned()).or_insert_with(|| cols.iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
        if !self.prog.inputs.iter().any(|i| i == rel) && !self.prog.rules.iter().any(|r| r.head.rel == rel) {
            self.prog.inputs.push(rel.to_owned());
        }
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

/// Whether a term is a float: a decimal number, `to_float(…)`, or arithmetic on one. (A variable's type is not
/// followed: `0.0 - f` is a float, `f - g` is not.)
fn is_float(t: &STerm) -> bool {
    match t {
        STerm::Float(_) => true,
        STerm::Call { name, functor: false, args } => name == "to_float" || (matches!(name.as_str(), "max" | "min") && args.iter().any(is_float)),
        STerm::Bin { l, r, .. } => is_float(l) || is_float(r),
        _ => false,
    }
}

/// A float as an integer key that orders as the floats do (decision 0008: the engine's values are integers).
pub(super) fn float_key(f: f64) -> i64 {
    crate::code_rule_dsl::logic::strings::functions::float_key(f)
}

/// The empty value `unset(x)` binds (`core.rr`): the id -1, which no node has, so no tree relation holds of it.
/// Each `unset(x)` is taken out and `x` replaced by it in the rule; `!unset(x)` is `x != -1`. Its text is "":
/// `text(x, t)` of it is `t = ""` (O77). (A `@place` with no valid line is `unset` too: `PLACE`.)
pub(super) const UNSET: i64 = -1;

fn unset(mut head: Vec<Term>, body: Vec<Literal>) -> Result<(Vec<Term>, Vec<Literal>), Error> {
    let mut vars = BTreeSet::new();
    let mut rest = Vec::new();
    for l in body {
        match l {
            Literal::Atom(a) if a.rel == "unset" && !a.neg => match a.args.as_slice() {
                [Term::Var(v)] => {
                    vars.insert(v.clone());
                }
                [Term::Wild] => {}
                _ => return not_yet("`unset` of a value"),
            },
            Literal::Atom(a) if a.rel == "unset" => match a.args.as_slice() {
                [x @ Term::Var(_)] => rest.push(Literal::Cmp { l: x.clone(), op: "!=", r: Term::Int(UNSET) }),
                _ => return not_yet("`!unset` of a value"),
            },
            l => rest.push(l),
        }
    }
    if vars.is_empty() {
        return Ok((head, rest));
    }
    fn put(t: &mut Term, vars: &BTreeSet<String>) {
        match t {
            Term::Var(v) if vars.contains(v) => *t = Term::Int(UNSET),
            Term::Call(_, args) => args.iter_mut().for_each(|a| put(a, vars)),
            _ => {}
        }
    }
    head.iter_mut().for_each(|t| put(t, &vars));
    for l in &mut rest {
        match l {
            Literal::Atom(a) => a.args.iter_mut().for_each(|t| put(t, &vars)),
            Literal::Cmp { l, r, .. } => {
                put(l, &vars);
                put(r, &vars);
            }
            other if !other.vars().is_disjoint(&vars) => return not_yet("`unset` in an aggregate, `nearest` or `ast`"),
            _ => {}
        }
    }
    // The empty value's text is "".
    let rest = rest
        .into_iter()
        .filter_map(|l| match l {
            Literal::Atom(a) if a.rel == "text" && a.args.first() == Some(&Term::Int(UNSET)) => {
                let t = a.args.get(1).cloned().unwrap_or(Term::Wild);
                match (t, a.neg) {
                    (Term::Wild, false) => None,
                    (Term::Wild, true) => Some(Literal::Cmp { l: Term::Int(1), op: "=", r: Term::Int(0) }),
                    (t, neg) => Some(Literal::Cmp { l: t, op: if neg { "!=" } else { "=" }, r: Term::Str(String::new()) }),
                }
            }
            l => Some(l),
        })
        .collect();
    Ok((head, rest))
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

/// `@place(path, line)` (`core.rr`) outside a `.violation`'s head: a place with no node, as a `loc` value:
/// `-2 - (path << 32 | line)`, `path` the path's symbol (its atom) and `0 <= line < 2^32`, so no node id
/// (`d << 32 | index`, never negative), no span and not `unset` (-1). `@place_path(x)` and `@place_line(x)` take it
/// apart; `plan`'s SQL computes all three, and the native set engine leaves rules that call them to SQL. A place with
/// no valid line is `unset`, which has no finding.
pub(super) const PLACE: &str = "place";
pub(super) const PLACE_PATH: &str = "place_path";
pub(super) const PLACE_LINE: &str = "place_line";

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
