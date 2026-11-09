//! A corpus run's file-local results, memoised per file (decision 0052): what the chunks hand on for one document
//! (its rows of each exported relation and its file-local output lines) under its content, its path and the run's
//! plan. A later run whose plan is the same reads a file's entry instead of parsing and evaluating it.
//!
//! The rows are replayable as they are because their values are stable across runs: a document's key is a function
//! of its path (`doc_keys`), a node is `key << 32 | index` in its tape (a function of the content and the grammar),
//! a kind is its grammar code, and a symbol is an atom of the persistent store, which never gives one out again
//! (decision 0049): the run interns into it. The plan names everything else the rows depend on: the engine build, the
//! grammar version and the file-local program (its constants as atoms). It is one per run, so the run's manifest names
//! it (decision 0053); the document key is in the entry, and an entry under another key is a miss.

use crate::content_store::{ContentStore, FactsFrame, FactsMemoKey, FactsPlanSet, FactsResult, HashKey, SourceId, StoreError, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, HashSet};

/// The memo's fact schema: bumped when an entry's meaning changes.
/// When it changes, the old number joins `RETIRED_CORPUS_MEMO_SCHEMAS` only if its rows could outlive their plans
/// (they cannot since decision 0053: the plan names the engine build, and dead plans are collected).
const SCHEMA: u32 = 5;
const _: () = {
    let retired = crate::content_store::RETIRED_CORPUS_MEMO_SCHEMAS;
    let mut i = 0;
    while i < retired.len() {
        assert!(retired[i] != SCHEMA, "the corpus memo's schema is retired");
        i += 1;
    }
};

/// The entry's key for a base relation's rows (`rows`), kept for the cross-file part that reads them: a memoised
/// file hands on its base rows as an evaluated one does.
pub(super) fn base_key(rel: &str) -> String {
    format!("base:{rel}")
}

/// What the chunks handed on for one document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Entry {
    /// The document key the rows were made under (`doc_keys`): their node ids are `key << 32 | index`.
    pub key: i64,
    /// The document's nodes (for the run's report).
    pub nodes: u64,
    /// Per exported relation: its width and its rows, flat.
    pub rows: BTreeMap<String, (usize, Vec<i64>)>,
    /// Per file-local output: its node columns and its lines.
    pub lines: BTreeMap<String, (Vec<usize>, Vec<String>)>,
}

/// Each path's document key: from its hash, in `1..2^30`, the next free one on a collision. A key is in each entry,
/// so a key a collision moved only misses the memo. Guests (decision 0055) take `2^30..2^31`, so every key's
/// `key << 32` stays positive.
pub(in crate::code_rule_dsl) fn doc_keys(paths: &[String]) -> Vec<i64> {
    const SPAN: u64 = (1 << 30) - 1;
    let mut taken = HashSet::with_capacity(paths.len());
    paths
        .iter()
        .map(|path| {
            let hash: [u8; 32] = Sha256::digest(path.as_bytes()).into();
            let mut key = u64::from_le_bytes(hash[..8].try_into().expect("8 bytes")) % SPAN + 1;
            while !taken.insert(key) {
                key = key % SPAN + 1;
            }
            key as i64
        })
        .collect()
}

/// The run's plan: the engine build, the grammar version and `program` (a digest of the file-local part as lowered,
/// its atoms included).
pub(super) fn plan_digest(program: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local/v2\0");
    hash.update(env!("RERAIL_ENGINE_BUILD_ID").as_bytes());
    hash.update(crate::code_rule_dsl::workspace::grammar_version().to_le_bytes());
    // A memoised row's node ids are those of the scheme it was made under: preorder, or per definition.
    if crate::code_rule_dsl::facts::vtab::syntax::definition_ids() {
        hash.update(b"node-ids:definition\0");
    }
    hash.update(program);
    hash.finalize().into()
}

fn plan(program: &[u8; 32]) -> FactsPlanSet {
    FactsPlanSet::new(vec![plan_digest(program)])
}

/// A digest of a program part: its rules, declarations, inputs and outputs as lowered, every fact, those the engine
/// reads (a guest policy) as well as those its rules read, and the spans and patterns the batch matches for it.
pub(super) fn program_digest(program: &crate::code_rule_dsl::logic::front::Program) -> [u8; 32] {
    let facts: std::collections::BTreeMap<&String, _> = program.facts.iter().collect();
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-program/v3\0");
    hash.update(format!("{:?}\n{:?}\n{:?}\n{:?}\n{:?}", program.rules, program.decls, facts, program.inputs, program.outputs).as_bytes());
    // What the batch makes for the part besides its rules' reads: the regex spans and AST patterns it matches
    // (O-stored-patterns).
    hash.update(format!("\n{:?}\n{:?}", program.rx_spans, program.ast_patterns).as_bytes());
    hash.finalize().into()
}

