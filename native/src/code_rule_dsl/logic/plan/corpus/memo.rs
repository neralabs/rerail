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

/// Which column of a base relation's row (in `input_columns` order) names the file the row is kept for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Owner {
    /// A node or a document key: the row is its document's (`document_of`).
    Node(usize),
    /// A string: the row is each file's whose text holds it, as a text test's row is (`strings::TEXT_COLUMNS`).
    Text(usize),
}

/// The owner column of `rel`'s rows. Most base relations lead with their node or document; a pattern's spans and
/// matches lead with the pattern (`rx_span(p, c, …)`, `ast_match(p, n, …)`), so their node is the second column;
/// `value_type(v, t)` is keyed by the value itself, a string some file's `value(n, v)` holds.
pub(super) fn owner(rel: &str) -> Owner {
    match rel {
        "rx_span" | "ast_match" => Owner::Node(1),
        "value_type" => Owner::Text(0),
        _ if rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) => Owner::Text(0),
        _ => Owner::Node(0),
    }
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
pub(in crate::code_rule_dsl) fn program_digest(program: &crate::code_rule_dsl::logic::front::Program) -> [u8; 32] {
    let facts: std::collections::BTreeMap<&String, _> = program.facts.iter().collect();
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-program/v3\0");
    hash.update(format!("{:?}\n{:?}\n{:?}\n{:?}\n{:?}", program.rules, program.decls, facts, program.inputs, program.outputs).as_bytes());
    // What the batch makes for the part besides its rules' reads: the regex spans and AST patterns it matches
    // (O-stored-patterns).
    hash.update(format!("\n{:?}\n{:?}", program.rx_spans, program.ast_patterns).as_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    // `RERAIL_MEMO_PROGRAM_DUMP=dir`: what the digest is of, a file a digest (to find what moved a memo's key).
    if let Some(dir) = std::env::var_os("RERAIL_MEMO_PROGRAM_DUMP") {
        let name: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
        let text = format!(
            "rules {:#?}\ndecls {:#?}\nfacts {:#?}\ninputs {:#?}\noutputs {:#?}\nrx_spans {:#?}\nast_patterns {:#?}\n",
            program.rules, program.decls, facts, program.inputs, program.outputs, program.rx_spans, program.ast_patterns
        );
        let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.txt")), text);
    }
    digest
}

/// `digest` with the run's options folded in: the key of a file-local part that reads them (`option_item`).
pub(super) fn with_options(digest: [u8; 32], options: &serde_json::Value) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-options/v1\0");
    hash.update(digest);
    hash.update(options.to_string().as_bytes());
    hash.finalize().into()
}

/// `digest` with rows folded in, by relation: the key of a file-local part that reads relations whose rows are not in
/// its text (a phase's exports). The rows' order is not part of it.
pub(super) fn with_rows<'a>(digest: [u8; 32], rels: impl Iterator<Item = (&'a str, &'a Vec<Vec<crate::code_rule_dsl::logic::front::Value>>)>) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-rows/v1\0");
    hash.update(digest);
    for (rel, rows) in rels {
        let mut lines: Vec<String> = rows.iter().map(|r| format!("{r:?}")).collect();
        lines.sort_unstable();
        hash.update(rel.as_bytes());
        hash.update([0]);
        for l in &lines {
            hash.update(l.as_bytes());
            hash.update([0]);
        }
    }
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
    // An entry still queued is read from memory (`write_back`): this process's writes are its reads.
    if let Some(entry) = store.pending::<Entry>(&queued(source, path, program)) {
        return Ok(Some((*entry).clone()).filter(|entry| entry.key == key));
    }
    let set = plan(program);
    // This program's row only: the file's other programs' rows are not decoded.
    match store.facts_result(FactsMemoKey { source, fact_schema: SCHEMA, path }, set.plans[0])? {
        Some(result) => Ok(Some(decode(&result.entry)?).filter(|entry| entry.key == key)),
        None => Ok(None),
    }
}

/// `lookup` of many documents, each `(source, path, key)`, in their order: one statement whatever their number.
pub(super) fn lookup_all(store: &ContentStore, program: &[u8; 32], docs: &[(SourceId, &str, i64)]) -> StoreResult<Vec<Option<Entry>>> {
    off();
    let set = plan(program);
    // Entries still queued from memory; the others in one statement.
    let held: Vec<Option<std::sync::Arc<Entry>>> = docs.iter().map(|(s, p, _)| store.pending::<Entry>(&queued(*s, p, program))).collect();
    let keys: Vec<(SourceId, &str)> = docs.iter().zip(&held).filter(|(_, h)| h.is_none()).map(|((s, p, _), _)| (*s, *p)).collect();
    let mut found = store.facts_results_for(SCHEMA, set.plans[0], &keys)?.into_iter();
    docs.iter()
        .zip(held)
        .map(|((_, _, key), held)| {
            Ok(match held {
                Some(e) => Some((*e).clone()).filter(|e| e.key == *key),
                None => match found.next().flatten() {
                    Some(b) => Some(decode(&b)?).filter(|e| e.key == *key),
                    None => None,
                },
            })
        })
        .collect()
}

