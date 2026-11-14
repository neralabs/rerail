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

use super::{RuleIndex, Split, split_in};
use crate::code_rule_dsl::logic::front::Program;
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
    heads: HashMap<String, u64>,
}

struct Kept {
    keys: Keys,
    split: Arc<Split>,
}

/// A run's kept splits, shared with its jobs.
type Splits = Arc<Mutex<Vec<Arc<Kept>>>>;

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

fn keys(prog: &Program, index: &RuleIndex<'_>, derived: &BTreeSet<&str>) -> Keys {
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
    // Each relation's digest on the pool's threads (a pack's program has thousands).
    use rayon::prelude::*;
    let heads = derived
        .iter()
        .copied()
        .collect::<Vec<&str>>()
        .par_iter()
        .map(|rel| {
            let mut h = hasher();
            prog.decls.get(*rel).hash(&mut h);
            index.rules_of(rel).for_each(|r| r.hash(&mut h));
            ((*rel).to_owned(), h.finish())
        })
        .collect::<HashMap<String, u64>>();
    // A base relation's declaration is its own key, as a derived relation's rules are: a relation that stops being
    // derived (factorizing hands an export on as its parts) changes only what reads it, not every relation.
    let mut heads = heads;
    for (rel, cols) in prog.decls.iter().filter(|(rel, _)| !derived.contains(rel.as_str())) {
        let mut h = hasher();
        b"base".hash(&mut h);
        cols.hash(&mut h);
        heads.insert(rel.to_string(), h.finish());
    }
    Keys { global: g.finish(), heads }
}

/// The relations whose rules differ between `was` and `now`, or that are derived in only one of them (a relation no
/// longer derived: what reads it reads a base relation now).
fn changed(was: &Keys, now: &Keys) -> Vec<String> {
    let mut out: Vec<String> = now.heads.iter().filter(|(rel, d)| was.heads.get(*rel) != Some(*d)).map(|(rel, _)| rel.clone()).collect();
    out.extend(was.heads.keys().filter(|rel| !now.heads.contains_key(*rel)).cloned());
    out
}

/// `prog`'s split: a kept one if the program is the one it was made for, else the closest kept one with its cone
/// computed again, else a whole split.
pub(super) fn split(prog: &Program) -> Arc<Split> {
    let index = RuleIndex::of(prog);
    let derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let keys = keys(prog, &index, &derived);
    let held = kept_splits();
    let kept: Vec<Arc<Kept>> = held.as_ref().map(|h| h.lock().expect("the kept splits").clone()).unwrap_or_default();
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
        let mut work: Vec<&str> = changed.iter().map(String::as_str).collect();
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
            let split = split_in(prog, &index, &derived, &cone, Some(&k.split));
            if std::env::var_os("RERAIL_SPLIT_CHECK").is_some() {
                let whole = split_in(prog, &index, &derived, &derived, None);
                assert_eq!((&split.constants, &split.locations, &split.filters, &split.local), (&whole.constants, &whole.locations, &whole.filters, &whole.local), "a split of its cone differs from a whole split");
            }
            split
        }
        _ => {
            let _span = crate::code_rule_dsl::logic::profile::span(|| format!("split: all {} relations ({why})", derived.len()));
            split_in(prog, &index, &derived, &derived, None)
        }
    };
    let split = Arc::new(split);
    if let Some(held) = held {
        let made = Arc::new(Kept { keys, split: Arc::clone(&split) });
        let mut kept = held.lock().expect("the kept splits");
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
        held.lock().expect("the kept splits").clear();
    }
}
