//! Core node facts from the tape (docs/final-shape/03-architecture.md §1, §5; 05 Phase 2), adapted
//! from facts-a's per-content syntactic facts (Moti-Velz/facts-engine `spikes/facts-a/src/facts.rs`):
//! columns indexed by preorder node id, built from Rerail's own tape so ids, kinds, fields and
//! spans equal the matcher's (preorder transducer ids over every tree-sitter node, named and
//! anonymous).
//!
//! facts-a parsed Python only and interned kind names in a process-wide table. Here any dialect's
//! tape is read, and a kind is the atom `tape tag · 1024 + kind code`, so kinds of different
//! dialects never collide and no table is needed. The abstract classes the tape records
//! (Tree-sitter supertypes, tape v8) get postings beside the concrete kinds.

use crate::code_rule_dsl::language::tape_tag;
use crate::code_rule_dsl::lines::LineTable;
use crate::code_rule_dsl::tape::TapeDoc;
use std::sync::Arc;

/// A file as facts read it: its dialect's label and its text. The engine's request files implement it
/// (`model::FileInput`).
pub(crate) trait SourceFile: Send + Sync {
    /// Its dialect's label (`language::dialect_by_label`).
    fn language(&self) -> &str;
    /// Its text.
    fn source(&self) -> &Arc<String>;
    /// The file as its implementation's own type (the engine's request file).
    fn as_any(&self) -> &dyn std::any::Any;
}

/// A file that is no request's (a rule program, a library, a guest): its dialect's label and its text.
pub(crate) struct Source {
    pub(crate) language: String,
    pub(crate) source: Arc<String>,
}

