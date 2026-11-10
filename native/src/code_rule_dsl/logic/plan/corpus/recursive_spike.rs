//! Spike (O125, decision 0090's open maintenance choice): exact maintenance of the recursive strata, compared with
//! delete-and-rederive on the same data. `RERAIL_SPIKE_RECURSIVE=1` runs it in a session step, after the cross-file
//! part: for each recursive component, its old rows (the kept relations), its inputs' old and new rows, and the rows
//! the engine derived, which every algorithm must reproduce.
//!
//! Four algorithms run in one small evaluator here (not the engine's), so they are compared with each other on
//! equal terms: the rows rule firings emit, CPU time, and the state each keeps between runs.
//!
//! - **recompute**: the component derived from scratch over the new inputs.
//! - **DRed**: over-delete everything a deleted row reaches, rederive one step from the survivors, insert.
//! - **levels** (counting over cycles): each row keeps a level (a semi-naive round it is derivable in) and a count
//!   of its derivations whose recursive premises all have lower levels. A cycle cannot hold itself up, because a
//!   row's counted derivations reach the inputs through strictly falling levels. A deletion takes a row out only
//!   when its count reaches zero; those rows are rederived one step from the survivors, and insertion follows. Each
//!   derivation is counted once, in the order its changed premises are processed.
//! - **B/F** (backward/forward, Motik et al. 2015): before a deletion propagates past a row, the row is checked
//!   by backward chaining: the rows its derivations could rest on, then forward from the new inputs over those, so it
//!   is kept only with a proof that reaches the inputs. No state is kept between runs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rustc_hash::FxHashMap;

/// `RERAIL_SPIKE_RECURSIVE_CARRY`: levels' witness kept from run to run, by component, rows by value.
type Carried = FxHashMap<String, Vec<(Vec<i64>, u32, u32)>>;
static CARRY: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Carried>>> = std::sync::LazyLock::new(Default::default);

use super::retain::Retained;
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::front::{self, Literal, Program, Term};
use crate::code_rule_dsl::logic::native::component::InRel;

pub(in crate::code_rule_dsl) fn on() -> bool {
    std::env::var_os("RERAIL_SPIKE_RECURSIVE").is_some()
}

fn cpu() -> f64 {
    let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `now` is a valid timespec for the call to fill.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };
    now.tv_sec as f64 + now.tv_nsec as f64 * 1e-9
}

type Rows = Vec<Vec<i64>>;

fn rows_of(source: &dyn Source) -> Rows {
    let width = source.columns().len();
    (0..source.len()).map(|i| (0..width).map(|c| source.value(i, c)).collect()).collect()
}

fn rel_rows(rel: &Arc<InRel>, width: usize) -> Rows {
    rows_of(rel.source((0..width).map(|c| format!("c{c}")).collect()).as_ref())
}

