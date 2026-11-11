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

use crate::content_store::{ContentStore, HashKey, SourceId, StoreError, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, HashSet};

/// The memo's fact schema: bumped when an entry's meaning changes.
/// When it changes, the old number joins `RETIRED_CORPUS_MEMO_SCHEMAS` only if its rows could outlive their plans
/// (they cannot since decision 0053: the plan names the engine build, and dead plans are collected).
const SCHEMA: u32 = 7;
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
    hash.update(b"rerail.corpus-local/v3\0");
    hash.update(env!("RERAIL_ENGINE_BUILD_ID").as_bytes());
    hash.update(crate::code_rule_dsl::workspace::grammar_version().to_le_bytes());
    // A memoised row's node ids are those of the scheme it was made under: preorder, or per definition.
    if crate::code_rule_dsl::facts::vtab::syntax::definition_ids() {
        hash.update(b"node-ids:definition\0");
    }
    hash.update(program);
    hash.finalize().into()
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

/// A local memo's stored statistics are a projection demanded by its consumers.
/// Include the projection in the key so a wider consumer cannot reuse a narrower frame.
pub(super) fn with_tree_demand(digest: [u8; 32], tree: bool) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-tree-demand/v1\0");
    hash.update(digest);
    hash.update([u8::from(tree)]);
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

/// The source of `text` when the store holds it: one read, and its pin (O95) queued on the store's writer (decision
/// 0104). `None`: not stored yet; `source`, run on the writer, stores it.
pub(super) fn stored_source(store: &ContentStore, text: &str) -> StoreResult<Option<SourceId>> {
    off();
    let found = store.sources_by_hashes(&[HashKey::sha256(text.as_bytes())])?.pop().flatten();
    if let Some(id) = found {
        store.write_back("a source's pin", 0, Vec::new(), Box::new(move |s| s.pin_held_sources(&[id])));
    }
    Ok(found)
}

/// The source of `text` for the run's manifest (`late`, which the manifest reads as the writer runs it), all of it
/// on the store's writer, behind the run (decision 0104): the source the store holds then, pinned, or, where `store`,
/// one it stores. Queued after a write that stores it (FIFO), it finds that one.
pub(super) fn named_source(cache: &ContentStore, text: String, store: bool, late: &std::sync::Arc<std::sync::Mutex<Vec<SourceId>>>) {
    let late = std::sync::Arc::clone(late);
    cache.write_back(
        "a source the manifest names",
        text.len(),
        Vec::new(),
        Box::new(move |s| {
            let hash = HashKey::sha256(text.as_bytes());
            let found = match s.held_source_by_hash(hash)? {
                Some(id) => Some(id),
                None if store => Some(s.put_source_hashed(text.as_bytes(), hash, &[])?),
                None => None,
            };
            late.lock().expect("the late sources").extend(found);
            Ok(())
        }),
    );
}

/// How many runs of a plan may leave an entry unread before it is forgotten (decision 0108 §4): the plan's runs are
/// every root's, so worktrees that take turns keep theirs.
const MEMO_RUNS: i64 = 32;

/// The memo's relation in its plan's space: one frame per entry (decision 0108).
fn entry_rel(plan: &[u8; 32]) -> crate::content_store::FrameRel {
    crate::content_store::FrameRel { name: "local".to_owned(), ident: *plan }
}

/// The part an entry of content `digest` at `path` is kept under (0103 §8's `local_key(d, h, p)`, its `p` the space).
fn part_of(digest: &[u8; 32], path: &str) -> i64 {
    let mut hash = Sha256::new();
    hash.update(b"rerail.local-key/v1\0");
    hash.update(digest);
    hash.update(path.as_bytes());
    let h: [u8; 32] = hash.finalize().into();
    i64::from_le_bytes(h[..8].try_into().expect("8 bytes")) & i64::MAX
}

/// What an entry's frame says it was derived from (its `reads`): its path, with its document key, and its content.
fn frame_reads(path: &str, key: i64, digest: &[u8; 32]) -> Vec<(String, u64)> {
    let mut reads = vec![(path.to_owned(), key as u64)];
    reads.extend(digest.chunks_exact(8).map(|c| ("sha256".to_owned(), u64::from_le_bytes(c.try_into().expect("8 bytes")))));
    reads
}

/// A memo entry as its stored frame holds it, not decoded: the document's path and key, and its content's digest.
pub(in crate::code_rule_dsl) struct Frame {
    pub path: String,
    key: i64,
    digest: [u8; 32],
    frame: std::sync::Arc<crate::content_store::AdmittedFrame>,
}