impl SourceFile for Source {
    fn language(&self) -> &str {
        &self.language
    }
    fn source(&self) -> &Arc<String> {
        &self.source
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// What reads the values of a file's nodes off its tape (W's, `matcher::file_values`), registered at load:
/// `each(followed, ordinal, text, types)` for every node with a known value, the plain reading where `plain`, the
/// one that follows bindings where `follow`.
pub(crate) trait ValueReader: Send + Sync {
    fn values(
        &self,
        tape: &TapeDoc,
        file: &dyn SourceFile,
        plain: bool,
        follow: bool,
        each: &mut dyn FnMut(bool, u32, Option<String>, &[&'static str]),
    );
}

static VALUE_READER: std::sync::OnceLock<&'static dyn ValueReader> = std::sync::OnceLock::new();

/// Registers the reader of values (once, at load).
pub(crate) fn install_value_reader(reader: &'static dyn ValueReader) {
    let _ = VALUE_READER.set(reader);
}

/// The registered reader of values; `None` where no engine registered one.
pub(crate) fn value_reader() -> Option<&'static dyn ValueReader> {
    VALUE_READER.get().copied()
}

/// The dialect of `file`'s label.
fn file_language(file: &dyn SourceFile) -> Option<crate::code_rule_dsl::language::GrammarDialect> {
    crate::code_rule_dsl::language::dialect_by_label(file.language())
}

pub(crate) const NONE: u32 = u32::MAX;
/// Kind codes per dialect (the tape's `KIND_LIMIT`).
pub(crate) const KIND_STRIDE: u32 = 1024;

/// Tree-sitter's ERROR symbol, past every grammar's kinds and so past the stride.
const ERROR_SYMBOL: u16 = u16::MAX;

/// The atom of kind `code` in the dialect whose tape tag is `tag`. ERROR's atom takes code 0, the grammar's end
/// symbol, which never labels a node (the tape's own `ERROR_CODE`), so every atom of a dialect stays in its stride:
/// `tag * 1024 + 0xffff` would read as another dialect's kind.
pub(crate) fn kind_atom(tag: u8, code: u16) -> u32 {
    u32::from(tag) * KIND_STRIDE + if code == ERROR_SYMBOL { 0 } else { u32::from(code) }
}

/// The tape tag of a kind atom's dialect.
pub(crate) fn kind_tag(atom: u32) -> u8 {
    (atom / KIND_STRIDE) as u8
}

/// The kind code of a kind atom in its dialect's catalog (`kind_atom`'s inverse: code 0 is ERROR).
pub(crate) fn kind_code(atom: u32) -> u16 {
    match atom % KIND_STRIDE {
        0 => ERROR_SYMBOL,
        code => code as u16,
    }
}

/// The tape tag of a document's dialect (`Facts::tag`): a text-only document, which has no nodes, has its own.
pub(crate) fn facts_tag(f: &Facts) -> u8 {
    f.tag
}

/// One file's facts. Columns are indexed by node id.
pub(crate) struct Facts {
    /// The tape tag of the file's dialect, which every kind atom is of.
    pub tag: u8,
    pub source: Arc<String>,
    pub lines: Arc<LineTable>,
    /// node(id, kind, parent, start, end) + subtree_end, flags, field.
    pub kind: Vec<u32>,
    pub parent: Vec<u32>,
    pub start: Vec<u32>,
    pub end: Vec<u32>,
    pub subtree_end: Vec<u32>,
    /// named | extra<<1 | missing<<2 | error<<3
    pub flags: Vec<u8>,
    /// Field (grammar field code, 0 = none) under which the node hangs from its parent.
    pub field: Vec<u16>,
    /// Children in order: CSR over parent ids.
    pub child_offsets: Vec<u32>,
    pub children: Vec<u32>,
    /// Kind postings (CSR over kind atoms present in this file, sorted by atom).
    pub kind_keys: Vec<u32>,
    pub kind_offsets: Vec<u32>,
    pub kind_nodes: Vec<u32>,
    /// Abstract class postings (CSR over supertype kind atoms, sorted by atom).
    pub class_keys: Vec<u32>,
    pub class_offsets: Vec<u32>,
    pub class_nodes: Vec<u32>,
    pub syntax_errors: bool,
    /// Computed only for a demanded Python syntax verdict; shared by every projection of this tape. `Err`: `ast.parse`
    /// raises another exception than `SyntaxError` (`facts::vtab::syntax::python_syntax`).
    pub python_syntax: std::sync::OnceLock<Result<Option<(u32, String)>, String>>,
}

/// An anonymous child: a token the tape stores ([`Facts::tokens`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Token {
    pub kind: u32,
    pub field: u16,
    pub start: u32,
    pub end: u32,
}

pub(crate) enum Built {
    Facts(Facts),
    /// The parser stopped at a limit (`file_tape` returned `None`).
    Unparsed,
}

/// Parses `file` in its own dialect and reads its facts off the tape.
pub(crate) fn build(file: &dyn SourceFile) -> Result<Built, String> {
    build_with_tape(file, false).map(|(built, _)| built)
}

/// The tape the facts were read off, restored from `primary` only when `keep` (a program's matchers run over it,
/// decision 0106).
fn kept_tape(keep: bool, file: &dyn SourceFile, grammar: &Arc<crate::code_rule_dsl::tape::Grammar>, primary: &crate::code_rule_dsl::tape::TapeImage, lines: &Arc<LineTable>) -> Result<Option<TapeDoc>, String> {
    if !keep {
        return Ok(None);
    }
    let _restore = crate::code_rule_dsl::logic::profile::span(|| "tape restore for the matchers".into());
    let image = crate::code_rule_dsl::tape::FileImage { primary: primary.clone(), derived: Vec::new() };
    TapeDoc::restore_with_lines(Arc::clone(file.source()), Arc::clone(grammar), &image, Arc::clone(lines), &|| false).map(Some)
}

/// [`build`], with the tape the facts were read off where `keep` (decision 0106).
pub(crate) fn build_with_tape(file: &dyn SourceFile, keep: bool) -> Result<(Built, Option<TapeDoc>), String> {
    use crate::code_rule_dsl::{lines::Terminators, tape::{Grammar, TapeImage}};
    let language = file_language(file).ok_or_else(|| format!("{} has no grammar", file.language()))?;
    let grammar = Arc::new(Grammar::new(language)?);
    let parse = crate::code_rule_dsl::logic::profile::span(|| "tape parse".into());
    let Some(image) = TapeImage::parse(&grammar, file.source(), &|| false) else { return Ok((Built::Unparsed, None)); };
    let image = image?;
    drop(parse);
    let lines = Arc::new(LineTable::new(file.source(), Terminators::of(language)));
    let doc = kept_tape(keep, file, &grammar, &image.primary, &lines)?;
    let _facts = crate::code_rule_dsl::logic::profile::span(|| "facts from tape".into());
    let facts = from_image(Arc::clone(file.source()), &grammar, &image.primary, lines)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, facts.start.len() as u64);
    Ok((Built::Facts(facts), doc))
}

/// [`build`], with the file's tape kept in `cache` (a content store, keyed by the source's content,
/// the grammar's version and the tape format): read from it where it has one, else parsed and stored.
/// The corpus runner's parse cache: a file parsed once is not parsed again by a later run.
pub(crate) fn build_cached(file: &dyn SourceFile, cache: &crate::content_store::ContentStore) -> Result<Built, String> {
    build_cached_as(file, cache, None)
}

/// [`build_cached`] for a caller that has already stored the file's source as `known`: it is not hashed or looked up again.
pub(crate) fn build_cached_as(file: &dyn SourceFile, cache: &crate::content_store::ContentStore, known: Option<crate::content_store::SourceId>) -> Result<Built, String> {
    build_cached_source(file, cache, known).map(|(built, _)| built)
}

/// What a file's tape is queued under until the store's writer has written it (decision 0102): its source's
/// SHA-256, its grammar's tag and version, and the tape format. A tape is named by the content it is parsed from.
fn queued_tape(source: &[u8; 32], tag: i64, gv: i64, fv: i64) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"tape", source, &tag.to_le_bytes(), &gv.to_le_bytes(), &fv.to_le_bytes()])
}

/// The sources a run's tape writes store, filled by the store's writer as it writes them: the run's manifest, queued
/// after them, names them.
pub(crate) type Stored = std::sync::Arc<std::sync::Mutex<Vec<crate::content_store::SourceId>>>;

