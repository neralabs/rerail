//! O11 (option b): a recursion that creates values is bounded when it moves along the tree.
//!
//! §12 rejects "cycles through value-creating builtins with no bound". A value cycle is one in the flow of
//! values between columns: a value a builtin made (`@cat` and the others, in a head or bound by `$v = @f(…)`)
//! reaching that builtin's arguments again. A component whose rules make values that never come back (T's
//! `qv(@cat($q, $x), …)` in `calls.rdl`, whose `$q` and `$x` come from columns no made value reaches) makes
//! finitely many. A value made into a lattice's value column is bounded by the lattice (a finite height, or
//! `widen`: `UnboundedLattice`), as `cv`'s `@flat_bin` in T's workloads. The bound of any other value cycle is the syntax tree: its component is accepted when one node column of each of its relations (its key) moves one way along tree edges, every
//! rule the same way: from the head's key to each recursive atom's key, strictly down for every rule that
//! creates a value and down or not at all for the others (or all of them up). A tree is finite and acyclic,
//! so every derivation is at most its depth deep, and only finitely many values are made. T's programs
//! recurse this way: `qn` along `attr_of` (a `field` edge), `resolves` along `field`, `modn` along
//! `nearest`.
//!
//! Tree edges are the base relations' (`child(p, c, _)`, `field(p, _, c)`, `tape(n, par, …)`), `nearest`,
//! and what a derived relation of lower strata guarantees between its node columns in every one of its rules
//! (`attr_of(n, o, a)`: `o` is below `n`). Nothing else bounds a cycle: a relation over a graph (imports,
//! calls) may itself be cyclic. Decided 2026-11-03 (open-items O11). A builtin that gives part of its argument
//! (`shrinks`: `@pkg_up`, `substr`) makes no new value, so a cycle through it alone is bounded too.
//!
//! Decision 0046 widens the bound to what the `.rr` libraries recurse by. A node is its preorder index, so
//! `Down` is any move to a later node and `Up` to an earlier one: `next_sibling` and `precedes` move down too,
//! and so does `$a < $b` between two keys. A derived relation's guarantee may be weak (down or the same node,
//! `unparen(e, m)`), and a recursive relation's guarantees are its greatest self-consistent ones, so `unparen`
//! and `deparen` carry theirs into the rules above. A variable that a relation of a lower stratum binds takes
//! finitely many values, so nothing it reads flows; `@add($m, k)` under `$m < C` (and `@sub` under `$m > C`)
//! counts up to a bound; and `@rx_group`, `@slice`, `@head`, `@dir` and `@replace_all(…, "")` give a part of
//! their argument, as `substr` does.

use super::{Error, Literal, Program, Rule, Term};
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

/// A move and whether it is strict (`false`: the move or the same node).
type Step = (Move, bool);

/// The moves between a relation's columns that hold in every row: `(from, to) → steps`.
type Guarantees = BTreeMap<(usize, usize), BTreeSet<Step>>;

/// Per relation, its guarantees.
type Moves = BTreeMap<String, Guarantees>;

/// The base relations' tree edges, both ways, and the siblings' order (a later sibling is a later node).
fn base() -> Moves {
    let mut moves = Moves::new();
    for (rel, parent, child) in [("child", 0, 1), ("field", 0, 2), ("tape", 1, 0), ("next_sibling", 0, 1), ("precedes", 0, 1)] {
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
        out.entry(b.to_owned()).or_default().push((a.to_owned(), m.map(|(m, s)| (m.back(), s))));
    };
    for literal in &rule.body {
        match literal {
            Literal::Atom(a) if !a.neg => {
                for (&(i, j), ms) in moves.get(&a.rel).into_iter().flatten() {
                    if let (Some(Term::Var(x)), Some(Term::Var(y))) = (a.args.get(i), a.args.get(j)) {
                        ms.iter().for_each(|&m| add(x, y, Some(m)));
                    }
                }
            }
            Literal::Nearest { n, a, .. } => add(n, a, Some((Move::Up, true))),
            Literal::Cmp { l: Term::Var(x), op, r: Term::Var(y) } => match *op {
                "=" => add(x, y, None),
                "<" => add(x, y, Some((Move::Down, true))),
                "<=" => add(x, y, Some((Move::Down, false))),
                ">" => add(x, y, Some((Move::Up, true))),
                ">=" => add(x, y, Some((Move::Up, false))),
                _ => {}
            },
            _ => {}
        }
    }
    out
}