/// A memo entry found before it is decoded: held in memory (a queued write's, or a session's), or its stored frame.
/// A run finds its documents' entries before its first wave and decodes each in its wave (decision 0104).
pub(super) enum Found {
    Held(std::sync::Arc<Entry>),
    Frame(FactsFrame),
}

impl Found {
    /// The entry, when it is the document's (`key`).
    pub(super) fn entry(&self, key: i64) -> StoreResult<Option<Entry>> {
        Ok(match self {
            Found::Held(e) => Some((**e).clone()),
            Found::Frame(f) => Some(decode(&f.bytes()?)?),
        }
        .filter(|e| e.key == key))
    }
}

/// `lookup_all`, each entry found and not decoded: one statement whatever the documents' number.
pub(super) fn found_all(store: &ContentStore, program: &[u8; 32], docs: &[(SourceId, &str)]) -> StoreResult<Vec<Option<Found>>> {
    off();
    let set = plan(program);
    let held: Vec<Option<std::sync::Arc<Entry>>> = docs.iter().map(|(s, p)| store.pending::<Entry>(&queued(*s, p, program))).collect();
    let keys: Vec<(SourceId, &str)> = docs.iter().zip(&held).filter(|(_, h)| h.is_none()).map(|((s, p), _)| (*s, *p)).collect();
    let mut frames = store.facts_frames_for(SCHEMA, set.plans[0], &keys)?.into_iter();
    Ok(held.into_iter().map(|h| match h {
        Some(e) => Some(Found::Held(e)),
        None => frames.next().flatten().map(Found::Frame),
    }).collect())
}

/// What a memo entry's write is queued under (`write_back`): the row's source, path and program.
fn queued(source: SourceId, path: &str, program: &[u8; 32]) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"facts_memo/corpus", &source.raw().to_le_bytes(), path.as_bytes(), program])
}

/// `queued`, for a content the store does not hold yet: by its digest.
fn queued_unstored(digest: &[u8; 32], path: &str, program: &[u8; 32]) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"facts_memo/corpus/unstored", digest, path.as_bytes(), program])
}

/// What an entry is queued under, by its content.
fn queued_content(content: Content, path: &str, program: &[u8; 32]) -> Option<crate::content_store::write_back::Key> {
    match content {
        Content::Stored(id) => Some(queued(id, path, program)),
        Content::Unstored(d) => Some(queued_unstored(&d, path, program)),
        Content::Held(_) => None,
    }
}

/// `source` of many texts, in their order: the stored ones found (and pinned) in one statement, the others stored.
pub(super) fn sources(store: &ContentStore, texts: &[&str]) -> StoreResult<Vec<SourceId>> {
    off();
    let hashes: Vec<HashKey> = texts.iter().map(|t| HashKey::sha256(t.as_bytes())).collect();
    let found = store.held_sources_by_hash(&hashes)?;
    texts.iter().zip(hashes).zip(found).map(|((text, hash), id)| match id { Some(id) => Ok(id), None => store.put_source_hashed(text.as_bytes(), hash, &[]) }).collect()
}

/// The stored sources of many contents, by their SHA-256 digests, in their order: one read and nothing written (a
/// lookup writes nothing, decision 0104). The sources found are pinned under the store's holder by its writer.
pub(super) fn found_sources(store: &ContentStore, digests: &[[u8; 32]]) -> StoreResult<Vec<Option<SourceId>>> {
    off();
    let hashes: Vec<HashKey> = digests.iter().map(|d| HashKey::from_sha256_digest(*d)).collect();
    let found = store.sources_by_hashes(&hashes)?;
    let pins: Vec<SourceId> = found.iter().flatten().copied().collect();
    if !pins.is_empty() {
        store.write_back("the pins of the sources the run read", pins.len() * 8, Vec::new(), Box::new(move |store| store.pin_held_sources(&pins)));
    }
    Ok(found)
}

