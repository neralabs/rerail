//! The document storage contract (integrated query plan, I6). Evaluation and node references read
//! a file's source and tape only through here: one exact file version per load, answered from the
//! resident copy, the store, or a reparse. Which of those answered never changes the document, and
//! a version that is no longer available is an error, never a different version.
//!
//! A node reference names a node by its preorder ordinal inside one file version, qualified by the
//! workspace and the generation it was taken in. It resolves only while that file version is
//! still current; a stale reference is refused, never rebound to the file's new contents.

use super::persisted::{file_context, grammar_id, grammar_version};
use super::*;
use crate::code_rule_dsl::eval::file_language;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::content_store::{ContentStore, HashKey, StoredDocument, StoredSourceObjects, TapeId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One logical file's exact version: path, source content, file context and the grammar build and
/// grammar a tape of it is parsed with.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct FileVersion {
    pub(super) path: String,
    source: HashKey,
    context: HashKey,
    grammar: i64,
    grammar_version: i64,
}

impl FileVersion {
    pub(super) fn of(file: &FileState) -> Self {
        Self {
            path: file.input.path.clone(),
            source: HashKey::from_sha256_digest(file.source_digest),
            context: file_context(&file.input),
            grammar: file_language(&file.input).map_or(0, grammar_id),
            grammar_version: grammar_version(),
        }
    }

    pub(super) fn source_digest(&self) -> [u8; 32] {
        self.source.sha256_bytes().expect("source uses SHA-256")
    }

    /// The (source, file context) key this version's stored clause results are kept under.
    pub(super) fn result_key(&self) -> (HashKey, HashKey) {
        (self.source, self.context)
    }
}

/// A document the contract loaded: exactly the source and documents of one file version.
pub(super) struct Document {
    pub(super) syntax: std::sync::OnceLock<Arc<crate::code_rule_dsl::relational::syntax::Facts>>,
    pub(super) source: Arc<String>,
    pub(super) image: tape::FileImage,
    language: GrammarDialect,
    bytes: std::sync::atomic::AtomicUsize,
}

/// Resident documents, bounded in bytes. A load beyond the bound evicts the least recently used
/// documents; an evicted document is loaded again from the store or reparsed, never approximated.
pub(super) struct DocumentCache {
    limit: usize,
    used: usize,
    clock: u64,
    entries: HashMap<FileVersion, (u64, Arc<Document>)>,
    pub(super) loads: DocumentLoads,
}

/// Where loaded documents came from, so tests and profiles can tell eviction happened.
#[derive(Clone, Copy, Default)]
pub(super) struct DocumentLoads {
    pub(super) resident: usize,
    pub(super) cached: usize,
    pub(super) stored: usize,
    pub(super) parsed: usize,
    pub(super) evicted: usize,
}

/// The default residency bound for documents loaded through the contract.
const DEFAULT_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

impl Default for DocumentCache {
    fn default() -> Self {
        Self {
            limit: DEFAULT_DOCUMENT_BYTES,
            used: 0,
            clock: 0,
            entries: HashMap::new(),
            loads: DocumentLoads::default(),
        }
    }
}

impl DocumentCache {
    pub(super) fn set_limit(&mut self, bytes: usize) {
        self.limit = bytes;
        self.shrink();
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.used = 0;
    }

    pub(super) fn account_syntax(&mut self, version: &FileVersion) {
        if let Some((_, document)) = self.entries.get(version)
            && let Some(facts) = document.syntax.get()
        {
            let bytes =
                document.source.len() + document.image.estimated_bytes() + facts.estimated_bytes();
            let previous = document
                .bytes
                .swap(bytes, std::sync::atomic::Ordering::Relaxed);
            self.used = self.used.saturating_sub(previous) + bytes;
        }
        self.shrink();
    }

    pub(super) fn resident_bytes(&self) -> usize {
        self.used
    }

