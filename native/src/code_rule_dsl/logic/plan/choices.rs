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
//! Kept in the process (each part of a run, each chunk), and across processes in the tape cache: an entry of facts
//! under a source named by the prepared program's key, a rewritten program as its `.rr` text (`program_text`, decision
//! 0155) named by its digest. The run's manifest names that source and the entry's plan (`named`), so collection keeps
//! the entry while it keeps the run.

use super::super::front::{Program, Value};
use super::super::program_facts::{self, Facts};
use super::super::program_text;
use crate::content_store::{ContentStore, FactsMemoKey, FactsPlanSet, FactsResult, HashKey, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

/// The entry's schema: bumped when its encoding or meaning changes (2: the program as its text, decision 0155).
const SCHEMA: u32 = 2;

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
    // The rules (and each derived-again group's), most of a program, digested by their structure: formatting the whole
    // program as text was 0.32 s of a 20-file edit's run (decision 0116, step 4). Every other field as its text. The
    // fields are named, not `..`: a field added to `Program` is a compile error here until it is digested.
    let Program {
        decls, nullary, lattice, facts, rules, inputs, outputs, kindsets, demand, demand_dropped, magic_extras, magic_report, gates, policies,
        settings, natives, rx_spans, ast_patterns, doc_scopes, answers, text_globs, qualifiers, tapes, again, together, shapes,
    } = prog;
    let rules_digest = super::super::derived::digest(|d| {
        let mut rules_of = |rules: &[super::super::front::Rule]| {
            d.u64(rules.len() as u64);
            for r in rules {
                super::digest_atom(d, &r.head);
                d.u64(r.body.len() as u64);
                r.body.iter().for_each(|l| super::digest_literal(d, l));
            }
        };
        rules_of(rules);
        for g in again {
            rules_of(&g.rules);
            rules_of(&g.deltas);
        }
    });
    hash.0.update(b"\0rules\0");
    hash.0.update(rules_digest.to_le_bytes());
    let again_names: Vec<(&BTreeSet<String>, &BTreeSet<String>, &Vec<String>, &Vec<String>)> = again.iter().map(|g| (&g.scc, &g.owned, &g.reads, &g.olds)).collect();
    let _ = std::fmt::Write::write_fmt(
        &mut hash,
        format_args!(
            "\0{decls:?}\0{nullary:?}\0{lattice:?}\0{facts:?}\0{inputs:?}\0{outputs:?}\0{kindsets:?}\0{demand:?}\0{demand_dropped:?}\0{magic_extras:?}\0{magic_report:?}\0{gates:?}\0{policies:?}\0{settings:?}\0{natives:?}\0{rx_spans:?}\0{ast_patterns:?}\0{doc_scopes:?}\0{answers:?}\0{text_globs:?}\0{qualifiers:?}\0{tapes:?}\0{again_names:?}\0{together:?}\0{shapes:?}"
        ),
    );
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
    /// With the entries, by path, and the texts of the programs they name, by digest: all read at the start. And the
    /// run's late sources (the manifest's), to which each program text read or stored is added, so collection keeps
    /// it while it keeps the run.
    static DURABLE: std::cell::RefCell<Option<Held>> = const { std::cell::RefCell::new(None) };
}

/// The store, the prepared program's key, the entries by path, the programs' texts by digest, the late sources.
type Held = (ContentStore, [u8; 32], HashMap<String, Vec<u8>>, HashMap<[u8; 32], String>, Arc<Mutex<Vec<crate::content_store::SourceId>>>);

/// While held, this thread's runs keep their choices in the store at `path`, under prepared program `prepared`.
pub(in crate::code_rule_dsl) struct Durable;

