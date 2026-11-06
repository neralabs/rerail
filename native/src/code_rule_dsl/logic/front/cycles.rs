//! O11 (option b): a recursion that creates values is bounded when it moves along the tree.
//!
//! §12 rejects "cycles through value-creating builtins with no bound". A value cycle is one in the flow of
//! values between columns: a value a builtin made (`@cat` and the others, in a head or bound by `$v = @f(…)`)
//! reaching that builtin's arguments again. A component whose rules make values that never come back (T's
//! `qv(@cat($q, $x), …)` in `calls.rdl`, whose `$q` and `$x` come from columns no made value reaches) makes
//! finitely many. A value made into a lattice's value column is bounded by the lattice (a finite height, or
//! `widen`: `UnboundedLattice`), as `cv`'s `@flat_bin` in T's workloads. The bound of any other value cycle is
//! the syntax tree: its component is accepted when one node column of each of its relations (its key) moves one
//! way along tree edges, every rule the same way: from the head's key to each recursive atom's key, strictly down
//! for every rule that creates a value and down or not at all for the others (or all of them up). A tree is
//! finite and acyclic, so every derivation is at most its depth deep, and only finitely many values are made.
//! T's programs recurse this way: `qn` along `attr_of` (a `field` edge), `resolves` along `field`, `modn` along
//! `nearest`.
//!
//! Tree edges are the base relations' (`child(p, c, _)`, `field(p, _, c)`, `tape(n, par, …)`), `nearest` and the
//! tree axes (`inside_<set>(n, a)`: `a` strictly above `n`), and what a derived relation of lower strata
//! guarantees between its node columns in every row: strictly below (`attr_of(n, o, a)`: `o` below `n`), or
//! below or the same node (`unparen(e, m)`: `m` is `e` or inside it). A recursive relation's guarantees are those
//! its rules keep given that its own rows keep them (induction over derivations: the greatest set of claims every
//! rule keeps), so `qualified(a) :- field(a, "object", o), unparen(o, v), qualified(v)` moves strictly down. A
//! relation with facts, or an input, has rows no rule made: it guarantees nothing. Nothing else bounds a cycle: a
//! relation over a graph (imports, calls) may itself be cyclic. Decided 2026-11-03 (open-items O11). A builtin
//! that gives part of its argument (`shrinks`: `@pkg_up`, `substr`) makes no new value, so a cycle through it
//! alone is bounded too.
//!
//! A position along a node's children bounds a cycle too (decision 0043, for joining a node's parts in order):
//! with a key of a node column and an integer column, every rule keeps the node and moves the integer one way,
//! strictly in a rule that creates a value, where the head's integer is the index of one of the node's children
//! (`child(c, p, i + 1)`). A node has finitely many children, so the derivation is as long as they are many:
//! `j(c, i + 1, s2) :- j(c, i, s), child(c, p, i + 1), part(p, t), s2 = cat(s, t)`.

use super::{AXIS, Error, Literal, Program, Rule, Term};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// A move along the tree from one node to another.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Move {
    Down,
    Up,
}

impl Move {
    fn back(self) -> Self {
        match self {
            Move::Down => Move::Up,
            Move::Up => Move::Down,
        }
    }
}

/// A move and whether it is strict: `false` where the far end may be the same node (`unparen(e, e)`).
type Step = (Move, bool);

/// Per relation, the moves between its columns that hold in every row: `(from, to) → steps`.
type Moves = BTreeMap<String, BTreeMap<(usize, usize), BTreeSet<Step>>>;

/// The base relations' tree edges, both ways.
fn base() -> Moves {
    let mut moves = Moves::new();
    for (rel, parent, child) in [("child", 0, 1), ("field", 0, 2), ("tape", 1, 0)] {
        let edges = moves.entry(rel.to_owned()).or_default();
        edges.entry((parent, child)).or_default().insert((Move::Down, true));
        edges.entry((child, parent)).or_default().insert((Move::Up, true));
    }
    moves
}