/// Memoises many documents' entries at once, each `(source, path, key, entry)` under `program`: one transaction of
/// a few statements (`put_facts_results`), not one per document. Queued behind the run (decision 0102): encoded and
/// written by the store's writer, each entry read from memory until it is written.
/// A content the store does not hold yet is written under the source its tape's write stored, queued before.
pub(super) fn remember_all(store: &ContentStore, program: &[u8; 32], entries: Vec<(Content, String, i64, Entry)>) {
    off();
    let set = plan(program);
    if crate::code_rule_dsl::logic::profile::on() {
        tally(entries.iter().map(|e| &e.3));
    }
    let held: Vec<(Content, String, std::sync::Arc<Entry>)> = entries.into_iter().filter(|e| e.0.in_store()).map(|(content, path, key, entry)| (content, path, std::sync::Arc::new(Entry { key, ..entry }))).collect();
    let bytes: usize = held.iter().map(|(_, _, e)| e.bytes()).sum();
    let keys = held.iter().filter_map(|(content, path, e)| Some((queued_content(*content, path, program)?, Some(std::sync::Arc::clone(e) as std::sync::Arc<dyn std::any::Any + Send + Sync>)))).collect();
    store.write_back(
        "the memo's entries",
        bytes,
        keys,
        Box::new(move |store| {
            let mut results: Vec<(SourceId, &str, FactsResult)> = Vec::with_capacity(held.len());
            for (content, path, e) in &held {
                // A source not stored (its tape's write failed) leaves the entry unwritten: the next run misses it.
                let Some(source) = content.resolve(store)? else { continue };
                results.push((source, path.as_str(), FactsResult { plan: set.plans[0], path_dependent: true, entry: encode(e) }));
            }
            store.put_facts_results(SCHEMA, &set, &results).map(drop)
        }),
    );
}

impl Entry {
    /// The bytes the entry holds (its rows and lines), about.
    fn bytes(&self) -> usize {
        self.rows.iter().map(|(r, (_, v))| r.len() + v.len() * 8).sum::<usize>() + self.lines.iter().map(|(r, (c, l))| r.len() + c.len() * 8 + l.iter().map(|t| t.len() + 24).sum::<usize>()).sum::<usize>()
    }
}

/// What the run's memo entries held, by relation (rows, values), and the documents' nodes: counted when profiling,
/// so the profile names what makes the memo large (t31 A2). Read and cleared by `sizes`.
static SIZES: std::sync::Mutex<(u64, BTreeMap<String, (u64, u64)>)> = std::sync::Mutex::new((0, BTreeMap::new()));

fn tally<'a>(entries: impl Iterator<Item = &'a Entry>) {
    let mut sizes = SIZES.lock().expect("the memo's sizes");
    for entry in entries {
        sizes.0 += entry.nodes;
        for (rel, (width, values)) in &entry.rows {
            let at = sizes.1.entry(rel.clone()).or_default();
            at.0 += (values.len() / (*width).max(1)) as u64;
            at.1 += values.len() as u64;
        }
    }
}

/// The nodes of the documents remembered, and each relation's (rows, values) over their entries, since the last call.
pub(super) fn sizes() -> (u64, BTreeMap<String, (u64, u64)>) {
    std::mem::take(&mut *SIZES.lock().expect("the memo's sizes"))
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

/// A file's content as the memo knows it: its stored source; a content the store does not hold yet, by its digest
/// (its source is written by the store's writer, behind the run, with its tape: decision 0104); or (held) its digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Content {
    Stored(SourceId),
    Unstored([u8; 32]),
    Held([u8; 32]),
}

impl Content {
    /// The stored source, which a run's manifest names (decision 0053).
    pub(super) fn stored(self) -> Option<SourceId> {
        match self {
            Content::Stored(id) => Some(id),
            Content::Unstored(_) | Content::Held(_) => None,
        }
    }

    /// Whether the store holds the content or will (its source is queued): a stored memo's content.
    fn in_store(self) -> bool {
        matches!(self, Content::Stored(_) | Content::Unstored(_))
    }