impl Durable {
    /// Every entry kept under `prepared` is read here, at once, before the run's first wave (decision 0104): a
    /// lookup in the waves reads memory only.
    pub(in crate::code_rule_dsl) fn start(path: &std::path::Path, prepared: [u8; 32], late: Arc<Mutex<Vec<crate::content_store::SourceId>>>) -> Durable {
        let store = ContentStore::open(path).ok();
        DURABLE.with(|d| {
            *d.borrow_mut() = store.map(|s| {
                let stored = read_all(&s, &prepared).unwrap_or_default();
                // The rewritten programs' texts too, in one read: parsed when looked up (`decode_chosen`).
                let digests: Vec<[u8; 32]> = stored.values().filter_map(|entry| rewritten(entry)).collect();
                let texts = program_text::texts(&s, &digests).unwrap_or_default();
                late.lock().expect("the late sources").extend(texts.values().map(|(_, id)| *id));
                let texts = texts.into_iter().map(|(d, (text, _))| (d, text)).collect();
                // And the plans of its rules (decision 0116 step 2), for this thread's run to take.
                let _span = super::super::profile::span(|| "kept plans, read from the store".into());
                let plans = read_plans(&s, &prepared).unwrap_or_default();
                super::super::profile::decision("kept plans (decision 0116)", || format!("{} plans read from the store", plans.len()));
                super::take_stored_plans(Arc::new(plans));
                (s, prepared, stored, texts, late)
            })
        });
        Durable
    }
}

