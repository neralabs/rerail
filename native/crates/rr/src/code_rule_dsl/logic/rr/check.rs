//! The checks of `docs/rule-syntax/site/check_rr.py` (and its `souffle.py`), over `.rerail` statements:
//! - every rule is grounded: each variable is bound by a positive atom, a pattern, a `match` group or an
//!   assignment before a negation, a comparison, a text test or the head needs it;
//! - no relation is declared twice in one program (a file, what it includes, and `core.rerail`), and no declaration
//!   takes a built-in constraint's name;
//! - the program is stratified: no relation depends on itself through `!`, an aggregate or a relation passed to a
//!   built-in, unless both are `ordered by` the same order and the use is tied to the head by it
//!   (`earlier(m, n)` in the same body).

use std::collections::{BTreeMap, BTreeSet};

use super::clause::{At, L, LitAt, Reads, T};
use super::from_tape::{RrFile, S, Statements, Stmt};
use super::lower::{RELATION_ARG, TESTS};
use super::tape::Field;

fn vars_of(f: &RrFile, t: &At) -> BTreeSet<String> {
    f.term_var_set(t)
}

/// What a term needs from outside, given what is bound: its variables, but of each aggregate inside it only what
/// that aggregate's body uses and does not bind (`1 + count : { f(x) }` needs nothing).
fn term_needs(f: &RrFile, t: &At, bound: &BTreeSet<String>) -> BTreeSet<String> {
    match f.tv(t) {
        T::Agg { body, .. } => inner_needs(f, &body, bound),
        T::Call { args, .. } => args.iter().flat_map(|a| term_needs(f, a, bound)).collect(),
        T::Bin { l, r, .. } => term_needs(f, &l, bound).union(&term_needs(f, &r, bound)).cloned().collect(),
        T::Named(_, t) => term_needs(f, &t, bound),
        // `top` no literal binds is the lattice's top, a constant (`unbound`): `a = top` binds `a`.
        T::Var(v) if v == "top" && !bound.contains(&v) => BTreeSet::new(),
        _ => vars_of(f, t),
    }
}

fn is_text_test(rel: &str) -> bool {
    TESTS.iter().any(|(rr, _)| *rr == rel && rel != "match")
}

/// An atom's variables, the relations it passes to a built-in left out.
fn atom_vars(f: &RrFile, rel: &str, args: &[At]) -> BTreeSet<String> {
    let skip: &[usize] = RELATION_ARG.iter().find(|(r, _)| *r == rel).map_or(&[], |(_, k)| k);
    args.iter().enumerate().filter(|(k, a)| !(skip.contains(k) && matches!(f.tv(a), T::Var(_)))).flat_map(|(_, a)| vars_of(f, a)).collect()
}

/// A regex's named groups.
fn groups(re: &str) -> BTreeSet<String> {
    super::lower::groups(re).into_iter().collect()
}

