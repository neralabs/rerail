//! The checks of `docs/rule-syntax/site/check_rr.py` (and its `souffle.py`), over `.rr` statements:
//! - every rule is grounded: each variable is bound by a positive atom, a pattern, a `match` group or an
//!   assignment before a negation, a comparison, a text test or the head needs it;
//! - no relation is declared twice in one program (a file, what it includes, and `core.rr`), and no declaration
//!   takes a built-in constraint's name;
//! - the program is stratified: no relation depends on itself through `!`, an aggregate or a relation passed to a
//!   built-in, unless both are `ordered by` the same order and the use is tied to the head by it
//!   (`earlier(m, n)` in the same body).

use std::collections::{BTreeMap, BTreeSet};

use super::lower::{RELATION_ARG, TESTS};
use super::syntax::{Head, Item, Lit, Statement, Term};

fn vars_of(t: &Term) -> BTreeSet<String> {
    let mut out = Vec::new();
    t.vars(&mut out);
    out.into_iter().collect()
}

/// What a term needs from outside, given what is bound: its variables, but of each aggregate inside it only what
/// that aggregate's body uses and does not bind (`1 + count : { f(x) }` needs nothing).
fn term_needs(t: &Term, bound: &BTreeSet<String>) -> BTreeSet<String> {
    match t {
        Term::Agg { body, .. } => inner_needs(body, bound),
        Term::Call { args, .. } => args.iter().flat_map(|a| term_needs(a, bound)).collect(),
        Term::Bin { l, r, .. } => term_needs(l, bound).union(&term_needs(r, bound)).cloned().collect(),
        Term::Named(_, t) => term_needs(t, bound),
        t => vars_of(t),
    }
}

fn lit_vars(l: &Lit) -> BTreeSet<String> {
    let mut out = Vec::new();
    l.vars(&mut out);
    out.into_iter().collect()
}

fn is_text_test(rel: &str) -> bool {
    TESTS.iter().any(|(rr, _)| *rr == rel && rel != "match")
}

/// An atom's variables, the relations it passes to a built-in left out.
fn atom_vars(rel: &str, args: &[Term]) -> BTreeSet<String> {
    let skip: &[usize] = RELATION_ARG.iter().find(|(r, _)| *r == rel).map_or(&[], |(_, k)| k);
    args.iter().enumerate().filter(|(k, a)| !(skip.contains(k) && matches!(a, Term::Var(_)))).flat_map(|(_, a)| vars_of(a)).collect()
}

/// A regex's named groups.
fn groups(re: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (at, _) in re.match_indices("(?") {
        let rest = &re[at + 2..];
        let rest = rest.strip_prefix('P').unwrap_or(rest);
        if let Some(rest) = rest.strip_prefix('<') {
            let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if !name.is_empty() && rest[name.len()..].starts_with('>') {
                out.insert(name);
            }
        }
    }
    out
}

