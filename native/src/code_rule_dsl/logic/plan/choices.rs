//! The whole-program choices a corpus run's plan makes, kept from one run to the next (decision 0090 §5).
//!
//! Before its strata are planned, a corpus run's program is rewritten twice from the planner's estimates: what each
//! derived relation keeps (decision 0099, `keeps::choose`: stored, or inlined into its readers), then which relations
//! are computed only for the keys their readers reach (decision 0048, `demand::split_negations`). Both read every
//! rule and price most of them, so they cost what the program's size is, not what the data is: on the Python pack
//! (534 programs), 6 s of CPU for the file-local part of a run that evaluates one edited file, 2 s for the cross-file
//! part, 1.3 s for the embed slice.
//!
//! Both rewrites keep the program's meaning: whatever they choose, the run derives the same rows. Their estimates
//! only make the choice cheap or dear to run. So the rewritten program is kept, under
//!
//! - **the program**, a digest of the whole program the rewrites read (every field, its rules, declarations, facts,
//!   inputs, outputs, gates and qualifiers), with the engine build and the switches that change the choices
//!   (`RERAIL_INLINE`, `RERAIL_INLINE_ONLY`, `RERAIL_OBSERVED_STEPS`);
//! - **the scale it was chosen at**, the corpus's nodes in the planner's statistics.
//!
//! A run takes the kept choices when its own scale is at most `GROWTH` times theirs: a choice made over more rows is
//! taken for fewer, where every relation is smaller and any plan costs less. A run over many more rows than the kept
//! choices saw (a relation stored small can be large there) chooses again and keeps its own. An edit's run, which
//! evaluates one file against a store whose last full run planned for every file, takes the full run's choices; so
//! does each later chunk of a run, where the first chunk's choices were made at the same size.
//!
//! Kept in the process (each part of a run, each chunk), and across processes in the tape cache, as the program as
//! facts (`program_facts`, decision 0074), under a source named by the prepared program's key: the run's manifest
//! names that source and the entry's plan (`named`), so collection keeps the entry while it keeps the run.

use super::super::front::{Program, Value};
use super::super::program_facts::{self, Facts};
use crate::content_store::{ContentStore, FactsMemoKey, FactsPlanSet, FactsResult, HashKey, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The entry's schema: bumped when its encoding or meaning changes.
const SCHEMA: u32 = 1;

/// The program's name in the entry's facts.
const P: &str = "chosen";

/// A run takes choices made at a scale at least its own over this.
pub(super) const GROWTH: f64 = 2.0;

/// The choices for one program: the program as the rewrites left it (`None`: as it was), what each named, and the
/// scale they were made at.
#[derive(Clone)]
pub(super) struct Chosen {
    pub(super) program: Option<Arc<Program>>,
    /// The relations inlined (`keeps::choose`) and demanded (`demand::split_negations`), for the profile.
    pub(super) inlined: Vec<String>,
    pub(super) demanded: Vec<String>,
    pub(super) nodes: f64,
}

/// A digest of everything the rewrites read of a program, and of what else changes their choices.
pub(super) fn key(prog: &Program) -> [u8; 32] {
    struct Hashing(Sha256);
    impl std::fmt::Write for Hashing {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            self.0.update(s.as_bytes());
            Ok(())
        }
    }
    let mut hash = Hashing(Sha256::new());
    hash.0.update(b"rerail.plan-choices/v1\0");
    hash.0.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    for switch in ["RERAIL_INLINE", "RERAIL_INLINE_ONLY", "RERAIL_OBSERVED_STEPS"] {
        hash.0.update(b"\0");
        hash.0.update(std::env::var(switch).unwrap_or_default().as_bytes());
    }
    let _ = std::fmt::Write::write_fmt(&mut hash, format_args!("\0{prog:?}"));
    hash.0.finalize().into()
}

/// Whether choices made at `kept` nodes are taken at `now`.
pub(super) fn holds(kept: f64, now: f64) -> bool {
    now <= kept.max(1.0) * GROWTH
}

/// The choices kept in this process, by key.
fn kept() -> &'static Mutex<HashMap<[u8; 32], Chosen>> {
    static KEPT: std::sync::OnceLock<Mutex<HashMap<[u8; 32], Chosen>>> = std::sync::OnceLock::new();
    KEPT.get_or_init(Default::default)
}

/// At most this many programs' choices are kept in the process.
const KEPT_PROGRAMS: usize = 16;

thread_local! {
    /// The durable side of this thread's corpus run, while it holds `Durable`: the tape cache's store and the
    /// prepared program's key, which names the source the entries live under.
    static DURABLE: std::cell::RefCell<Option<(ContentStore, [u8; 32], HashMap<String, Chosen>)>> = const { std::cell::RefCell::new(None) };
}