/// (binds, needs, ready) for one body literal, given what is bound.
fn literal(f: &RrFile, l: &LitAt, bound: &BTreeSet<String>, force: bool) -> (BTreeSet<String>, BTreeSet<String>, bool) {
    let none = BTreeSet::new;
    let vars = |ts: &[At]| -> BTreeSet<String> { ts.iter().flat_map(|a| vars_of(f, a)).collect() };
    match f.lit(l) {
        // A negated conjunction's own positive literals bind its local variables (`!(callee(m, c), fname(f, c))`
        // needs only `c`), as a group's branches do; it binds nothing outside.
        L::Group { alts, neg: true } => {
            let mut needs = BTreeSet::new();
            for branch in &alts {
                let mut inner = bound.clone();
                for lit in branch {
                    let (b, n, _) = literal(f, lit, &inner, true);
                    inner.extend(b);
                    needs.extend(n.difference(&inner).cloned().collect::<Vec<_>>());
                }
            }
            let ready = force || needs.is_subset(bound);
            (none(), needs, ready)
        }
        // A negated pattern reads its subject; its captures are its own.
        L::Pattern { subject, neg: true, .. } => {
            let vs = vars_of(f, &subject);
            let ready = force || vs.is_subset(bound);
            (none(), vs, ready)
        }
        L::Group { alts, neg: false } => {
            let (mut sets, mut needs) = (Vec::new(), BTreeSet::new());
            for branch in &alts {
                let mut inner = bound.clone();
                for lit in branch {
                    let (b, n, _) = literal(f, lit, &inner, true);
                    inner.extend(b);
                    needs.extend(n.difference(&inner).cloned().collect::<Vec<_>>());
                }
                sets.push(inner.difference(bound).cloned().collect::<BTreeSet<_>>());
            }
            let binds = sets.iter().skip(1).fold(sets.first().cloned().unwrap_or_default(), |a, b| a.intersection(b).cloned().collect());
            (binds, needs, true)
        }
        L::Pattern { subject, pattern, neg: false, .. } => {
            let mut b = vars_of(f, &subject);
            b.extend(rerail_rr_syntax::syntax::tick_vars(&pattern));
            (b, none(), true)
        }
        L::Atom { rel, args, neg: true } => {
            let vs = if matches!(rel.as_str(), "match" | "match_all" | "match_group") { vars(args.get(1..).unwrap_or(&[])) } else { atom_vars(f, &rel, &args) };
            let ready = force || vs.is_subset(bound);
            (none(), vs, ready)
        }
        L::Atom { rel, args, neg: false } if is_text_test(&rel) => {
            let vs = vars(&args);
            let ready = force || vs.is_subset(bound);
            (none(), vs, ready)
        }
        L::Atom { rel, args, neg: false } if matches!(rel.as_str(), "match" | "match_all" | "match_group") => {
            let mut g = match args.first().map(|a| f.tv(a)) {
                Some(T::Str(re)) if rel != "match_group" => groups(&re),
                _ => none(),
            };
            // `match_all("re", sp, m)`, `match_group("re", sp, m, g, gsp)`: the match, the group and its span are
            // bound by it, the span `sp` needed.
            let (subject, binds) = match args.as_slice() {
                [_, sp, m] if rel == "match_all" => (std::slice::from_ref(sp), std::slice::from_ref(m)),
                // `match_all(re, s, i, a, b)` over a symbol binds each match's ordinal and offsets too.
                [_, s, rest @ ..] if rel == "match_all" && rest.len() == 3 => (std::slice::from_ref(s), rest),
                [_, sp, rest @ ..] if rel == "match_group" => (std::slice::from_ref(sp), rest),
                [_, rest @ ..] => (rest, &[][..]),
                [] => (&[][..], &[][..]),
            };
            g.extend(vars(binds));
            let subject = vars(subject);
            let ready = force || subject.is_subset(bound);
            (g, subject, ready)
        }
        L::Atom { rel, args, neg: false } => (atom_vars(f, &rel, &args), none(), true),
        L::Cmp { l, op, r } => {
            let (lt, rt) = (f.tv(&l), f.tv(&r));
            let body = match (&lt, &rt) {
                (T::Agg { body, .. }, _) | (_, T::Agg { body, .. }) => Some(body),
                _ => None,
            };
            if let Some(body) = body {
                let inner = inner_needs(f, body, bound);
                if op == "=" && let (T::Var(v), T::Agg { .. }) = (&lt, &rt) {
                    let ready = force || inner.is_subset(bound);
                    return (BTreeSet::from([v.clone()]), inner, ready);
                }
                let other = if matches!(lt, T::Agg { .. }) { &r } else { &l };
                let mut vs = vars_of(f, other);
                vs.extend(inner);
                let ready = force || vs.is_subset(bound);
                return (none(), vs, ready);
            }
            let (lv, rv) = (term_needs(f, &l, bound), term_needs(f, &r, bound));
            if op != "=" {
                let vs: BTreeSet<String> = lv.union(&rv).cloned().collect();
                let ready = force || vs.is_subset(bound);
                return (none(), vs, ready);
            }
            if let T::Var(x) = &lt
                && rv.is_subset(bound)
            {
                return (BTreeSet::from([x.clone()]), rv, true);
            }
            if let T::Var(x) = &rt
                && lv.is_subset(bound)
            {
                return (BTreeSet::from([x.clone()]), lv, true);
            }
            // `m = @dot(q, x)` with `m` bound splits it at each dot (`split_dot`), and `m = @cat(q, x)` at each
            // position (the engine's `@cat` split, which `@dot` lowers to: T's `cmp_binds`), binding `q` and `x`.
            for (whole, parts) in [(&lt, &rt), (&rt, &lt)] {
                if let (T::Var(m), T::Call { name, args, .. }) = (whole, parts)
                    && (name == "dot" || name == "cat")
                    && args.iter().all(|a| matches!(f.tv(a), T::Var(_)))
                {
                    let ready = force || bound.contains(m);
                    return (vars(args), BTreeSet::from([m.clone()]), ready);
                }
            }
            let vs: BTreeSet<String> = lv.union(&rv).cloned().collect();
            let ready = force || vs.is_subset(bound);
            (none(), vs, ready)
        }
    }
}