/// [`build`] in a run's waves (decision 0104): no SQL is read or written. The tape is `found` (read before the
/// first wave), or one still queued by this process, else the file is parsed and its source and tape are handed to
/// the store's writer, behind the run (the id it stores the source under goes to `stored`). `digest` is the source's
/// SHA-256, when the caller has it.
pub(crate) fn build_in_wave(
    file: &dyn SourceFile,
    cache: &crate::content_store::ContentStore,
    digest: Option<[u8; 32]>,
    found: Option<&crate::content_store::TapeFrame>,
    stored: Option<&Stored>,
) -> Result<Built, String> {
    build_in_wave_with_tape(file, cache, digest, found, stored, false).map(|(built, _)| built)
}

/// [`build_in_wave`], with the tape the facts were read off where `keep` (a program's matchers run over it, decision
/// 0106).
pub(crate) fn build_in_wave_with_tape(
    file: &dyn SourceFile,
    cache: &crate::content_store::ContentStore,
    digest: Option<[u8; 32]>,
    found: Option<&crate::content_store::TapeFrame>,
    stored: Option<&Stored>,
    keep: bool,
) -> Result<(Built, Option<TapeDoc>), String> {
    use crate::code_rule_dsl::lines::Terminators;
    use crate::code_rule_dsl::tape::{self, TAPE_FORMAT_VERSION, TapeImage};
    use crate::code_rule_dsl::build_ids::grammar_version;
    use crate::content_store::HashKey;
    use sha2::Digest as _;
    let language = file_language(file).ok_or_else(|| format!("{} has no grammar", file.language()))?;
    let tag = i64::from(tape_tag(language).ok_or("the dialect has no tape tag")?);
    let (gv, fv) = (grammar_version(), i64::from(TAPE_FORMAT_VERSION));
    let bytes = file.source().as_bytes();
    let digest: [u8; 32] = match digest {
        Some(d) => d,
        None => {
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
            sha2::Sha256::digest(bytes).into()
        }
    };
    let key = queued_tape(&digest, tag, gv, fv);
    let grammar = Arc::new(tape::Grammar::new(language)?);
    let kept: Option<std::sync::Arc<Vec<u8>>> = match found {
        Some(frame) => {
            let _decode = crate::code_rule_dsl::logic::profile::span(|| "tape decompress".into());
            Some(std::sync::Arc::new(frame.bytes().map_err(|e| e.to_string())?))
        }
        None => cache.pending::<Vec<u8>>(&key),
    };
    let primary = match kept {
        Some(stored) => {
            crate::code_rule_dsl::logic::profile::decision("input tape cache", || "hit".into());
            let _decode = crate::code_rule_dsl::logic::profile::span(|| "tape decode".into());
            TapeImage::from_bytes(&stored, &grammar)?
        }
        None => {
            crate::code_rule_dsl::logic::profile::decision("input tape cache", || "miss".into());
            let parse = crate::code_rule_dsl::logic::profile::span(|| "tape parse".into());
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, file.source().len() as u64);
            let Some(image) = TapeImage::parse(&grammar, file.source(), &|| false) else {
                return Ok((Built::Unparsed, None));
            };
            let image = image?;
            drop(parse);
            let tape = std::sync::Arc::new(image.primary.to_bytes()?);
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, tape.len() as u64);
            // Handed to the writer, the source and the tape each moved, not copied; read from memory until written.
            let (source, held, stored) = (std::sync::Arc::clone(file.source()), std::sync::Arc::clone(&tape), stored.cloned());
            cache.write_back(
                "a tape and its source",
                bytes.len() + tape.len(),
                vec![(key, Some(std::sync::Arc::clone(&tape) as std::sync::Arc<dyn std::any::Any + Send + Sync>))],
                Box::new(move |store| {
                    let (id, _) = store.put_source_and_tape(source.as_bytes(), HashKey::from_sha256_digest(digest), tag, gv, fv, &held)?;
                    if let Some(stored) = &stored {
                        stored.lock().expect("the stored sources").push(id);
                    }
                    Ok(())
                }),
            );
            image.primary
        }
    };
    let lines = Arc::new(LineTable::new(file.source(), Terminators::of(language)));
    let doc = kept_tape(keep, file, &grammar, &primary, &lines)?;
    let _facts = crate::code_rule_dsl::logic::profile::span(|| "facts from tape".into());
    let facts = from_image(Arc::clone(file.source()), &grammar, &primary, lines)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, facts.start.len() as u64);
    Ok((Built::Facts(facts), doc))
}

/// [`build_cached_as`], with the source's id as the store holds it after the build. A parsed tape is stored with its
/// source in one transaction (O95), so a source collected since the caller resolved `known` is stored again, under a
/// new id; `None` when the file did not parse and its source is not known.
pub(crate) fn build_cached_source(file: &dyn SourceFile, cache: &crate::content_store::ContentStore, known: Option<crate::content_store::SourceId>) -> Result<(Built, Option<crate::content_store::SourceId>), String> {
    build_cached_with_tape(file, cache, known, false).map(|(built, id, _)| (built, id))
}

