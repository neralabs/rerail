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
//!
//! A key may also be a number column that counts: the head's is `@add($b, k)` (`k > 0`) of the recursive
//! atom's `$b`, which a lower stratum binds or `$b < C` caps, so it rises through finitely many values. And a
//! relation of the component with no node column (`negate(a, b)` between `number`'s rows) needs no key when
//! it is a function: no rule of it reads such a relation of the component, its output columns are made from
//! its input columns (each a column a positive atom binds, or a constant) and finite values; every rule that
//! reads it is then a creating one, and binds the function's inputs by an atom strictly along the key.
//! A text column may be the key too: `@rx_group($r, re, g)` is strictly shorter than `$r` when `re` matches
//! something outside `g` in every match (`rest` after a `%%`), and the parts of finitely many texts are
//! finitely many. A key is a column and, at the same node, a count that may rise but not fall
//! (`tpl_upto(t, i + 1, …)` keeps its template `t`); a creating rule may keep the key when no cycle of rules
//! that keep it comes back (`deps_dkey(n, …)` from `deps_dchain(n, …)`, which reads a node below `n`).

use super::{Error, Literal, Program, Rule, Term};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// A move along the tree from one node to another.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Move {
    Down,
    Up,
    /// From a text to a part of it.
    Shorter,
    Longer,
    /// To a node nearer the root, and further from it (`Depths`).
    Shallower,
    Deeper,
    /// To a greater and a smaller value, by `<` between two variables, where that is not preorder: under node ids
    /// per definition (`RERAIL_NODE_IDS=definition`, decision 0094 §2) a node's id follows preorder only inside its
    /// granule, so `$a < $b` and a tree edge are moves in two orders, and a cycle may not mix them.
    Later,
    Earlier,
}

impl Move {
    fn back(self) -> Self {
        match self {
            Move::Down => Move::Up,
            Move::Up => Move::Down,
            Move::Shorter => Move::Longer,
            Move::Longer => Move::Shorter,
            Move::Shallower => Move::Deeper,
            Move::Deeper => Move::Shallower,
            Move::Later => Move::Earlier,
            Move::Earlier => Move::Later,
        }
    }
}

/// The text `t` is a part of, and whether strictly: `@rx_group($x, re, g)` is strictly shorter than `$x`
/// when what `re` matches outside the group `g` is never empty, and the other shrinking built-ins give a
/// part that may be the whole.
fn part_of(t: &Term) -> Option<(&String, bool)> {
    let Term::Call(f, args) = t else { return None };
    let Some(Term::Var(x)) = args.first() else { return None };
    if let (true, [_, Term::Str(re), Term::Str(g)]) = (f == "rx_group", args.as_slice()) {
        return Some((x, strict_group(re, g)));
    }
    shrinks(t).then_some((x, false))
}

