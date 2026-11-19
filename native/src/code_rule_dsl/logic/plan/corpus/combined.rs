//! The combined program kept in the store across processes, keyed by the module graph (decision 0094 §6).
//!
//! Combining a pack (`combine_texts`) parses each program, reads its library modules, holds the libraries once and
//! merges the programs: on the Python pack, 534 programs and 10,721 rules, about 6.4 s of a warm run's CPU. The result
//! is a function of the programs' texts and of the library files they read. So a run keeps it under:
//!
//! - **the texts' digest** (`texts_digest`, which also names the switches that change combining), as a small source
//!   of its own;
//! - **a plan** naming the engine build, the grammar version, the library's directory and the switches that change
//!   combining (`corpus::COMBINING`);
//! - **the read record**: every library file the combine read or probed, misses included, with its bytes' digest
//!   (`rr::recording`). A run takes the entry only when each file still reads as recorded: an edited library file, or
//!   a file appearing where an include looked and found none, combines again.
//!
//! The entry is the read record as rows of `read_file` and the program's digest; the program is kept as its `.rr`
//! text (`program_text`, decision 0155). The run's manifest names the source and the plan (`named`), so collection
//! keeps the entry while it keeps the run.

use crate::code_rule_dsl::logic::front::{Program, Value};
use crate::code_rule_dsl::logic::program_facts::{self, Facts};
use crate::code_rule_dsl::logic::program_text;
use crate::code_rule_dsl::logic::rr::{self, LibraryFile};
use crate::content_store::{ContentStore, FactsMemoKey, FactsPlanSet, FactsResult, HashKey, StoreResult};
use sha2::{Digest as _, Sha256};
use std::path::PathBuf;

/// The entry's schema: bumped when its encoding or meaning changes (105: the program as its text, decision 0155).
const SCHEMA: u32 = crate::content_store::live_fact_schema(105);

/// The path a lookup names. The entry reads no path (`path_dependent: false`).
const PATH: &str = "rerail/combined";

/// The relation holding the program's text's digest (sha256 hex).
const PROGRAM: &str = "rr_combined_program";

/// The relation holding the images the combine read or kept (decision 0246 §6): their aliases (sha256 hex). A run
/// that names the entry names them too, so collection keeps them while it keeps the run.
const IMAGE: &str = "rr_combined_image";

/// The relation holding the read record: (path, sha256 hex, or "" for a miss).
const READ_FILE: &str = "rr_combined_read_file";