/// The recursive components of `program`.
fn recursive(program: &Program) -> Vec<BTreeSet<String>> {
    let Ok((sccs, _)) = front::strata(program) else { return Vec::new() };
    sccs.into_iter()
        .filter(|scc| {
            scc.len() > 1
                || program.rules.iter().any(|r| scc.contains(&r.head.rel) && r.body.iter().any(|l| matches!(l, Literal::Atom(a) if scc.contains(&a.rel))))
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// The evaluator.

const OLD: u8 = 1;
const NEW: u8 = 2;

/// A relation's rows (every row any algorithm saw, by id), its inputs' old/new marks, and the indexes the plans read.
struct Store {
    width: usize,
    rows: Vec<i64>,
    ids: FxHashMap<Vec<i64>, u32>,
    io: Vec<u8>,
    idx: Vec<(u64, Vec<usize>, FxHashMap<Vec<i64>, Vec<u32>>)>,
}

impl Store {
    fn new(width: usize) -> Self {
        Store { width, rows: Vec::new(), ids: FxHashMap::default(), io: Vec::new(), idx: Vec::new() }
    }
    fn len(&self) -> u32 {
        self.io.len() as u32
    }
    fn row(&self, id: u32) -> &[i64] {
        &self.rows[id as usize * self.width..][..self.width]
    }
    fn id(&self, row: &[i64]) -> Option<u32> {
        self.ids.get(row).copied()
    }
    fn add(&mut self, row: &[i64], io: u8) -> u32 {
        if let Some(&id) = self.ids.get(row) {
            self.io[id as usize] |= io;
            return id;
        }
        let id = self.len();
        self.rows.extend_from_slice(row);
        self.ids.insert(row.to_vec(), id);
        self.io.push(io);
        for (_, cols, map) in &mut self.idx {
            map.entry(cols.iter().map(|&c| row[c]).collect()).or_default().push(id);
        }
        id
    }
    fn index(&mut self, mask: u64) {
        if mask == 0 || self.idx.iter().any(|(m, _, _)| *m == mask) {
            return;
        }
        let cols: Vec<usize> = (0..self.width).filter(|c| mask >> c & 1 == 1).collect();
        let mut map: FxHashMap<Vec<i64>, Vec<u32>> = FxHashMap::default();
        for id in 0..self.len() {
            let row = self.row(id);
            map.entry(cols.iter().map(|&c| row[c]).collect()).or_default().push(id);
        }
        self.idx.push((mask, cols, map));
    }
    fn lookup(&self, mask: u64, key: &[i64]) -> &[u32] {
        let (_, _, map) = self.idx.iter().find(|(m, _, _)| *m == mask).expect("an index built for every plan");
        map.get(key).map_or(&[], Vec::as_slice)
    }
}

#[derive(Clone, Copy, Debug)]
enum Arg {
    Slot(usize),
    Const(i64),
    Wild,
}

#[derive(Clone, Debug)]
enum Head {
    Slot(usize),
    Const(i64),
    Cat(Box<Head>, Box<Head>),
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Clone, Debug)]
enum Lit {
    Atom { rel: usize, args: Vec<Arg>, neg: bool },
    Cmp { l: Arg, op: Op, r: Arg },
}

/// A step of a plan: an atom probed by its bound columns, a negation, a comparison, or `$x = …` binding a slot.
#[derive(Clone, Debug)]
enum Step {
    Pos { lit: usize, rel: usize, mask: u64, key: Vec<Arg>, binds: Vec<(usize, usize)>, eqs: Vec<(usize, usize)> },
    Neg { lit: usize, rel: usize, mask: u64, key: Vec<Arg>, eqs: Vec<(usize, usize)> },
    Cmp { l: Arg, op: Op, r: Arg },
    Assign { slot: usize, from: Arg },
}

struct Rule {
    head_rel: usize,
    head: Vec<Head>,
    lits: Vec<Lit>,
    slots: usize,
    /// Plans: `[0]` unseeded, `[1]` the head bound, `[2 + i]` literal `i` seeded.
    plans: Vec<Vec<Step>>,
    text: String,
}

/// Where a firing starts.
#[derive(Clone, Copy)]
enum Seed<'a> {
    None,
    Head(&'a [i64]),
    Lit(usize, u32),
}

/// One firing's view: whether row `id` of `rel` counts at literal `lit` (for a negation: whether it blocks).
type Vis<'a> = &'a dyn Fn(usize, usize, u32) -> bool;

struct Eval<'c> {
    stores: Vec<Store>,
    names: Vec<String>,
    rules: Vec<Rule>,
    cat: &'c dyn Fn(i64, i64) -> i64,
    /// Rows emitted by firings.
    emitted: std::cell::Cell<u64>,
    /// `RERAIL_SPIKE_RECURSIVE_PROFILE`: CPU seconds, firings, rows probed and premises visibility asked, by (rule, plan).
    profile: Option<std::cell::RefCell<FxHashMap<(usize, usize), (f64, u64, u64, f64)>>>,
    /// Rows probed (index hits or scanned) by firings.
    probes: std::cell::Cell<u64>,
}

fn compare(op: Op, a: i64, b: i64) -> bool {
    match op {
        Op::Lt => a < b,
        Op::Le => a <= b,
        Op::Gt => a > b,
        Op::Ge => a >= b,
        Op::Eq => a == b,
        Op::Ne => a != b,
    }
}

impl Eval<'_> {
    fn arg(slots: &[i64], a: Arg) -> i64 {
        match a {
            Arg::Slot(s) => slots[s],
            Arg::Const(c) => c,
            Arg::Wild => unreachable!("a wildcard is not a value"),
        }
    }

    fn head_value(&self, slots: &[i64], h: &Head) -> i64 {
        match h {
            Head::Slot(s) => slots[*s],
            Head::Const(c) => *c,
            Head::Cat(a, b) => (self.cat)(self.head_value(slots, a), self.head_value(slots, b)),
        }
    }

    /// Fires rule `r` from `seed` over `vis`; `emit` gets each derivation (head row, its positive premises) and
    /// returns whether to go on.
    fn emitted(&self) -> u64 {
        self.emitted.get()
    }

    fn fire(&self, r: usize, seed: Seed, vis: Vis, emit: &mut dyn FnMut(&[i64], &[(usize, u32)]) -> bool) {
        let rule = &self.rules[r];
        let mut slots = vec![0i64; rule.slots];
        let mut prem: Vec<(usize, u32)> = Vec::new();
        let plan = match seed {
            Seed::None => 0,
            Seed::Head(row) => {
                for (h, v) in rule.head.iter().zip(row) {
                    match h {
                        Head::Slot(s) => slots[*s] = *v,
                        Head::Const(c) if c != v => return,
                        _ => {}
                    }
                }
                // A slot named twice in the head: the same value.
                for (i, h) in rule.head.iter().enumerate() {
                    if let Head::Slot(s) = h
                        && slots[*s] != row[i]
                    {
                        return;
                    }
                }
                1
            }
            Seed::Lit(i, id) => {
                let Lit::Atom { rel, args, neg } = &rule.lits[i] else { unreachable!("a seed is an atom") };
                let row = self.stores[*rel].row(id);
                let mut set = vec![false; rule.slots];
                for (a, v) in args.iter().zip(row) {
                    match a {
                        Arg::Slot(s) if set[*s] => {
                            if slots[*s] != *v {
                                return;
                            }
                        }
                        Arg::Slot(s) => {
                            slots[*s] = *v;
                            set[*s] = true;
                        }
                        Arg::Const(c) if c != v => return,
                        _ => {}
                    }
                }
                if !neg {
                    prem.push((*rel, id));
                }
                2 + i
            }
        };
        let head_row = if let Seed::Head(row) = seed { Some(row.to_vec()) } else { None };
        let steps = &rule.plans[plan];
        let mut emitted = 0u64;
        let (c0, p0) = if self.profile.is_some() { (cpu(), self.probes.get()) } else { (0.0, 0) };
        self.go(r, steps, 0, &mut slots, &mut prem, vis, head_row.as_deref(), emit, &mut emitted);
        self.emitted.set(self.emitted.get() + emitted);
        if let Some(p) = &self.profile {
            let t = cpu() - c0;
            if t > 2.0 {
                eprintln!("recursive spike:     slow firing {t:.2} s ({emitted} emitted, plan {plan}) of {}: {:?}", rule.text, steps);
            }
            let mut p = p.borrow_mut();
            let e = p.entry((r, plan)).or_default();
            e.0 += t;
            e.1 += 1;
            e.2 += self.probes.get() - p0;
            e.3 += emitted as f64;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn go(
        &self,
        r: usize,
        steps: &[Step],
        k: usize,
        slots: &mut [i64],
        prem: &mut Vec<(usize, u32)>,
        vis: Vis,
        head_row: Option<&[i64]>,
        emit: &mut dyn FnMut(&[i64], &[(usize, u32)]) -> bool,
        emitted: &mut u64,
    ) -> bool {
        let Some(step) = steps.get(k) else {
            let rule = &self.rules[r];
            let head: Vec<i64> = rule.head.iter().map(|h| self.head_value(slots, h)).collect();
            if head_row.is_some_and(|want| want != head.as_slice()) {
                return true;
            }
            *emitted += 1;
            return emit(&head, prem);
        };
        match step {
            Step::Cmp { l, op, r: rr } => {
                if compare(*op, Self::arg(slots, *l), Self::arg(slots, *rr)) {
                    return self.go(r, steps, k + 1, slots, prem, vis, head_row, emit, emitted);
                }
                true
            }
            Step::Assign { slot, from } => {
                slots[*slot] = Self::arg(slots, *from);
                self.go(r, steps, k + 1, slots, prem, vis, head_row, emit, emitted)
            }
            Step::Neg { lit, rel, mask, key, eqs } => {
                let store = &self.stores[*rel];
                let blocked = |id: u32| {
                    self.probes.set(self.probes.get() + 1);
                    let row = store.row(id);
                    eqs.iter().all(|(a, b)| row[*a] == row[*b]) && vis(*lit, *rel, id)
                };
                let any = if *mask == 0 {
                    (0..store.len()).any(blocked)
                } else {
                    let kv: Vec<i64> = key.iter().map(|a| Self::arg(slots, *a)).collect();
                    store.lookup(*mask, &kv).iter().any(|&id| blocked(id))
                };
                if any {
                    return true;
                }
                self.go(r, steps, k + 1, slots, prem, vis, head_row, emit, emitted)
            }
            Step::Pos { lit, rel, mask, key, binds, eqs } => {
                let store = &self.stores[*rel];
                let mut each = |id: u32, slots: &mut [i64], prem: &mut Vec<(usize, u32)>| -> bool {
                    self.probes.set(self.probes.get() + 1);
                    if !vis(*lit, *rel, id) {
                        return true;
                    }
                    let row = store.row(id);
                    if !eqs.iter().all(|(a, b)| row[*a] == row[*b]) {
                        return true;
                    }
                    for (c, s) in binds {
                        slots[*s] = row[*c];
                    }
                    prem.push((*rel, id));
                    let go_on = self.go(r, steps, k + 1, slots, prem, vis, head_row, emit, emitted);
                    prem.pop();
                    go_on
                };
                if *mask == 0 {
                    for id in 0..store.len() {
                        if !each(id, slots, prem) {
                            return false;
                        }
                    }
                } else {
                    let kv: Vec<i64> = key.iter().map(|a| Self::arg(slots, *a)).collect();
                    for &id in store.lookup(*mask, &kv) {
                        if !each(id, slots, prem) {
                            return false;
                        }
                    }
                }
                true
            }
        }
    }
}

/// A rule in the evaluator's form, `None` (with why) when it uses what the spike does not evaluate.
/// `fanout(rel, mask)`: the rows a probe of `rel` by the columns in `mask` finds on average (all of them for 0).
fn compile(rule: &front::Rule, rel_of: &dyn Fn(&str) -> Option<usize>, fanout: &dyn Fn(usize, u64) -> f64) -> Result<Rule, String> {
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    let slot = |v: &str, names: &mut BTreeMap<String, usize>| {
        let n = names.len();
        *names.entry(v.to_owned()).or_insert(n)
    };
    let arg = |t: &Term, names: &mut BTreeMap<String, usize>| -> Result<Arg, String> {
        Ok(match t {
            Term::Var(v) => Arg::Slot(slot(v, names)),
            Term::Wild => Arg::Wild,
            Term::Int(i) => Arg::Const(*i),
            other => return Err(format!("term {other:?}")),
        })
    };
    let mut lits = Vec::new();
    for l in &rule.body {
        lits.push(match l {
            Literal::Atom(a) => {
                let rel = rel_of(&a.rel).ok_or_else(|| format!("no rows of {}", a.rel))?;
                if a.gated {
                    return Err("a gated negation".into());
                }
                Lit::Atom { rel, args: a.args.iter().map(|t| arg(t, &mut names)).collect::<Result<_, _>>()?, neg: a.neg }
            }
            Literal::Cmp { l, op, r } => {
                let op = match *op {
                    "<" => Op::Lt,
                    "<=" => Op::Le,
                    ">" => Op::Gt,
                    ">=" => Op::Ge,
                    "=" | "==" => Op::Eq,
                    "!=" => Op::Ne,
                    o => return Err(format!("comparison {o}")),
                };
                Lit::Cmp { l: arg(l, &mut names)?, op, r: arg(r, &mut names)? }
            }
            other => return Err(format!("literal {other}")),
        });
    }
    fn head_term(t: &Term, names: &BTreeMap<String, usize>) -> Result<Head, String> {
        Ok(match t {
            Term::Var(v) => Head::Slot(*names.get(v).ok_or_else(|| format!("head variable {v} unbound"))?),
            Term::Int(i) => Head::Const(*i),
            Term::Call(f, args) if f == "cat" && args.len() == 2 => Head::Cat(Box::new(head_term(&args[0], names)?), Box::new(head_term(&args[1], names)?)),
            other => return Err(format!("head term {other:?}")),
        })
    }
    let head = rule.head.args.iter().map(|t| head_term(t, &names)).collect::<Result<Vec<_>, _>>()?;
    let head_rel = rel_of(&rule.head.rel).ok_or("no head relation")?;
    let slots = names.len();
    let head_slots: BTreeSet<usize> = head
        .iter()
        .flat_map(|h| {
            let mut out = Vec::new();
            fn walk(h: &Head, out: &mut Vec<usize>) {
                match h {
                    Head::Slot(s) => out.push(*s),
                    Head::Cat(a, b) => {
                        walk(a, out);
                        walk(b, out);
                    }
                    Head::Const(_) => {}
                }
            }
            walk(h, &mut out);
            out
        })
        .collect();
    let lit_slots = |l: &Lit| -> Vec<usize> {
        match l {
            Lit::Atom { args, .. } => args.iter().filter_map(|a| if let Arg::Slot(s) = a { Some(*s) } else { None }).collect(),
            Lit::Cmp { l, r, .. } => [l, r].iter().filter_map(|a| if let Arg::Slot(s) = a { Some(*s) } else { None }).collect(),
        }
    };
    // A plan: the literals in a greedy order from what the seed binds.
    let plan = |bound0: BTreeSet<usize>, skip: Option<usize>| -> Result<Vec<Step>, String> {
        let mut bound = bound0;
        let mut left: Vec<usize> = (0..lits.len()).filter(|i| Some(*i) != skip).collect();
        // Slots some positive atom left to place binds.
        let mut steps = Vec::new();
        while !left.is_empty() {
            let positive_slots: BTreeSet<usize> =
                left.iter().filter(|i| matches!(&lits[**i], Lit::Atom { neg: false, .. })).flat_map(|i| lit_slots(&lits[*i])).collect();
            // A comparison or negation whose slots are bound (a negation's unbound slots no positive atom binds are
            // wildcards); else `$x = bound`; else the positive atom with the most bound columns.
            let ready = left.iter().position(|&i| match &lits[i] {
                Lit::Cmp { .. } => lit_slots(&lits[i]).iter().all(|s| bound.contains(s)),
                Lit::Atom { neg: true, .. } => lit_slots(&lits[i]).iter().all(|s| bound.contains(s) || !positive_slots.contains(s)),
                _ => false,
            });
            if let Some(p) = ready {
                let i = left.remove(p);
                steps.push(match &lits[i] {
                    Lit::Cmp { l, op, r } => Step::Cmp { l: *l, op: *op, r: *r },
                    Lit::Atom { rel, args, .. } => {
                        let (mut mask, mut key, mut eqs, mut first) = (0u64, Vec::new(), Vec::new(), BTreeMap::new());
                        for (c, a) in args.iter().enumerate() {
                            match a {
                                Arg::Const(_) => {
                                    mask |= 1 << c;
                                    key.push(*a);
                                }
                                Arg::Slot(s) if bound.contains(s) => {
                                    mask |= 1 << c;
                                    key.push(*a);
                                }
                                Arg::Slot(s) => {
                                    if let Some(&f) = first.get(s) {
                                        eqs.push((f, c));
                                    } else {
                                        first.insert(*s, c);
                                    }
                                }
                                Arg::Wild => {}
                            }
                        }
                        Step::Neg { lit: i, rel: *rel, mask, key, eqs }
                    }
                });
                continue;
            }
            let assign = left.iter().position(|&i| match &lits[i] {
                Lit::Cmp { l: Arg::Slot(s), op: Op::Eq, r } => !bound.contains(s) && !matches!(r, Arg::Slot(t) if !bound.contains(t)),
                Lit::Cmp { l, op: Op::Eq, r: Arg::Slot(s) } => !bound.contains(s) && !matches!(l, Arg::Slot(t) if !bound.contains(t)),
                _ => false,
            });
            if let Some(p) = assign {
                let i = left.remove(p);
                let Lit::Cmp { l, r, .. } = &lits[i] else { unreachable!() };
                let (slot, from) = match (l, r) {
                    (Arg::Slot(s), from) if !bound.contains(s) => (*s, *from),
                    (from, Arg::Slot(s)) => (*s, *from),
                    _ => unreachable!(),
                };
                bound.insert(slot);
                steps.push(Step::Assign { slot, from });
                continue;
            }
            // The positive atom whose probe finds fewest rows, by the relation's measured rows per key (bound
            // columns alone mislead: `pv($a, $k, $v)` by `$k, $v` finds hundreds, `cfn($c, _, $f)` by `$c` one);
            // ties go to the earlier literal.
            let best = left
                .iter()
                .enumerate()
                .filter(|(_, i)| matches!(&lits[**i], Lit::Atom { neg: false, .. }))
                .map(|(p, i)| {
                    let Lit::Atom { rel, args, .. } = &lits[*i] else { unreachable!() };
                    let mask = args.iter().enumerate().filter(|(_, a)| matches!(a, Arg::Const(_)) || matches!(a, Arg::Slot(s) if bound.contains(s))).fold(0u64, |m, (c, _)| m | 1 << c);
                    (p, fanout(*rel, mask))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
                .map(|(p, _)| p)
                .ok_or_else(|| format!("no literal can be placed in {}", rule))?;
            let i = left.remove(best);
            let Lit::Atom { rel, args, .. } = &lits[i] else { unreachable!() };
            let (mut mask, mut key, mut binds, mut eqs, mut first) = (0u64, Vec::new(), Vec::new(), Vec::new(), BTreeMap::new());
            for (c, a) in args.iter().enumerate() {
                match a {
                    Arg::Const(_) => {
                        mask |= 1 << c;
                        key.push(*a);
                    }
                    Arg::Slot(s) if bound.contains(s) => {
                        mask |= 1 << c;
                        key.push(*a);
                    }
                    Arg::Slot(s) => {
                        if let Some(&f) = first.get(s) {
                            eqs.push((f, c));
                        } else {
                            first.insert(*s, c);
                            binds.push((c, *s));
                        }
                    }
                    Arg::Wild => {}
                }
            }
            for (_, s) in &binds {
                bound.insert(*s);
            }
            steps.push(Step::Pos { lit: i, rel: *rel, mask, key, binds, eqs });
        }
        if let Some(s) = head_slots.iter().find(|s| !bound.contains(s)) {
            return Err(format!("head slot {s} unbound in {rule}"));
        }
        Ok(steps)
    };
    let head_bound: BTreeSet<usize> = rule
        .head
        .args
        .iter()
        .filter_map(|t| if let Term::Var(v) = t { names.get(v).copied() } else { None })
        .collect();
    let mut plans = vec![plan(BTreeSet::new(), None)?, plan(head_bound, None)?];
    // A negated seed (a row of the negated relation gone or new) binds the slots the rule binds elsewhere, and keeps
    // its literal: another row of that relation can still block the negation (`!opaque_dec($f, _)`).
    let bound_slots: BTreeSet<usize> = lits.iter().filter(|l| matches!(l, Lit::Atom { neg: false, .. })).flat_map(|l| lit_slots(l)).collect();
    for (i, l) in lits.iter().enumerate() {
        plans.push(match l {
            Lit::Atom { neg: true, .. } => plan(lit_slots(l).into_iter().filter(|s| bound_slots.contains(s)).collect(), None)?,
            Lit::Atom { .. } => plan(lit_slots(l).into_iter().collect(), Some(i))?,
            Lit::Cmp { .. } => Vec::new(),
        });
    }
    Ok(Rule { head_rel, head, lits, slots, plans, text: rule.to_string() })
}

// ---------------------------------------------------------------------------------------------------------------
// The algorithms. Each keeps its own marks per row (`st[rel][id]`), and the stores are shared.

const P: u8 = 1; // present
const D: u8 = 2; // deleted (DRed: over-deleted; levels: count reached zero; B/F: disproved)
const X: u8 = 4; // processed as a deletion seed
const Y: u8 = 8; // processed as an insertion seed
const N: u8 = 16; // inserted or rederived this run
const PR: u8 = 32; // proved (B/F)
const CK: u8 = 64; // in the check being made (B/F)

#[derive(Default, Clone, Debug)]
struct Report {
    name: &'static str,
    cpu: f64,
    /// Rows emitted, by phase.
    phases: Vec<(&'static str, u64)>,
    /// Rows taken out before a rederive or insert (DRed's over-deletion), and rederived.
    deleted: usize,
    rederived: usize,
    /// Rows checked backward (B/F).
    checked: usize,
    /// Bytes kept between runs.
    kept_bytes: usize,
    /// Rows that differ from the engine's (extra, missing), and anomalies.
    extra: usize,
    missing: usize,
    anomalies: usize,
}

struct Component<'c> {
    ev: Eval<'c>,
    scc: Vec<usize>,
    is_scc: Vec<bool>,
    /// By relation: (rule, literal) positions that read it, positive or negated.
    reads: Vec<Vec<(usize, usize, bool)>>,
    /// The engine's rows after, by relation of the component.
    want: Vec<std::collections::HashSet<Vec<i64>>>,
    /// The old rows every algorithm starts from: the component derived over the old inputs.
    old_p: Vec<Vec<u8>>,
    /// Inputs' rows gone and new.
    gone: Vec<(usize, u32)>,
    came: Vec<(usize, u32)>,
}

impl Component<'_> {
    fn state(&self) -> Vec<Vec<u8>> {
        self.ev.stores.iter().map(|s| vec![0; s.len() as usize]).collect()
    }

    fn grow(st: &mut [Vec<u8>], ev: &Eval, rel: usize) {
        let n = ev.stores[rel].len() as usize;
        if st[rel].len() < n {
            st[rel].resize(n, 0);
        }
    }

    fn check(&self, st: &[Vec<u8>], r: &mut Report) {
        for &rel in &self.scc {
            let store = &self.ev.stores[rel];
            let have: std::collections::HashSet<Vec<i64>> = (0..store.len()).filter(|&id| st[rel].get(id as usize).is_some_and(|s| s & P != 0)).map(|id| store.row(id).to_vec()).collect();
            let extra: Vec<&Vec<i64>> = have.difference(&self.want[rel]).collect();
            let missing: Vec<&Vec<i64>> = self.want[rel].difference(&have).collect();
            if (!extra.is_empty() || !missing.is_empty()) && std::env::var_os("RERAIL_SPIKE_RECURSIVE_DIFF").is_some() {
                eprintln!("recursive spike:     {} {}: +{} -{}; e.g. +{:?} -{:?}", r.name, self.ev.names[rel], extra.len(), missing.len(), extra.first(), missing.first());
                for rule in self.ev.rules.iter().filter(|x| x.head_rel == rel) {
                    eprintln!("recursive spike:       rule {}", rule.text);
                }
            }
            if std::env::var_os("RERAIL_SPIKE_RECURSIVE_DIFF").is_some() {
                for row in extra.iter().take(2) {
                    self.explain(rel, row, "extra");
                    self.explain_in(st, rel, row, NEW, "the algorithm's rows");
                    self.explain_in(&self.old_p, rel, row, OLD, "the old rows");
                }
                for row in missing.iter().take(2) {
                    self.explain(rel, row, "missing");
                }
            }
            r.extra += extra.len();
            r.missing += missing.len();
        }
    }

    /// What holds row `row` of `rel` up in an algorithm's rows `st`: a derivation over them, each premise marked
    /// with whether the reference has it, and for an input whether it is old, new or both.
    fn explain_in(&self, st: &[Vec<u8>], rel: usize, row: &[i64], io: u8, what: &str) {
        let (is_scc, stores) = (&self.is_scc, &self.ev.stores);
        let vis = |_: usize, r: usize, id: u32| if is_scc[r] { st[r].get(id as usize).is_some_and(|s| s & P != 0) } else { stores[r].io[id as usize] & io != 0 };
        for r in (0..self.ev.rules.len()).filter(|r| self.ev.rules[*r].head_rel == rel) {
            let mut first: Option<Vec<(usize, u32)>> = None;
            self.ev.fire(r, Seed::Head(row), &vis, &mut |_, p| {
                first = Some(p.to_vec());
                false
            });
            if let Some(p) = first {
                let premises: Vec<String> = p
                    .iter()
                    .map(|&(r2, id)| {
                        let x = stores[r2].row(id);
                        let mark = if is_scc[r2] { if self.want[r2].contains(x) { "in reference" } else { "NOT in reference" } } else { match stores[r2].io[id as usize] { 1 => "input old", 2 => "input new", _ => "input both" } };
                        format!("{}{x:?} ({mark})", self.ev.names[r2])
                    })
                    .collect();
                eprintln!("recursive spike:       held up in {what} by {} from {premises:?}", self.ev.rules[r].text);
                return;
            }
        }
        eprintln!("recursive spike:       not derivable in {what}");
    }

    /// Why row `row` of `rel` differs: a derivation of it over the new inputs and the engine's rows of the
    /// component, or, with none, each rule's literals that find no row.
    fn explain(&self, rel: usize, row: &[i64], what: &str) {
        let wanted: Vec<Vec<bool>> = self.ev.stores.iter().enumerate().map(|(r, s)| (0..s.len()).map(|id| self.is_scc[r] && self.want[r].contains(s.row(id))).collect()).collect();
        let (is_scc, stores) = (&self.is_scc, &self.ev.stores);
        let vis = |_: usize, r: usize, id: u32| if is_scc[r] { wanted[r].get(id as usize).copied().unwrap_or(false) } else { stores[r].io[id as usize] & NEW != 0 };
        let mut shown = false;
        for r in (0..self.ev.rules.len()).filter(|r| self.ev.rules[*r].head_rel == rel) {
            let mut first: Option<Vec<(usize, u32)>> = None;
            self.ev.fire(r, Seed::Head(row), &vis, &mut |_, p| {
                first = Some(p.to_vec());
                false
            });
            if let Some(p) = first {
                let premises: Vec<String> = p.iter().map(|&(r2, id)| format!("{}{:?}", self.ev.names[r2], stores[r2].row(id))).collect();
                eprintln!("recursive spike:       {what} {:?}: derived over the engine's rows by {} from {premises:?}", row, self.ev.rules[r].text);
                shown = true;
            }
        }
        if !shown {
            eprintln!("recursive spike:       {what} {:?}: no derivation over the engine's rows", row);
            // Each rule's atoms, with the head's values bound, and how many rows of each match alone.
            for rule in self.ev.rules.iter().filter(|x| x.head_rel == rel) {
                let mut bind: BTreeMap<usize, i64> = BTreeMap::new();
                for (h, v) in rule.head.iter().zip(row) {
                    if let Head::Slot(s) = h {
                        bind.insert(*s, *v);
                    }
                }
                let counts: Vec<String> = rule
                    .lits
                    .iter()
                    .filter_map(|l| match l {
                        Lit::Atom { rel: r2, args, neg } => {
                            let store = &stores[*r2];
                            let n = (0..store.len())
                                .filter(|&id| vis(0, *r2, id))
                                .filter(|&id| {
                                    let x = store.row(id);
                                    args.iter().zip(x).all(|(a, v)| match a {
                                        Arg::Slot(s) => bind.get(s).is_none_or(|b| b == v),
                                        Arg::Const(c) => c == v,
                                        Arg::Wild => true,
                                    })
                                })
                                .count();
                            Some(format!("{}{}:{n}", if *neg { "!" } else { "" }, self.ev.names[*r2]))
                        }
                        Lit::Cmp { .. } => None,
                    })
                    .collect();
                eprintln!("recursive spike:         {} -> {}", rule.text, counts.join(" "));
            }
        }
    }

    /// Seeds of a phase: an input row gone (at positive literals, `gone`) or new (at negated ones), or the other way.
    fn input_seeds(&self, deleting: bool) -> Vec<(usize, usize, u32)> {
        let mut out = Vec::new();
        for (list, positive) in [(&self.gone, deleting), (&self.came, !deleting)] {
            for &(rel, id) in list {
                for &(r, i, neg) in &self.reads[rel] {
                    if neg != positive {
                        out.push((r, i, id));
                    }
                }
            }
        }
        out
    }

    /// The (rule, literal) positions row `id` of `rel` seeds: in a deletion, an input row gone or a component row at
    /// positive literals and an input row new at negated ones; in an insertion, the other way for inputs.
    fn positions(&self, rel: usize, id: u32, deleting: bool) -> Vec<(usize, usize)> {
        let io = self.ev.stores[rel].io[id as usize];
        let positive = self.is_scc[rel] || (io & OLD != 0 && io & NEW == 0) == deleting;
        self.reads[rel].iter().filter(|(_, _, neg)| *neg != positive).map(|&(r, i, _)| (r, i)).collect()
    }

    fn scc_seeds(&self, rel: usize, id: u32) -> Vec<(usize, usize, u32)> {
        self.reads[rel].iter().filter(|(_, _, neg)| !neg).map(|&(r, i, _)| (r, i, id)).collect()
    }

    fn present(&self, st: &[Vec<u8>], rel: usize, id: u32) -> bool {
        st[rel].get(id as usize).is_some_and(|s| s & P != 0)
    }

    // -- recompute -------------------------------------------------------------------------------------------------

    /// The component from scratch over the inputs with mark `io`: its rows (`P`), each row's level (the round it was
    /// first derived in), and the rows emitted.
    fn derive(&mut self, io: u8) -> (Vec<Vec<u8>>, Vec<Vec<u32>>) {
        let mut st = self.state();
        let mut lv: Vec<Vec<u32>> = self.ev.stores.iter().map(|s| vec![u32::MAX; s.len() as usize]).collect();
        let rules: Vec<usize> = (0..self.ev.rules.len()).collect();
        let mut round = 0u32;
        let mut delta: Vec<(usize, u32)> = Vec::new();
        loop {
            let mut found: Vec<(usize, Vec<i64>)> = Vec::new();
            {
                let st_ref = &st;
                let is_scc = &self.is_scc;
                let stores = &self.ev.stores;
                let vis = |_: usize, rel: usize, id: u32| if is_scc[rel] { st_ref[rel].get(id as usize).is_some_and(|s| s & P != 0) } else { stores[rel].io[id as usize] & io != 0 };
                if round == 0 {
                    for &r in &rules {
                        let hr = self.ev.rules[r].head_rel;
                        self.ev.fire(r, Seed::None, &vis, &mut |h, _| {
                            found.push((hr, h.to_vec()));
                            true
                        });
                    }
                } else {
                    for &(rel, id) in &delta {
                        for (r, i, _) in self.scc_seeds(rel, id) {
                            let hr = self.ev.rules[r].head_rel;
                            self.ev.fire(r, Seed::Lit(i, id), &vis, &mut |h, _| {
                                found.push((hr, h.to_vec()));
                                true
                            });
                        }
                    }
                }
            }
            delta.clear();
            for (rel, row) in found {
                let id = self.ev.stores[rel].add(&row, 0);
                Self::grow(&mut st, &self.ev, rel);
                if lv[rel].len() < st[rel].len() {
                    lv[rel].resize(st[rel].len(), u32::MAX);
                }
                if st[rel][id as usize] & P == 0 {
                    st[rel][id as usize] |= P;
                    lv[rel][id as usize] = round;
                    delta.push((rel, id));
                }
            }
            if self.ev.profile.is_some() {
                eprintln!("recursive spike:     derive round {round}: {} new rows, {} emitted so far", delta.len(), self.ev.emitted());
            }
            if delta.is_empty() {
                break;
            }
            round += 1;
        }
        (st, lv)
    }

    /// The component from scratch over the new inputs, checked against the engine's rows; its rows become what the
    /// maintaining algorithms are checked against.
    fn recompute(&mut self) -> Report {
        let (c0, e0) = (cpu(), self.ev.emitted());
        let (st, _) = self.derive(NEW);
        let mut r = Report { name: "recompute", cpu: cpu() - c0, phases: vec![("derive", self.ev.emitted() - e0)], ..Default::default() };
        self.check(&st, &mut r);
        for &rel in &self.scc {
            let store = &self.ev.stores[rel];
            self.want[rel] = (0..store.len()).filter(|&id| st[rel].get(id as usize).is_some_and(|s| s & P != 0)).map(|id| store.row(id).to_vec()).collect();
        }
        r
    }

    /// The old rows present: the component derived over the old inputs.
    fn old_state(&self) -> Vec<Vec<u8>> {
        let mut st = self.old_p.clone();
        for (rel, s) in st.iter_mut().enumerate() {
            s.resize(self.ev.stores[rel].len() as usize, 0);
        }
        st
    }

    /// Insertion, semi-naive from `queue` (rows of the component made present) and the inputs' changes, over the
    /// present rows: a head not present is inserted. `on_new` gets each inserted row and the derivation's premises.
    fn insert(&mut self, st: &mut Vec<Vec<u8>>, mut queue: Vec<(usize, u32)>, on_new: &mut dyn FnMut(&mut Vec<Vec<u8>>, usize, u32, &[(usize, u32)])) {
        let mut seeds = self.input_seeds(false);
        loop {
            let mut found: Vec<(usize, Vec<i64>, Vec<(usize, u32)>)> = Vec::new();
            for &(rel, id) in &queue {
                seeds.extend(self.scc_seeds(rel, id));
            }
            queue.clear();
            if seeds.is_empty() {
                break;
            }
            {
                let st_ref = &*st;
                let is_scc = &self.is_scc;
                let stores = &self.ev.stores;
                let vis = |_: usize, rel: usize, id: u32| if is_scc[rel] { st_ref[rel].get(id as usize).is_some_and(|s| s & P != 0) } else { stores[rel].io[id as usize] & NEW != 0 };
                for (r, i, id) in seeds.drain(..) {
                    let hr = self.ev.rules[r].head_rel;
                    self.ev.fire(r, Seed::Lit(i, id), &vis, &mut |h, p| {
                        found.push((hr, h.to_vec(), p.to_vec()));
                        true
                    });
                }
            }
            for (rel, row, prem) in found {
                let id = self.ev.stores[rel].add(&row, 0);
                Self::grow(st, &self.ev, rel);
                if st[rel][id as usize] & P == 0 {
                    st[rel][id as usize] |= P | N;
                    on_new(st, rel, id, &prem);
                    queue.push((rel, id));
                }
            }
        }
    }

    // -- levels, checked ---------------------------------------------------------------------------------------------

    /// Levels without counts: each row keeps only a level, and every present row has a derivation from rows below
    /// it. A row that lost a derivation is checked in level order: it stays when a derivation from rows below it
    /// (all decided already) still holds. New rows take levels above every level held (`base` + the wave), so no
    /// firing need name its premises. Kept: 4 bytes a row.
    fn levels_check(&mut self) -> Report {
        let carry = std::env::var_os("RERAIL_SPIKE_RECURSIVE_CARRY").is_some();
        let key = format!("check:{}", self.key());
        let carried = if carry { CARRY.lock().unwrap().get(&key).cloned() } else { None };
        let (mut st, mut lv) = if let Some(map) = carried {
            let mut placed: Vec<(usize, u32, u32)> = Vec::new();
            for &rel in &self.scc.clone() {
                for (row, l, _) in map.get(&self.ev.names[rel]).into_iter().flatten() {
                    placed.push((rel, self.ev.stores[rel].add(row, 0), *l));
                }
            }
            let mut st = self.state();
            let mut lv: Vec<Vec<u32>> = self.ev.stores.iter().map(|s| vec![u32::MAX; s.len() as usize]).collect();
            for (rel, id, l) in placed {
                st[rel][id as usize] |= P;
                lv[rel][id as usize] = l;
            }
            if self.carried_fits(&st) {
                (st, lv)
            } else {
                eprintln!("recursive spike:   levels, checked: the carried state is stale, built again");
                let e = self.ev.emitted();
                let r = self.derive(OLD);
                self.ev.emitted.set(e);
                r
            }
        } else {
            let e = self.ev.emitted();
            let r = self.derive(OLD);
            self.ev.emitted.set(e);
            r
        };
        let (c0, e0) = (cpu(), self.ev.emitted());
        let mut base = 0u32;
        for &rel in &self.scc {
            for i in 0..st[rel].len() {
                if st[rel][i] & P != 0 {
                    base = base.max(lv[rel][i]);
                }
            }
        }
        // Candidates: heads of old derivations that lost a premise, by level.
        let mut heap: std::collections::BinaryHeap<std::cmp::Reverse<(u32, usize, u32)>> = Default::default();
        let old_heads = |c: &Self, st: &[Vec<u8>], seeds: Vec<(usize, usize, u32)>| -> Vec<(usize, u32)> {
            let mut out = Vec::new();
            let (is_scc, stores) = (&c.is_scc, &c.ev.stores);
            // The old rows: present before this run (deleted ones included).
            let vis = |_: usize, r: usize, i: u32| if is_scc[r] { st[r].get(i as usize).is_some_and(|s| s & (P | D) != 0) } else { stores[r].io[i as usize] & OLD != 0 };
            for (r, i, id) in seeds {
                let hr = c.ev.rules[r].head_rel;
                c.ev.fire(r, Seed::Lit(i, id), &vis, &mut |h, _| {
                    if let Some(hid) = c.ev.stores[hr].id(h) {
                        out.push((hr, hid));
                    }
                    true
                });
            }
            out
        };
        for (rel, id) in old_heads(self, &st, self.input_seeds(true)) {
            if st[rel].get(id as usize).is_some_and(|s| s & P != 0) {
                heap.push(std::cmp::Reverse((lv[rel][id as usize], rel, id)));
            }
        }
        let e_seed = self.ev.emitted();
        let (mut checked, mut deleted) = (0usize, Vec::new());
        while let Some(std::cmp::Reverse((l, rel, id))) = heap.pop() {
            if st[rel][id as usize] & (CK | D) != 0 {
                continue;
            }
            st[rel][id as usize] |= CK;
            checked += 1;
            let row = self.ev.stores[rel].row(id).to_vec();
            let mut held = false;
            {
                let (st_ref, lv_ref, is_scc, stores) = (&st, &lv, &self.is_scc, &self.ev.stores);
                let vis = |_: usize, r: usize, i: u32| {
                    if is_scc[r] {
                        st_ref[r].get(i as usize).is_some_and(|s| s & (P | D) == P) && lv_ref[r][i as usize] < l
                    } else {
                        stores[r].io[i as usize] & NEW != 0
                    }
                };
                for r in (0..self.ev.rules.len()).filter(|r| self.ev.rules[*r].head_rel == rel) {
                    self.ev.fire(r, Seed::Head(&row), &vis, &mut |_, _| {
                        held = true;
                        false
                    });
                    if held {
                        break;
                    }
                }
            }
            if held {
                continue;
            }
            st[rel][id as usize] |= D;
            deleted.push((rel, id));
            // Its consumers above it lose a derivation from below.
            for (hr, hid) in old_heads(self, &st, self.scc_seeds(rel, id)) {
                if lv[hr].get(hid as usize).is_some_and(|&hl| hl > l) && st[hr][hid as usize] & (P | D | CK) == P {
                    heap.push(std::cmp::Reverse((lv[hr][hid as usize], hr, hid)));
                }
            }
        }
        let e1 = self.ev.emitted();
        for &(rel, id) in &deleted {
            st[rel][id as usize] &= !P;
        }
        // Rederive one step from the survivors, then insert semi-naively; new rows above every level held.
        let mut queue = Vec::new();
        for &(rel, id) in &deleted {
            let row = self.ev.stores[rel].row(id).to_vec();
            let mut found = false;
            {
                let (st_ref, is_scc, stores) = (&st, &self.is_scc, &self.ev.stores);
                let vis = |_: usize, r: usize, i: u32| if is_scc[r] { st_ref[r].get(i as usize).is_some_and(|s| s & P != 0) } else { stores[r].io[i as usize] & NEW != 0 };
                for r in (0..self.ev.rules.len()).filter(|r| self.ev.rules[*r].head_rel == rel) {
                    self.ev.fire(r, Seed::Head(&row), &vis, &mut |_, _| {
                        found = true;
                        false
                    });
                    if found {
                        break;
                    }
                }
            }
            if found {
                queue.push((rel, id));
            }
        }
        let rederived = queue.len();
        for &(rel, id) in &queue {
            st[rel][id as usize] |= P | N;
            lv[rel][id as usize] = base + 1;
        }
        let e2 = self.ev.emitted();
        let mut seeds = self.input_seeds(false);
        let mut wave = 1u32;
        loop {
            for &(rel, id) in &queue {
                seeds.extend(self.scc_seeds(rel, id));
            }
            queue.clear();
            if seeds.is_empty() {
                break;
            }
            wave += 1;
            let mut found: Vec<(usize, Vec<i64>)> = Vec::new();
            {
                let (st_ref, is_scc, stores) = (&st, &self.is_scc, &self.ev.stores);
                let vis = |_: usize, r: usize, i: u32| if is_scc[r] { st_ref[r].get(i as usize).is_some_and(|s| s & P != 0) } else { stores[r].io[i as usize] & NEW != 0 };
                for (r, i, id) in seeds.drain(..) {
                    let hr = self.ev.rules[r].head_rel;
                    self.ev.fire(r, Seed::Lit(i, id), &vis, &mut |h, _| {
                        found.push((hr, h.to_vec()));
                        true
                    });
                }
            }
            for (rel, row) in found {
                let id = self.ev.stores[rel].add(&row, 0);
                Self::grow(&mut st, &self.ev, rel);
                if lv[rel].len() < st[rel].len() {
                    lv[rel].resize(st[rel].len(), u32::MAX);
                }
                if st[rel][id as usize] & P == 0 {
                    st[rel][id as usize] |= P | N;
                    lv[rel][id as usize] = base + wave;
                    queue.push((rel, id));
                }
            }
        }
        let e3 = self.ev.emitted();
        let spent = cpu() - c0;
        // Not timed: every present row has a derivation from rows below it.
        let mut anomalies = 0usize;
        if std::env::var_os("RERAIL_SPIKE_RECURSIVE_VERIFY").is_some() {
            for &rel in &self.scc {
                for id in 0..st[rel].len() as u32 {
                    if st[rel][id as usize] & P == 0 {
                        continue;
                    }
                    let l = lv[rel][id as usize];
                    let row = self.ev.stores[rel].row(id).to_vec();
                    let (st_ref, lv_ref, is_scc, stores) = (&st, &lv, &self.is_scc, &self.ev.stores);
                    let vis = |_: usize, r: usize, i: u32| if is_scc[r] { st_ref[r].get(i as usize).is_some_and(|s| s & P != 0) && lv_ref[r][i as usize] < l } else { stores[r].io[i as usize] & NEW != 0 };
                    let mut held = false;
                    for r in (0..self.ev.rules.len()).filter(|r| self.ev.rules[*r].head_rel == rel) {
                        self.ev.fire(r, Seed::Head(&row), &vis, &mut |_, _| {
                            held = true;
                            false
                        });
                    }
                    anomalies += usize::from(!held);
                }
            }
            self.ev.emitted.set(e3);
        }
        let rows: usize = self.scc.iter().map(|&rel| st[rel].iter().filter(|s| **s & P != 0).count()).sum();
        let mut r = Report {
            name: "levels, checked",
            cpu: spent,
            phases: vec![("seed", e_seed - e0), ("check", e1 - e_seed), ("rederive", e2 - e1), ("insert", e3 - e2)],
            deleted: deleted.len(),
            rederived,
            checked,
            kept_bytes: rows * 4,
            anomalies,
            ..Default::default()
        };
        self.check(&st, &mut r);
        if carry {
            let mut map: FxHashMap<String, Vec<(Vec<i64>, u32, u32)>> = FxHashMap::default();
            let (mut sum, mut max, mut n) = (0u64, 0u32, 0usize);
            for &rel in &self.scc {
                let rows = map.entry(self.ev.names[rel].clone()).or_default();
                for id in 0..st[rel].len() {
                    if st[rel][id] & P != 0 {
                        rows.push((self.ev.stores[rel].row(id as u32).to_vec(), lv[rel][id], 0));
                        sum += lv[rel][id] as u64;
                        max = max.max(lv[rel][id]);
                        n += 1;
                    }
                }
            }
            CARRY.lock().unwrap().insert(key, map);
            if n > 0 {
                eprintln!("recursive spike:   levels, checked: mean level {:.2}, max {max}", sum as f64 / n as f64);
            }
        }
        r
    }

    // -- DRed ------------------------------------------------------------------------------------------------------

    fn dred(&mut self) -> Report {
        let (c0, e0) = (cpu(), self.ev.emitted());
        let mut st = self.old_state();
        // Over-delete: whatever a gone input row, or a deleted row, derived in the old rows.
        let mut seeds = self.input_seeds(true);
        let mut deleted: Vec<(usize, u32)> = Vec::new();
        while !seeds.is_empty() {
            let mut found: Vec<(usize, Vec<i64>)> = Vec::new();
            {
                let st_ref = &st;
                let is_scc = &self.is_scc;
                let stores = &self.ev.stores;
                let vis = |_: usize, rel: usize, id: u32| if is_scc[rel] { st_ref[rel].get(id as usize).is_some_and(|s| s & P != 0) } else { stores[rel].io[id as usize] & OLD != 0 };
                for (r, i, id) in seeds.drain(..) {
                    let hr = self.ev.rules[r].head_rel;
                    self.ev.fire(r, Seed::Lit(i, id), &vis, &mut |h, _| {
                        found.push((hr, h.to_vec()));
                        true
                    });
                }
            }
            for (rel, row) in found {
                if let Some(id) = self.ev.stores[rel].id(&row)
                    && st[rel][id as usize] & (P | D) == P
                {
                    st[rel][id as usize] |= D;
                    deleted.push((rel, id));
                    seeds.extend(self.scc_seeds(rel, id));
                }
            }
        }
        let e1 = self.ev.emitted();
        for &(rel, id) in &deleted {
            st[rel][id as usize] &= !P;
        }
        // Rederive one step from the survivors.
        let mut again = Vec::new();
        for &(rel, id) in &deleted {
            let row = self.ev.stores[rel].row(id).to_vec();
            let st_ref = &st;
            let is_scc = &self.is_scc;
            let stores = &self.ev.stores;
            let vis = |_: usize, rel: usize, id: u32| if is_scc[rel] { st_ref[rel].get(id as usize).is_some_and(|s| s & P != 0) } else { stores[rel].io[id as usize] & NEW != 0 };
            let mut yes = false;
            for r in (0..self.ev.rules.len()).filter(|r| self.ev.rules[*r].head_rel == rel) {
                self.ev.fire(r, Seed::Head(&row), &vis, &mut |_, _| {
                    yes = true;
                    false
                });
                if yes {
                    break;
                }
            }
            if yes {
                again.push((rel, id));
            }
        }
        for &(rel, id) in &again {
            st[rel][id as usize] |= P;
        }
        let e2 = self.ev.emitted();
        self.insert(&mut st, again.clone(), &mut |_, _, _, _| {});
        let e3 = self.ev.emitted();
        let mut r = Report {
            name: "DRed",
            cpu: cpu() - c0,
            phases: vec![("over-delete", e1 - e0), ("rederive", e2 - e1), ("insert", e3 - e2)],
            deleted: deleted.len(),
            rederived: again.len(),
            ..Default::default()
        };
        self.check(&st, &mut r);
        r
    }

    // -- levels ----------------------------------------------------------------------------------------------------

    /// Whether every premise of the component in `prem` is below `level`.
    fn below(&self, lv: &[Vec<u32>], prem: &[(usize, u32)], level: u32) -> bool {
        prem.iter().all(|&(rel, id)| !self.is_scc[rel] || lv[rel].get(id as usize).is_some_and(|l| *l < level))
    }

    /// Counts row `id`'s derivations over `vis` whose premises of the component are all below its level.
    fn count_supports(&self, rel: usize, id: u32, lv: &[Vec<u32>], vis: Vis) -> u32 {
        let row = self.ev.stores[rel].row(id).to_vec();
        let level = lv[rel][id as usize];
        let mut n = 0u32;
        for r in (0..self.ev.rules.len()).filter(|r| self.ev.rules[*r].head_rel == rel) {
            let is_scc = &self.is_scc;
            self.ev.fire(r, Seed::Head(&row), vis, &mut |_, p| {
                if p.iter().all(|&(rel, id)| !is_scc[rel] || lv[rel].get(id as usize).is_some_and(|l| *l < level)) {
                    n += 1;
                }
                true
            });
        }
        n
    }

    /// Whether a carried state's rows are the old rows (else it is stale: the component moved without it).
    fn carried_fits(&self, st: &[Vec<u8>]) -> bool {
        self.scc.iter().all(|&rel| {
            (0..self.ev.stores[rel].len() as usize).all(|i| st[rel].get(i).is_some_and(|s| s & P != 0) == self.old_p[rel].get(i).is_some_and(|s| s & P != 0))
        })
    }

    /// The component's name across runs: its relations, sorted.
    fn key(&self) -> String {
        let mut names: Vec<&str> = self.scc.iter().map(|&r| self.ev.names[r].as_str()).collect();
        names.sort_unstable();
        names.join(",")
    }

    fn levels(&mut self) -> (Report, Report) {
        // The witness, as the last run would have left it: under `RERAIL_SPIKE_RECURSIVE_CARRY`, what the last run's
        // maintenance left (levels drift as runs go); else levels from a derivation over the old inputs, counts of
        // each row's supports from below.
        let (c0, e0) = (cpu(), self.ev.emitted());
        let carried = if std::env::var_os("RERAIL_SPIKE_RECURSIVE_CARRY").is_some() { CARRY.lock().unwrap().get(&self.key()).cloned() } else { None };
        let mut was_carried = carried.is_some();
        let (mut st, mut lv, mut cnt) = if let Some(map) = carried {
            let mut placed: Vec<(usize, u32, u32, u32)> = Vec::new();
            for &rel in &self.scc.clone() {
                for (row, l, c) in map.get(&self.ev.names[rel]).into_iter().flatten() {
                    let id = self.ev.stores[rel].add(row, 0);
                    placed.push((rel, id, *l, *c));
                }
            }
            let mut st = self.state();
            let mut lv: Vec<Vec<u32>> = self.ev.stores.iter().map(|s| vec![u32::MAX; s.len() as usize]).collect();
            let mut cnt: Vec<Vec<u32>> = self.ev.stores.iter().map(|s| vec![0; s.len() as usize]).collect();
            for (rel, id, l, c) in placed {
                st[rel][id as usize] |= P;
                lv[rel][id as usize] = l;
                cnt[rel][id as usize] = c;
            }
            if self.carried_fits(&st) {
                (st, lv, cnt)
            } else {
                eprintln!("recursive spike:   levels: the carried state is stale, built again");
                was_carried = false;
                let (st, lv) = self.derive(OLD);
                let cnt: Vec<Vec<u32>> = self.ev.stores.iter().map(|s| vec![0; s.len() as usize]).collect();
                (st, lv, cnt)
            }
        } else {
            let (st, lv) = self.derive(OLD);
            let cnt: Vec<Vec<u32>> = self.ev.stores.iter().map(|s| vec![0; s.len() as usize]).collect();
            (st, lv, cnt)
        };
        for &rel in self.scc.iter().filter(|_| !was_carried) {
            for id in 0..self.ev.stores[rel].len() {
                if st[rel].get(id as usize).is_some_and(|s| s & P != 0) {
                    let (st_ref, is_scc, stores) = (&st, &self.is_scc, &self.ev.stores);
                    let vis = |_: usize, r: usize, i: u32| if is_scc[r] { st_ref[r].get(i as usize).is_some_and(|s| s & P != 0) } else { stores[r].io[i as usize] & OLD != 0 };
                    cnt[rel][id as usize] = self.count_supports(rel, id, &lv, &vis);
                }
            }
        }
        let rows: usize = self.scc.iter().map(|&rel| st[rel].iter().filter(|s| **s & P != 0).count()).sum();
        let mut init = Report { name: if was_carried { "levels: carried" } else { "levels: the witness" }, cpu: cpu() - c0, phases: vec![("derive and count", self.ev.emitted() - e0)], kept_bytes: rows * 8, ..Default::default() };
        let mut old_ok = Report::default();
        {
            // The old derivation is the kept rows.
            for &rel in &self.scc {
                let store = &self.ev.stores[rel];
                for id in 0..store.len() {
                    let derived = st[rel].get(id as usize).is_some_and(|s| s & P != 0);
                    let kept = store.io[id as usize] & OLD != 0;
                    if derived && !kept {
                        old_ok.extra += 1;
                    }
                    if kept && !derived {
                        old_ok.missing += 1;
                    }
                }
            }
        }
        init.extra = old_ok.extra;
        init.missing = old_ok.missing;
        let zero_support = self.scc.iter().map(|&rel| (0..st[rel].len()).filter(|&i| st[rel][i] & P != 0 && cnt[rel][i] == 0).count()).sum::<usize>();
        init.anomalies = zero_support;

        // Maintenance.
        let (c0, e0) = (cpu(), self.ev.emitted());
        let mut anomalies = 0usize;
        // Deletion: each derivation that loses a premise is found once, from its first premise processed; a counted
        // one (premises below its head) takes one from the head's count.
        let mut queue: Vec<(usize, u32)> = self.gone.iter().chain(&self.came).copied().collect();
        queue.reverse();
        let mut taken: Vec<(usize, u32)> = Vec::new();
        while let Some((rel, id)) = queue.pop() {
            Self::grow(&mut st, &self.ev, rel);
            st[rel][id as usize] |= X;
            let mut found: Vec<(usize, Vec<i64>, Vec<(usize, u32)>)> = Vec::new();
            for (r, i) in self.positions(rel, id, true) {
                let negs: Vec<bool> = self.ev.rules[r].lits.iter().map(|l| matches!(l, Lit::Atom { neg: true, .. })).collect();
                let st_ref = &st;
                let is_scc = &self.is_scc;
                let stores = &self.ev.stores;
                let seed = (rel, id);
                let vis = |j: usize, r2: usize, i2: u32| {
                    let s = st_ref[r2].get(i2 as usize).copied().unwrap_or(0);
                    let here = if is_scc[r2] { s & P != 0 } else { stores[r2].io[i2 as usize] & OLD != 0 };
                    let processed = s & X != 0 && !(j >= i && (r2, i2) == seed);
                    // A negation is blocked by a row present in the old rows, or by an inserted row processed.
                    if negs[j] { here || processed } else { here && !processed }
                };
                let hr = self.ev.rules[r].head_rel;
                self.ev.fire(r, Seed::Lit(i, id), &vis, &mut |h, p| {
                    found.push((hr, h.to_vec(), p.to_vec()));
                    true
                });
            }
            for (hr, row, prem) in found {
                let Some(h) = self.ev.stores[hr].id(&row) else {
                    anomalies += 1;
                    continue;
                };
                let level = lv[hr][h as usize];
                if self.below(&lv, &prem, level) {
                    let c = &mut cnt[hr][h as usize];
                    if *c == 0 {
                        anomalies += 1;
                        continue;
                    }
                    *c -= 1;
                    if *c == 0 {
                        st[hr][h as usize] |= D;
                        taken.push((hr, h));
                        queue.push((hr, h));
                    }
                }
            }
        }
        let e1 = self.ev.emitted();
        for &(rel, id) in &taken {
            st[rel][id as usize] &= !P;
        }
        // Rederive one step from the survivors, at the lowest level a derivation's premises allow. Its count is of
        // those derivations at that level whose premises are all unchanged: one with a new premise is counted by the
        // insertion, from that premise.
        let mut again = Vec::new();
        for &(rel, id) in &taken {
            let row = self.ev.stores[rel].row(id).to_vec();
            let mut found: Vec<(u32, bool)> = Vec::new();
            {
                let st_ref = &st;
                let is_scc = &self.is_scc;
                let stores = &self.ev.stores;
                let lv_ref = &lv;
                let vis = |_: usize, r2: usize, i2: u32| if is_scc[r2] { st_ref[r2].get(i2 as usize).is_some_and(|s| s & P != 0) } else { stores[r2].io[i2 as usize] & NEW != 0 };
                for r in (0..self.ev.rules.len()).filter(|r| self.ev.rules[*r].head_rel == rel) {
                    let mut all: Vec<Vec<(usize, u32)>> = Vec::new();
                    self.ev.fire(r, Seed::Head(&row), &vis, &mut |_, p| {
                        all.push(p.to_vec());
                        true
                    });
                    // The derivations whose negations held before too: an input row gone that matches a negation
                    // makes the derivation new, counted by the insertion from that row.
                    let negs: Vec<bool> = self.ev.rules[r].lits.iter().map(|l| matches!(l, Lit::Atom { neg: true, .. })).collect();
                    let strict = |j: usize, r2: usize, i2: u32| {
                        let io = stores[r2].io[i2 as usize];
                        vis(j, r2, i2) || (negs[j] && !is_scc[r2] && io & OLD != 0)
                    };
                    let mut held: Vec<Vec<(usize, u32)>> = Vec::new();
                    self.ev.fire(r, Seed::Head(&row), &strict, &mut |_, p| {
                        held.push(p.to_vec());
                        true
                    });
                    for p in all {
                        let l = p.iter().filter(|(r2, _)| is_scc[*r2]).map(|&(r2, i2)| lv_ref[r2][i2 as usize] + 1).max().unwrap_or(0);
                        let new_input = p.iter().any(|&(r2, i2)| !is_scc[r2] && stores[r2].io[i2 as usize] & OLD == 0) || !held.contains(&p);
                        found.push((l, new_input));
                    }
                }
            }
            if let Some(l) = found.iter().map(|(l, _)| *l).min() {
                lv[rel][id as usize] = l;
                cnt[rel][id as usize] = found.iter().filter(|(k, new)| *k <= l && !new).count() as u32;
                again.push((rel, id));
            }
        }
        // Marked after: one rederived row is a new premise of another's derivation, counted by the insertion.
        for &(rel, id) in &again {
            st[rel][id as usize] |= P | N;
        }
        let e2 = self.ev.emitted();
        // Insertion: each new derivation found once, from its last new premise processed. A survivor's count gains
        // the counted ones; an inserted row is counted after.
        let mut queue: Vec<(usize, u32)> = self.gone.iter().chain(&self.came).copied().chain(again.iter().copied()).collect();
        queue.reverse();
        let mut inserted: Vec<(usize, u32)> = again.clone();
        while let Some((rel, id)) = queue.pop() {
            Self::grow(&mut st, &self.ev, rel);
            st[rel][id as usize] |= Y;
            let mut found: Vec<(usize, Vec<i64>, Vec<(usize, u32)>)> = Vec::new();
            for (r, i) in self.positions(rel, id, false) {
                let negs: Vec<bool> = self.ev.rules[r].lits.iter().map(|l| matches!(l, Lit::Atom { neg: true, .. })).collect();
                let st_ref = &st;
                let is_scc = &self.is_scc;
                let stores = &self.ev.stores;
                let seed = (rel, id);
                let vis = |j: usize, r2: usize, i2: u32| {
                    let s = st_ref[r2].get(i2 as usize).copied().unwrap_or(0);
                    let io = stores[r2].io.get(i2 as usize).copied().unwrap_or(0);
                    // A premise new this run (a row inserted, an input row new, or for a negation one gone): seen once
                    // processed, the seed itself only after its own literal.
                    let fresh = if is_scc[r2] { s & N != 0 } else if negs[j] { io & OLD != 0 && io & NEW == 0 } else { io & NEW != 0 && io & OLD == 0 };
                    let allowed = !fresh || if (r2, i2) == seed { j >= i } else { s & Y != 0 };
                    let here = if is_scc[r2] { s & P != 0 } else { io & NEW != 0 };
                    if negs[j] { here || !allowed } else { here && allowed }
                };
                let hr = self.ev.rules[r].head_rel;
                self.ev.fire(r, Seed::Lit(i, id), &vis, &mut |h, p| {
                    found.push((hr, h.to_vec(), p.to_vec()));
                    true
                });
            }
            for (hr, row, prem) in found {
                let h = self.ev.stores[hr].add(&row, 0);
                Self::grow(&mut st, &self.ev, hr);
                if lv[hr].len() < st[hr].len() {
                    lv[hr].resize(st[hr].len(), u32::MAX);
                    cnt[hr].resize(st[hr].len(), 0);
                }
                // Each new derivation is found once: a present row's count gains it when its premises are below; a row
                // not present is inserted at the level its premises give, with this derivation counted.
                let s = st[hr][h as usize];
                if s & P != 0 {
                    if self.below(&lv, &prem, lv[hr][h as usize]) {
                        cnt[hr][h as usize] += 1;
                    }
                } else {
                    let l = prem.iter().filter(|(r2, _)| self.is_scc[*r2]).map(|&(r2, i2)| lv[r2][i2 as usize] + 1).max().unwrap_or(0);
                    st[hr][h as usize] |= P | N;
                    lv[hr][h as usize] = l;
                    cnt[hr][h as usize] = 1;
                    inserted.push((hr, h));
                    queue.push((hr, h));
                }
            }
        }
        let e3 = self.ev.emitted();
        let spent = cpu() - c0;
        // Not timed: every row's count against its supports counted again over the rows now (the witness checked).
        let mut wrong_counts = 0usize;
        let verify = std::env::var_os("RERAIL_SPIKE_RECURSIVE_VERIFY").is_some();
        for &rel in self.scc.iter().filter(|_| verify) {
            for id in 0..st[rel].len() as u32 {
                if st[rel][id as usize] & P == 0 {
                    continue;
                }
                let (st_ref, is_scc, stores) = (&st, &self.is_scc, &self.ev.stores);
                let vis = |_: usize, r: usize, i: u32| if is_scc[r] { st_ref[r].get(i as usize).is_some_and(|s| s & P != 0) } else { stores[r].io[i as usize] & NEW != 0 };
                let n = self.count_supports(rel, id, &lv, &vis);
                if n == 0 || n != cnt[rel][id as usize] {
                    if wrong_counts < 6 {
                        eprintln!(
                            "recursive spike:     levels count: {}{:?} level {} counted {} recounted {n}",
                            self.ev.names[rel],
                            self.ev.stores[rel].row(id),
                            lv[rel][id as usize],
                            cnt[rel][id as usize]
                        );
                    }
                    if wrong_counts < 2 {
                        let row = self.ev.stores[rel].row(id).to_vec();
                        let old_st = self.old_state();
                        for (what, io, s2) in [("new", NEW, &st), ("old", OLD, &old_st)] {
                            let vis = |_: usize, r: usize, i: u32| if is_scc[r] { s2[r].get(i as usize).is_some_and(|s| s & P != 0) } else { stores[r].io[i as usize] & io != 0 };
                            for r in (0..self.ev.rules.len()).filter(|r| self.ev.rules[*r].head_rel == rel) {
                                self.ev.fire(r, Seed::Head(&row), &vis, &mut |_, p| {
                                    let prem: Vec<String> = p.iter().map(|&(pr, pi)| format!("{}{:?} io {}", self.ev.names[pr], self.ev.stores[pr].row(pi), stores[pr].io[pi as usize])).collect();
                                    eprintln!("recursive spike:       {what}: {} from {prem:?}", self.ev.rules[r].text);
                                    true
                                });
                            }
                        }
                    }
                    wrong_counts += 1;
                }
            }
        }
        anomalies += wrong_counts;
        self.ev.emitted.set(e3);
        let rows: usize = self.scc.iter().map(|&rel| st[rel].iter().filter(|s| **s & P != 0).count()).sum();
        let mut r = Report {
            name: "levels",
            cpu: spent,
            phases: vec![("delete", e1 - e0), ("rederive", e2 - e1), ("insert", e3 - e2)],
            deleted: taken.len(),
            rederived: again.len(),
            kept_bytes: rows * 8,
            anomalies,
            ..Default::default()
        };
        self.check(&st, &mut r);
        if std::env::var_os("RERAIL_SPIKE_RECURSIVE_CARRY").is_some() {
            // Kept for the next run, by row value.
            let mut map: FxHashMap<String, Vec<(Vec<i64>, u32, u32)>> = FxHashMap::default();
            for &rel in &self.scc {
                let rows = map.entry(self.ev.names[rel].clone()).or_default();
                for id in 0..st[rel].len() {
                    if st[rel][id] & P != 0 {
                        rows.push((self.ev.stores[rel].row(id as u32).to_vec(), lv[rel][id], cnt[rel][id]));
                    }
                }
            }
            CARRY.lock().unwrap().insert(self.key(), map);
            // Drift: the levels kept against fresh ones, from a derivation over the new inputs (not timed).
            let e = self.ev.emitted();
            let (fst, flv) = self.derive(NEW);
            self.ev.emitted.set(e);
            let (mut sum, mut fsum, mut max, mut fmax, mut above, mut n, mut ones) = (0u64, 0u64, 0u32, 0u32, 0usize, 0usize, 0usize);
            for &rel in &self.scc {
                for id in 0..st[rel].len() {
                    if st[rel][id] & P != 0 && fst[rel].get(id).is_some_and(|s| s & P != 0) {
                        let (l, f) = (lv[rel][id], flv[rel][id]);
                        sum += l as u64;
                        fsum += f as u64;
                        max = max.max(l);
                        fmax = fmax.max(f);
                        above += usize::from(l > f);
                        ones += usize::from(cnt[rel][id] == 1);
                        n += 1;
                    }
                }
            }
            if n > 0 {
                eprintln!(
                    "recursive spike:   levels drift: mean level {:.2} (fresh {:.2}), max {max} (fresh {fmax}), {above} of {n} rows above fresh, {ones} held by one support",
                    sum as f64 / n as f64,
                    fsum as f64 / n as f64
                );
            }
        }
        (init, r)
    }

    // -- B/F -------------------------------------------------------------------------------------------------------

    fn backward_forward(&mut self) -> Report {
        let (c0, e0) = (cpu(), self.ev.emitted());
        let mut st = self.old_state();
        let mut seeds = self.input_seeds(true);
        let (mut deleted, mut checked) = (0usize, 0usize);
        let (mut forward, mut backward) = (0u64, 0u64);
        while !seeds.is_empty() {
            // The rows the seeds derived in the old rows.
            let mut heads: Vec<(usize, Vec<i64>)> = Vec::new();
            let f0 = self.ev.emitted();
            {
                let st_ref = &st;
                let is_scc = &self.is_scc;
                let stores = &self.ev.stores;
                let vis = |_: usize, rel: usize, id: u32| if is_scc[rel] { st_ref[rel].get(id as usize).is_some_and(|s| s & P != 0) } else { stores[rel].io[id as usize] & OLD != 0 };
                for (r, i, id) in seeds.drain(..) {
                    let hr = self.ev.rules[r].head_rel;
                    self.ev.fire(r, Seed::Lit(i, id), &vis, &mut |h, _| {
                        heads.push((hr, h.to_vec()));
                        true
                    });
                }
            }
            forward += self.ev.emitted() - f0;
            for (rel, row) in heads {
                let Some(id) = self.ev.stores[rel].id(&row) else { continue };
                if st[rel][id as usize] & (P | D | PR) != P {
                    continue;
                }
                let b0 = self.ev.emitted();
                // Backward: the rows its derivations rest on, over the rows not deleted and the new inputs.
                let mut set = vec![(rel, id)];
                st[rel][id as usize] |= CK;
                let mut k = 0;
                while k < set.len() {
                    let (gr, gi) = set[k];
                    k += 1;
                    let grow = self.ev.stores[gr].row(gi).to_vec();
                    let mut more: Vec<(usize, u32)> = Vec::new();
                    {
                        let st_ref = &st;
                        let is_scc = &self.is_scc;
                        let stores = &self.ev.stores;
                        let vis = |_: usize, r2: usize, i2: u32| if is_scc[r2] { st_ref[r2].get(i2 as usize).is_some_and(|s| s & (P | D) == P) } else { stores[r2].io[i2 as usize] & NEW != 0 };
                        for r in (0..self.ev.rules.len()).filter(|r| self.ev.rules[*r].head_rel == gr) {
                            self.ev.fire(r, Seed::Head(&grow), &vis, &mut |_, p| {
                                for &(r2, i2) in p {
                                    if is_scc[r2] && st_ref[r2][i2 as usize] & (PR | CK) == 0 {
                                        more.push((r2, i2));
                                    }
                                }
                                true
                            });
                        }
                    }
                    for (r2, i2) in more {
                        if st[r2][i2 as usize] & (PR | CK) == 0 {
                            st[r2][i2 as usize] |= CK;
                            set.push((r2, i2));
                        }
                    }
                }
                checked += set.len();
                // Forward over the set: proved from the new inputs and rows proved.
                let mut proved: Vec<(usize, u32)> = Vec::new();
                for &(gr, gi) in &set {
                    let grow = self.ev.stores[gr].row(gi).to_vec();
                    let mut yes = false;
                    {
                        let st_ref = &st;
                        let is_scc = &self.is_scc;
                        let stores = &self.ev.stores;
                        let vis = |_: usize, r2: usize, i2: u32| if is_scc[r2] { st_ref[r2].get(i2 as usize).is_some_and(|s| s & (P | D) == P && s & PR != 0) } else { stores[r2].io[i2 as usize] & NEW != 0 };
                        for r in (0..self.ev.rules.len()).filter(|r| self.ev.rules[*r].head_rel == gr) {
                            self.ev.fire(r, Seed::Head(&grow), &vis, &mut |_, _| {
                                yes = true;
                                false
                            });
                            if yes {
                                break;
                            }
                        }
                    }
                    if yes {
                        proved.push((gr, gi));
                    }
                }
                // Proved rows prove more of the set, forward.
                let mut q = proved.clone();
                for &(gr, gi) in &proved {
                    st[gr][gi as usize] = (st[gr][gi as usize] | PR) & !CK;
                }
                while let Some((gr, gi)) = q.pop() {
                    let mut heads2: Vec<(usize, Vec<i64>)> = Vec::new();
                    {
                        let st_ref = &st;
                        let is_scc = &self.is_scc;
                        let stores = &self.ev.stores;
                        let vis = |_: usize, r2: usize, i2: u32| if is_scc[r2] { st_ref[r2].get(i2 as usize).is_some_and(|s| s & (P | D) == P && s & PR != 0) } else { stores[r2].io[i2 as usize] & NEW != 0 };
                        for (r, i, id2) in self.scc_seeds(gr, gi) {
                            let hr = self.ev.rules[r].head_rel;
                            self.ev.fire(r, Seed::Lit(i, id2), &vis, &mut |h, _| {
                                heads2.push((hr, h.to_vec()));
                                true
                            });
                        }
                    }
                    for (hr, row) in heads2 {
                        if let Some(h) = self.ev.stores[hr].id(&row)
                            && st[hr][h as usize] & CK != 0
                        {
                            st[hr][h as usize] = (st[hr][h as usize] | PR) & !CK;
                            q.push((hr, h));
                        }
                    }
                }
                backward += self.ev.emitted() - b0;
                // The set's rows not proved are gone: their consequences are checked in turn.
                for &(gr, gi) in &set {
                    if st[gr][gi as usize] & CK != 0 {
                        st[gr][gi as usize] = (st[gr][gi as usize] | D) & !CK;
                        deleted += 1;
                        seeds.extend(self.scc_seeds(gr, gi));
                    }
                }
            }
        }
        for &rel in &self.scc {
            for s in &mut st[rel] {
                if *s & D != 0 {
                    *s &= !P;
                }
            }
        }
        let e1 = self.ev.emitted();
        self.insert(&mut st, Vec::new(), &mut |_, _, _, _| {});
        let e2 = self.ev.emitted();
        let mut r = Report {
            name: "B/F",
            cpu: cpu() - c0,
            phases: vec![("forward from deletions", forward), ("backward checks", backward), ("insert", e2 - e1)],
            deleted,
            checked,
            ..Default::default()
        };
        let _ = e0;
        self.check(&st, &mut r);
        r
    }
}

/// Runs the spike over each recursive component of `program` it can evaluate.
pub(in crate::code_rule_dsl) fn run(
    program: &Program,
    old: Option<&Retained>,
    derived: &[(String, usize, Arc<InRel>)],
    inputs: &[(String, Arc<dyn Source>)],
    cat: &dyn Fn(i64, i64) -> i64,
) {
    let Some(old) = old else {
        eprintln!("recursive spike: nothing kept");
        return;
    };
    let min_rows: usize = std::env::var("RERAIL_SPIKE_RECURSIVE_MIN").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);
    for scc in recursive(program) {
        let rules: Vec<&front::Rule> = program.rules.iter().filter(|r| scc.contains(&r.head.rel)).collect();
        let reads: BTreeSet<String> = rules.iter().flat_map(|r| &r.body).filter_map(|l| if let Literal::Atom(a) = l { Some(a.rel.clone()) } else { None }).filter(|r| !scc.contains(r)).collect();
        let facts = |rel: &str| -> Option<Rows> {
            program.facts.get(rel).map(|rows| rows.iter().map(|r| r.iter().map(|v| match v { front::Value::Int(i) => *i, front::Value::Str(_) => i64::MIN }).collect()).collect())
        };
        let new_of = |rel: &str| derived.iter().find(|(r, _, _)| r == rel).map(|(_, w, h)| rel_rows(h, *w)).or_else(|| inputs.iter().find(|(r, _)| r == rel).map(|(_, s)| rows_of(s.as_ref())));
        let old_of = |rel: &str| old.kept(rel).map(|(h, w)| rel_rows(h, w)).or_else(|| old.input_source(rel).map(|s| rows_of(s.as_ref())));
        let old_rows: usize = scc.iter().filter_map(|r| old_of(r)).map(|r| r.len()).sum();
        // A component not maintained this run loses what it carried: its rows moved without it.
        let forget = || {
            let mut names: Vec<&str> = scc.iter().map(String::as_str).collect();
            names.sort_unstable();
            let key = names.join(",");
            let mut carry = CARRY.lock().unwrap();
            carry.remove(&key);
            carry.remove(&format!("check:{key}"));
        };
        if old_rows < min_rows {
            forget();
            continue;
        }
        // `RERAIL_SPIKE_RECURSIVE_ONLY=r`: only the component holding relation `r`.
        if std::env::var("RERAIL_SPIKE_RECURSIVE_ONLY").is_ok_and(|r| !scc.contains(&r)) {
            forget();
            continue;
        }
        let lattices: Vec<&String> = scc.iter().chain(&reads).filter(|r| program.lattice.contains_key(*r)).collect();
        if !lattices.is_empty() {
            eprintln!("recursive spike: component of {} relations reads lattices {lattices:?} (as sets here)", scc.len());
        }
        let names: Vec<String> = scc.iter().chain(&reads).cloned().collect();
        let index: BTreeMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
        let mut stores: Vec<Store> = names.iter().map(|n| Store::new(program.decls.get(n).map_or(0, Vec::len))).collect();
        let mut assumed: Vec<String> = Vec::new();
        let mut as_was: Vec<String> = Vec::new();
        let mut want: Vec<std::collections::HashSet<Vec<i64>>> = vec![Default::default(); names.len()];
        let mut skip = None;
        for (i, rel) in names.iter().enumerate() {
            let is_scc = scc.contains(rel);
            let (o, n) = (old_of(rel), new_of(rel));
            let (o, n) = match (o, n) {
                (Some(o), Some(n)) => (o, n),
                // Not derived this run: as it was.
                (Some(o), None) => {
                    if !is_scc {
                        as_was.push(rel.clone());
                    }
                    (o.clone(), o)
                }
                (None, Some(n)) => {
                    assumed.push(rel.clone());
                    (n.clone(), n)
                }
                (None, None) => match facts(rel) {
                    Some(f) => (f.clone(), f),
                    None => {
                        skip = Some(format!("no rows of {rel}"));
                        break;
                    }
                },
            };
            for row in &o {
                stores[i].add(row, OLD);
            }
            if is_scc {
                want[i] = n.into_iter().collect();
            } else {
                for row in &n {
                    stores[i].add(row, NEW);
                }
            }
        }
        if let Some(why) = skip {
            eprintln!("recursive spike: component of {} relations skipped: {why}", scc.len());
            forget();
            continue;
        }
        let rel_of = |r: &str| index.get(r).copied();
        // Rows per key, measured on the rows loaded (old and new), by (relation, mask).
        let fan_cache: std::cell::RefCell<FxHashMap<(usize, u64), f64>> = Default::default();
        let fanout = |rel: usize, mask: u64| -> f64 {
            *fan_cache.borrow_mut().entry((rel, mask)).or_insert_with(|| {
                let st = &stores[rel];
                let n = st.len() as f64;
                if mask == 0 || n == 0.0 {
                    return n;
                }
                let cols: Vec<usize> = (0..st.width).filter(|c| mask >> c & 1 == 1).collect();
                let keys: rustc_hash::FxHashSet<Vec<i64>> = (0..st.len()).map(|id| cols.iter().map(|&c| st.row(id)[c]).collect()).collect();
                n / keys.len() as f64
            })
        };
        let compiled: Result<Vec<Rule>, String> = rules.iter().map(|r| compile(r, &rel_of, &fanout)).collect();
        let compiled = match compiled {
            Ok(c) => c,
            Err(why) => {
                eprintln!("recursive spike: component of {} relations skipped: {why}", scc.len());
                forget();
                continue;
            }
        };
        for rule in &compiled {
            for plan in &rule.plans {
                for step in plan {
                    if let Step::Pos { rel, mask, .. } | Step::Neg { rel, mask, .. } = step {
                        stores[*rel].index(*mask);
                    }
                }
            }
        }
        let is_scc: Vec<bool> = names.iter().map(|n| scc.contains(n)).collect();
        let mut reads_of: Vec<Vec<(usize, usize, bool)>> = vec![Vec::new(); names.len()];
        for (r, rule) in compiled.iter().enumerate() {
            for (i, l) in rule.lits.iter().enumerate() {
                if let Lit::Atom { rel, neg, .. } = l {
                    reads_of[*rel].push((r, i, *neg));
                }
            }
        }
        let (mut gone, mut came) = (Vec::new(), Vec::new());
        for (rel, s) in stores.iter().enumerate() {
            if is_scc[rel] {
                continue;
            }
            for id in 0..s.len() {
                match s.io[id as usize] {
                    OLD => gone.push((rel, id)),
                    NEW => came.push((rel, id)),
                    _ => {}
                }
            }
        }
        let scc_ids: Vec<usize> = (0..names.len()).filter(|i| is_scc[*i]).collect();
        let want_rows: usize = want.iter().map(|w| w.len()).sum();
        let mut c = Component { ev: Eval { stores, names: names.clone(), rules: compiled, cat, emitted: std::cell::Cell::new(0), profile: None, probes: std::cell::Cell::new(0) }, scc: scc_ids, is_scc, reads: reads_of, want, old_p: Vec::new(), gone, came };
        let _ = &c.ev.names;
        eprintln!(
            "recursive spike: component of {} relations ({}…), {} rules, {} inputs; rows {old_rows} old, {want_rows} after; inputs' rows {} gone, {} new{}",
            scc.len(),
            scc.iter().take(4).cloned().collect::<Vec<_>>().join(", "),
            rules.len(),
            reads.len(),
            c.gone.len(),
            c.came.len(),
            if assumed.is_empty() { String::new() } else { format!("; old rows assumed as new: {assumed:?}") }
        );
        if std::env::var_os("RERAIL_SPIKE_RECURSIVE_DIFF").is_some() {
            eprintln!("recursive spike:   inputs not derived this run, read as they were: {as_was:?}");
        }
        let _ = c.ev.rules.iter().map(|r| r.text.len()).sum::<usize>();
        // The old rows: derived over the old inputs (the engine's kept rows are not every algorithm's start, as some
        // of them are not closed under these rules), and how they differ from the engine's.
        if std::env::var_os("RERAIL_SPIKE_RECURSIVE_PROFILE").is_some() {
            c.ev.profile = Some(std::cell::RefCell::new(FxHashMap::default()));
        }
        // The rules that took most CPU since the last call, and none counted after.
        let show = |c: &Component, what: &str| {
            let Some(p) = &c.ev.profile else { return };
            let mut by: Vec<((usize, usize), (f64, u64, u64, f64))> = p.borrow().iter().map(|(k, v)| (*k, *v)).collect();
            by.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
            let total: f64 = by.iter().map(|x| x.1.0).sum();
            eprintln!("recursive spike:     {what}: {total:.2} s in firings");
            for ((r, plan), (t, n, probes, em)) in by.into_iter().take(6).filter(|(_, (t, ..))| *t > 0.05) {
                let rule = &c.ev.rules[r];
                let order: Vec<String> = rule.plans[plan]
                    .iter()
                    .map(|s| match s {
                        Step::Pos { rel, mask, .. } => format!("{}/{mask:b}", c.ev.names[*rel]),
                        Step::Neg { rel, mask, .. } => format!("!{}/{mask:b}", c.ev.names[*rel]),
                        Step::Cmp { .. } => "cmp".into(),
                        Step::Assign { .. } => "=".into(),
                    })
                    .collect();
                let seed = match plan { 0 => "none".to_string(), 1 => "head".into(), i => format!("lit {}", i - 2) };
                eprintln!(
                    "recursive spike:       {t:.2} s, {n} firings seeded {seed}, {probes} probes, {em} emitted: {} | {}",
                    order.join(" "),
                    rule.text
                );
            }
            p.borrow_mut().clear();
        };
        let (old_p, _) = c.derive(OLD);
        show(&c, "derive old");
        let (mut kept_extra, mut kept_missing) = (0usize, 0usize);
        for &rel in &c.scc {
            for id in 0..c.ev.stores[rel].len() {
                let derived = old_p[rel].get(id as usize).is_some_and(|s| s & P != 0);
                let kept = c.ev.stores[rel].io[id as usize] & OLD != 0;
                kept_extra += usize::from(derived && !kept);
                kept_missing += usize::from(kept && !derived);
                if kept && !derived && kept_missing <= 6 && std::env::var_os("RERAIL_SPIKE_RECURSIVE_DIFF").is_some() {
                    eprintln!("recursive spike:     kept, not derived over the old inputs: {}{:?}", c.ev.names[rel], c.ev.stores[rel].row(id as u32));
                }
            }
        }
        c.old_p = old_p;
        let mut first = c.recompute();
        first.name = "recompute (vs engine)";
        eprintln!("recursive spike:   old rows derived over the old inputs, against the engine's kept rows: +{kept_extra} -{kept_missing}");
        show(&c, "recompute");
        // `RERAIL_SPIKE_RECURSIVE_ALGOS`: which to run, of dred, levels, bf, check (all by default).
        let algos = std::env::var("RERAIL_SPIKE_RECURSIVE_ALGOS").unwrap_or_else(|_| "dred,levels,bf,check".into());
        let mut reports = vec![first];
        if algos.contains("dred") {
            reports.push(c.dred());
            show(&c, "DRed");
        }
        let mut init = None;
        if algos.contains("levels") {
            let (i, levels) = c.levels();
            show(&c, "levels");
            reports.push(levels);
            init = Some(i);
        }
        if algos.contains("bf") {
            reports.push(c.backward_forward());
            show(&c, "B/F");
        }
        if algos.contains("check") {
            reports.push(c.levels_check());
            show(&c, "levels, checked");
        }
        reports.extend(init);
        for r in reports {
            let total: u64 = r.phases.iter().map(|(_, n)| n).sum();
            eprintln!(
                "recursive spike:   {:<20} cpu {:>7.3} s, emitted {total:>9} ({}); deleted {}, rederived {}, checked {}; kept {} B; differs +{} -{}, anomalies {}",
                r.name,
                r.cpu,
                r.phases.iter().map(|(p, n)| format!("{p} {n}")).collect::<Vec<_>>().join(", "),
                r.deleted,
                r.rederived,
                r.checked,
                r.kept_bytes,
                r.extra,
                r.missing,
                r.anomalies
            );
        }
    }
}
