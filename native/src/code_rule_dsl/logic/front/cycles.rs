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
//!    A variable bound outside the component takes finitely many values, whatever reaches it, so nothing flows
//!    through it. A component in which no other made value comes back to the term that made it has no value cycle,
//!    nor does a head term a variable bound outside the component is assigned (`era_run`).
//! 2. A component that recurses only between constant keys that never come back (`constant_keyed`: `era_re`'s
//!    grammar from `N` up to `STMT`) derives at most as deep as their graph.
//! 3. Otherwise each relation of the component has a key column, and every rule moves from its head's key to
//!    each recursive atom's key one way along an order with finitely many values and no cycle, strictly or not
//!    at all, and every cycle of moves that are not strict is through rules that make no value (`creates`;
//!    `pct_rest(v, rest, i + 1) :- pct_spec(v, i, …, rest)` keeps the text and counts, `pct_spec` cuts the
//!    text). The orders are
//!    - nodes in document order (`Later` or `Earlier`, a node's id is its preorder index): `child`, `field` and
//!      `tape` go down, the tree axes (`rr_inside_<set>`, `rr_has_<set>`) and `nearest` up, `next_sibling` and
//!      `precedes` along, and so do two children of one parent by index (`child(p, a, i), child(p, b, j), i <
//!      j`) and two variables compared (`$a < $b`; `<=` not strictly, `>` and `>=` the other way; decision 0046);
//!    - nodes by depth (`Shallower` or `Deeper`, strictly only: `Depths`), which bounds what document order cannot
//!      (`ast_parent`'s decorator edge, a later node one level up);
//!    - strings by length (`Longer` only): the head's key a part of the atom's (`shrinks`), strictly when the
//!      regex around the group matches at least one character (`gap`: `pct_rest`'s `%%`);
//!    - either, with a count beside it (an integer column), in order of the two: a key moves strictly when its
//!      column does, or when the column stays and the count goes up by a positive integer from a value bound
//!      outside the component (`tpl_upto(t, i + 1, …) :- tpl_upto(t, i, …), child(t, c, i)`), or to one (a
//!      node's parts joined in child order, decision 0043: `j(c, i + 1, …) :- j(c, i, …), child(c, p, i + 1)`),
//!      or under a bound as a counter is (rule 1), or to a value bound outside the component that a comparison
//!      puts above it; a relation with no count is above every count (`const_str(n, v) :- tpl_upto(n, i, v)`);
//!    - a count alone, when every relation of the component has one: the place never moves, and the count goes
//!      up as beside a key.
//!
//!    A relation of the component with no node column needs no key when it is a function (`function`:
//!    `negate(a, b)` between `number`'s rows): no rule of it reads such a relation of the component, and each
//!    column that is not an input (a column every rule binds by a positive atom, or a constant) is made from its
//!    inputs and values bound outside the component. A rule reading a function then makes a value, and binds
//!    the function's inputs outside the component or by an atom strictly along the key.
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
//! (imports, calls) may itself be cyclic, and an unguarded counter (`n(x + 1) :- n(x)`) never stops; depth's
//! guarantees likewise (`depth_summaries`). Decided
//! 2026-11-03 (open-items O11); the counters, sibling order, strings and recursive summaries 2026-11-06 (O77);
//! comparisons, functions and counts alone with decision 0046.

use super::{Error, LatticeDecl, is_axis};
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::logic::program_image::handles::{AtomRef, CmpOp, Lit, LitRef, RuleRef, TermRef};
use super::graph;
use crate::code_rule_dsl::logic::program_image::{About, AtomId, Slot, Version};
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
    /// To a node nearer the root, and further from it (`Depths`).
    Shallower,
    Deeper,
    /// To a greater and a smaller value, by `<` between two variables, where that is not document order: under node
    /// ids per definition (`RERAIL_NODE_IDS=definition`, decision 0094 §2) a node's id follows preorder only inside
    /// its granule, so `$a < $b` and a tree edge are moves in two orders, and a cycle may not mix them.
    Greater,
    Smaller,
}

impl Move {
    fn back(self) -> Self {
        match self {
            Move::Later => Move::Earlier,
            Move::Earlier => Move::Later,
            Move::Longer => Move::Shorter,
            Move::Shorter => Move::Longer,
            Move::Shallower => Move::Deeper,
            Move::Deeper => Move::Shallower,
            Move::Greater => Move::Smaller,
            Move::Smaller => Move::Greater,
        }
    }

    /// The type of the values the move is between.
    fn ty(self) -> &'static str {
        match self {
            Move::Later | Move::Earlier | Move::Shallower | Move::Deeper | Move::Greater | Move::Smaller => "node",
            Move::Longer | Move::Shorter => "sym",
        }
    }
}

/// A rule of the version, read in place: its variables, relations and strings by the names the version gives them.
#[derive(Clone, Copy)]
struct Rule<'v> {
    v: &'v Version,
    r: RuleRef<'v>,
}

impl<'v> Rule<'v> {
    fn head_rel(self) -> &'v str {
        self.v.rel_name(self.r.head().rel())
    }
    fn head_args(self) -> Vec<TermRef<'v>> {
        self.r.head().args().collect()
    }
    fn head_arg(self, j: usize) -> Option<TermRef<'v>> {
        self.r.head().args().nth(j)
    }
    fn arity(self) -> usize {
        self.r.head().arity()
    }
    fn body(self) -> impl Iterator<Item = LitRef<'v>> {
        self.r.body()
    }
    fn lits(self) -> impl Iterator<Item = Lit<'v>> {
        self.r.body().map(LitRef::view)
    }
    fn rel(self, a: AtomRef<'v>) -> &'v str {
        self.v.rel_name(a.rel())
    }
    fn text(self, a: AtomId) -> &'v str {
        self.v.text(a)
    }
    fn name(self, s: Slot) -> &'v str {
        self.v.text(self.r.var(s))
    }
    /// The variable a term is, by name.
    fn var(self, t: TermRef<'v>) -> Option<&'v str> {
        t.var().map(|s| self.name(s))
    }
    fn is_var(self, t: TermRef<'v>, x: &str) -> bool {
        self.var(t) == Some(x)
    }
    /// A call's builtin and its arguments.
    fn call(self, t: TermRef<'v>) -> Option<(&'v str, Vec<TermRef<'v>>)> {
        if let TermRef::Call(f, args) = t { Some((self.text(f), args.collect())) } else { None }
    }
    /// The variables a term reads, a call's arguments' too.
    fn vars(self, t: TermRef<'v>, out: &mut BTreeSet<String>) {
        match t {
            TermRef::Var(s) => {
                out.insert(self.name(s).to_owned());
            }
            TermRef::Call(_, args) => args.for_each(|a| self.vars(a, out)),
            _ => {}
        }
    }
    fn atom_vars(self, a: AtomRef<'v>) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        a.args().for_each(|t| self.vars(t, &mut out));
        out
    }
    /// The variables a literal names.
    fn lit_vars(self, l: Lit<'v>) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        match l {
            Lit::Atom(a) => out = self.atom_vars(a),
            Lit::Cmp { l, r, .. } => {
                self.vars(l, &mut out);
                self.vars(r, &mut out);
            }
            Lit::Agg { var, over, atom, .. } => {
                out = self.atom_vars(atom);
                out.insert(self.name(var).to_owned());
                out.extend(over.map(|o| self.name(o).to_owned()));
            }
            Lit::Nearest { n, a, .. } => out.extend([self.name(n).to_owned(), self.name(a).to_owned()]),
            Lit::Ast { document, node, captures, .. } => {
                out.extend([self.name(document).to_owned(), self.name(node).to_owned()]);
                out.extend(captures.map(|(_, s)| self.name(s).to_owned()));
            }
        }
        out
    }
    /// Whether two of the rule's terms are the same term.
    fn same(self, a: TermRef<'v>, b: TermRef<'v>) -> bool {
        match (a, b) {
            (TermRef::Var(x), TermRef::Var(y)) => x == y,
            (TermRef::Wild, TermRef::Wild) | (TermRef::Top, TermRef::Top) => true,
            (TermRef::Int(x), TermRef::Int(y)) => x == y,
            (TermRef::Str(x), TermRef::Str(y)) => self.text(x) == self.text(y),
            (TermRef::Branded(x), TermRef::Branded(y)) => x == y,
            (TermRef::Call(f, xs), TermRef::Call(g, ys)) => {
                self.text(f) == self.text(g) && xs.count() == ys.count() && xs.zip(ys).all(|(x, y)| self.same(x, y))
            }
            _ => false,
        }
    }
    /// The rule as its program's text writes it, for an error.
    fn shown(self) -> String {
        self.v.show_rule(self.r)
    }
}

