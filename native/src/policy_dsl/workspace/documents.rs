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
use crate::content_store::{ContentStore, HashKey, StoredSourceObjects};
use crate::policy_dsl::eval::{file_language, span_json};
use crate::policy_dsl::lines::{LineTable, Terminators};
use ast_grep_language::LanguageExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tree_sitter::Parser;

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

    /// The (source, file context) key this version's stored check results are kept under.
    pub(super) fn result_key(&self) -> (HashKey, HashKey) {
        (self.source, self.context)
    }
}

/// A document the contract loaded: exactly the source and tape of one file version.
pub(super) struct Document {
    pub(super) source: Arc<String>,
    pub(super) image: tape::TapeImage,
    language: GrammarDialect,
    bytes: usize,
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
        self.used += document.bytes;
        if let Some((_, previous)) = self.entries.insert(version, (self.clock, document)) {
            self.used -= previous.bytes;
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
                self.used -= document.bytes;
                self.loads.evicted += 1;
            }
        }
    }
}

/// The stored object metadata for a batch of sources. Tape frames are deliberately not prefetched:
/// selector eligibility can prove every AST terminal empty without decompressing a stored tape.
#[derive(Default)]
pub(super) struct StoredDocuments {
    pub(super) objects: HashMap<HashKey, StoredSourceObjects>,
}

pub(super) fn prefetch(store: &ContentStore, sources: &[HashKey]) -> Result<StoredDocuments> {
    let objects = store
        .stored_objects_by_hash(
            sources,
            grammar_version(),
            i64::from(tape::TAPE_FORMAT_VERSION),
        )
        .map_err(|error| invalid(error.to_string()))?;
    Ok(StoredDocuments { objects })
}

/// Attach one stored tape after selector eligibility requires ordinary evaluation. A tape that
/// does not decode is left out and the file is parsed instead.
pub(super) fn attach_stored_tape(
    store: &ContentStore,
    tape_id: crate::content_store::TapeId,
    file: &mut FileState,
    grammar: &Arc<tape::Grammar>,
) {
    let Ok(tapes) = store.get_tapes_bulk(&[tape_id]) else {
        return;
    };
    let Some((bytes, cache_hit)) = tapes.get(&tape_id) else {
        return;
    };
    file.tape_cache_hit = *cache_hit;
    file.tape_decompressed = !*cache_hit;
    if let Ok(image) = tape::TapeImage::from_bytes(bytes, grammar) {
        file.tape = Some((file.source_revision, image));
    }
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
pub(super) fn load(state: &mut WorkspaceState, path: &str) -> Result<(FileVersion, Arc<Document>)> {
    let version = current_version(state, path)
        .ok_or_else(|| invalid(format!("{path} is not a file of this workspace")))?;
    if let Some(document) = state.documents.get(&version) {
        state.documents.loads.cached += 1;
        return Ok((version, document));
    }
    let language = u8::try_from(version.grammar)
        .ok()
        .and_then(crate::policy_dsl::language::from_tape_tag)
        .ok_or_else(|| invalid(format!("{path} has no tape grammar")))?;
    let grammar = tape::Grammar::new(language).map_err(invalid)?;
    let resident = state.files.get(path).filter(|file| {
        FileVersion::of(file) == version && (file.stored.is_none() || !file.input.source.is_empty())
    });
    let (source, image) = if let Some(file) = resident {
        state.documents.loads.resident += 1;
        let image = match &file.tape {
            Some((revision, image)) if *revision == file.source_revision => image.clone(),
            _ => parse(&file.input.source, language, &grammar)?,
        };
        (Arc::clone(&file.input.source), image)
    } else {
        let storage = state.storage.as_ref().ok_or_else(|| {
            invalid(format!(
                "{path} is not resident and the workspace has no store"
            ))
        })?;
        let stored = prefetch(&storage.store, &[version.source])?;
        let objects = stored
            .objects
            .get(&version.source)
            .ok_or_else(|| invalid(format!("{path}'s current version is not in the store")))?;
        let bytes = storage
            .store
            .get_source(objects.source)
            .map_err(|error| invalid(error.to_string()))?;
        if HashKey::sha256(&bytes) != version.source {
            return Err(invalid(format!(
                "{path}'s stored source is not its current version"
            )));
        }
        let source = Arc::new(
            String::from_utf8(bytes).map_err(|_| invalid(format!("{path} is not UTF-8")))?,
        );
        let image = objects.tapes.get(&version.grammar).and_then(|&tape_id| {
            let tapes = storage.store.get_tapes_bulk(&[tape_id]).ok()?;
            let (bytes, _) = tapes.get(&tape_id)?;
            tape::TapeImage::from_bytes(bytes, &grammar).ok()
        });
        let image = match image {
            Some(image) => {
                state.documents.loads.stored += 1;
                image
            }
            None => {
                state.documents.loads.parsed += 1;
                parse(&source, language, &grammar)?
            }
        };
        (source, image)
    };
    let document = Arc::new(Document {
        bytes: source.len() + image.estimated_bytes(),
        source,
        image,
        language,
    });
    state
        .documents
        .insert(version.clone(), Arc::clone(&document));
    Ok((version, document))
}

fn parse(
    source: &str,
    language: GrammarDialect,
    grammar: &Arc<tape::Grammar>,
) -> Result<tape::TapeImage> {
    let mut parser = Parser::new();
    parser
        .set_language(&language.get_ts_language())
        .map_err(|error| invalid(error.to_string()))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| invalid("document could not be parsed"))?;
    tape::TapeImage::encode(tree.root_node(), grammar, source, &|| false).map_err(invalid)
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
        "span": span_json(&lines, span.start, span.end),
        "text": &document.source[span],
    }))
}
