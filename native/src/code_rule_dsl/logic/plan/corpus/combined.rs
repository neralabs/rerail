//! The combined program kept in the store across processes, keyed by the module graph (decision 0094 §6).
//!
//! Combining a pack (`combine_texts`) parses each program, reads its library modules, holds the libraries once and
//! merges the programs: on the Python pack, 534 programs and 10,721 rules, about 6.4 s of a warm run's CPU. The result
//! is a function of the programs' texts and of the library files they read. So a run keeps it under:
//!
//! - **the texts' digest** (`texts_digest`, which also names the switches that change combining), as a small source
//!   of its own;
//! - **a plan** naming the engine build, the grammar version, the library's directory and the comparison switches
//!   (`RERAIL_LIBRARIES`, `RERAIL_REQUESTS`);
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
    for switch in ["RERAIL_LIBRARIES", "RERAIL_REQUESTS"] {
        hash.update(b"\0");
        hash.update(std::env::var(switch).unwrap_or_default().as_bytes());
    }
    FactsPlanSet::new(vec![hash.finalize().into()])
}

/// The combined program of texts `key`, when the store keeps one whose library files all still read as recorded.
pub(super) fn lookup(store: &ContentStore, key: &[u8; 32], lists: &crate::code_rule_dsl::logic::front::Lists) -> StoreResult<Option<Program>> {
    let Some(source) = store.held_source_by_hash(HashKey::sha256(source_text(key).as_bytes()))? else { return Ok(None) };
    let set = plan();
    let memo = store.facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH })?;
    let Some(result) = memo.results.get(&set.plans[0]) else { return Ok(None) };
    let Ok(facts) = program_facts::decode(&result.entry) else { return Ok(None) };
    let read = facts.get(READ_FILE).into_iter().flatten().map(|row| match row.as_slice() {
        [Value::Str(path), Value::Str(digest)] => Some(LibraryFile { path: PathBuf::from(path.as_str()), sha256: unhex(digest) }),
        _ => None,
    });
    for file in read {
        match file {
            Some(file) if file.unchanged() => {}
            _ => return Ok(None),
        }
    }
    let Some([Value::Str(digest)]) = facts.get(PROGRAM).and_then(|rows| rows.first()).map(|r| r.as_slice()) else { return Ok(None) };
    let Some(digest) = unhex(&digest) else { return Ok(None) };
    Ok(program_text::load(store, &digest, lists)?.map(|(program, _)| program))
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

/// Keeps `program`, combined from texts `key` after reading library files `read`, in the store. A program that does
/// not read back as written is not kept.
pub(super) fn remember(store: &ContentStore, key: &[u8; 32], read: &[LibraryFile], program: &Program) -> StoreResult<bool> {
    // A program that does not read back the same from its text is not kept: the next run combines it again.
    let Some((digest, _)) = program_text::store(store, program)? else { return Ok(false) };
    let mut facts = Facts::new();
    facts.insert(PROGRAM.to_owned(), vec![[Value::Str(hex(&digest).into())].into()]);
    let rows = read.iter().map(|f| [Value::Str(f.path.to_string_lossy().into_owned().into()), Value::Str(f.sha256.map(|d| hex(&d)).unwrap_or_default().into())].into()).collect();
    facts.insert(READ_FILE.to_owned(), rows);
    let text = source_text(key);
    let hash = HashKey::sha256(text.as_bytes());
    let source = match store.held_source_by_hash(hash)? {
        Some(id) => id,
        None => store.put_source_hashed(text.as_bytes(), hash, &[])?,
    };
    let set = plan();
    let entry = program_facts::encode(&facts);
    store.put_facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH }, &set, &[FactsResult { plan: set.plans[0], path_dependent: false, entry }], &[])?;
    Ok(true)
}

/// Names the program text of texts `key`'s entry, when the store keeps it, among the run's late sources (the
/// manifest's), so collection keeps it while it keeps the run: queued on the store's writer, after the entry's own
/// write when this process queued one.
pub(super) fn name_program(cache: &ContentStore, key: &[u8; 32], late: &std::sync::Arc<std::sync::Mutex<Vec<crate::content_store::SourceId>>>) {
    let (key, late) = (*key, std::sync::Arc::clone(late));
    cache.write_back(
        "the combined program's text, named",
        0,
        Vec::new(),
        Box::new(move |store| {
            let Some(source) = store.held_source_by_hash(HashKey::sha256(source_text(&key).as_bytes()))? else { return Ok(()) };
            let memo = store.facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH })?;
            let Some(result) = memo.results.get(&plan().plans[0]) else { return Ok(()) };
            let Ok(facts) = program_facts::decode(&result.entry) else { return Ok(()) };
            let Some([Value::Str(digest)]) = facts.get(PROGRAM).and_then(|rows| rows.first()).map(|r| r.as_slice()) else { return Ok(()) };
            let Some(digest) = unhex(&digest) else { return Ok(()) };
            let found = store.held_source_by_hash(HashKey::from_sha256_digest(digest))?;
            late.lock().expect("the late sources").extend(found);
            Ok(())
        }),
    );
}

/// What the write of texts `key`'s program is queued under (`write_back`, decision 0102).
pub(super) fn queued(key: &[u8; 32]) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"facts_memo/combined", key])
}

/// The source and plan the entry of texts `key` lives under, when the store keeps it: for the run's manifest. A
/// queued write of it first.
pub(super) fn named(key: &[u8; 32]) -> (String, [u8; 32]) {
    (source_text(key), plan().plans[0])
}

#[cfg(test)]
pub mod tests;