/// Whether every match of `re` has text outside its group `g`.
fn strict_group(re: &str, g: &str) -> bool {
    use regex_syntax::hir::{Capture, Hir, HirKind, Repetition};
    fn strip(h: &Hir, g: &str) -> Hir {
        match h.kind() {
            HirKind::Capture(c) if c.name.as_deref() == Some(g) => Hir::empty(),
            HirKind::Capture(c) => Hir::capture(Capture { index: c.index, name: c.name.clone(), sub: Box::new(strip(&c.sub, g)) }),
            HirKind::Concat(hs) => Hir::concat(hs.iter().map(|h| strip(h, g)).collect()),
            HirKind::Alternation(hs) => Hir::alternation(hs.iter().map(|h| strip(h, g)).collect()),
            HirKind::Repetition(r) => Hir::repetition(Repetition { sub: Box::new(strip(&r.sub, g)), ..r.clone() }),
            _ => h.clone(),
        }
    }
    regex_syntax::parse(re).is_ok_and(|h| strip(&h, g).properties().minimum_len().is_some_and(|n| n > 0))
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
/// Whether `<` between variables is its own order (`Move::Later`), not preorder: node ids per definition.
fn by_value() -> bool {
    crate::code_rule_dsl::facts::vtab::syntax::definition_ids()
}

/// The orders a key may move along: the tree's, and under node ids per definition, values' (`Move::Later`).
fn ways() -> Vec<Move> {
    let mut out = vec![Move::Down, Move::Up];
    if by_value() {
        out.extend([Move::Later, Move::Earlier]);
    }
    out
}

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
            Literal::Cmp { l: Term::Var(v), op: "=", r: r @ Term::Call(..) } => {
                if let Some((x, strict)) = part_of(r) {
                    add(x, v, Some((Move::Shorter, strict)));
                }
            }
            Literal::Cmp { l: Term::Var(x), op, r: Term::Var(y) } => {
                // A node is its preorder index, so `<` moves down, unless ids are per definition (`Move::Later`).
                let (later, earlier) = if by_value() { (Move::Later, Move::Earlier) } else { (Move::Down, Move::Up) };
                match *op {
                    "=" => add(x, y, None),
                    "<" => add(x, y, Some((later, true))),
                    "<=" => add(x, y, Some((later, false))),
                    ">" => add(x, y, Some((earlier, true))),
                    ">=" => add(x, y, Some((earlier, false))),
                    _ => {}
                }
            }
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

/// Depth, a second measure beside preorder: how much deeper one node column lies than another, as an
/// interval `[lo, hi]` (`INF` for no bound that side). Every tree edge changes depth by exactly one, so the
/// moves along a path add up, whatever they are; preorder only knows their direction. A climb through a
/// decorator's expression to the definition it decorates (`ast_parent(e, f) :- decorator(f, e)`) moves to a
/// later node, so preorder cannot bound it, but its depth falls by one: up two edges, down one.
type Span = (i64, i64);
const INF: i64 = 1 << 40;
/// No row: the start of a recursive relation's fixpoint.
const EMPTY: Span = (INF, -INF);

/// Per relation, the depth from one of its node columns to another that holds in every row.
type Depths = BTreeMap<String, BTreeMap<(usize, usize), Span>>;

fn add_span(a: Span, b: Span) -> Span {
    let lo = if a.0 <= -INF || b.0 <= -INF { -INF } else { (a.0 + b.0).min(INF) };
    let hi = if a.1 >= INF || b.1 >= INF { INF } else { (a.1 + b.1).max(-INF) };
    (lo, hi)
}

fn back_span((lo, hi): Span) -> Span {
    (hi.saturating_neg(), lo.saturating_neg())
}

/// The base relations' depths: a child one deeper than its parent, a next sibling as deep.
fn depth_base() -> Depths {
    let mut depths = Depths::new();
    for (rel, from, to, d) in [("child", 0, 1, 1), ("field", 0, 2, 1), ("tape", 1, 0, 1), ("next_sibling", 0, 1, 0)] {
        let pairs = depths.entry(rel.to_owned()).or_default();
        pairs.insert((from, to), (d, d));
        pairs.insert((to, from), (-d, -d));
    }
    depths
}

/// A rule's variables' depth edges: `var → [(var, span)]`.
fn depth_edges(rule: &Rule, depths: &Depths) -> BTreeMap<String, Vec<(String, Span)>> {
    let mut out = BTreeMap::<String, Vec<(String, Span)>>::new();
    let mut add = |a: &str, b: &str, d: Span| {
        out.entry(a.to_owned()).or_default().push((b.to_owned(), d));
        out.entry(b.to_owned()).or_default().push((a.to_owned(), back_span(d)));
    };
    for literal in &rule.body {
        match literal {
            Literal::Atom(a) if !a.neg => {
                for (&(i, j), &d) in depths.get(&a.rel).into_iter().flatten() {
                    if let (Some(Term::Var(x)), Some(Term::Var(y))) = (a.args.get(i), a.args.get(j)) {
                        add(x, y, d);
                    }
                }
            }
            Literal::Nearest { n, a, .. } => add(n, a, (-INF, -1)),
            Literal::Cmp { l: Term::Var(x), op: "=", r: Term::Var(y) } => add(x, y, (0, 0)),
            _ => {}
        }
    }
    out
}

/// The depth of each variable from `from`: per variable the tightest bounds that some walk gives (each walk's
/// sum is a valid bound, so the least `hi` and the greatest `lo` of any are).
fn depth_reach(edges: &BTreeMap<String, Vec<(String, Span)>>, from: &str) -> BTreeMap<String, Span> {
    let mut at = BTreeMap::from([(from.to_owned(), (0, 0))]);
    for _ in 0..=edges.len() {
        let mut changed = false;
        for (x, &dx) in at.clone().iter() {
            for (y, d) in edges.get(x).into_iter().flatten() {
                let (lo, hi) = add_span(dx, *d);
                let e = at.entry(y.clone()).or_insert((-INF, INF));
                let next = (e.0.max(lo), e.1.min(hi));
                if next != *e {
                    *e = next;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    at
}

/// What `rules` (one relation) guarantee in depth between its node columns, given `depths`: per pair the
/// widest of its rules' spans; a pair some rule does not connect has none.
fn depth_summary(prog: &Program, rules: &[&Rule], depths: &Depths) -> BTreeMap<(usize, usize), Span> {
    let columns = node_columns(prog, &rules[0].head.rel);
    let reached: Vec<BTreeMap<usize, BTreeMap<String, Span>>> = rules
        .iter()
        .map(|rule| {
            let edges = depth_edges(rule, depths);
            columns.iter().filter_map(|&i| match rule.head.args.get(i) { Some(Term::Var(x)) => Some((i, depth_reach(&edges, x))), _ => None }).collect()
        })
        .collect();
    let mut out = BTreeMap::new();
    for &i in &columns {
        for &j in columns.iter().filter(|&&j| j != i) {
            let mut span = EMPTY;
            let mut all = true;
            for (rule, reached) in rules.iter().zip(&reached) {
                let d = match (reached.get(&i), rule.head.args.get(j)) {
                    (Some(r), Some(Term::Var(y))) => r.get(y).copied(),
                    _ => None,
                };
                let Some(d) = d.filter(|d| d.0 > -INF || d.1 < INF) else {
                    all = false;
                    break;
                };
                span = (span.0.min(d.0), span.1.max(d.1));
            }
            if all {
                out.insert((i, j), span);
            }
        }
    }
    out
}

/// The depth guarantees of a component's relations: for a recursive one the least spans its rules keep,
/// from no rows up, widened to no bound on a side that still moves after a few rounds.
fn depth_summaries(prog: &Program, component: &BTreeSet<String>, by_head: &BTreeMap<&str, Vec<&Rule>>, depths: &mut Depths) {
    let rels: Vec<&String> = component.iter().filter(|rel| by_head.contains_key(rel.as_str()) && node_columns(prog, rel).len() > 1).collect();
    for rel in &rels {
        let columns = node_columns(prog, rel);
        let empty = columns.iter().flat_map(|&i| columns.iter().filter(move |&&j| j != i).map(move |&j| ((i, j), EMPTY))).collect();
        depths.insert((*rel).clone(), empty);
    }
    for round in 0..64 {
        let mut changed = false;
        for rel in &rels {
            let next = depth_summary(prog, &by_head[rel.as_str()], depths);
            let old = &depths[rel.as_str()];
            // Joined with what held (a pair either lacks is unbounded), and widened past round 8.
            let joined: BTreeMap<(usize, usize), Span> = next
                .into_iter()
                .filter_map(|(pair, (lo, hi))| {
                    let &(olo, ohi) = old.get(&pair)?;
                    let (mut lo, mut hi) = (lo.min(olo), hi.max(ohi));
                    if round >= 8 {
                        if lo < olo {
                            lo = -INF;
                        }
                        if hi > ohi {
                            hi = INF;
                        }
                    }
                    (lo > -INF || hi < INF).then_some((pair, (lo, hi)))
                })
                .collect();
            if &joined != old {
                depths.insert((*rel).clone(), joined);
                changed = true;
            }
        }
        if !changed {
            return;
        }
    }
    for rel in rels {
        depths.remove(rel.as_str());
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
        let finite = finite_vars(rule, component);
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
        // A head term that a finite variable is assigned (`$t = @add($l, 1), lines(d, $t, _)` and the head's
        // `@add($l, 1)`, as `.rr` lowers a call it binds and also writes) takes that variable's finitely many values
        // (O111: `era001`'s `era_run`).
        let finite_terms: Vec<&Term> = rule
            .body
            .iter()
            .filter_map(|l| match l {
                Literal::Cmp { l: Term::Var(v), op: "=", r } if finite.contains(v) => Some(r),
                _ => None,
            })
            .collect();
        for (j, t) in rule.head.args.iter().enumerate() {
            if finite_terms.contains(&t) {
                continue;
            }
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
            for way in ways() {
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
            .flat_map(|&i| columns.iter().filter(move |&&j| j != i).map(move |&j| ((i, j), ways().into_iter().map(|m| (m, true)).collect::<BTreeSet<_>>())))
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

/// The variables a rule's positive atoms of relations outside `component` bind: finitely many values each.
fn finite_vars<'a>(rule: &'a Rule, component: &BTreeSet<String>) -> BTreeSet<&'a String> {
    rule.body
        .iter()
        .filter_map(|l| match l {
            Literal::Atom(a) if !a.neg && !component.contains(&a.rel) => Some(a),
            _ => None,
        })
        .flat_map(|a| a.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v) } else { None }))
        .collect()
}

/// The variable a counting key term rises from: `@add($b, k)` with `k > 0`, `$b` finite or capped.
fn counter<'a>(t: &'a Term, rule: &Rule, finite: &BTreeSet<&String>) -> Option<&'a String> {
    let Term::Call(f, args) = t else { return None };
    match (f.as_str(), args.as_slice()) {
        ("add", [Term::Var(b), Term::Int(k)] | [Term::Int(k), Term::Var(b)]) if *k > 0 && (finite.contains(b) || counts(t, rule)) => Some(b),
        _ => None,
    }
}

/// For a relation of `component` with no node column, its input columns when it is a function of them.
fn function(prog: &Program, rules: &[&Rule], component: &BTreeSet<String>, keyless: &BTreeSet<&str>) -> Option<BTreeSet<usize>> {
    let arity = rules.first()?.head.args.len();
    let bound_by_atom = |rule: &Rule, v: &String| rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if !a.neg && a.args.contains(&Term::Var(v.clone()))));
    let inputs: BTreeSet<usize> = (0..arity)
        .filter(|&j| {
            rules.iter().all(|rule| match &rule.head.args[j] {
                Term::Int(_) | Term::Str(_) => true,
                Term::Var(v) => bound_by_atom(rule, v),
                _ => false,
            })
        })
        .collect();
    for rule in rules {
        if rule.body.iter().any(|l| l.atom().is_some_and(|a| keyless.contains(a.rel.as_str()))) {
            return None;
        }
        let mut known: BTreeSet<String> = finite_vars(rule, component).into_iter().cloned().collect();
        for &j in &inputs {
            rule.head.args[j].vars(&mut known);
        }
        loop {
            let before = known.len();
            for l in &rule.body {
                if let Literal::Cmp { l: Term::Var(v), op: "=", r } = l {
                    let mut vars = BTreeSet::new();
                    r.vars(&mut vars);
                    if vars.is_subset(&known) {
                        known.insert(v.clone());
                    }
                }
            }
            if known.len() == before {
                break;
            }
        }
        for (_, t) in rule.head.args.iter().enumerate().filter(|(j, _)| !inputs.contains(j)) {
            let mut vars = BTreeSet::new();
            t.vars(&mut vars);
            if !vars.is_subset(&known) {
                return None;
            }
        }
    }
    let _ = prog;
    Some(inputs)
}

