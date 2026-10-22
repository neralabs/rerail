//! The dataflow (step 3): one timely/differential dataflow holding the base relations once and
//! every worktree's relations as signed deltas.
//!
//! Representation: a relation is one collection of tagged records `(w, record)`. Tag 0 is the
//! base (X_B); tag w ≥ 1 carries worktree w's signed delta X_w - X_B. A keyed relation is arranged
//! once, `key -> (w, value)`, holding base and all deltas together. The operators expand each
//! relational operator into its base + delta form (the expansion Phase 2's compiler would emit):
//!
//! - map/filter/flat_map: per record, tag kept.
//! - join (one join_core over the two shared arrangements, pairs filtered by tag):
//!   (0,0)→0: A_B⋈B_B;  (w,0)→w: ΔA⋈B_B;  (0,w)→w: A_B⋈ΔB;  (w,w)→w: ΔA⋈ΔB;  (w1,w2), w1≠w2: dropped.
//! - presence (distinct): one reduce keyed by the record over tagged counts: base count c, delta
//!   counts d_w; emits (0) once if c > 0 and (w) with diff [c + d_w > 0] - [c > 0] (counts for
//!   negation).
//! - antijoin: A ▷ K = A - A⋈presence(K) (join is bilinear in the ±1 presence delta).
//! - group reduce: one reduce keyed by the group over tagged members; emits logic(base) under tag 0
//!   and, for every worktree with members in the delta, logic(base + delta_w) - logic(base) under w.
//! - recursion (R3): reach is materialized for the base (tag 0) and, in full, for every worktree
//!   whose file delta contains call-edge or seed facts; base edges apply to every such worktree,
//!   delta edges only to their own.
use crate::derive::{Node, World};
use crate::rules::{self, Family, Root, Surface, SurfaceOrIssue, R1F, R2F, R3F, MAX_HOPS};
use crate::store::{Cid, Sym};
use differential_dataflow::input::{Input, InputSession};
use differential_dataflow::operators::arrange::{Arranged, TraceAgent};
use differential_dataflow::operators::iterate::Iterate;
use differential_dataflow::operators::ThresholdTotal;
use differential_dataflow::trace::implementations::{ValBuilder, ValSpine};
use differential_dataflow::{ExchangeData, VecCollection};
use rustc_hash::{FxHashMap, FxHasher};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use timely::dataflow::operators::probe::Handle;

pub type T = u64;
pub type W = u32;
pub const BASE: W = 0;
type Col<'s, D> = VecCollection<'s, T, D, isize>;
type Arr<'s, K, V> = Arranged<'s, TraceAgent<ValSpine<K, V, T, isize>>>;

pub trait Rec: ExchangeData + Hash + Default {}
impl<D: ExchangeData + Hash + Default> Rec for D {}

/// A tagged relation: (w, record), w = 0 base.
pub type BD<'s, D> = Col<'s, (W, D)>;
/// A tagged keyed relation arranged once: key -> (w, value).
pub type BDK<'s, K, V> = Arr<'s, K, (W, V)>;

pub fn arrange<'s, K: Rec, V: Rec>(r: BD<'s, (K, V)>, name: &str) -> BDK<'s, K, V> {
    r.map(|(w, (k, v))| (k, (w, v))).arrange_by_key_named(&format!("A:{name}"))
}

fn tag(w1: W, w2: W) -> Option<W> {
    match (w1, w2) {
        (a, b) if a == b => Some(a),
        (a, BASE) => Some(a),
        (BASE, b) => Some(b),
        _ => None,
    }
}

pub fn join<'s, K: Rec, V1: Rec, V2: Rec, D: Rec>(a: &BDK<'s, K, V1>, b: &BDK<'s, K, V2>, f: impl Fn(&K, &V1, &V2) -> D + 'static) -> BD<'s, D> {
    a.clone().join_core(b.clone(), move |k, (w1, v1), (w2, v2)| tag(*w1, *w2).map(|w| (w, f(k, v1, v2))))
}

fn flip(before: isize, after: isize) -> isize {
    debug_assert!(before >= 0 && after >= 0, "negative multiplicity: {before} -> {after}");
    (after > 0) as isize - (before > 0) as isize
}