/// (binds, needs, ready) for one body literal, given what is bound.
fn literal(l: &Lit, bound: &BTreeSet<String>, force: bool) -> (BTreeSet<String>, BTreeSet<String>, bool) {
    let none = BTreeSet::new;
    match l {
        Lit::Group { neg: true, .. } | Lit::Pattern { neg: true, .. } => {
            let vs = lit_vars(l);
            let ready = force || vs.is_subset(bound);
            (none(), vs, ready)
        }
        Lit::Group { alts, neg: false } => {
            let (mut sets, mut needs) = (Vec::new(), BTreeSet::new());
            for branch in alts {
                let mut inner = bound.clone();
                for lit in branch {
                    let (b, n, _) = literal(lit, &inner, true);
                    inner.extend(b);
                    needs.extend(n.difference(&inner).cloned().collect::<Vec<_>>());
                }
                sets.push(inner.difference(bound).cloned().collect::<BTreeSet<_>>());
            }
            let binds = sets.iter().skip(1).fold(sets.first().cloned().unwrap_or_default(), |a, b| a.intersection(b).cloned().collect());
            (binds, needs, true)
        }
        Lit::Pattern { subject, pattern, neg: false } => {
            let mut b = vars_of(subject);
            b.extend(super::syntax::tick_vars(pattern));
            (b, none(), true)
        }
        Lit::Atom { rel, args, neg: true } => {
            let vs = if matches!(rel.as_str(), "match" | "match_all" | "match_group") { args.iter().skip(1).flat_map(vars_of).collect() } else { atom_vars(rel, args) };
            let ready = force || vs.is_subset(bound);
            (none(), vs, ready)
        }
        Lit::Atom { rel, args, neg: false } if is_text_test(rel) => {
            let vs: BTreeSet<String> = args.iter().flat_map(vars_of).collect();
            let ready = force || vs.is_subset(bound);
            (none(), vs, ready)
        }
        Lit::Atom { rel, args, neg: false } if matches!(rel.as_str(), "match" | "match_all" | "match_group") => {
            let mut g = match args.first() {
                Some(Term::Str(re)) if rel != "match_group" => groups(re),
                _ => none(),
            };
            // `match_all("re", sp, m)`, `match_group("re", sp, m, g, gsp)`: the match, the group and its span are
            // bound by it, the span `sp` needed.
            let (subject, binds) = match args.as_slice() {
                [_, sp, m] if rel == "match_all" => (std::slice::from_ref(sp), std::slice::from_ref(m)),
                // `match_all("re", s, ordinal, start, end)` over a symbol binds the three positions.
                [_, s, rest @ ..] if rel == "match_all" && rest.len() == 3 => (std::slice::from_ref(s), rest),
                [_, sp, rest @ ..] if rel == "match_group" => (std::slice::from_ref(sp), rest),
                [_, rest @ ..] => (rest, &[][..]),
                [] => (&[][..], &[][..]),
            };
            g.extend(binds.iter().flat_map(vars_of));
            let subject: BTreeSet<String> = subject.iter().flat_map(vars_of).collect();
            let ready = force || subject.is_subset(bound);
            (g, subject, ready)
        }
        Lit::Atom { rel, args, neg: false } => (atom_vars(rel, args), none(), true),
        Lit::Cmp { l, op, r } => {
            if let Some(body) = aggregate(l).or_else(|| aggregate(r)) {
                let inner = inner_needs(body, bound);
                if *op == "=" && let (Term::Var(v), Term::Agg { .. }) = (l, r) {
                    let ready = force || inner.is_subset(bound);
                    return (BTreeSet::from([v.clone()]), inner, ready);
                }
                let other = if aggregate(l).is_some() { r } else { l };
                let mut vs = vars_of(other);
                vs.extend(inner);
                let ready = force || vs.is_subset(bound);
                return (none(), vs, ready);
            }
            let (lv, rv) = (term_needs(l, bound), term_needs(r, bound));
            if *op != "=" {
                let vs: BTreeSet<String> = lv.union(&rv).cloned().collect();
                let ready = force || vs.is_subset(bound);
                return (none(), vs, ready);
            }
            if let Term::Var(x) = l
                && rv.is_subset(bound)
            {
                return (BTreeSet::from([x.clone()]), rv, true);
            }
            if let Term::Var(x) = r
                && lv.is_subset(bound)
            {
                return (BTreeSet::from([x.clone()]), lv, true);
            }
            let vs: BTreeSet<String> = lv.union(&rv).cloned().collect();
            let ready = force || vs.is_subset(bound);
            (none(), vs, ready)
        }
    }
}

fn aggregate(t: &Term) -> Option<&Vec<Lit>> {
    match t {
        Term::Agg { body, .. } => Some(body),
        _ => None,
    }
}