/// What the check reads of the program besides its rules.
struct Prog<'v> {
    v: &'v Version,
    lattice: BTreeMap<String, LatticeDecl>,
}

impl Prog<'_> {
    /// The columns of `rel` of type `ty`, as declared.
    fn columns(&self, rel: &str, ty: &str) -> Vec<usize> {
        self.v.columns_of(rel).map_or_else(Vec::new, |cols| cols.iter().enumerate().filter(|(_, (_, t))| t == ty).map(|(i, _)| i).collect())
    }
    /// Whether `rel`'s rows are given, not derived: an input, or a relation with facts.
    fn given(&self, rel: &str) -> bool {
        self.v.has_input(rel) || ProgramFields::states_facts(self.v, rel)
    }
}

/// Per relation, the moves between its columns that hold in every row: `(from, to) → (move → strictly)`.
type Moves = BTreeMap<String, BTreeMap<(usize, usize), BTreeMap<Move, bool>>>;

/// The base relations' moves (`edges` adds each one's way back).
fn base(p: &Prog<'_>) -> Moves {
    let mut moves = Moves::new();
    let mut add = |rel: &str, from: usize, to: usize| {
        moves.entry(rel.to_owned()).or_default().entry((from, to)).or_default().insert(Move::Later, true);
    };
    for (rel, from, to) in [("child", 0, 1), ("field", 0, 2), ("tape", 1, 0), ("next_sibling", 0, 1), ("precedes", 0, 1)] {
        add(rel, from, to);
    }
    // The tree axes, `rr_inside_<set>(n, a)` and `rr_has_<set>(n, a)`: a is an ancestor of n.
    for rel in p.v.input_names().iter().filter(|r| is_axis(r)) {
        add(rel, 1, 0);
    }
    moves
}

/// Whether `<` between variables is its own order (`Move::Greater`), not document order: node ids per definition.
fn by_value() -> bool {
    crate::code_rule_dsl::facts::vtab::syntax::definition_ids()
}

/// The orders a node key may move along: document order, and under node ids per definition, values'
/// (`Move::Greater`).
fn ways() -> Vec<Move> {
    let mut out = vec![Move::Later, Move::Earlier];
    if by_value() {
        out.extend([Move::Greater, Move::Smaller]);
    }
    out
}

/// A rule's variables' edges: `var → [(var, Some((move, strictly)) or None for the same value)]`. The head's
/// column j is the variable `head(j)`.
type Edges = BTreeMap<String, Vec<(String, Option<(Move, bool)>)>>;

fn head(j: usize) -> String {
    format!("#{j}")
}

/// The variable at an atom's column `i`, if it is one.
fn arg_var<'v>(rule: Rule<'v>, args: &[TermRef<'v>], i: usize) -> Option<&'v str> {
    args.get(i).and_then(|&t| rule.var(t))
}

/// A term as a variable's value or a part of it: `(variable, None)` for the variable itself.
fn part<'v>(rule: Rule<'v>, t: TermRef<'v>) -> Option<(&'v str, Option<(Move, bool)>)> {
    match t {
        TermRef::Var(s) => Some((rule.name(s), None)),
        TermRef::Call(..) => {
            let strict = shrinks(rule, t)?;
            let x = rule.call(t)?.1.first().and_then(|&a| rule.var(a))?;
            Some((x, Some((Move::Shorter, strict))))
        }
        _ => None,
    }
}

