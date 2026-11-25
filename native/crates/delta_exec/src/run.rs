//! The four operators over K-form (decision 0686): a rule's steps as change operations, a stratum as a wave or a
//! fixpoint, strata in order, and a phase.
//!
//! **A rule.** Its steps run in order over the rows so far, held as registers while the steps run (nothing between
//! steps is stored); its head rows are written. Cold, every step reads its relation's state after the run. Across a
//! change, a rule's change is a sum of one term a step (0678 step 5, from two inputs to k): term `i` reads step `i`'s
//! change, the steps before it their states after, the steps after it their states before:
//!
//!   Π f_j(after) − Π f_j(before) = Σ_i [Π_{j<i} f_j(after)] · (f_i(after) − f_i(before)) · [Π_{j>i} f_j(before)]
//!
//! where `f_j` is step `j`'s factor for a binding (an atom's match, a negation's absence of one). A step whose input did
//! not change has no term (L4). Each term's derivations come gone or added, so a head row's derivations are counted
//! (`change::project`, 0678 step 9) and the head changes only where its count leaves or reaches 0.
//!
//! **A stratum** is a module's rules (`k_order`). One that does not read its own heads runs as one wave: by the terms
//! above when its heads' counts are kept, else by the reference strategy (its rules run on the after side, the heads'
//! counts made again, and the result diffed with each head's version), which is always allowed. One that reads its own
//! heads runs semi-naive to its fixpoint, from cold every time (the reference strategy: run the after side, diff with
//! the version); its heads keep no counts. A stratum that asks for loads runs the same way, answering them between
//! fixpoints until no document comes or goes (discovery as a fixpoint).
//!
//! **Waves.** A view's change is the change of the documents that changed, so a rule that reads a view does work for
//! the changed documents only, and none for the others: the single-file waves, read off the views' changes. Every
//! other rule's work is the cross-file wave's.

use crate::db::{Db, Doc, Text, read_row};
use crate::kform::{Arg, Plan, Program, Refused, StepKind, Words};
use crate::sym::Symbols;
use crate::tape::{self, Names, Reads};
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};
use rerail_blocks::change::{self, Sign, State, Tally};
use rerail_blocks::relation::{Relation, View};
use rerail_blocks::table::Table;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;
use std::ptr::NonNull;
use std::sync::Arc;

/// What answers a `load`: a path's bytes.
pub trait World {
    fn read(&self, path: &str) -> Option<String>;
}

/// No files.
pub struct NoWorld;

impl World for NoWorld {
    fn read(&self, _: &str) -> Option<String> {
        None
    }
}

/// The executor: the symbols, and the names it reads K-form and views by.
pub struct Exec<'s> {
    pub sym: &'s Symbols,
    pub words: Words,
    pub names: Names,
    views: FxHashMap<i64, tape::View>,
    load: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Before,
    After,
}

