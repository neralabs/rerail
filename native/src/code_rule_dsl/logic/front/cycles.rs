//! O11 (option b): a recursion that creates values is bounded when it moves along the data.
//!
//! §12 rejects "cycles through value-creating builtins with no bound". A value cycle is one in the flow of
//! values between columns: a value a builtin made (`@cat` and the others, in a head or bound by `$v = @f(…)`)
//! reaching that builtin's arguments again. A component is accepted exactly when each of its value cycles is
//! bounded by one of these rules; any other is `Error::UnboundedValueCycle`.
//!
//! 1. A term makes no new value (`Bounds::makes`) when
//!    - it is a part of its first argument, which makes none (`shrinks`: `substr`, `@slice`, `@dir`, `@head`,
//!      `@pkg_up`, a regex group `@rx_group`, `@replace_all(s, re, "")`): parts of finitely many values are
//!      finitely many (decision 0043's `pkg_above` and `loads`, `gi_path`'s directories);
//!    - each variable it reads is bound outside the component, by a positive atom of a lower stratum, `nearest`,
//!      `ast` or an aggregate over a lower stratum: it is a function of finitely many fixed values
//!      (`same_upto(x, y, i + 1) :- …, same_child(x, i, c)`);
//!    - it is a counter `x + c` (or `x - c`), c a positive integer, and its rule bounds x or the result above
//!      (below) by an integer or a variable bound outside the component (`py_minor(m + 1) :- py_minor(m),
//!      m < 20`): a counter moving one way below a fixed bound reaches finitely many values.
//!
//!    A component in which no other made value comes back to the term that made it has no value cycle.
//! 2. Otherwise each relation of the component has a key column, and every rule moves from its head's key to
//!    each recursive atom's key one way along an order with finitely many values and no cycle, strictly or not
//!    at all, and every cycle of moves that are not strict is through rules that make no value (`creates`;
//!    `pct_rest(v, rest, i + 1) :- pct_spec(v, i, …, rest)` keeps the text and counts, `pct_spec` cuts the
//!    text). The orders are
//!    - nodes in document order (`Later` or `Earlier`, a node's id is its preorder index): `child`, `field` and
//!      `tape` go down, `inside_<set>` and `nearest` up, `next_sibling` and `precedes` along, and so do two
//!      children of one parent by index (`child(p, a, i), child(p, b, j), i < j`);
//!    - strings by length (`Longer` only): the head's key a part of the atom's (`shrinks`), strictly when the
//!      regex around the group matches at least one character (`gap`: `pct_rest`'s `%%`);
//!    - either, with a count beside it (an integer column), in order of the two: a key moves strictly when its
//!      column does, or when the column stays and the count goes up by a positive integer from a value bound
//!      outside the component (`tpl_upto(t, i + 1, …) :- tpl_upto(t, i, …), child(t, c, i)`), or to one (a
//!      node's parts joined in child order, decision 0043: `j(c, i + 1, …) :- j(c, i, …), child(c, p, i + 1)`); a
//!      relation with no count is above every count (`const_str(n, v) :- tpl_upto(n, i, v)`).
//!
//!    A non-strict move between unequal keys (`e != n`) is strict. Along the rows a made value passes through,
//!    keys never move back, and a rule making a value is used again only after a strict move; keys are finitely
//!    many (the tree's nodes, the parts of the strings that entered), so a value is made from other made values
//!    at most finitely many times over, and only finitely many values are made.
//!
//! The moves between columns are the base relations', and what a derived relation guarantees between its
//! columns in every row (`summarize`): the greatest set of moves each of its rules gives when its recursive
//! atoms are assumed to give them, which holds of every row by induction on its derivation (`unparen(e, m)`:
//! m is e or below it; `attr_of(n, o, a)`: o is below n). Nothing else bounds a cycle: a relation over a graph
//! (imports, calls) may itself be cyclic, and an unguarded counter (`n(x + 1) :- n(x)`) never stops. Decided
//! 2026-11-03 (open-items O11); the counters, sibling order, strings and recursive summaries 2026-11-06 (O77).

use super::{AXIS, Error, Literal, Program, Rule, Term};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// A move from one value to another along an order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Move {
    /// To a node after it in document order (below it, or a later sibling's).
    Later,
    Earlier,
    /// To a string it is a part of.
    Longer,
    Shorter,
}

