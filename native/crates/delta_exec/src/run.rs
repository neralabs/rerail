//! The delta operators of the design page over K-form (decision 0686): the single-file wave, the cross-file wave,
//! the fixpoint and the phase. Every rule body runs through one operator of `blocks`, `change::body` (0678 step 5's
//! join from two inputs to k), over relations, views and a fixpoint's rounds, each read where it lies. What is
//! written here is which states each body reads, and what is done with its derivations.
//!
//! **A phase** runs a program across its change over a level's facts. The program's change is taken at the module
//! grain: a changed module's rules before are removed and its rules after added; the others are the level's cached
//! plans (L5), with the strata they make. Its strata are the ones the program states (`k_order`), checked and run in
//! order; the executor computes none (`Program::order`).
//!
//! **A wave** is a stratum that reads none of its own heads. Each head keeps its derivations' counts
//! (`{head}#counts`), and changes where a count crosses 0 (`change::project`). Its rules run:
//!
//! - removed: every derivation before, gone; added: every derivation after, added;
//! - a rule of one document's views (single-file): for each changed document, its derivations over the old text gone
//!   and over the new text added (the file is the grain); none for the other documents;
//! - any other rule (cross-file): one term for each part whose input changed (`Body::Term`).
//!
//! A stratum with no counts kept yet (its first run) runs by the reference strategy: its derivations after added,
//! every kept count gone, the head set to the counts' keys.
//!
//! **A fixpoint** is a stratum that reads its own heads: semi-naive rounds from cold (the reference strategy, which
//! the agreed design allows for recursion across a change), each round a frozen relation, a round's terms reading
//! the last round as the change; then each head diffed with its version.
//!
//! **Loads** (discovery, the page's part II): a stratum that writes `load` first runs its `load` rules alone, answers
//! the paths they ask for (`World`), and again until no document comes, goes or changes; then its other rules run,
//! reading `load` as an input. A file whose stamp has not changed is not read again.

use crate::db::{Db, Text, read_row};
use crate::kform::{self, Fun, PartKind, Plan, Program, Refused, Val, Words};
use crate::src::{Rounds, Src};
use crate::sym::Symbols;
use crate::tape::{Names, View as TapeKind};
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read};
use rerail_blocks::change::{self, Body, Part, Side, Sign, State, Tally, Tuple};
use rerail_blocks::relation::{Relation, View};
use rerail_blocks::table::Table;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::ptr::NonNull;
use std::sync::Arc;

/// What answers a `load`: a path's stamp (a file's change mark), and its bytes.
pub trait World {
    fn stamp(&self, path: &str) -> Option<u64>;
    fn read(&self, path: &str) -> Option<String>;
}

/// No files.
pub struct NoWorld;

impl World for NoWorld {
    fn stamp(&self, _: &str) -> Option<u64> {
        None
    }
    fn read(&self, _: &str) -> Option<String> {
        None
    }
}

/// The executor: the symbols, and the names it reads views and loads by.
pub struct Exec<'s> {
    pub sym: &'s Symbols,
    pub names: Names,
    views: FxHashMap<i64, TapeKind>,
    load: i64,
    /// Derivations made, for tests of how much work a change costs.
    pub work: Cell<u64>,
}

/// Which states a plan's parts read.
#[derive(Clone, Copy)]
enum Inputs<'r, 'a> {
    /// Each relation and view across the run.
    Run,
    /// The views of document `d` alone, on one side, as a cold run.
    Doc(i64, bool),
    /// A fixpoint's: its heads' rounds (the last round the change when `delta`), the rest after the run.
    Fix(&'r FxHashMap<i64, Vec<&'a Relation>>, bool),
}