/// What one step reads.
enum Src<'a> {
    /// A relation of the level, across the run.
    Rel(Bistate<View<'a>>),
    /// An open relation of a fixpoint, rows `from .. to`.
    Open(&'a Relation, u32, u32),
    /// A view over the level's documents.
    View(tape::View),
    /// Nothing: a comparison's.
    None,
}

/// How a rule's steps read: all on one side, or the term of step `i` (before it after, after it before).
#[derive(Clone, Copy)]
enum Mode {
    All(Side),
    Term(usize),
}

impl<'s> Exec<'s> {
    pub fn new(sym: &'s Symbols) -> Exec<'s> {
        let views = tape::View::ALL.iter().map(|&(v, n, _)| (sym.intern(n), v)).collect();
        Exec { sym, words: Words::new(sym), names: Names::new(sym), views, load: sym.intern("load") }
    }

    fn view(&self, rel: i64) -> Option<tape::View> {
        self.views.get(&rel).copied()
    }

    // ---- reading a step's input -------------------------------------------------------------------------------

    /// Every row of `src` on `side` that agrees with `bound`.
    fn scan(&self, db: &Db, src: &Src<'_>, side: Side, bound: &[Option<i64>], each: &mut dyn FnMut(&[i64])) {
        match src {
            Src::Rel(b) => {
                let s = match side {
                    Side::Before => b.before().expect("a level's relation has a version"),
                    Side::After => b.after(),
                };
                scan_table(s, bound, each);
            }
            Src::Open(r, from, to) => {
                let v = View::full(r);
                let mut row = vec![0i64; bound.len()];
                for id in *from..*to {
                    read_row(&v, RowId::new(id), &mut row);
                    if fits(&row, bound) {
                        each(&row);
                    }
                }
            }
            Src::View(v) => {
                for (i, doc) in db.docs.iter().enumerate() {
                    let text = match side {
                        Side::Before => &doc.before,
                        Side::After => &doc.after,
                    };
                    if let Some(t) = text {
                        self.view_rows(*v, i as i64 + 1, doc, t, bound, each);
                    }
                }
            }
            Src::None => {}
        }
    }

    fn view_rows(&self, v: tape::View, d: i64, doc: &Doc, t: &Text, bound: &[Option<i64>], each: &mut dyn FnMut(&[i64])) {
        if bound[0].is_some_and(|b| b != d) {
            return;
        }
        tape::rows(v, d, doc.path, &Reads::of(t), self.sym, &self.names, bound, each);
    }

    /// The change of `src`'s rows that agree with `bound`: each gone (read before) or added (read after).
    fn changes(&self, db: &Db, src: &Src<'_>, bound: &[Option<i64>], each: &mut dyn FnMut(Sign, &[i64])) {
        match src {
            Src::Rel(b) => {
                let mut row = vec![0i64; bound.len()];
                let before = b.before().expect("a version");
                for id in b.gone() {
                    read_row(before, id, &mut row);
                    if fits(&row, bound) {
                        each(Sign::Gone, &row);
                    }
                }
                for id in b.added() {
                    read_row(b.after(), id, &mut row);
                    if fits(&row, bound) {
                        each(Sign::Added, &row);
                    }
                }
            }
            Src::View(v) => {
                for (i, doc) in db.docs.iter().enumerate().filter(|(_, d)| d.changed()) {
                    if let Some(t) = &doc.before {
                        self.view_rows(*v, i as i64 + 1, doc, t, bound, &mut |r| each(Sign::Gone, r));
                    }
                    if let Some(t) = &doc.after {
                        self.view_rows(*v, i as i64 + 1, doc, t, bound, &mut |r| each(Sign::Added, r));
                    }
                }
            }
            Src::Open(..) | Src::None => {}
        }
    }

    /// Whether `src` changed in the run.
    fn changed(&self, db: &Db, src: &Src<'_>) -> bool {
        match src {
            Src::Rel(b) => !b.is_identity(),
            Src::View(_) => db.docs_changed(),
            Src::Open(..) | Src::None => false,
        }
    }

    fn src<'a>(&self, db: &'a Db, rel: i64, kind: &StepKind) -> Src<'a> {
        match kind {
            StepKind::Cmp(_) => Src::None,
            _ => match self.view(rel) {
                Some(v) => Src::View(v),
                None => Src::Rel(db.bistate(rel)),
            },
        }
    }

    // ---- a rule ---------------------------------------------------------------------------------------------------

    /// Plan `p`'s derivations, each with its sign and its head row, its steps reading `srcs` as `mode` says.
    fn derive(&self, db: &Db, p: &Plan, srcs: &[Src<'_>], mode: Mode, emit: &mut dyn FnMut(i64, &[i64])) {
        let mut regs = vec![0i64; p.regs.max(1)];
        let mut head = vec![0i64; p.head_args.len()];
        self.walk(db, p, srcs, mode, 0, &mut regs, 1, &mut |sign, regs| {
            for (h, a) in head.iter_mut().zip(&p.head_args) {
                *h = match *a {
                    Arg::Const(c) => c,
                    Arg::Test(r) | Arg::Bind(r) | Arg::Again(r) => regs[r],
                    Arg::Wild => unreachable!("refused when read"),
                };
            }
            emit(sign, &head);
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(&self, db: &Db, p: &Plan, srcs: &[Src<'_>], mode: Mode, k: usize, regs: &mut [i64], sign: i64, emit: &mut dyn FnMut(i64, &[i64])) {
        let Some(step) = p.steps.get(k) else {
            emit(sign, regs);
            return;
        };
        let bound: Vec<Option<i64>> = step
            .args
            .iter()
            .map(|a| match *a {
                Arg::Const(c) => Some(c),
                Arg::Test(r) => Some(regs[r]),
                Arg::Bind(_) | Arg::Again(_) | Arg::Wild => None,
            })
            .collect();
        let side = |at: usize| match mode {
            Mode::All(s) => Some(s),
            Mode::Term(i) if at < i => Some(Side::After),
            Mode::Term(i) if at > i => Some(Side::Before),
            Mode::Term(_) => None,
        };
        let src = &srcs[k];
        match &step.kind {
            StepKind::Atom => {
                // Each row of the step's input, read where it lies, runs the steps after it before the next row is read.
                let mut go = |s: i64, row: &[i64]| {
                    for (a, &v) in step.args.iter().zip(row) {
                        if let Arg::Bind(r) = *a {
                            regs[r] = v;
                        }
                    }
                    // A variable twice in one atom: its later column must equal its first.
                    if step.args.iter().zip(row).all(|(a, &v)| !matches!(*a, Arg::Again(r) if regs[r] != v)) {
                        self.walk(db, p, srcs, mode, k + 1, regs, sign * s, &mut *emit);
                    }
                };
                match side(k) {
                    Some(s) => self.scan(db, src, s, &bound, &mut |row| go(1, row)),
                    None => self.changes(db, src, &bound, &mut |s, row| go(if s == Sign::Added { 1 } else { -1 }, row)),
                }
            }
            StepKind::Not => {
                let none = |s: Side| {
                    let mut any = false;
                    self.scan(db, src, s, &bound, &mut |_| any = true);
                    !any
                };
                let f = match side(k) {
                    Some(s) => i64::from(none(s)),
                    None => i64::from(none(Side::After)) - i64::from(none(Side::Before)),
                };
                if f != 0 {
                    self.walk(db, p, srcs, mode, k + 1, regs, sign * f, emit);
                }
            }
            StepKind::Cmp(op) => {
                // A comparison never changes: it has no term of its own.
                if side(k).is_none() {
                    return;
                }
                if let [Some(a), Some(b)] = bound[..] {
                    if op.holds(a, b) {
                        self.walk(db, p, srcs, mode, k + 1, regs, sign, emit);
                    }
                }
            }
        }
    }

    // ---- a phase --------------------------------------------------------------------------------------------------

    /// One phase: the program held in `prog`'s K-form relations, run across `prog`'s change over `db`, whose inputs'
    /// changes are written. It writes each head's change into `db`. Loads are answered from `world`.
    pub fn phase(&self, prog: &Db, form: &Words, db: &mut Db, world: &dyn World) -> Result<(), Refused> {
        let after = Program::read(prog, true, form, self.sym)?;
        let before = Program::read(prog, false, form, self.sym)?;
        // The modules whose rows changed: their rules before are removed, their rules after added (the module is the
        // grain, as the file is a document's).
        let mut changed: FxHashSet<i64> = FxHashSet::default();
        for &k in &form.k {
            if prog.has(k) && prog.changed(k) {
                let b = prog.bistate(k);
                let mut row = vec![0i64; prog.arity(k).expect("declared")];
                for id in b.gone() {
                    read_row(b.before().expect("a version"), id, &mut row);
                    changed.insert(row[0]);
                }
                for id in b.added() {
                    read_row(b.after(), id, &mut row);
                    changed.insert(row[0]);
                }
            }
        }
        // Every relation a rule reads or writes, declared.
        for p in before.plans.iter().chain(&after.plans) {
            db.declare(p.head, p.head_args.len());
            for s in &p.steps {
                if !matches!(s.kind, StepKind::Cmp(_)) && self.view(s.rel).is_none() {
                    db.declare(s.rel, s.args.len());
                }
            }
        }
        // Strata, in order.
        let mut strata: BTreeMap<i64, Vec<&Plan>> = BTreeMap::new();
        for p in &after.plans {
            strata.entry(after.stratum(p.module)).or_default().push(p);
        }
        let mut stratum_of: FxHashMap<i64, i64> = FxHashMap::default();
        for (&k, plans) in &strata {
            for p in plans {
                if let Some(&j) = stratum_of.get(&p.head).filter(|&&j| j != k) {
                    return Err(Refused { module: p.module, rule: p.rule, why: format!("{} is written in strata {j} and {k}", self.sym.name(p.head)) });
                }
                if self.view(p.head).is_some() {
                    return Err(Refused { module: p.module, rule: p.rule, why: format!("{} is a view", self.sym.name(p.head)) });
                }
                stratum_of.insert(p.head, k);
            }
        }
        // A head no rule writes any more has no rows.
        let gone: FxHashSet<i64> = before.plans.iter().map(|p| p.head).filter(|h| !stratum_of.contains_key(h)).collect();
        for h in gone {
            db.write_set(h, &FxHashSet::default());
            db.counted.remove(&h);
        }
        for (&k, plans) in &strata {
            let heads: FxHashSet<i64> = plans.iter().map(|p| p.head).collect();
            let mut recursive = false;
            for p in plans {
                for s in &p.steps {
                    match stratum_of.get(&s.rel) {
                        Some(&j) if j > k => {
                            return Err(Refused { module: p.module, rule: p.rule, why: format!("{} is read before its stratum {j} runs", self.sym.name(s.rel)) });
                        }
                        Some(&j) if j == k && s.kind == StepKind::Not => {
                            return Err(Refused { module: p.module, rule: p.rule, why: format!("{} is negated in its own stratum", self.sym.name(s.rel)) });
                        }
                        Some(&j) if j == k => recursive = true,
                        _ => {}
                    }
                }
            }
            let asks = heads.contains(&self.load);
            if recursive || asks {
                self.fixpoint(db, plans, &heads, asks, world)?;
            } else if heads.iter().all(|h| db.counted.contains(h)) {
                let removed: Vec<&Plan> = before.plans.iter().filter(|p| heads.contains(&p.head) && changed.contains(&p.module)).collect();
                self.wave(db, plans, &removed, &heads, &changed);
            } else {
                self.wave_cold(db, plans, &heads);
            }
        }
        Ok(())
    }

    /// A stratum that reads none of its heads, by its rules' terms: removed rules' derivations gone, added rules'
    /// added, and each unchanged rule's terms; each head's derivation counts settled (`change::project`).
    fn wave(&self, db: &mut Db, plans: &[&Plan], removed: &[&Plan], heads: &FxHashSet<i64>, changed: &FxHashSet<i64>) {
        let mut tallies = self.tallies(db, heads);
        {
            let db: &Db = db;
            let mut add = |h: i64, sign: i64, row: &[i64]| tallies.get_mut(&h).expect("a head").add(sign_of(sign), row, &[]);
            for p in removed {
                let srcs: Vec<Src<'_>> = p.steps.iter().map(|s| self.src(db, s.rel, &s.kind)).collect();
                self.derive(db, p, &srcs, Mode::All(Side::Before), &mut |s, row| add(p.head, -s, row));
            }
            for p in plans {
                let srcs: Vec<Src<'_>> = p.steps.iter().map(|s| self.src(db, s.rel, &s.kind)).collect();
                if changed.contains(&p.module) {
                    self.derive(db, p, &srcs, Mode::All(Side::After), &mut |s, row| add(p.head, s, row));
                    continue;
                }
                for (i, src) in srcs.iter().enumerate() {
                    if self.changed(db, src) {
                        self.derive(db, p, &srcs, Mode::Term(i), &mut |s, row| add(p.head, s, row));
                    }
                }
            }
        }
        self.settle_heads(db, tallies, false);
    }

    /// A stratum that reads none of its heads, by the reference strategy: its rules' derivations on the after side
    /// added, every count kept gone, so each head's counts are made again and the head changes where one crosses 0.
    fn wave_cold(&self, db: &mut Db, plans: &[&Plan], heads: &FxHashSet<i64>) {
        let mut tallies = self.tallies(db, heads);
        {
            let db: &Db = db;
            for p in plans {
                let srcs: Vec<Src<'_>> = p.steps.iter().map(|s| self.src(db, s.rel, &s.kind)).collect();
                let t = tallies.get_mut(&p.head).expect("a head");
                self.derive(db, p, &srcs, Mode::All(Side::After), &mut |s, row| t.add(sign_of(s), row, &[]));
            }
        }
        for &h in heads {
            let name = self.counts(db, h);
            let t = tallies.get_mut(&h).expect("a head");
            let kept = db.before(name);
            let n = kept.arity() - 1;
            let mut row = vec![0i64; n + 1];
            for id in kept.live_ids() {
                read_row(&kept, id, &mut row);
                for _ in 0..row[n] {
                    t.add(Sign::Gone, &row[..n], &[]);
                }
            }
        }
        self.settle_heads(db, tallies, true);
    }

    fn tallies(&self, db: &Db, heads: &FxHashSet<i64>) -> FxHashMap<i64, Tally> {
        heads.iter().map(|&h| (h, Tally::new(db.arity(h).expect("declared"), 1))).collect()
    }

    /// Each head's tally settled against its counts (`change::project`), and the head's change written: its rows whose
    /// counts left or reached 0, or (`cold`, when its rows may not be its counts' keys) its version diffed with its
    /// counts' keys after.
    fn settle_heads(&self, db: &mut Db, tallies: FxHashMap<i64, Tally>, cold: bool) {
        for (h, t) in tallies {
            let counts = self.counts(db, h);
            let mut moved: Vec<(Sign, Box<[i64]>)> = Vec::new();
            db.write(counts, |w, version| {
                change::project(State { rel: Some(version.top), by_key: 0 }, &t, w, &mut |s, row| {
                    if !cold {
                        // copy: derived (a head row whose count crossed 0: the head's change, written once below)
                        moved.push((s, row.into()));
                    }
                });
            });
            if cold {
                db.write_keys(h, counts);
            } else {
                write_moved(db, h, &moved);
            }
            db.counted.insert(h);
        }
    }

    /// Head `h`'s derivation counts: a relation of its rows and their counts, declared.
    fn counts(&self, db: &mut Db, h: i64) -> i64 {
        let name = self.sym.intern(&format!("{}#counts", self.sym.name(h)));
        let arity = db.arity(h).expect("declared");
        db.declare_counts(name, arity);
        name
    }

    /// A stratum that reads its own heads, or asks for loads: semi-naive to its fixpoint on the after side, from cold;
    /// loads answered between fixpoints until no document comes or goes; each head then diffed with its version.
    fn fixpoint(&self, db: &mut Db, plans: &[&Plan], heads: &FxHashSet<i64>, asks: bool, world: &dyn World) -> Result<(), Refused> {
        loop {
            let open = self.semi_naive(db, plans, heads);
            if asks && self.answer(db, open[&self.load], world) {
                continue;
            }
            for (h, r) in open {
                db.write_diff(h, r);
                db.counted.remove(&h);
            }
            return Ok(());
        }
    }

    /// The stratum's heads' rows at its fixpoint, from cold, each head's in an open relation of the level's block. A
    /// round's rule terms read the last round's new rows (a range of the open relation) at one step and the rows
    /// before this round at the others; a derived row is inserted where it is found, past the ranges being read.
    fn semi_naive(&self, db: &Db, plans: &[&Plan], heads: &FxHashSet<i64>) -> FxHashMap<i64, NonNull<Relation>> {
        let open: FxHashMap<i64, NonNull<Relation>> = heads.iter().map(|&h| (h, db.scratch(db.arity(h).expect("declared")))).collect();
        let rel = |h: i64| db.open(open[&h]);
        let len = |h: i64| rel(h).rows.len();
        let mut last: FxHashMap<i64, (u32, u32)> = heads.iter().map(|&h| (h, (0, 0))).collect();
        let mut round = 0;
        loop {
            let now: FxHashMap<i64, u32> = heads.iter().map(|&h| (h, len(h))).collect();
            for p in plans {
                let own: Vec<usize> = p.steps.iter().enumerate().filter(|(_, s)| heads.contains(&s.rel) && s.kind == StepKind::Atom).map(|(i, _)| i).collect();
                // Round 0 runs every rule once over no rows of the heads; later rounds, one term an own step.
                let terms: Vec<Option<usize>> = if round == 0 { vec![None] } else { own.iter().map(|&i| Some(i)).collect() };
                let out = rel(p.head);
                for term in terms {
                    let srcs: Vec<Src<'_>> = p
                        .steps
                        .iter()
                        .enumerate()
                        .map(|(j, s)| match heads.contains(&s.rel) {
                            true if Some(j) == term => Src::Open(rel(s.rel), last[&s.rel].0, last[&s.rel].1),
                            true => Src::Open(rel(s.rel), 0, now[&s.rel]),
                            false => self.src(db, s.rel, &s.kind),
                        })
                        .collect();
                    self.derive(db, p, &srcs, Mode::All(Side::After), &mut |_, row| {
                        db.insert_into(out, row);
                    });
                }
            }
            let mut any = false;
            for &h in heads {
                last.insert(h, (now[&h], len(h)));
                any |= len(h) > now[&h];
            }
            round += 1;
            if !any {
                return open;
            }
        }
    }

    /// Answers the loads in open relation `asked` (each row's column 0 a path): each path's document read from
    /// `world`, parsed, and given its new text when its bytes changed; documents no load asks for are dropped.
    /// Whether any document came, went or changed.
    fn answer(&self, db: &mut Db, asked: NonNull<Relation>, world: &dyn World) -> bool {
        let mut moved = false;
        let paths: FxHashSet<i64> = {
            let v = View::full(db.open(asked));
            v.live_ids().map(|id| Read::get(&v, id, ColumnIndex::new(0))).collect()
        };
        for &path in &paths {
            let src = world.read(&self.sym.name(path));
            let same = |t: &Option<Arc<Text>>| match (t, &src) {
                (Some(t), Some(s)) => t.src == *s,
                (None, None) => true,
                _ => false,
            };
            match db.doc_of(path) {
                Some(d) => {
                    let doc = &mut db.docs[d as usize - 1];
                    doc.asked = true;
                    if !same(&doc.after) {
                        doc.after = src.and_then(|s| Text::parse(s).ok()).map(Arc::new);
                        moved = true;
                    }
                }
                None => {
                    if let Some(s) = src {
                        let text = Text::parse(s).ok().map(Arc::new);
                        db.docs.push(Doc { path, before: None, after: text, given: false, asked: true });
                        moved = true;
                    }
                }
            }
        }
        for doc in &mut db.docs {
            if !doc.given && !paths.contains(&doc.path) && doc.after.is_some() {
                doc.after = None;
                moved = true;
            }
        }
        moved
    }
}

fn sign_of(s: i64) -> Sign {
    if s > 0 { Sign::Added } else { Sign::Gone }
}

/// Head `h`'s change, from its rows whose counts left or reached 0: gone rows killed in its version, added rows pushed.
fn write_moved(db: &mut Db, h: i64, moved: &[(Sign, Box<[i64]>)]) {
    let arity = db.arity(h).expect("declared");
    let cols: Vec<ColumnIndex> = (0..arity).map(|c| ColumnIndex::new(c as u16)).collect();
    db.write(h, |w, version| {
        for (s, row) in moved {
            match s {
                Sign::Gone => {
                    Table::probe(version, &cols, row, &mut |id| {
                        w.kill(id.get());
                        false
                    });
                }
                Sign::Added => {
                    w.push(row);
                }
            }
        }
    });
}

/// Whether `row` agrees with `bound`.
fn fits(row: &[i64], bound: &[Option<i64>]) -> bool {
    row.iter().zip(bound).all(|(v, b)| b.is_none_or(|b| b == *v))
}

/// Every live row of `t` that agrees with `bound`: by a probe on the bound columns, else a scan.
pub(crate) fn scan_table<T: Table>(t: &T, bound: &[Option<i64>], each: &mut dyn FnMut(&[i64])) {
    let mut row = vec![0i64; bound.len()];
    let (cols, key): (Vec<ColumnIndex>, Vec<i64>) = bound.iter().enumerate().filter_map(|(c, b)| b.map(|b| (ColumnIndex::new(c as u16), b))).unzip();
    if cols.is_empty() {
        for id in t.live_ids() {
            read_row(t, id, &mut row);
            each(&row);
        }
        return;
    }
    t.probe(&cols, &key, &mut |id| {
        read_row(t, id, &mut row);
        each(&row);
        true
    });
}