/// [`build_cached_source`], with the tape the facts were read off (decision 0106).
pub(crate) fn build_cached_with_tape(
    file: &dyn SourceFile,
    cache: &crate::content_store::ContentStore,
    known: Option<crate::content_store::SourceId>,
    keep: bool,
) -> Result<(Built, Option<crate::content_store::SourceId>, Option<TapeDoc>), String> {
    // A standalone cache build has the same lookup/parse/publication window as a corpus run.
    let _reading = cache.hold_collection().map_err(|e| e.to_string())?;
    use crate::code_rule_dsl::lines::Terminators;
    use crate::code_rule_dsl::tape::{self, TAPE_FORMAT_VERSION, TapeImage};
    use crate::code_rule_dsl::build_ids::grammar_version;
    use crate::content_store::HashKey;
    let language = file_language(file).ok_or_else(|| format!("{} has no grammar", file.language()))?;
    let tag = i64::from(tape_tag(language).ok_or("the dialect has no tape tag")?);
    let (gv, fv) = (grammar_version(), i64::from(TAPE_FORMAT_VERSION));
    let e = |e: crate::content_store::StoreError| e.to_string();
    let bytes = file.source().as_bytes();
    // A tape this process queued is read from memory (`build_in_wave`): reads see writes.
    {
        use sha2::Digest as _;
        let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
        if let Some(held) = cache.pending::<Vec<u8>>(&queued_tape(&digest, tag, gv, fv)) {
            let grammar = Arc::new(tape::Grammar::new(language)?);
            let primary = TapeImage::from_bytes(&held, &grammar)?;
            let lines = Arc::new(LineTable::new(file.source(), Terminators::of(language)));
            let doc = kept_tape(keep, file, &grammar, &primary, &lines)?;
            return Ok((Built::Facts(from_image(Arc::clone(file.source()), &grammar, &primary, lines)?), known, doc));
        }
    }
    let lookup = crate::code_rule_dsl::logic::profile::span(|| "tape cache lookup".into());
    // The source as the store has it, when it has it: a new one is stored with its tape below.
    let source_id = match known {
        Some(id) => Some(id),
        None => {
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
            cache.source_by_hash(HashKey::sha256(bytes)).map_err(e)?
        }
    };
    let stored = match source_id {
        Some(id) => cache.tape_by_source(id, tag, gv, fv).map_err(e)?,
        None => None,
    };
    drop(lookup);
    let grammar = Arc::new(tape::Grammar::new(language)?);
    let primary = match stored.and_then(|id| cache.get_tapes_bulk(&[id]).ok()?.remove(&id)) {
        Some((stored, _)) => {
            crate::code_rule_dsl::logic::profile::decision("input tape cache", || "hit".into());
            let _decode = crate::code_rule_dsl::logic::profile::span(|| "tape decode".into());
            TapeImage::from_bytes(&stored, &grammar)?
        }
        None => {
            crate::code_rule_dsl::logic::profile::decision("input tape cache", || "miss".into());
            let parse = crate::code_rule_dsl::logic::profile::span(|| "tape parse".into());
            // Its own work: the source prepared for the parser (tree-sitter and the encoding charge their own).
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, file.source().len() as u64);
            let Some(image) = TapeImage::parse(&grammar, file.source(), &|| false) else {
                return Ok((Built::Unparsed, source_id, None));
            };
            let image = image?;
            drop(parse);
            let tape = std::sync::Arc::new(image.primary.to_bytes()?);
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, tape.len() as u64);
            // Handed to the store's writer, behind the run (decision 0104), and read from memory until written
            // (`pending`, above): the source's id is the writer's, so none is returned for a source not stored yet.
            use sha2::Digest as _;
            let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
            let (source, held) = (std::sync::Arc::clone(file.source()), std::sync::Arc::clone(&tape));
            cache.write_back(
                "a tape and its source",
                bytes.len() + tape.len(),
                vec![(queued_tape(&digest, tag, gv, fv), Some(tape as std::sync::Arc<dyn std::any::Any + Send + Sync>))],
                Box::new(move |store| store.put_source_and_tape(source.as_bytes(), HashKey::from_sha256_digest(digest), tag, gv, fv, &held).map(|_| ())),
            );
            image.primary
        }
    };
    let lines = Arc::new(LineTable::new(file.source(), Terminators::of(language)));
    let doc = kept_tape(keep, file, &grammar, &primary, &lines)?;
    let _facts = crate::code_rule_dsl::logic::profile::span(|| "facts from tape".into());
    let facts = from_image(Arc::clone(file.source()), &grammar, &primary, lines)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, facts.start.len() as u64);
    Ok((Built::Facts(facts), source_id, doc))
}

mod postings;

