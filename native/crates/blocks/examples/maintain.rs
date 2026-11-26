//! Decision 0328's spike: maintenance on 0266's blocks, from the deltas alone.
//!
//! The question (the user, 2026-11-17): with immutable blocks and deltas, is anything needed beyond reading the
//! deltas? Here a run keeps nothing but the relations themselves: no counts, no levels, no support relations, no
//! maintained forms. The last run's block is the old state, read in place; this run's delta block is the new one.
//! A relation's change in this run is read off its delta: its kills past the carried ones, its rows past the carried
//! ones.
//!
//! The program, one `Facts` block in five strata, over documents' `def`, `imp` and `call` rows:
//!
//! ```text
//! S1  edge(a, b)          :- imp(a, n), def(b, n), a != b.          a projection: many derivations a row
//!     defined(n)          :- def(_, n).                              the same
//! S2  unresolved(d, s, n) :- call(d, s, n), !defined(n).             a negation
//!     resolved(d, s, b)   :- call(d, s, n), imp(d, n), def(b, n).    three atoms, a projection
//! S3  reach(a, b)         :- edge(a, b).                             recursion
//!     reach(a, c)         :- reach(a, b), edge(b, c).
//! S4  tainted(a)          :- reach(a, b), unresolved(b, _, _).       over the recursive stratum
//!     uses(a, b)          :- resolved(a, _, b).
//! ```
//!
//! Each edit is applied three ways, each from the same kept block, each into a delta of its own:
//!
//! - **again:** every derived stratum derived again into scratch, then diffed into the delta (what 0090 §4's
//!   threshold falls back to);
//! - **delta:** each non-recursive stratum joined from its reads' deltas (removed rows over the old state, added over
//!   the new), its removal candidates rechecked by a probe over the new state, and the recursive stratum derived
//!   again and diffed;
//! - **delta+dred:** the same, and the recursive stratum by overdelete and rederive, from the deltas alone;
//! - **delta+part:** the same, and the recursive stratum, partitioned by its first column (checked on the rules),
//!   derived again only for the keys the change reaches, each with its key bound;
//! - **delta+levels:** the same, and the recursive stratum by 0090's *levels, checked*: each row's level (one more
//!   than its derivation's highest), kept; a removal candidate survives when a derivation from lower levels does.
//!   The kept levels are the one piece of state beyond the relations. Here they are a side map computed outside the
//!   timed part (in a block they would be a column); the levels the edit changes are counted.
//!
//! Every result is checked against a derive from scratch, by every relation's digest.
//!
//! The last edit is a big one: one document's every definition removed.
//!
//! Usage: `maintain [docs] [edits]`.

use rerail_blocks::facts::{Facts, FrozenFacts, RelDef, Schema};
use rerail_blocks::relation::{Spec, View};
use rerail_blocks::{footprint, retired};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

// ---- the program ----

#[derive(Clone, Copy, Debug, PartialEq)]
enum T {
    V(usize),
    W,
}

#[derive(Clone, Debug)]
enum L {
    A(usize, Vec<T>),
    N(usize, Vec<T>),
    Ne(usize, usize),
}

#[derive(Clone, Debug)]
struct Rule {
    head: usize,
    args: Vec<T>,
    body: Vec<L>,
}

const DEF: usize = 0;
const IMP: usize = 1;
const CALL: usize = 2;
const EDGE: usize = 3;
const DEFINED: usize = 4;
const UNRES: usize = 5;
const RESOLVED: usize = 6;
const REACH: usize = 7;
const TAINTED: usize = 8;
const USES: usize = 9;
const INPUTS: [usize; 3] = [DEF, IMP, CALL];
const NREL: usize = 10;

fn spec(widths: &[u32], indexes: &[&[u16]]) -> Spec {
    Spec { widths: widths.to_vec(), sort: Some(0), indexes: indexes.iter().map(|k| k.to_vec()).collect(), min_log: 10 }
}

fn schema() -> Arc<Schema> {
    let two = || spec(&[4, 4], &[&[0], &[1]]);
    let three = || spec(&[4, 4, 4], &[&[0], &[2]]);
    let rels: Vec<(&str, usize, Spec)> = vec![
        ("def", 0, two()),
        ("imp", 0, two()),
        ("call", 0, three()),
        ("edge", 1, two()),
        ("defined", 1, spec(&[4], &[&[0]])),
        ("unresolved", 2, three()),
        ("resolved", 2, three()),
        ("reach", 3, two()),
        ("tainted", 4, spec(&[4], &[&[0]])),
        ("uses", 4, two()),
    ];
    Arc::new(Schema { rels: rels.into_iter().map(|(n, s, sp)| RelDef { name: n.into(), stratum: s, spec: sp }).collect(), nstrata: 5 })
}

fn program() -> Vec<Rule> {
    use L::*;
    use T::*;
    let r = |head, args: Vec<T>, body: Vec<L>, _nvars: usize| Rule { head, args, body };
    vec![
        r(EDGE, vec![V(0), V(1)], vec![A(IMP, vec![V(0), V(2)]), A(DEF, vec![V(1), V(2)]), Ne(0, 1)], 3),
        r(DEFINED, vec![V(0)], vec![A(DEF, vec![W, V(0)])], 1),
        r(UNRES, vec![V(0), V(1), V(2)], vec![A(CALL, vec![V(0), V(1), V(2)]), N(DEFINED, vec![V(2)])], 3),
        r(RESOLVED, vec![V(0), V(1), V(3)], vec![A(CALL, vec![V(0), V(1), V(2)]), A(IMP, vec![V(0), V(2)]), A(DEF, vec![V(3), V(2)])], 4),
        r(REACH, vec![V(0), V(1)], vec![A(EDGE, vec![V(0), V(1)])], 2),
        r(REACH, vec![V(0), V(2)], vec![A(REACH, vec![V(0), V(1)]), A(EDGE, vec![V(1), V(2)])], 3),
        r(TAINTED, vec![V(0)], vec![A(REACH, vec![V(0), V(1)]), A(UNRES, vec![V(1), W, W])], 2),
        r(USES, vec![V(0), V(1)], vec![A(RESOLVED, vec![V(0), W, V(1)])], 2),
    ]
}

// ---- sources: what a rule's atoms read ----

/// A relation's rows, read by the columns a probe binds. `f` returns false to stop; `probe` returns false if stopped.
trait Src {
    fn probe(&self, cols: &[usize], key: &[i64], f: &mut dyn FnMut(&[i64]) -> bool) -> bool;
    fn contains(&self, row: &[i64]) -> bool {
        let cols: Vec<usize> = (0..row.len()).collect();
        !self.probe(&cols, row, &mut |_| false)
    }
}

/// A frozen relation: a full block's, or a base and its delta read as one (`View`), through its hash indexes.
struct VSrc<'a>(View<'a>);