impl Frame {
    fn of(frame: std::sync::Arc<crate::content_store::AdmittedFrame>) -> Option<Frame> {
        let reads = &frame.stored.reads;
        let (path, key) = reads.first()?.clone();
        if reads.len() != 5 || reads[1..].iter().any(|(n, _)| n != "sha256") {
            return None;
        }
        let mut digest = [0u8; 32];
        for (c, (_, v)) in digest.chunks_exact_mut(8).zip(&reads[1..]) {
            c.copy_from_slice(&v.to_le_bytes());
        }
        Some(Frame { path, key: key as i64, digest, frame })
    }

    fn entry(&self) -> StoreResult<Entry> {
        self.projected(&|_| true)
    }

    fn projected(&self, keep: &dyn Fn(&str) -> bool) -> StoreResult<Entry> {
        let mut entry = from_rows_projected(&self.frame.rows()?, keep)?;
        entry.key = self.key;
        Ok(entry)
    }
}

/// The newest corpus run of `root`: the relations its memo plan holds, by name, as recorded (`.decl` columns as
/// JSON), and its documents' frames, not decoded, by path. A frame's document key is `frame_key`, its entry
/// `decode_frame`.
pub(in crate::code_rule_dsl) fn read_run(store: &ContentStore, root: &str) -> StoreResult<Option<(BTreeMap<String, String>, Vec<Frame>)>> {
    let Some((plan, sources, relations)) = store.corpus_run_of(root)? else { return Ok(None) };
    let ids: Vec<SourceId> = sources.into_iter().map(SourceId::from_raw).collect();
    let contents: HashSet<[u8; 32]> = store.source_digests(&ids)?.into_iter().flatten().collect();
    let mut frames: Vec<Frame> = store
        .admit_all_parts(&crate::content_store::local_space(&plan), &entry_rel(&plan))?
        .into_iter()
        .filter_map(|(_, f)| Frame::of(f))
        .filter(|f| contents.contains(&f.digest))
        .collect();
    frames.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Some((relations.into_iter().collect(), frames)))
}

/// The document key of a frame's entry.
pub(in crate::code_rule_dsl) fn frame_key(frame: &Frame) -> StoreResult<i64> {
    Ok(frame.key)
}

/// A frame's entry.
pub(in crate::code_rule_dsl) fn decode_frame(frame: &Frame) -> StoreResult<Entry> {
    frame.entry()
}

/// The memoised entry of the document at `path` (key `key`, content `source`) under `program`.
pub(super) fn lookup(store: &ContentStore, source: SourceId, path: &str, key: i64, program: &[u8; 32]) -> StoreResult<Option<Entry>> {
    off();
    match found_all(store, program, &[(source, path)])?.pop().flatten() {
        Some(found) => found.entry(key),
        None => Ok(None),
    }
}

/// `lookup` of many documents, each `(source, path, key)`, in their order: one statement whatever their number.
pub(super) fn lookup_all(store: &ContentStore, program: &[u8; 32], docs: &[(SourceId, &str, i64)]) -> StoreResult<Vec<Option<Entry>>> {
    off();
    let keys: Vec<(SourceId, &str)> = docs.iter().map(|(s, p, _)| (*s, *p)).collect();
    found_all(store, program, &keys)?.into_iter().zip(docs).map(|(f, (_, _, key))| f.map_or(Ok(None), |f| f.entry(*key))).collect()
}

/// A memo entry found before it is decoded: held in memory (a queued write's, or a session's), or its stored frame.
/// A run finds its documents' entries before its first wave and decodes each in its wave (decision 0104).
pub(super) enum Found {
    Held(std::sync::Arc<Entry>),
    Frame(Frame),
}

impl Found {
    /// The entry, when it is the document's (`key`).
    pub(super) fn entry(&self, key: i64) -> StoreResult<Option<Entry>> {
        self.projected(key, &|_| true)
    }

    /// Decode only demanded relations. Skipped relations' rows are checked, not kept.
    pub(super) fn projected(&self, key: i64, keep: &dyn Fn(&str) -> bool) -> StoreResult<Option<Entry>> {
        let entry = match self {
            Found::Held(e) => Entry { key: e.key, nodes: e.nodes,
                rows: e.rows.iter().filter(|(r, _)| keep(r)).map(|(r, v)| (r.clone(), v.clone())).collect(),
                lines: e.lines.clone() },
            Found::Frame(f) => f.projected(keep)?,
        };
        Ok(Some(entry).filter(|e| e.key == key))
    }

}