/// A rule's variables' edges: `var → [(var, Some(step) or None for the same node)]`.
fn edges(rule: &Rule, moves: &Moves) -> BTreeMap<String, Vec<(String, Option<Step>)>> {
    let mut out = BTreeMap::<String, Vec<(String, Option<Step>)>>::new();
    let mut add = |a: &str, b: &str, m: Option<Step>| {
        out.entry(a.to_owned()).or_default().push((b.to_owned(), m));
        out.entry(b.to_owned()).or_default().push((a.to_owned(), m.map(|(way, strict)| (way.back(), strict))));
    };
    for literal in &rule.body {
        match literal {
            Literal::Atom(a) if !a.neg => {
                for (&(i, j), ms) in moves.get(&a.rel).into_iter().flatten() {
                    if let (Some(Term::Var(x)), Some(Term::Var(y))) = (a.args.get(i), a.args.get(j)) {
                        ms.iter().for_each(|&m| add(x, y, Some(m)));
                    }
                }
                // A tree axis: its second node is a strict ancestor of its first (decision 0036).
                if a.rel.starts_with(AXIS)
                    && let [Term::Var(n), Term::Var(above)] = a.args.as_slice()
                {
                    add(n, above, Some((Move::Up, true)));
                }
            }
            Literal::Nearest { n, a, .. } => add(n, a, Some((Move::Up, true))),
            Literal::Cmp { l: Term::Var(x), op: "=", r: Term::Var(y) } => add(x, y, None),
            _ => {}
        }
    }
    out
}

/// How `to` lies from `from` along `way`: `Some(true)` strictly, `Some(false)` maybe at the same node, `None` not.
fn reach(edges: &BTreeMap<String, Vec<(String, Option<Step>)>>, from: &str, to: &str, way: Move) -> Option<bool> {
    let mut seen = BTreeSet::from([(from.to_owned(), false)]);
    let mut queue = VecDeque::from([(from.to_owned(), false)]);
    while let Some((at, strict)) = queue.pop_front() {
        for (next, m) in edges.get(&at).into_iter().flatten() {
            if m.is_some_and(|(m, _)| m != way) {
                continue;
            }
            let state = (next.clone(), strict || m.is_some_and(|(_, s)| s));
            if seen.insert(state.clone()) {
                queue.push_back(state);
            }
        }
    }
    if seen.contains(&(to.to_owned(), true)) {
        Some(true)
    } else if seen.contains(&(to.to_owned(), false)) {
        Some(false)
    } else {
        None
    }
}

/// Whether `a` and `b` are one node: the same variable, or equal through `$x = $y`.
fn same(edges: &BTreeMap<String, Vec<(String, Option<Step>)>>, a: &str, b: &str) -> bool {
    let mut seen = BTreeSet::from([a]);
    let mut stack = vec![a];
    while let Some(at) = stack.pop() {
        if at == b {
            return true;
        }
        for (next, m) in edges.get(at).into_iter().flatten() {
            if m.is_none() && seen.insert(next) {
                stack.push(next);
            }
        }
    }
    false
}

/// Whether a rule makes a value: a computed head term, or `$v = @f(…)` binding a head variable.
fn creates(rule: &Rule) -> bool {
    let head: BTreeSet<String> = rule.head.vars();
    rule.head.args.iter().any(|t| matches!(t, Term::Call(..)))
        || rule.body.iter().any(|l| matches!(l, Literal::Cmp { l: Term::Var(v), op: "=", r: Term::Call(..) } if head.contains(v)))
}

/// A builtin whose value is a part of its first argument (`@pkg_up(q, k)` a prefix of q, `substr` a substring):
/// a cycle through it alone gives only parts of values that entered it, finitely many, so it makes no new
/// value. (An extension of O11 for `.rr`'s Python library, `pkg_above` and `loads`: decision 0043.)
fn shrinks(f: &str) -> bool {
    matches!(f, "pkg_up" | "substr")
}

/// A column of a relation.
type Column = (String, usize);