    /// The source on the store's writer, where a content queued before is written by now.
    pub(super) fn resolve(self, store: &ContentStore) -> StoreResult<Option<SourceId>> {
        match self {
            Content::Stored(id) => Ok(Some(id)),
            Content::Unstored(d) => store.source_by_hash(HashKey::from_sha256_digest(d)),
            Content::Held(_) => Ok(None),
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

    /// `content` of many texts, in their order (`sources`).
    pub(super) fn contents(self, texts: &[&str]) -> StoreResult<Vec<Content>> {
        match self {
            Memo::Stored(store) => Ok(sources(store, texts)?.into_iter().map(Content::Stored).collect()),
            Memo::Held(_) => Ok(texts.iter().map(|t| Content::Held(Sha256::digest(t.as_bytes()).into())).collect()),
        }
    }

    /// The contents of many texts as the memo knows them, by their SHA-256 digests, in their order: `None` for a
    /// text the store does not hold yet (its source is stored with its tape). Nothing is written (`found_sources`).
    pub(super) fn found(self, digests: &[[u8; 32]]) -> StoreResult<Vec<Option<Content>>> {
        match self {
            Memo::Stored(store) => Ok(found_sources(store, digests)?.into_iter().map(|id| id.map(Content::Stored)).collect()),
            Memo::Held(_) => Ok(digests.iter().map(|d| Some(Content::Held(*d))).collect()),
        }
    }

    /// `lookup` of many documents, each `(content, path, key)`, in their order: stored ones in one statement.
    pub(super) fn lookup_all(self, program: &[u8; 32], docs: &[(Content, &str, i64)]) -> StoreResult<Vec<Option<Entry>>> {
        match self {
            Memo::Stored(store) => {
                let mut out: Vec<Option<Entry>> = vec![None; docs.len()];
                let (at, stored): (Vec<usize>, Vec<(SourceId, &str, i64)>) = docs.iter().enumerate().filter_map(|(i, (c, p, k))| c.stored().map(|id| (i, (id, *p, *k)))).unzip();
                for (i, e) in at.into_iter().zip(lookup_all(store, program, &stored)?) {
                    out[i] = e;
                }
                Ok(out)
            }
            Memo::Held(_) => docs.iter().map(|(c, p, k)| self.lookup(*c, p, *k, program)).collect(),
        }
    }

    /// The entries of many documents, each `(content, path)`, found and not decoded (`Found`): stored ones in one
    /// statement, held ones from memory.
    pub(super) fn found_all(self, program: &[u8; 32], docs: &[(Content, &str)]) -> StoreResult<Vec<Option<Found>>> {
        match self {
            Memo::Stored(store) => {
                let mut out: Vec<Option<Found>> = (0..docs.len()).map(|_| None).collect();
                let (at, stored): (Vec<usize>, Vec<(SourceId, &str)>) = docs.iter().enumerate().filter_map(|(i, (c, p))| c.stored().map(|id| (i, (id, *p)))).unzip();
                for (i, e) in at.into_iter().zip(found_all(store, program, &stored)?) {
                    out[i] = e;
                }
                // A content not stored yet has no stored entry: only one this process queued.
                for (i, (c, p)) in docs.iter().enumerate() {
                    if let Content::Unstored(d) = c {
                        out[i] = store.pending::<Entry>(&queued_unstored(d, p, program)).map(Found::Held);
                    }
                }
                Ok(out)
            }
            Memo::Held(held) => {
                let held = held.lock().expect("the session memo");
                Ok(docs
                    .iter()
                    .map(|(c, p)| match c {
                        Content::Held(digest) => held.get(&(*digest, (*p).to_owned(), *program)).map(|e| Found::Held(std::sync::Arc::new(e.clone()))),
                        Content::Stored(_) | Content::Unstored(_) => None,
                    })
                    .collect())
            }
        }
    }

    pub(super) fn lookup(self, content: Content, path: &str, key: i64, program: &[u8; 32]) -> StoreResult<Option<Entry>> {
        match (self, content) {
            (Memo::Stored(store), Content::Stored(id)) => lookup(store, id, path, key, program),
            (Memo::Stored(store), Content::Unstored(d)) => Ok(store.pending::<Entry>(&queued_unstored(&d, path, program)).map(|e| (*e).clone()).filter(|e| e.key == key)),
            (Memo::Held(held), Content::Held(digest)) => {
                Ok(held.lock().expect("the session memo").get(&(digest, path.to_owned(), *program)).filter(|e| e.key == key).cloned())
            }
            _ => Ok(None),
        }
    }

    /// `remember` of many documents, stored ones written as one wave (`remember_all`), behind the run.
    pub(super) fn remember_all(self, program: &[u8; 32], entries: Vec<(Content, String, i64, Entry)>) -> StoreResult<()> {
        match self {
            Memo::Stored(store) => {
                remember_all(store, program, entries);
                Ok(())
            }
            Memo::Held(_) => {
                for (content, path, key, entry) in &entries {
                    self.remember(*content, path, *key, program, entry)?;
                }
                Ok(())
            }
        }
    }

    pub(super) fn remember(self, content: Content, path: &str, key: i64, program: &[u8; 32], entry: &Entry) -> StoreResult<()> {
        match (self, content) {
            (Memo::Stored(store), c @ (Content::Stored(_) | Content::Unstored(_))) => {
                remember_all(store, program, vec![(c, path.to_owned(), key, entry.clone())]);
                Ok(())
            }
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