pub(crate) fn from_tape(doc: &TapeDoc, lines: Arc<LineTable>) -> Result<Facts, String> {
    let tag = tape_tag(doc.file_dialect()).ok_or("the tape's dialect has no tape tag")?;
    let n = doc.transducer_len();
    let mut f = empty(tag, Arc::clone(doc.document_source()), lines, n);
    f.syntax_errors = !doc.syntax_error_ranges().is_empty();
    let columns_span = crate::code_rule_dsl::logic::profile::span(|| "node columns".into());
    for id in 0..n as u32 {
        f.kind.push(kind_atom(tag, doc.transducer_kind(id)));
        f.parent.push(doc.transducer_parent(id).unwrap_or(NONE));
        let span = doc.transducer_span(id);
        f.start.push(span.start as u32);
        f.end.push(span.end as u32);
        f.subtree_end.push(doc.transducer_descendants(id).end);
        f.flags.push(doc.transducer_flags(id));
        f.field.push(doc.transducer_field(id));
    }
    drop(columns_span);
    index(&mut f);
    let _classes = crate::code_rule_dsl::logic::profile::span(|| "class postings".into());
    let classes = doc.transducer_supertypes().map(|(node, class)| (kind_atom(tag, class), node)).collect();
    (f.class_keys, f.class_offsets, f.class_nodes) = postings(classes);
    Ok(f)
}

/// Allocates only the core columns. Line numbers are lookups in the shared source line table.
fn empty(tag: u8, source: Arc<String>, lines: Arc<LineTable>, n: usize) -> Facts {
    Facts {
        tag,
        source,
        lines,
        kind: Vec::with_capacity(n),
        parent: Vec::with_capacity(n),
        start: Vec::with_capacity(n),
        end: Vec::with_capacity(n),
        subtree_end: Vec::with_capacity(n),
        flags: Vec::with_capacity(n),
        field: Vec::with_capacity(n),
        child_offsets: Vec::new(),
        children: Vec::new(),
        kind_keys: Vec::new(),
        kind_offsets: Vec::new(),
        kind_nodes: Vec::new(),
        class_keys: Vec::new(),
        class_offsets: Vec::new(),
        class_nodes: Vec::new(),
        syntax_errors: false,
        python_syntax: Default::default(),
    }

}

/// Reads the admitted tape directly into columns. The matcher and this reader share validation.
pub(crate) fn from_image(source: Arc<String>, grammar: &crate::code_rule_dsl::tape::Grammar,
    image: &crate::code_rule_dsl::tape::TapeImage, lines: Arc<LineTable>) -> Result<Facts, String> {
    let tag = tape_tag(grammar.dialect()).ok_or("the tape's dialect has no tape tag")?;
    let mut f = empty(tag, Arc::clone(&source), lines, 0);
    let mut classes = Vec::new();
    let decode = crate::code_rule_dsl::logic::profile::span(|| "validated tape columns".into());
    crate::code_rule_dsl::tape::validated::visit(&source, grammar, image, &|| false, |node, memberships| {
        if node.id == 0 { f = empty(tag, Arc::clone(&f.source), Arc::clone(&f.lines), image.node_count()); }
        f.kind.push(kind_atom(tag, node.kind));
        f.parent.push(node.parent);
        f.start.push(node.start);
        f.end.push(node.end);
        f.subtree_end.push(node.id + node.size);
        f.flags.push(node.flags);
        f.field.push(node.field);
        f.syntax_errors |= node.flags & 12 != 0;
        classes.extend(memberships.iter().map(|&class| (kind_atom(tag, class), node.id)));
    })?;
    drop(decode);
    index(&mut f);
    let _classes = crate::code_rule_dsl::logic::profile::span(|| "class postings".into());
    (f.class_keys, f.class_offsets, f.class_nodes) = postings(classes);
    Ok(f)
}

/// The tape tag of a text-only document's facts (`text`): no dialect has it.
pub(crate) const TEXT: u8 = u8::MAX;

/// The facts of a file read as text only (core.rr's `.scope text`): its source and lines, no tree. Its lines are W's
/// for a file with no grammar (`Terminators::Eslint`: `\r\n`, `\r`, `\n`, U+2028, U+2029).
pub(crate) fn text(source: Arc<String>) -> Facts {
    let lines = Arc::new(LineTable::new(&source, crate::code_rule_dsl::lines::Terminators::Eslint));
    let mut f = Facts {
        tag: TEXT,
        source,
        lines,
        kind: Vec::new(),
        parent: Vec::new(),
        start: Vec::new(),
        end: Vec::new(),
        subtree_end: Vec::new(),
        flags: Vec::new(),
        field: Vec::new(),
        child_offsets: Vec::new(),
        children: Vec::new(),
        kind_keys: Vec::new(),
        kind_offsets: Vec::new(),
        kind_nodes: Vec::new(),
        class_keys: Vec::new(),
        class_offsets: Vec::new(),
        class_nodes: Vec::new(),
        syntax_errors: false,
        python_syntax: Default::default(),
    };
    index(&mut f);
    (f.class_keys, f.class_offsets, f.class_nodes) = postings(Vec::new());
    f
}