/// `lookup_all`, each entry found and not decoded: the contents' digests in one read, their frames in one more. The
/// frames found are marked as used by this run, behind it (a lookup writes nothing itself: decision 0104).
pub(super) fn found_all(store: &ContentStore, program: &[u8; 32], docs: &[(SourceId, &str)]) -> StoreResult<Vec<Option<Found>>> {
    off();
    let plan = plan_digest(program);
    let held: Vec<Option<std::sync::Arc<Entry>>> = docs.iter().map(|(s, p)| store.pending::<Entry>(&queued(*s, p, program))).collect();
    let asked: Vec<usize> = (0..docs.len()).filter(|i| held[*i].is_none()).collect();
    let digests = store.source_digests(&asked.iter().map(|i| docs[*i].0).collect::<Vec<_>>())?;
    let wanted: Vec<(usize, [u8; 32], i64)> = asked.iter().zip(&digests).filter_map(|(i, d)| d.map(|d| (*i, d, part_of(&d, docs[*i].1)))).collect();
    let (space, rel) = (crate::content_store::local_space(&plan), entry_rel(&plan));
    let mut frames = store.admit_parts_of(&space, &rel, &wanted.iter().map(|(_, _, p)| *p).collect::<Vec<_>>())?;
    let mut out: Vec<Option<Found>> = held.into_iter().map(|h| h.map(Found::Held)).collect();
    let mut used = Vec::new();
    for (i, digest, part) in wanted {
        let Some(frame) = frames.remove(&part).and_then(Frame::of) else { continue };
        // A part another content or path collides into is a miss.
        if frame.digest == digest && frame.path == docs[i].1 {
            used.push(part);
            out[i] = Some(Found::Frame(frame));
        }
    }
    if !used.is_empty() {
        store.queue_frame_parts_used(space, rel, used, super::run_token(), MEMO_RUNS);
    }
    Ok(out)
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

/// Memoises many documents' entries at once, each `(content, path, key, entry)` under `program`: their frames in
/// one transaction (decision 0108). Queued behind the run (decision 0102): encoded and written by the store's
/// writer, each entry read from memory until it is written. A content is keyed by its digest, so one the store
/// does not hold yet is written with the others.
pub(super) fn remember_all(store: &ContentStore, program: &[u8; 32], entries: Vec<(Content, String, i64, Entry)>) {
    off();
    let plan = plan_digest(program);
    if crate::code_rule_dsl::logic::profile::on() {
        tally(entries.iter().map(|e| &e.3));
    }
    let held: Vec<(Content, String, std::sync::Arc<Entry>)> = entries.into_iter().filter(|e| e.0.in_store()).map(|(content, path, key, entry)| (content, path, std::sync::Arc::new(Entry { key, ..entry }))).collect();
    let bytes: usize = held.iter().map(|(_, _, e)| e.bytes()).sum();
    let keys = held.iter().filter_map(|(content, path, e)| Some((queued_content(*content, path, program)?, Some(std::sync::Arc::clone(e) as std::sync::Arc<dyn std::any::Any + Send + Sync>)))).collect();
    let token = super::run_token();
    store.write_back(
        "the memo's entries",
        bytes,
        keys,
        Box::new(move |store| {
            let ids: Vec<SourceId> = held.iter().filter_map(|(c, _, _)| match c { Content::Stored(id) => Some(*id), _ => None }).collect();
            let mut stored = store.source_digests(&ids)?.into_iter();
            let mut puts = Vec::with_capacity(held.len());
            for (content, path, e) in &held {
                let digest = match content {
                    Content::Stored(_) => stored.next().flatten(),
                    Content::Unstored(d) => Some(*d),
                    Content::Held(_) => None,
                };
                // A source collected since the run found it leaves the entry unwritten: the next run misses it.
                let Some(digest) = digest else { continue };
                let part = part_of(&digest, path);
                puts.push(crate::content_store::FramePut { rel: entry_rel(&plan), part, digest: e.key as u64, reads: frame_reads(path, e.key, &digest), rows: to_rows(e) });
            }
            let space = crate::content_store::local_space(&plan);
            store.keep_frames(&space, &puts, &[], None)?;
            let parts: Vec<i64> = puts.iter().map(|p| p.part).collect();
            store.use_frame_parts(&space, &entry_rel(&plan), &parts, token, MEMO_RUNS)
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

// An entry's frame (decision 0108): typed rows of integers, each row's first value its kind, its second the
// relation or output it is of. A relation's rows are rows of it, a value to a column; text (names, lines) is packed
// eight bytes to a value.
const NODES: i64 = 0;
const REL: i64 = 2;
const REL_NAME: i64 = 3;
const REL_ROW: i64 = 4;
const OUT: i64 = 5;
const OUT_NAME: i64 = 6;
const OUT_COLUMN: i64 = 7;
const LINE: i64 = 8;
const LINE_TEXT: i64 = 9;

/// The entry's rows; the frame's width: two, and the widest relation's, at least eight.
fn to_rows(entry: &Entry) -> crate::content_store::FrameRows {
    let width = 2 + entry.rows.values().map(|(w, _)| *w).max().unwrap_or(0).max(8);
    let mut rows: Vec<i64> = Vec::new();
    let mut n = 0;
    let mut row = |rows: &mut Vec<i64>, values: &[i64]| {
        rows.extend_from_slice(values);
        rows.extend(std::iter::repeat_n(0, width - values.len()));
        n += 1;
    };
    // Text after a row that gives its length: `width - 3` values a row, after the kind, the owner and a sequence.
    let text = |rows: &mut Vec<i64>, row: &mut dyn FnMut(&mut Vec<i64>, &[i64]), kind: i64, owner: i64, at: i64, s: &str| {
        for chunk in s.as_bytes().chunks((width - 3) * 8) {
            let mut values = vec![kind, owner, at];
            values.extend(chunk.chunks(8).map(|b| {
                let mut word = [0u8; 8];
                word[..b.len()].copy_from_slice(b);
                i64::from_le_bytes(word)
            }));
            row(rows, &values);
        }
    };
    row(&mut rows, &[NODES, 0, entry.nodes as i64]);
    for (r, (name, (w, values))) in entry.rows.iter().enumerate() {
        let r = r as i64;
        let count = if *w == 0 { 0 } else { values.len() / w };
        row(&mut rows, &[REL, r, *w as i64, count as i64, name.len() as i64]);
        text(&mut rows, &mut row, REL_NAME, r, 0, name);
        for v in values.chunks(*w.max(&1)) {
            let mut values = vec![REL_ROW, r];
            values.extend_from_slice(v);
            row(&mut rows, &values);
        }
    }
    for (o, (name, (columns, lines))) in entry.lines.iter().enumerate() {
        let o = o as i64;
        row(&mut rows, &[OUT, o, columns.len() as i64, lines.len() as i64, name.len() as i64]);
        text(&mut rows, &mut row, OUT_NAME, o, 0, name);
        for c in columns {
            row(&mut rows, &[OUT_COLUMN, o, *c as i64]);
        }
        for (i, line) in lines.iter().enumerate() {
            row(&mut rows, &[LINE, o, i as i64, line.len() as i64]);
            text(&mut rows, &mut row, LINE_TEXT, o, i as i64, line);
        }
    }
    crate::content_store::FrameRows { types: vec![b'i'; width], rows, n }
}

/// The entry `to_rows` made (its key is the frame's).
fn from_rows(frame: &crate::content_store::FrameRows) -> StoreResult<Entry> {
    from_rows_projected(frame, &|_| true)
}

/// `from_rows` keeping only the relations `keep` takes: the others' rows are checked, not kept.
fn from_rows_projected(frame: &crate::content_store::FrameRows, keep: &dyn Fn(&str) -> bool) -> StoreResult<Entry> {
    let bad = |what: &str| StoreError::Invalid(format!("a corpus memo entry's frame {what}"));
    let width = frame.types.len();
    if width < 3 || frame.rows.len() != width * frame.n {
        return Err(bad("has no rows of its width"));
    }
    let mut entry = Entry::default();
    let mut rels: Vec<(String, usize, usize, Vec<u8>, Vec<i64>)> = Vec::new();
    let mut outs: Vec<(Vec<u8>, usize, Vec<usize>, Vec<(usize, Vec<u8>)>)> = Vec::new();
    let bytes = |values: &[i64]| values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    for row in frame.rows.chunks_exact(width) {
        let owner = row[1] as usize;
        match row[0] {
            NODES => entry.nodes = row[2] as u64,
            REL => {
                if owner != rels.len() {
                    return Err(bad("names a relation out of order"));
                }
                rels.push((String::new(), row[2] as usize, row[4] as usize, Vec::new(), Vec::new()));
            }
            REL_NAME => rels.get_mut(owner).ok_or_else(|| bad("names no relation"))?.3.extend(bytes(&row[3..])),
            REL_ROW => {
                let r = rels.get_mut(owner).ok_or_else(|| bad("has a row of no relation"))?;
                let w = r.1;
                r.4.extend_from_slice(row.get(2..2 + w).ok_or_else(|| bad("has a row wider than itself"))?);
            }
            OUT => {
                if owner != outs.len() {
                    return Err(bad("names an output out of order"));
                }
                outs.push((Vec::new(), row[4] as usize, Vec::new(), Vec::new()));
            }
            OUT_NAME => outs.get_mut(owner).ok_or_else(|| bad("names no output"))?.0.extend(bytes(&row[3..])),
            OUT_COLUMN => outs.get_mut(owner).ok_or_else(|| bad("has a column of no output"))?.2.push(row[2] as usize),
            LINE => outs.get_mut(owner).ok_or_else(|| bad("has a line of no output"))?.3.push((row[3] as usize, Vec::new())),
            LINE_TEXT => {
                let o = outs.get_mut(owner).ok_or_else(|| bad("has a line of no output"))?;
                o.3.last_mut().ok_or_else(|| bad("has text of no line"))?.1.extend(bytes(&row[3..]));
            }
            _ => return Err(bad("has a row of no kind")),
        }
    }
    let text = |mut b: Vec<u8>, len: usize| -> StoreResult<String> {
        if b.len() < len {
            return Err(bad("has text shorter than its length"));
        }
        b.truncate(len);
        String::from_utf8(b).map_err(|_| bad("has text that is not UTF-8"))
    };
    for (_, w, len, name, values) in rels {
        let name = text(name, len)?;
        if keep(&name) {
            entry.rows.insert(name, (w, values));
        }
    }
    for (name, len, columns, lines) in outs {
        let lines = lines.into_iter().map(|(n, b)| text(b, n)).collect::<StoreResult<Vec<_>>>()?;
        entry.lines.insert(text(name, len)?, (columns, lines));
    }
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_preserves_outputs_and_validates_skipped_payloads() {
        let mut entry = Entry { key: 5, nodes: 100, ..Entry::default() };
        entry.rows.insert("wanted".into(), (2, vec![5,9]));
        entry.rows.insert("unused".into(), (2, vec![1;2000]));
        entry.lines.insert("diagnostic".into(), (vec![0], vec!["5\tbad".into()]));
        let keep = |r: &str| r == "wanted";
        let rows = to_rows(&entry);
        let entry_copy = entry.clone();
        let projected = Entry { key: 5, ..from_rows_projected(&rows, &keep).unwrap() };
        assert_eq!(projected.rows.len(),1);
        assert_eq!(projected.rows["wanted"],entry.rows["wanted"]);
        assert_eq!(projected.lines,entry.lines);
        assert_eq!(projected.nodes,100);
        let held = Found::Held(std::sync::Arc::new(entry));
        assert_eq!(held.projected(5,&keep).unwrap(),Some(projected));
        assert!(held.projected(6,&keep).unwrap().is_none());
        let mut cut = to_rows(&entry_copy);
        cut.rows.truncate(cut.rows.len() - 1);
        assert!(from_rows_projected(&cut,&keep).is_err(),"a truncated unused payload is still corrupt");
        assert_ne!(with_tree_demand([0;32],false),with_tree_demand([0;32],true),"wider consumers cannot hit narrower frames");
    }

    #[test]
    fn an_entry_round_trips_and_keys_are_stable() {
        let mut entry = Entry { key: 5, nodes: 7, ..Entry::default() };
        entry.rows.insert("fl1_violation_at".into(), (3, vec![5, 1, 2, 5, 3, 4]));
        entry.lines.insert("violation".into(), (vec![1], vec!["r\t21474836480\tx".into()]));
        entry.rows.insert("wide".into(), (11, (0..22).collect()));
        entry.rows.insert("empty".into(), (2, Vec::new()));
        entry.lines.insert("a name longer than one row of text holds, which is fifty-six bytes".into(), (vec![], vec![String::new(), "ünïcode".into()]));
        assert_eq!(from_rows(&to_rows(&entry)).map(|e| Entry { key: 5, ..e }).unwrap(), entry);
        let mut cut = to_rows(&entry);
        cut.rows.truncate(cut.rows.len() - 1);
        assert!(from_rows(&cut).is_err());
        let paths = ["a.py".to_owned(), "b/c.py".to_owned()];
        let keys = doc_keys(&paths);
        assert_eq!(keys, doc_keys(&paths));
        assert_eq!(doc_keys(&paths[1..]), keys[1..], "a key does not depend on the other paths");
        assert!(keys.iter().all(|&k| (1..1 << 30).contains(&k)));
        assert_eq!(document_of(keys[0] << 32 | 9), keys[0]);
        assert_eq!(document_of(keys[1]), keys[1]);
    }
}