/// What an aggregate's body needs from outside: what it uses and does not bind.
fn inner_needs(lits: &[Lit], bound: &BTreeSet<String>) -> BTreeSet<String> {
    let mut b = bound.clone();
    let mut needs = BTreeSet::new();
    let mut pending: Vec<&Lit> = lits.iter().collect();
    let mut changed = true;
    while changed {
        changed = false;
        let mut left = Vec::new();
        for l in pending {
            let (bb, nn, done) = literal(l, &b, false);
            if done {
                b.extend(bb);
                needs.extend(nn);
                changed = true;
            } else {
                left.push(l);
            }
        }
        pending = left;
    }
    for l in pending {
        needs.extend(literal(l, &b, true).1);
    }
    let local: BTreeSet<String> = b.difference(bound).cloned().collect();
    needs.difference(&local).cloned().collect()
}

/// A rule's unbound variables, sorted; empty when it is grounded. `given`: relations whose heads are bound from
/// outside (fix relations and edits).
pub(super) fn unbound(heads: &[Head], body: &[Lit], edits: &[Term], given: &BTreeSet<String>) -> Vec<String> {
    let head_vars: BTreeSet<String> = heads.iter().flat_map(|h| h.args.iter().flat_map(vars_of)).collect();
    let mut bound = BTreeSet::new();
    if heads.first().is_some_and(|h| given.contains(&h.rel)) {
        bound.extend(head_vars.iter().cloned());
    }
    let mut need = BTreeSet::new();
    let mut pending: Vec<&Lit> = body.iter().collect();
    let mut changed = true;
    while changed {
        changed = false;
        let mut left = Vec::new();
        for l in pending {
            let (b, n, done) = literal(l, &bound, false);
            if done {
                bound.extend(b);
                need.extend(n);
                changed = true;
            } else {
                left.push(l);
            }
        }
        pending = left;
    }
    for l in pending {
        need.extend(literal(l, &bound, true).1);
    }
    need.extend(heads.iter().flat_map(|h| h.args.iter().flat_map(|a| term_needs(a, &bound))));
    need.extend(edits.iter().flat_map(vars_of));
    need.difference(&bound).cloned().collect()
}

/// The relations a file's `fix` lines and `.edit` declarations name.
pub(super) fn fix_relations(statements: &[Statement]) -> BTreeSet<String> {
    statements
        .iter()
        .flat_map(|(item, _)| match item {
            Item::Violation { fixes, .. } => fixes.iter().map(|f| f.name.clone()).collect(),
            Item::Edit { name } => vec![name.clone()],
            _ => Vec::new(),
        })
        .collect()
}

/// Ungrounded rules of one file: (line, unbound variables).
pub(super) fn grounded(statements: &[Statement], given: &BTreeSet<String>) -> Vec<(usize, Vec<String>)> {
    let mut given = given.clone();
    given.extend(fix_relations(statements));
    statements
        .iter()
        .filter_map(|(item, line)| match item {
            Item::Clause { heads, body, edits } => {
                let bad = unbound(heads, body, edits, &given);
                (!bad.is_empty()).then_some((*line, bad))
            }
            _ => None,
        })
        .collect()
}

/// Declarations made more than once in a program, or of a built-in constraint: (relation, files) per problem.
pub(super) fn declarations(files: &[(String, Vec<Statement>)]) -> Vec<String> {
    let mut decls = BTreeMap::<String, Vec<String>>::new();
    for (file, statements) in files {
        for (item, _) in statements {
            if let Item::Decl { name, .. } | Item::Edit { name } = item {
                decls.entry(name.clone()).or_default().push(file.clone());
            }
        }
    }
    let mut out = Vec::new();
    for (name, files) in decls {
        if TESTS.iter().any(|(rr, _)| *rr == name) || name == "match_all" || name == "match_group" {
            out.push(format!("{name} is a built-in constraint and cannot be declared ({})", files.join(", ")));
        }
        if files.len() > 1 {
            out.push(format!("{name} declared {} times: {}", files.len(), files.join(", ")));
        }
    }
    out
}

