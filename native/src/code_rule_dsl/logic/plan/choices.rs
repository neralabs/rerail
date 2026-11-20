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

use super::super::front::Value;
use super::super::program_facts::{self, Facts};
use super::super::program_text;
use crate::content_store::{ContentStore, FactsMemoKey, FactsPlanSet, FactsResult, HashKey, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

/// The entry's schema: bumped when its encoding or meaning changes (the program as its text, decision 0155). Never a
/// retired corpus memo schema (`live_fact_schema`): it was 3, and a new store's first collection swept the entries.
const SCHEMA: u32 = crate::content_store::live_fact_schema(106);

/// A run takes choices made at a scale at least its own over this.
pub(super) const GROWTH: f64 = 2.0;

/// The choices for one program: the program's version as the rewrites left it (`None`: as it was), what each named,
/// and the scale they were made at.
#[derive(Clone)]
pub(super) struct Chosen {
    /// The rewritten program's version (t36 items 6 and 7), which the run reads: the passes' overlay of the program's,
    /// or the stored image (or the image of the stored text). `None` where the program was not rewritten.
    pub(super) version: Option<super::body::Placed>,
    /// The relations inlined (`keeps::choose`) and demanded (`demand::split_negations`), for the profile.
    pub(super) inlined: Vec<String>,
    pub(super) demanded: Vec<String>,
    pub(super) nodes: f64,
}

/// A digest of everything the rewrites read of a program, and of what else changes their choices: the program's
/// version's digest (decision 0173 §7: its rules, declarations, facts, inputs, outputs and every other field, as its
/// image holds them, with its maintained parts'), kept as the version was made, so the key reads no rule. Both
/// rewrites keep the program's meaning whatever its rules' order, so the digest, a sum, need not see it: a program of
/// the same rules in another order takes the same choices, which derive the same rows.
pub fn key(p: &super::body::Placed) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.plan-choices/v2\0");
    hash.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    for switch in ["RERAIL_INLINE", "RERAIL_INLINE_ONLY", "RERAIL_OBSERVED_STEPS"] {
        hash.update(b"\0");
        hash.update(std::env::var(switch).unwrap_or_default().as_bytes());
    }
    // A finding's message template is beside the program, not of it (decision 0225 §3): its edit plans nothing again.
    // The settings are one `.about` relation of the version's digest (a sum of its fact relations' hashes): its hash
    // is taken out and the settings other than `.message` put in, so the key reads no rule and a few settings.
    use crate::code_rule_dsl::logic::program_image::Cell;
    let v = &p.v;
    let settings = v.about_facts().into_iter().find(|f| v.text(f.name()) == "setting");
    hash.update(b"\0program\0");
    hash.update(p.digest().wrapping_sub(settings.map_or(0, |f| f.hash())).to_le_bytes());
    for row in settings.into_iter().flat_map(|f| f.rows()) {
        let cells: Vec<Cell> = row.collect();
        let text = |c: &Cell| match *c { Cell::Int(i) => i.to_string(), Cell::Str(a) => v.text(a).to_owned(), Cell::Branded(b) => b.to_string() };
        if cells.get(2).is_some_and(|k| text(k) == crate::code_rule_dsl::logic::rr::lower::findings::MESSAGE) {
            continue;
        }
        hash.update(b"\0setting");
        for c in &cells {
            hash.update(b"\0");
            hash.update(text(c).as_bytes());
        }
    }
    hash.update([u8::from(p.whole)]);
    hash.finalize().into()
}

/// Whether choices made at `kept` nodes are taken at `now`.
pub(super) fn holds(kept: f64, now: f64) -> bool {
    now <= kept.max(1.0) * GROWTH
}

/// The choices kept by key, the caller's (decision 0442: the shared object's).
#[derive(Default)]
struct KeptChoices(Mutex<HashMap<[u8; 32], Chosen>>);

fn kept() -> std::sync::Arc<KeptChoices> {
    crate::code_rule_dsl::logic::profile::shared::<KeptChoices>()
}

/// At most this many programs' choices are kept in the process.
const KEPT_PROGRAMS: usize = 16;

thread_local! {
    /// The durable side of this thread's corpus run, while it holds `Durable`: the tape cache's store and the
    /// prepared program's key, which names the source the entries live under.
    /// With the entries, by path, and the texts of the programs they name, by digest: all read at the start.
    static DURABLE: std::cell::RefCell<Option<Arc<Held>>> = const { std::cell::RefCell::new(None) };
}

