//! The last splits made (`super::split_in`), kept so a split of a program that differs from one of them in a few
//! rules computes only what the difference reaches.
//!
//! Preparation splits a program many times as it rewrites it: each round of pushing file-local joins checks its new
//! helpers by splitting the whole program again, and so do factorizing and locating. A pack's program has tens of
//! thousands of rules, and a round changes a few hundred. So each split here finds the kept split closest to the
//! program, by a digest of each derived relation's rules: the relations whose digest differs (or that came or went)
//! and every relation reading one, transitively, are its cone, and only the cone is computed again. Outside the cone,
//! no relation reads one inside, so each keeps its place in every fixpoint; that is why the result equals a whole
//! split (`RERAIL_SPLIT_CHECK=1` computes both and compares them).
//!
//! The kept splits are a run's own (decision 0192): each run opens its own at its start (`own`) and gives its caller's
//! back as it ends, and a job of the run shares them (`carried`). A nested run, on any thread, keeps its splits apart
//! from its caller's, whose program it would otherwise push out. They are read and replaced under a lock, but never
//! computed under it.

use super::{RuleIndex, Split, shape::Form, split_in};
use crate::code_rule_dsl::logic::front::{List, Literal, Name, Program};
use rustc_hash::FxHashMap as HashMap;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

/// How many splits are kept: the program being rewritten, and the one before it.
const KEPT: usize = 2;

/// What a split reads of a program: one digest of what every relation's place reads besides rules (the inputs, the
/// facts' relations, the natives and the lattices), one per derived relation of its rules, in order, and its
/// declaration, and one per base relation of its declaration.
struct Keys {
    global: u64,
    heads: HashMap<Name, u64>,
}

/// A rule body as the run's splits read it: its list (held, so its address stays its own), its digest, and its form
/// in numbers (`shape::Form`), made the first time a split asks it of the body.
pub(super) struct Body {
    list: List<Literal>,
    digest: u64,
    form: std::sync::OnceLock<Form>,
}

impl Body {
    fn new(list: &List<Literal>) -> Body {
        let mut h = hasher();
        list.as_slice().hash(&mut h);
        Body { list: list.clone(), digest: h.finish(), form: std::sync::OnceLock::new() }
    }

    pub(super) fn form(&self) -> &Form {
        self.form.get_or_init(|| Form::of(&self.list))
    }
}

struct Kept {
    keys: Keys,
    split: Arc<Split>,
}

/// A run's kept splits, shared with its jobs, and the digests of the rule bodies its keys have read.
type Splits = Arc<Held>;

#[derive(Default)]
struct Held {
    kept: Mutex<Vec<Arc<Kept>>>,
    /// Each rule body digested in this run, by its list's address, with the list itself: holding it keeps the
    /// address its own and the list shared, so an edit of the body copies it first (`List::make_mut`) and gets a new
    /// address. A round of the push rewrites a few rules and shares the rest, so the keys of the next split hash only
    /// what it changed (hashing every rule each split was 4.6% of a cold batch's main thread), and the next split
    /// numbers only those bodies (`Body::form`).
    bodies: parking_lot::RwLock<HashMap<(usize, usize), Arc<Body>>>,
}

thread_local! {
    /// The kept splits of the run under way on this thread (`own`); `None` outside a run, where a split keeps nothing.
    static SPLITS: std::cell::RefCell<Option<Splits>> = std::cell::RefCell::new(None);
}

/// Carries the run's kept splits to a job on another thread (`logic::run_state`).
pub(in crate::code_rule_dsl) fn carried(c: &mut crate::code_rule_dsl::logic::run_state::Capture) {
    c.cloned(&["SPLITS"], &SPLITS);
}

/// A run's own kept splits on this thread until dropped, when its caller's come back.
pub(in crate::code_rule_dsl) fn own() -> impl Drop {
    struct Own(Option<Splits>);
    impl Drop for Own {
        fn drop(&mut self) {
            SPLITS.with(|s| s.replace(self.0.take()));
        }
    }
    Own(SPLITS.with(|s| s.replace(Some(Splits::default()))))
}