/// The derived columns: the children CSR (preorder lists a parent's children in order) and the
/// kind postings.
fn index(f: &mut Facts) {
    let children_span = crate::code_rule_dsl::logic::profile::span(|| "child CSR".into());
    let n = f.len();
    let mut counts = vec![0u32; n + 1];
    for &p in &f.parent {
        if p != NONE {
            counts[p as usize + 1] += 1;
        }
    }
    for i in 1..=n {
        counts[i] += counts[i - 1];
    }
    let mut cursor = counts.clone();
    let mut children = vec![0u32; counts[n] as usize];
    for (id, &p) in f.parent.iter().enumerate() {
        if p != NONE {
            children[cursor[p as usize] as usize] = id as u32;
            cursor[p as usize] += 1;
        }
    }
    f.child_offsets = counts;
    f.children = children;
    drop(children_span);
    let _kinds = crate::code_rule_dsl::logic::profile::span(|| "kind postings".into());
    (f.kind_keys, f.kind_offsets, f.kind_nodes) = postings::by_kind(&f.kind);
}

/// (key, node) pairs as a CSR: sorted keys, offsets (one past the last), nodes by key then id.
fn postings(mut pairs: Vec<(u32, u32)>) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    let (mut keys, mut offsets, mut nodes) = (Vec::new(), Vec::new(), Vec::with_capacity(pairs.len()));
    pairs.sort_unstable();
    for (k, id) in pairs {
        if keys.last() != Some(&k) {
            keys.push(k);
            offsets.push(nodes.len() as u32);
        }
        nodes.push(id);
    }
    offsets.push(nodes.len() as u32);
    (keys, offsets, nodes)
}

fn posting<'a>(keys: &[u32], offsets: &[u32], nodes: &'a [u32], key: u32) -> &'a [u32] {
    match keys.binary_search(&key) {
        Ok(i) => &nodes[offsets[i] as usize..offsets[i + 1] as usize],
        Err(_) => &[],
    }
}

impl Facts {
    pub fn start_line(&self, id: u32) -> u32 {
        self.lines.line(self.start[id as usize] as usize) as u32 + 1
    }
    pub fn len(&self) -> usize {
        self.kind.len()
    }
    /// The tree's depth: nodes on the longest root-to-leaf path (ids are preorder, so a parent's
    /// id is below its children's).
    pub fn depth(&self) -> u32 {
        let mut at = vec![1u32; self.len()];
        for i in 0..self.len() {
            let p = self.parent[i] as usize;
            if p < i {
                at[i] = at[p] + 1;
            }
        }
        at.into_iter().max().unwrap_or(0)
    }
    pub fn children(&self, id: u32) -> &[u32] {
        let a = self.child_offsets[id as usize] as usize;
        let b = self.child_offsets[id as usize + 1] as usize;
        &self.children[a..b]
    }
    /// Whether `id` has no children.
    pub fn is_leaf(&self, id: u32) -> bool {
        self.children(id).is_empty()
    }
    /// `id`'s anonymous children in source order: the tokens the tape stores (the grammar's implied
    /// ones are not, RER-131).
    pub fn tokens(&self, id: u32) -> Vec<Token> {
        self.children(id)
            .iter()
            .filter(|&&c| !self.named(c))
            .map(|&c| Token { kind: self.kind[c as usize], field: self.field[c as usize], start: self.start[c as usize], end: self.end[c as usize] })
            .collect()
    }
    /// A token's text.
    pub fn token_text(&self, token: &Token) -> &str {
        &self.source[token.start as usize..token.end as usize]
    }
    pub fn of_kind(&self, kind: u32) -> &[u32] {
        posting(&self.kind_keys, &self.kind_offsets, &self.kind_nodes, kind)
    }
    /// Nodes of the abstract class `class` (a supertype's kind atom), by id.
    pub fn of_class(&self, class: u32) -> &[u32] {
        posting(&self.class_keys, &self.class_offsets, &self.class_nodes, class)
    }
    pub fn text(&self, id: u32) -> &str {
        &self.source[self.start[id as usize] as usize..self.end[id as usize] as usize]
    }
    pub fn named(&self, id: u32) -> bool {
        self.flags[id as usize] & 1 != 0
    }
    pub fn parent(&self, id: u32) -> Option<u32> {
        let p = self.parent[id as usize];
        (p != NONE).then_some(p)
    }
    /// Approximate resident bytes of the columns (excluding source).
    pub fn bytes(&self) -> usize {
        let n = self.len();
        n * (4 * 5 + 1 + 2)
            + self.children.len() * 4
            + self.child_offsets.len() * 4
            + self.kind_nodes.len() * 4
            + self.class_nodes.len() * 4
    }
}

/// The digest of a file's core facts: every column a reader can see, and the source their spans
/// read text from, so equal digests mean equal facts (the core derivation's output, `families`).
/// Cutoff on an edit happens in the narrower families that read these, not here.
pub(crate) fn digest(f: &Facts) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"rerail.core-facts/v1\0");
    hash.update((f.source.len() as u64).to_le_bytes());
    hash.update(f.source.as_bytes());
    hash.update((f.len() as u64).to_le_bytes());
    // Each column as one run of little-endian bytes (what one `update` per value fed, at a fraction of the calls).
    let mut bytes = f.source.len() + f.flags.len() + std::mem::size_of_val(&f.field[..]);
    for column in [&f.kind, &f.parent, &f.start, &f.end, &f.class_keys, &f.class_offsets, &f.class_nodes] {
        hash.update((column.len() as u64).to_le_bytes());
        update_le(&mut hash, column);
        bytes += std::mem::size_of_val(&column[..]);
    }
    hash.update(&f.flags);
    update_le(&mut hash, &f.field);
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes as u64);
    hash.update([u8::from(f.syntax_errors)]);
    hash.finalize().into()
}

