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

/// Per relation, the strict moves between its columns that hold in every row: `(from, to) → moves`.
type Moves = BTreeMap<String, BTreeMap<(usize, usize), BTreeSet<Move>>>;

/// The base relations' tree edges, both ways.
fn base() -> Moves {
    let mut moves = Moves::new();
    for (rel, parent, child) in [("child", 0, 1), ("field", 0, 2), ("tape", 1, 0)] {
        let edges = moves.entry(rel.to_owned()).or_default();
        edges.entry((parent, child)).or_default().insert(Move::Down);
        edges.entry((child, parent)).or_default().insert(Move::Up);
    }
    moves
}

/// A rule's variables' edges: `var → [(var, Some(move) or None for the same node)]`.
fn edges(rule: &Rule, moves: &Moves) -> BTreeMap<String, Vec<(String, Option<Move>)>> {
    let mut out = BTreeMap::<String, Vec<(String, Option<Move>)>>::new();
    let mut add = |a: &str, b: &str, m: Option<Move>| {
        out.entry(a.to_owned()).or_default().push((b.to_owned(), m));
        out.entry(b.to_owned()).or_default().push((a.to_owned(), m.map(Move::back)));
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
            Literal::Nearest { n, a, .. } => add(n, a, Some(Move::Up)),
            Literal::Cmp { l: Term::Var(x), op: "=", r: Term::Var(y) } => add(x, y, None),
            _ => {}
        }
    }
    out
}

/// How `to` lies from `from` along `way`: `Some(true)` strictly, `Some(false)` at the same node, `None` not.
fn reach(edges: &BTreeMap<String, Vec<(String, Option<Move>)>>, from: &str, to: &str, way: Move) -> Option<bool> {
    let mut seen = BTreeSet::from([(from.to_owned(), false)]);
    let mut queue = VecDeque::from([(from.to_owned(), false)]);
    while let Some((at, strict)) = queue.pop_front() {
        for (next, m) in edges.get(&at).into_iter().flatten() {
            if m.is_some_and(|m| m != way) {
                continue;
            }
            let state = (next.clone(), strict || m.is_some());
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

/// The columns of `rel` that hold nodes, as declared.
fn node_columns(prog: &Program, rel: &str) -> Vec<usize> {
    prog.decls.get(rel).map_or_else(Vec::new, |cols| cols.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect())
}

/// What `rules` (a relation of a lower stratum) guarantee between its node columns in every rule.
fn summary(prog: &Program, rules: &[&Rule], moves: &Moves) -> BTreeMap<(usize, usize), BTreeSet<Move>> {
    let columns = node_columns(prog, &rules[0].head.rel);
    let mut out = BTreeMap::new();
    for &i in &columns {
        for &j in columns.iter().filter(|&&j| j != i) {
            let held: BTreeSet<Move> = [Move::Down, Move::Up]
                .into_iter()
                .filter(|&way| {
                    rules.iter().all(|rule| {
                        let (Some(Term::Var(x)), Some(Term::Var(y))) = (rule.head.args.get(i), rule.head.args.get(j)) else { return false };
                        reach(&edges(rule, moves), x, y, way) == Some(true)
                    })
                })
                .collect();
            if !held.is_empty() {
                out.insert((i, j), held);
            }
        }
    }
    out
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
        if !recursive {
            // A lower stratum's guarantees, for the components above it.
            for rel in component {
                if let Some(rules) = by_head.get(rel.as_str()) {
                    let summary = summary(prog, rules, &moves);
                    if !summary.is_empty() {
                        moves.insert(rel.clone(), summary);
                    }
                }
            }
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