/// Carries the run's durable side to a job on another thread (`run_state`): a nested run there looks up and keeps
/// its choices in the same store, under the same program, as the run's thread.
pub(in crate::code_rule_dsl) fn carried(c: &mut crate::code_rule_dsl::logic::run_state::Capture) {
    c.cloned(&["DURABLE"], &DURABLE);
}

/// The store, the prepared program's key, the entries by path, the programs' images by digest.
type Held = (ContentStore, [u8; 32], HashMap<String, Vec<u8>>, HashMap<[u8; 32], Vec<u8>>);

/// While held, this thread's runs keep their choices in the store at `path`, under prepared program `prepared`.
pub(in crate::code_rule_dsl) struct Durable;

impl Durable {
    /// Every entry kept under `prepared` is read here, at once, before the run's first wave (decision 0104): a
    /// lookup in the waves reads memory only.
    pub(in crate::code_rule_dsl) fn start(path: &std::path::Path, prepared: [u8; 32], late: Arc<Mutex<Vec<crate::content_store::SourceId>>>) -> Durable {
        let store = ContentStore::open(path).ok();
        DURABLE.with(|d| {
            *d.borrow_mut() = store.map(|s| -> Arc<Held> {
                let stored = read_all(&s, &prepared).unwrap_or_default();
                // The rewritten programs' images too, in one read: opened when looked up (`decode_chosen`).
                let digests: Vec<[u8; 32]> = stored.values().filter_map(|entry| rewritten(entry)).collect();
                let images = program_text::stored(&s, &digests).unwrap_or_default();
                late.lock().expect("the late sources").extend(images.values().flat_map(|(_, ids)| ids.iter().copied()));
                let images = images.into_iter().map(|(d, (bytes, _))| (d, bytes)).collect();
                // And the plans of its rules (decision 0116 step 2), for this thread's run to take.
                let _span = super::super::profile::span(|| "kept plans, read from the store".into());
                let (plans, ready) = read_plans(&s, &prepared).unwrap_or_default();
                super::super::profile::decision("kept plans (decision 0116)", || format!("{} plans and {} versions' prepared plans read from the store", plans.len(), ready.len()));
                super::take_stored_plans(Arc::new(plans));
                super::take_stored_prepared(ready);
                Arc::new((s, prepared, stored, images))
            })
        });
        Durable
    }
}

impl Drop for Durable {
    /// The run's plans, where it changed any, queued for the store (decision 0116 step 2): the run's plans are
    /// still held here (`KeepPlans` is let before this).
    fn drop(&mut self) {
        if let Some(held) = DURABLE.with(|d| d.borrow_mut().take()) {
            let (store, prepared) = (&held.0, held.1);
            // Decision 0243: a one-shot run serves its waiting join searches here, within a share of its CPU, so their
            // proposals are written with the memo.
            super::joins::drain();
            // The join memo (decision 0203 §6) is written with the plans: where only it changed, the plans as read.
            let notes = super::joins::to_store().map(note_rows);
            // And the prepared plans (decision 0361 step 5, Q228), where the run made any.
            let ready = super::prepared_to_store();
            let Some(plans) = super::plans_to_store().or_else(|| (notes.is_some() || ready.is_some()).then(super::stored_plans).flatten()) else { return };
            let notes = notes.unwrap_or_default();
            let ready = super::prepared::store_rows(&ready.unwrap_or_else(super::prepared_held));
            let _span = super::super::profile::span(|| "kept plans, queued for the store".into());
            let rows: Vec<Vec<Value>> = plans.iter().map(|(k, p)| super::StoredPlan::row(*k, p).into_iter().map(Value::Int).collect()).collect();
            super::super::profile::decision("kept plans (decision 0116)", || format!("{} plans and {} join notes queued for the store", rows.len(), notes.0.len()));
            // Plans and notes hold keys and numbers, no atom ids (decision 0170).
            store.record("the kept plans", vec![crate::content_store::write_back::Key::of(&[b"facts_memo/plans", &prepared])], &prepared, Some(plans_entry(&rows, &notes, &ready)), apply_plans);
        }
    }
}