/// The source the entry of texts `key` is kept under: a small text naming the key.
fn source_text(key: &[u8; 32]) -> String {
    format!("rerail/combined\0{}", hex(key))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The entry's plan: what combining reads besides the texts and the library files.
fn plan() -> FactsPlanSet {
    let mut hash = Sha256::new();
    hash.update(b"rerail.combined/v1\0");
    hash.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    hash.update(crate::code_rule_dsl::build_ids::grammar_version().to_le_bytes());
    hash.update(rr::library().to_string_lossy().as_bytes());
    for switch in super::COMBINING {
        hash.update(b"\0");
        hash.update(std::env::var(switch).unwrap_or_default().as_bytes());
    }
    FactsPlanSet::new(vec![hash.finalize().into()])
}

/// The alias the entry of texts `key` ships under in the engine's bundle (decision 0264): its schema, plan and key.
fn shipped_key(key: &[u8; 32]) -> HashKey {
    HashKey::sha256(&[format!("rerail combined entry {SCHEMA}\0").as_bytes(), plan().plans[0].as_slice(), key.as_slice()].concat())
}

/// The combined program of texts `key`, when the store keeps one whose library files all still read as recorded; or,
/// where the store has none, the one the engine ships (decision 0264).
pub(super) fn lookup(store: &ContentStore, key: &[u8; 32], lists: &crate::code_rule_dsl::logic::front::Lists) -> StoreResult<Option<Program>> {
    Ok(lookup_read(store, key, lists)?.map(|(program, _)| program))
}

/// The key of a rule file's entry (decision 0275): the program `rr::read_file` reads of the file at `path` (canonical)
/// whose bytes' sha256 is `sha256`. The path is in it: the files an include finds beside it are another directory's
/// elsewhere, and the read record names this one's.
pub(in crate::code_rule_dsl) fn rule_file_key(path: &std::path::Path, library: &std::path::Path, sha256: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail/rule-file/v2\0");
    hash.update(library.as_os_str().as_encoded_bytes());
    hash.update(b"\0");
    hash.update(path.as_os_str().as_encoded_bytes());
    hash.update(b"\0");
    hash.update(sha256);
    hash.finalize().into()
}

/// The key of a program text's header (`rr::text_header`, decision 0275): the text's bytes. What its includes read
/// is the entry's read record, checked as a rule file's is.
pub(in crate::code_rule_dsl) fn text_header_key(src: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail/text-header/v2\0");
    hash.update(rr::library_root().as_os_str().as_encoded_bytes());
    hash.update(b"\0");
    hash.update(src.as_bytes());
    hash.finalize().into()
}

/// `lookup`, with the read record: every library file the program read or probed, in reading order. Each is checked
/// as `LibraryFile::unchanged` checks it, by its `stat` while that holds and by its bytes' sha256 when it moved: the
/// first key that settles it.
pub(in crate::code_rule_dsl) fn lookup_read(store: &ContentStore, key: &[u8; 32], lists: &crate::code_rule_dsl::logic::front::Lists) -> StoreResult<Option<(Program, Vec<LibraryFile>)>> {
    let Some((digest, read)) = read_record(store, key)? else { return Ok(None) };
    if !LibraryFile::inputs_match(&read) {
        return Ok(None);
    }
    Ok(program_text::load(store, &digest, lists)?.map(|(program, _)| (program, read)))
}

/// A process hit already checked these inputs. Match them against this store's entry and verify its program is available.
pub(in crate::code_rule_dsl) fn retains_read(store: &ContentStore, key: &[u8; 32], files: &[LibraryFile]) -> StoreResult<bool> {
    let Some((digest, read)) = read_record(store, key)? else { return Ok(false) };
    if read != files { return Ok(false); }
    program_text::available(store, &digest)
}

fn read_record(store: &ContentStore, key: &[u8; 32]) -> StoreResult<Option<([u8; 32], Vec<LibraryFile>)>> {
    // The store's entry, or, where the store has none, the one the engine ships (decision 0264).
    let entry = match store.source_by_hash(HashKey::sha256(source_text(key).as_bytes()))? {
        Some(source) => {
            let set = plan();
            let memo = store.facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH })?;
            let Some(result) = memo.results.get(&set.plans[0]) else { return Ok(None) };
            std::borrow::Cow::Owned(result.entry.clone())
        }
        None => match rr::images::shipped().and_then(|b| b.get(shipped_key(key))) {
            Some(bytes) => {
                crate::code_rule_dsl::prof::count("corpus.combined_shipped", 1);
                std::borrow::Cow::Borrowed(bytes)
            }
            None => return Ok(None),
        },
    };
    let Ok(facts) = program_facts::decode(&entry) else { return Ok(None) };
    let mut read = Vec::new();
    for row in facts.get(READ_FILE).into_iter().flatten() {
        let [Value::Str(path), Value::Str(digest)] = row.as_slice() else { return Ok(None) };
        let sha256 = if digest.is_empty() {
            None
        } else {
            let Some(hash) = unhex(digest) else { return Ok(None) };
            Some(hash)
        };
        read.push(LibraryFile { path: PathBuf::from(path.as_str()), sha256 });
    }
    let Some([Value::Str(digest)]) = facts.get(PROGRAM).and_then(|rows| rows.first()).map(|r| r.as_slice()) else { return Ok(None) };
    let Some(digest) = unhex(&digest) else { return Ok(None) };
    Ok(Some((digest, read)))
}