/// The flow of values in `rules` between the columns of `component`: `(from, to, made)`, where `made` is a
/// builtin's result from its arguments.
fn flow(rules: &[&Rule], component: &BTreeSet<String>) -> Vec<(Column, Column, bool)> {
    let mut out = Vec::new();
    for rule in rules {
        // Where each variable's values come from: the component's columns it is read from, and through which
        // assignments (`$v = term`, made when the term is a builtin's).
        let mut from = BTreeMap::<String, BTreeSet<(Column, bool)>>::new();
        let mut bound = BTreeSet::new();
        for literal in &rule.body {
            if let Some(a) = literal.atom().filter(|a| !a.neg) {
                for (i, t) in a.args.iter().enumerate() {
                    if let Term::Var(v) = t {
                        bound.insert(v.clone());
                        if component.contains(&a.rel) {
                            from.entry(v.clone()).or_default().insert(((a.rel.clone(), i), false));
                        }
                    }
                }
            }
        }
        let sources = |t: &Term, from: &BTreeMap<String, BTreeSet<(Column, bool)>>| -> BTreeSet<(Column, bool)> {
            let mut vars = BTreeSet::new();
            t.vars(&mut vars);
            let made = matches!(t, Term::Call(f, _) if !shrinks(f));
            vars.iter().flat_map(|v| from.get(v).into_iter().flatten().map(move |(c, m)| (c.clone(), *m || made))).collect()
        };
        // Assignments in order, until nothing more flows (an assignment may read another's variable).
        loop {
            let before = from.values().map(BTreeSet::len).sum::<usize>();
            for literal in &rule.body {
                if let Literal::Cmp { l: Term::Var(v), op: "=", r } = literal
                    && !bound.contains(v)
                {
                    let s = sources(r, &from);
                    from.entry(v.clone()).or_default().extend(s);
                }
            }
            if from.values().map(BTreeSet::len).sum::<usize>() == before {
                break;
            }
        }
        for (j, t) in rule.head.args.iter().enumerate() {
            for (c, made) in sources(t, &from) {
                out.push((c, (rule.head.rel.clone(), j), made));
            }
        }
    }
    out
}

/// Whether a value made in `rules` flows back to the builtin that made it.
fn value_cycle(prog: &Program, rules: &[&Rule], component: &BTreeSet<String>) -> bool {
    let lattice_value = |(rel, col): &Column| prog.lattice.get(rel).is_some_and(|d| d.keys == *col);
    let flow: Vec<_> = flow(rules, component).into_iter().map(|(a, b, made)| {
        let made = made && !lattice_value(&b);
        (a, b, made)
    }).collect();
    let mut next = BTreeMap::<&Column, Vec<&Column>>::new();
    for (a, b, _) in &flow {
        next.entry(a).or_default().push(b);
    }
    flow.iter().filter(|(_, _, made)| *made).any(|(start, made_into, _)| {
        let mut seen = BTreeSet::from([made_into]);
        let mut stack = vec![made_into];
        while let Some(at) = stack.pop() {
            if at == start {
                return true;
            }
            for &n in next.get(at).into_iter().flatten() {
                if seen.insert(n) {
                    stack.push(n);
                }
            }
        }
        false
    })
}

/// The columns of `rel` of declared type `ty` (`node`, `int`).
fn columns(prog: &Program, rel: &str, ty: &str) -> Vec<usize> {
    prog.decls.get(rel).map_or_else(Vec::new, |cols| cols.iter().enumerate().filter(|(_, (_, t))| t == ty).map(|(i, _)| i).collect())
}

/// What the relations of `component` (a lower stratum, recursive or not) guarantee between their node columns in
/// every row: the greatest set of claims each of their rules keeps when the component's own atoms keep them.
fn guarantees(prog: &Program, component: &BTreeSet<String>, by_head: &BTreeMap<&str, Vec<&Rule>>, moves: &Moves) -> Moves {
    // Rows no rule made (facts, an input's) keep nothing.
    let free = |rel: &str| prog.facts.get(rel).is_some_and(|rows| !rows.is_empty()) || prog.inputs.iter().any(|i| i == rel);
    let mut claims = Moves::new();
    for rel in component.iter().filter(|rel| by_head.contains_key(rel.as_str()) && !free(rel)) {
        let nodes = columns(prog, rel, "node");
        let every: BTreeSet<Step> = [Move::Down, Move::Up].into_iter().flat_map(|way| [(way, true), (way, false)]).collect();
        let pairs: BTreeMap<(usize, usize), BTreeSet<Step>> =
            nodes.iter().flat_map(|&i| nodes.iter().filter(move |&&j| j != i).map(move |&j| (i, j))).map(|p| (p, every.clone())).collect();
        if !pairs.is_empty() {
            claims.insert(rel.clone(), pairs);
        }
    }
    while !claims.is_empty() {
        let mut assumed = moves.clone();
        assumed.extend(claims.iter().map(|(rel, pairs)| (rel.clone(), pairs.clone())));
        let mut changed = false;
        for (rel, pairs) in &mut claims {
            let rules = &by_head[rel.as_str()];
            let rule_edges: Vec<_> = rules.iter().map(|rule| edges(rule, &assumed)).collect();
            for (&(i, j), steps) in pairs.iter_mut() {
                steps.retain(|&(way, strict)| {
                    let held = rules.iter().zip(&rule_edges).all(|(rule, edges)| {
                        let (Some(Term::Var(x)), Some(Term::Var(y))) = (rule.head.args.get(i), rule.head.args.get(j)) else { return false };
                        matches!(reach(edges, x, y, way), Some(s) if s || !strict)
                    });
                    changed |= !held;
                    held
                });
            }
            pairs.retain(|_, steps| !steps.is_empty());
        }
        if !changed {
            break;
        }
    }
    claims.retain(|_, pairs| !pairs.is_empty());
    claims
}