/// A key: the column that moves (a node, or a count), and a count that rises while it stays.
type Key = (usize, Option<usize>);

/// How a recursive atom's key lies from the head's.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lies {
    /// Strictly along the way, or at the same place with a count that rises.
    Before,
    /// At the same place and count.
    Same,
}

/// Whether, along `way`, keys `key` (a node or counting column per keyed relation of `component`, with
/// perhaps a count) bound every recursive rule; `functions` are the keyless relations, with their inputs.
/// Every step from an atom to the head moves the key along `way` or keeps it; a count kept at the same place
/// may rise but not fall; and a creating step that keeps the key lies on no cycle of such steps. Along any
/// derivation the key then settles, and after that no value is made.
fn bounded(
    rules: &[&Rule],
    component: &BTreeSet<String>,
    key: &BTreeMap<&str, Key>,
    functions: &BTreeMap<&str, BTreeSet<usize>>,
    way: Move,
    moves: &Moves,
    depths: &Depths,
) -> Result<(), String> {
    let mut same = BTreeMap::<&str, BTreeSet<&str>>::new();
    let mut creating_same = Vec::new();
    for rule in rules.iter().filter(|r| !functions.contains_key(r.head.rel.as_str())) {
        let edges = edges(rule, moves);
        // Along depth, a key moves strictly or not at all: the head's node, and where each atom's lies.
        let by_depth = matches!(way, Move::Shallower | Move::Deeper);
        let depth_from = match (by_depth, rule.head.args.get(key[rule.head.rel.as_str()].0)) {
            (true, Some(Term::Var(h))) => Some(depth_reach(&depth_edges(rule, depths), h)),
            _ => None,
        };
        let finite = finite_vars(rule, component);
        let (primary, count) = key[rule.head.rel.as_str()];
        let head_primary = rule.head.args.get(primary);
        let head_count = count.and_then(|c| rule.head.args.get(c));
        let uses_function = rule.body.iter().any(|l| l.atom().is_some_and(|a| functions.contains_key(a.rel.as_str())));
        let creating = creates(rule) || uses_function;
        let lies = |a: &super::Atom| -> Option<Lies> {
            let (p, c) = key[a.rel.as_str()];
            let Some(Term::Var(b)) = a.args.get(p) else { return None };
            if by_depth {
                let &(lo, hi) = depth_from.as_ref()?.get(b)?;
                let strict = if way == Move::Shallower { hi <= -1 } else { lo >= 1 };
                return (strict && c.is_none()).then_some(Lies::Before);
            }
            match head_primary? {
                Term::Var(h) => match reach(&edges, h, b, way)? {
                    true => Some(Lies::Before),
                    false => match (head_count, c.and_then(|c| a.args.get(c))) {
                        (None, None) => Some(Lies::Same),
                        // No count is above every count.
                        (None, Some(_)) => Some(Lies::Before),
                        (Some(_), None) => None,
                        (Some(t), Some(Term::Var(bc))) => match t {
                            Term::Var(hc) => reach(&edges, hc, bc, Move::Up).map(|s| if s { Lies::Before } else { Lies::Same }),
                            t => {
                                let y = counter(t, rule, &finite)?;
                                reach(&edges, y, bc, Move::Up).map(|_| Lies::Before)
                            }
                        },
                        _ => None,
                    },
                },
                // A head key made from the atom's: a count up from it, or a strict part of it.
                t => {
                    let y = match (way, part_of(t)) {
                        (Move::Up, _) => counter(t, rule, &finite)?,
                        (Move::Longer, Some((y, true))) => y,
                        _ => return None,
                    };
                    reach(&edges, y, b, way).map(|_| Lies::Before)
                }
            }
        };
        // The variables an atom before the head binds, for the functions' inputs.
        let mut before = BTreeSet::<&String>::new();
        for literal in &rule.body {
            let Some(a) = literal.atom().filter(|a| component.contains(&a.rel) && !functions.contains_key(a.rel.as_str())) else { continue };
            match lies(a) {
                Some(Lies::Before) => {
                    if matches!(literal, Literal::Atom(x) if !x.neg) {
                        before.extend(a.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v) } else { None }));
                    }
                }
                Some(Lies::Same) => {
                    same.entry(a.rel.as_str()).or_default().insert(rule.head.rel.as_str());
                    if creating {
                        creating_same.push((a.rel.as_str(), rule.head.rel.as_str(), rule));
                    }
                }
                None => return Err(rule.to_string()),
            }
        }
        for literal in &rule.body {
            let Some(a) = literal.atom().filter(|a| functions.contains_key(a.rel.as_str())) else { continue };
            for &j in &functions[a.rel.as_str()] {
                match a.args.get(j) {
                    Some(Term::Var(v)) if before.contains(v) || finite.contains(v) => {}
                    Some(Term::Int(_) | Term::Str(_)) => {}
                    _ => return Err(rule.to_string()),
                }
            }
        }
    }
    // A creating step that keeps the key, from `a` to `h`, may not come back to `a` by such steps.
    for (a, h, rule) in creating_same {
        let mut seen = BTreeSet::from([h]);
        let mut stack = vec![h];
        while let Some(at) = stack.pop() {
            if at == a {
                return Err(rule.to_string());
            }
            for &n in same.get(at).into_iter().flatten() {
                if seen.insert(n) {
                    stack.push(n);
                }
            }
        }
    }
    Ok(())
}