fn edges(rule: Rule<'_>, moves: &Moves) -> Edges {
    let mut out = Edges::new();
    let mut add = |a: &str, b: &str, m: Option<(Move, bool)>| {
        out.entry(a.to_owned()).or_default().push((b.to_owned(), m));
        out.entry(b.to_owned()).or_default().push((a.to_owned(), m.map(|(m, s)| (m.back(), s))));
    };
    // A term and the variable it is (a part of): `part` gives the way from the variable to the term.
    for (j, t) in rule.head_args().into_iter().enumerate() {
        if let Some((x, m)) = part(rule, t) {
            add(x, &head(j), m);
        }
    }
    let mut children = Vec::new();
    for literal in rule.lits() {
        match literal {
            Lit::Atom(a) if !a.neg() => {
                let args: Vec<TermRef<'_>> = a.args().collect();
                for (&(i, j), ms) in moves.get(rule.rel(a)).into_iter().flatten() {
                    if let (Some(x), Some(y)) = (arg_var(rule, &args, i), arg_var(rule, &args, j)) {
                        ms.iter().for_each(|(&m, &s)| add(x, y, Some((m, s))));
                    }
                }
                if rule.rel(a) == "child"
                    && let [p, c, i] = args.as_slice()
                    && let (Some(p), Some(c)) = (rule.var(*p), rule.var(*c))
                {
                    children.push((p, c, *i));
                }
            }
            Lit::Nearest { n, a, .. } => add(rule.name(n), rule.name(a), Some((Move::Earlier, true))),
            Lit::Cmp { l, op: CmpOp::Eq, r } => {
                let (v, t) = match (rule.var(l), rule.var(r)) {
                    (Some(v), _) => (v, r),
                    (None, Some(v)) => (v, l),
                    _ => continue,
                };
                if let Some((x, m)) = part(rule, t) {
                    add(x, v, m);
                }
            }
            Lit::Cmp { l, op, r } => {
                let (Some(x), Some(y)) = (rule.var(l), rule.var(r)) else { continue };
                // A node is its preorder index, so `<` moves later, unless ids are per definition (`Move::Greater`).
                let (later, earlier) = if by_value() { (Move::Greater, Move::Smaller) } else { (Move::Later, Move::Earlier) };
                match op {
                    CmpOp::Lt => add(x, y, Some((later, true))),
                    CmpOp::Le => add(x, y, Some((later, false))),
                    CmpOp::Gt => add(x, y, Some((earlier, true))),
                    CmpOp::Ge => add(x, y, Some((earlier, false))),
                    _ => {}
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
fn before<'v>(rule: Rule<'v>, i: TermRef<'v>, j: TermRef<'v>) -> bool {
    if let (TermRef::Int(a), TermRef::Int(b)) = (i, j) {
        return a < b;
    }
    rule.lits().any(|lit| match lit {
        Lit::Cmp { l, op: CmpOp::Lt, r } => rule.same(l, i) && rule.same(r, j),
        Lit::Cmp { l, op: CmpOp::Gt, r } => rule.same(l, j) && rule.same(r, i),
        Lit::Cmp { l, op: CmpOp::Eq, r } if rule.same(l, j) => match rule.call(r) {
            Some(("add", args)) => matches!(args.as_slice(), [x, TermRef::Int(c)] | [TermRef::Int(c), x] if rule.same(*x, i) && *c > 0),
            _ => false,
        },
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
fn depth_edges(rule: Rule<'_>, depths: &Depths) -> BTreeMap<String, Vec<(String, Span)>> {
    let mut out = BTreeMap::<String, Vec<(String, Span)>>::new();
    let mut add = |a: &str, b: &str, d: Span| {
        out.entry(a.to_owned()).or_default().push((b.to_owned(), d));
        out.entry(b.to_owned()).or_default().push((a.to_owned(), back_span(d)));
    };
    for literal in rule.lits() {
        match literal {
            Lit::Atom(a) if !a.neg() => {
                let args: Vec<TermRef<'_>> = a.args().collect();
                for (&(i, j), &d) in depths.get(rule.rel(a)).into_iter().flatten() {
                    if let (Some(x), Some(y)) = (arg_var(rule, &args, i), arg_var(rule, &args, j)) {
                        add(x, y, d);
                    }
                }
            }
            Lit::Nearest { n, a, .. } => add(rule.name(n), rule.name(a), (-INF, -1)),
            Lit::Cmp { l, op: CmpOp::Eq, r } => {
                if let (Some(x), Some(y)) = (rule.var(l), rule.var(r)) {
                    add(x, y, (0, 0));
                }
            }
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
fn depth_summary(p: &Prog<'_>, rules: &[Rule<'_>], depths: &Depths) -> BTreeMap<(usize, usize), Span> {
    let nodes = p.columns(rules[0].head_rel(), "node");
    let reached: Vec<BTreeMap<usize, BTreeMap<String, Span>>> = rules
        .iter()
        .map(|&rule| {
            let edges = depth_edges(rule, depths);
            let args = rule.head_args();
            nodes.iter().filter_map(|&i| arg_var(rule, &args, i).map(|x| (i, depth_reach(&edges, x)))).collect()
        })
        .collect();
    let mut out = BTreeMap::new();
    for &i in &nodes {
        for &j in nodes.iter().filter(|&&j| j != i) {
            let mut span = EMPTY;
            let mut all = true;
            for (rule, reached) in rules.iter().zip(&reached) {
                let d = match (reached.get(&i), rule.head_arg(j).and_then(|t| rule.var(t))) {
                    (Some(r), Some(y)) => r.get(y).copied(),
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
/// from no rows up, widened to no bound on a side that still moves after a few rounds. As with `summarize`, an
/// input, or a relation with facts, guarantees nothing.
/// A component that reads no member of itself (not `recursive`) is settled by its first round: its rules read only
/// relations below it, so a second round gives the first's spans again.
fn depth_summaries(p: &Prog<'_>, component: &BTreeSet<String>, by_head: &BTreeMap<&str, Vec<Rule<'_>>>, depths: &mut Depths, recursive: bool) {
    let (rels, none): (Vec<&String>, Vec<&String>) =
        component.iter().filter(|rel| p.columns(rel, "node").len() > 1).partition(|rel| !p.given(rel) && by_head.contains_key(rel.as_str()));
    for rel in none {
        depths.remove(rel.as_str());
    }
    for rel in &rels {
        let nodes = p.columns(rel, "node");
        let empty = nodes.iter().flat_map(|&i| nodes.iter().filter(move |&&j| j != i).map(move |&j| ((i, j), EMPTY))).collect();
        depths.insert((*rel).clone(), empty);
    }
    for round in 0..64 {
        let mut changed = false;
        for rel in &rels {
            let next = depth_summary(p, &by_head[rel.as_str()], depths);
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
        if !changed || !recursive {
            return;
        }
    }
    for rel in rels {
        depths.remove(rel.as_str());
    }
}

/// `reach`, with a move to an unequal value (`$e != $n` in the rule) strict.
fn step(rule: Rule<'_>, edges: &Edges, from: &str, to: &str, way: Move) -> Option<bool> {
    let got = reach(edges, from, to, way)?;
    // The head's column `#j` is its variable there (a part of a variable is tested by nothing).
    let named = |v: &str| -> Option<String> {
        match v.strip_prefix('#').and_then(|j| j.parse::<usize>().ok()).and_then(|j| rule.head_arg(j)) {
            Some(t) => rule.var(t).map(str::to_owned),
            None => Some(v.to_owned()),
        }
    };
    let unequal = named(from).zip(named(to)).is_some_and(|(f, t)| {
        rule.lits().any(|lit| {
            matches!(lit, Lit::Cmp { l, op: CmpOp::Ne, r } if (rule.is_var(l, &f) && rule.is_var(r, &t)) || (rule.is_var(l, &t) && rule.is_var(r, &f)))
        })
    });
    Some(got || unequal)
}

/// Whether a term is a part of its first argument, and whether always a strictly shorter one: `Some(strictly)`.
/// `@replace_all(s, re, "")` deletes from s; its values are parts of s too (as a subsequence).
fn shrinks<'v>(rule: Rule<'v>, t: TermRef<'v>) -> Option<bool> {
    let (f, args) = rule.call(t)?;
    match (f, args.as_slice()) {
        ("rx_group", [_, TermRef::Str(re), TermRef::Str(g)]) => Some(gap(rule.text(*re), rule.text(*g)).is_some_and(|n| n > 0)),
        ("rx_group", [_, _, _]) | ("pkg_up" | "substr" | "slice" | "dir" | "head", [_, ..]) => Some(false),
        ("replace_all", [_, _, TermRef::Str(r)]) if rule.text(*r).is_empty() => Some(false),
        _ => None,
    }
}
/// The fewest bytes a match of `re` has outside its group `g` when g takes part (its capture is that much
/// shorter than the text matched); `None` when `re` does not parse or has no group g.
pub fn gap(re: &str, g: &str) -> Option<usize> {
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
struct Bounds<'v> {
    rule: Rule<'v>,
    /// The variables bound outside the component, and those the rule bounds above and below by integers (`$t >= 0,
    /// $t <= 100000`: a whole number between two, as a table of them would bind it): finitely many values each,
    /// fixed while it runs.
    outside: BTreeSet<String>,
}

impl<'v> Bounds<'v> {
    fn new(rule: Rule<'v>, component: &BTreeSet<String>) -> Self {
        let mut outside = BTreeSet::new();
        for l in rule.lits() {
            match l {
                Lit::Atom(a) if !a.neg() && !component.contains(rule.rel(a)) => outside.extend(rule.atom_vars(a)),
                Lit::Agg { var, atom, .. } if !component.contains(rule.rel(atom)) => {
                    outside.insert(rule.name(var).to_owned());
                }
                Lit::Nearest { .. } | Lit::Ast { .. } => outside.extend(rule.lit_vars(l)),
                _ => {}
            }
        }
        let bound = |v: &str, below: bool| {
            rule.lits().any(|lit| match lit {
                Lit::Cmp { l, op, r: TermRef::Int(_) } if rule.is_var(l, v) => {
                    if below { matches!(op, CmpOp::Gt | CmpOp::Ge) } else { matches!(op, CmpOp::Lt | CmpOp::Le) }
                }
                Lit::Cmp { l: TermRef::Int(_), op, r } if rule.is_var(r, v) => {
                    if below { matches!(op, CmpOp::Lt | CmpOp::Le) } else { matches!(op, CmpOp::Gt | CmpOp::Ge) }
                }
                _ => false,
            })
        };
        for lit in rule.lits() {
            let Lit::Cmp { l, r, .. } = lit else { continue };
            let v = match (l, r) {
                (TermRef::Var(_), TermRef::Int(_)) => rule.var(l),
                (TermRef::Int(_), TermRef::Var(_)) => rule.var(r),
                _ => None,
            };
            if let Some(v) = v
                && bound(v, true)
                && bound(v, false)
            {
                outside.insert(v.to_owned());
            }
        }
        Bounds { rule, outside }
    }

    /// Whether `t` makes a new value; `alias` is the variable it is assigned to, if any.
    fn makes(&self, t: TermRef<'v>, alias: Option<&str>) -> bool {
        let Some((_, args)) = self.rule.call(t) else { return false };
        if shrinks(self.rule, t).is_some() {
            return args.first().is_some_and(|&a| self.makes(a, None));
        }
        let mut vars = BTreeSet::new();
        self.rule.vars(t, &mut vars);
        !vars.is_subset(&self.outside) && !self.counter(t, alias)
    }

    /// `x + c` or `x - c` (c a positive integer) with x or `alias` bounded the way it moves.
    fn counter(&self, t: TermRef<'v>, alias: Option<&str>) -> bool {
        let rule = self.rule;
        let Some((f, args)) = rule.call(t) else { return false };
        let var_int = |a: TermRef<'v>, b: TermRef<'v>| match (rule.var(a), b) {
            (Some(x), TermRef::Int(c)) => Some((x, c)),
            _ => None,
        };
        let (x, by) = match (f, args.as_slice()) {
            ("add", &[a, b]) => match var_int(a, b).or_else(|| var_int(b, a)) {
                Some(got) => got,
                None => return false,
            },
            ("sub", &[a, b]) => match var_int(a, b).and_then(|(x, c)| c.checked_neg().map(|by| (x, by))) {
                Some(got) => got,
                None => return false,
            },
            _ => return false,
        };
        let bound = |k: TermRef<'v>| matches!(k, TermRef::Int(_)) || rule.var(k).is_some_and(|v| self.outside.contains(v));
        let this = |t: TermRef<'v>| rule.var(t).is_some_and(|v| v == x || Some(v) == alias);
        let (below, above): (&[CmpOp], &[CmpOp]) = if by > 0 { (&[CmpOp::Lt, CmpOp::Le], &[CmpOp::Gt, CmpOp::Ge]) } else { (&[CmpOp::Gt, CmpOp::Ge], &[CmpOp::Lt, CmpOp::Le]) };
        by == 0
            || rule.lits().any(|lit| match lit {
                Lit::Cmp { l, op, r } => (this(l) && below.contains(&op) && bound(r)) || (this(r) && above.contains(&op) && bound(l)),
                _ => false,
            })
    }

    /// The variables bound by positive atoms (any relation's).
    fn atoms(&self) -> BTreeSet<String> {
        self.rule.body().filter_map(|l| l.atom().filter(|a| !a.neg())).flat_map(|a| self.rule.atom_vars(a)).collect()
    }

    /// The assignments `$v = t` (either way round), in order.
    fn assignments(&self) -> impl Iterator<Item = (&'v str, TermRef<'v>)> + use<'v> {
        let rule = self.rule;
        rule.lits().filter_map(move |lit| match lit {
            Lit::Cmp { l, op: CmpOp::Eq, r } => match (rule.var(l), rule.var(r)) {
                (Some(v), _) => Some((v, r)),
                (None, Some(v)) => Some((v, l)),
                _ => None,
            },
            _ => None,
        })
    }
}

/// Whether a rule makes a value into its head: a head term that makes one, or reads a variable assigned one.
fn creates(rule: Rule<'_>, component: &BTreeSet<String>) -> bool {
    let b = Bounds::new(rule, component);
    let bound = b.atoms();
    let mut made = BTreeSet::<&str>::new();
    let reads_made = |t: TermRef<'_>, made: &BTreeSet<&str>| {
        let mut vars = BTreeSet::new();
        rule.vars(t, &mut vars);
        vars.iter().any(|v| made.contains(v.as_str()))
    };
    loop {
        let before = made.len();
        for (v, t) in b.assignments() {
            if !bound.contains(v) && (b.makes(t, Some(v)) || reads_made(t, &made)) {
                made.insert(v);
            }
            // A split of a made value (`m2 = @cat(p, x)`, m2 made): a part may be all of it (`"" . m2`).
            if made.contains(v)
                && let Some(("cat", parts)) = rule.call(t)
                && parts.iter().all(|&p| rule.var(p).is_some())
            {
                made.extend(parts.iter().filter_map(|&p| rule.var(p)).filter(|p| !bound.contains(*p)));
            }
        }
        if made.len() == before {
            break;
        }
    }
    rule.head_args().into_iter().any(|t| b.makes(t, None) || reads_made(t, &made))
}

/// A column of a relation.
type Column = (String, usize);

/// Where a term's values come from in a rule (`flow`): the columns its variables are read from, each with whether a
/// value was made on the way (the term's own call, when it makes one).
fn sources<'v>(b: &Bounds<'v>, t: TermRef<'v>, alias: Option<&str>, from: &BTreeMap<String, BTreeSet<(Column, bool)>>) -> BTreeSet<(Column, bool)> {
    let mut vars = BTreeSet::new();
    b.rule.vars(t, &mut vars);
    let made = b.makes(t, alias);
    vars.iter().flat_map(|v| from.get(v).into_iter().flatten().map(move |(c, m)| (c.clone(), *m || made))).collect()
}

/// The flow of values in `rules` between the columns of `component`: `(from, to, made)`, where `made` is a
/// builtin's new value from its arguments.
fn flow(rules: &[Rule<'_>], component: &BTreeSet<String>) -> Vec<(Column, Column, bool)> {
    let mut out = Vec::new();
    for &rule in rules {
        let b = Bounds::new(rule, component);
        // Where each variable's values come from: the component's columns it is read from, and through which
        // assignments (`$v = term`, made when the term makes a value). A variable bound outside the component
        // takes finitely many values, whatever reaches it: nothing flows through it.
        let mut from = BTreeMap::<String, BTreeSet<(Column, bool)>>::new();
        let bound = b.atoms();
        let fixed = |v: &str| bound.contains(v) || b.outside.contains(v);
        for literal in rule.body() {
            if let Some(a) = literal.atom().filter(|a| !a.neg() && component.contains(rule.rel(*a))) {
                for (i, t) in a.args().enumerate() {
                    if let Some(v) = rule.var(t)
                        && !b.outside.contains(v)
                    {
                        from.entry(v.to_owned()).or_default().insert(((rule.rel(a).to_owned(), i), false));
                    }
                }
            }
        }
        // A variable no atom binds and no assignment gives is a part of a bound value it is split from
        // (`$m = @cat($q, $x)` with `$m` bound).
        let assigned: BTreeSet<&str> = b.assignments().map(|(v, _)| v).filter(|v| !bound.contains(*v)).collect();
        // Assignments in order, until nothing more flows (an assignment may read another's variable).
        loop {
            let before = from.values().map(BTreeSet::len).sum::<usize>();
            for (v, t) in b.assignments() {
                if !fixed(v) {
                    let s = sources(&b, t, Some(v), &from);
                    from.entry(v.to_owned()).or_default().extend(s);
                }
                let mut parts = BTreeSet::new();
                rule.vars(t, &mut parts);
                let whole = from.get(v).cloned().unwrap_or_default();
                for p in parts.iter().filter(|p| !fixed(p.as_str()) && !assigned.contains(p.as_str())) {
                    from.entry(p.clone()).or_default().extend(whole.iter().cloned());
                }
            }
            if from.values().map(BTreeSet::len).sum::<usize>() == before {
                break;
            }
        }
        // A head term that a variable bound outside the component is assigned (`$t = @add($l, 1), lines(d, $t, _)`
        // and the head's `@add($l, 1)`, as `.rr` lowers a call it binds and also writes) takes that variable's
        // finitely many values (O111: `era001`'s `era_run`).
        let finite_terms: Vec<TermRef<'_>> = rule
            .lits()
            .filter_map(|lit| match lit {
                Lit::Cmp { l, op: CmpOp::Eq, r } if rule.var(l).is_some_and(|v| b.outside.contains(v)) => Some(r),
                _ => None,
            })
            .collect();
        for (j, t) in rule.head_args().into_iter().enumerate() {
            if finite_terms.iter().any(|&f| rule.same(f, t)) {
                continue;
            }
            for (c, made) in sources(&b, t, None, &from) {
                out.push((c, (rule.head_rel().to_owned(), j), made));
            }
        }
    }
    out
}

/// Whether a value made in `rules` flows back to the builtin that made it.
fn value_cycle(p: &Prog<'_>, rules: &[Rule<'_>], component: &BTreeSet<String>) -> bool {
    let lattice_value = |(rel, col): &Column| p.lattice.get(rel).is_some_and(|d| d.keys == *col);
    let flow: Vec<_> = flow(rules, component)
        .into_iter()
        .map(|(a, b, made)| {
            let made = made && !lattice_value(&b);
            (a, b, made)
        })
        .collect();
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

/// What the relations of `component` guarantee between their columns in every row, added to `moves`: the
/// greatest set of moves each rule gives, its recursive atoms assumed to give them (an input, or a relation
/// with facts, guarantees nothing).
/// As `depth_summaries`, a component that is not `recursive` is settled by its first round.
fn summarize(p: &Prog<'_>, component: &BTreeSet<String>, by_head: &BTreeMap<&str, Vec<Rule<'_>>>, moves: &mut Moves, recursive: bool) {
    let (rels, none): (Vec<&String>, Vec<&String>) = component.iter().partition(|r| !p.given(r) && by_head.contains_key(r.as_str()));
    for rel in none {
        moves.remove(rel);
    }
    // At first every move between columns of a type, strictly; then what the rules do not give is taken out, until
    // they give all that is left.
    let mut assumed = BTreeMap::<String, BTreeMap<(usize, usize), BTreeMap<Move, bool>>>::new();
    for rel in &rels {
        let entry = assumed.entry((*rel).clone()).or_default();
        for way in ways().into_iter().chain([Move::Longer, Move::Shorter]) {
            let cols = p.columns(rel, way.ty());
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
            let all: Vec<Edges> = rules.iter().map(|&r| edges(r, moves)).collect();
            let held = next.get_mut(rel.as_str()).expect("assumed");
            for (&(i, j), ways) in held.iter_mut() {
                ways.retain(|&way, strict| match rules.iter().zip(&all).map(|(&r, e)| step(r, e, &head(i), &head(j), way)).collect::<Option<Vec<bool>>>() {
                    Some(got) => {
                        *strict = *strict && got.iter().all(|&s| s);
                        true
                    }
                    None => false,
                });
            }
            held.retain(|_, ways| !ways.is_empty());
        }
        if next == assumed || !recursive {
            assumed = next;
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

/// A relation's key: a column along the way, and maybe a count beside it (an integer column); or a count alone
/// (`(None, Some(c))`), the place never moving.
type Key = (Option<usize>, Option<usize>);

/// How a rule's count moves from a recursive atom's (`body`, None for a relation with none) to its head's
/// (column `head`): `Some(true)` up by a positive integer from a value bound outside the component, or to one (a
/// child's index: `j(c, i + 1, …) :- j(c, i, …), child(c, p, i + 1)`), or under a bound as a counter is
/// (`Bounds::counter`), or to a value bound outside the component that a comparison puts above it (`$k > $i`), so
/// through finitely many values; `Some(false)` the same; a relation with no count is above every count.
fn count_step<'v>(rule: Rule<'v>, bounds: &Bounds<'v>, edges: &Edges, head: Option<usize>, body: Option<TermRef<'v>>) -> Option<bool> {
    let outside = &bounds.outside;
    let (j, b) = match (head, body) {
        (None, None) => return Some(false),
        (None, Some(_)) => return Some(true),
        (Some(j), Some(t)) => (j, rule.var(t)?),
        _ => return None,
    };
    let up = |t: TermRef<'v>| {
        matches!(rule.call(t), Some(("add", args)) if matches!(args.as_slice(), [x, TermRef::Int(c)] | [TermRef::Int(c), x] if rule.is_var(*x, b) && *c > 0))
    };
    // `t` is the value of a variable bound outside the component (`$k = t`, either way round).
    let outside_value = |t: TermRef<'v>| {
        rule.lits().any(|lit| {
            matches!(lit, Lit::Cmp { l, op: CmpOp::Eq, r } if [(l, r), (r, l)].into_iter().any(|(v, o)| rule.var(v).is_some_and(|v| outside.contains(v)) && rule.same(o, t)))
        })
    };
    // A comparison is a move in values' order: document order's under preorder ids (`by_value`).
    let smaller = if by_value() { Move::Smaller } else { Move::Earlier };
    let above = |x: &str| outside.contains(x) && reach(edges, x, b, smaller) == Some(true);
    let h = rule.head_arg(j)?;
    match rule.var(h) {
        Some(x) if x == b => Some(false),
        Some(x) => rule
            .lits()
            .find_map(|lit| {
                let Lit::Cmp { l, op: CmpOp::Eq, r } = lit else { return None };
                if [(l, r), (r, l)].into_iter().any(|(p, q)| rule.is_var(p, x) && rule.is_var(q, b)) {
                    return Some(false);
                }
                (rule.is_var(l, x) && up(r) && (outside.contains(b) || outside.contains(x) || bounds.counter(r, Some(x)))).then_some(true)
            })
            .or_else(|| above(x).then_some(true)),
        None if up(h) && (outside.contains(b) || outside_value(h) || bounds.counter(h, None)) => Some(true),
        None => None,
    }
}

/// For a relation of `component` with no node column, its input columns when it is a function of them (rule 2's
/// functions): an input is a column every rule binds by a positive atom or to a constant; no rule reads a
/// relation of `keyless`, and each other column is made from the inputs and values bound outside the component.
fn function(rules: &[Rule<'_>], component: &BTreeSet<String>, keyless: &BTreeSet<&str>) -> Option<BTreeSet<usize>> {
    let arity = rules.first()?.arity();
    let bound_by_atom = |rule: Rule<'_>, v: &str| rule.lits().any(|l| matches!(l, Lit::Atom(a) if !a.neg() && a.args().any(|t| rule.is_var(t, v))));
    let inputs: BTreeSet<usize> = (0..arity)
        .filter(|&j| {
            rules.iter().all(|&rule| match rule.head_arg(j) {
                Some(TermRef::Int(_) | TermRef::Str(_)) => true,
                Some(t) => rule.var(t).is_some_and(|v| bound_by_atom(rule, v)),
                None => false,
            })
        })
        .collect();
    for &rule in rules {
        if rule.body().any(|l| l.atom().is_some_and(|a| keyless.contains(rule.rel(a)))) {
            return None;
        }
        let mut known = Bounds::new(rule, component).outside;
        let args = rule.head_args();
        for &j in &inputs {
            rule.vars(args[j], &mut known);
        }
        loop {
            let before = known.len();
            for lit in rule.lits() {
                if let Lit::Cmp { l, op: CmpOp::Eq, r } = lit
                    && let Some(v) = rule.var(l)
                {
                    let mut vars = BTreeSet::new();
                    rule.vars(r, &mut vars);
                    if vars.is_subset(&known) {
                        known.insert(v.to_owned());
                    }
                }
            }
            if known.len() == before {
                break;
            }
        }
        for (_, &t) in args.iter().enumerate().filter(|(j, _)| !inputs.contains(j)) {
            let mut vars = BTreeSet::new();
            rule.vars(t, &mut vars);
            if !vars.is_subset(&known) {
                return None;
            }
        }
    }
    Some(inputs)
}

/// Whether, along `way`, keys `key` (one per relation of `component` but its `functions`, with their inputs)
/// bound every recursive rule: each moves from its head's key to each recursive atom's key, strictly or not, and
/// no cycle of moves that are not strict goes through a rule that makes a value (`creating`, per rule, and every
/// rule reading a function). With a count, a key moves strictly when its column does, or stays and its count
/// goes up (`tpl_upto(t, i + 1, …) :- tpl_upto(t, i, …), child(t, c, i)`): one column's values by the other's,
/// finitely many either way. A function's inputs are bound outside the component or by an atom strictly along
/// the key. Along depth (`Shallower`, `Deeper`) a key moves strictly or not at all (`depth_reach` over `depths`).
#[allow(clippy::too_many_arguments)]
fn bounded(
    rules: &[Rule<'_>],
    creating: &[bool],
    component: &BTreeSet<String>,
    key: &BTreeMap<&str, Key>,
    functions: &BTreeMap<&str, BTreeSet<usize>>,
    way: Move,
    moves: &Moves,
    depths: &Depths,
) -> Result<(), String> {
    // The moves that are not strict, head → atom, and those of rules making a value.
    let mut same = super::Edges::new();
    let mut made = Vec::new();
    for (&rule, &creates) in rules.iter().zip(creating) {
        let hrel = rule.head_rel();
        if functions.contains_key(hrel) {
            continue;
        }
        let (k, count) = key[hrel];
        let edges = edges(rule, moves);
        let bounds = Bounds::new(rule, component);
        let creates = creates || rule.body().any(|l| l.atom().is_some_and(|a| functions.contains_key(rule.rel(a))));
        // Along depth, a key moves strictly or not at all: the depth of each variable from the head's node.
        let depth_from = match (matches!(way, Move::Shallower | Move::Deeper), k.and_then(|k| rule.head_arg(k)).and_then(|t| rule.var(t))) {
            (true, Some(h)) => Some(depth_reach(&depth_edges(rule, depths), h)),
            (true, None) => Some(BTreeMap::new()),
            (false, _) => None,
        };
        // The variables an atom strictly along the key binds, for the functions' inputs.
        let mut before = BTreeSet::<&str>::new();
        for literal in rule.body() {
            let Some(a) = literal.atom().filter(|a| component.contains(rule.rel(*a)) && !functions.contains_key(rule.rel(*a))) else { continue };
            let (ak, acount) = key[rule.rel(a)];
            let args: Vec<TermRef<'_>> = a.args().collect();
            let placed = match (k, ak) {
                (Some(k), Some(ak)) => {
                    let Some(b) = arg_var(rule, &args, ak) else { return Err(rule.shown()) };
                    match &depth_from {
                        Some(from) => {
                            let &(lo, hi) = from.get(b).ok_or_else(|| rule.shown())?;
                            (if way == Move::Shallower { hi <= -1 } else { lo >= 1 }).then_some(true)
                        }
                        None => step(rule, &edges, &head(k), b, way),
                    }
                }
                // Counts alone: the place stays.
                (None, None) => Some(false),
                _ => None,
            };
            let moved = match placed {
                Some(false) => count_step(rule, &bounds, &edges, count, acount.map(|c| args.get(c).copied().unwrap_or(TermRef::Wild))),
                other => other,
            };
            match moved {
                Some(true) => {
                    if matches!(literal.view(), Lit::Atom(x) if !x.neg()) {
                        before.extend(args.iter().filter_map(|&t| rule.var(t)));
                    }
                }
                Some(false) => {
                    same.entry(hrel.to_owned()).or_default().insert((rule.rel(a).to_owned(), false));
                    if creates {
                        made.push((hrel, rule.rel(a), rule));
                    }
                }
                None => return Err(rule.shown()),
            }
        }
        for literal in rule.body() {
            let Some(a) = literal.atom().filter(|a| functions.contains_key(rule.rel(*a))) else { continue };
            let args: Vec<TermRef<'_>> = a.args().collect();
            for &j in &functions[rule.rel(a)] {
                match args.get(j) {
                    Some(&t) if rule.var(t).is_some_and(|v| before.contains(v) || bounds.outside.contains(v)) => {}
                    Some(TermRef::Int(_) | TermRef::Str(_)) => {}
                    _ => return Err(rule.shown()),
                }
            }
        }
    }
    let cycles = super::tarjan(component, &same);
    match made.into_iter().find(|(h, a, _)| cycles.iter().any(|c| c.contains(*h) && c.contains(*a))) {
        Some((_, _, rule)) => Err(rule.shown()),
        None => Ok(()),
    }
}

/// Assignments of a key to each relation, at most `limit` of them.
fn keys<'a, T: Copy>(choices: &[(&'a str, Vec<T>)], limit: usize) -> Vec<BTreeMap<&'a str, T>> {
    let mut out = vec![BTreeMap::new()];
    for (rel, columns) in choices {
        out = out
            .into_iter()
            .flat_map(|key| {
                columns.iter().map(move |&c| {
                    let mut key = key.clone();
                    key.insert(*rel, c);
                    key
                })
            })
            .take(limit)
            .collect();
    }
    out
}

/// Whether a component recurses only between constant keys, and never back to one (O111): each relation has a
/// column that is a constant in every rule's head and in every atom of the component read, and the graph from a head's
/// `(relation, constant)` to its atoms' is acyclic. Every derivation is then at most as deep as that graph, and makes
/// finitely many values: `era001`'s `era_re` builds its statement grammar from `N` up to `STMT` this way.
fn constant_keyed(rules: &[Rule<'_>], component: &BTreeSet<String>) -> bool {
    let key = |rule: Rule<'_>, t: TermRef<'_>| match t {
        TermRef::Str(s) => Some(format!("s{}", rule.text(s))),
        TermRef::Int(n) => Some(format!("i{n}")),
        _ => None,
    };
    let arity = |rel: &str| rules.iter().find(|r| r.head_rel() == rel).map_or(0, |r| r.arity());
    // Each relation's key column: the first that every head and every read in the component holds a constant at.
    let mut columns = BTreeMap::<&str, usize>::new();
    for rel in component {
        let heads = rules.iter().filter(|r| r.head_rel() == rel.as_str()).map(|&r| (r, r.r.head()));
        let reads = rules.iter().flat_map(|&r| {
            r.lits().filter_map(move |l| match l {
                Lit::Atom(a) | Lit::Agg { atom: a, .. } if r.rel(a) == rel.as_str() => Some((r, a)),
                _ => None,
            })
        });
        let atoms: Vec<_> = heads.chain(reads).collect();
        let Some(c) = (0..arity(rel)).find(|&c| atoms.iter().all(|&(r, a)| a.args().nth(c).and_then(|t| key(r, t)).is_some())) else { return false };
        columns.insert(rel.as_str(), c);
    }
    let node = |r: Rule<'_>, a: AtomRef<'_>| {
        let rel = r.rel(a);
        key(r, a.args().nth(*columns.get(rel)?)?).map(|k| (rel.to_owned(), k))
    };
    let mut graph = BTreeMap::<(String, String), BTreeSet<(String, String)>>::new();
    for &rule in rules {
        let Some(head) = node(rule, rule.r.head()) else { return false };
        let to = graph.entry(head).or_default();
        for l in rule.lits() {
            if let Lit::Atom(a) | Lit::Agg { atom: a, .. } = l
                && component.contains(rule.rel(a))
            {
                let Some(n) = node(rule, a) else { return false };
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

/// The components with a value cycle, each bounded by its keys; fails on one that is not
/// (`Error::UnboundedValueCycle`). It reads the version's rules in place, its components as the tree's strata made
/// them (`graph::components_by_name`).
pub fn check(v: &Version) -> Result<Vec<BTreeSet<String>>, Error> {
    checked(v, &Given::default())
}

/// `check` of a program lowered without the rules of the library it reads (`rr::Text::lower_in`, decision 0246 §2):
/// a library relation it reads guarantees what its rules in `library` do (`deparen(e, a)`: a is e or below it), as
/// in the program lowered whole. Read only when the program alone fails: its library relations have no rules in it,
/// so they guarantee nothing there (PR #508's 363b91c0b, t47).
pub fn check_given(v: &Version, library: &Version) -> Result<Vec<BTreeSet<String>>, Error> {
    match check(v) {
        Err(Error::UnboundedValueCycle { .. }) => checked(v, &given(v, library)),
        done => done,
    }
}

/// What a library gives a program's check: the moves and depths of the library relations the program reads and does
/// not derive, by name, and those relations.
#[derive(Default)]
struct Given {
    moves: Moves,
    depths: Depths,
    rels: BTreeSet<String>,
}

/// The guarantees of the relations of `library` that `v` reads and does not derive itself, summarized over their
/// rules there and the relations below them (`summarize`, `depth_summaries`).
fn given(v: &Version, library: &Version) -> Given {
    let reads = |rule: Rule<'_>| -> Vec<String> {
        rule.lits()
            .filter_map(|l| match l {
                Lit::Atom(a) | Lit::Agg { atom: a, .. } => Some(rule.v.rel_name(a.rel()).to_owned()),
                _ => None,
            })
            .collect()
    };
    let own: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
    let mut by_head = BTreeMap::<&str, Vec<Rule<'_>>>::new();
    for r in library.rules() {
        let rule = Rule { v: library, r };
        by_head.entry(rule.head_rel()).or_default().push(rule);
    }
    let mut reach: BTreeSet<String> = BTreeSet::new();
    let mut todo: Vec<String> = v.rules().flat_map(|r| reads(Rule { v, r })).filter(|r| !own.contains(r.as_str()) && by_head.contains_key(r.as_str())).collect();
    while let Some(rel) = todo.pop() {
        if reach.contains(&rel) {
            continue;
        }
        todo.extend(by_head[rel.as_str()].iter().flat_map(|&r| reads(r)).filter(|r| by_head.contains_key(r.as_str()) && !reach.contains(r)));
        reach.insert(rel);
    }
    let about = About::of(library);
    let Ok(sccs) = graph::components_by_name(library, &about) else { return Given::default() };
    let edges_by = graph::edges_by_name(library, &about);
    let p = Prog { v: library, lattice: ProgramFields::lattice(library) };
    let (mut moves, mut depths) = (base(&p), depth_base());
    for component in sccs.iter().filter(|c| c.iter().any(|r| reach.contains(r))) {
        let recursive = component.len() > 1 || component.iter().any(|r| edges_by.get(r).is_some_and(|e| e.iter().any(|(w, _)| w == r)));
        summarize(&p, component, &by_head, &mut moves, recursive);
        depth_summaries(&p, component, &by_head, &mut depths, recursive);
    }
    let rels: BTreeSet<String> = reach.into_iter().filter(|r| !own.contains(r.as_str()) && !v.has_input(r) && !ProgramFields::states_facts(v, r)).collect();
    moves.retain(|r, _| rels.contains(r));
    depths.retain(|r, _| rels.contains(r));
    Given { moves, depths, rels }
}

/// `check`, the relations of `given` guaranteeing what it holds for them.
fn checked(v: &Version, given: &Given) -> Result<Vec<BTreeSet<String>>, Error> {
    let about = About::of(v);
    // A program that does not stratify fails there, by name.
    let Ok(sccs) = graph::components_by_name(v, &about) else { return Ok(Vec::new()) };
    let edges_by = graph::edges_by_name(v, &about);
    let p = Prog { v, lattice: ProgramFields::lattice(v) };
    let mut treed = Vec::new();
    let mut moves = base(&p);
    let mut depths = depth_base();
    moves.extend(given.moves.iter().map(|(r, m)| (r.clone(), m.clone())));
    depths.extend(given.depths.iter().map(|(r, d)| (r.clone(), d.clone())));
    let mut by_head = BTreeMap::<&str, Vec<Rule<'_>>>::new();
    for r in v.rules() {
        let rule = Rule { v, r };
        by_head.entry(rule.head_rel()).or_default().push(rule);
    }
    // Each component: whether it recurses, and whether its value cycle needs a bound (neither reads the summaries).
    // A recursion through constants only (`era_re("EXPR", …) :- era_re("OPD", …)`): finitely many rules, so finitely
    // many values when the constants do not come back.
    let kinds: Vec<(bool, Option<bool>)> = sccs
        .iter()
        .map(|component| {
            let recursive = component.len() > 1 || component.iter().any(|r| edges_by.get(r).is_some_and(|e| e.iter().any(|(w, _)| w == r)));
            if !recursive {
                return (false, None);
            }
            let rules: Vec<Rule<'_>> = component.iter().flat_map(|rel| by_head.get(rel.as_str()).into_iter().flatten().copied()).collect();
            (true, value_cycle(&p, &rules, component).then(|| !constant_keyed(&rules, component)))
        })
        .collect();
    // The guarantees are read by the components that need a bound, of those below them: none are made past the last.
    let last = kinds.iter().rposition(|(_, bound)| *bound == Some(true));
    for (i, component) in sccs.iter().enumerate() {
        let (recursive, bound) = kinds[i];
        // A component's guarantees, for its own rules and the components above it; a library relation's are given.
        if last.is_some_and(|last| i <= last) && !component.iter().all(|r| given.rels.contains(r)) {
            summarize(&p, component, &by_head, &mut moves, recursive);
            depth_summaries(&p, component, &by_head, &mut depths, recursive);
        }
        match bound {
            None => continue,
            Some(false) => {
                treed.push(component.clone());
                continue;
            }
            Some(true) => {}
        }
        let rules: Vec<Rule<'_>> = component.iter().flat_map(|rel| by_head.get(rel.as_str()).into_iter().flatten().copied()).collect();
        let creating: Vec<bool> = rules.iter().map(|&r| creates(r, component)).collect();
        // The relations with no node column that are functions of their inputs, which need no key.
        let keyless: BTreeSet<&str> = component.iter().map(String::as_str).filter(|rel| p.columns(rel, "node").is_empty()).collect();
        let functions: BTreeMap<&str, BTreeSet<usize>> = keyless
            .iter()
            .filter_map(|&rel| function(by_head.get(rel).map_or(&[][..], Vec::as_slice), component, &keyless).map(|inputs| (rel, inputs)))
            .collect();
        let mut first = None;
        // Strings get only shorter: a longer one is no bound. Keys without counts first, then with them.
        let ok = [false, true].into_iter().any(|counted| {
            ways().into_iter().chain([Move::Longer, Move::Shallower, Move::Deeper]).any(|way| {
                let choices: Vec<(&str, Vec<Key>)> = component
                    .iter()
                    .filter(|rel| !functions.contains_key(rel.as_str()))
                    .map(|rel| {
                        let ints = p.columns(rel, "int");
                        let counts: Vec<Option<usize>> = if counted { ints.iter().copied().map(Some).chain([None]).collect() } else { vec![None] };
                        let mut ks: Vec<Key> = p.columns(rel, way.ty()).into_iter().flat_map(|c| counts.iter().map(move |&n| (Some(c), n))).collect();
                        // A count alone moves the same whatever the way: tried once, and only for a relation with no
                        // node to place it (one with a node keeps its count at a place: a position whose node
                        // changes is no bound).
                        if counted && way == Move::Later && ks.is_empty() {
                            ks.extend(ints.into_iter().map(|n| (None, Some(n))));
                        }
                        (rel.as_str(), ks)
                    })
                    .collect();
                !choices.is_empty()
                    && choices.iter().all(|(_, columns)| !columns.is_empty())
                    && (!counted || choices.iter().any(|(_, ks)| ks.iter().any(|(_, n)| n.is_some())))
                    && keys(&choices, 4096).iter().any(|key| match bounded(&rules, &creating, component, key, &functions, way, &moves, &depths) {
                        Ok(()) => true,
                        Err(rule) => {
                            first.get_or_insert(rule);
                            false
                        }
                    })
            })
        });
        if !ok {
            let rule = first.unwrap_or_else(|| rules.iter().zip(&creating).find(|(_, c)| **c).map(|(r, _)| r.shown()).unwrap_or_default());
            return Err(Error::UnboundedValueCycle { component: component.clone(), rule });
        }
        treed.push(component.clone());
    }
    Ok(treed)
}