impl Src for VSrc<'_> {
    fn probe(&self, cols: &[usize], key: &[i64], f: &mut dyn FnMut(&[i64]) -> bool) -> bool {
        let v = &self.0;
        let a = v.arity();
        let mut row = [0i64; 8];
        let mut emit = |id: u32| -> bool {
            for (c, cell) in row.iter_mut().enumerate().take(a) {
                *cell = v.get(id, c);
            }
            if !cols.iter().zip(key).all(|(&c, &k)| row[c] == k) {
                return true;
            }
            f(&row[..a])
        };
        // The index whose columns are all bound, the widest.
        let ixs = v.top.indexes.as_slice();
        let best = ixs.iter().enumerate().filter(|(_, ix)| ix.key[..ix.ncols as usize].iter().all(|c| cols.contains(&(*c as usize)))).max_by_key(|(_, ix)| ix.ncols);
        match best {
            Some((i, ix)) => {
                let mut k = [0i64; 4];
                for (j, c) in ix.key[..ix.ncols as usize].iter().enumerate() {
                    k[j] = key[cols.iter().position(|x| *x == *c as usize).unwrap()];
                }
                let k = &k[..ix.ncols as usize];
                for id in v.probe_iter(i, k) {
                    if !emit(id) {
                        return false;
                    }
                }
            }
            None => {
                for id in v.ordered_iter() {
                    if !emit(id) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Rows in memory, with a hash index per column: a fixpoint's rows while it runs, and the sets maintenance builds.
#[derive(Default)]
struct Mem {
    rows: Vec<Vec<i64>>,
    set: HashSet<Vec<i64>>,
    by: Vec<HashMap<i64, Vec<u32>>>,
}

impl Mem {
    fn new(arity: usize) -> Mem {
        Mem { by: (0..arity).map(|_| HashMap::new()).collect(), ..Mem::default() }
    }
    fn insert(&mut self, row: &[i64]) -> bool {
        if self.set.contains(row) {
            return false;
        }
        let id = self.rows.len() as u32;
        for (c, &v) in row.iter().enumerate() {
            self.by[c].entry(v).or_default().push(id);
        }
        self.rows.push(row.to_vec());
        self.set.insert(row.to_vec());
        true
    }
    fn has(&self, row: &[i64]) -> bool {
        self.set.contains(row)
    }
}

impl Src for Mem {
    fn probe(&self, cols: &[usize], key: &[i64], f: &mut dyn FnMut(&[i64]) -> bool) -> bool {
        let ok = |r: &[i64]| cols.iter().zip(key).all(|(&c, &k)| r[c] == k);
        if let Some((&c, &k)) = cols.iter().zip(key).next() {
            for &id in self.by[c].get(&k).map(Vec::as_slice).unwrap_or(&[]) {
                let r = &self.rows[id as usize];
                if ok(r) && !f(r) {
                    return false;
                }
            }
        } else {
            for r in &self.rows {
                if !f(r) {
                    return false;
                }
            }
        }
        true
    }
    fn contains(&self, row: &[i64]) -> bool {
        self.has(row)
    }
}

/// A frozen relation less some rows, plus others: a recursive stratum's rows while it is maintained.
struct Overlay<'a> {
    base: &'a dyn Src,
    minus: &'a HashSet<Vec<i64>>,
    plus: &'a Mem,
}

impl Src for Overlay<'_> {
    fn probe(&self, cols: &[usize], key: &[i64], f: &mut dyn FnMut(&[i64]) -> bool) -> bool {
        let minus = self.minus;
        self.base.probe(cols, key, &mut |r| minus.contains(r) || f(r)) && self.plus.probe(cols, key, f)
    }
    fn contains(&self, row: &[i64]) -> bool {
        self.plus.has(row) || (!self.minus.contains(row) && self.base.contains(row))
    }
}

/// A relation not readable here (not made yet): a rule that reads it is a bug of the driver.
struct Missing;
impl Src for Missing {
    fn probe(&self, _: &[usize], _: &[i64], _: &mut dyn FnMut(&[i64]) -> bool) -> bool {
        panic!("a read of a relation not made yet")
    }
}
static MISSING: Missing = Missing;

// ---- the evaluator: one rule, any sources ----

/// Joins `rule`'s literals in `todo` (a bit per literal) under `env`, over `srcs` (one per relation), calling `out`
/// with each binding. The next literal is a filter whose variables are bound, else the atom with the most bound
/// columns. Returns false if `out` stopped it.
fn solve(rule: &Rule, todo: u32, env: &mut [Option<i64>; 8], srcs: &[&dyn Src], stats: &mut Stats, out: &mut dyn FnMut(&[Option<i64>; 8]) -> bool) -> bool {
    if todo == 0 {
        return out(env);
    }
    let bound = |t: &T, env: &[Option<i64>; 8]| match t {
        T::V(v) => env[*v].is_some(),
        T::W => false,
    };
    let lits: Vec<usize> = (0..rule.body.len()).filter(|i| todo >> i & 1 == 1).collect();
    // A filter first.
    for &i in &lits {
        match &rule.body[i] {
            L::Ne(x, y) if env[*x].is_some() && env[*y].is_some() => {
                return env[*x] == env[*y] || solve(rule, todo & !(1 << i), env, srcs, stats, out);
            }
            L::N(r, args) if args.iter().all(|t| matches!(t, T::W) || bound(t, env)) => {
                let (cols, key): (Vec<usize>, Vec<i64>) = args.iter().enumerate().filter_map(|(c, t)| match t {
                    T::V(v) => Some((c, env[*v].unwrap())),
                    T::W => None,
                }).unzip();
                stats.probes += 1;
                let exists = !srcs[*r].probe(&cols, &key, &mut |_| false);
                return exists || solve(rule, todo & !(1 << i), env, srcs, stats, out);
            }
            _ => {}
        }
    }
    let pick = lits.iter().copied().filter(|&i| matches!(rule.body[i], L::A(..))).max_by_key(|&i| match &rule.body[i] {
        L::A(_, args) => (args.iter().filter(|t| bound(t, env)).count(), usize::MAX - i),
        _ => unreachable!(),
    });
    let Some(i) = pick else { panic!("rule {rule:?}: a negation or comparison of an unbound variable") };
    let L::A(r, args) = &rule.body[i] else { unreachable!() };
    let (cols, key): (Vec<usize>, Vec<i64>) = args.iter().enumerate().filter_map(|(c, t)| match t {
        T::V(v) => env[*v].map(|x| (c, x)),
        T::W => None,
    }).unzip();
    stats.probes += 1;
    let rest = todo & !(1 << i);
    srcs[*r].probe(&cols, &key, &mut |row| {
        stats.rows += 1;
        let mut set = [usize::MAX; 8];
        let mut n = 0;
        let mut ok = true;
        for (c, t) in args.iter().enumerate() {
            if let T::V(v) = t {
                match env[*v] {
                    Some(x) if x != row[c] => {
                        ok = false;
                        break;
                    }
                    Some(_) => {}
                    None => {
                        env[*v] = Some(row[c]);
                        set[n] = *v;
                        n += 1;
                    }
                }
            }
        }
        let go = !ok || solve(rule, rest, env, srcs, stats, out);
        for &v in &set[..n] {
            env[v] = None;
        }
        go
    })
}

fn head_of(rule: &Rule, env: &[Option<i64>; 8]) -> Vec<i64> {
    rule.args.iter().map(|t| match t {
        T::V(v) => env[*v].expect("a head variable bound"),
        T::W => unreachable!("a head wildcard"),
    }).collect()
}

/// Every head `rule` derives with literal `seed` bound to each of `rows` (the rest over `srcs`). A positive seed is
/// that atom's row; a negated seed binds its variables and is still checked over `srcs`.
fn from_seed(rule: &Rule, seed: usize, rows: &[Vec<i64>], srcs: &[&dyn Src], stats: &mut Stats, out: &mut dyn FnMut(Vec<i64>)) {
    let all = (1u32 << rule.body.len()) - 1;
    let (args, neg) = match &rule.body[seed] {
        L::A(_, a) => (a, false),
        L::N(_, a) => (a, true),
        L::Ne(..) => return,
    };
    let todo = if neg { all } else { all & !(1 << seed) };
    for row in rows {
        let mut env = [None; 8];
        let mut ok = true;
        for (c, t) in args.iter().enumerate() {
            if let T::V(v) = t {
                match env[*v] {
                    Some(x) if x != row[c] => ok = false,
                    _ => env[*v] = Some(row[c]),
                }
            }
        }
        if ok {
            solve(rule, todo, &mut env, srcs, stats, &mut |e| {
                out(head_of(rule, e));
                true
            });
        }
    }
}

/// Whether `rule` derives `head` over `srcs`: its body probed with the head's variables bound.
fn derives(rule: &Rule, head: &[i64], srcs: &[&dyn Src], stats: &mut Stats) -> bool {
    let mut env = [None; 8];
    for (t, &x) in rule.args.iter().zip(head) {
        if let T::V(v) = t {
            if env[*v].is_some_and(|y| y != x) {
                return false;
            }
            env[*v] = Some(x);
        }
    }
    stats.rechecks += 1;
    !solve(rule, (1 << rule.body.len()) - 1, &mut env, srcs, stats, &mut |_| false)
}

fn reads(rule: &Rule, r: usize) -> bool {
    rule.body.iter().any(|l| matches!(l, L::A(x, _) | L::N(x, _) if *x == r))
}

#[derive(Default, Clone, Copy)]
struct Stats {
    probes: u64,
    rows: u64,
    rechecks: u64,
    candidates: u64,
    kills: u64,
    adds: u64,
    level_writes: u64,
}

// ---- the documents ----

fn rng(mut x: u64) -> impl FnMut() -> u64 {
    move || {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

const PACKAGE: u64 = 20;
const NAMES: u64 = 64;

/// A document's rows: `def(d, n)`, `imp(d, n)`, `call(d, site, n)`, each sorted.
#[derive(Clone)]
struct Doc {
    rows: [Vec<Vec<i64>>; 3],
}

fn import_name(d: u64, r: &mut impl FnMut() -> u64) -> i64 {
    let p = d / PACKAGE;
    // 5% from the package before: the recursion crosses packages.
    let p = if p > 0 && r().is_multiple_of(20) { p - 1 } else { p };
    ((p * PACKAGE + r() % PACKAGE) * NAMES + r() % NAMES) as i64
}

fn make_doc(d: u64) -> Doc {
    let mut r = rng(d << 20);
    let mut def: Vec<Vec<i64>> = (0..40).map(|_| vec![d as i64, (d * NAMES + r() % NAMES) as i64]).collect();
    let mut imp: Vec<Vec<i64>> = (0..40).map(|_| vec![d as i64, import_name(d, &mut r)]).collect();
    def.sort();
    def.dedup();
    imp.sort();
    imp.dedup();
    let mut call: Vec<Vec<i64>> = (0..120)
        .map(|s| {
            let x = r() % 10;
            let n = if x < 7 { imp[(r() % imp.len() as u64) as usize][1] } else if x < 9 { def[(r() % def.len() as u64) as usize][1] } else { import_name(d, &mut r) };
            vec![d as i64, s, n]
        })
        .collect();
    call.sort();
    Doc { rows: [def, imp, call] }
}

/// A small edit of `doc`: one definition, one import and one call changed.
fn edit(doc: &mut Doc, d: u64, seed: u64) {
    let mut r = rng(seed);
    let [def, imp, call] = &mut doc.rows;
    let i = (r() % def.len() as u64) as usize;
    def[i][1] = (d * NAMES + r() % NAMES) as i64;
    let i = (r() % imp.len() as u64) as usize;
    imp[i][1] = import_name(d, &mut r);
    let i = (r() % call.len() as u64) as usize;
    call[i][2] = import_name(d, &mut r);
    for rows in [def, imp, call] {
        rows.sort();
        rows.dedup();
    }
}

// ---- blocks ----

fn reserve() -> usize {
    std::env::var("MAINTAIN_RESERVE_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(1024usize) << 20
}

fn push_all(f: &Facts, r: usize, rows: impl Iterator<Item = Vec<i64>>) {
    let w = f.w(r);
    let flat: Vec<i64> = rows.flatten().collect();
    w.extend(&flat);
}

/// Each relation of a block as a source (`Missing` where not made).
fn srcs_of<'a>(views: &'a [Option<VSrc<'a>>]) -> Vec<&'a dyn Src> {
    views.iter().map(|v| v.as_ref().map_or(&MISSING as &dyn Src, |v| v as &dyn Src)).collect()
}

/// Derives stratum `s` of `f` from scratch (every lower stratum frozen in `f`), its relations made, then frozen.
fn derive_stratum(f: &Facts, prog: &[Rule], s: usize, stats: &mut Stats) {
    let rels: Vec<usize> = f.schema.in_stratum(s).collect();
    let views: Vec<Option<VSrc>> = (0..NREL).map(|r| (f.schema.rels[r].stratum < s).then(|| VSrc(f.view(r)))).collect();
    let mut mems: BTreeMap<usize, Mem> = rels.iter().map(|&r| (r, Mem::new(f.schema.rels[r].spec.widths.len()))).collect();
    let rules: Vec<&Rule> = prog.iter().filter(|r| rels.contains(&r.head)).collect();
    fixpoint(&rules, &rels, &views, &mut mems, None, stats);
    for &r in &rels {
        f.create(r);
        push_all(f, r, std::mem::take(&mut mems.get_mut(&r).unwrap().rows).into_iter());
        f.freeze_rel(r);
    }
}

/// Semi-naive: the rules' heads in `rels` (held in `mems`, read as they grow), from `start` (rows already in `mems`,
/// the first wave's delta) or from the rules that read none of `rels`. Other relations are read in `views`.
fn fixpoint(rules: &[&Rule], rels: &[usize], views: &[Option<VSrc>], mems: &mut BTreeMap<usize, Mem>, start: Option<BTreeMap<usize, Vec<Vec<i64>>>>, stats: &mut Stats) {
    let mut delta: BTreeMap<usize, Vec<Vec<i64>>> = match start {
        Some(d) => d,
        None => {
            let mut d: BTreeMap<usize, Vec<Vec<i64>>> = BTreeMap::new();
            for rule in rules.iter().filter(|r| !rels.iter().any(|&h| reads(r, h))) {
                let srcs = srcs_of(views);
                let mut found = Vec::new();
                solve(rule, (1 << rule.body.len()) - 1, &mut [None; 8], &srcs, stats, &mut |e| {
                    found.push(head_of(rule, e));
                    true
                });
                for h in found {
                    if mems.get_mut(&rule.head).unwrap().insert(&h) {
                        d.entry(rule.head).or_default().push(h);
                    }
                }
            }
            d
        }
    };
    while delta.values().any(|d| !d.is_empty()) {
        let mut next: BTreeMap<usize, Vec<Vec<i64>>> = BTreeMap::new();
        for rule in rules {
            for (i, l) in rule.body.iter().enumerate() {
                let L::A(r, _) = l else { continue };
                let Some(rows) = delta.get(r).filter(|_| rels.contains(r)) else { continue };
                let mut found = Vec::new();
                {
                    let mut srcs = srcs_of(views);
                    for (&m, mem) in mems.iter() {
                        srcs[m] = mem;
                    }
                    from_seed(rule, i, rows, &srcs, stats, &mut |h| found.push(h));
                }
                for h in found {
                    if mems.get_mut(&rule.head).unwrap().insert(&h) {
                        next.entry(rule.head).or_default().push(h);
                    }
                }
            }
        }
        delta = next;
    }
}

/// A full block derived from the documents: the oracle, and the cold run.
fn cold(sc: &Arc<Schema>, prog: &[Rule], docs: &[Doc], stats: &mut Stats) -> FrozenFacts {
    let f = Facts::full(1, sc.clone(), reserve());
    f.begin(0);
    for (k, &r) in INPUTS.iter().enumerate() {
        f.create(r);
        push_all(&f, r, docs.iter().flat_map(|d| d.rows[k].iter().cloned()));
        f.freeze_rel(r);
    }
    f.end(0);
    for s in 1..sc.nstrata {
        f.begin(s);
        derive_stratum(&f, prog, s, stats);
        f.end(s);
    }
    seal(f)
}

fn seal(f: Facts) -> FrozenFacts {
    let bytes: usize = (0..f.schema.rels.len()).filter_map(|r| f.rel(r)).map(|r| r.bytes()).sum();
    f.charge(bytes);
    f.freeze()
}

/// The id of `row` in `v`, if live.
fn find(v: &View, row: &[i64]) -> Option<u32> {
    v.probe_iter(0, &row[..1]).find(|&id| (0..row.len()).all(|c| v.get(id, c) == row[c]))
}

/// Where this run's changes of each relation start: its rows and kills once carried.
#[derive(Clone, Copy, Default)]
struct Mark {
    rows: u32,
    kills: u32,
}

/// A relation's change in this run, read off the delta: the rows it lost (from the old block) and gained.
#[derive(Default)]
struct Change {
    removed: Vec<Vec<i64>>,
    added: Vec<Vec<i64>>,
}

fn change(old: &FrozenFacts, f: &Facts, r: usize, m: Mark) -> Change {
    let rel = f.rel(r).unwrap();
    let (ov, nv) = (old.view(r), f.view(r));
    let removed = (m.kills..rel.killed.len()).map(|k| ov.row(rel.killed.get(k, 0) as u32)).collect();
    let added = (rel.base_rows + m.rows..rel.base_rows + rel.rows.len()).filter(|&id| nv.live(id)).map(|id| nv.row(id)).collect();
    Change { removed, added }
}

/// A delta of `kept`, opened: its base, and the block to carry from.
fn open_delta(kept: &FrozenFacts) -> (Facts, Option<FrozenFacts>) {
    let (base, old) = match &kept.base {
        Some(b) => ((**b).clone(), Some(kept.clone())),
        None => (kept.clone(), None),
    };
    (Facts::open(rerail_blocks::block::kind::FACTS, 1, kept.schema.clone(), reserve(), Some(base)), old)
}

/// Makes relation `r` in the delta, carried from the last delta when there is one; returns its mark.
fn make(f: &Facts, carry_from: &Option<FrozenFacts>, r: usize) -> Mark {
    match carry_from {
        Some(o) => _ = f.carry(o, r),
        None => _ = f.create(r),
    }
    let rel = f.rel(r).unwrap();
    Mark { rows: rel.rows.len(), kills: rel.killed.len() }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Way {
    Again,
    Delta,
    DeltaDred,
    DeltaPart,
    DeltaLevels,
}

/// One edit's run: the inputs' deltas, then every derived stratum by `way`. Returns the new kept block and the
/// instructions of the derived strata.
fn maintained(way: Way, kept: &FrozenFacts, prog: &[Rule], docs: &[Doc], edited: &[usize], lv: &Levels, stats: &mut Stats) -> (FrozenFacts, u64) {
    let sc = kept.schema.clone();
    let (f, from) = open_delta(kept);
    let mut marks = [Mark::default(); NREL];
    let mut changes: Vec<Change> = (0..NREL).map(|_| Change::default()).collect();
    // The inputs: the edited documents' rows diffed against the old block's.
    f.begin(0);
    for (k, &r) in INPUTS.iter().enumerate() {
        marks[r] = make(&f, &from, r);
        let ov = kept.view(r);
        let w = f.w(r);
        for &d in edited {
            let mut was: Vec<(Vec<i64>, u32)> = ov.probe_iter(0, &[d as i64]).map(|id| (ov.row(id), id)).collect();
            was.sort();
            let now = &docs[d].rows[k];
            let (mut i, mut j) = (0, 0);
            while i < was.len() || j < now.len() {
                match (was.get(i), now.get(j)) {
                    (Some((a, _)), Some(b)) if a == b => (i, j) = (i + 1, j + 1),
                    (Some((a, id)), Some(b)) if a < b => {
                        w.kill(*id);
                        i += 1;
                    }
                    (Some((_, id)), None) => {
                        w.kill(*id);
                        i += 1;
                    }
                    (_, Some(b)) => {
                        w.push(b);
                        j += 1;
                    }
                    (None, None) => unreachable!(),
                }
            }
        }
        f.freeze_rel(r);
        changes[r] = change(kept, &f, r, marks[r]);
    }
    f.end(0);
    let t0 = retired();
    let old_views: Vec<Option<VSrc>> = (0..NREL).map(|r| Some(VSrc(kept.view(r)))).collect();
    let old_srcs = srcs_of(&old_views);
    for s in 1..sc.nstrata {
        f.begin(s);
        let rels: Vec<usize> = sc.in_stratum(s).collect();
        let rules: Vec<&Rule> = prog.iter().filter(|r| rels.contains(&r.head)).collect();
        let recursive = rules.iter().any(|r| rels.iter().any(|&h| reads(r, h)));
        let moved = rules.iter().any(|r| (0..NREL).any(|x| reads(r, x) && (!changes[x].removed.is_empty() || !changes[x].added.is_empty())));
        for &r in &rels {
            marks[r] = make(&f, &from, r);
        }
        if moved {
            let new_views: Vec<Option<VSrc>> = (0..NREL).map(|r| (sc.rels[r].stratum < s).then(|| VSrc(f.view(r)))).collect();
            let (kill, add) = match (way, recursive) {
                (Way::Again, _) | (Way::Delta, true) => again(&sc, &rules, &rels, &new_views, kept, stats),
                (_, false) => by_deltas(&rules, &rels, &changes, &old_srcs, &new_views, kept, stats),
                (Way::DeltaDred, true) => dred(&rules, &rels, &changes, Views { old: &old_views, new: &new_views }, kept, stats),
                (Way::DeltaPart, true) => match partition_col(&rules, &rels) {
                    true => by_partition(&rules, &rels, &changes, &old_srcs, &new_views, kept, stats),
                    false => again(&sc, &rules, &rels, &new_views, kept, stats),
                },
                (Way::DeltaLevels, true) => by_levels(&rules, &rels, &changes, Views { old: &old_views, new: &new_views }, kept, lv, stats),
            };
            for &r in &rels {
                let w = f.w(r);
                let ov = kept.view(r);
                for row in kill.get(&r).into_iter().flatten() {
                    w.kill(find(&ov, row).expect("a killed row is live in the old block"));
                }
                for row in add.get(&r).into_iter().flatten() {
                    w.push(row);
                }
                stats.kills += kill.get(&r).map_or(0, |k| k.len() as u64);
                stats.adds += add.get(&r).map_or(0, |k| k.len() as u64);
            }
        }
        for &r in &rels {
            f.freeze_rel(r);
            changes[r] = change(kept, &f, r, marks[r]);
        }
        f.end(s);
    }
    let derived = retired() - t0;
    (seal(f), derived)
}

type Rows = BTreeMap<usize, Vec<Vec<i64>>>;

/// The stratum derived again into scratch from the new state, and diffed against the old block's rows.
fn again(sc: &Arc<Schema>, rules: &[&Rule], rels: &[usize], new_views: &[Option<VSrc>], kept: &FrozenFacts, stats: &mut Stats) -> (Rows, Rows) {
    let mut mems: BTreeMap<usize, Mem> = rels.iter().map(|&r| (r, Mem::new(sc.rels[r].spec.widths.len()))).collect();
    fixpoint(rules, rels, new_views, &mut mems, None, stats);
    let (mut kill, mut add) = (Rows::new(), Rows::new());
    for &r in rels {
        let ov = kept.view(r);
        let mem = &mems[&r];
        let mut old_rows = HashSet::new();
        for id in ov.ordered_iter() {
            let row = ov.row(id);
            if !mem.has(&row) {
                kill.entry(r).or_default().push(row.clone());
            }
            old_rows.insert(row);
        }
        for row in &mem.rows {
            if !old_rows.contains(row) {
                add.entry(r).or_default().push(row.clone());
            }
        }
    }
    (kill, add)
}

/// A non-recursive stratum from its reads' deltas. Candidates for removal: the heads a removed row (or a row added
/// to a negated read) took part in, over the old state. Each is rechecked over the new state, and killed when no
/// rule derives it. Additions: the heads an added row (or a row removed from a negated read) makes over the new
/// state, less those the old block holds.
fn by_deltas(rules: &[&Rule], rels: &[usize], changes: &[Change], old: &[&dyn Src], new_views: &[Option<VSrc>], kept: &FrozenFacts, stats: &mut Stats) -> (Rows, Rows) {
    let new = srcs_of(new_views);
    let mut cand: BTreeMap<usize, HashSet<Vec<i64>>> = BTreeMap::new();
    let mut ins: BTreeMap<usize, HashSet<Vec<i64>>> = BTreeMap::new();
    for rule in rules {
        for (i, l) in rule.body.iter().enumerate() {
            let (r, neg) = match l {
                L::A(r, _) => (*r, false),
                L::N(r, _) => (*r, true),
                L::Ne(..) => continue,
            };
            let ch = &changes[r];
            let (lose, gain) = if neg { (&ch.added, &ch.removed) } else { (&ch.removed, &ch.added) };
            if !lose.is_empty() {
                let c = cand.entry(rule.head).or_default();
                from_seed(rule, i, lose, old, stats, &mut |h| _ = c.insert(h));
            }
            if !gain.is_empty() {
                let c = ins.entry(rule.head).or_default();
                from_seed(rule, i, gain, &new, stats, &mut |h| _ = c.insert(h));
            }
        }
    }
    let (mut kill, mut add) = (Rows::new(), Rows::new());
    for &r in rels {
        let ov = kept.view(r);
        let ins_r = ins.remove(&r).unwrap_or_default();
        for h in cand.remove(&r).unwrap_or_default() {
            stats.candidates += 1;
            if ins_r.contains(&h) || find(&ov, &h).is_none() {
                continue;
            }
            if !rules.iter().filter(|x| x.head == r).any(|x| derives(x, &h, &new, stats)) {
                kill.entry(r).or_default().push(h);
            }
        }
        for h in ins_r {
            if find(&ov, &h).is_none() {
                add.entry(r).or_default().push(h);
            }
        }
    }
    (kill, add)
}

/// The views of every relation over the old state and over the new one (`None`: not yet made in the new), which a
/// recursive stratum's maintenance reads both of.
#[derive(Clone, Copy)]
struct Views<'a, 'v> {
    old: &'a [Option<VSrc<'v>>],
    new: &'a [Option<VSrc<'v>>],
}

/// A recursive stratum by delete and rederive, from the deltas alone. Overdelete: the old rows a removed read
/// reached, and what they reach in turn, over the old state. Rederive: of those, the rows some rule still derives
/// directly over the new state, the stratum's own rows being the old ones less the overdeleted. Insert: semi-naive
/// from those and the added reads, over that state.
fn dred(rules: &[&Rule], rels: &[usize], changes: &[Change], views: Views<'_, '_>, kept: &FrozenFacts, stats: &mut Stats) -> (Rows, Rows) {
    let old = srcs_of(views.old);
    let own = |r: usize| rels.contains(&r);
    // Overdelete, over the old state.
    let mut del: BTreeMap<usize, HashSet<Vec<i64>>> = rels.iter().map(|&r| (r, HashSet::new())).collect();
    let mut wave: Rows = Rows::new();
    let old_has = |r: usize, h: &[i64]| find(&kept.view(r), h).is_some();
    for rule in rules {
        for (i, l) in rule.body.iter().enumerate() {
            let (r, neg) = match l {
                L::A(r, _) => (*r, false),
                L::N(r, _) => (*r, true),
                L::Ne(..) => continue,
            };
            if own(r) {
                continue;
            }
            let lose = if neg { &changes[r].added } else { &changes[r].removed };
            if lose.is_empty() {
                continue;
            }
            let mut found = Vec::new();
            from_seed(rule, i, lose, &old, stats, &mut |h| found.push(h));
            for h in found {
                if old_has(rule.head, &h) && del.get_mut(&rule.head).unwrap().insert(h.clone()) {
                    wave.entry(rule.head).or_default().push(h);
                }
            }
        }
    }
    while wave.values().any(|w| !w.is_empty()) {
        let mut next = Rows::new();
        for rule in rules {
            for (i, l) in rule.body.iter().enumerate() {
                let L::A(r, _) = l else { continue };
                let Some(rows) = wave.get(r).filter(|_| own(*r)) else { continue };
                let mut found = Vec::new();
                from_seed(rule, i, rows, &old, stats, &mut |h| found.push(h));
                for h in found {
                    if old_has(rule.head, &h) && del.get_mut(&rule.head).unwrap().insert(h.clone()) {
                        next.entry(rule.head).or_default().push(h);
                    }
                }
            }
        }
        wave = next;
    }
    stats.candidates += del.values().map(|d| d.len() as u64).sum::<u64>();
    // Rederive, one pass over the new state with the stratum's rows the old less `del`: the overdeleted rows some
    // rule still derives directly. What they support comes back in the insert phase, as additions do.
    let mut plus: BTreeMap<usize, Mem> = rels.iter().map(|&r| (r, Mem::new(kept.view(r).arity()))).collect();
    let mut start = Rows::new();
    {
        let overlays: Vec<(usize, Overlay)> = rels.iter().map(|&r| (r, Overlay { base: old[r], minus: &del[&r], plus: &plus[&r] })).collect();
        let mut srcs = srcs_of(views.new);
        for (r, o) in &overlays {
            srcs[*r] = o;
        }
        for &r in rels {
            for h in &del[&r] {
                if rules.iter().filter(|x| x.head == r).any(|x| derives(x, h, &srcs, stats)) {
                    start.entry(r).or_default().push(h.clone());
                }
            }
        }
        // Insert: what the added reads make over that state.
        for rule in rules {
            for (i, l) in rule.body.iter().enumerate() {
                let (r, neg) = match l {
                    L::A(r, _) => (*r, false),
                    L::N(r, _) => (*r, true),
                    L::Ne(..) => continue,
                };
                if own(r) {
                    continue;
                }
                let gain = if neg { &changes[r].removed } else { &changes[r].added };
                if gain.is_empty() {
                    continue;
                }
                from_seed(rule, i, gain, &srcs, stats, &mut |h| {
                    if !srcs[rule.head].contains(&h) {
                        start.entry(rule.head).or_default().push(h);
                    }
                });
            }
        }
    }
    let mut start2 = Rows::new();
    for (r, hs) in start {
        for h in hs {
            if plus.get_mut(&r).unwrap().insert(&h) {
                start2.entry(r).or_default().push(h);
            }
        }
    }
    // The semi-naive waves over overlays whose `plus` grows: one wave at a time.
    let mut delta = start2;
    while delta.values().any(|d| !d.is_empty()) {
        let mut next = Rows::new();
        let mut found_all: Vec<(usize, Vec<i64>)> = Vec::new();
        {
            let overlays: Vec<(usize, Overlay)> = rels.iter().map(|&r| (r, Overlay { base: old[r], minus: &del[&r], plus: &plus[&r] })).collect();
            let mut srcs = srcs_of(views.new);
            for (r, o) in &overlays {
                srcs[*r] = o;
            }
            for rule in rules {
                for (i, l) in rule.body.iter().enumerate() {
                    let L::A(r, _) = l else { continue };
                    let Some(rows) = delta.get(r).filter(|_| own(*r)) else { continue };
                    from_seed(rule, i, rows, &srcs, stats, &mut |h| {
                        if !srcs[rule.head].contains(&h) {
                            found_all.push((rule.head, h));
                        }
                    });
                }
            }
        }
        for (r, h) in found_all {
            if plus.get_mut(&r).unwrap().insert(&h) {
                next.entry(r).or_default().push(h);
            }
        }
        delta = next;
    }
    // Kills: overdeleted and not back. Adds: in `plus` and not rows of the old block (a rederived row is one).
    let (mut kill, mut add) = (Rows::new(), Rows::new());
    for &r in rels {
        for h in &del[&r] {
            if !plus[&r].has(h) {
                kill.entry(r).or_default().push(h.clone());
            }
        }
        for h in &plus[&r].rows {
            if !del[&r].contains(h) && !old_has(r, h) {
                add.entry(r).or_default().push(h.clone());
            }
        }
    }
    (kill, add)
}

/// Whether the stratum is partitioned by its relations' first column: in every rule, the head's first argument is a
/// variable that is the first argument of every atom of the stratum's own relations. Then a key's rows depend only
/// on rows of the same key and on other strata.
fn partition_col(rules: &[&Rule], rels: &[usize]) -> bool {
    rules.iter().all(|r| match r.args.first() {
        Some(T::V(x)) => r.body.iter().all(|l| match l {
            L::A(a, args) | L::N(a, args) if rels.contains(a) => args.first() == Some(&T::V(*x)),
            _ => true,
        }),
        _ => false,
    })
}

/// A recursive stratum partitioned by its first column, from the deltas: the keys a changed read reaches (the heads'
/// first columns its rows make, over the old state for removed rows and over the new for added ones) are derived
/// again, each with its key bound, and diffed against the old block's rows of those keys. No other key is read.
fn by_partition(rules: &[&Rule], rels: &[usize], changes: &[Change], old: &[&dyn Src], new_views: &[Option<VSrc>], kept: &FrozenFacts, stats: &mut Stats) -> (Rows, Rows) {
    let own = |r: usize| rels.contains(&r);
    let new = srcs_of(new_views);
    let mut keys: HashSet<i64> = HashSet::new();
    for rule in rules {
        for (i, l) in rule.body.iter().enumerate() {
            let (r, neg) = match l {
                L::A(r, _) => (*r, false),
                L::N(r, _) => (*r, true),
                L::Ne(..) => continue,
            };
            if own(r) {
                continue;
            }
            let (lose, gain) = if neg { (&changes[r].added, &changes[r].removed) } else { (&changes[r].removed, &changes[r].added) };
            // A seed's heads over the old state: the stratum's own atoms read the old block.
            from_seed(rule, i, lose, old, stats, &mut |h| _ = keys.insert(h[0]));
            // Over the new state the own atoms are not known yet; the old block's rows stand in. A key the change
            // reaches only through rows the edit adds is reached through an earlier key's rows, already a key here.
            from_seed(rule, i, gain, &mixed(old, &new, rels), stats, &mut |h| _ = keys.insert(h[0]));
        }
    }
    stats.candidates += keys.len() as u64;
    let (mut kill, mut add) = (Rows::new(), Rows::new());
    for &k in &keys {
        let mut mems: BTreeMap<usize, Mem> = rels.iter().map(|&r| (r, Mem::new(kept.view(r).arity()))).collect();
        let mut start = Rows::new();
        for rule in rules.iter().filter(|r| !rels.iter().any(|&h| reads(r, h))) {
            let T::V(x) = rule.args[0] else { unreachable!() };
            let mut env = [None; 8];
            env[x] = Some(k);
            let mut found = Vec::new();
            solve(rule, (1 << rule.body.len()) - 1, &mut env, &new, stats, &mut |e| {
                found.push(head_of(rule, e));
                true
            });
            for h in found {
                if mems.get_mut(&rule.head).unwrap().insert(&h) {
                    start.entry(rule.head).or_default().push(h);
                }
            }
        }
        fixpoint(rules, rels, new_views, &mut mems, Some(start), stats);
        for &r in rels {
            let ov = kept.view(r);
            let mem = &mems[&r];
            let mut was = HashSet::new();
            for id in ov.probe_iter(0, &[k]) {
                let row = ov.row(id);
                if !mem.has(&row) {
                    kill.entry(r).or_default().push(row.clone());
                }
                was.insert(row);
            }
            for row in &mem.rows {
                if !was.contains(row) {
                    add.entry(r).or_default().push(row.clone());
                }
            }
        }
    }
    (kill, add)
}

/// The new state for the other strata's relations and the old block's for the stratum's own.
fn mixed<'a>(old: &[&'a dyn Src], new: &[&'a dyn Src], rels: &[usize]) -> Vec<&'a dyn Src> {
    (0..NREL).map(|r| if rels.contains(&r) { old[r] } else { new[r] }).collect()
}

/// Each recursive stratum's rows and their levels: a base rule's rows 1, a row first derived in wave `k` level `k`.
type Levels = BTreeMap<usize, HashMap<Vec<i64>, u32>>;

/// The levels of `kept`'s recursive strata, from scratch (outside the timed part: in a block a column, kept).
fn levels_of(kept: &FrozenFacts, prog: &[Rule]) -> Levels {
    let sc = &kept.schema;
    let mut out = Levels::new();
    let mut st = Stats::default();
    for s in 1..sc.nstrata {
        let rels: Vec<usize> = sc.in_stratum(s).collect();
        let rules: Vec<&Rule> = prog.iter().filter(|r| rels.contains(&r.head)).collect();
        if !rules.iter().any(|r| rels.iter().any(|&h| reads(r, h))) {
            continue;
        }
        let views: Vec<Option<VSrc>> = (0..NREL).map(|r| (sc.rels[r].stratum < s).then(|| VSrc(kept.view(r)))).collect();
        let mut mems: BTreeMap<usize, Mem> = rels.iter().map(|&r| (r, Mem::new(sc.rels[r].spec.widths.len()))).collect();
        let mut lv: BTreeMap<usize, HashMap<Vec<i64>, u32>> = rels.iter().map(|&r| (r, HashMap::new())).collect();
        let mut delta = Rows::new();
        for rule in rules.iter().filter(|r| !rels.iter().any(|&h| reads(r, h))) {
            let srcs = srcs_of(&views);
            let mut found = Vec::new();
            solve(rule, (1 << rule.body.len()) - 1, &mut [None; 8], &srcs, &mut st, &mut |e| {
                found.push(head_of(rule, e));
                true
            });
            for h in found {
                if mems.get_mut(&rule.head).unwrap().insert(&h) {
                    lv.get_mut(&rule.head).unwrap().insert(h.clone(), 1);
                    delta.entry(rule.head).or_default().push(h);
                }
            }
        }
        let mut k = 1;
        while delta.values().any(|d| !d.is_empty()) {
            k += 1;
            let mut next = Rows::new();
            for rule in &rules {
                for (i, l) in rule.body.iter().enumerate() {
                    let L::A(r, _) = l else { continue };
                    let Some(rows) = delta.get(r).filter(|_| rels.contains(r)) else { continue };
                    let mut found = Vec::new();
                    {
                        let mut srcs = srcs_of(&views);
                        for (&m, mem) in mems.iter() {
                            srcs[m] = mem;
                        }
                        from_seed(rule, i, rows, &srcs, &mut st, &mut |h| found.push(h));
                    }
                    for h in found {
                        if mems.get_mut(&rule.head).unwrap().insert(&h) {
                            lv.get_mut(&rule.head).unwrap().insert(h.clone(), k);
                            next.entry(rule.head).or_default().push(h);
                        }
                    }
                }
            }
            delta = next;
        }
        out.extend(lv);
    }
    out
}

/// A stratum's own relation read below a level: its rows (less `minus`, plus `plus`) whose level is under `bound`.
struct Below<'a> {
    base: &'a dyn Src,
    minus: &'a HashSet<Vec<i64>>,
    plus: &'a Mem,
    lv: &'a HashMap<Vec<i64>, u32>,
    bound: &'a std::cell::Cell<u32>,
}

impl Below<'_> {
    fn level(&self, r: &[i64]) -> u32 {
        self.lv.get(r).copied().unwrap_or(u32::MAX)
    }
}

impl Src for Below<'_> {
    fn probe(&self, cols: &[usize], key: &[i64], f: &mut dyn FnMut(&[i64]) -> bool) -> bool {
        let b = self.bound.get();
        let minus = self.minus;
        self.base.probe(cols, key, &mut |r| minus.contains(r) || self.level(r) >= b || f(r))
            && self.plus.probe(cols, key, &mut |r| self.level(r) >= b || f(r))
    }
}

/// The level a derivation of `rule` under `env` gives: one more than its own atoms' highest (their rows rebuilt from
/// `env`), 1 when it reads none of them.
fn level_of(rule: &Rule, env: &[Option<i64>; 8], rels: &[usize], lv: &BTreeMap<usize, HashMap<Vec<i64>, u32>>) -> u32 {
    let mut top = 0;
    for l in &rule.body {
        if let L::A(r, args) = l
            && rels.contains(r)
        {
            let row: Vec<i64> = args.iter().map(|t| match t {
                T::V(v) => env[*v].unwrap(),
                T::W => panic!("a wildcard in a recursive atom: its row cannot be rebuilt"),
            }).collect();
            top = top.max(lv[r][&row]);
        }
    }
    top + 1
}

/// A recursive stratum by *levels, checked* (0090). Candidates: the old rows a removed read took part in, over the
/// old state. In level order, each survives when some rule derives it from rows of lower level that survive (and the
/// new state's other reads); a row that does not is killed, and the rows it took part in become candidates in turn.
/// Then the killed rows some rule derives at any level, and the added reads' heads, are inserted semi-naive, each at
/// one more than its derivation's highest level. Kept rows keep their levels.
fn by_levels(rules: &[&Rule], rels: &[usize], changes: &[Change], views: Views<'_, '_>, kept: &FrozenFacts, lv0: &Levels, stats: &mut Stats) -> (Rows, Rows) {
    let old = srcs_of(views.old);
    let own = |r: usize| rels.contains(&r);
    let old_has = |r: usize, h: &[i64]| lv0[&r].contains_key(h);
    let mut lv: Levels = rels.iter().map(|&r| (r, lv0[&r].clone())).collect();
    let mut heap: std::collections::BinaryHeap<std::cmp::Reverse<(u32, usize, Vec<i64>)>> = Default::default();
    let mut seen: HashSet<(usize, Vec<i64>)> = HashSet::new();
    let mut push = |heap: &mut std::collections::BinaryHeap<_>, r: usize, h: Vec<i64>, lv: &Levels| {
        if old_has(r, &h) && seen.insert((r, h.clone())) {
            heap.push(std::cmp::Reverse((lv[&r][&h], r, h)));
        }
    };
    for rule in rules {
        for (i, l) in rule.body.iter().enumerate() {
            let (r, neg) = match l {
                L::A(r, _) => (*r, false),
                L::N(r, _) => (*r, true),
                L::Ne(..) => continue,
            };
            if own(r) {
                continue;
            }
            let lose = if neg { &changes[r].added } else { &changes[r].removed };
            let mut found = Vec::new();
            from_seed(rule, i, lose, &old, stats, &mut |h| found.push(h));
            for h in found {
                push(&mut heap, rule.head, h, &lv);
            }
        }
    }
    let mut killed: BTreeMap<usize, HashSet<Vec<i64>>> = rels.iter().map(|&r| (r, HashSet::new())).collect();
    let empty: BTreeMap<usize, Mem> = rels.iter().map(|&r| (r, Mem::new(kept.view(r).arity()))).collect();
    let bound = std::cell::Cell::new(0u32);
    while let Some(std::cmp::Reverse((l, r, h))) = heap.pop() {
        stats.candidates += 1;
        let survives = {
            let belows: Vec<(usize, Below)> = rels.iter().map(|&x| (x, Below { base: old[x], minus: &killed[&x], plus: &empty[&x], lv: &lv[&x], bound: &bound })).collect();
            let mut srcs = srcs_of(views.new);
            for (x, b) in &belows {
                srcs[*x] = b;
            }
            bound.set(l);
            rules.iter().filter(|x| x.head == r).any(|x| derives(x, &h, &srcs, stats))
        };
        if survives {
            continue;
        }
        killed.get_mut(&r).unwrap().insert(h.clone());
        for rule in rules {
            for (i, a) in rule.body.iter().enumerate() {
                if !matches!(a, L::A(x, _) if *x == r) {
                    continue;
                }
                let mut found = Vec::new();
                from_seed(rule, i, std::slice::from_ref(&h), &old, stats, &mut |x| found.push(x));
                for x in found {
                    if !killed[&rule.head].contains(&x) {
                        push(&mut heap, rule.head, x, &lv);
                    }
                }
            }
        }
    }
    // Insert: the killed rows derivable at any level, and the added reads' heads, then semi-naive. A row's level is
    // one more than its derivation's highest.
    let mut plus: BTreeMap<usize, Mem> = rels.iter().map(|&r| (r, Mem::new(kept.view(r).arity()))).collect();
    for &r in rels {
        for h in &killed[&r] {
            lv.get_mut(&r).unwrap().remove(h);
        }
    }
    let mut wave: Vec<(usize, Vec<i64>, u32)> = Vec::new();
    {
        let overlays: Vec<(usize, Overlay)> = rels.iter().map(|&r| (r, Overlay { base: old[r], minus: &killed[&r], plus: &plus[&r] })).collect();
        let mut srcs = srcs_of(views.new);
        for (r, o) in &overlays {
            srcs[*r] = o;
        }
        let all = |rule: &Rule| (1u32 << rule.body.len()) - 1;
        for &r in rels {
            for h in &killed[&r] {
                for rule in rules.iter().filter(|x| x.head == r) {
                    let mut env = [None; 8];
                    for (t, &x) in rule.args.iter().zip(h) {
                        if let T::V(v) = t {
                            env[*v] = Some(x);
                        }
                    }
                    let mut best = None::<u32>;
                    solve(rule, all(rule), &mut env, &srcs, stats, &mut |e| {
                        let l = level_of(rule, e, rels, &lv);
                        best = Some(best.map_or(l, |b| b.min(l)));
                        true
                    });
                    if let Some(l) = best {
                        wave.push((r, h.clone(), l));
                    }
                }
            }
        }
        for rule in rules {
            for (i, l) in rule.body.iter().enumerate() {
                let (r, neg) = match l {
                    L::A(r, _) => (*r, false),
                    L::N(r, _) => (*r, true),
                    L::Ne(..) => continue,
                };
                if own(r) {
                    continue;
                }
                let gain = if neg { &changes[r].removed } else { &changes[r].added };
                let L::A(_, args) = &rule.body[i] else { continue };
                for row in gain {
                    let mut env = [None; 8];
                    for (t, &x) in args.iter().zip(row) {
                        if let T::V(v) = t {
                            env[*v] = Some(x);
                        }
                    }
                    solve(rule, all(rule) & !(1 << i), &mut env, &srcs, stats, &mut |e| {
                        let h = head_of(rule, e);
                        if !srcs[rule.head].contains(&h) {
                            wave.push((rule.head, h, level_of(rule, e, rels, &lv)));
                        }
                        true
                    });
                }
            }
        }
    }
    let mut level_writes = 0u64;
    let mut delta = Rows::new();
    for (r, h, l) in wave {
        if plus.get_mut(&r).unwrap().insert(&h) {
            lv.get_mut(&r).unwrap().insert(h.clone(), l);
            level_writes += 1;
            delta.entry(r).or_default().push(h);
        }
    }
    while delta.values().any(|d| !d.is_empty()) {
        let mut found_all: Vec<(usize, Vec<i64>, u32)> = Vec::new();
        {
            let overlays: Vec<(usize, Overlay)> = rels.iter().map(|&r| (r, Overlay { base: old[r], minus: &killed[&r], plus: &plus[&r] })).collect();
            let mut srcs = srcs_of(views.new);
            for (r, o) in &overlays {
                srcs[*r] = o;
            }
            for rule in rules {
                for (i, l) in rule.body.iter().enumerate() {
                    let L::A(r, args) = l else { continue };
                    let Some(rows) = delta.get(r).filter(|_| own(*r)) else { continue };
                    for row in rows {
                        let mut env = [None; 8];
                        for (t, &x) in args.iter().zip(row) {
                            if let T::V(v) = t {
                                env[*v] = Some(x);
                            }
                        }
                        solve(rule, ((1u32 << rule.body.len()) - 1) & !(1 << i), &mut env, &srcs, stats, &mut |e| {
                            let h = head_of(rule, e);
                            if !srcs[rule.head].contains(&h) {
                                found_all.push((rule.head, h, level_of(rule, e, rels, &lv)));
                            }
                            true
                        });
                    }
                }
            }
        }
        let mut next = Rows::new();
        for (r, h, l) in found_all {
            if plus.get_mut(&r).unwrap().insert(&h) {
                lv.get_mut(&r).unwrap().insert(h.clone(), l);
                level_writes += 1;
                next.entry(r).or_default().push(h);
            }
        }
        delta = next;
    }
    stats.level_writes += level_writes;
    let (mut kill, mut add) = (Rows::new(), Rows::new());
    for &r in rels {
        for h in &killed[&r] {
            if !plus[&r].has(h) {
                kill.entry(r).or_default().push(h.clone());
            }
        }
        for h in &plus[&r].rows {
            if !killed[&r].contains(h) && !old_has(r, h) {
                add.entry(r).or_default().push(h.clone());
            }
        }
    }
    (kill, add)
}

/// `MAINTAIN_EDGES=DIR`: `edge`'s rows of state `n` to `DIR/edge-<n>.tsv` (the cold state 00, then each edit's), for
/// counting yes/no/maybe on this closure (`scripts/profiling/o125/recursion/ynm_reach.py`).
fn dump_edges(f: &FrozenFacts, n: usize) {
    let Ok(dir) = std::env::var("MAINTAIN_EDGES") else { return };
    let mut out = String::new();
    VSrc(f.view(EDGE)).probe(&[], &[], &mut |r| {
        out.push_str(&format!("{}\t{}\n", r[0], r[1]));
        true
    });
    std::fs::write(format!("{dir}/edge-{n:02}.tsv"), out).expect("write the edges");
}

fn digests(f: &FrozenFacts) -> Vec<u64> {
    (0..NREL).map(|r| f.view(r).digest()).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let ndocs: u64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(300);
    let nedits: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(8);
    let sc = schema();
    let prog = program();
    let mut docs: Vec<Doc> = (0..ndocs).map(make_doc).collect();

    let mut st = Stats::default();
    let t = retired();
    let mut kept = cold(&sc, &prog, &docs, &mut st);
    let cold_insn = retired() - t;
    dump_edges(&kept, 0);
    let sizes: Vec<String> = (0..NREL).map(|r| format!("{} {}", sc.rels[r].name, kept.view(r).live_len())).collect();
    println!("{ndocs} documents; cold {:.1} M insn; rows: {}", cold_insn as f64 / 1e6, sizes.join(", "));

    let mut r = rng(11);
    let mut sum: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    println!("{:>4} {:>5} {:>10} {:>10} {:>10} {:>10} {:>10}  part: keys, rechecks, kills, adds; dred: overdeleted, rechecks; levels: candidates, level writes", "edit", "doc", "again M", "delta M", "+dred M", "+part M", "+levels M");
    for e in 1..=nedits + 1 {
        let d = (r() % ndocs) as usize;
        if e == nedits + 1 {
            // The big edit: a document's every definition removed (a module deleted), its importers' edges gone.
            docs[d].rows[0].clear();
            println!("     (a big edit: every definition of doc {d} removed)");
        } else {
            edit(&mut docs[d], d as u64, 1000 + e as u64);
        }
        let mut so = Stats::default();
        let oracle = cold(&sc, &prog, &docs, &mut so);
        dump_edges(&oracle, e);
        let want = digests(&oracle);
        let mut out = Vec::new();
        let mut next = None;
        let mut shown = Stats::default();
        let mut dred_shown = Stats::default();
        let lv = levels_of(&kept, &prog);
        let mut lv_shown = Stats::default();
        for way in [Way::Again, Way::Delta, Way::DeltaDred, Way::DeltaPart, Way::DeltaLevels] {
            let mut s = Stats::default();
            let (f, insn) = maintained(way, &kept, &prog, &docs, &[d], &lv, &mut s);
            let got = digests(&f);
            for (i, (a, b)) in got.iter().zip(&want).enumerate() {
                assert_eq!(a, b, "edit {e}, {way:?}: {} differs from the derive from scratch", sc.rels[i].name);
            }
            let k = match way {
                Way::Again => "again",
                Way::Delta => "delta",
                Way::DeltaDred => "delta+dred",
                Way::DeltaPart => "delta+part",
                Way::DeltaLevels => "delta+levels",
            };
            let x = sum.entry(k).or_default();
            x.0 += insn;
            x.1 += 1;
            out.push(insn);
            if way == Way::DeltaPart {
                next = Some(f);
                shown = s;
            }
            if way == Way::DeltaDred {
                dred_shown = s;
            }
            if way == Way::DeltaLevels {
                lv_shown = s;
            }
        }
        kept = next.unwrap();
        let m = |i: usize| out[i] as f64 / 1e6;
        println!("{e:>4} {:>5} {:>10.2} {:>10.2} {:>10.2} {:>10.2} {:>10.2}  {}, {}, {}, {}; {}, {}; {}, {}", d, m(0), m(1), m(2), m(3), m(4), shown.candidates, shown.rechecks, shown.kills, shown.adds, dred_shown.candidates, dred_shown.rechecks, lv_shown.candidates, lv_shown.level_writes);
        // The roll-up rule (0266 §6).
        let (cost, base) = kept.cost();
        if cost >= base {
            let rolled = kept.rollup(reserve());
            assert_eq!(digests(&rolled), digests(&kept), "a roll-up reads the same");
            kept = rolled;
            println!("     rolled up");
        }
    }
    for (k, (insn, n)) in &sum {
        println!("{k:>11}: mean {:.2} M insn an edit's derived strata", *insn as f64 / 1e6 / *n as f64);
    }
    println!("checks: every edit, every way, every relation equal to the derive from scratch; footprint {} MB", footprint() >> 20);
}