/// Assignments of a key to each relation, at most `limit` of them.
fn keys<'a>(choices: &[(&'a str, Vec<Key>)], limit: usize) -> Vec<BTreeMap<&'a str, Key>> {
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

/// Whether a component recurses only between constant keys, and never back to one (O111): each relation has a
/// column that is a constant in every rule's head and in every atom of the component read, and the graph from a head's
/// `(relation, constant)` to its atoms' is acyclic. Every derivation is then at most as deep as that graph, and makes
/// finitely many values: `era001`'s `era_re` builds its statement grammar from `N` up to `STMT` this way.
fn constant_keyed(rules: &[&Rule], component: &BTreeSet<String>) -> bool {
    let key = |t: &Term| match t {
        Term::Str(s) => Some(format!("s{s}")),
        Term::Int(n) => Some(format!("i{n}")),
        _ => None,
    };
    let arity = |rel: &str| rules.iter().find(|r| r.head.rel == rel).map_or(0, |r| r.head.args.len());
    // Each relation's key column: the first that every head and every read in the component holds a constant at.
    let mut columns = BTreeMap::<&str, usize>::new();
    for rel in component {
        let reads = rules.iter().flat_map(|r| {
            r.body.iter().filter_map(|l| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } if a.rel == *rel => Some(a),
                _ => None,
            })
        });
        let heads = rules.iter().filter(|r| r.head.rel == *rel).map(|r| &r.head);
        let atoms: Vec<_> = heads.chain(reads).collect();
        let Some(c) = (0..arity(rel)).find(|&c| atoms.iter().all(|a| a.args.get(c).and_then(key).is_some())) else { return false };
        columns.insert(rel.as_str(), c);
    }
    let node = |a: &super::Atom| key(&a.args[columns[a.rel.as_str()]]).map(|k| (a.rel.clone(), k));
    let mut graph = BTreeMap::<(String, String), BTreeSet<(String, String)>>::new();
    for rule in rules {
        let Some(head) = node(&rule.head) else { return false };
        let to = graph.entry(head).or_default();
        for l in &rule.body {
            if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l
                && component.contains(&a.rel)
            {
                let Some(n) = node(a) else { return false };
                to.insert(n);
            }
        }
    }
    // Acyclic: repeatedly remove the keys that reach no other key.
    let mut left = graph;
    loop {
        let sinks: Vec<_> = left.iter().filter(|(_, to)| to.iter().all(|t| !left.contains_key(t))).map(|(k, _)| k.clone()).collect();
        if sinks.is_empty() {
            return left.is_empty();
        }
        for k in sinks {
            left.remove(&k);
        }
    }
}

