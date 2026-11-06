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
//! - a relation of facts given to `nearest`, `inside` or `has` is a kind set;
//! - `@dot` is the engine's `@cat`, and `split_dot(q, m, x)` is `q = @dot(m, x)` (it runs either way);
//! - `count : { … }`, `min x : { … }`, `max x : { … }` are aggregates; a body of more than one atom gets a helper
//!   relation; an aggregate inside a comparison gets a variable;
//! - other disjunctions are one rule per alternative.
//!
//! - `.violation V(at: node, …)` declares V as an output, and each row of V is a finding
//!   `violation(policy, V, code, path, line)` at `at`'s line (the code is V, or the `code {c}` column);
//! - a rule ending in `=> edits` defines a fix, and fixes are not run: it is left out;
//! - `.input r(IO=…, filename="…")` is an input, its parameters kept as the setting `input`;
//! - `within(n, a)` is the engine's relation read off the tree, and `within_stop(n, a, stop)` that less a stop
//!   between (`Lowerer::within`, decision 0050);
//! - `match("(?<x>…)", s)` binds x to the named group (`Lowerer::captures`); `+ - * /` and Soufflé's `cat`,
//!   `strlen`, `to_string`, `substr` are functors.
//!
//! Not lowered yet, each a named error: patterns (``n ~ `…` ``), `must`/`may`, a `.violation` at a span or place,
//! `ordered by`, `sum`/`mean`/`join`,
//! negated disjunctions other than test sets, and the built-ins that take a relation besides the kind-set ones.

use std::collections::{BTreeMap, BTreeSet};