thread_local! {
    /// The rule files' entries read on this thread before its run (`rr::read_file`, decision 0275): the outermost
    /// run names them in its manifest (`take_read_before`), as it names its combined program's.
    static READ_BEFORE: std::cell::RefCell<Vec<[u8; 32]>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Notes that a run on this thread reads the rule file entry `key` (found, or queued to be written).
pub(in crate::code_rule_dsl) fn read_before_the_run(key: [u8; 32]) {
    READ_BEFORE.with_borrow_mut(|keys| {
        if !keys.contains(&key) {
            keys.push(key);
        }
    });
}

/// The rule file entries this thread read since the last outermost run took them.
pub(super) fn take_read_before() -> Vec<[u8; 32]> {
    READ_BEFORE.with_borrow_mut(std::mem::take)
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// Keeps `program`, combined from texts `key` after reading library files `read`, in the store: made into facts and
/// encoded here, written by the store's writer in the run's record as `what` (decisions 0102, 0306). A program that
/// does not read back as written is not kept: the next run combines it again.
pub(in crate::code_rule_dsl) fn record(store: &ContentStore, what: &'static str, key: &[u8; 32], read: &[LibraryFile], program: &Program, images: &[HashKey]) {
    let Some(encoded) = program_text::encode(program) else { return };
    let mut facts = Facts::new();
    facts.insert(PROGRAM.to_owned(), vec![[Value::Str(hex(&encoded.digest).into())].into()]);
    let rows = read.iter().map(|f| [Value::Str(f.path.to_string_lossy().into_owned().into()), Value::Str(f.sha256.map(|d| hex(&d)).unwrap_or_default().into())].into()).collect();
    facts.insert(READ_FILE.to_owned(), rows);
    let images = images.iter().filter_map(|k| k.sha256_bytes()).map(|d| [Value::Str(hex(&d).into())].into()).collect();
    facts.insert(IMAGE.to_owned(), images);
    let entry = program_facts::encode(&facts);
    // A run building the engine's bundle (`RERAIL_EXPORT_IMAGES`) ships the entry (its program's image ships with
    // `program_text::encode`).
    rr::images::export(shipped_key(key), &entry);
    let (packed, bytes) = program_text::pack(vec![encoded], entry);
    store.record(what, vec![queued(key)], &(*key, packed), Some(bytes), apply);
}

fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
    let (key, packed): ([u8; 32], program_text::Packed) = crate::content_store::record::codec::from_body("the combined program", body)?;
    let store = cx.store;
    let (text_source, entry) = program_text::store_packed(store, &packed, cx.bytes)?;
    let text = source_text(&key);
    let hash = HashKey::sha256(text.as_bytes());
    let source = match store.source_by_hash(hash)? {
        Some(id) => id,
        None => store.put_source_hashed(text.as_bytes(), hash, &[])?,
    };
    let set = plan();
    store.put_facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH }, &set, &[FactsResult { plan: set.plans[0], path_dependent: false, entry: entry.to_vec() }], &[])?;
    // No manifest names the entry until the run that combined it records its own, and the run's hold comes after the
    // combine. Meanwhile a collection (a batch's earlier run's, started once that run's manifest was written) took it:
    // a batch combined its pack again every time. Pinned under the entry's own owner until that run holds it
    // (`release`); a run that never names it leaves the pins to lapse.
    let lease = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) + LEASE_SECS;
    // The entry, and every source its text was kept as (the text and its image, decision 0173).
    for kept in std::iter::once(source).chain(text_source) {
        store.pin_source(kept, &owner(&key), lease)?;
    }
    Ok(())
}

/// How long an entry stays pinned when no run names it: longer than a run.
const LEASE_SECS: i64 = 6 * 3600;

/// The owner of the pins `remember` makes for texts `key`'s entry.
fn owner(key: &[u8; 32]) -> String {
    format!("combined:{}", hex(key))
}

/// `remember`'s pins of texts `key`'s entry, held by the run that names it (`named`, `name_program`) and released
/// when that run ends: queued after its manifest, which names the entry by then, as the run's own hold is
/// (`HeldPlan`).
pub(super) struct Pins<'a> {
    cache: &'a ContentStore,
    key: [u8; 32],
}

