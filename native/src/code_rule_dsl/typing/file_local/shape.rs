//! A rule's variables numbered once for a split (`split_in`), and the two questions the split's fixpoints ask of it
//! each round in that form: which classes of variables a rule's reads place (`placed_classes`, for the location
//! columns), and whether its reads place one document (`one_document`, for the file-local relations).
//!
//! The fixpoints asked them of the rule's text: each round, for each rule, a set of variable names (`String`s),
//! closed under the rule's equalities by scanning them again, the relations looked up by name, and each value tape
//! named by a `format!`. That was about 10% of a cold batch's main thread. Here a rule's variables are numbered once,
//! the equalities are a class per variable (a closure under them is the classes it touches), the derived relations
//! are numbered (their location columns a vector by number), and a round's sets are bits.
//!
//! Only the decisions are made here. A rule that places more than one document is explained by `split_in`'s rule
//! check over the text, which names the documents' variables; it runs once per failing rule.

use super::{base_locations, value_tapes, var_equalities};
use crate::code_rule_dsl::logic::front::{Literal, Program, Rule, Term};
use rustc_hash::FxHashMap;

/// A split's derived relations, numbered in their order.
pub(super) struct Derived<'p> {
    pub(super) id: FxHashMap<&'p str, u32>,
    pub(super) names: Vec<&'p str>,
}

impl<'p> Derived<'p> {
    pub(super) fn of(derived: &std::collections::BTreeSet<&'p str>) -> Self {
        let names: Vec<&str> = derived.iter().copied().collect();
        Derived { id: names.iter().enumerate().map(|(i, r)| (*r, i as u32)).collect(), names }
    }
}

/// A body literal as the split reads it.
#[derive(Clone)]
pub(super) enum Lit {
    /// A read (`Literal::Atom`, or an aggregate's atom).
    Read {
        neg: bool,
        /// A plain atom, not an aggregate's.
        plain: bool,
        /// The class of each argument that is a variable.
        args: Vec<Option<u32>>,
        /// The relation's number, when derived.
        derived: Option<u32>,
        /// A base relation's location columns (`base_locations`).
        base: Vec<usize>,
        /// Read where no document is placed whatever the round: a constant, a chunk's own value, a text test.
        placeless: bool,
        /// A value's text binder (`ValueTapes::by_binder`).
        binder: bool,
        /// The value tapes standing with it, and `tape_of(d, t)`'s tape.
        tapes: Vec<u32>,
    },
    Ast,
    Nearest(u32, u32),
    Cmp,
}

/// A rule numbered for a split: variables by first appearance, each with its class under the rule's equalities
/// (`var_equalities`); then the value tapes; then one id per body literal (a value's binder of no location).
#[derive(Clone)]
pub(super) struct Shape {
    /// The head's relation number.
    pub(super) head_rel: u32,
    /// The class of each head argument that is a variable.
    pub(super) head: Vec<Option<u32>>,
    pub(super) lits: Vec<Lit>,
    /// Each spliced tape's node class and tape id.
    pub(super) splices: Vec<(u32, u32)>,
    /// How many ids a set of this rule's needs.
    pub(super) ids: usize,
    /// Where the binder ids start.
    binders: u32,
}