impl Drop for Durable {
    /// The run's plans, where it changed any, queued for the store (decision 0116 step 2): the run's plans are
    /// still held here (`KeepPlans` is let before this).
    fn drop(&mut self) {
        if let Some((store, prepared, _, _, _)) = DURABLE.with(|d| d.borrow_mut().take())
            && let Some(plans) = super::plans_to_store()
        {
            let _span = super::super::profile::span(|| "kept plans, queued for the store".into());
            let rows: Vec<Vec<Value>> = plans.iter().map(|(k, p)| super::StoredPlan::row(*k, p).into_iter().map(Value::Int).collect()).collect();
            let bytes = rows.iter().map(|r| 9 * r.len() + 8).sum();
            super::super::profile::decision("kept plans (decision 0116)", || format!("{} plans queued for the store", rows.len()));
            store.write_back("the kept plans", bytes, vec![(crate::content_store::write_back::Key::of(&[b"facts_memo/plans", &prepared]), None)], Box::new(move |store| write_plans(store, &prepared, &rows).map(drop)));
        }
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
    // Decoded when first looked up: a run asks for a few of the entries read at its start (decoding them all there was
    // 0.07 s of an edit's).
    let found = DURABLE.with(|d| {
        let d = d.borrow();
        let (_, _, entries, texts, _) = d.as_ref()?;
        entries.get(&path(key)).and_then(|entry| decode_chosen(texts, entry))
    })?;
    keep(*key, found.clone());
    holds(found.nodes, nodes).then_some(found)
}

/// Keeps the choices made for program `key`, in this process and in the store.
pub(super) fn remember(key: [u8; 32], chosen: Chosen) {
    keep(key, chosen.clone());
    DURABLE.with(|d| {
        let d = d.borrow();
        if let Some((store, prepared, _, _, late)) = d.as_ref() {
            let _span = super::super::profile::span(|| "planning choices, kept in the store, queued".into());
            // Made into facts and written by the store's writer (decision 0102).
            let (prepared, late) = (*prepared, Arc::clone(late));
            store.write_back("the planning choices", 0, vec![(queued(&prepared, &key), None)], Box::new(move |store| write(store, &prepared, &key, &chosen, &late).map(drop)));
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
/// Every entry kept under `prepared`, by path, as stored: each is decoded when looked up (`lookup`).
fn read_all(store: &ContentStore, prepared: &[u8; 32]) -> StoreResult<HashMap<String, Vec<u8>>> {
    let Some(source) = store.sources_by_hashes(&[HashKey::sha256(source_text(prepared).as_bytes())])?.pop().flatten() else { return Ok(HashMap::new()) };
    Ok(store.facts_results_of(source, SCHEMA, plan().plans[0])?.into_iter().collect())
}

/// The kept plans' schema, apart from the choices' so neither reads the other's entries.
const PLANS_SCHEMA: u32 = 2;
/// The kept plans' entry under the prepared program's source, and its relation: one row per plan
/// (`StoredPlan::row`).
const PLANS_PATH: &str = "rerail/plans";
const PLANS_REL: &str = "rr_kept_plan";

/// The plans kept under `prepared`: one read of the store.
fn read_plans(store: &ContentStore, prepared: &[u8; 32]) -> StoreResult<super::StoredPlans> {
    let Some(source) = store.sources_by_hashes(&[HashKey::sha256(source_text(prepared).as_bytes())])?.pop().flatten() else { return Ok(Default::default()) };
    let Some((_, entry)) = store.facts_results_of(source, PLANS_SCHEMA, plan().plans[0])?.into_iter().find(|(path, _)| path == PLANS_PATH) else { return Ok(Default::default()) };
    let Ok(facts) = program_facts::decode(&entry) else { return Ok(Default::default()) };
    Ok(facts
        .get(PLANS_REL)
        .into_iter()
        .flatten()
        .filter_map(|row| super::StoredPlan::of_row(&row.iter().map(|v| match v { Value::Int(i) => Some(*i), Value::Str(_) => None }).collect::<Option<Vec<_>>>()?))
        .collect())
}

fn write_plans(store: &ContentStore, prepared: &[u8; 32], rows: &[Vec<Value>]) -> StoreResult<bool> {
    let mut facts = Facts::new();
    facts.insert(PLANS_REL.to_owned(), rows.to_vec());
    let text = source_text(prepared);
    let hash = HashKey::sha256(text.as_bytes());
    let source = match store.held_source_by_hash(hash)? {
        Some(id) => id,
        None => store.put_source_hashed(text.as_bytes(), hash, &[])?,
    };
    let set = plan();
    let entry = program_facts::encode(&facts);
    store.put_facts_memo(FactsMemoKey { source, fact_schema: PLANS_SCHEMA, path: PLANS_PATH }, &set, &[FactsResult { plan: set.plans[0], path_dependent: true, entry }], &[])?;
    Ok(true)
}

/// The digest of the rewritten program a choice's entry names, if it names one.
fn rewritten(entry: &[u8]) -> Option<[u8; 32]> {
    match program_facts::decode(entry).ok()?.get(REWRITTEN)?.first()?.as_slice() {
        [Value::Str(digest)] => unhex(digest),
        _ => None,
    }
}

/// A choice's entry: its facts, and the rewritten program (`REWRITTEN`, its digest) read off its text in `texts`.
fn decode_chosen(texts: &HashMap<[u8; 32], String>, entry: &[u8]) -> Option<Chosen> {
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
    let program = match facts.get(REWRITTEN).and_then(|rows| rows.first()).map(Vec::as_slice) {
        Some([Value::Str(digest)]) => {
            let digest = unhex(digest)?;
            Some(Arc::new(program_text::read(texts.get(&digest)?).ok()?))
        }
        Some(_) => return None,
        None => None,
    };
    Some(Chosen { program, inlined: names(INLINED), demanded: names(DEMANDED), nodes })
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    if text.len() != 64 {
        return None;
    }
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn write(store: &ContentStore, prepared: &[u8; 32], key: &[u8; 32], chosen: &Chosen, late: &Mutex<Vec<crate::content_store::SourceId>>) -> StoreResult<bool> {
    let mut facts = Facts::new();
    if let Some(program) = &chosen.program {
        // Kept as its text (decision 0155). A program that does not read back the same is not kept: the next run
        // chooses again.
        let Some((digest, text)) = program_text::store(store, program)? else { return Ok(false) };
        late.lock().expect("the late sources").push(text);
        facts.insert(REWRITTEN.to_owned(), vec![vec![Value::Str(hex(&digest))]]);
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
pub mod tests;