    fn get(&mut self, version: &FileVersion) -> Option<Arc<Document>> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(version).map(|(used, document)| {
            *used = clock;
            Arc::clone(document)
        })
    }

    fn insert(&mut self, version: FileVersion, document: Arc<Document>) {
        self.clock += 1;
        self.used += document.bytes.load(std::sync::atomic::Ordering::Relaxed);
        if let Some((_, previous)) = self.entries.insert(version, (self.clock, document)) {
            self.used -= previous.bytes.load(std::sync::atomic::Ordering::Relaxed);
        }
        self.shrink();
    }

    fn shrink(&mut self) {
        while self.used > self.limit {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(version, _)| version.clone())
            else {
                break;
            };
            if let Some((_, document)) = self.entries.remove(&oldest) {
                self.used -= document.bytes.load(std::sync::atomic::Ordering::Relaxed);
                self.loads.evicted += 1;
            }
        }
    }
}

/// The stored object metadata for a batch of sources, with the documents each stored tape
/// derives (`file_documents`). Tape frames are deliberately not prefetched: selector eligibility
/// can prove every AST terminal empty without decompressing a stored tape.
#[derive(Default)]
pub(super) struct StoredDocuments {
    pub(super) objects: HashMap<HashKey, StoredSourceObjects>,
    pub(super) derived: HashMap<TapeId, Vec<StoredDocument>>,
}

pub(super) fn prefetch(store: &ContentStore, sources: &[HashKey]) -> Result<StoredDocuments> {
    let objects = store
        .stored_objects_by_hash(
            sources,
            grammar_version(),
            i64::from(tape::TAPE_FORMAT_VERSION),
        )
        .map_err(|error| invalid(error.to_string()))?;
    let tapes = objects
        .values()
        .flat_map(|stored| stored.tapes.values().copied())
        .collect::<Vec<_>>();
    let derived = store
        .file_documents_bulk(&tapes)
        .map_err(|error| invalid(error.to_string()))?;
    Ok(StoredDocuments { objects, derived })
}