impl Shape {
    pub(super) fn of(rule: &Rule, prog: &Program, derived: &Derived<'_>, placeless: &dyn Fn(&str) -> bool) -> Shape {
        // Read before the numbering, which borrows their names.
        let equalities = var_equalities(rule);
        let placed = value_tapes(rule);
        let mut vars: FxHashMap<&str, u32> = FxHashMap::default();
        fn number<'a>(vars: &mut FxHashMap<&'a str, u32>, v: &'a str) -> u32 {
            let n = vars.len() as u32;
            *vars.entry(v).or_insert(n)
        }
        // Number every variable the questions read: the head's, the reads', the equalities', the nearest's.
        let head_vars: Vec<Option<u32>> = rule.head.args.iter().map(|t| if let Term::Var(v) = t { Some(number(&mut vars, v)) } else { None }).collect();
        let mut lit_vars: Vec<Vec<Option<u32>>> = Vec::with_capacity(rule.body.len());
        let mut nearest: Vec<Option<(u32, u32)>> = Vec::with_capacity(rule.body.len());
        for l in &rule.body {
            lit_vars.push(l.atom().map_or_else(Vec::new, |a| a.args.iter().map(|t| if let Term::Var(v) = t { Some(number(&mut vars, v)) } else { None }).collect()));
            nearest.push(if let Literal::Nearest { n, a, .. } = l { Some((number(&mut vars, n), number(&mut vars, a))) } else { None });
        }
        let equal: Vec<(u32, u32)> = equalities.iter().map(|(a, b)| (number(&mut vars, a), number(&mut vars, b))).collect();
        let splice_nodes: Vec<u32> = placed.splices.iter().map(|(n, _)| number(&mut vars, n)).collect();
        let n = vars.len();
        // Each variable's class: its root under the equalities.
        let mut root: Vec<u32> = (0..n as u32).collect();
        fn find(root: &mut [u32], mut x: u32) -> u32 {
            while root[x as usize] != x {
                root[x as usize] = root[root[x as usize] as usize];
                x = root[x as usize];
            }
            x
        }
        for (a, b) in equal {
            let (a, b) = (find(&mut root, a), find(&mut root, b));
            if a != b {
                root[a.max(b) as usize] = a.min(b);
            }
        }
        let class: Vec<u32> = (0..n as u32).map(|v| find(&mut root, v)).collect();
        let of = |v: Option<u32>| v.map(|v| class[v as usize]);
        // The value tapes, numbered after the variables, by the name of their class.
        let mut tapes: FxHashMap<String, u32> = FxHashMap::default();
        let mut tape = |t: &str| -> u32 {
            let k = n as u32 + tapes.len() as u32;
            *tapes.entry(t.to_owned()).or_insert(k)
        };
        let splices: Vec<(u32, u32)> = placed.splices.iter().zip(&splice_nodes).map(|((_, t), &v)| (class[v as usize], tape(t))).collect();
        let mut lits = Vec::with_capacity(rule.body.len());
        for (index, (l, args)) in rule.body.iter().zip(lit_vars).enumerate() {
            lits.push(match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => {
                    let d = derived.id.get(a.rel.as_str()).copied();
                    let mut stand: Vec<u32> = placed.by_binder.iter().filter(|(_, b)| **b == index).map(|(t, _)| tape(t)).collect();
                    if a.rel == "tape_of"
                        && let Some(Term::Var(t)) = a.args.get(1)
                        && let Some(t) = placed.canonical(t)
                    {
                        stand.push(tape(&t));
                    }
                    Lit::Read {
                        neg: a.neg,
                        plain: matches!(l, Literal::Atom(_)),
                        args: args.into_iter().map(of).collect(),
                        derived: d,
                        base: if d.is_none() { base_locations(prog, &a.rel) } else { Vec::new() },
                        placeless: placeless(&a.rel),
                        binder: placed.by_binder.values().any(|b| *b == index),
                        tapes: stand,
                    }
                }
                Literal::Ast { .. } => Lit::Ast,
                Literal::Nearest { .. } => {
                    let (a, b) = nearest[index].expect("a nearest's variables");
                    Lit::Nearest(class[a as usize], class[b as usize])
                }
                Literal::Cmp { .. } => Lit::Cmp,
            });
        }
        let binders = (n + tapes.len()) as u32;
        Shape {
            head_rel: derived.id[rule.head.rel.as_str()],
            head: head_vars.into_iter().map(of).collect(),
            lits,
            splices,
            ids: binders as usize + rule.body.len(),
            binders,
        }
    }

    /// The classes the rule's positive reads place, under the location columns `locations` (by relation number).
    pub(super) fn placed_classes(&self, locations: &[Vec<usize>]) -> Bits {
        let mut out = Bits::new(self.ids);
        for l in &self.lits {
            if let Lit::Read { neg: false, args, derived, base, .. } = l {
                let columns = derived.map_or(base.as_slice(), |d| locations[d as usize].as_slice());
                for &c in columns {
                    if let Some(Some(k)) = args.get(c) {
                        out.set(*k);
                    }
                }
            }
        }
        out
    }

    /// Whether the head's column `c` is a variable of a class in `placed`.
    pub(super) fn head_placed(&self, c: usize, placed: &Bits) -> bool {
        matches!(self.head.get(c), Some(Some(k)) if placed.has(*k))
    }

    /// Whether the rule's reads place one document (`split_in`'s rule check, without its explanation): `local`,
    /// `filters` by relation number, `locations` the location columns.
    pub(super) fn one_document(&self, local: &[bool], filters: &[bool], locations: &[Vec<usize>]) -> bool {
        let head_filter = filters[self.head_rel as usize];
        let mut root: Vec<u32> = (0..self.ids as u32).collect();
        fn find(root: &mut [u32], mut x: u32) -> u32 {
            while root[x as usize] != x {
                root[x as usize] = root[root[x as usize] as usize];
                x = root[x as usize];
            }
            x
        }
        fn join(root: &mut [u32], a: u32, b: u32) {
            let (a, b) = (find(root, a), find(root, b));
            if a != b {
                root[a.max(b) as usize] = a.min(b);
            }
        }
        // Each group's first id: groups sharing an id are one document.
        let mut firsts: Vec<u32> = Vec::new();
        for &(n, t) in &self.splices {
            join(&mut root, n, t);
            firsts.push(n);
        }
        let mut group: Vec<u32> = Vec::new();
        for (index, l) in self.lits.iter().enumerate() {
            match l {
                Lit::Read { plain, args, derived, base, placeless, binder, tapes, .. } => {
                    if *placeless || derived.is_some_and(|d| filters[d as usize]) || head_filter {
                        continue;
                    }
                    if derived.is_some_and(|d| !local[d as usize]) {
                        return false;
                    }
                    let columns = derived.map_or(base.as_slice(), |d| locations[d as usize].as_slice());
                    group.clear();
                    group.extend(columns.iter().filter_map(|&c| args.get(c).copied().flatten()));
                    if group.is_empty() && *plain && *binder && !columns.is_empty() {
                        group.push(self.binders + index as u32);
                    }
                    if !group.is_empty() {
                        group.extend(tapes.iter().copied());
                    }
                    let Some(&first) = group.first() else { return false };
                    for &g in &group[1..] {
                        join(&mut root, first, g);
                    }
                    firsts.push(first);
                }
                Lit::Ast => return false,
                Lit::Nearest(n, a) => {
                    join(&mut root, *n, *a);
                    firsts.push(*n);
                }
                Lit::Cmp => {}
            }
        }
        let mut roots: Vec<u32> = firsts.iter().map(|&f| find(&mut root, f)).collect();
        roots.sort_unstable();
        roots.dedup();
        roots.len() == 1 || (roots.is_empty() && head_filter)
    }
}

/// A set of a rule's ids.
pub(super) struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Bits {
        Bits(vec![0; n.div_ceil(64)])
    }
    fn set(&mut self, i: u32) {
        self.0[i as usize / 64] |= 1 << (i % 64);
    }
    fn has(&self, i: u32) -> bool {
        self.0.get(i as usize / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }
}