/// A dependency of a rule: (relation, negative, its first argument, the scope it is read in).
type Dep = (String, bool, Option<String>, usize);

/// The binary atoms `r(m, n)` of a scope, for `ordered by` ties.
type Scope = BTreeSet<(String, String, String)>;

fn binary_atoms(lits: &[Lit], out: &mut Scope) {
    fn term(t: &Term, out: &mut Scope) {
        match t {
            Term::Agg { body, .. } => binary_atoms(body, out),
            Term::Bin { l, r, .. } => {
                term(l, out);
                term(r, out);
            }
            Term::Call { args, .. } => args.iter().for_each(|a| term(a, out)),
            _ => {}
        }
    }
    for l in lits {
        match l {
            Lit::Atom { rel, args, .. } => {
                if let [Term::Var(m), Term::Var(n)] = args.as_slice() {
                    out.insert((rel.clone(), m.clone(), n.clone()));
                }
                args.iter().for_each(|a| term(a, out));
            }
            Lit::Group { alts, .. } => alts.iter().for_each(|a| binary_atoms(a, out)),
            Lit::Cmp { l, r, .. } => {
                term(l, out);
                term(r, out);
            }
            Lit::Pattern { .. } => {}
        }
    }
}

fn first(args: &[Term]) -> Option<String> {
    match args.first() {
        Some(Term::Var(v)) => Some(v.clone()),
        _ => None,
    }
}

fn deps(lits: &[Lit], neg: bool, scope: usize, scopes: &mut Vec<Scope>, out: &mut Vec<Dep>) {
    fn aggregates<'t>(t: &'t Term, out: &mut Vec<&'t Vec<Lit>>) {
        match t {
            Term::Agg { body, .. } => out.push(body),
            Term::Bin { l, r, .. } => {
                aggregates(l, out);
                aggregates(r, out);
            }
            Term::Call { args, .. } => args.iter().for_each(|a| aggregates(a, out)),
            _ => {}
        }
    }
    for l in lits {
        match l {
            Lit::Group { alts, neg: n } => alts.iter().for_each(|a| deps(a, neg || *n, scope, scopes, out)),
            Lit::Pattern { .. } => {}
            Lit::Atom { rel, args, neg: n } => {
                out.push((rel.clone(), neg || *n, first(args), scope));
                if let Some((_, ks)) = RELATION_ARG.iter().find(|(r, _)| r == rel) {
                    // A relation passed to a built-in is read whole, like a negation.
                    for &k in *ks {
                        if let Some(Term::Var(r)) = args.get(k) {
                            out.push((r.clone(), true, None, scope));
                        }
                    }
                }
            }
            Lit::Cmp { l, r, .. } => {
                let mut bodies = Vec::new();
                aggregates(l, &mut bodies);
                aggregates(r, &mut bodies);
                for body in bodies {
                    let mut s = Scope::new();
                    binary_atoms(body, &mut s);
                    scopes.push(s);
                    let inner = scopes.len() - 1;
                    deps(body, true, inner, scopes, out);
                }
            }
        }
    }
}

fn tarjan(nodes: &BTreeSet<String>, graph: &BTreeMap<String, BTreeSet<String>>) -> BTreeMap<String, usize> {
    let edges: crate::code_rule_dsl::logic::front::Edges =
        graph.iter().map(|(h, ds)| (h.clone(), ds.iter().map(|d| (d.clone(), false)).collect())).collect();
    let mut out = BTreeMap::new();
    for (i, scc) in crate::code_rule_dsl::logic::front::tarjan(nodes, &edges).into_iter().enumerate() {
        for v in scc {
            out.insert(v, i);
        }
    }
    out
}

