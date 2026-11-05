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
//! - a rule ending in `=> edits` defines a fix, and fixes are not run: it is left out.
//!
//! Not lowered yet, each a named error: patterns (``n ~ `…` ``), `must`/`may`, a `.violation` at a span or place,
//! `ordered by`, `sum`/`mean`/`join`, `match` captures, arithmetic, Soufflé's built-in functions (`cat`, `strlen`),
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
            Item::Io { params, .. } if !params.is_empty() => return not_yet(format!("I/O parameters on {}", params.join(", "))).map_err(at),
            Item::Io { output, names, .. } => if output { &mut l.prog.outputs } else { &mut l.prog.inputs }.extend(names),
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
        let mut acc = vec![Vec::new()];
        for (k, lit) in lits.iter().enumerate() {
            // The variables used outside this literal, which an aggregate inside it is correlated on.
            let mut outer: Vec<String> = context.iter().cloned().collect();
            lits.iter().enumerate().filter(|(j, _)| *j != k).for_each(|(_, l)| l.vars(&mut outer));
            let alts = self.literal(lit, &outer.into_iter().collect())?;
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
        if let Some(members) = members
            && members.iter().all(|(x, _)| *x == members[0].0)
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
                return not_yet(format!("match with named groups, {re:?}"));
            }
            let (s, mut p) = (self.term(s)?, self.term(p)?);
            // `match` is whole-string, as in Soufflé; the engine's `rx` searches.
            if op == "rx" && let Term::Str(re) = &p {
                p = Term::Str(format!("^(?:{re})$"));
            }
            return Ok(Literal::Cmp { l: Term::Call(op.to_owned(), vec![s, p]), op: "=", r: Term::Int(if neg { 0 } else { 1 }) });
        }
        match (rel, args) {
            ("nearest", [n, a, set]) if !neg => {
                let kindset = self.set_name(rel, set)?;
                return Ok(Literal::Nearest { n: var(n)?, a: var(a)?, kindset });
            }
            // `!nearest(n, _, set)`: n has no ancestor in the set, which is `!inside(n, _, set)`.
            ("nearest", [n, STerm::Wild, set]) if neg => {
                let set = self.set_name(rel, set)?;
                return Ok(Literal::Atom(Atom { rel: format!("{AXIS}{set}"), args: vec![self.term(n)?, Term::Wild], neg, gated: false }));
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
                let (l, r) = if matches!(l, STerm::Agg { .. }) { (Term::Var(v), other) } else { (other, Term::Var(v)) };
                out.push(Literal::Cmp { l, op, r });
                Ok(vec![out])
            }
            _ => Ok(vec![vec![Literal::Cmp { l: self.term(l)?, op, r: self.term(r)? }]]),
        }
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
            let Literal::Atom(atom) = self.atom(rel, args, false)? else { unreachable!("a plain atom") };
            return Ok(vec![Literal::Agg { var: var.to_owned(), op, over, atom }]);
        }
        // Otherwise a helper relation of the body. `count` counts the body's distinct rows, so it keeps every
        // variable; `min` and `max` keep the aggregated one and those the rule shares.
        let inner = body_vars(body);
        let cols: Vec<String> = inner.iter().filter(|v| op == AggOp::Count || over.as_ref() == Some(*v) || outer.contains(*v)).cloned().collect();
        let name = self.fresh("agg");
        let bodies = self.conjunction(body, outer)?;
        let types = cols.iter().map(|c| (c.clone(), self.type_of(c, &bodies))).collect();
        self.prog.decls.insert(name.clone(), types);
        let args: Vec<Term> = cols.iter().map(|c| Term::Var(c.clone())).collect();
        for b in bodies {
            self.prog.rules.push(Rule { head: Atom { rel: name.clone(), args: args.clone(), neg: false, gated: false }, body: b });
        }
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
            STerm::Var(v) if v == "top" => Term::Top,
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
            STerm::Json(_) | STerm::Named(..) => return Err(Error::Syntax("a JSON term or an `ast` capture outside `ast(…)`".to_owned())),
        })
    }

    /// Kind sets: the relations of facts given to `nearest`, `inside` and `has`. One read nowhere else is only a set.
    fn kind_sets(&mut self) -> Result<(), Error> {
        let used: BTreeSet<String> = self
            .prog
            .rules
            .iter()
            .flat_map(|r| std::iter::once(&r.head).chain(r.body.iter().filter_map(Literal::atom)))
            .map(|a| a.rel.clone())
            .collect();
        for set in std::mem::take(&mut self.sets) {
            if self.prog.kindsets.contains_key(&set) {
                continue;
            }
            let rows = self.prog.facts.get(&set).cloned().unwrap_or_default();
            let kinds = rows
                .iter()
                .map(|row| match row.as_slice() {
                    [Value::Str(k)] => Some(k.clone()),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
                .filter(|_| !used.contains(&set) && !self.prog.inputs.contains(&set) && self.prog.decls.get(&set).is_none_or(|c| c.len() == 1))
                .ok_or_else(|| Error::Syntax(format!("the kind set {set} is not a relation of kinds (one column of facts, no rules)")))?;
            self.prog.kindsets.insert(set.clone(), kinds);
            self.prog.facts.remove(&set);
            self.prog.decls.remove(&set);
        }
        Ok(())
    }
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
                other.vars(&mut vs);
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