impl Move {
    fn back(self) -> Self {
        match self {
            Move::Later => Move::Earlier,
            Move::Earlier => Move::Later,
            Move::Longer => Move::Shorter,
            Move::Shorter => Move::Longer,
        }
    }

    /// The type of the values the move is between.
    fn ty(self) -> &'static str {
        match self {
            Move::Later | Move::Earlier => "node",
            Move::Longer | Move::Shorter => "sym",
        }
    }
}

/// Per relation, the moves between its columns that hold in every row: `(from, to) → (move → strictly)`.
type Moves = BTreeMap<String, BTreeMap<(usize, usize), BTreeMap<Move, bool>>>;

/// The base relations' moves (`edges` adds each one's way back).
fn base(prog: &Program) -> Moves {
    let mut moves = Moves::new();
    let mut add = |rel: &str, from: usize, to: usize| {
        moves.entry(rel.to_owned()).or_default().entry((from, to)).or_default().insert(Move::Later, true);
    };
    for (rel, from, to) in [("child", 0, 1), ("field", 0, 2), ("tape", 1, 0), ("next_sibling", 0, 1), ("precedes", 0, 1)] {
        add(rel, from, to);
    }
    // `inside_<set>(n, a)`: a is an ancestor of n.
    for rel in prog.inputs.iter().filter(|r| r.starts_with(AXIS)) {
        add(rel, 1, 0);
    }
    moves
}

/// A rule's variables' edges: `var → [(var, Some((move, strictly)) or None for the same value)]`. The head's
/// column j is the variable `head(j)`.
type Edges = BTreeMap<String, Vec<(String, Option<(Move, bool)>)>>;

fn head(j: usize) -> String {
    format!("#{j}")
}

/// A term as a variable's value or a part of it: `(variable, None)` for the variable itself.
fn part(t: &Term) -> Option<(&str, Option<(Move, bool)>)> {
    match t {
        Term::Var(x) => Some((x, None)),
        Term::Call(_, args) => {
            let strict = shrinks(t)?;
            let Some(Term::Var(x)) = args.first() else { return None };
            Some((x, Some((Move::Shorter, strict))))
        }
        _ => None,
    }
}

fn edges(rule: &Rule, moves: &Moves) -> Edges {
    let mut out = Edges::new();
    let mut add = |a: &str, b: &str, m: Option<(Move, bool)>| {
        out.entry(a.to_owned()).or_default().push((b.to_owned(), m));
        out.entry(b.to_owned()).or_default().push((a.to_owned(), m.map(|(m, s)| (m.back(), s))));
    };
    // A term and the variable it is (a part of): `part` gives the way from the variable to the term.
    for (j, t) in rule.head.args.iter().enumerate() {
        if let Some((x, m)) = part(t) {
            add(x, &head(j), m);
        }
    }
    let mut children = Vec::new();
    for literal in &rule.body {
        match literal {
            Literal::Atom(a) if !a.neg => {
                for (&(i, j), ms) in moves.get(&a.rel).into_iter().flatten() {
                    if let (Some(Term::Var(x)), Some(Term::Var(y))) = (a.args.get(i), a.args.get(j)) {
                        ms.iter().for_each(|(&m, &s)| add(x, y, Some((m, s))));
                    }
                }
                if a.rel == "child"
                    && let [Term::Var(p), Term::Var(c), i] = a.args.as_slice()
                {
                    children.push((p, c, i));
                }
            }
            Literal::Nearest { n, a, .. } => add(n, a, Some((Move::Earlier, true))),
            Literal::Cmp { l: Term::Var(v), op: "=", r: t } | Literal::Cmp { l: t, op: "=", r: Term::Var(v) } => {
                if let Some((x, m)) = part(t) {
                    add(x, v, m);
                }
            }
            _ => {}
        }
    }
    // Two children of one parent: the one of the lower index comes first.
    for &(p, a, i) in &children {
        for &(q, b, j) in &children {
            if p == q && a != b && before(rule, i, j) {
                add(a, b, Some((Move::Later, true)));
            }
        }
    }
    out
}