/// The choices for program `key` that a run at `nodes` takes: this process's, else the store's.
pub(super) fn lookup(key: &[u8; 32], nodes: f64) -> Option<Chosen> {
    // Memory is ahead of the store (decision 0102): choices this process holds are the newest there are, so the store
    // is read only for a program this process has not planned. Nor does the read wait for the writer: what a queued
    // write holds is in memory already.
    if let Some(c) = kept().0.lock().expect("the kept choices").get(key) {
        return holds(c.nodes, nodes).then(|| c.clone());
    }
    // Decoded when first looked up: a run asks for a few of the entries read at its start (decoding them all there was
    // 0.07 s of an edit's).
    let found = DURABLE.with(|d| {
        let d = d.borrow();
        let (_, _, entries, images) = &**d.as_ref()?;
        entries.get(&path(key)).and_then(|entry| decode_chosen(images, entry))
    })?;
    keep(*key, found.clone());
    holds(found.nodes, nodes).then_some(found)
}

/// Keeps the choices made for program `key`, in this process and in the store.
pub(super) fn remember(key: [u8; 32], chosen: Chosen) {
    keep(key, chosen.clone());
    DURABLE.with(|d| {
        let d = d.borrow();
        if let Some((store, prepared, _, _)) = d.as_deref() {
            let _span = super::super::profile::span(|| "planning choices, kept in the store, queued".into());
            // Made into facts here, written by the store's writer in the run's record (decisions 0102, 0306). Its facts
            // hold strings as text (`program_facts::encode`'s own table), no atom: kept after a fork too (decision 0170).
            let Some((packed, bytes)) = encode(&chosen) else { return };
            store.record("the planning choices", vec![queued(prepared, &key)], &(prepared, key, packed), Some(bytes), apply);
        }
    });
}

/// What the write of program `key`'s choices is queued under (`write_back`).
fn queued(prepared: &[u8; 32], key: &[u8; 32]) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"facts_memo/choices", prepared, key])
}