/// Presence of a multiset relation as a tagged keyed relation `x -> (w, ())` with ±1 deltas.
pub fn presence<'s, D: Rec>(r: BD<'s, D>, name: &str) -> BDK<'s, D, ()> {
    let input: Arr<D, W> = r.map(|(w, x)| (x, w)).arrange_by_key_named(&format!("A:{name}:counts"));
    input.reduce_abelian::<_, ValBuilder<D, (W, ()), T, isize>, ValSpine<D, (W, ()), T, isize>, _, _>(
        &format!("R:{name}:presence"),
        |_x, vals: &[(&W, isize)], out: &mut Vec<((W, ()), isize)>| {
            // vals sorted by w: the base count first when present.
            let c = vals.iter().find(|(w, _)| **w == BASE).map(|(_, c)| *c).unwrap_or(0);
            if c > 0 {
                out.push(((BASE, ()), 1));
            }
            for (w, d) in vals {
                if **w != BASE {
                    let f = flip(c, c + d);
                    if f != 0 {
                        out.push(((**w, ()), f));
                    }
                }
            }
        },
        |vec, key, upds| {
            vec.clear();
            vec.extend(upds.drain(..).map(|(v, t, r)| ((key.clone(), v), t, r)));
        },
    )
}

/// A ▷ K for K given as its presence.
pub fn antijoin<'s, K: Rec, V: Rec>(a: BD<'s, (K, V)>, a_arr: &BDK<'s, K, V>, present: &BDK<'s, K, ()>) -> BD<'s, (K, V)> {
    let hit = join(a_arr, present, |k: &K, v: &V, _: &()| (k.clone(), v.clone()));
    a.concat(hit.negate())
}

/// Group aggregate in base + delta form. `logic` sees a group's complete, positive input.
pub fn reduce<'s, K: Rec, V: Rec, V2: Rec>(
    r: BD<'s, (K, V)>,
    name: &str,
    logic: impl Fn(&K, &[(&V, isize)], &mut Vec<(V2, isize)>) + 'static,
) -> BD<'s, (K, V2)> {
    let input = arrange(r, &format!("{name}:in"));
    input
        .reduce_abelian::<_, ValBuilder<K, (W, V2), T, isize>, ValSpine<K, (W, V2), T, isize>, _, _>(
            &format!("R:{name}"),
            move |k, vals: &[(&(W, V), isize)], out: &mut Vec<((W, V2), isize)>| {
                let base: Vec<(&V, isize)> = vals.iter().filter(|((w, _), _)| *w == BASE).map(|((_, v), c)| (v, *c)).collect();
                let mut base_out = Vec::new();
                if !base.is_empty() {
                    logic(k, &base, &mut base_out);
                }
                for (v2, c) in &base_out {
                    out.push(((BASE, v2.clone()), *c));
                }
                let mut i = 0;
                while i < vals.len() {
                    let w = vals[i].0 .0;
                    let mut j = i;
                    while j < vals.len() && vals[j].0 .0 == w {
                        j += 1;
                    }
                    if w != BASE {
                        // members_w = base + delta_w (vals sorted by (w, v): merge by value)
                        let mut m: std::collections::BTreeMap<&V, isize> = base.iter().map(|(v, c)| (*v, *c)).collect();
                        for ((_, v), d) in &vals[i..j] {
                            *m.entry(v).or_default() += d;
                        }
                        let full: Vec<(&V, isize)> = m.into_iter().filter(|(_, c)| *c != 0).collect();
                        debug_assert!(full.iter().all(|(_, c)| *c > 0), "negative group member");
                        let mut wo = Vec::new();
                        if !full.is_empty() {
                            logic(k, &full, &mut wo);
                        }
                        for (v2, c) in wo {
                            out.push(((w, v2), c));
                        }
                        for (v2, c) in &base_out {
                            out.push(((w, v2.clone()), -c));
                        }
                    }
                    i = j;
                }
                // DD expects consolidated output.
                differential_dataflow::consolidation::consolidate(out);
            },
            |vec, key, upds| {
                vec.clear();
                vec.extend(upds.drain(..).map(|(v, t, r)| ((key.clone(), v), t, r)));
            },
        )
        .as_collection(|k, (w, v2)| (*w, (k.clone(), v2.clone())))
}

pub type Out<F> = Rc<RefCell<FxHashMap<(W, F), isize>>>;