/// `digest` with the run's options folded in: the key of a file-local part that reads them (`option_item`).
pub(super) fn with_options(digest: [u8; 32], options: &serde_json::Value) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-options/v1\0");
    hash.update(digest);
    hash.update(options.to_string().as_bytes());
    hash.finalize().into()
}

/// The memo is the tape cache's: a build without `tape-cache` has no store to keep it in, so the runner never reads or
/// writes it, and saying so here, where the compiler sees it, leaves the store's zstd frames unlinked.
pub(super) fn off() {
    if !cfg!(feature = "tape-cache") {
        unreachable!("the corpus memo is off in a build without `tape-cache`");
    }
}

/// The stored source of `text`, stored if it is not.
pub(super) fn source(store: &ContentStore, text: &str) -> StoreResult<SourceId> {
    off();
    let hash = HashKey::sha256(text.as_bytes());
    // Held (O95): a source the run finds is pinned with it, as one it stores is.
    match store.held_source_by_hash(hash)? {
        Some(id) => Ok(id),
        None => store.put_source_hashed(text.as_bytes(), hash, &[]),
    }
}

/// The newest corpus run of `root`: the relations its memo plan holds, by name, as recorded (`.decl` columns as
/// JSON), and its documents' frames, still compressed, by path. A frame's document key is `frame_key`, its entry
/// `decode_frame`.
pub(in crate::code_rule_dsl) fn read_run(store: &ContentStore, root: &str) -> StoreResult<Option<(BTreeMap<String, String>, Vec<FactsFrame>)>> {
    let Some((plan, sources, relations)) = store.corpus_run_of(root)? else { return Ok(None) };
    let sources: HashSet<i64> = sources.into_iter().collect();
    Ok(Some((relations.into_iter().collect(), store.facts_frames_of_plan(plan, SCHEMA, &sources)?)))
}

/// The document key of a frame's entry: its first eight bytes, decompressing no more.
pub(in crate::code_rule_dsl) fn frame_key(frame: &FactsFrame) -> StoreResult<i64> {
    Ok(i64::from_le_bytes(frame.head(8)?.try_into().expect("8 bytes")))
}

/// A frame's entry.
pub(in crate::code_rule_dsl) fn decode_frame(frame: &FactsFrame) -> StoreResult<Entry> {
    decode(&frame.bytes()?)
}

/// The memoised entry of the document at `path` (key `key`, content `source`) under `program`.
pub(super) fn lookup(store: &ContentStore, source: SourceId, path: &str, key: i64, program: &[u8; 32]) -> StoreResult<Option<Entry>> {
    off();
    let set = plan(program);
    let memo = store.facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path })?;
    match memo.results.get(&set.plans[0]) {
        Some(result) => Ok(Some(decode(&result.entry)?).filter(|entry| entry.key == key)),
        None => Ok(None),
    }
}

/// Memoises `entry` as the document's at `path`.
pub(super) fn remember(store: &ContentStore, source: SourceId, path: &str, key: i64, program: &[u8; 32], entry: &Entry) -> StoreResult<()> {
    off();
    let set = plan(program);
    let result = FactsResult { plan: set.plans[0], path_dependent: true, entry: encode(&Entry { key, ..entry.clone() }) };
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, result.entry.len() as u64);
    store.put_facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path }, &set, &[result], &[])?;
    Ok(())
}