/// Relations a program's rules read that none of its files declares, nor the lowering makes (its built-ins): the
/// syntax-level form of `front::declared`, which a run and check-rr apply to the lowered program.
pub(super) fn undeclared(statements: &[&Statement]) -> Vec<String> {
    fn reads(lits: &[Lit], out: &mut Vec<String>) {
        fn term(t: &Term, out: &mut Vec<String>) {
            match t {
                Term::Agg { body, .. } => reads(body, out),
                Term::Bin { l, r, .. } => {
                    term(l, out);
                    term(r, out);
                }
                Term::Call { args, .. } => args.iter().for_each(|a| term(a, out)),
                _ => {}
            }
        }
        for l in lits {
            match l {
                Lit::Group { alts, .. } => alts.iter().for_each(|a| reads(a, out)),
                Lit::Atom { rel, args, .. } => {
                    out.push(rel.clone());
                    args.iter().for_each(|a| term(a, out));
                }
                Lit::Cmp { l, r, .. } => {
                    term(l, out);
                    term(r, out);
                }
                Lit::Pattern { .. } => {}
            }
        }
    }
    let mut declared = BTreeSet::new();
    for (item, _) in statements {
        match item {
            Item::Decl { name, .. } | Item::Edit { name } => {
                declared.insert(name.clone());
            }
            Item::Violation { name, .. } => {
                declared.insert(name.clone());
            }
            _ => {}
        }
    }
    let built_in = |rel: &str| TESTS.iter().any(|(t, _)| *t == rel) || RELATION_ARG.iter().any(|(r, _)| *r == rel) || super::lower::BUILT_INS.contains(&rel);
    let mut out = BTreeSet::new();
    for (item, line) in statements {
        if let Item::Clause { heads, body, .. } = item {
            let mut read = Vec::new();
            reads(body, &mut read);
            for rel in read.into_iter().filter(|r| !declared.contains(r) && !built_in(r)) {
                out.insert(format!("line {line}: relation {rel} is read by the rule for {} and not declared", heads[0].rel));
            }
        }
    }
    out.into_iter().collect()
}

/// Stratification problems of one program (every statement of its files): "h depends on d" per problem.
pub(super) fn strata(statements: &[&Statement]) -> Vec<String> {
    let mut ordered = BTreeMap::new();
    let mut rules = Vec::new();
    for (item, line) in statements {
        match item {
            Item::Decl { name, tail, .. } => {
                if let [.., o, b, order] = tail.as_slice()
                    && o == "ordered"
                    && b == "by"
                {
                    ordered.insert(name.clone(), order.clone());
                }
            }
            Item::Clause { heads, body, .. } if !body.is_empty() => {
                let mut scopes = vec![Scope::new()];
                binary_atoms(body, &mut scopes[0]);
                let mut ds = Vec::new();
                deps(body, false, 0, &mut scopes, &mut ds);
                rules.push((*line, heads[0].rel.clone(), first(&heads[0].args), ds, scopes));
            }
            _ => {}
        }
    }
    let idb: BTreeSet<String> = rules.iter().map(|r| r.1.clone()).collect();
    let mut graph = BTreeMap::<String, BTreeSet<String>>::new();
    for (_, h, _, ds, _) in &rules {
        for (d, ..) in ds {
            if idb.contains(d) {
                graph.entry(h.clone()).or_default().insert(d.clone());
            }
        }
    }
    let comp = tarjan(&idb, &graph);
    let mut out = Vec::new();
    for (line, h, n, ds, scopes) in &rules {
        for (d, neg, m, scope) in ds {
            if !neg || !idb.contains(d) || comp[d] != comp[h] {
                continue;
            }
            if let (Some(o), Some(m), Some(n)) = (ordered.get(h), m, n)
                && ordered.get(d) == Some(o)
                && scopes[*scope].contains(&(o.clone(), m.clone(), n.clone()))
            {
                continue;
            }
            out.push(format!("line {line}: {h} depends on {d} through ! or an aggregate in a cycle"));
        }
    }
    out
}