impl<'a> Pins<'a> {
    pub(super) fn new(cache: &'a ContentStore, key: &[u8; 32]) -> Self {
        Pins { cache, key: *key }
    }
}

impl Drop for Pins<'_> {
    fn drop(&mut self) {
        self.cache.record_release("the combined program's pins released", owner(&self.key));
    }
}

/// Names the program text of texts `key`'s entry, when the store keeps it, among the run's late sources (the
/// manifest's), so collection keeps it while it keeps the run: queued on the store's writer, after the entry's own
/// write when this process queued one.
pub(super) fn name_program(cache: &ContentStore, key: &[u8; 32], late: &std::sync::Arc<std::sync::Mutex<Vec<crate::content_store::SourceId>>>) {
    // Stored already (decision 0228): named now, kept meanwhile by the run's collection lease (decision 0306 step 4).
    // The writer finds what a write queued before it stores.
    if let Some(found) = program_source(cache, key).ok().flatten() {
        late.lock().expect("the late sources").push(found);
        late.lock().expect("the late sources").extend(image_sources(cache, key).ok().unwrap_or_default());
        return;
    }
    // Not stored, and no write of the entry queued (`remember`'s): the writer would find nothing either.
    if !cache.queued(&queued(key)) {
        return;
    }
    // In the run's record (decision 0306), after the entry's write.
    fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
        let key: [u8; 32] = crate::content_store::record::codec::from_body("the combined program's text, named", body)?;
        cx.name(program_source(cx.store, &key)?);
        cx.name(image_sources(cx.store, &key)?);
        Ok(())
    }
    cache.record("the combined program's text, named", Vec::new(), key, None, apply);
}

/// The source of the program text of texts `key`'s entry, when the store keeps the entry and the text.
fn program_source(store: &ContentStore, key: &[u8; 32]) -> StoreResult<Option<crate::content_store::SourceId>> {
    let Some(source) = store.source_by_hash(HashKey::sha256(source_text(key).as_bytes()))? else { return Ok(None) };
    let memo = store.facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH })?;
    let Some(result) = memo.results.get(&plan().plans[0]) else { return Ok(None) };
    let Ok(facts) = program_facts::decode(&result.entry) else { return Ok(None) };
    let Some([Value::Str(digest)]) = facts.get(PROGRAM).and_then(|rows| rows.first()).map(|r| r.as_slice()) else { return Ok(None) };
    let Some(digest) = unhex(digest) else { return Ok(None) };
    store.source_by_hash(HashKey::from_sha256_digest(digest))
}

/// The images texts `key`'s entry names (`IMAGE`) that the store still keeps.
fn image_sources(store: &ContentStore, key: &[u8; 32]) -> StoreResult<Vec<crate::content_store::SourceId>> {
    let Some(source) = store.source_by_hash(HashKey::sha256(source_text(key).as_bytes()))? else { return Ok(Vec::new()) };
    let memo = store.facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH })?;
    let Some(result) = memo.results.get(&plan().plans[0]) else { return Ok(Vec::new()) };
    let Ok(facts) = program_facts::decode(&result.entry) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for row in facts.get(IMAGE).into_iter().flatten() {
        if let [Value::Str(digest)] = row.as_slice()
            && let Some(digest) = unhex(digest)
            && let Some(id) = store.source_by_alias(HashKey::from_sha256_digest(digest))?
        {
            out.push(id);
        }
    }
    crate::code_rule_dsl::prof::count("corpus.combined_images_named", out.len());
    Ok(out)
}

/// What the write of texts `key`'s program is queued under (`write_back`, decision 0102).
pub(in crate::code_rule_dsl) fn queued(key: &[u8; 32]) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"facts_memo/combined", key])
}

/// The source and plan the entry of texts `key` lives under, when the store keeps it: for the run's manifest. A
/// queued write of it first.
pub(super) fn named(key: &[u8; 32]) -> (String, [u8; 32]) {
    (source_text(key), plan().plans[0])
}

#[cfg(test)]
pub mod tests;