/// Feeds `values` to `hash` as their little-endian bytes, in order.
fn update_le<T: Copy + LeBytes>(hash: &mut sha2::Sha256, values: &[T]) {
    use sha2::Digest as _;
    if cfg!(target_endian = "little") {
        // SAFETY: `T` is a primitive integer (`LeBytes`), so `values` is `size_of_val(values)` initialized bytes with
        // no padding, in memory order, which is little-endian here.
        hash.update(unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values)) });
    } else {
        for value in values {
            value.feed(hash);
        }
    }
}

/// An integer type with no padding whose bytes `update_le` may read in place.
trait LeBytes {
    fn feed(&self, hash: &mut sha2::Sha256);
}
impl LeBytes for u32 {
    fn feed(&self, hash: &mut sha2::Sha256) {
        sha2::Digest::update(hash, self.to_le_bytes());
    }
}
impl LeBytes for u16 {
    fn feed(&self, hash: &mut sha2::Sha256) {
        sha2::Digest::update(hash, self.to_le_bytes());
    }
}

#[cfg(test)]
mod tests;

/// The relations `core.rr` declares (the engine's names, decision 0047), as the rule language reads them;
/// registered at load (`install_core_names`). Empty without one.
pub(crate) fn core_names() -> &'static std::collections::BTreeSet<String> {
    static EMPTY: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    CORE_NAMES.get().map_or(&EMPTY, |names| names())
}

static CORE_NAMES: std::sync::OnceLock<fn() -> &'static std::collections::BTreeSet<String>> = std::sync::OnceLock::new();

/// Registers what reads `core.rr`'s names (once, at load).
pub(crate) fn install_core_names(names: fn() -> &'static std::collections::BTreeSet<String>) {
    let _ = CORE_NAMES.set(names);
}

/// The columns of the planner's base relations (T's input schema). Others are `c0`, `c1`, ….
pub(crate) fn input_columns(rel: &str) -> Option<&'static [&'static str]> {
    Some(match rel {
        "node" => &["d", "n", "kind", "start", "end_"],
        "line" => &["n", "l"],
        "doc" => &["d", "path", "module"],
        "child" => &["p", "c", "idx"],
        "field" => &["p", "f", "c"],
        "text" | "name" | "str" | "strprefix" | "node_text" | "comment" | "structural_before" | "structural_after" => &["n", "s"],
        // W's values (`facts::vtab::syntax::Extra`).
        "value" | "static_value" => &["n", "v"],
        "value_type_of" => &["n", "t"],
        "value_type" => &["v", "t"],
        // W's f-string trees (`facts::vtab::syntax::fstring`).
        "fstring_piece" => &["n", "i", "kind", "v"],
        "fstring_template" => &["n", "marker", "v"],
        // CPython's verdict (`facts::vtab::syntax::cpython`; core.rr's `syntax_error(d, l, msg)`).
        "cpython_syntax_error" => &["d", "l", "msg"],
        // Ruff's string annotations and their trees (`facts::vtab::syntax::annotation`).
        "string_annotation" => &["s"],
        "parsed" => &["s", "grammar", "root"],
        // W's pythonAst `literal_json` (`facts::vtab::syntax::cpython`).
        "literal_json" => &["n", "v"],
        // core.rr's @py_unparse(n) and @ast_dump(n), and W's hashes of dumps (`facts::vtab::syntax::cpython`).
        "cpython_unparse" | "cpython_ast_dump" => &["n", "s"],
        "dump_sha256" => &["n", "h"],
        "body_sha256" | "signature_sha256" => &["f", "h"],
        "intval" => &["n", "v"],
        "op_at" => &["n", "i", "s"],
        "op" | "op_t" => &["n", "s"],
        // `core.rr`'s (`facts::vtab::syntax`, decision 0046).
        "syntax_error" => &["d", "l", "msg"],
        "spread" => &["n"],
        "next_sibling" => &["a", "b"],
        "visit_index" => &["n", "i"],
        "node_order" => &["n", "k"],
        "ast_depth" => &["n", "k"],
        "line_count" => &["d", "k"],
        "end_line" => &["n", "l"],
        "col" => &["n", "c"],
        "docof" => &["n", "d"],
        "source" => &["d", "sp"],
        "src_line" | "src_line_lf" | "tape_line" => &["d", "l", "sp"],
        "split_line" | "lf_line" => &["d", "l", "t"],
        "str_part" => &["lit", "i", "v", "at", "spec"],
        "str_part_word" => &["lit", "i", "w", "at", "spec"],
        "str_part_start" => &["lit", "i", "l", "c"],
        "first_token" | "last_token" => &["n", "t"],
        "token_before" | "token_after" => &["x", "t"],
        "tree_digest" | "def_digest" => &["n", "s"],
        // core.rr's tokens (`facts::vtab::syntax::tree`).
        "prev_token" => &["n", "s"],
        "token" => &["p", "i", "s", "start", "end_"],
        // W's Python reference graph (`logic::plan::corpus::references`).
        "reference_edge" => &["src", "dst", "via"],
        "reference_open" => &["n"],
        "rx_span" => &["p", "c", "j", "g", "m"],
        "ast_match" => &["p", "n", "g", "c"],
        // Decision 0055's (`facts::vtab::syntax::guests`).
        "guest" => &["g", "host", "at", "grammar"],
        "doc_role" => &["d", "r"],
        "doc_root" => &["d", "n"],
        "tape_of" => &["d", "t"],
        "sub_tape" => &["at", "grammar", "t"],
        "host_node" => &["n", "h"],
        // Decision 0058's tape operations.
        "parse_of" => &["x", "g", "t"],
        "unparse_of" => &["t", "x"],
        "splice_of" => &["t", "n", "t2", "t3"],
        // The repository inputs (`facts::repository`).
        "option" => &["name", "v"],
        "option_item" => &["name", "i", "v"],
        "file" => &["path"],
        "package" => &["dir", "name"],
        "package_field" => &["dir", "key", "v"],
        "package_for" => &["path", "dir"],
        "pkg" => &["d", "p"],
        "path_part" | "file_name" => &["d", "s"],
        "tape" => &["n", "par", "size", "depth"],
        "core_node" => &["d", "t", "n"],
        // Containment read off the tree (`facts::vtab::syntax::within`, decision 0050).
        "within" => &["n", "a"],
        "span_within" => &["inner", "outer"],
        // The transducer's `nearest` rows, and the tree axes' (`facts::vtab::syntax::with_nearest`).
        rel if crate::code_rule_dsl::logic::front::is_transduced(rel) => &["n", "a"],
        rel if rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) => &["s"],
        // An `ast` matcher's matched nodes (decision 0106).
        rel if rel.starts_with(crate::code_rule_dsl::logic::plan::corpus::AST_MATCHES) => crate::code_rule_dsl::logic::plan::corpus::ast_columns(rel),
        // A `match_all` regex's matches (`logic::strings::as_relations`, `facts::vtab::syntax::Extra`).
        rel if rel.starts_with(crate::code_rule_dsl::logic::strings::MATCHES) => &["sp", "m"],
        rel if rel.starts_with(crate::code_rule_dsl::logic::strings::GROUPS) => &["sp", "m", "g", "gsp"],
        // A registered family's lazy relations (`facts::vtab::syntax::Families`: Python's `py.*`).
        rel if rel.starts_with("py_") => crate::code_rule_dsl::facts::vtab::syntax::family_columns(rel)?,
        _ => return None,
    })
}