/// While held, this thread's runs keep their choices in the store at `path`, under prepared program `prepared`.
pub(in crate::code_rule_dsl) struct Durable;

impl Durable {
    /// Every entry kept under `prepared` is read here, at once, before the run's first wave (decision 0104): a
    /// lookup in the waves reads memory only.
    pub(in crate::code_rule_dsl) fn start(path: &std::path::Path, prepared: [u8; 32]) -> Durable {
        let store = ContentStore::open(path).ok();
        DURABLE.with(|d| {
            *d.borrow_mut() = store.map(|s| {
                let stored = read_all(&s, &prepared).unwrap_or_default();
                (s, prepared, stored)
            })
        });
        Durable
    }
}

impl Drop for Durable {
    fn drop(&mut self) {
        DURABLE.with(|d| d.borrow_mut().take());
    }
}

/// The choices for program `key` that a run at `nodes` takes: this process's, else the store's.
pub(super) fn lookup(key: &[u8; 32], nodes: f64) -> Option<Chosen> {
    // Memory is ahead of the store (decision 0102): choices this process holds are the newest there are, so the store
    // is read only for a program this process has not planned. Nor does the read wait for the writer: what a queued
    // write holds is in memory already.
    if let Some(c) = kept().lock().expect("the kept choices").get(key) {
        return holds(c.nodes, nodes).then(|| c.clone());
    }
    let found = DURABLE.with(|d| d.borrow().as_ref()?.2.get(&path(key)).cloned())?;
    keep(*key, found.clone());
    holds(found.nodes, nodes).then_some(found)
}

/// Keeps the choices made for program `key`, in this process and in the store.
pub(super) fn remember(key: [u8; 32], chosen: Chosen) {
    keep(key, chosen.clone());
    DURABLE.with(|d| {
        let d = d.borrow();
        if let Some((store, prepared, _)) = d.as_ref() {
            let _span = super::super::profile::span(|| "planning choices, kept in the store, queued".into());
            // Made into facts and written by the store's writer (decision 0102).
            let prepared = *prepared;
            store.write_back("the planning choices", 0, vec![(queued(&prepared, &key), None)], Box::new(move |store| write(store, &prepared, &key, &chosen).map(drop)));
        }
    });
}

/// What the write of program `key`'s choices is queued under (`write_back`).
fn queued(prepared: &[u8; 32], key: &[u8; 32]) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"facts_memo/choices", prepared, key])
}