fn kept_splits() -> Option<Splits> {
    SPLITS.with(|s| s.borrow().clone())
}

/// The keys' hasher: foldhash's quality hasher with a fixed seed, so a key is the same on every thread of the run.
/// std's SipHash, over every rule of a pack's program on every split, was 2% of a cold batch's main thread.
fn hasher() -> impl Hasher {
    use std::hash::BuildHasher;
    foldhash::quality::FixedState::with_seed(0x5eed_5011_7000_0001).build_hasher()
}

/// Each rule's body as the run holds it (`Body`), in program order: the run's own for a list it has read, else made
/// now and kept. The map's lock is taken once, not once per rule.
pub(super) fn bodies_of(prog: &Program) -> Vec<Arc<Body>> {
    bodies_held(prog, kept_splits().as_deref())
}

fn bodies_held(prog: &Program, held: Option<&Held>) -> Vec<Arc<Body>> {
    let at = |body: &List<Literal>| (body.as_slice().as_ptr() as usize, body.len());
    let Some(held) = held else { return prog.rules.iter().map(|r| Arc::new(Body::new(&r.body))).collect() };
    let found: Vec<Option<Arc<Body>>> = {
        let map = held.bodies.read();
        prog.rules.iter().map(|r| if r.body.is_empty() { None } else { map.get(&at(&r.body)).cloned() }).collect()
    };
    let made: Vec<(usize, Arc<Body>)> = found
        .iter()
        .enumerate()
        .filter(|(_, b)| b.is_none())
        .map(|(i, _)| (i, Arc::new(Body::new(&prog.rules[i].body))))
        .collect();
    let mut out: Vec<Option<Arc<Body>>> = found;
    if !made.is_empty() {
        let mut map = held.bodies.write();
        for (i, b) in made {
            let body = &prog.rules[i].body;
            if !body.is_empty() {
                map.insert(at(body), Arc::clone(&b));
            }
            out[i] = Some(b);
        }
    }
    out.into_iter().map(|b| b.expect("every rule's body")).collect()
}

fn keys(prog: &Program, index: &RuleIndex<'_>, derived: &BTreeSet<&str>, bodies: &[Arc<Body>]) -> Keys {
    let mut g = hasher();
    // Only the inputs a split asks about: whether a derived relation, a fact or a phase's export is an input
    // (`split_constants`, `run_constant`, the constant facts). An input that is none of them is read as a base relation
    // either way, so dropping one no rule reads (`corpus`, before the last split) leaves the kept splits usable.
    prog.inputs
        .iter()
        .filter(|i| derived.contains(i.as_str()) || prog.facts.contains_key(i.as_str()) || crate::code_rule_dsl::logic::plan::corpus::phases::is_phased(i))
        .for_each(|i| i.hash(&mut g));
    prog.facts.keys().for_each(|k| k.hash(&mut g));
    prog.lattice.keys().for_each(|k| k.hash(&mut g));
    format!("{:?}", prog.natives).hash(&mut g);
    // The relations `locate` moved to the cross-file part (decision 0290).
    super::moved(prog).hash(&mut g);
    // Each relation's digest (a pack's program has thousands).
    let heads_new = derived
        .iter()
        .copied()
        .collect::<Vec<&str>>()
        .iter()
        .map(|rel| {
            let mut h = hasher();
            prog.decls.get(*rel).hash(&mut h);
            let ids = index.rule_ids_of(rel);
            for &i in ids {
                prog.rules[i].head.hash(&mut h);
                h.write_u64(bodies[i].digest);
            }
            (prog.rules[ids[0]].head.rel.clone(), h.finish())
        })
        .collect::<Vec<_>>();
    let mut heads: HashMap<Name, u64> = HashMap::with_capacity_and_hasher(heads_new.len() + prog.decls.len(), Default::default());
    heads.extend(heads_new);
    // A base relation's declaration is its own key, as a derived relation's rules are: a relation that stops being
    // derived (factorizing hands an export on as its parts) changes only what reads it, not every relation.
    for (rel, cols) in prog.decls.iter().filter(|(rel, _)| !derived.contains(rel.as_str())) {
        let mut h = hasher();
        b"base".hash(&mut h);
        cols.hash(&mut h);
        heads.insert(rel.clone(), h.finish());
    }
    Keys { global: g.finish(), heads }
}