/// Whether the rule gives `i < j` for two child indexes: integers, `i < j`, or `j = i + c` with c positive.
fn before(rule: &Rule, i: &Term, j: &Term) -> bool {
    if let (Term::Int(a), Term::Int(b)) = (i, j) {
        return a < b;
    }
    rule.body.iter().any(|l| match l {
        Literal::Cmp { l, op: "<", r } => l == i && r == j,
        Literal::Cmp { l, op: ">", r } => l == j && r == i,
        Literal::Cmp { l, op: "=", r: Term::Call(f, args) } if f == "add" && l == j => {
            matches!(args.as_slice(), [x, Term::Int(c)] | [Term::Int(c), x] if x == i && *c > 0)
        }
        _ => false,
    })
}

/// How `to` lies from `from` along `way`: `Some(true)` strictly, `Some(false)` at most the same value, `None`
/// not at all.
fn reach(edges: &Edges, from: &str, to: &str, way: Move) -> Option<bool> {
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

/// `reach`, with a move to an unequal value (`$e != $n` in the rule) strict.
fn step(rule: &Rule, edges: &Edges, from: &str, to: &str, way: Move) -> Option<bool> {
    let got = reach(edges, from, to, way)?;
    let var = |t: &Term, v: &str| matches!(t, Term::Var(x) if x == v);
    // The head's column `#j` is its variable there (a part of a variable is tested by nothing).
    let named = |v: &str| -> Option<String> {
        match v.strip_prefix('#').and_then(|j| j.parse::<usize>().ok()).and_then(|j| rule.head.args.get(j)) {
            Some(Term::Var(x)) => Some(x.clone()),
            Some(_) => None,
            None => Some(v.to_owned()),
        }
    };
    let unequal = named(from).zip(named(to)).is_some_and(|(f, t)| {
        rule.body.iter().any(|l| matches!(l, Literal::Cmp { l, op: "!=", r } if (var(l, &f) && var(r, &t)) || (var(l, &t) && var(r, &f))))
    });
    Some(got || unequal)
}

/// Whether a term is a part of its first argument, and whether always a strictly shorter one: `Some(strictly)`.
/// `@replace_all(s, re, "")` deletes from s; its values are parts of s too (as a subsequence).
fn shrinks(t: &Term) -> Option<bool> {
    let Term::Call(f, args) = t else { return None };
    match (f.as_str(), args.as_slice()) {
        ("rx_group", [_, Term::Str(re), Term::Str(g)]) => Some(gap(re, g).is_some_and(|n| n > 0)),
        ("rx_group", [_, _, _]) | ("pkg_up" | "substr" | "slice" | "dir" | "head", [_, ..]) => Some(false),
        ("replace_all", [_, _, Term::Str(r)]) if r.is_empty() => Some(false),
        _ => None,
    }
}

/// The fewest bytes a match of `re` has outside its group `g` when g takes part (its capture is that much
/// shorter than the text matched); `None` when `re` does not parse or has no group g.
pub(super) fn gap(re: &str, g: &str) -> Option<usize> {
    use regex_syntax::hir::{Hir, HirKind};
    fn min(h: &Hir) -> usize {
        h.properties().minimum_len().unwrap_or(0)
    }
    fn walk(h: &Hir, g: &str) -> Option<usize> {
        match h.kind() {
            HirKind::Capture(c) if c.name.as_deref() == Some(g) => Some(0),
            HirKind::Capture(c) => walk(&c.sub, g),
            HirKind::Concat(subs) => {
                let (k, inner) = subs.iter().enumerate().find_map(|(k, s)| walk(s, g).map(|n| (k, n)))?;
                Some(inner + subs.iter().enumerate().filter(|(i, _)| *i != k).map(|(_, s)| min(s)).sum::<usize>())
            }
            // A branch without g gives no capture.
            HirKind::Alternation(subs) => subs.iter().filter_map(|s| walk(s, g)).min(),
            // g is captured in the last of at least one round; the others match in full.
            HirKind::Repetition(r) => walk(&r.sub, g).map(|n| n + (r.min as usize).saturating_sub(1) * min(&r.sub)),
            _ => None,
        }
    }
    walk(&regex_syntax::parse(re).ok()?, g)
}

/// What bounds a rule's made values (rule 1 of the module's).
struct Bounds<'a> {
    rule: &'a Rule,
    /// The variables bound outside the component: finitely many values each, fixed while it runs.
    outside: BTreeSet<String>,
}