/// What an aggregate's body needs from outside: what it uses and does not bind.
fn inner_needs(f: &RrFile, lits: &[LitAt], bound: &BTreeSet<String>) -> BTreeSet<String> {
    let mut b = bound.clone();
    let mut needs = BTreeSet::new();
    let mut pending: Vec<&LitAt> = lits.iter().collect();
    let mut changed = true;
    while changed {
        changed = false;
        let mut left = Vec::new();
        for l in pending {
            let (bb, nn, done) = literal(f, l, &b, false);
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
        needs.extend(literal(f, l, &b, true).1);
    }
    let local: BTreeSet<String> = b.difference(bound).cloned().collect();
    needs.difference(&local).cloned().collect()
}

/// Clause `c`'s body, read off its tape.
/// Clause `c`'s body literals.
pub fn clause_body(f: &RrFile, c: u32) -> Vec<LitAt> {
    f.tape().get(c, Field::Body).map_or_else(Vec::new, |b| f.body_at(b, 0))
}

/// Clause `c`'s unbound variables, sorted; empty when it is grounded. `given`: relations whose heads are bound from
/// outside (fix relations and edits).
pub(super) fn unbound(f: &RrFile, c: u32, given: &BTreeSet<String>) -> Vec<String> {
    let heads = f.heads_at(c);
    let body = clause_body(f, c);
    let head_vars: BTreeSet<String> = heads.iter().flat_map(|h| h.args.iter().flat_map(|a| vars_of(f, a))).collect();
    // `top` no literal binds is the lattice's top, not a variable (`lower`'s `top_is_var`): bound from the start, so
    // `a = top` binds `a`.
    let mut bound = BTreeSet::from(["top".to_owned()]);
    if heads.first().is_some_and(|h| given.contains(&h.rel)) {
        bound.extend(head_vars.iter().cloned());
    }
    let mut need = BTreeSet::new();
    let mut pending: Vec<&LitAt> = body.iter().collect();
    let mut changed = true;
    while changed {
        changed = false;
        let mut left = Vec::new();
        for l in pending {
            let (b, n, done) = literal(f, l, &bound, false);
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
        need.extend(literal(f, l, &bound, true).1);
    }
    need.extend(heads.iter().flat_map(|h| h.args.iter().flat_map(|a| term_needs(f, a, &bound))));
    let t = f.tape();
    need.extend(t.all(c, Field::Edit).flat_map(|e| vars_of(f, &At::Node(e, 0))));
    need.difference(&bound).cloned().collect()
}

/// The relations a file's `fix` lines and `.edit` declarations name.
pub fn fix_relations(statements: &Statements) -> BTreeSet<String> {
    statements
        .iter()
        .flat_map(|s| match s.what() {
            Some(S::Violation(_)) => {
                let (t, n) = s.tape();
                t.fixes(n).into_iter().map(str::to_owned).collect()
            }
            Some(S::Edit(name)) => vec![name.to_owned()],
            _ => Vec::new(),
        })
        .collect()
}

/// Ungrounded rules of one file: (line, unbound variables).
pub fn grounded(statements: &Statements, given: &BTreeSet<String>) -> Vec<(usize, Vec<String>)> {
    let mut given = given.clone();
    given.extend(fix_relations(statements));
    statements
        .iter()
        .filter(|s| s.what() == Some(S::Clause))
        .filter_map(|s| {
            let (f, n) = (s.file(), s.node());
            f.check_clause(n).ok()?;
            let bad = unbound(f, n, &given);
            (!bad.is_empty()).then(|| (s.line(), bad))
        })
        .collect()
}

/// Declarations made more than once in a program, or of a built-in constraint: (relation, files) per problem.
pub fn declarations(files: &[(String, Statements)]) -> Vec<String> {
    let mut decls = BTreeMap::<String, Vec<String>>::new();
    for (file, statements) in files {
        for s in statements {
            if let Some(S::Decl(name) | S::Edit(name)) = s.what() {
                decls.entry(name.to_owned()).or_default().push(file.clone());
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

fn binary_atoms(f: &RrFile, lits: &[LitAt], out: &mut Scope) {
    fn term(f: &RrFile, t: &At, out: &mut Scope) {
        match f.tv(t) {
            T::Agg { body, .. } => binary_atoms(f, &body, out),
            T::Bin { l, r, .. } => {
                term(f, &l, out);
                term(f, &r, out);
            }
            T::Call { args, .. } => args.iter().for_each(|a| term(f, a, out)),
            _ => {}
        }
    }
    for l in lits {
        match f.lit(l) {
            L::Atom { rel, args, .. } => {
                if let [m, n] = args.as_slice()
                    && let (T::Var(m), T::Var(n)) = (f.tv(m), f.tv(n))
                {
                    out.insert((rel.clone(), m, n));
                }
                args.iter().for_each(|a| term(f, a, out));
            }
            L::Group { alts, .. } => alts.iter().for_each(|a| binary_atoms(f, a, out)),
            L::Cmp { l, r, .. } => {
                term(f, &l, out);
                term(f, &r, out);
            }
            L::Pattern { .. } => {}
        }
    }
}

fn first(f: &RrFile, args: &[At]) -> Option<String> {
    match args.first().map(|a| f.tv(a)) {
        Some(T::Var(v)) => Some(v),
        _ => None,
    }
}

fn deps(f: &RrFile, lits: &[LitAt], neg: bool, scope: usize, scopes: &mut Vec<Scope>, out: &mut Vec<Dep>) {
    fn aggregates(f: &RrFile, t: &At, out: &mut Vec<Vec<LitAt>>) {
        match f.tv(t) {
            T::Agg { body, .. } => out.push(body),
            T::Bin { l, r, .. } => {
                aggregates(f, &l, out);
                aggregates(f, &r, out);
            }
            T::Call { args, .. } => args.iter().for_each(|a| aggregates(f, a, out)),
            _ => {}
        }
    }
    for l in lits {
        match f.lit(l) {
            L::Group { alts, neg: n } => alts.iter().for_each(|a| deps(f, a, neg || n, scope, scopes, out)),
            L::Pattern { .. } => {}
            L::Atom { rel, args, neg: n } => {
                out.push((rel.clone(), neg || n, first(f, &args), scope));
                if let Some((_, ks)) = RELATION_ARG.iter().find(|(r, _)| *r == rel) {
                    // A relation passed to a built-in is read whole, like a negation.
                    for &k in *ks {
                        if let Some(T::Var(r)) = args.get(k).map(|a| f.tv(a)) {
                            out.push((r, true, None, scope));
                        }
                    }
                }
            }
            L::Cmp { l, r, .. } => {
                let mut bodies = Vec::new();
                aggregates(f, &l, &mut bodies);
                aggregates(f, &r, &mut bodies);
                for body in bodies {
                    let mut s = Scope::new();
                    binary_atoms(f, &body, &mut s);
                    scopes.push(s);
                    let inner = scopes.len() - 1;
                    deps(f, &body, true, inner, scopes, out);
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

/// The relations a clause body reads, aggregates' and groups' bodies included (in order, with repeats).
pub fn clause_reads(f: &RrFile, lits: &[LitAt], out: &mut Vec<String>) {
    fn term(f: &RrFile, t: &At, out: &mut Vec<String>) {
        match f.tv(t) {
            T::Agg { body, .. } => clause_reads(f, &body, out),
            T::Bin { l, r, .. } => {
                term(f, &l, out);
                term(f, &r, out);
            }
            T::Call { args, .. } => args.iter().for_each(|a| term(f, a, out)),
            _ => {}
        }
    }
    for l in lits {
        match f.lit(l) {
            L::Group { alts, .. } => alts.iter().for_each(|a| clause_reads(f, a, out)),
            L::Atom { rel, args, .. } => {
                out.push(rel);
                args.iter().for_each(|a| term(f, a, out));
            }
            L::Cmp { l, r, .. } => {
                term(f, &l, out);
                term(f, &r, out);
            }
            L::Pattern { .. } => {}
        }
    }
}

/// Relations a program's rules read that none of its files declares, nor the lowering makes (its built-ins): the
/// syntax-level form of `lowering_tail::Declared`, which a run and check-rr apply to the lowered program.
pub fn undeclared(statements: &Statements) -> Vec<String> {
    let mut declared = BTreeSet::new();
    for s in statements {
        if let Some(S::Decl(name) | S::Edit(name) | S::Violation(name)) = s.what() {
            declared.insert(name.to_owned());
        }
    }
    // The import closure's relations (`rr_cx_*`) are the engine's: a run adds `components.rerail` to a program that reads
    // one, with their declarations (`plan::corpus`'s `reads_components`, decision 0450 §3).
    let built_in = |rel: &str| {
        TESTS.iter().any(|(t, _)| *t == rel) || RELATION_ARG.iter().any(|(r, _)| *r == rel) || super::lower::BUILT_INS.contains(&rel) || rel.starts_with("rr_cx_")
    };
    let mut out = BTreeSet::new();
    for s in statements.iter().filter(|s| s.what() == Some(S::Clause)) {
        let (f, n) = (s.file(), s.node());
        if f.check_clause(n).is_err() {
            continue;
        }
        let mut read = Vec::new();
        clause_reads(f, &clause_body(f, n), &mut read);
        let head = s.heads().first().map_or("", |h| *h).to_owned();
        for rel in read.into_iter().filter(|r| !declared.contains(r) && !built_in(r)) {
            out.insert(format!("line {}: relation {rel} is read by the rule for {head} and not declared", s.line()));
        }
    }
    out.into_iter().collect()
}

/// Stratification problems of one program (every statement of its files): "h depends on d" per problem.
pub fn strata(statements: &Statements) -> Vec<String> {
    let mut ordered = BTreeMap::new();
    let mut rules = Vec::new();
    for s in statements {
        let (t, n) = s.tape();
        match s.what() {
            Some(S::Decl(_)) => {
                let Ok((name, _, tail)) = t.decl(n) else { continue };
                if let [.., o, b, order] = tail.as_slice()
                    && o == "ordered"
                    && b == "by"
                {
                    ordered.insert(name, order.clone());
                }
            }
            Some(S::Clause) if s.has_body() => {
                let f = s.file();
                if f.check_clause(n).is_err() {
                    continue;
                }
                let (heads, body) = (f.heads_at(n), clause_body(f, n));
                let mut scopes = vec![Scope::new()];
                binary_atoms(f, &body, &mut scopes[0]);
                let mut ds = Vec::new();
                deps(f, &body, false, 0, &mut scopes, &mut ds);
                rules.push((s.line(), heads[0].rel.clone(), first(f, &heads[0].args), ds, scopes));
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
    // A component that reads an answer of the world (`WORLD`: the file system's, a language's) holds its request too
    // once lowered, and the engine gates each negation in it per key until its answers are whole (decision 0065 §2a,
    // `answers::gate`). An aggregate there (a dep of an aggregate's body: a scope past the rule's own) is refused by
    // the engine, and here.
    let answer = |rel: &str| super::lower::WORLD.iter().any(|(a, _)| *a == rel);
    let answered: BTreeSet<usize> = rules.iter().filter(|(_, _, _, ds, _)| ds.iter().any(|(d, ..)| answer(d))).map(|(_, h, ..)| comp[h]).collect();
    let mut out = Vec::new();
    for (line, h, n, ds, scopes) in &rules {
        for (d, neg, m, scope) in ds {
            if !neg || !idb.contains(d) || comp[d] != comp[h] {
                continue;
            }
            if *scope == 0 && answered.contains(&comp[h]) {
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