/// Where a run's memo is: the tape cache's store (decision 0052), or a session's memory (`session::Session`), whose
/// rows are under the session store's atoms and live as long as the session.
#[derive(Clone, Copy)]
pub(super) enum Memo<'a> {
    Stored(&'a ContentStore),
    Held(&'a Held),
}

/// A session's memo: each entry under its content's digest, its path and the file-local program's digest.
pub(super) type Held = std::sync::Mutex<std::collections::HashMap<([u8; 32], String, [u8; 32]), Entry>>;

/// A file's content as the memo knows it: its stored source, or (held) its digest.
#[derive(Clone, Copy, Debug)]
pub(super) enum Content {
    Stored(SourceId),
    Held([u8; 32]),
}

impl Content {
    /// The stored source, which a run's manifest names (decision 0053).
    pub(super) fn stored(self) -> Option<SourceId> {
        match self {
            Content::Stored(id) => Some(id),
            Content::Held(_) => None,
        }
    }
}

/// The most entries a session holds: past it, the memo starts again empty (its entries are a cache).
const HELD_ENTRIES: usize = 4096;

impl Memo<'_> {
    /// `text` as the memo knows it.
    pub(super) fn content(self, text: &str) -> StoreResult<Content> {
        match self {
            Memo::Stored(store) => source(store, text).map(Content::Stored),
            Memo::Held(_) => Ok(Content::Held(Sha256::digest(text.as_bytes()).into())),
        }
    }

    pub(super) fn lookup(self, content: Content, path: &str, key: i64, program: &[u8; 32]) -> StoreResult<Option<Entry>> {
        match (self, content) {
            (Memo::Stored(store), Content::Stored(id)) => lookup(store, id, path, key, program),
            (Memo::Held(held), Content::Held(digest)) => {
                Ok(held.lock().expect("the session memo").get(&(digest, path.to_owned(), *program)).filter(|e| e.key == key).cloned())
            }
            _ => Ok(None),
        }
    }

    pub(super) fn remember(self, content: Content, path: &str, key: i64, program: &[u8; 32], entry: &Entry) -> StoreResult<()> {
        match (self, content) {
            (Memo::Stored(store), Content::Stored(id)) => remember(store, id, path, key, program, entry),
            (Memo::Held(held), Content::Held(digest)) => {
                let mut held = held.lock().expect("the session memo");
                if held.len() >= HELD_ENTRIES {
                    held.clear();
                }
                held.insert((digest, path.to_owned(), *program), Entry { key, ..entry.clone() });
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// The document a row's location value names: a node (`key << 32 | index`) or a document key.
pub(super) fn document_of(value: i64) -> i64 {
    if value >= 1 << 32 { value >> 32 } else { value }
}

fn encode(entry: &Entry) -> Vec<u8> {
    let mut out = Vec::new();
    let put = |out: &mut Vec<u8>, v: u64| out.extend_from_slice(&v.to_le_bytes());
    let text = |out: &mut Vec<u8>, s: &str| {
        out.extend_from_slice(&(s.len() as u64).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    };
    put(&mut out, entry.key as u64);
    put(&mut out, entry.nodes);
    put(&mut out, entry.rows.len() as u64);
    for (rel, (width, values)) in &entry.rows {
        text(&mut out, rel);
        put(&mut out, *width as u64);
        put(&mut out, values.len() as u64);
        for v in values {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    put(&mut out, entry.lines.len() as u64);
    for (rel, (columns, lines)) in &entry.lines {
        text(&mut out, rel);
        put(&mut out, columns.len() as u64);
        for c in columns {
            put(&mut out, *c as u64);
        }
        put(&mut out, lines.len() as u64);
        for line in lines {
            text(&mut out, line);
        }
    }
    out
}

fn decode(bytes: &[u8]) -> StoreResult<Entry> {
    struct Reader<'a> {
        bytes: &'a [u8],
        at: usize,
    }
    impl<'a> Reader<'a> {
        fn take(&mut self, n: usize) -> StoreResult<&'a [u8]> {
            let slice = self.bytes.get(self.at..self.at + n).ok_or_else(|| StoreError::Invalid("a corpus memo entry is truncated".into()))?;
            self.at += n;
            Ok(slice)
        }
        fn u64(&mut self) -> StoreResult<u64> {
            Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8 bytes")))
        }
        fn text(&mut self) -> StoreResult<String> {
            let n = self.u64()? as usize;
            String::from_utf8(self.take(n)?.to_vec()).map_err(|_| StoreError::Invalid("a corpus memo entry names a relation that is not UTF-8".into()))
        }
    }
    let mut r = Reader { bytes, at: 0 };
    let key = r.u64()? as i64;
    let mut entry = Entry { key, nodes: r.u64()?, ..Entry::default() };
    for _ in 0..r.u64()? {
        let rel = r.text()?;
        let width = r.u64()? as usize;
        let n = r.u64()? as usize;
        let mut values = Vec::with_capacity(n);
        for _ in 0..n {
            values.push(i64::from_le_bytes(r.take(8)?.try_into().expect("8 bytes")));
        }
        entry.rows.insert(rel, (width, values));
    }
    for _ in 0..r.u64()? {
        let rel = r.text()?;
        let columns = (0..r.u64()?).map(|_| r.u64().map(|c| c as usize)).collect::<StoreResult<Vec<_>>>()?;
        let lines = (0..r.u64()?).map(|_| r.text()).collect::<StoreResult<Vec<_>>>()?;
        entry.lines.insert(rel, (columns, lines));
    }
    if r.at != bytes.len() {
        return Err(StoreError::Invalid("a corpus memo entry has trailing bytes".into()));
    }
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_round_trips_and_keys_are_stable() {
        let mut entry = Entry { key: 5, nodes: 7, ..Entry::default() };
        entry.rows.insert("fl1_violation_at".into(), (3, vec![5, 1, 2, 5, 3, 4]));
        entry.lines.insert("violation".into(), (vec![1], vec!["r\t21474836480\tx".into()]));
        assert_eq!(decode(&encode(&entry)).unwrap(), entry);
        assert!(decode(&encode(&entry)[..10]).is_err());
        let paths = ["a.py".to_owned(), "b/c.py".to_owned()];
        let keys = doc_keys(&paths);
        assert_eq!(keys, doc_keys(&paths));
        assert_eq!(doc_keys(&paths[1..]), keys[1..], "a key does not depend on the other paths");
        assert!(keys.iter().all(|&k| (1..1 << 30).contains(&k)));
        assert_eq!(document_of(keys[0] << 32 | 9), keys[0]);
        assert_eq!(document_of(keys[1]), keys[1]);
    }
}