impl<'a> Bounds<'a> {
    fn new(rule: &'a Rule, component: &BTreeSet<String>) -> Self {
        let mut outside = BTreeSet::new();
        for l in &rule.body {
            match l {
                Literal::Atom(a) if !a.neg && !component.contains(&a.rel) => outside.extend(a.vars()),
                Literal::Agg { var, atom, .. } if !component.contains(&atom.rel) => {
                    outside.insert(var.clone());
                }
                Literal::Nearest { .. } | Literal::Ast { .. } => outside.extend(l.vars()),
                _ => {}
            }
        }
        Bounds { rule, outside }
    }

    /// Whether `t` makes a new value; `alias` is the variable it is assigned to, if any.
    fn makes(&self, t: &Term, alias: Option<&str>) -> bool {
        let Term::Call(_, args) = t else { return false };
        if shrinks(t).is_some() {
            return args.first().is_some_and(|a| self.makes(a, None));
        }
        let mut vars = BTreeSet::new();
        t.vars(&mut vars);
        !vars.is_subset(&self.outside) && !self.counter(t, alias)
    }

    /// `x + c` or `x - c` (c a positive integer) with x or `alias` bounded the way it moves.
    fn counter(&self, t: &Term, alias: Option<&str>) -> bool {
        let (x, by) = match t {
            Term::Call(f, args) if f == "add" => match args.as_slice() {
                [Term::Var(x), Term::Int(c)] | [Term::Int(c), Term::Var(x)] => (x, *c),
                _ => return false,
            },
            Term::Call(f, args) if f == "sub" => match args.as_slice() {
                [Term::Var(x), Term::Int(c)] => match c.checked_neg() {
                    Some(by) => (x, by),
                    None => return false,
                },
                _ => return false,
            },
            _ => return false,
        };
        let bound = |k: &Term| matches!(k, Term::Int(_)) || matches!(k, Term::Var(v) if self.outside.contains(v));
        let this = |t: &Term| matches!(t, Term::Var(v) if v == x || Some(v.as_str()) == alias);
        let (below, above): (&[&str], &[&str]) = if by > 0 { (&["<", "<="], &[">", ">="]) } else { (&[">", ">="], &["<", "<="]) };
        by == 0
            || self.rule.body.iter().any(|l| match l {
                Literal::Cmp { l, op, r } => (this(l) && below.contains(op) && bound(r)) || (this(r) && above.contains(op) && bound(l)),
                _ => false,
            })
    }

    /// The variables bound by positive atoms (any relation's).
    fn atoms(&self) -> BTreeSet<String> {
        self.rule.body.iter().filter_map(|l| l.atom().filter(|a| !a.neg)).flat_map(|a| a.vars()).collect()
    }

    /// The assignments `$v = t` (either way round), in order.
    fn assignments(&self) -> impl Iterator<Item = (&'a str, &'a Term)> {
        self.rule.body.iter().filter_map(|l| match l {
            Literal::Cmp { l: Term::Var(v), op: "=", r } => Some((v.as_str(), r)),
            Literal::Cmp { l, op: "=", r: Term::Var(v) } if !matches!(l, Term::Var(_)) => Some((v.as_str(), l)),
            _ => None,
        })
    }
}

/// Whether a rule makes a value into its head: a head term that makes one, or reads a variable assigned one.
fn creates(rule: &Rule, component: &BTreeSet<String>) -> bool {
    let b = Bounds::new(rule, component);
    let bound = b.atoms();
    let mut made = BTreeSet::<&str>::new();
    let reads_made = |t: &Term, made: &BTreeSet<&str>| {
        let mut vars = BTreeSet::new();
        t.vars(&mut vars);
        vars.iter().any(|v| made.contains(v.as_str()))
    };
    loop {
        let before = made.len();
        for (v, t) in b.assignments() {
            if !bound.contains(v) && (b.makes(t, Some(v)) || reads_made(t, &made)) {
                made.insert(v);
            }
            // A split of a made value (`m2 = @cat(p, x)`, m2 made): a part may be all of it (`"" . m2`).
            if made.contains(v) && t.is_cat() && let Term::Call(_, parts) = t {
                made.extend(parts.iter().filter_map(|p| if let Term::Var(p) = p { Some(p.as_str()) } else { None }).filter(|p| !bound.contains(*p)));
            }
        }
        if made.len() == before {
            break;
        }
    }
    rule.head.args.iter().any(|t| b.makes(t, None) || reads_made(t, &made))
}