/// The components with a value cycle, each bounded by the tree; fails on one that is not
/// (`Error::UnboundedValueCycle`).
pub(in crate::code_rule_dsl) fn check(prog: &Program) -> Result<Vec<BTreeSet<String>>, Error> {
    // A program that does not stratify fails there, by name.
    let Ok((sccs, edges_by)) = super::strata(prog) else { return Ok(Vec::new()) };
    let mut treed = Vec::new();
    let mut moves = base();
    let mut depths = depth_base();
    let mut by_head = BTreeMap::<&str, Vec<&Rule>>::new();
    for rule in &prog.rules {
        by_head.entry(rule.head.rel.as_str()).or_default().push(rule);
    }
    for component in &sccs {
        let recursive = component.len() > 1 || component.iter().any(|r| edges_by.get(r).is_some_and(|e| e.iter().any(|(w, _)| w == r)));
        // The component's guarantees, for its own rules and the components above it.
        summaries(prog, component, &by_head, &mut moves);
        depth_summaries(prog, component, &by_head, &mut depths);
        if !recursive {
            continue;
        }
        let rules: Vec<&Rule> = component.iter().flat_map(|rel| by_head.get(rel.as_str()).into_iter().flatten().copied()).collect();
        if !value_cycle(prog, &rules, component) {
            continue;
        }
        // A recursion through constants only (`era_re("EXPR", …) :- era_re("OPD", …)`): finitely many rules, so
        // finitely many values when the constants do not come back.
        if constant_keyed(&rules, component) {
            treed.push(component.clone());
            continue;
        }
        // The relations with no node column: functions between the others' rows, or keyed by a count.
        let keyless: BTreeSet<&str> = component.iter().map(String::as_str).filter(|rel| node_columns(prog, rel).is_empty()).collect();
        let functions: BTreeMap<&str, BTreeSet<usize>> = keyless
            .iter()
            .filter_map(|&rel| function(prog, by_head.get(rel).map_or(&[][..], Vec::as_slice), component, &keyless).map(|inputs| (rel, inputs)))
            .collect();
        let choices: Vec<(&str, Vec<Key>)> = component
            .iter()
            .filter(|rel| !functions.contains_key(rel.as_str()))
            .map(|rel| {
                let cols = prog.decls.get(rel).map_or(&[][..], Vec::as_slice);
                let number = |t: &str| t == "int" || t == "number";
                let counts: Vec<usize> = cols.iter().enumerate().filter(|(_, (_, t))| number(t)).map(|(i, _)| i).collect();
                let mut keys: Vec<Key> = Vec::new();
                for (i, (_, t)) in cols.iter().enumerate().filter(|(_, (_, t))| t == "node" || number(t) || t == "sym") {
                    keys.push((i, None));
                    if t == "node" {
                        keys.extend(counts.iter().map(|&c| (i, Some(c))));
                    }
                }
                (rel.as_str(), keys)
            })
            .collect();
        let mut first = None;
        let ok = !choices.is_empty()
            && choices.iter().all(|(_, columns)| !columns.is_empty())
            && ways().into_iter().chain([Move::Longer, Move::Shallower, Move::Deeper]).any(|way| {
                keys(&choices, 4096).iter().any(|key| match bounded(&rules, component, key, &functions, way, &moves, &depths) {
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
