//! A split's program in numbers (`split_in`): each rule body numbered once for the run (`Form`), each split's relations
//! numbered once for the split (`Rels`), and the questions the split's fixpoints ask of a rule each round in that form:
//! whether its reads are all constants (`constant_relations`), which classes of variables they place (the location
//! columns), whether it is a string filter, and whether its reads place one document (the file-local relations).
//!
//! The fixpoints asked them of the rule's text: each round, for each rule, a set of variable names (`String`s),
//! closed under the rule's equalities by scanning them again, the relations looked up by name in string-keyed sets,
//! and each value tape named by a `format!`. Here a body's variables are numbered once, the equalities are a class per
//! variable, and a round's sets are bits. A body's form reads nothing but the body, so the run keeps it with the
//! body's digest (`split_memo`): a split numbers only the bodies a round rewrote.
//!
//! Relations are numbered by their name's address: a `Name` is interned, one name one address while any holds it,
//! so a relation's number is found without hashing or comparing its text. What the split asks of a relation by name
//! (a fact? an input? its location columns?) is asked once per split, in `Rels::of`.
//!
//! Only the decisions are made here. A rule that places more than one document is explained by `split_in`'s rule
//! check over the text, which names the documents' variables; it runs once per failing rule.

use super::{CHUNK_VALUES, RuleIndex, base_locations, option_input, value_tapes, var_equalities};
use crate::code_rule_dsl::logic::front::{Atom, Literal, Program, Term};
use rustc_hash::FxHashMap;

/// A relation's (or a variable's) name's address.
pub(super) fn addr(name: &str) -> usize {
    name.as_ptr() as usize
}

/// A split's relations: the derived ones first, in their order, then the base relations the rules read. What the
/// split asks of each by name is asked here once.
pub(super) struct Rels<'p> {
    id: FxHashMap<usize, u32>,
    pub(super) names: Vec<&'p str>,
    /// How many are derived: the numbers below.
    pub(super) derived: usize,
    /// A base relation that is a fact and not an input: a constant.
    pub(super) fact: Vec<bool>,
    /// An input with one value for the run (`run_constant`).
    pub(super) run_constant: Vec<bool>,
    /// A base relation whose rows a chunk makes for itself (`CHUNK_VALUES`).
    pub(super) chunk_value: Vec<bool>,
    /// A text test's relation (`strtest_<k>`).
    pub(super) test: Vec<bool>,
    /// A text relation's string columns (`strings::TEXT_COLUMNS`).
    pub(super) text: Vec<Option<&'static [usize]>>,
    /// A base relation's location columns (`base_locations`).
    pub(super) base_locs: Vec<Vec<usize>>,
    /// A derived relation that may be a constant: not native, not a lattice, not an input.
    pub(super) candidate: Vec<bool>,
    /// A derived relation's rules, by index.
    pub(super) rules: Vec<&'p [usize]>,
    /// The rules reading each relation, by index.
    pub(super) reading: Vec<&'p [usize]>,
    /// Each rule's head.
    pub(super) head: Vec<u32>,
}