impl<'s> Exec<'s> {
    pub fn new(sym: &'s Symbols) -> Exec<'s> {
        let views = TapeKind::ALL
            .iter()
            .map(|&(v, n, _)| (sym.intern(n), v))
            .collect();
        Exec {
            sym,
            names: Names::new(sym),
            views,
            load: sym.intern("load"),
            work: Cell::new(0),
        }
    }

    pub fn is_view(&self, rel: i64) -> bool {
        self.views.contains_key(&rel)
    }

    // ---- a rule ---------------------------------------------------------------------------------------------------

    /// Each part's input.
    fn inputs<'a>(
        &'a self,
        db: &'a Db,
        p: &Plan,
        how: Inputs<'_, 'a>,
    ) -> Vec<Option<Bistate<Src<'a>>>> {
        p.parts
            .iter()
            .map(|part| {
                if matches!(part.kind, PartKind::Test(_)) {
                    return None;
                }
                let (sym, names) = (self.sym, &self.names);
                Some(match (self.views.get(&part.rel), how) {
                    (Some(&v), Inputs::Run) => Bistate::new(
                        Src::Tape(db.view(v, false, sym, names)),
                        Src::Tape(db.view(v, true, sym, names)),
                    ),
                    (Some(&v), Inputs::Doc(d, after)) => {
                        Bistate::cold(Src::Tape(db.doc_view(v, d, after, sym, names)))
                    }
                    (Some(&v), Inputs::Fix(..)) => Bistate::new(
                        Src::Tape(db.view(v, true, sym, names)),
                        Src::Tape(db.view(v, true, sym, names)),
                    ),
                    (None, Inputs::Fix(rounds, delta)) if rounds.contains_key(&part.rel) => {
                        let r = &rounds[&part.rel];
                        let n = part.terms.len();
                        let before = if delta { &r[..r.len() - 1] } else { &r[..] };
                        Bistate::new(
                            Src::Rounds(Rounds::new(before, n, false)),
                            Src::Rounds(Rounds::new(r, n, delta)),
                        )
                    }
                    (None, Inputs::Fix(..)) => {
                        Bistate::new(Src::Rel(db.after(part.rel)), Src::Rel(db.after(part.rel)))
                    }
                    (None, Inputs::Run | Inputs::Doc(..)) => {
                        Bistate::new(Src::Rel(db.before(part.rel)), Src::Rel(db.after(part.rel)))
                    }
                })
            })
            .collect()
    }

    /// Plan `p`'s derivations as `how` reads `inputs`, each with its sign and head row.
    fn derive(
        &self,
        p: &Plan,
        inputs: &[Option<Bistate<Src<'_>>>],
        how: Body,
        emit: &mut dyn FnMut(Sign, &[i64]),
    ) {
        let parts: Vec<Part<'_, Src<'_>>> = p
            .parts
            .iter()
            .zip(inputs)
            .map(|(part, b)| match part.kind {
                PartKind::Join => Part::Join(b.as_ref().expect("an input"), &part.terms),
                PartKind::Not => Part::Not(b.as_ref().expect("an input"), &part.terms),
                PartKind::Test(op) => Part::Test(part.terms[0], op, part.terms[1]),
            })
            .collect();
        let mut head = vec![0i64; p.head_args.len()];
        change::body(&parts, p.vars, how, &mut |s, t| {
            for (h, a) in head.iter_mut().zip(&p.head_args) {
                *h = eval(a, t);
            }
            self.work.set(self.work.get() + 1);
            emit(s, &head);
        });
    }

    /// Whether part `i`'s input changed in the run.
    fn input_changed(&self, db: &Db, p: &Plan, i: usize) -> bool {
        let part = &p.parts[i];
        match part.kind {
            PartKind::Test(_) => false,
            _ if self.is_view(part.rel) => db.changed_docs().next().is_some(),
            _ => db.changed(part.rel),
        }
    }

    // ---- a phase --------------------------------------------------------------------------------------------------

    /// One phase: the program held in `prog`'s K-form relations under `form`, run across `prog`'s change over `db`,
    /// whose inputs' changes are written. It writes each head's change into `db`. Loads are answered from `world`.
    pub fn phase(
        &self,
        prog: &Db,
        form: &Words,
        db: &mut Db,
        world: &dyn World,
    ) -> Result<(), Refused> {
        let r = self.phase_in(prog, form, db, world);
        if r.is_err() {
            // The cache names no state now (L5): read again next run.
            db.plans = None;
        }
        r
    }

    fn phase_in(
        &self,
        prog: &Db,
        form: &Words,
        db: &mut Db,
        world: &dyn World,
    ) -> Result<(), Refused> {
        let is_view = |r: i64| self.is_view(r);
        let changed = kform::changed_modules(prog, form);
        let (mut cache, fresh) = match db.plans.take() {
            Some(c) => (c, false),
            None => {
                let mut c = Program::default();
                for m in kform::modules(prog, form, false) {
                    // A module refused before the run has no rules before it; its rules after are read below.
                    if let Ok(ps) = kform::read_module(prog, form, false, self.sym, &is_view, m) {
                        c.modules.insert(m, ps);
                    }
                }
                (c, true)
            }
        };
        let mut removed: Vec<Plan> = Vec::new();
        for &m in &changed {
            removed.extend(cache.modules.remove(&m).unwrap_or_default());
            let new = kform::read_module(prog, form, true, self.sym, &is_view, m)?;
            if !new.is_empty() {
                cache.modules.insert(m, new);
            }
        }
        if fresh || !changed.is_empty() {
            cache.order(self.sym, &is_view, self.load)?;
            // Every relation a rule reads or writes, declared, with the indexes its probes want.
            for p in cache
                .modules
                .values()
                .flatten()
                .filter(|p| fresh || changed.contains(&p.module))
            {
                db.declare(p.head, p.head_args.len());
                for part in p
                    .parts
                    .iter()
                    .filter(|x| !matches!(x.kind, PartKind::Test(_)) && !self.is_view(x.rel))
                {
                    db.declare(part.rel, part.terms.len());
                }
                for (rel, cols) in &p.probes {
                    db.want(*rel, cols);
                }
            }
        }
        let result = self.strata(db, &cache, &removed, &changed, world);
        db.plans = Some(cache);
        result
    }

    fn strata(
        &self,
        db: &mut Db,
        prog: &Program,
        removed: &[Plan],
        changed: &BTreeSet<i64>,
        world: &dyn World,
    ) -> Result<(), Refused> {
        // A head no rule writes any more has no rows, and no counts.
        let gone: BTreeSet<i64> = removed
            .iter()
            .map(|p| p.head)
            .filter(|h| !prog.strata.iter().any(|s| s.heads.contains(h)))
            .collect();
        for h in gone {
            db.clear(h);
            let c = self.counts_name(h);
            if db.has(c) {
                db.clear(c);
            }
            db.counted.remove(&h);
        }
        db.prepare_views();
        for s in &prog.strata {
            let plans: Vec<&Plan> = s.plans.iter().map(|&a| prog.plan(a)).collect();
            if s.recursive {
                self.fixpoint(db, &plans, &s.heads, s.asks, world)?;
                continue;
            }
            let (loads, rest): (Vec<&Plan>, Vec<&Plan>) =
                plans.iter().partition(|p| p.head == self.load);
            if s.asks {
                self.discover(db, &loads, world)?;
            }
            let own: FxHashSet<i64> = rest.iter().map(|p| p.head).collect();
            let rm: Vec<&Plan> = removed.iter().filter(|p| own.contains(&p.head)).collect();
            self.wave(db, &rest, &rm, &own, changed);
        }
        Ok(())
    }

    // ---- a wave ---------------------------------------------------------------------------------------------------

    fn wave(
        &self,
        db: &mut Db,
        plans: &[&Plan],
        removed: &[&Plan],
        heads: &FxHashSet<i64>,
        changed: &BTreeSet<i64>,
    ) {
        if heads.is_empty() {
            return;
        }
        let cold = !heads.iter().all(|h| db.counted.contains(h));
        let mut tallies: FxHashMap<i64, Tally> = heads
            .iter()
            .map(|&h| (h, Tally::new(db.arity(h).expect("declared"), 1)))
            .collect();
        {
            let db: &Db = db;
            let mut add = |h: i64, s: Sign, row: &[i64]| {
                tallies.get_mut(&h).expect("a head").add(s, row, &[])
            };
            if cold {
                for p in plans {
                    self.derive(
                        p,
                        &self.inputs(db, p, Inputs::Run),
                        Body::All(Side::After),
                        &mut |s, row| add(p.head, s, row),
                    );
                }
            } else {
                for p in removed {
                    self.derive(
                        p,
                        &self.inputs(db, p, Inputs::Run),
                        Body::All(Side::Before),
                        &mut |s, row| add(p.head, flip(s), row),
                    );
                }
                for p in plans {
                    if changed.contains(&p.module) {
                        self.derive(
                            p,
                            &self.inputs(db, p, Inputs::Run),
                            Body::All(Side::After),
                            &mut |s, row| add(p.head, s, row),
                        );
                    } else if p.single_file {
                        // The single-file waves: each changed document's old text's derivations gone, its new text's
                        // added.
                        for d in db.changed_docs() {
                            self.derive(
                                p,
                                &self.inputs(db, p, Inputs::Doc(d, false)),
                                Body::All(Side::After),
                                &mut |s, row| add(p.head, flip(s), row),
                            );
                            self.derive(
                                p,
                                &self.inputs(db, p, Inputs::Doc(d, true)),
                                Body::All(Side::After),
                                &mut |s, row| add(p.head, s, row),
                            );
                        }
                    } else {
                        // The cross-file wave: a term for each part whose input changed.
                        let inputs = self.inputs(db, p, Inputs::Run);
                        for i in (0..p.parts.len()).filter(|&i| self.input_changed(db, p, i)) {
                            self.derive(p, &inputs, Body::Term(i), &mut |s, row| {
                                add(p.head, s, row)
                            });
                        }
                    }
                }
            }
        }
        if cold {
            // Every kept count gone: the counts are made again.
            for &h in heads {
                let name = self.counts(db, h);
                let kept = db.before(name);
                let n = Read::arity(&kept) - 1;
                let mut row = vec![0i64; n + 1];
                let t = tallies.get_mut(&h).expect("a head");
                for id in kept.live_ids() {
                    read_row(&kept, id, &mut row);
                    for _ in 0..row[n] {
                        t.add(Sign::Gone, &row[..n], &[]);
                    }
                }
            }
        }
        for (h, t) in tallies {
            let counts = self.counts(db, h);
            if cold {
                db.write(counts, |w, version| {
                    change::project(
                        State {
                            rel: Some(version.top),
                            by_key: 0,
                        },
                        &t,
                        w,
                        &mut |_, _| {},
                    )
                });
                db.write_keys(h, counts);
            } else {
                // Each head row whose count left or reached 0, written into the head's delta where it is found.
                let cols: Vec<ColumnIndex> = (0..db.arity(h).expect("declared"))
                    .map(|c| ColumnIndex::new(c as u16))
                    .collect();
                db.write_pair(counts, h, |w, version, hw, head| {
                    change::project(
                        State {
                            rel: Some(version.top),
                            by_key: 0,
                        },
                        &t,
                        w,
                        &mut |s, row| match s {
                            Sign::Gone => {
                                Table::probe(head, &cols, row, &mut |id| {
                                    hw.kill(id.get());
                                    false
                                });
                            }
                            Sign::Added => {
                                hw.push(row);
                            }
                        },
                    );
                });
            }
            db.counted.insert(h);
        }
    }

    fn counts_name(&self, h: i64) -> i64 {
        self.sym.intern(&format!("{}#counts", self.sym.name(h)))
    }

    /// Head `h`'s derivation counts: a relation of its rows and their counts, declared.
    fn counts(&self, db: &mut Db, h: i64) -> i64 {
        let name = self.counts_name(h);
        db.declare_counts(name, db.arity(h).expect("declared"));
        name
    }

    // ---- a fixpoint -----------------------------------------------------------------------------------------------

    /// A stratum that reads its own heads: semi-naive from cold; loads answered between fixpoints until no document
    /// moves; each head then diffed with its version.
    fn fixpoint(
        &self,
        db: &mut Db,
        plans: &[&Plan],
        heads: &FxHashSet<i64>,
        asks: bool,
        world: &dyn World,
    ) -> Result<(), Refused> {
        loop {
            let rounds = self.semi_naive(db, plans, heads);
            let arity: FxHashMap<i64, usize> = heads
                .iter()
                .map(|&h| (h, db.arity(h).expect("declared")))
                .collect();
            // SAFETY: the rounds lie in the level's block, which neither answering nor writing moves or frees.
            let table = |h: i64| {
                let rels: Vec<&Relation> =
                    rounds[&h].iter().map(|r| unsafe { r.as_ref() }).collect();
                Src::Rounds(Rounds::new(&rels, arity[&h], false))
            };
            if asks {
                let asked = table(self.load);
                if self.answer(db, &asked, world)? {
                    db.prepare_views();
                    continue;
                }
            }
            let mut hs: Vec<i64> = heads.iter().copied().collect();
            hs.sort_unstable();
            for h in hs {
                let t = table(h);
                db.write_diff(h, &t);
                db.counted.remove(&h);
            }
            return Ok(());
        }
    }

    /// Each head's rounds at the fixpoint, from cold: round 0 runs every rule once with the heads empty; each later
    /// round runs, for each part of a rule that joins a head of the stratum, its term over the last round's rows. A
    /// row an earlier round holds is not kept again.
    fn semi_naive(
        &self,
        db: &Db,
        plans: &[&Plan],
        heads: &FxHashSet<i64>,
    ) -> FxHashMap<i64, Vec<NonNull<Relation>>> {
        let mut rounds: FxHashMap<i64, Vec<&Relation>> =
            heads.iter().map(|&h| (h, Vec::new())).collect();
        let cols = |h: i64| -> Vec<ColumnIndex> {
            (0..db.arity(h).expect("declared"))
                .map(|c| ColumnIndex::new(c as u16))
                .collect()
        };
        for round in 0.. {
            let open: FxHashMap<i64, &Relation> =
                heads.iter().map(|&h| (h, db.scratch(h))).collect();
            for p in plans {
                let inputs = self.inputs(db, p, Inputs::Fix(&rounds, round > 0));
                let terms: Vec<Body> = if round == 0 {
                    vec![Body::All(Side::After)]
                } else {
                    (0..p.parts.len())
                        .filter(|&i| {
                            p.parts[i].kind == PartKind::Join && heads.contains(&p.parts[i].rel)
                        })
                        .map(Body::Term)
                        .collect()
                };
                let (out, have, keys) = (open[&p.head], &rounds[&p.head], cols(p.head));
                for how in terms {
                    self.derive(p, &inputs, how, &mut |s, row| {
                        assert_eq!(s, Sign::Added, "a fixpoint's rounds only add");
                        if have
                            .iter()
                            .all(|r| Table::probe(&View::full(r), &keys, row, &mut |_| false))
                        {
                            db.insert_into(out, row);
                        }
                    });
                }
            }
            let mut any = false;
            for (&h, r) in &open {
                db.freeze(h, r);
                any |= r.rows.len() > 0;
                rounds.get_mut(&h).expect("a head").push(r);
            }
            if !any {
                break;
            }
        }
        rounds
            .into_iter()
            .map(|(h, r)| (h, r.into_iter().map(NonNull::from).collect()))
            .collect()
    }

    // ---- loads ----------------------------------------------------------------------------------------------------

    /// A stratum's `load` rules run alone, from cold, and their paths answered, until no document moves; then
    /// `load`'s rows are written, its change read by the stratum's other rules.
    fn discover(&self, db: &mut Db, loads: &[&Plan], world: &dyn World) -> Result<(), Refused> {
        loop {
            let s = NonNull::from(db.scratch(self.load));
            {
                let db: &Db = db;
                // SAFETY: an open relation of the level's block, alive while the block is.
                let open = unsafe { s.as_ref() };
                for p in loads {
                    self.derive(
                        p,
                        &self.inputs(db, p, Inputs::Run),
                        Body::All(Side::After),
                        &mut |_, row| {
                            db.insert_into(open, row);
                        },
                    );
                }
                db.freeze(self.load, open);
            }
            // SAFETY: as above: answering moves no relation.
            let asked = Src::Rel(View::full(unsafe { s.as_ref() }));
            if self.answer(db, &asked, world)? {
                db.prepare_views();
                continue;
            }
            db.write_diff(self.load, &asked);
            db.counted.remove(&self.load);
            return Ok(());
        }
    }

    /// Answers the paths of `asked` (column 0): each path whose stamp changed, or that has no text, read and parsed;
    /// documents no load asks for any more dropped. Whether any document came, went or changed.
    fn answer<T: Table>(&self, db: &mut Db, asked: &T, world: &dyn World) -> Result<bool, Refused> {
        // copy: not block data (the paths asked, each once)
        let paths: BTreeSet<i64> = asked
            .live_ids()
            .map(|id| asked.get(id, ColumnIndex::new(0)))
            .collect();
        let base = |db: &Db, path: i64| {
            db.doc_of(path)
                .and_then(|d| db.docs[d as usize - 1].after.as_ref().map(|p| p.base))
        };
        let mut moved = false;
        for &path in &paths {
            let name = self.sym.name(path);
            let stamp = world.stamp(&name);
            let doc = db.doc_of(path).map(|d| &db.docs[d as usize - 1]);
            if doc.is_some_and(|d| d.after.is_some() && stamp.is_some() && d.stamp == stamp) {
                continue;
            }
            let text = match world.read(&name) {
                Some(src) => Some(Arc::new(Text::parse(src).map_err(|e| Refused {
                    module: 0,
                    rule: path,
                    why: format!("{name} does not parse: {e}"),
                })?)),
                None => None,
            };
            let was = base(db, path);
            db.set_doc(path, text, false, stamp);
            moved |= base(db, path) != was;
        }
        let dropped: Vec<i64> = db
            .docs
            .iter()
            .filter(|d| !d.given && d.after.is_some() && !paths.contains(&d.path))
            .map(|d| d.path)
            .collect();
        for path in dropped {
            db.set_doc(path, None, false, None);
            moved = true;
        }
        Ok(moved)
    }
}

/// A head argument's value for a derivation.
fn eval(v: &Val, t: &Tuple<'_, '_, Src<'_>>) -> i64 {
    match v {
        Val::T(term) => t.value(*term).expect("a head's term"),
        Val::Fn(f, l, r) => {
            let (a, b) = (eval(l, t), eval(r, t));
            match f {
                Fun::Add => a.wrapping_add(b),
                Fun::Sub => a.wrapping_sub(b),
                Fun::Mul => a.wrapping_mul(b),
            }
        }
    }
}

fn flip(s: Sign) -> Sign {
    if s == Sign::Added {
        Sign::Gone
    } else {
        Sign::Added
    }
}