/// The documents the store keeps for one file: its tape `tape_id` and the documents `derived`
/// from it, read in bulk, with whether the read cache held all of them. `None` where any part is
/// missing or fails its checks: a primary is never read without its derived documents.
fn stored_image(
    store: &ContentStore,
    tape_id: TapeId,
    derived: &[StoredDocument],
    grammar: &Arc<tape::Grammar>,
) -> Option<(tape::FileImage, bool)> {
    let sources = derived
        .iter()
        .map(|document| document.source)
        .collect::<Option<Vec<_>>>()?;
    let tape_ids = std::iter::once(tape_id)
        .chain(derived.iter().map(|document| document.tape))
        .collect::<Vec<_>>();
    let tapes = store.get_tapes_bulk(&tape_ids).ok()?;
    let texts = store.get_sources_bulk(&sources).ok()?;
    let (primary, primary_hit) = tapes.get(&tape_id)?;
    let mut cache_hit = *primary_hit;
    let stored = derived
        .iter()
        .zip(&sources)
        .map(|(document, source)| {
            let (text, text_hit) = texts.get(source)?;
            let (tape, tape_hit) = tapes.get(&document.tape)?;
            cache_hit &= *text_hit && *tape_hit;
            Some(tape::StoredDerived {
                ordinal: document.ordinal,
                parent: document.parent_ordinal,
                role: document.role_id,
                via: document.via_id,
                text,
                source_map: &document.source_map,
                host_errors: &document.host_errors,
                tape,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let image = tape::FileImage::read(primary, grammar, &stored).ok()?;
    Some((image, cache_hit))
}

/// Attach one stored file's documents after selector eligibility requires ordinary evaluation.
/// Documents that are missing or do not decode are left out and the file is parsed instead.
pub(super) fn attach_stored_tape(
    store: &ContentStore,
    tape_id: TapeId,
    derived: &[StoredDocument],
    file: &mut FileState,
    grammar: &Arc<tape::Grammar>,
) {
    let Some((image, cache_hit)) = stored_image(store, tape_id, derived, grammar) else {
        return;
    };
    file.tape_cache_hit = cache_hit;
    file.tape_decompressed = !cache_hit;
    file.tape = Some((file.source_revision, image));
}

/// The version the workspace currently holds for `path`: its resident file, else the version it
/// had when evicted.
pub(super) fn current_version(state: &WorkspaceState, path: &str) -> Option<FileVersion> {
    state
        .files
        .get(path)
        .map(FileVersion::of)
        .or_else(|| state.versions.get(path).cloned())
}

/// The exact current version of `path`, loaded from the resident copy, the document cache, the
/// store, or a reparse of the stored source.
pub(super) fn source(state: &mut WorkspaceState, path: &str) -> Result<Arc<String>> {
    let version = current_version(state, path)
        .ok_or_else(|| invalid(format!("{path} is not a file of this workspace")))?;
    if let Some(document) = state.documents.get(&version) {
        return Ok(Arc::clone(&document.source));
    }
    if let Some(file) = state.files.get(path).filter(|file| {
        FileVersion::of(file) == version
            && !file.results_only
            && (file.stored.is_none() || !file.input.source.is_empty())
    }) {
        return Ok(Arc::clone(&file.input.source));
    }
    if let Some(source) = released_source(state, path, &version) {
        return Ok(source);
    }
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| invalid(format!("{path} has no available source")))?;
    let stored = prefetch(&storage.store, &[version.source])?;
    let objects = stored
        .objects
        .get(&version.source)
        .ok_or_else(|| invalid(format!("{path}'s current source is not in the store")))?;
    let bytes = storage
        .store
        .get_source(objects.source)
        .map_err(|e| invalid(e.to_string()))?;
    if HashKey::sha256(&bytes) != version.source {
        return Err(invalid(format!(
            "{path}'s stored source is not its current version"
        )));
    }
    Ok(Arc::new(
        String::from_utf8(bytes).map_err(|_| invalid(format!("{path} is not UTF-8")))?,
    ))
}

pub(super) fn load(state: &mut WorkspaceState, path: &str) -> Result<(FileVersion, Arc<Document>)> {
    load_many(state, &[path.to_owned()], usize::MAX)?
        .pop()
        .ok_or_else(|| invalid("missing document"))
}

/// Restore one bounded publication batch. Only storage misses enter bulk SQL reads.
pub(super) fn load_many(
    state: &mut WorkspaceState,
    paths: &[String],
    max_bytes: usize,
) -> Result<Vec<(FileVersion, Arc<Document>)>> {
    let mut result = (0..paths.len()).map(|_| None).collect::<Vec<_>>();
    let mut pending = Vec::new();
    let mut bytes = 0usize;
    for (index, path) in paths.iter().enumerate() {
        let version = current_version(state, path)
            .ok_or_else(|| invalid(format!("{path} is not a file of this workspace")))?;
        if let Some(document) = state.documents.get(&version) {
            bytes = document_bytes(bytes, document.source.len(), max_bytes)?;
            state.documents.loads.cached += 1;
            result[index] = Some((version, document));
            continue;
        }
        let language = u8::try_from(version.grammar)
            .ok()
            .and_then(crate::code_rule_dsl::language::from_tape_tag)
            .ok_or_else(|| invalid(format!("{path} has no tape grammar")))?;
        let grammar = tape::Grammar::new(language).map_err(invalid)?;
        let resident = state.files.get(path).filter(|file| {
            FileVersion::of(file) == version
                && !file.results_only
                && (file.stored.is_none() || !file.input.source.is_empty())
        });
        let loaded = if let Some(file) = resident {
            bytes = document_bytes(bytes, file.input.source.len(), max_bytes)?;
            state.documents.loads.resident += 1;
            let image = match &file.tape {
                Some((revision, image)) if *revision == file.source_revision => image.clone(),
                _ => parse(&file.input.source, &grammar)?,
            };
            Some((Arc::clone(&file.input.source), image))
        } else if let Some(source) = released_source(state, path, &version) {
            bytes = document_bytes(bytes, source.len(), max_bytes)?;
            state.documents.loads.parsed += 1;
            let image = parse(&source, &grammar)?;
            Some((source, image))
        } else {
            None
        };
        if let Some((source, image)) = loaded {
            result[index] = Some(install_document(state, version, language, source, image));
        } else {
            pending.push((index, version, language, grammar));
        }
    }
    if !pending.is_empty() {
        let store = state
            .storage
            .as_ref()
            .map(|storage| std::sync::Arc::clone(&storage.store))
            .ok_or_else(|| {
                invalid("document batch is not resident and the workspace has no store")
            })?;
        let stored = prefetch(
            &store,
            &pending
                .iter()
                .map(|(_, v, _, _)| v.source)
                .collect::<Vec<_>>(),
        )?;
        let ids = pending
            .iter()
            .filter_map(|(_, v, _, _)| stored.objects.get(&v.source).map(|o| (o.source, v.source)))
            .collect::<Vec<_>>();
        let sources = store
            .get_sources_by_hash_bulk(&ids, max_bytes.saturating_sub(bytes))
            .map_err(|e| invalid(e.to_string()))?;
        // Charge each logical document, including equal contents at different paths, before parsing tapes.
        for (_, version, _, _) in &pending {
            let objects = stored.objects.get(&version.source).ok_or_else(|| {
                invalid(format!(
                    "{}'s current version is not in the store",
                    version.path
                ))
            })?;
            let source = sources
                .get(&objects.source)
                .ok_or_else(|| invalid("stored source is unavailable"))?;
            bytes = document_bytes(bytes, source.len(), max_bytes)?;
        }
        let ids = pending
            .iter()
            .filter_map(|(_, v, _, _)| {
                stored
                    .objects
                    .get(&v.source)
                    .and_then(|o| o.tapes.get(&v.grammar))
                    .copied()
            })
            .collect::<Vec<_>>();
        let tapes = store.get_tapes_bulk(&ids).unwrap_or_default();
        for (index, version, language, grammar) in pending {
            let objects = &stored.objects[&version.source];
            let raw = &sources[&objects.source];
            if HashKey::sha256(raw) != version.source {
                return Err(invalid(format!(
                    "{}'s stored source is not its current version",
                    version.path
                )));
            }
            let source = Arc::new(
                String::from_utf8(raw.clone())
                    .map_err(|_| invalid(format!("{} is not UTF-8", version.path)))?,
            );
            // A tape that derives documents (`file_documents`) is read with them, never without:
            // the primary alone comes from the batch's bulk read.
            let image = objects.tapes.get(&version.grammar).and_then(|&id| {
                let derived = stored.derived.get(&id).map_or(&[][..], Vec::as_slice);
                if derived.is_empty() {
                    let (bytes, _) = tapes.get(&id)?;
                    tape::FileImage::read(bytes, &grammar, &[]).ok()
                } else {
                    stored_image(&store, id, derived, &grammar).map(|(image, _)| image)
                }
            });
            let image = match image {
                Some(image) => {
                    state.documents.loads.stored += 1;
                    image
                }
                None => {
                    state.documents.loads.parsed += 1;
                    parse(&source, &grammar)?
                }
            };
            result[index] = Some(install_document(state, version, language, source, image));
        }
    }
    Ok(result
        .into_iter()
        .map(|d| d.expect("completed document batch"))
        .collect())
}
fn document_bytes(current: usize, added: usize, limit: usize) -> Result<usize> {
    current
        .checked_add(added)
        .filter(|bytes| *bytes <= limit)
        .ok_or_else(|| invalid("repository input budget exceeded"))
}
fn install_document(
    state: &mut WorkspaceState,
    version: FileVersion,
    language: GrammarDialect,
    source: Arc<String>,
    image: tape::FileImage,
) -> (FileVersion, Arc<Document>) {
    let document = Arc::new(Document {
        syntax: std::sync::OnceLock::new(),
        bytes: std::sync::atomic::AtomicUsize::new(source.len() + image.estimated_bytes()),
        source,
        image,
        language,
    });
    state
        .documents
        .insert(version.clone(), Arc::clone(&document));
    (version, document)
}

/// The source of a file whose source was freed while the store was busy (`results_only`), read
/// back from the worktree when it still hashes to the version the file was evaluated at. `None`
/// for any other file, outside a Git worktree, or when the worktree's copy changed: the store is
/// asked then, and a version nobody holds is an error, never a different version.
fn released_source(
    state: &WorkspaceState,
    path: &str,
    version: &FileVersion,
) -> Option<Arc<String>> {
    state
        .files
        .get(path)
        .filter(|file| file.results_only && FileVersion::of(file) == *version)?;
    let storage = state.storage.as_ref()?;
    let repository = storage.repository.as_ref()?;
    let bytes = std::fs::read(
        repository
            .worktree_root
            .join(format!("{}{path}", storage.repository_prefix)),
    )
    .ok()?;
    if HashKey::sha256(&bytes) != version.source {
        return None;
    }
    String::from_utf8(bytes).ok().map(Arc::new)
}

fn parse(source: &str, grammar: &Arc<tape::Grammar>) -> Result<tape::FileImage> {
    tape::TapeImage::parse(grammar, source, &|| false)
        .ok_or_else(|| invalid("document could not be parsed"))?
        .map_err(invalid)
}

/// A snapshot-qualified reference to one node of one file version.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NodeReference {
    format: String,
    workspace: u64,
    generation: u32,
    path: String,
    source: String,
    context: String,
    grammar: i64,
    grammar_version: i64,
    ordinal: u32,
}

const NODE_REFERENCE_FORMAT: &str = "rerail.node-reference/v1";

fn hash_text(hash: HashKey) -> String {
    hash.sha256_bytes()
        .map(|bytes| bytes.iter().map(|byte| format!("{byte:02x}")).collect())
        .unwrap_or_default()
}

fn tape_doc(document: &Document) -> Result<tape::TapeDoc> {
    let grammar = tape::Grammar::new(document.language).map_err(invalid)?;
    let lines = Arc::new(LineTable::new(
        &document.source,
        Terminators::of(document.language),
    ));
    tape::TapeDoc::restore_with_lines(
        Arc::clone(&document.source),
        grammar,
        &document.image,
        lines,
        &|| false,
    )
    .map_err(invalid)
}

/// A reference to the deepest node spanning exactly `start..end` in `path`'s current version.
pub(super) fn reference(
    state: &mut WorkspaceState,
    path: &str,
    start: usize,
    end: usize,
) -> Result<Option<NodeReference>> {
    let (version, document) = load(state, path)?;
    let doc = tape_doc(&document)?;
    let ordinal = (0..doc.transducer_len() as u32)
        .rev()
        .find(|&node| doc.transducer_span(node) == (start..end));
    Ok(ordinal.map(|ordinal| NodeReference {
        format: NODE_REFERENCE_FORMAT.to_owned(),
        workspace: state.instance,
        generation: state.revision,
        path: version.path.clone(),
        source: hash_text(version.source),
        context: hash_text(version.context),
        grammar: version.grammar,
        grammar_version: version.grammar_version,
        ordinal,
    }))
}

/// The node a reference names, or an error when the reference is not this workspace's or its
/// file version is no longer current (edited, removed, moved to another module, or reparsed with
/// another grammar build).
pub(super) fn resolve(state: &mut WorkspaceState, reference: &NodeReference) -> Result<Value> {
    if reference.format != NODE_REFERENCE_FORMAT {
        return Err(invalid("node reference format is not supported"));
    }
    if reference.workspace != state.instance || reference.generation > state.revision {
        return Err(invalid("node reference belongs to another workspace"));
    }
    let current = current_version(state, &reference.path).ok_or_else(|| {
        invalid(format!(
            "stale node reference: {} is no longer in the workspace",
            reference.path
        ))
    })?;
    if hash_text(current.source) != reference.source
        || hash_text(current.context) != reference.context
        || current.grammar != reference.grammar
        || current.grammar_version != reference.grammar_version
    {
        return Err(invalid(format!(
            "stale node reference: {} changed since generation {}",
            reference.path, reference.generation
        )));
    }
    let (_, document) = load(state, &reference.path)?;
    let doc = tape_doc(&document)?;
    if reference.ordinal as usize >= doc.transducer_len() {
        return Err(invalid("node reference ordinal is out of range"));
    }
    let span = doc.transducer_span(reference.ordinal);
    let kind = document
        .language
        .kind_name(doc.transducer_kind(reference.ordinal))
        .unwrap_or("")
        .to_owned();
    let lines = LineTable::new(&document.source, Terminators::of(document.language));
    Ok(json!({
        "path": reference.path,
        "ordinal": reference.ordinal,
        "kind": kind,
        "span": lines.span(span.start, span.end),
        "text": &document.source[span],
    }))
}