/// Whether, along `way`, keys `key` (one node column per relation of `component`) bound every recursive rule.
fn bounded(rules: &[&Rule], component: &BTreeSet<String>, key: &BTreeMap<&str, usize>, way: Move, moves: &Moves) -> Result<(), String> {
    for rule in rules {
        let Some(Term::Var(h)) = rule.head.args.get(key[rule.head.rel.as_str()]) else { return Err(rule.to_string()) };
        let edges = edges(rule, moves);
        let strict = creates(rule);
        for literal in &rule.body {
            let Some(a) = literal.atom().filter(|a| component.contains(&a.rel)) else { continue };
            let Some(Term::Var(b)) = a.args.get(key[a.rel.as_str()]) else { return Err(rule.to_string()) };
            match reach(&edges, h, b, way) {
                Some(true) => {}
                Some(false) if !strict => {}
                _ => return Err(rule.to_string()),
            }
        }
    }
    Ok(())
}

/// `t` with each variable an equality of the body gives (`$v = term`) replaced by its term, a few levels deep:
/// `$t` of `$t = @add($i, 1)` is `@add($i, 1)`.
fn resolved(t: &Term, defs: &BTreeMap<&str, &Term>, depth: usize) -> Term {
    match t {
        Term::Var(v) if depth > 0 => defs.get(v.as_str()).map_or_else(|| t.clone(), |d| resolved(d, defs, depth - 1)),
        Term::Call(f, args) => Term::Call(f.clone(), args.iter().map(|a| resolved(a, defs, depth)).collect()),
        _ => t.clone(),
    }
}

/// `k` where `to` is `from + k` (`@add(from, k)`, `@sub(from, -k)`, or `from` itself for 0).
fn offset(from: &Term, to: &Term) -> Option<i64> {
    if from == to {
        return Some(0);
    }
    match to {
        Term::Call(f, args) if f == "add" => match args.as_slice() {
            [x, Term::Int(k)] | [Term::Int(k), x] if x == from => Some(*k),
            _ => None,
        },
        Term::Call(f, args) if f == "sub" => match args.as_slice() {
            [x, Term::Int(k)] if x == from => Some(-k),
            _ => None,
        },
        _ => None,
    }
}