fn keep(key: [u8; 32], chosen: Chosen) {
    let held = kept();
    let mut kept = held.0.lock().expect("the kept choices");
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

/// The kept plans' schema, apart from the choices' (`SCHEMA`) so neither reads the other's entries.
const PLANS_SCHEMA: u32 = crate::content_store::live_fact_schema(107);
/// The kept plans' entry under the prepared program's source, and its relation: one row per plan
/// (`StoredPlan::row`).
const PLANS_PATH: &str = "rerail/plans";
const PLANS_REL: &str = "rr_kept_plan";
/// The join memo's notes and exact statistics (decision 0203 §6), in the same entry.
const NOTES_REL: &str = "rr_join_note";
const EXACT_REL: &str = "rr_join_exact";
/// The columns the notes raised to keep heavy values (decision 0209 §4, O172).
const RAISED_REL: &str = "rr_join_raised";
/// The prepared plans of the versions a run planned (decision 0361 step 5, Q228), in the same entry
/// (`prepared::store_rows`).
const PREPARED_REL: &str = "rr_prepared_plan";

/// The join memo's notes and exact statistics read from `facts`, given to this process's memo.
fn take_notes(facts: &Facts) {
    let notes = facts
        .get(NOTES_REL)
        .into_iter()
        .flatten()
        .filter_map(|row| super::joins::of_note_row(&row.iter().map(|v| match v { Value::Int(i) => Some(*i), Value::Str(_) | Value::Branded(_) => None }).collect::<Option<Vec<_>>>()?))
        .collect();
    let exact = facts
        .get(EXACT_REL)
        .into_iter()
        .flatten()
        .filter_map(|row| match row.as_slice() {
            [Value::Int(hi), Value::Int(lo), Value::Int(rule), Value::Str(rel), Value::Str(cols), Value::Int(a), Value::Int(b)] => super::joins::of_exact_row(&[*hi, *lo, *rule], rel, cols, [*a, *b]),
            _ => None,
        })
        .collect();
    let raised = facts
        .get(RAISED_REL)
        .into_iter()
        .flatten()
        .filter_map(|row| match row.as_slice() {
            [Value::Int(hi), Value::Int(lo), Value::Str(rel), Value::Int(c)] => Some(((*hi as u64 as u128) << 64 | *lo as u64 as u128, rel.to_string(), usize::try_from(*c).ok()?)),
            _ => None,
        })
        .collect();
    super::joins::from_store((notes, exact, raised));
}

/// The memo's rows for the entry.
pub(super) type NoteRows = (Vec<Vec<Value>>, Vec<Vec<Value>>, Vec<Vec<Value>>);

fn note_rows((notes, exact, raised): super::joins::Stored) -> NoteRows {
    let notes = notes.iter().map(|(k, n)| super::joins::note_row(*k, n).into_iter().map(Value::Int).collect()).collect();
    let exact = exact
        .iter()
        .map(|(k, rule, sig, c)| {
            let (ints, rel, cols, c) = super::joins::exact_row(*k, *rule, sig, *c);
            ints.into_iter().map(Value::Int).chain([Value::Str(rel.into()), Value::Str(cols.into()), Value::Int(c[0]), Value::Int(c[1])]).collect()
        })
        .collect();
    let raised = raised.iter().map(|(k, rel, c)| vec![Value::Int((k >> 64) as i64), Value::Int(*k as i64), Value::Str(rel.as_str().into()), Value::Int(*c as i64)]).collect();
    (notes, exact, raised)
}

/// The plans kept under `prepared`: one read of the store.
fn read_plans(store: &ContentStore, prepared: &[u8; 32]) -> StoreResult<(super::StoredPlans, super::prepared::ByVersion)> {
    let Some(source) = store.sources_by_hashes(&[HashKey::sha256(source_text(prepared).as_bytes())])?.pop().flatten() else { return Ok(Default::default()) };
    let Some((_, entry)) = store.facts_results_of(source, PLANS_SCHEMA, plan().plans[0])?.into_iter().find(|(path, _)| path == PLANS_PATH) else { return Ok(Default::default()) };
    let Ok(facts) = program_facts::decode(&entry) else { return Ok(Default::default()) };
    take_notes(&facts);
    let ready: Vec<Vec<Value>> = facts.get(PREPARED_REL).into_iter().flatten().map(|row| row.as_slice().to_vec()).collect();
    let plans = facts
        .get(PLANS_REL)
        .into_iter()
        .flatten()
        .filter_map(|row| super::StoredPlan::of_row(&row.iter().map(|v| match v { Value::Int(i) => Some(*i), Value::Str(_) | Value::Branded(_) => None }).collect::<Option<Vec<_>>>()?))
        .collect();
    Ok((plans, super::prepared::of_store_rows(&ready)))
}

/// The kept plans' and join notes' facts entry, made on the run's side (decision 0306).
fn plans_entry(rows: &[Vec<Value>], notes: &NoteRows, ready: &[Vec<Value>]) -> Vec<u8> {
    let mut facts = Facts::new();
    facts.insert(PLANS_REL.to_owned(), rows.iter().map(crate::code_rule_dsl::logic::front::List::from).collect());
    if !ready.is_empty() {
        facts.insert(PREPARED_REL.to_owned(), ready.iter().map(crate::code_rule_dsl::logic::front::List::from).collect());
    }
    if !notes.0.is_empty() {
        facts.insert(NOTES_REL.to_owned(), notes.0.iter().map(crate::code_rule_dsl::logic::front::List::from).collect());
        facts.insert(EXACT_REL.to_owned(), notes.1.iter().map(crate::code_rule_dsl::logic::front::List::from).collect());
        facts.insert(RAISED_REL.to_owned(), notes.2.iter().map(crate::code_rule_dsl::logic::front::List::from).collect());
    }
    program_facts::encode(&facts)
}

/// Writes the kept plans' entry (`plans_entry`, the item's bytes) under prepared program `prepared`'s source.
fn apply_plans(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
    let prepared: [u8; 32] = crate::content_store::record::codec::from_body("the kept plans", body)?;
    let store = cx.store;
    let text = source_text(&prepared);
    let hash = HashKey::sha256(text.as_bytes());
    let source = match store.source_by_hash(hash)? {
        Some(id) => id,
        None => store.put_source_hashed(text.as_bytes(), hash, &[])?,
    };
    let set = plan();
    store.put_facts_memo(FactsMemoKey { source, fact_schema: PLANS_SCHEMA, path: PLANS_PATH }, &set, &[FactsResult { plan: set.plans[0], path_dependent: true, entry: cx.bytes.to_vec() }], &[]).map(drop)
}

/// The digest of the rewritten program a choice's entry names, if it names one.
fn rewritten(entry: &[u8]) -> Option<[u8; 32]> {
    match program_facts::decode(entry).ok()?.get(REWRITTEN)?.first()?.as_slice() {
        [Value::Str(digest)] => unhex(digest),
        _ => None,
    }
}

/// A choice's entry: its facts, and the rewritten program (`REWRITTEN`, its digest) opened from its image in `images`.
fn decode_chosen(images: &HashMap<[u8; 32], Vec<u8>>, entry: &[u8]) -> Option<Chosen> {
    let Ok(facts) = program_facts::decode(entry) else { return None };
    let nodes = match facts.get(SCALE).and_then(|rows| rows.first()).map(|r| r.as_slice()) {
        Some([Value::Int(n)]) => *n as f64,
        _ => return None,
    };
    let names = |rel: &str| -> Vec<String> {
        let mut rows: Vec<(i64, String)> = facts
            .get(rel)
            .into_iter()
            .flatten()
            .filter_map(|row| match row.as_slice() {
                [Value::Int(k), Value::Str(s)] => Some((*k, s.to_string())),
                _ => None,
            })
            .collect();
        rows.sort();
        rows.into_iter().map(|(_, s)| s).collect()
    };
    let version = match facts.get(REWRITTEN).and_then(|rows| rows.first()).map(|r| r.as_slice()) {
        Some([Value::Str(digest)]) => {
            // The image is the program's version, read as it is, its rules in order.
            let bytes = images.get(&unhex(&digest)?)?;
            Some(super::body::Placed::whole(crate::code_rule_dsl::logic::program_image::Version::open(bytes).ok()?))
        }
        Some(_) => return None,
        None => None,
    };
    Some(Chosen { version, inlined: names(INLINED), demanded: names(DEMANDED), nodes })
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

/// `chosen` as its record's item carries it: its rewritten program's image, and its facts entry. `None` for a program
/// whose image is not made: nothing is kept, and the next run chooses again.
fn encode(chosen: &Chosen) -> Option<(program_text::Packed, Vec<u8>)> {
    let mut facts = Facts::new();
    let mut programs = Vec::new();
    if let Some(version) = &chosen.version {
        let encoded = program_text::encode_version(&version.v)?;
        facts.insert(REWRITTEN.to_owned(), vec![[Value::Str(hex(&encoded.digest).into())].into()]);
        programs.push(encoded);
    }
    facts.insert(SCALE.to_owned(), vec![[Value::Int(chosen.nodes.round() as i64)].into()]);
    let list = |names: &[String]| names.iter().enumerate().map(|(k, s)| [Value::Int(k as i64), Value::Str(s.clone().into())].into()).collect();
    facts.insert(INLINED.to_owned(), list(&chosen.inlined));
    facts.insert(DEMANDED.to_owned(), list(&chosen.demanded));
    Some(program_text::pack(programs, program_facts::encode(&facts)))
}

/// Writes choices `encode` made for program `key` of prepared program `prepared`: the rewritten program's text, named
/// by the manifest, and the entry, under the prepared program's source.
fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
    let (prepared, key, packed): ([u8; 32], [u8; 32], program_text::Packed) = crate::content_store::record::codec::from_body("the planning choices", body)?;
    let store = cx.store;
    let (sources, entry) = program_text::store_packed(store, &packed, cx.bytes)?;
    cx.name(sources);
    let text = source_text(&prepared);
    let hash = HashKey::sha256(text.as_bytes());
    let source = match store.source_by_hash(hash)? {
        Some(id) => id,
        None => store.put_source_hashed(text.as_bytes(), hash, &[])?,
    };
    let set = plan();
    let path = path(&key);
    store.put_facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: &path }, &set, &[FactsResult { plan: set.plans[0], path_dependent: true, entry: entry.to_vec() }], &[]).map(drop)
}

/// The source and plan a prepared program's choices live under, for the run's manifest: the source is made when the
/// store has none yet, so the first run's manifest names the entries it writes.
pub(in crate::code_rule_dsl) fn named(prepared: &[u8; 32]) -> (String, [u8; 32]) {
    (source_text(prepared), plan().plans[0])
}

#[cfg(test)]
pub mod tests;