fn keep(key: [u8; 32], chosen: Chosen) {
    let mut kept = kept().lock().expect("the kept choices");
    if kept.len() >= KEPT_PROGRAMS && !kept.contains_key(&key) {
        kept.clear();
    }
    kept.insert(key, chosen);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The source a prepared program's choices are kept under: a small text naming its key.
fn source_text(prepared: &[u8; 32]) -> String {
    format!("rerail/plan-choices\0{}", hex(prepared))
}

/// The entry's plan: the build and the schema (the program's digest is the entry's path).
fn plan() -> FactsPlanSet {
    let mut hash = Sha256::new();
    hash.update(b"rerail.plan-choices/v1\0");
    hash.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    FactsPlanSet::new(vec![hash.finalize().into()])
}

/// One program's entry, by its key, under the prepared program's source.
fn path(key: &[u8; 32]) -> String {
    format!("rerail/plan-choices/{}", hex(key))
}

const SCALE: &str = "rr_chosen_scale";
const INLINED: &str = "rr_chosen_inlined";
const DEMANDED: &str = "rr_chosen_demanded";
const REWRITTEN: &str = "rr_chosen_rewritten";

/// Every program's choices kept under `prepared`, by entry path: one read of the store (the source's pin is the
/// manifest's, `named`).
fn read_all(store: &ContentStore, prepared: &[u8; 32]) -> StoreResult<HashMap<String, Chosen>> {
    let Some(source) = store.sources_by_hashes(&[HashKey::sha256(source_text(prepared).as_bytes())])?.pop().flatten() else { return Ok(HashMap::new()) };
    Ok(store.facts_results_of(source, SCHEMA, plan().plans[0])?.into_iter().filter_map(|(path, entry)| Some((path, decode_chosen(&entry)?))).collect())
}

fn decode_chosen(entry: &[u8]) -> Option<Chosen> {
    let Ok(facts) = program_facts::decode(entry) else { return None };
    let nodes = match facts.get(SCALE).and_then(|rows| rows.first()).map(Vec::as_slice) {
        Some([Value::Int(n)]) => *n as f64,
        _ => return None,
    };
    let names = |rel: &str| -> Vec<String> {
        let mut rows: Vec<(i64, String)> = facts
            .get(rel)
            .into_iter()
            .flatten()
            .filter_map(|row| match row.as_slice() {
                [Value::Int(k), Value::Str(s)] => Some((*k, s.clone())),
                _ => None,
            })
            .collect();
        rows.sort();
        rows.into_iter().map(|(_, s)| s).collect()
    };
    let program = if facts.contains_key(REWRITTEN) {
        match program_facts::program(&facts, P) {
            Ok(p) => Some(Arc::new(p)),
            Err(_) => return None,
        }
    } else {
        None
    };
    Some(Chosen { program, inlined: names(INLINED), demanded: names(DEMANDED), nodes })
}

fn write(store: &ContentStore, prepared: &[u8; 32], key: &[u8; 32], chosen: &Chosen) -> StoreResult<bool> {
    let mut facts = Facts::new();
    if let Some(program) = &chosen.program {
        program_facts::write(&mut facts, P, program);
        // A program that does not read back the same is not kept: the next run chooses again.
        let back = program_facts::program(&facts, P);
        if !back.is_ok_and(|back| program_facts::debug_digest(&back) == program_facts::debug_digest(program)) {
            return Ok(false);
        }
        facts.insert(REWRITTEN.to_owned(), vec![vec![Value::Int(1)]]);
    }
    facts.insert(SCALE.to_owned(), vec![vec![Value::Int(chosen.nodes.round() as i64)]]);
    let list = |names: &[String]| names.iter().enumerate().map(|(k, s)| vec![Value::Int(k as i64), Value::Str(s.clone())]).collect();
    facts.insert(INLINED.to_owned(), list(&chosen.inlined));
    facts.insert(DEMANDED.to_owned(), list(&chosen.demanded));
    let text = source_text(prepared);
    let hash = HashKey::sha256(text.as_bytes());
    let source = match store.held_source_by_hash(hash)? {
        Some(id) => id,
        None => store.put_source_hashed(text.as_bytes(), hash, &[])?,
    };
    let set = plan();
    let path = path(key);
    let entry = program_facts::encode(&facts);
    store.put_facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: &path }, &set, &[FactsResult { plan: set.plans[0], path_dependent: true, entry }], &[])?;
    Ok(true)
}

/// The source and plan a prepared program's choices live under, for the run's manifest: the source is made when the
/// store has none yet, so the first run's manifest names the entries it writes.
pub(in crate::code_rule_dsl) fn named(prepared: &[u8; 32]) -> (String, [u8; 32]) {
    (source_text(prepared), plan().plans[0])
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn kept_choices_are_taken_at_their_scale_or_less_and_read_back_from_the_store() {
        let dir = std::env::temp_dir().join(format!("rerail-choices-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("store.sqlite");
        let program = super::super::super::rr::parse(".decl e(x: int)\n.decl f(x: int)\n.output f\ne(1).\nf(x) :- e(x).\n").unwrap();
        let rewritten = super::super::super::rr::parse(".decl e(x: int)\n.decl f(x: int)\n.output f\ne(1).\nf(1).\n").unwrap();
        let key = key(&program);
        assert_ne!(key, super::key(&rewritten), "the key is the program's");
        assert!(holds(1000.0, 10.0) && holds(1000.0, 2000.0) && !holds(1000.0, 2001.0));
        let prepared = [3u8; 32];
        {
            let _durable = Durable::start(&db, prepared);
            remember(key, Chosen { program: Some(Arc::new(rewritten.clone())), inlined: vec!["e".into()], demanded: vec![], nodes: 1000.0 });
        }
        // Another process: nothing kept in memory, the store's entry read back.
        kept().lock().unwrap().clear();
        let store = ContentStore::open(&db).unwrap();
        let back = read_all(&store, &prepared).unwrap().remove(&path(&key)).expect("kept in the store");
        assert_eq!(format!("{:?}", back.program.as_deref()), format!("{:?}", Some(&rewritten)));
        assert_eq!((back.inlined.clone(), back.nodes), (vec!["e".to_owned()], 1000.0));
        let (text, _) = named(&prepared);
        assert!(store.sources_by_hashes(&[HashKey::sha256(text.as_bytes())]).unwrap()[0].is_some());
        // Under another prepared program, nothing.
        assert!(read_all(&store, &[4u8; 32]).unwrap().is_empty());
        {
            let _durable = Durable::start(&db, prepared);
            assert!(lookup(&key, 10.0).is_some(), "a smaller run takes the choices");
            assert!(lookup(&key, 5000.0).is_none(), "a run over many more nodes chooses again");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