impl<'p> Rels<'p> {
    pub(super) fn of(prog: &'p Program, index: &'p RuleIndex<'_>, derived: &std::collections::BTreeSet<&'p str>) -> Rels<'p> {
        let mut names: Vec<&'p str> = derived.iter().copied().collect();
        let n = names.len();
        names.extend(index.read_relations().filter(|r| !derived.contains(r)));
        let id: FxHashMap<usize, u32> = names.iter().enumerate().map(|(i, r)| (addr(r), i as u32)).collect();
        debug_assert_eq!(id.len(), names.len(), "one address per relation name");
        let inputs: rustc_hash::FxHashSet<&str> = prog.inputs.iter().map(String::as_str).collect();
        let test = |r: &str| r.starts_with(crate::code_rule_dsl::logic::strings::RELATION);
        let rels = Rels {
            fact: names.iter().enumerate().map(|(i, r)| i >= n && prog.facts.contains_key(*r) && !inputs.contains(r)).collect(),
            run_constant: names.iter().map(|r| option_input(r) || (crate::code_rule_dsl::logic::plan::corpus::phases::is_phased(r) && inputs.contains(r))).collect(),
            chunk_value: names.iter().enumerate().map(|(i, r)| i >= n && CHUNK_VALUES.contains(r)).collect(),
            test: names.iter().map(|r| test(r)).collect(),
            text: names.iter().map(|r| crate::code_rule_dsl::logic::strings::TEXT_COLUMNS.iter().find(|(t, _)| t == r).map(|(_, c)| *c)).collect(),
            base_locs: names.iter().enumerate().map(|(i, r)| if i >= n { base_locations(prog, r) } else { Vec::new() }).collect(),
            candidate: names.iter().enumerate().map(|(i, r)| i < n && !prog.natives.contains_key(*r) && !prog.lattice.contains_key(*r) && !inputs.contains(r)).collect(),
            rules: names[..n].iter().map(|r| index.rule_ids_of(r)).collect(),
            reading: names.iter().map(|r| index.reading(r)).collect(),
            head: prog.rules.iter().map(|r| id[&addr(r.head.rel.as_str())]).collect(),
            id,
            names,
            derived: n,
        };
        rels
    }

    /// The number of a relation a rule names.
    pub(super) fn of_addr(&self, a: usize) -> u32 {
        self.id[&a]
    }

    /// The number of a relation named by a rule (`None` for a name no rule reads or derives).
    pub(super) fn get(&self, rel: &str) -> Option<u32> {
        self.id.get(&addr(rel)).copied()
    }

    pub(super) fn is_derived(&self, r: u32) -> bool {
        (r as usize) < self.derived
    }

    /// The derived relations whose rules read any of `rels`, each once, in order.
    pub(super) fn heads_reading(&self, rels: impl Iterator<Item = u32>, keep: impl Fn(u32) -> bool) -> Vec<u32> {
        let mut out: Vec<u32> = rels.flat_map(|r| self.reading[r as usize].iter().map(|&i| self.head[i])).filter(|&h| keep(h)).collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// A body literal as the split reads it.
pub(super) enum Lit {
    /// A read (`Literal::Atom`, or an aggregate's atom).
    Read {
        neg: bool,
        /// A plain atom, not an aggregate's.
        plain: bool,
        /// The relation's name's address (`Rels::of_addr`).
        rel: usize,
        /// Each argument that is a variable.
        args: Vec<Option<u32>>,
        /// A value's text binder (`ValueTapes::by_binder`).
        binder: bool,
        /// The value tapes standing with it, and `tape_of(d, t)`'s tape.
        tapes: Vec<u32>,
    },
    Ast,
    /// A `nearest`'s node and ancestor classes.
    Nearest(u32, u32),
    Cmp,
}

/// A rule body numbered for the split: variables (by first appearance), each with its class under the body's
/// equalities (`var_equalities`); then the value tapes; then one id per body literal (a value's binder of no
/// location). It reads only the body, so the run keeps it with the body's digest.
pub(super) struct Form {
    /// Each variable's name's address, where a `Term::Var` names it (0 where only a text does: a `nearest`'s): a head's
    /// variables are found by it.
    names: Vec<usize>,
    class: Vec<u32>,
    lits: Vec<Lit>,
    /// Each spliced tape's node class and tape id.
    splices: Vec<(u32, u32)>,
    /// How many ids a set of this body's needs.
    ids: usize,
    /// Where the binder ids start.
    binders: u32,
}

impl Form {
    pub(super) fn of(body: &[Literal]) -> Form {
        let equalities = var_equalities(body);
        let placed = value_tapes(body);
        let mut vars: FxHashMap<&str, u32> = FxHashMap::default();
        let mut names: Vec<usize> = Vec::new();
        fn number<'a>(vars: &mut FxHashMap<&'a str, u32>, names: &mut Vec<usize>, v: &'a str, named: bool) -> u32 {
            let n = vars.len() as u32;
            let id = *vars.entry(v).or_insert(n);
            if id == n {
                names.push(0);
            }
            if named {
                names[id as usize] = addr(v);
            }
            id
        }
        fn terms<'a>(t: &'a Term, vars: &mut FxHashMap<&'a str, u32>, names: &mut Vec<usize>) {
            match t {
                Term::Var(v) => {
                    number(vars, names, v.as_str(), true);
                }
                Term::Call(_, xs) => xs.iter().for_each(|x| terms(x, vars, names)),
                _ => {}
            }
        }
        // Every variable a term names, with its name's address (a head's variable is found by it); then the texts.
        for l in body {
            match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => a.args.iter().for_each(|t| terms(t, &mut vars, &mut names)),
                Literal::Cmp { l, r, .. } => {
                    terms(l, &mut vars, &mut names);
                    terms(r, &mut vars, &mut names);
                }
                _ => {}
            }
        }
        let arg = |vars: &FxHashMap<&str, u32>, t: &Term| if let Term::Var(v) = t { Some(vars[v.as_str()]) } else { None };
        let lit_args: Vec<Vec<Option<u32>>> = body.iter().map(|l| l.atom().map_or_else(Vec::new, |a| a.args.iter().map(|t| arg(&vars, t)).collect())).collect();
        let nearest: Vec<Option<(u32, u32)>> = body
            .iter()
            .map(|l| if let Literal::Nearest { n, a, .. } = l { Some((number(&mut vars, &mut names, n, false), number(&mut vars, &mut names, a, false))) } else { None })
            .collect();
        let equal: Vec<(u32, u32)> = equalities.iter().map(|(a, b)| (number(&mut vars, &mut names, a, false), number(&mut vars, &mut names, b, false))).collect();
        let splice_nodes: Vec<u32> = placed.splices.iter().map(|(n, _)| number(&mut vars, &mut names, n, false)).collect();
        let n = vars.len();
        // Each variable's class: its root under the equalities.
        let mut root: Vec<u32> = (0..n as u32).collect();
        for (a, b) in equal {
            join(&mut root, a, b);
        }
        let class: Vec<u32> = (0..n as u32).map(|v| find(&mut root, v)).collect();
        // The value tapes, numbered after the variables, by the name of their class.
        let mut tapes: FxHashMap<String, u32> = FxHashMap::default();
        let mut tape = |t: &str| -> u32 {
            let k = n as u32 + tapes.len() as u32;
            *tapes.entry(t.to_owned()).or_insert(k)
        };
        let splices: Vec<(u32, u32)> = placed.splices.iter().zip(&splice_nodes).map(|((_, t), &v)| (class[v as usize], tape(t))).collect();
        let mut lits = Vec::with_capacity(body.len());
        for (index, (l, args)) in body.iter().zip(lit_args).enumerate() {
            lits.push(match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => {
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
                        rel: addr(a.rel.as_str()),
                        args,
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
        Form { names, class, lits, splices, ids: binders as usize + body.len(), binders }
    }

    /// The variable a head argument is, when the body names it.
    fn var(&self, t: &Term) -> Option<u32> {
        let Term::Var(v) = t else { return None };
        let a = addr(v.as_str());
        self.names.iter().position(|&n| n == a).map(|i| i as u32)
    }

    /// Whether every read is a constant: a constant relation (`constant`, by number), an input with one value for
    /// the run, or a fact; comparisons aside, nothing else (`split_constants`).
    pub(super) fn reads_constants(&self, rels: &Rels<'_>, constant: &[bool]) -> bool {
        self.lits.iter().all(|l| match l {
            Lit::Read { rel, .. } => {
                let r = rels.of_addr(*rel) as usize;
                constant[r] || rels.run_constant[r] || rels.fact[r]
            }
            Lit::Cmp => true,
            _ => false,
        })
    }

    /// The classes the body's positive reads place, under the location columns `locs` (by derived relation).
    pub(super) fn placed_classes(&self, rels: &Rels<'_>, locs: &[Vec<usize>]) -> Bits {
        let mut out = Bits::new(self.ids);
        for l in &self.lits {
            if let Lit::Read { neg: false, rel, args, .. } = l {
                let r = rels.of_addr(*rel);
                let columns = if rels.is_derived(r) { &locs[r as usize] } else { &rels.base_locs[r as usize] };
                for &c in columns {
                    if let Some(Some(v)) = args.get(c) {
                        out.set(self.class[*v as usize]);
                    }
                }
            }
        }
        out
    }

    /// Whether `head`'s column `c` is a variable of a class in `placed`.
    pub(super) fn head_placed(&self, head: &Atom, c: usize, placed: &Bits) -> bool {
        head.args.get(c).and_then(|t| self.var(t)).is_some_and(|v| placed.has(self.class[v as usize]))
    }

    /// Whether the rule is a string filter's (decision 0037): every literal a text test, a filter (`filters`, by
    /// derived relation), a positive read of a text relation or a comparison, and every head argument a variable
    /// read from a text column.
    pub(super) fn filters(&self, head: &Atom, rels: &Rels<'_>, filters: &[bool]) -> bool {
        let mut strings = Bits::new(self.ids);
        let reads_only_text = self.lits.iter().all(|l| match l {
            Lit::Read { plain: true, neg, rel, args, .. } => {
                let r = rels.of_addr(*rel);
                if rels.test[r as usize] || (rels.is_derived(r) && filters[r as usize]) {
                    true
                } else if let Some(cols) = rels.text[r as usize].filter(|_| !neg) {
                    cols.iter().filter_map(|&c| args.get(c).copied().flatten()).for_each(|v| strings.set(v));
                    true
                } else {
                    false
                }
            }
            Lit::Cmp => true,
            _ => false,
        });
        reads_only_text && head.args.iter().all(|t| self.var(t).is_some_and(|v| strings.has(v)))
    }

    /// Whether the rule's reads place one document (`split_in`'s rule check, without its explanation): `head` the
    /// head's relation, `placeless` by relation, `local`, `filters` and `locs` by derived relation.
    pub(super) fn one_document(&self, head: u32, rels: &Rels<'_>, placeless: &[bool], local: &[bool], filters: &[bool], locs: &[Vec<usize>]) -> bool {
        let head_filter = filters[head as usize];
        let mut root: Vec<u32> = (0..self.ids as u32).collect();
        // Each group's first id: groups sharing an id are one document.
        let mut firsts: Vec<u32> = Vec::new();
        for &(n, t) in &self.splices {
            join(&mut root, n, t);
            firsts.push(n);
        }
        let mut group: Vec<u32> = Vec::new();
        for (index, l) in self.lits.iter().enumerate() {
            match l {
                Lit::Read { plain, rel, args, binder, tapes, .. } => {
                    let r = rels.of_addr(*rel);
                    let derived = rels.is_derived(r);
                    if placeless[r as usize] || (derived && filters[r as usize]) || head_filter {
                        continue;
                    }
                    if derived && !local[r as usize] {
                        return false;
                    }
                    let columns = if derived { &locs[r as usize] } else { &rels.base_locs[r as usize] };
                    group.clear();
                    group.extend(columns.iter().filter_map(|&c| args.get(c).copied().flatten().map(|v| self.class[v as usize])));
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

/// A set of a body's ids.
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