/// A column of a relation.
type Column = (String, usize);

/// The flow of values in `rules` between the columns of `component`: `(from, to, made)`, where `made` is a
/// builtin's new value from its arguments.
fn flow(rules: &[&Rule], component: &BTreeSet<String>) -> Vec<(Column, Column, bool)> {
    let mut out = Vec::new();
    for rule in rules {
        let b = Bounds::new(rule, component);
        // Where each variable's values come from: the component's columns it is read from, and through which
        // assignments (`$v = term`, made when the term makes a value).
        let mut from = BTreeMap::<String, BTreeSet<(Column, bool)>>::new();
        let bound = b.atoms();
        for literal in &rule.body {
            if let Some(a) = literal.atom().filter(|a| !a.neg && component.contains(&a.rel)) {
                for (i, t) in a.args.iter().enumerate() {
                    if let Term::Var(v) = t {
                        from.entry(v.clone()).or_default().insert(((a.rel.clone(), i), false));
                    }
                }
            }
        }
        let sources = |t: &Term, alias: Option<&str>, from: &BTreeMap<String, BTreeSet<(Column, bool)>>| -> BTreeSet<(Column, bool)> {
            let mut vars = BTreeSet::new();
            t.vars(&mut vars);
            let made = b.makes(t, alias);
            vars.iter().flat_map(|v| from.get(v).into_iter().flatten().map(move |(c, m)| (c.clone(), *m || made))).collect()
        };
        // A variable no atom binds and no assignment gives is a part of a bound value it is split from
        // (`$m = @cat($q, $x)` with `$m` bound).
        let assigned: BTreeSet<&str> = b.assignments().map(|(v, _)| v).filter(|v| !bound.contains(*v)).collect();
        // Assignments in order, until nothing more flows (an assignment may read another's variable).
        loop {
            let before = from.values().map(BTreeSet::len).sum::<usize>();
            for (v, t) in b.assignments() {
                if !bound.contains(v) {
                    let s = sources(t, Some(v), &from);
                    from.entry(v.to_owned()).or_default().extend(s);
                }
                let mut parts = BTreeSet::new();
                t.vars(&mut parts);
                let whole = from.get(v).cloned().unwrap_or_default();
                for p in parts.iter().filter(|p| !bound.contains(p.as_str()) && !assigned.contains(p.as_str())) {
                    from.entry(p.clone()).or_default().extend(whole.iter().cloned());
                }
            }
            if from.values().map(BTreeSet::len).sum::<usize>() == before {
                break;
            }
        }
        for (j, t) in rule.head.args.iter().enumerate() {
            for (c, made) in sources(t, None, &from) {
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

/// The columns of `rel` of type `ty`, as declared.
fn columns(prog: &Program, rel: &str, ty: &str) -> Vec<usize> {
    prog.decls.get(rel).map_or_else(Vec::new, |cols| cols.iter().enumerate().filter(|(_, (_, t))| t == ty).map(|(i, _)| i).collect())
}

/// What the relations of `component` guarantee between their columns in every row, added to `moves`: the
/// greatest set of moves each rule gives, its recursive atoms assumed to give them (an input, or a relation
/// with facts, guarantees nothing).
fn summarize(prog: &Program, component: &BTreeSet<String>, by_head: &BTreeMap<&str, Vec<&Rule>>, moves: &mut Moves) {
    let (rels, none): (Vec<&String>, Vec<&String>) = component.iter().partition(|r| !prog.inputs.contains(r) && !prog.facts.contains_key(*r) && by_head.contains_key(r.as_str()));
    for rel in none {
        moves.remove(rel);
    }
    // At first every move between columns of a type, strictly; then what the rules do not give is taken out, until
    // they give all that is left.
    let mut assumed = BTreeMap::<String, BTreeMap<(usize, usize), BTreeMap<Move, bool>>>::new();
    for rel in &rels {
        let entry = assumed.entry((*rel).clone()).or_default();
        for way in [Move::Later, Move::Earlier, Move::Longer, Move::Shorter] {
            let cols = columns(prog, rel, way.ty());
            for &i in &cols {
                for &j in cols.iter().filter(|&&j| j != i) {
                    entry.entry((i, j)).or_default().insert(way, true);
                }
            }
        }
    }
    loop {
        for (rel, held) in &assumed {
            moves.insert(rel.clone(), held.clone());
        }
        let mut next = assumed.clone();
        for rel in &rels {
            let rules = &by_head[rel.as_str()];
            let all: Vec<Edges> = rules.iter().map(|r| edges(r, moves)).collect();
            let held = next.get_mut(rel.as_str()).expect("assumed");
            for (&(i, j), ways) in held.iter_mut() {
                ways.retain(|&way, strict| match rules.iter().zip(&all).map(|(r, e)| step(r, e, &head(i), &head(j), way)).collect::<Option<Vec<bool>>>() {
                    Some(got) => {
                        *strict = *strict && got.iter().all(|&s| s);
                        true
                    }
                    None => false,
                });
            }
            held.retain(|_, ways| !ways.is_empty());
        }
        if next == assumed {
            break;
        }
        assumed = next;
    }
    for (rel, held) in assumed {
        if held.is_empty() {
            moves.remove(&rel);
        } else {
            moves.insert(rel, held);
        }
    }
}

/// A relation's key: a column along the way, and maybe a count beside it (an integer column).
type Key = (usize, Option<usize>);

/// How a rule's count moves from a recursive atom's (`body`, None for a relation with none) to its head's
/// (column `head`): `Some(true)` up by a positive integer from a value bound outside the component, or to one (a
/// child's index: `j(c, i + 1, …) :- j(c, i, …), child(c, p, i + 1)`), so through finitely many values;
/// `Some(false)` the same; a relation with no count is above every count.
fn count_step(rule: &Rule, outside: &BTreeSet<String>, head: Option<usize>, body: Option<&Term>) -> Option<bool> {
    let (j, b) = match (head, body) {
        (None, None) => return Some(false),
        (None, Some(_)) => return Some(true),
        (Some(j), Some(Term::Var(b))) => (j, b),
        _ => return None,
    };
    let up = |t: &Term| matches!(t, Term::Call(f, args) if f == "add" && matches!(args.as_slice(), [Term::Var(x), Term::Int(c)] | [Term::Int(c), Term::Var(x)] if x == b && *c > 0));
    // `t` is the value of a variable bound outside the component (`$k = t`, either way round).
    let outside_value = |t: &Term| {
        rule.body.iter().any(|l| matches!(l, Literal::Cmp { l: Term::Var(v), op: "=", r } | Literal::Cmp { l: r, op: "=", r: Term::Var(v) } if outside.contains(v) && r == t))
    };
    match rule.head.args.get(j)? {
        Term::Var(x) if x == b => Some(false),
        Term::Var(x) => rule.body.iter().find_map(|l| match l {
            Literal::Cmp { l: Term::Var(v), op: "=", r: Term::Var(y) } | Literal::Cmp { l: Term::Var(y), op: "=", r: Term::Var(v) } if v == x && y == b => Some(false),
            Literal::Cmp { l: Term::Var(v), op: "=", r } if v == x && up(r) && (outside.contains(b) || outside.contains(x)) => Some(true),
            _ => None,
        }),
        t if up(t) && (outside.contains(b) || outside_value(t)) => Some(true),
        _ => None,
    }
}

/// Whether, along `way`, keys `key` (one per relation of `component`) bound every recursive rule: each moves
/// from its head's key to each recursive atom's key, strictly or not, and no cycle of moves that are not strict
/// goes through a rule that makes a value (`creating`, per rule). With a count, a key moves strictly when its
/// column does, or stays and its count goes up (`tpl_upto(t, i + 1, …) :- tpl_upto(t, i, …), child(t, c, i)`):
/// one column's values by the other's, finitely many either way.
fn bounded(rules: &[&Rule], creating: &[bool], component: &BTreeSet<String>, key: &BTreeMap<&str, Key>, way: Move, moves: &Moves) -> Result<(), String> {
    // The moves that are not strict, head → atom, and those of rules making a value.
    let mut same = super::Edges::new();
    let mut made = Vec::new();
    for (rule, &creates) in rules.iter().zip(creating) {
        let (k, count) = key[rule.head.rel.as_str()];
        let h = head(k);
        let edges = edges(rule, moves);
        let outside = Bounds::new(rule, component).outside;
        for literal in &rule.body {
            let Some(a) = literal.atom().filter(|a| component.contains(&a.rel)) else { continue };
            let (ak, acount) = key[a.rel.as_str()];
            let Some(Term::Var(b)) = a.args.get(ak) else { return Err(rule.to_string()) };
            let moved = match step(rule, &edges, &h, b, way) {
                Some(false) => count_step(rule, &outside, count, acount.and_then(|c| a.args.get(c)).or(acount.map(|_| &Term::Wild))),
                other => other,
            };
            match moved {
                Some(true) => {}
                Some(false) => {
                    same.entry(rule.head.rel.clone()).or_default().insert((a.rel.clone(), false));
                    if creates {
                        made.push((&rule.head.rel, &a.rel, rule));
                    }
                }
                None => return Err(rule.to_string()),
            }
        }
    }
    let cycles = super::tarjan(component, &same);
    match made.into_iter().find(|(h, a, _)| cycles.iter().any(|c| c.contains(*h) && c.contains(*a))) {
        Some((_, _, rule)) => Err(rule.to_string()),
        None => Ok(()),
    }
}

/// Assignments of a key to each relation, at most `limit` of them.
fn keys<'a, T: Copy>(choices: &[(&'a str, Vec<T>)], limit: usize) -> Vec<BTreeMap<&'a str, T>> {
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

/// The components with a value cycle, each bounded by its keys; fails on one that is not
/// (`Error::UnboundedValueCycle`).
pub(in crate::code_rule_dsl) fn check(prog: &Program) -> Result<Vec<BTreeSet<String>>, Error> {
    // A program that does not stratify fails there, by name.
    let Ok((sccs, edges_by)) = super::strata(prog) else { return Ok(Vec::new()) };
    let mut treed = Vec::new();
    let mut moves = base(prog);
    let mut by_head = BTreeMap::<&str, Vec<&Rule>>::new();
    for rule in &prog.rules {
        by_head.entry(rule.head.rel.as_str()).or_default().push(rule);
    }
    for component in &sccs {
        // A component's guarantees, for its own rules and the components above it.
        summarize(prog, component, &by_head, &mut moves);
        let recursive = component.len() > 1 || component.iter().any(|r| edges_by.get(r).is_some_and(|e| e.iter().any(|(w, _)| w == r)));
        if !recursive {
            continue;
        }
        let rules: Vec<&Rule> = component.iter().flat_map(|rel| by_head.get(rel.as_str()).into_iter().flatten().copied()).collect();
        if !value_cycle(prog, &rules, component) {
            continue;
        }
        let creating: Vec<bool> = rules.iter().map(|r| creates(r, component)).collect();
        let mut first = None;
        // Strings get only shorter: a longer one is no bound. Keys without counts first, then with them.
        let ok = [false, true].into_iter().any(|counted| {
            [Move::Later, Move::Earlier, Move::Longer].into_iter().any(|way| {
                let choices: Vec<(&str, Vec<Key>)> = component
                    .iter()
                    .map(|rel| {
                        let counts: Vec<Option<usize>> = if counted { columns(prog, rel, "int").into_iter().map(Some).chain([None]).collect() } else { vec![None] };
                        (rel.as_str(), columns(prog, rel, way.ty()).into_iter().flat_map(|c| counts.iter().map(move |&n| (c, n))).collect())
                    })
                    .collect();
                choices.iter().all(|(_, columns)| !columns.is_empty())
                    && (!counted || choices.iter().any(|(_, ks)| ks.iter().any(|(_, n)| n.is_some())))
                    && keys(&choices, 4096).iter().any(|key| match bounded(&rules, &creating, component, key, way, &moves) {
                        Ok(()) => true,
                        Err(rule) => {
                            first.get_or_insert(rule);
                            false
                        }
                    })
            })
        });
        if !ok {
            let rule = first.unwrap_or_else(|| rules.iter().zip(&creating).find(|(_, c)| **c).map(|(r, _)| r.to_string()).unwrap_or_default());
            return Err(Error::UnboundedValueCycle { component: component.clone(), rule });
        }
        treed.push(component.clone());
    }
    Ok(treed)
}