/// The relations whose rules differ between `was` and `now`, or that are derived in only one of them (a relation no
/// longer derived: what reads it reads a base relation now).
fn changed(was: &Keys, now: &Keys) -> Vec<Name> {
    let mut out: Vec<Name> = now.heads.iter().filter(|(rel, d)| was.heads.get(*rel) != Some(*d)).map(|(rel, _)| rel.clone()).collect();
    out.extend(was.heads.keys().filter(|rel| !now.heads.contains_key(*rel)).cloned());
    out
}

/// `prog`'s split: a kept one if the program is the one it was made for, else the closest kept one with its cone
/// computed again, else a whole split.
pub(super) fn split(prog: &Program) -> Arc<Split> {
    let index = RuleIndex::of(prog);
    let derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let held = kept_splits();
    let bodies = bodies_held(prog, held.as_deref());
    let keys = keys(prog, &index, &derived, &bodies);
    let kept: Vec<Arc<Kept>> = held.as_ref().map(|h| h.kept.lock().expect("the kept splits").clone()).unwrap_or_default();
    let closest = kept
        .iter()
        .filter(|k| k.keys.global == keys.global)
        .map(|k| (changed(&k.keys, &keys), k))
        .min_by_key(|(c, _)| c.len());
    if let Some((c, k)) = &closest
        && c.is_empty()
    {
        return Arc::clone(&k.split);
    }
    // The cone: the changed relations and every relation reading one, transitively.
    let cone = closest.as_ref().map(|(changed, _)| {
        let mut cone: BTreeSet<&str> = BTreeSet::new();
        let mut work: Vec<&str> = changed.iter().map(Name::as_str).collect();
        while let Some(rel) = work.pop() {
            for reader in index.heads_reading(rel) {
                if cone.insert(reader) {
                    work.push(reader);
                }
            }
        }
        cone.extend(changed.iter().filter_map(|r| derived.get(r.as_str()).copied()));
        cone
    });
    let why = match (&closest, &cone) {
        (None, _) => format!("no kept split of this program's inputs and facts ({} kept)", kept.len()),
        (Some((c, _)), Some(cone)) => format!("{} changed, a cone of {}", c.len(), cone.len()),
        _ => String::new(),
    };
    let split = match (&closest, cone) {
        // Past half the program, the cone saves little over a whole split.
        (Some((_, k)), Some(cone)) if cone.len() * 2 <= derived.len() => {
            let _span = crate::code_rule_dsl::logic::profile::span(|| format!("split: {} of {} relations again", cone.len(), derived.len()));
            let split = split_in(prog, &index, &derived, &bodies, &cone, Some(&k.split));
            if std::env::var_os("RERAIL_SPLIT_CHECK").is_some() {
                let whole = split_in(prog, &index, &derived, &bodies, &derived, None);
                assert_eq!((&split.constants, &split.locations, &split.filters, &split.local), (&whole.constants, &whole.locations, &whole.filters, &whole.local), "a split of its cone differs from a whole split");
            }
            split
        }
        _ => {
            let _span = crate::code_rule_dsl::logic::profile::span(|| format!("split: all {} relations ({why})", derived.len()));
            split_in(prog, &index, &derived, &bodies, &derived, None)
        }
    };
    let split = Arc::new(split);
    if let Some(held) = held {
        let made = Arc::new(Kept { keys, split: Arc::clone(&split) });
        let mut kept = held.kept.lock().expect("the kept splits");
        if kept.len() >= KEPT {
            kept.remove(0);
        }
        kept.push(made);
    }
    split
}

/// Drops the run's kept splits: its preparation is done with them.
pub fn forget() {
    if let Some(held) = kept_splits() {
        held.kept.lock().expect("the kept splits").clear();
        held.bodies.write().clear();
    }
}