/// Whether keys `key` (a node column and an integer column per relation of `component`) bound every recursive
/// rule by position: the node kept, the integer moved by a constant the same way in every rule (`rising`: up),
/// strictly where a rule creates a value and only where the head's integer is a child index of the node.
fn positioned(rules: &[&Rule], component: &BTreeSet<String>, key: &BTreeMap<&str, (usize, usize)>, rising: bool) -> Result<(), String> {
    for rule in rules {
        let (nc, ic) = key[rule.head.rel.as_str()];
        let Some(Term::Var(h)) = rule.head.args.get(nc) else { return Err(rule.to_string()) };
        let defs: BTreeMap<&str, &Term> = rule
            .body
            .iter()
            .filter_map(|l| match l {
                Literal::Cmp { l: Term::Var(v), op: "=", r } => Some((v.as_str(), r)),
                _ => None,
            })
            .collect();
        let at = resolved(&rule.head.args[ic], &defs, 8);
        let edges = edges(rule, &Moves::new());
        let placed = rule.body.iter().any(|l| {
            matches!(l, Literal::Atom(a) if !a.neg && a.rel == "child"
                && matches!(&a.args[..], [Term::Var(p), _, i] if same(&edges, h, p) && resolved(i, &defs, 8) == at))
        });
        let strict = creates(rule);
        for literal in &rule.body {
            let Some(a) = literal.atom().filter(|a| component.contains(&a.rel)) else { continue };
            let (bn, bi) = key[a.rel.as_str()];
            let Some(Term::Var(b)) = a.args.get(bn) else { return Err(rule.to_string()) };
            let step = offset(&resolved(&a.args[bi], &defs, 8), &at).map(|k| if rising { k } else { -k });
            match step {
                _ if !same(&edges, h, b) => return Err(rule.to_string()),
                Some(k) if k > 0 && placed => {}
                Some(0) if !strict => {}
                _ => return Err(rule.to_string()),
            }
        }
    }
    Ok(())
}

/// Assignments of a key to each relation, at most `limit` of them.
fn keys<'a, K: Copy>(choices: &[(&'a str, Vec<K>)], limit: usize) -> Vec<BTreeMap<&'a str, K>> {
    let mut out = vec![BTreeMap::new()];
    for (rel, options) in choices {
        out = out.into_iter().flat_map(|key| options.iter().map(move |&c| {
            let mut key = key.clone();
            key.insert(*rel, c);
            key
        })).take(limit).collect();
    }
    out
}

/// The components with a value cycle, each bounded by the tree; fails on one that is not
/// (`Error::UnboundedValueCycle`).
pub(in crate::code_rule_dsl) fn check(prog: &Program) -> Result<Vec<BTreeSet<String>>, Error> {
    // A program that does not stratify fails there, by name.
    let Ok((sccs, edges_by)) = super::strata(prog) else { return Ok(Vec::new()) };
    let mut treed = Vec::new();
    let mut moves = base();
    let mut by_head = BTreeMap::<&str, Vec<&Rule>>::new();
    for rule in &prog.rules {
        by_head.entry(rule.head.rel.as_str()).or_default().push(rule);
    }
    for component in &sccs {
        let recursive = component.len() > 1 || component.iter().any(|r| edges_by.get(r).is_some_and(|e| e.iter().any(|(w, _)| w == r)));
        let rules: Vec<&Rule> = component.iter().flat_map(|rel| by_head.get(rel.as_str()).into_iter().flatten().copied()).collect();
        if recursive && value_cycle(prog, &rules, component) {
            let choices: Vec<(&str, Vec<usize>)> = component.iter().map(|rel| (rel.as_str(), columns(prog, rel, "node"))).collect();
            let mut first = None;
            let mut fails = |r: Result<(), String>| match r {
                Ok(()) => true,
                Err(rule) => {
                    first.get_or_insert(rule);
                    false
                }
            };
            let tree = choices.iter().all(|(_, columns)| !columns.is_empty())
                && [Move::Down, Move::Up]
                    .into_iter()
                    .any(|way| keys(&choices, 4096).iter().any(|key| fails(bounded(&rules, component, key, way, &moves))));
            let ok = tree || {
                let positions: Vec<(&str, Vec<(usize, usize)>)> = component
                    .iter()
                    .map(|rel| {
                        let ints = columns(prog, rel, "int");
                        (rel.as_str(), columns(prog, rel, "node").into_iter().flat_map(|n| ints.iter().map(move |&i| (n, i))).collect())
                    })
                    .collect();
                positions.iter().all(|(_, options)| !options.is_empty())
                    && [true, false].into_iter().any(|rising| keys(&positions, 4096).iter().any(|key| positioned(&rules, component, key, rising).is_ok()))
            };
            if !ok {
                let rule = first.unwrap_or_else(|| rules.iter().find(|r| creates(r)).map(|r| r.to_string()).unwrap_or_default());
                return Err(Error::UnboundedValueCycle { component: component.clone(), rule });
            }
            treed.push(component.clone());
        }
        // The component's guarantees, for the components above it.
        let held = guarantees(prog, component, &by_head, &moves);
        moves.extend(held);
    }
    Ok(treed)
}