fn capture<'s, F: Rec + Eq>(r: BD<'s, F>, probe: &Handle<T>) -> Out<F> {
    let out: Out<F> = Rc::default();
    let o = out.clone();
    r.consolidate()
        .inspect(move |(k, _t, d)| {
            let mut o = o.borrow_mut();
            let e = o.entry(k.clone()).or_default();
            *e += d;
            if *e == 0 {
                o.remove(k);
            }
        })
        .probe_with(probe);
    out
}

/// Live per-worktree records per relation (measurement only, `--track`): (relation, w, record
/// hash) -> multiplicity. Bytes per record: the update's in-memory size.
#[derive(Default)]
pub struct Tracker {
    pub on: bool,
    pub live: FxHashMap<(&'static str, W, u64), isize>,
    pub size: FxHashMap<&'static str, usize>,
}
pub type TrackerRef = Rc<RefCell<Tracker>>;

fn track<'s, D: Rec>(t: &TrackerRef, name: &'static str, c: Col<'s, (W, D)>) -> Col<'s, (W, D)> {
    if !t.borrow().on {
        return c;
    }
    t.borrow_mut().size.insert(name, std::mem::size_of::<((W, D), T, isize)>());
    let t = t.clone();
    c.inspect(move |((w, d), _, r)| {
        if *w == BASE {
            return;
        }
        let mut h = FxHasher::default();
        d.hash(&mut h);
        let mut tr = t.borrow_mut();
        let k = (name, *w, h.finish());
        let e = tr.live.entry(k).or_default();
        *e += r;
        if *e == 0 {
            tr.live.remove(&k);
        }
    })
}

pub struct Handles {
    /// (w, (path, content)); w = 0 for the base membership, w ≥ 1 for signed worktree deltas.
    pub files: InputSession<T, (W, (Sym, Cid)), isize>,
    pub probe: Handle<T>,
    pub r1: Out<R1F>,
    pub r2: Out<R2F>,
    pub r3: Out<R3F>,
    pub tracker: TrackerRef,
}

pub fn build(worker: &mut timely::worker::Worker, world: Rc<RefCell<World>>, track_on: bool) -> Handles {
    let probe = Handle::new();
    let tracker: TrackerRef = Rc::new(RefCell::new(Tracker { on: track_on, ..Default::default() }));
    worker.dataflow::<T, _, _>(|scope| {
        let (files_in, files) = scope.new_collection::<(W, (Sym, Cid)), isize>();
        let files = track(&tracker, "files", files);
        macro_rules! derived {
            ($name:literal, |$p:ident, $c:ident, $wd:ident| $body:expr) => {{
                let wd0 = world.clone();
                track(
                    &tracker,
                    $name,
                    files.clone().flat_map(move |(w, ($p, $c))| {
                        #[allow(unused_mut)]
                        let mut $wd = wd0.borrow_mut();
                        let v = $body;
                        v.into_iter().map(move |x| (w, x))
                    }),
                )
            }};
        }

        // ---- R1: dead code. dead = cand ▷ presence(ref) -------------------------------------
        let refs = derived!("r1_ref", |p, c, wd| { let _ = p; wd.r1_refs(c) });
        let cand = derived!("r1_cand", |p, c, wd| wd.r1_cand(p, c).into_iter().map(|(n, p, q)| (n, (p, q))).collect::<Vec<_>>());
        let live = presence(refs, "r1_live");
        let cand_arr = arrange(cand.clone(), "r1_cand");
        let dead = antijoin(cand, &cand_arr, &live).map(|(w, (_, f))| (w, f));
        let r1 = capture(dead, &probe);

        // ---- R2: public API function families ---------------------------------------------
        let decl = derived!("r2_decl", |p, c, wd| wd.r2_decl(p, c));
        let bind = derived!("r2_bind", |p, c, wd| wd.r2_bind(p, c));
        let exports = derived!("r2_export", |p, c, wd| wd.r2_export(p, c));
        let decl_present = presence(decl.clone().map(|(w, (k, _))| (w, k)), "r2_decl_keys");
        let decl = arrange(decl, "r2_decl");
        let bind = arrange(bind, "r2_bind");
        let mut cur = exports;
        let mut roots: Option<BD<Root>> = None;
        for hop in 0..=MAX_HOPS {
            let cur_arr = arrange(cur.clone(), "r2_hop");
            let res = join(&cur_arr, &decl, |t: &(Sym, Sym), &(em, en): &(Sym, Sym), &(p, flags): &(Sym, u8)| (em, en, p, t.1, flags));
            roots = Some(match roots {
                None => res,
                Some(r) => r.concat(res),
            });
            if hop < MAX_HOPS {
                let nodecl = arrange(antijoin(cur, &cur_arr, &decl_present), "r2_nodecl");
                cur = join(&nodecl, &bind, |_t, &root: &(Sym, Sym), &next: &(Sym, Sym)| (next, root));
            }
        }
        let roots = presence(roots.unwrap(), "r2_roots").as_collection(|r: &Root, (w, ()): &(W, ())| (*w, r.clone()));
        let wd = world.clone();
        let surf_or = roots.flat_map(move |(w, r): (W, Root)| {
            let mut wd = wd.borrow_mut();
            match rules::surface(&mut wd, &r) {
                SurfaceOrIssue::Surface(s) => Some((w, (Some(s), None))),
                SurfaceOrIssue::Mixed(f) => Some((w, (None, Some(f)))),
                SurfaceOrIssue::None => None,
            }
        });
        let mixed = surf_or.clone().flat_map(|(w, (_, f)): (W, (Option<Surface>, Option<R2F>))| f.map(|f| (w, f)));
        let surfaces = surf_or.flat_map(|(w, (s, _)): (W, (Option<Surface>, Option<R2F>))| s.map(|s| (w, (s.0, s))));
        let wd = world.clone();
        let fams = reduce(surfaces, "r2_owner", move |owner: &Sym, input: &[(&Surface, isize)], out: &mut Vec<(Family, isize)>| {
            let ss: Vec<Surface> = input.iter().map(|(s, _)| (*s).clone()).collect();
            for f in rules::owner_families(&wd.borrow(), *owner, &ss) {
                out.push((f, 1));
            }
        });
        let keyed = fams.map(|(w, (owner, (base, imp, ex))): (W, (Sym, Family))| (w, ((base, imp), (owner, ex))));
        let wd = world.clone();
        let issues = reduce(keyed, "r2_family", move |(base, imp): &(Sym, Vec<(u8, Sym, Sym)>), input: &[(&(Sym, (Sym, Sym)), isize)], out: &mut Vec<(R2F, isize)>| {
            let owners: Vec<(Sym, (Sym, Sym))> = input.iter().map(|(o, _)| (*o).clone()).collect();
            for f in rules::family_issues(&wd.borrow(), *base, imp, &owners) {
                out.push((f, 1));
            }
        });
        let r2 = capture(issues.map(|(w, (_, f))| (w, f)).concat(mixed), &probe);

        // ---- R3: reachability to the seeds, demand-driven from them --------------------------
        let target = derived!("r3_target", |p, c, wd| wd.r3_target(p, c));
        let local_edges = derived!("r3_local_edge", |p, c, wd| wd.r3_calls(p, c).0);
        let ext = derived!("r3_callsite", |p, c, wd| wd.r3_calls(p, c).1);
        let seeds = derived!("r3_seed", |p, c, wd| wd.r3_seeds(p, c));
        let ext_edges = join(&arrange(ext, "r3_callsite"), &arrange(target, "r3_target"), |_k, &caller: &Node, &callee: &Node| (caller, callee));
        // Edges by callee, base and deltas in one arrangement shared by every iteration.
        let edges: Arr<Node, (W, Node)> = local_edges.concat(ext_edges).map(|(w, (a, b))| (b, (w, a))).arrange_by_key_named("A:r3_edge_by_callee");
        // Worktrees whose file delta carries call-edge or seed facts: R3 materialized for them.
        let wd = world.clone();
        let active = files
            .clone()
            .filter(move |(w, (_, c))| {
                let wd = wd.borrow();
                let ct = &wd.store.contents[*c as usize];
                *w != BASE && !ct.has_error && (!ct.calls.is_empty() || !ct.seeds.is_empty())
            })
            .map(|(w, _)| (w, ()))
            .reduce(|_w, _vals, out: &mut Vec<((), isize)>| out.push(((), 1)))
            .map(|(w, ())| w);
        let active = track(&tracker, "r3_active", active.map(|w| (w, ()))).map(|(w, ())| w);
        let (seeds_b, seeds_d) = (seeds.clone().filter(|(w, _)| *w == BASE), seeds.filter(|(w, _)| *w != BASE));
        // seeds_w = seeds_B for every active worktree, plus that worktree's seed delta.
        let seeds_all = active
            .clone()
            .map(|w| ((), w))
            .join(seeds_b.clone().map(|(_, s)| ((), s)))
            .map(|((), (w, s))| (w, s))
            .concat(seeds_d)
            .concat(seeds_b);
        let e1 = edges.clone();
        let sa = seeds_all.clone();
        let reach = seeds_all.map(|(w, (n, f))| (n, (w, f))).iterate(move |inner_scope, inner| {
            let e = e1.enter(inner_scope);
            let s = sa.enter(inner_scope).map(|(w, (n, f))| (n, (w, f)));
            inner
                .join_core(e, |_callee, &(w, f), &(we, caller)| (we == BASE || we == w).then_some((caller, (w, f))))
                .concat(s)
                .threshold(|_, c| if *c > 0 { 1 } else { 0 })
        });
        let reach = track(&tracker, "r3_reach", reach.map(|(n, (w, f))| (w, (n, f)))).map(|(w, (n, f))| (n, (w, f)));
        let reach_arr: Arr<Node, (W, Sym)> = reach.arrange_by_key_named("A:r3_reach");
        let wd = world.clone();
        let found = edges
            .join_core(reach_arr, |_callee, &(we, caller), &(w, f)| (we == BASE || we == w).then_some((w, (caller, f))))
            .threshold_total(|_, c| if *c > 0 { 1 } else { 0 })
            .filter(move |(_, (n, _)): &(W, (Node, Sym))| wd.borrow().info(n.0).r3_scope);
        let found = track(&tracker, "r3_found", found);
        // Δ for active worktrees: found_w - found_B.
        let found_b = found.clone().filter(|(w, _)| *w == BASE);
        let minus = active.map(|w| ((), w)).join(found_b.map(|(_, f)| ((), f))).map(|((), (w, f))| (w, f));
        let r3 = capture(found.concat(minus.negate()).map(|(w, ((p, q), f))| (w, (p, q, f))), &probe);
        Handles { files: files_in, probe: probe.clone(), r1, r2, r3, tracker: tracker.clone() }
    })
}

/// Arrangement and batcher sizes from differential's logging, per operator.
#[derive(Default)]
pub struct Sizes {
    pub records: FxHashMap<usize, isize>,
    pub batcher_bytes: FxHashMap<usize, isize>,
    pub names: FxHashMap<usize, String>,
}

pub fn register_logging(worker: &mut timely::worker::Worker) -> Rc<RefCell<Sizes>> {
    use differential_dataflow::logging::{DifferentialEvent, DifferentialEventBuilder};
    let sizes: Rc<RefCell<Sizes>> = Rc::default();
    let s1 = sizes.clone();
    let s2 = sizes.clone();
    if let Some(mut reg) = worker.log_register() {
        reg.insert::<DifferentialEventBuilder, _>("differential/arrange", move |_time, data| {
            if let Some(data) = data {
                let mut s = s1.borrow_mut();
                for (_, e) in data.iter() {
                    match e {
                        DifferentialEvent::Batch(b) => *s.records.entry(b.operator).or_default() += b.length as isize,
                        DifferentialEvent::Drop(d) => *s.records.entry(d.operator).or_default() -= d.length as isize,
                        DifferentialEvent::Merge(m) => {
                            if let Some(c) = m.complete {
                                *s.records.entry(m.operator).or_default() += c as isize - m.length1 as isize - m.length2 as isize;
                            }
                        }
                        DifferentialEvent::Batcher(b) => *s.batcher_bytes.entry(b.operator).or_default() += b.size_diff,
                        _ => {}
                    }
                }
            }
        });
        reg.insert::<timely::logging::TimelyEventBuilder, _>("timely", move |_time, data| {
            if let Some(data) = data {
                let mut s = s2.borrow_mut();
                for (_, e) in data.iter() {
                    if let timely::logging::TimelyEvent::Operates(o) = e {
                        s.names.insert(o.id, o.name.clone());
                    }
                }
            }
        });
    }
    sizes
}