/// How `to` lies from `from` along `way`: `Some(true)` strictly, `Some(false)` at the same node, `None` not.
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

/// Whether a rule makes a value: a computed head term, or `$v = @f(…)` binding a head variable.
fn creates(rule: &Rule) -> bool {
    let head: BTreeSet<String> = rule.head.vars();
    rule.head.args.iter().any(|t| matches!(t, Term::Call(..)))
        || rule.body.iter().any(|l| matches!(l, Literal::Cmp { l: Term::Var(v), op: "=", r: Term::Call(..) } if head.contains(v)))
}

/// A builtin whose value is a part of its first argument (`@pkg_up(q, k)` a prefix of q, `substr` a substring):
/// a cycle through it alone gives only parts of values that entered it, finitely many, so it makes no new
/// value. (An extension of O11 for `.rr`'s Python library, `pkg_above` and `loads`: decision 0043.)
fn shrinks(t: &Term) -> bool {
    match t {
        Term::Call(f, args) => {
            matches!(f.as_str(), "pkg_up" | "substr" | "slice" | "head" | "dir" | "rx_group")
                || (f == "replace_all" && matches!(args.get(2), Some(Term::Str(s)) if s.is_empty()))
        }
        _ => false,
    }
}

/// Whether `t` counts toward a bound the rule sets: `@add($m, k)` with `k > 0` under `$m < C` or `$m <= C`,
/// or `@sub($m, k)` under `$m > C` or `$m >= C`. Every value such a cycle makes lies between its seeds and
/// the bounds, finitely many.
fn counts(t: &Term, rule: &Rule) -> bool {
    let Term::Call(f, args) = t else { return false };
    let (m, up) = match (f.as_str(), args.as_slice()) {
        ("add", [Term::Var(m), Term::Int(k)] | [Term::Int(k), Term::Var(m)]) if *k > 0 => (m, true),
        ("sub", [Term::Var(m), Term::Int(k)]) if *k > 0 => (m, false),
        _ => return false,
    };
    rule.body.iter().any(|l| match l {
        Literal::Cmp { l: Term::Var(x), op, r: Term::Int(_) } if x == m => if up { matches!(*op, "<" | "<=") } else { matches!(*op, ">" | ">=") },
        Literal::Cmp { l: Term::Int(_), op, r: Term::Var(x) } if x == m => if up { matches!(*op, ">" | ">=") } else { matches!(*op, "<" | "<=") },
        _ => false,
    })
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
        // A variable a relation of a lower stratum binds takes finitely many values, whatever reaches it.
        let finite: BTreeSet<&String> = rule
            .body
            .iter()
            .filter_map(|l| match l {
                Literal::Atom(a) if !a.neg && !component.contains(&a.rel) => Some(a),
                _ => None,
            })
            .flat_map(|a| a.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v) } else { None }))
            .collect();
        for literal in &rule.body {
            if let Some(a) = literal.atom().filter(|a| !a.neg) {
                for (i, t) in a.args.iter().enumerate() {
                    if let Term::Var(v) = t {
                        bound.insert(v.clone());
                        if component.contains(&a.rel) && !finite.contains(v) {
                            from.entry(v.clone()).or_default().insert(((a.rel.clone(), i), false));
                        }
                    }
                }
            }
        }
        let sources = |t: &Term, from: &BTreeMap<String, BTreeSet<(Column, bool)>>| -> BTreeSet<(Column, bool)> {
            let mut vars = BTreeSet::new();
            t.vars(&mut vars);
            let made = matches!(t, Term::Call(..)) && !shrinks(t) && !counts(t, rule);
            vars.retain(|v| !finite.contains(v));
            vars.iter().flat_map(|v| from.get(v).into_iter().flatten().map(move |(c, m)| (c.clone(), *m || made))).collect()
        };
        // Assignments in order, until nothing more flows (an assignment may read another's variable).
        loop {
            let before = from.values().map(BTreeSet::len).sum::<usize>();
            for literal in &rule.body {
                if let Literal::Cmp { l: Term::Var(v), op: "=", r } = literal
                    && !bound.contains(v)
                    && !finite.contains(v)
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

/// The columns of `rel` that hold nodes, as declared.
fn node_columns(prog: &Program, rel: &str) -> Vec<usize> {
    prog.decls.get(rel).map_or_else(Vec::new, |cols| cols.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect())
}

/// What `rules` (one relation) guarantee between its node columns in every rule, given `moves`.
fn summary(prog: &Program, rules: &[&Rule], moves: &Moves) -> Guarantees {
    let columns = node_columns(prog, &rules[0].head.rel);
    let edges: Vec<_> = rules.iter().map(|rule| edges(rule, moves)).collect();
    let mut out = Guarantees::new();
    for &i in &columns {
        for &j in columns.iter().filter(|&&j| j != i) {
            let mut held = BTreeSet::new();
            for way in [Move::Down, Move::Up] {
                let reached: Option<Vec<bool>> = rules
                    .iter()
                    .zip(&edges)
                    .map(|(rule, edges)| {
                        let (Some(Term::Var(x)), Some(Term::Var(y))) = (rule.head.args.get(i), rule.head.args.get(j)) else { return None };
                        reach(edges, x, y, way)
                    })
                    .collect();
                if let Some(reached) = reached {
                    held.insert((way, reached.iter().all(|&s| s)));
                }
            }
            if !held.is_empty() {
                out.insert((i, j), held);
            }
        }
    }
    out
}

/// A step that holds under both guarantees: the weaker of the two.
fn meet(a: &Guarantees, b: &Guarantees) -> Guarantees {
    let mut out = Guarantees::new();
    for (pair, steps) in a {
        let Some(other) = b.get(pair) else { continue };
        let held: BTreeSet<Step> = steps
            .iter()
            .filter_map(|&(m, s)| other.iter().find(|(o, _)| *o == m).map(|&(_, t)| (m, s && t)))
            .collect();
        if !held.is_empty() {
            out.insert(*pair, held);
        }
    }
    out
}

/// The guarantees of a component's relations: for a recursive one the greatest that its rules keep, from
/// every move strict down.
fn summaries(prog: &Program, component: &BTreeSet<String>, by_head: &BTreeMap<&str, Vec<&Rule>>, moves: &mut Moves) {
    let rels: Vec<&String> = component.iter().filter(|rel| by_head.contains_key(rel.as_str()) && node_columns(prog, rel).len() > 1).collect();
    for rel in &rels {
        let columns = node_columns(prog, rel);
        let all: Guarantees = columns
            .iter()
            .flat_map(|&i| columns.iter().filter(move |&&j| j != i).map(move |&j| ((i, j), BTreeSet::from([(Move::Down, true), (Move::Up, true)]))))
            .collect();
        moves.insert((*rel).clone(), all);
    }
    for _ in 0..64 {
        let mut changed = false;
        for rel in &rels {
            let next = summary(prog, &by_head[rel.as_str()], moves);
            let next = meet(&moves[rel.as_str()], &next);
            if next != moves[rel.as_str()] {
                moves.insert((*rel).clone(), next);
                changed = true;
            }
        }
        if !changed {
            return;
        }
    }
    // Not settled: claim nothing.
    for rel in rels {
        moves.remove(rel.as_str());
    }
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

/// Assignments of a key column to each relation, at most `limit` of them.
fn keys<'a>(choices: &[(&'a str, Vec<usize>)], limit: usize) -> Vec<BTreeMap<&'a str, usize>> {
    let mut out = vec![BTreeMap::new()];
    for (rel, columns) in choices {
        out = out.into_iter().flat_map(|key| columns.iter().map(move |&c| {
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
        // The component's guarantees, for its own rules and the components above it.
        summaries(prog, component, &by_head, &mut moves);
        if !recursive {
            continue;
        }
        let rules: Vec<&Rule> = component.iter().flat_map(|rel| by_head.get(rel.as_str()).into_iter().flatten().copied()).collect();
        if !value_cycle(prog, &rules, component) {
            continue;
        }
        let choices: Vec<(&str, Vec<usize>)> = component.iter().map(|rel| (rel.as_str(), node_columns(prog, rel))).collect();
        let mut first = None;
        let ok = choices.iter().all(|(_, columns)| !columns.is_empty())
            && [Move::Down, Move::Up].into_iter().any(|way| {
                keys(&choices, 4096).iter().any(|key| match bounded(&rules, component, key, way, &moves) {
                    Ok(()) => true,
                    Err(rule) => {
                        first.get_or_insert(rule);
                        false
                    }
                })
            });
        if !ok {
            let rule = first.unwrap_or_else(|| rules.iter().find(|r| creates(r)).map(|r| r.to_string()).unwrap_or_default());
            return Err(Error::UnboundedValueCycle { component: component.clone(), rule });
        }
        treed.push(component.clone());
    }
    Ok(treed)
}