/// The executor of a binding (the run's lanes: every lane under the device binding, the cores' otherwise), registered
/// at load (`install_executors`); without one, a fresh executor of the machine's lanes.
pub(crate) fn executor_for(device: bool) -> &'static rerail_accel::Executor {
    static OWN: std::sync::LazyLock<rerail_accel::Executor> = std::sync::LazyLock::new(rerail_accel::Executor::new);
    EXECUTORS.get().map_or(&*OWN, |executor| executor(device))
}

static EXECUTORS: std::sync::OnceLock<fn(bool) -> &'static rerail_accel::Executor> = std::sync::OnceLock::new();

/// Registers the run's executors (once, at load).
pub(crate) fn install_executors(executor: fn(bool) -> &'static rerail_accel::Executor) {
    let _ = EXECUTORS.set(executor);
}

/// The relations CPython's facts bind (the corpus's names for the verdict, `ast.unparse`, `ast.dump` and their
/// hashes) and their columns: base relations the planner reads by name.
pub(crate) mod cpython_relations {
    /// The relation the corpus binds for CPython's verdict (core.rr's `syntax_error(d, l, msg)`).
    pub(crate) const SYNTAX_ERROR: &str = "cpython_syntax_error";
    /// W's `literal_json` (`cpython::literal`).
    pub(crate) const LITERAL_JSON: &str = "literal_json";
    /// core.rr's `@py_unparse(n)` (`cpython::unparse`).
    pub(crate) const UNPARSE: &str = "cpython_unparse";
    /// core.rr's `@ast_dump(n)` (`cpython::dump`).
    pub(crate) const AST_DUMP: &str = "cpython_ast_dump";
    /// W's `dump_sha256`, `body_sha256` and `signature_sha256` (`cpython::dump`).
    pub(crate) const DUMP_SHA256: &str = "dump_sha256";
    pub(crate) const BODY_SHA256: &str = "body_sha256";
    pub(crate) const SIGNATURE_SHA256: &str = "signature_sha256";

    /// A printed relation's columns.
    pub(crate) fn columns(rel: &str) -> &'static [&'static str] {
        match rel {
            UNPARSE | AST_DUMP => &["n", "s"],
            DUMP_SHA256 => &["n", "h"],
            _ => &["f", "h"],
        }
    }
}