use super::syntax::{Head, Item, Lit, Statement, Term as STerm};
use crate::code_rule_dsl::logic::front::{AXIS, AggOp, Atom, Error, LatticeDecl, Literal, Program, Rule, Term, Value};
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
    /// `.violation`s: (relation, columns, the column holding the code).
    violations: Vec<(String, usize, Option<usize>)>,
    next: usize,
    /// The literals beside the one being lowered, in its conjunction and those enclosing it: what binds an
    /// aggregate's shared variables (`aggregate`).
    siblings: Vec<Lit>,
    /// Whether the clause being lowered binds a variable named `top` (`path_part(d, top)`); otherwise `top` is a
    /// lattice's top value (`v != top`), as the translated programs write it.
    top_is_var: bool,
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
            Item::Violation { name, cols, code, tag, .. } => l.violation(name, cols, code, tag).map_err(at)?,
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
    l.findings();
    l.projections();
    // The built-ins lowering writes out (`nearest`, `within`, …) are declared in `core.rr` for the checker; they
    // are no relations of the program.
    // `within` stays where a rule reads it: the lowering made it the engine's relation (decision 0050).
    let reads_within = l.prog.rules.iter().flat_map(|r| &r.body).filter_map(Literal::atom).any(|a| a.rel == crate::code_rule_dsl::facts::vtab::syntax::WITHIN);
    for b in RELATION_ARG.iter().map(|(r, _)| *r).chain(["within", "earlier", "split_dot", "unset"]).filter(|b| !(reads_within && *b == "within")) {
        l.prog.decls.remove(b);
        l.prog.inputs.retain(|i| i != b);
    }
    l.kind_sets()?;
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
                "widen" => widen = Some(words.next().and_then(|k| k.parse::<u32>().ok()).ok_or_else(|| Error::Syntax(format!("{name}: `widen` needs a count")))?),
                other => return not_yet(format!("`{other}` on .decl {name}")),
            }
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

    /// `core.rr`'s `kind(n, k)` and `docof(n, d)` are projections of `node`, as T's programs write them: rules, which
    /// the planner reads from `node` in place, not inputs; `precedes` is a rule over `child`.
    fn projections(&mut self) {
        let v = |s: &str| Term::Var(s.to_owned());
        for (rel, args) in [("kind", [Term::Wild, v("n"), v("k"), Term::Wild, Term::Wild]), ("docof", [v("d"), v("n"), Term::Wild, Term::Wild, Term::Wild])] {
            if !self.prog.inputs.iter().any(|i| i == rel) || !self.prog.decls.contains_key("node") {
                continue;
            }
            self.prog.inputs.retain(|i| i != rel);
            let head = if rel == "kind" { vec![v("n"), v("k")] } else { vec![v("n"), v("d")] };
            self.prog.rules.push(Rule {
                head: Atom { rel: rel.to_owned(), args: head, neg: false, gated: false },
                body: vec![Literal::Atom(Atom { rel: "node".to_owned(), args: args.to_vec(), neg: false, gated: false })],
            });
        }
        // `precedes(a, b)`: a comes before b among one parent's children.
        if self.prog.inputs.iter().any(|i| i == "precedes") && self.prog.decls.contains_key("child") {
            self.prog.inputs.retain(|i| i != "precedes");
            let child = |c: &str, i: &str| Literal::Atom(Atom { rel: "child".to_owned(), args: vec![v("p"), v(c), v(i)], neg: false, gated: false });
            self.prog.rules.push(Rule {
                head: Atom { rel: "precedes".to_owned(), args: vec![v("a"), v("b")], neg: false, gated: false },
                body: vec![child("a", "i"), child("b", "j"), Literal::Cmp { l: v("i"), op: "<", r: v("j") }],
            });
        }
    }

    /// `.violation V(at, …)`: V is an output relation, and each of its rows is a finding (`findings`).
    fn violation(&mut self, name: String, cols: Vec<(String, String)>, code: Option<String>, tag: Option<String>) -> Result<(), Error> {
        let Some((_, at)) = cols.first() else { return Err(Error::Syntax(format!(".violation {name} has no `at` column"))) };
        if at != "node" {
            return not_yet(format!(".violation {name} at a {at} (only at a node)"));
        }
        // `tag {t}` keeps findings at one place apart; the engine's findings are keyed by place and code, so
        // it is checked and not carried.
        for c in [&code, &tag].into_iter().flatten() {
            if !cols.iter().any(|(n, _)| n == c) {
                return Err(Error::Syntax(format!(".violation {name}: no column {c}")));
            }
        }
        let code = code.map(|c| cols.iter().position(|(n, _)| *n == c).expect("a column"));
        self.violations.push((name.clone(), cols.len(), code));
        self.decl(name.clone(), cols, &[])?;
        self.prog.outputs.push(name);
        Ok(())
    }

    /// The findings of the `.violation`s: `violation(policy, V, code, path, line)` for each row of V, at its first
    /// column's line (the engine's findings relation, as the converter's programs give it).
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
        for (name, arity, code) in std::mem::take(&mut self.violations) {
            let cols: Vec<Term> = (0..arity).map(|i| if i == 0 || Some(i) == code { v(&format!("rr_v{i}")) } else { Term::Wild }).collect();
            let code = code.map_or_else(|| Term::Str(name.clone()), |i| v(&format!("rr_v{i}")));
            let atom = |rel: &str, args: Vec<Term>| Literal::Atom(Atom { rel: rel.to_owned(), args, neg: false, gated: false });
            self.prog.rules.push(Rule {
                head: Atom { rel: "violation".to_owned(), args: vec![Term::Str(policy.clone()), Term::Str(name.clone()), code, v("rr_p"), v("rr_l")], neg: false, gated: false },
                body: vec![
                    atom(&name, cols),
                    atom("node", vec![v("rr_d"), v("rr_v0"), Term::Wild, Term::Wild, Term::Wild]),
                    atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild]),
                    atom("line", vec![v("rr_v0"), v("rr_l")]),
                ],
            });
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
        let mut outer = Vec::new();
        heads.iter().flat_map(|h| &h.args).for_each(|a| a.vars(&mut outer));
        let bodies = self.conjunction(body, &outer.into_iter().collect())?;
        for head in heads {
            if let Some(m) = &head.modal {
                return not_yet(format!("`{m}` head {}", head.rel));
            }
            let args = head.args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
            for body in &bodies {
                let (args, body) = unset(args.clone(), body.clone())?;
                self.prog.rules.push(Rule { head: Atom { rel: head.rel.clone(), args, neg: false, gated: false }, body });
            }
        }
        Ok(())
    }

    /// A conjunction's alternatives (one per choice in its disjunctions).
    /// `context`: the variables used outside the conjunction.
    fn conjunction(&mut self, lits: &[Lit], context: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
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

    fn literal(&mut self, lit: &Lit, outer: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        Ok(match lit {
            Lit::Group { alts, neg } => {
                if let Some(Literal::Cmp { l, op, .. }) = self.test_set(alts)? {
                    return Ok(vec![vec![Literal::Cmp { l, op, r: Term::Int(if *neg { 0 } else { 1 }) }]]);
                }
                if *neg {
                    return not_yet("a negated disjunction");
                }
                let mut out = Vec::new();
                for alt in alts {
                    out.extend(self.conjunction(alt, outer)?);
                }
                out
            }
            Lit::Pattern { pattern, .. } => return not_yet(format!("the pattern `{pattern}`")),
            Lit::Atom { rel, args, neg } if matches!(rel.as_str(), "within" | "within_stop") => self.within(rel, args, *neg)?,
            Lit::Atom { rel, args, neg } if rel == "earlier" => self.earlier(args, *neg)?,
            Lit::Atom { rel, args, neg: false } if rel == "match" && matches!(args.as_slice(), [STerm::Str(re), _] if !groups(re).is_empty()) => self.captures(args)?,
            Lit::Atom { rel, args, neg } => {
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

    /// `within(n, a)`: n is strictly below a, the engine's relation read off the tree (decision 0050): a's
    /// subtree for a bound `a`, n's ancestors for a bound `n`, made per node looked up. No range over ids and no
    /// climb through `child` (`core.rr`: tape arithmetic, never Datalog recursion). `within_stop(n, a, stop)`: also
    /// no node of `stop` strictly between them. The ancestors of n are ordered by id, so that is: n has no
    /// ancestor in `stop`, or its nearest one is a or above it.
    fn within(&mut self, rel: &str, args: &[STerm], neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        let (n, a, stop) = match args {
            [n, a] if rel == "within" => (n, a, None),
            [n, a, stop] if rel == "within_stop" && !neg => (n, a, Some(stop)),
            _ => return not_yet(format!("{}{rel}/{}", if neg { "!" } else { "" }, args.len())),
        };
        let (n, a) = (self.term(n)?, self.term(a)?);
        let within = crate::code_rule_dsl::facts::vtab::syntax::WITHIN;
        self.prog.decls.insert(within.to_owned(), vec![("n".to_owned(), "node".to_owned()), ("a".to_owned(), "node".to_owned())]);
        if !self.prog.inputs.iter().any(|i| i == within) {
            self.prog.inputs.push(within.to_owned());
        }
        let below = vec![Literal::Atom(Atom { rel: within.to_owned(), args: vec![n.clone(), a.clone()], neg, gated: false })];
        let cmp = |l: &Term, op: &'static str, r: &Term| Literal::Cmp { l: l.clone(), op, r: r.clone() };
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

    /// `earlier(m, n)`: m comes before n in one document. A node is `d << 32 | its preorder index`, so that is
    /// `docof(m, d), docof(n, d), m < n`: document order. CPython's visit order differs from it for a few
    /// children (a def's decorators and return annotation after its body, `core.rr`); that is not followed yet.
    fn earlier(&mut self, args: &[STerm], neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        let [m, n] = args else { return not_yet(format!("earlier/{}", args.len())) };
        if neg {
            return not_yet("!earlier");
        }
        let (m, n) = (self.term(m)?, self.term(n)?);
        let d = Term::Var(self.fresh("doc"));
        let docof = |x: &Term| Literal::Atom(Atom { rel: "docof".to_owned(), args: vec![x.clone(), d.clone()], neg: false, gated: false });
        Ok(vec![vec![docof(&m), docof(&n), Literal::Cmp { l: m.clone(), op: "<", r: n.clone() }]])
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

    /// `(glob("a", s) ; glob("b", s))` as one test of the set, and `(s = "a" ; s = "b")` as one membership.
    fn test_set(&mut self, alts: &[Vec<Lit>]) -> Result<Option<Literal>, Error> {
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
        // Only a test when something else binds x: `kind(c, k), (x = "a" ; x = "b")` gives x each value.
        if let Some(members) = members
            && members.iter().all(|(x, _)| *x == members[0].0)
            && self.bound_beside(&members[0].0)
        {
            let set = Term::Call(SET.to_owned(), members.iter().map(|(_, c)| Term::Str(c.clone())).collect());
            return Ok(Some(Literal::Cmp { l: Term::Call("equals".to_owned(), vec![Term::Var(members[0].0.clone()), set]), op: "=", r: Term::Int(1) }));
        }
        Ok(None)
    }

    fn set_name(&mut self, rel: &str, arg: &STerm) -> Result<String, Error> {
        match arg {
            STerm::Var(name) => {
                self.sets.insert(name.clone());
                Ok(name.clone())
            }
            STerm::Str(kind) => {
                let name = format!("rr_kind_{}", kind.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>());
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
            // `match` is whole-string, as in Soufflé; the engine's `rx` searches.
            if op == "rx" && let Term::Str(re) = &p {
                p = Term::Str(format!("^(?:{re})$"));
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
            ("inside" | "has", [x, y, set]) => {
                let set = self.set_name(rel, set)?;
                let (x, y) = (self.term(x)?, self.term(y)?);
                let args = if rel == "inside" { vec![x, y] } else { vec![y, x] };
                return Ok(Literal::Atom(Atom { rel: format!("{AXIS}{set}"), args, neg, gated: false }));
            }
            ("split_dot", [q, m, x]) if !neg => {
                return Ok(Literal::Cmp { l: self.term(q)?, op: "=", r: Term::Call("cat".to_owned(), vec![self.term(m)?, self.term(x)?]) });
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
        let op = match agg {
            "count" => AggOp::Count,
            "min" => AggOp::Min,
            "max" => AggOp::Max,
            other => return not_yet(format!("the aggregate {other}")),
        };
        if !by.is_empty() || sep.is_some() {
            return not_yet("`by` and `sep`");
        }
        let over = match over {
            None => None,
            Some(STerm::Var(v)) => Some(v.clone()),
            Some(other) => return not_yet(format!("an aggregate over {other:?}")),
        };
        if op != AggOp::Count && over.is_none() {
            return Err(Error::Syntax(format!("{agg} needs the variable it aggregates")));
        }
        // One plain atom: the engine's aggregate reads it.
        if let [Lit::Atom { rel, args, neg: false }] = body
            && test_of(rel).is_none()
            && !matches!(rel.as_str(), "nearest" | "inside" | "has" | "split_dot" | "ast")
        {
            let Literal::Atom(mut atom) = self.atom(rel, args, false)? else { unreachable!("a plain atom") };
            // A variable only the body uses is counted over, not grouped by: a wildcard to the engine.
            for t in &mut atom.args {
                if matches!(t, Term::Var(v) if !outer.contains(v) && over.as_ref() != Some(v)) {
                    *t = Term::Wild;
                }
            }
            return Ok(vec![Literal::Agg { var: var.to_owned(), op, over, atom }]);
        }
        // Otherwise a helper relation of the body. `count` counts the body's distinct rows, so it keeps every
        // variable; `min` and `max` keep the aggregated one and those the rule shares.
        let inner = body_vars(body);
        let cols: Vec<String> = inner.iter().filter(|v| op == AggOp::Count || over.as_ref() == Some(*v) || outer.contains(*v)).cloned().collect();
        let name = self.fresh("agg");
        let mut bodies = self.conjunction(body, outer)?;
        // A shared variable the body only tests (`count : { slash_at(ps, j), i < j }`) is bound inside the helper by
        // a context relation: the rule's other literals, projected on those variables.
        let bound: BTreeSet<String> = body_vars(
            &body.iter().filter(|l| matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none() && rel != "earlier")).cloned().collect::<Vec<_>>(),
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
        // The helper keeps every column (`count` counts distinct rows), but only the rule's variables group it.
        let args = cols.iter().map(|c| if outer.contains(c) || over.as_ref() == Some(c) { Term::Var(c.clone()) } else { Term::Wild }).collect();
        Ok(vec![Literal::Agg { var: var.to_owned(), op, over, atom: Atom { rel: name, args, neg: false, gated: false } }])
    }

    /// A helper column's type: the column it fills in the body's atoms.
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
                let f = match (name.as_str(), args.len()) {
                    ("cat", 2) => "concat",
                    ("strlen" | "to_string", 1) => name.as_str(),
                    ("substr", 3) => "substr",
                    _ => return not_yet(format!("the built-in function {name}/{}", args.len())),
                };
                let args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
                Term::Call(f.to_owned(), args)
            }
            STerm::Bin { op, l, r } => {
                let f = match *op {
                    "+" => "add",
                    "-" => "sub",
                    "*" => "mul",
                    "/" => "div",
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
    fn kind_sets(&mut self) -> Result<(), Error> {
        for set in std::mem::take(&mut self.sets) {
            if self.prog.kindsets.contains_key(&set) {
                continue;
            }
            let kinds = self
                .kinds_of(&set, &mut BTreeSet::new())
                .ok_or_else(|| Error::Syntax(format!("the kind set {set} is not a relation of kinds (one column of facts, or rules `{set}(k) :- s(k)` over such sets)")))?;
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
/// The empty value `unset(x)` binds (`core.rr`): the id -1, which no node has, so no tree relation holds of it.
/// Each `unset(x)` is taken out and `x` replaced by it in the rule. (`text(x, "")` does not hold of it yet: O77.)
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
    Ok((head, rest))
}

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
