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
pub trait SourceFile: Send + Sync {
    /// Its dialect's label (`language::dialect_by_label`).
    fn language(&self) -> &str;
    /// Its text.
    fn source(&self) -> &Arc<String>;
    /// The file as its implementation's own type (the engine's request file).
    fn as_any(&self) -> &dyn std::any::Any;
}

/// A file that is no request's (a rule program, a library, a guest): its dialect's label and its text.
pub struct Source {
    pub language: String,
    pub source: Arc<String>,
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
pub trait ValueReader: Send + Sync {
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
pub fn install_value_reader(reader: &'static dyn ValueReader) {
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

pub const NONE: u32 = u32::MAX;
/// Kind codes per dialect (the tape's `KIND_LIMIT`).
pub const KIND_STRIDE: u32 = 1024;

/// Tree-sitter's ERROR symbol, past every grammar's kinds and so past the stride.
const ERROR_SYMBOL: u16 = u16::MAX;

/// The codes a test of a document reads, as its table holds them.
#[derive(Clone, Copy)]
enum Codes {
    /// A kind's (by its code): the table's codes of it with no ERROR flag; ERROR's slot also every ERROR code and
    /// kind 0's.
    Kind(u16),
    /// A class's.
    Class(u16),
    /// The ERROR and MISSING codes.
    Faults,
}

/// The atom of kind `code` in the dialect whose tape tag is `tag`. ERROR's atom takes code 0, the grammar's end
/// symbol, which never labels a node (the tape's own `ERROR_CODE`), so every atom of a dialect stays in its stride:
/// `tag * 1024 + 0xffff` would read as another dialect's kind.
pub fn kind_atom(tag: u8, code: u16) -> u32 {
    u32::from(tag) * KIND_STRIDE + if code == ERROR_SYMBOL { 0 } else { u32::from(code) }
}

/// The tape tag of a kind atom's dialect.
pub fn kind_tag(atom: u32) -> u8 {
    (atom / KIND_STRIDE) as u8
}

/// The kind code of a kind atom in its dialect's catalog (`kind_atom`'s inverse: code 0 is ERROR).
pub fn kind_code(atom: u32) -> u16 {
    match atom % KIND_STRIDE {
        0 => ERROR_SYMBOL,
        code => code as u16,
    }
}

/// The tape tag of a document's dialect (`Facts::tag`): a text-only document, which has no nodes, has its own.
pub fn facts_tag(f: &Facts) -> u8 {
    f.tag
}

/// One file's facts, read off its tape where it lies (decision 0293, step 4): a node's kind, parent, span, subtree,
/// flags, field and children are the tape's answers through the one API, in whichever format the store keeps, and
/// the nodes of a kind or class are a scan of the tape's codes, 64 nodes at a time. Nothing is copied off the tape.
pub struct Facts {
    /// The tape tag of the file's dialect, which every kind atom is of.
    pub tag: u8,
    pub source: Arc<String>,
    pub lines: Arc<LineTable>,
    /// The tape, its ids the facts' (decision 0293); `None` for a text-only document, which has no nodes.
    pub tape: Option<Arc<crate::code_rule_dsl::tape::formats::DocTape>>,
    /// The dialect's grammar: whether a kind is named.
    grammar: Option<Arc<crate::code_rule_dsl::tape::Grammar>>,
    /// The ERROR and MISSING nodes, ascending, found on first ask (a scan of the codes): few, and most files have none.
    errors: std::sync::OnceLock<Vec<u32>>,
    /// The codes the document holds, ascending, found on the first kind or class test (a scan of the codes): a
    /// summary of the document, not of its nodes. On format 16 the table is the language's, so a test of a kind the
    /// document lacks is answered here without a scan, as format 15's table of the document's own codes answers it.
    held: std::sync::OnceLock<Held>,
    /// Computed only for a demanded Python syntax verdict; shared by every projection of this tape. `Err`: `ast.parse`
    /// raises another exception than `SyntaxError` (`facts::vtab::syntax::python_syntax`).
    pub python_syntax: std::sync::OnceLock<Result<Option<(u32, String)>, String>>,
}

/// What a document holds, ascending: its codes, the kinds of those with no ERROR flag, and their classes.
struct Held {
    codes: Box<[u32]>,
    kinds: Box<[u16]>,
    classes: Box<[u16]>,
}

/// An anonymous child: a token the tape stores ([`Facts::tokens`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Token {
    pub kind: u32,
    pub field: u16,
    pub start: u32,
    pub end: u32,
}

pub enum Built {
    Facts(Facts),
    /// The parser stopped at a limit (`file_tape` returned `None`).
    Unparsed,
}

/// Parses `file` in its own dialect and reads its facts off the tape.
pub fn build(file: &dyn SourceFile) -> Result<Built, String> {
    build_with_tape(file, false).map(|(built, _)| built)
}

/// The facts of `primary`: the kept document's tape where the matchers keep one (shared, not held twice), else the
/// image's, checked and held.
fn read_off(doc: Option<&TapeDoc>, file: &dyn SourceFile, grammar: &Arc<crate::code_rule_dsl::tape::Grammar>, primary: crate::code_rule_dsl::tape::TapeImage, lines: Arc<LineTable>) -> Result<Facts, String> {
    match doc {
        Some(doc) => from_tape(doc, lines),
        None => from_image(Arc::clone(file.source()), grammar, primary, lines),
    }
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
pub fn build_with_tape(file: &dyn SourceFile, keep: bool) -> Result<(Built, Option<TapeDoc>), String> {
    use crate::code_rule_dsl::{lines::Terminators, tape::{Grammar, TapeImage}};
    let language = file_language(file).ok_or_else(|| format!("{} has no grammar", file.language()))?;
    let grammar = Grammar::embedded(language)?;
    let parse = crate::code_rule_dsl::logic::profile::span(|| "tape parse".into());
    let Some(image) = TapeImage::parse(&grammar, file.source(), &|| false) else { return Ok((Built::Unparsed, None)); };
    let image = image?;
    drop(parse);
    let lines = Arc::new(LineTable::new(file.source(), Terminators::of(language)));
    let doc = kept_tape(keep, file, &grammar, &image.primary, &lines)?;
    let _facts = crate::code_rule_dsl::logic::profile::span(|| "facts from tape".into());
    let facts = read_off(doc.as_ref(), file, &grammar, image.primary, lines)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, facts.len() as u64);
    Ok((Built::Facts(facts), doc))
}

/// [`build`], with the file's tape kept in `cache` (a content store, keyed by the source's content,
/// the grammar's version and the tape format): read from it where it has one, else parsed and stored.
/// The corpus runner's parse cache: a file parsed once is not parsed again by a later run.
pub fn build_cached(file: &dyn SourceFile, cache: &crate::content_store::ContentStore) -> Result<Built, String> {
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

/// [`build`] in a run's waves (decision 0104): no SQL is read or written. The tape is `found` (read before the
/// first wave), or one still queued by this process, else the file is parsed and its source and tape are handed to
/// the store's writer, behind the run (`named`: the run's manifest names the source it stores, decision 0306). `digest` is the source's
/// SHA-256, when the caller has it.
pub fn build_in_wave(
    file: &dyn SourceFile,
    cache: &crate::content_store::ContentStore,
    digest: Option<[u8; 32]>,
    found: Option<&crate::content_store::TapeFrame>,
    named: bool,
) -> Result<Built, String> {
    build_in_wave_with_tape(file, cache, digest, found, named, false).map(|(built, _)| built)
}

/// [`build_in_wave`], with the tape the facts were read off where `keep` (a program's matchers run over it, decision
/// 0106).
pub fn build_in_wave_with_tape(
    file: &dyn SourceFile,
    cache: &crate::content_store::ContentStore,
    digest: Option<[u8; 32]>,
    found: Option<&crate::content_store::TapeFrame>,
    named: bool,
    keep: bool,
) -> Result<(Built, Option<TapeDoc>), String> {
    use crate::code_rule_dsl::lines::Terminators;
    use crate::code_rule_dsl::tape::{self, TapeImage};
    use crate::code_rule_dsl::build_ids::grammar_version;
    use crate::content_store::HashKey;
    use sha2::Digest as _;
    let language = file_language(file).ok_or_else(|| format!("{} has no grammar", file.language()))?;
    let tag = i64::from(tape_tag(language).ok_or("the dialect has no tape tag")?);
    let (gv, fv) = (grammar_version(), i64::from(tape::formats::stored_version()));
    let bytes = file.source().as_bytes();
    let digest: [u8; 32] = match digest {
        Some(d) => d,
        None => {
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
            sha2::Sha256::digest(bytes).into()
        }
    };
    let key = queued_tape(&digest, tag, gv, fv);
    let grammar = tape::Grammar::embedded(language)?;
    let kept: Option<crate::content_store::ObjectBytes> = match found {
        // Stored: read in its segment, or (an older row) decompressed.
        Some(frame) => {
            let _decode = crate::code_rule_dsl::logic::profile::span(|| "tape decompress".into());
            Some(frame.bytes().map_err(|e| e.to_string())?)
        }
        // Still queued for the writer: read where it is (gathered, placed, or a compressed frame).
        None => cache.queued_tape(&key).transpose().map_err(|e| e.to_string())?,
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
            // The run's gather takes the source and tape as they are (no copy), placed in a segment of the store's
            // when it is full or the wave ends, and read there until written (decision 0295 step 5). No wave waits for
            // the writer (decision 0104).
            let canonical = HashKey::from_sha256_digest(digest);
            cache.queue_source_and_tape(key, canonical, tag, gv, fv, file.source(), tape, named).map_err(|e| e.to_string())?;
            image.primary
        }
    };
    let lines = Arc::new(LineTable::new(file.source(), Terminators::of(language)));
    let doc = kept_tape(keep, file, &grammar, &primary, &lines)?;
    let _facts = crate::code_rule_dsl::logic::profile::span(|| "facts from tape".into());
    let facts = read_off(doc.as_ref(), file, &grammar, primary, lines)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, facts.len() as u64);
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
    use crate::code_rule_dsl::tape::{self, TapeImage};
    use crate::code_rule_dsl::build_ids::grammar_version;
    use crate::content_store::HashKey;
    let language = file_language(file).ok_or_else(|| format!("{} has no grammar", file.language()))?;
    let tag = i64::from(tape_tag(language).ok_or("the dialect has no tape tag")?);
    let (gv, fv) = (grammar_version(), i64::from(tape::formats::stored_version()));
    let e = |e: crate::content_store::StoreError| e.to_string();
    let bytes = file.source().as_bytes();
    // A tape this process queued is read from memory (`build_in_wave`): reads see writes.
    {
        use sha2::Digest as _;
        let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
        if let Some(held) = cache.queued_tape(&queued_tape(&digest, tag, gv, fv)) {
            let held = held.map_err(|e| e.to_string())?;
            let grammar = tape::Grammar::embedded(language)?;
            let primary = TapeImage::from_bytes(&held, &grammar)?;
            let lines = Arc::new(LineTable::new(file.source(), Terminators::of(language)));
            let doc = kept_tape(keep, file, &grammar, &primary, &lines)?;
            return Ok((Built::Facts(read_off(doc.as_ref(), file, &grammar, primary, lines)?), known, doc));
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
    let grammar = tape::Grammar::embedded(language)?;
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
            // Queued as `build_in_wave` queues it: gathered in a run's waves, else compressed here and queued alone.
            let canonical = HashKey::from_sha256_digest(digest);
            cache.queue_source_and_tape(queued_tape(&digest, tag, gv, fv), canonical, tag, gv, fv, file.source(), tape, false).map_err(|e| e.to_string())?;
            image.primary
        }
    };
    let lines = Arc::new(LineTable::new(file.source(), Terminators::of(language)));
    let doc = kept_tape(keep, file, &grammar, &primary, &lines)?;
    let _facts = crate::code_rule_dsl::logic::profile::span(|| "facts from tape".into());
    let facts = read_off(doc.as_ref(), file, &grammar, primary, lines)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, facts.len() as u64);
    Ok((Built::Facts(facts), source_id, doc))
}


/// The facts of a restored document: its tape, shared (`TapeDoc::doc_tape`).
pub fn from_tape(doc: &TapeDoc, lines: Arc<LineTable>) -> Result<Facts, String> {
    let tag = tape_tag(doc.file_dialect()).ok_or("the tape's dialect has no tape tag")?;
    Ok(Facts {
        tag,
        source: Arc::clone(doc.document_source()),
        lines,
        tape: Some(Arc::clone(doc.doc_tape())),
        grammar: Some(Arc::clone(doc.grammar())),
        errors: Default::default(),
        held: Default::default(),
        python_syntax: Default::default(),
    })
}

/// The facts of an admitted image: checked against its source and grammar (the matcher's restore checks the same,
/// `tape::validated`), then held, moved in, and read in place.
pub fn from_image(source: Arc<String>, grammar: &Arc<crate::code_rule_dsl::tape::Grammar>,
    image: crate::code_rule_dsl::tape::TapeImage, lines: Arc<LineTable>) -> Result<Facts, String> {
    let tag = tape_tag(grammar.dialect()).ok_or("the tape's dialect has no tape tag")?;
    let check = crate::code_rule_dsl::logic::profile::span(|| "tape check".into());
    crate::code_rule_dsl::tape::validated::visit(&source, grammar, &image, &|| false, |_, _| {})?;
    drop(check);
    Ok(holding(Arc::new(crate::code_rule_dsl::tape::formats::DocTape::new(Arc::new(image), Arc::clone(&source))), source, grammar, tag, lines))
}

/// The facts of `tape` (checked), of `source` in `grammar`'s dialect.
fn holding(tape: Arc<crate::code_rule_dsl::tape::formats::DocTape>, source: Arc<String>, grammar: &Arc<crate::code_rule_dsl::tape::Grammar>, tag: u8, lines: Arc<LineTable>) -> Facts {
    Facts { tag, source, lines, tape: Some(tape), grammar: Some(Arc::clone(grammar)), errors: Default::default(), held: Default::default(), python_syntax: Default::default() }
}

/// The tape tag of a text-only document's facts (`text`): no dialect has it.
pub const TEXT: u8 = u8::MAX;

/// The facts of a file read as text only (core.rr's `.scope text`): its source and lines, no tree. Its lines are W's
/// for a file with no grammar (`Terminators::Eslint`: `\r\n`, `\r`, `\n`, U+2028, U+2029).
pub fn text(source: Arc<String>) -> Facts {
    let lines = Arc::new(LineTable::new(&source, crate::code_rule_dsl::lines::Terminators::Eslint));
    Facts { tag: TEXT, source, lines, tape: None, grammar: None, errors: std::sync::OnceLock::from(Vec::new()), held: Default::default(), python_syntax: Default::default() }
}

/// Counts a read of the facts' `$part` of node `$node` (E17, `tape::reads`); nothing without `tape-reads`.
macro_rules! watched {
    ($part:literal, $node:expr) => {
        #[cfg(feature = "tape-reads")]
        {
            const PART: usize = crate::code_rule_dsl::tape::reads::pattern_part($part);
            crate::code_rule_dsl::tape::reads::at(PART, $node);
        }
    };
}

/// `$body` over the facts' tape `$t`, monomorphised per format.
macro_rules! on_tape {
    ($f:expr, |$t:ident| $body:expr) => {
        rerail_tape_format::with_tape!($f.doc_tape().tape(), |$t| $body)
    };
}

use rerail_tape_format::tape::Tape as _;

impl Facts {
    fn doc_tape(&self) -> &crate::code_rule_dsl::tape::formats::DocTape {
        self.tape.as_deref().expect("a node read from facts with no tape (a text-only document has no nodes)")
    }
    /// The atom of a code's kind: ERROR's where it is one.
    fn atom(&self, e: &rerail_tape_format::tape::Entry) -> u32 {
        kind_atom(self.tag, if e.flags & rerail_tape_format::CODE_ERROR != 0 { ERROR_SYMBOL } else { e.kind })
    }
    #[cfg_attr(feature = "tape-reads", track_caller)]
    pub fn start_line(&self, id: u32) -> u32 {
        self.lines.line(self.start(id) as usize) as u32 + 1
    }
    pub fn len(&self) -> usize {
        self.tape.as_ref().map_or(0, |t| t.image().node_count())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The node's kind atom.
    pub fn kind(&self, id: u32) -> u32 {
        on_tape!(self, |t| self.atom(t.table().entry(t.code(id as usize))))
    }
    /// named | extra<<1 | missing<<2 | error<<3
    pub fn flags(&self, id: u32) -> u8 {
        use rerail_tape_format::{CODE_ERROR, CODE_EXTRA, CODE_MISSING};
        on_tape!(self, |t| {
            let e = t.table().entry(t.code(id as usize));
            let error = e.flags & CODE_ERROR != 0;
            let named = match &e.alias {
                Some(alias) => alias.1,
                None => self.grammar.as_ref().is_some_and(|g| g.kind_is_named(if error { ERROR_SYMBOL } else { e.kind })),
            };
            u8::from(named) | (u8::from(e.flags & CODE_EXTRA != 0) << 1) | (u8::from(e.flags & CODE_MISSING != 0) << 2) | (u8::from(error) << 3)
        })
    }
    /// The field (grammar field code, 0 = none) under which the node hangs from its parent.
    pub fn field(&self, id: u32) -> u16 {
        on_tape!(self, |t| t.table().entry(t.code(id as usize)).field)
    }
    #[cfg_attr(feature = "tape-reads", track_caller)]
    pub fn start(&self, id: u32) -> u32 {
        watched!("facts start", id);
        on_tape!(self, |t| t.start(id as usize) as u32)
    }
    #[cfg_attr(feature = "tape-reads", track_caller)]
    pub fn end(&self, id: u32) -> u32 {
        watched!("facts end", id);
        on_tape!(self, |t| t.end(id as usize) as u32)
    }
    /// The node's span, its start and end read together.
    pub fn span(&self, id: u32) -> (u32, u32) {
        watched!("facts start", id);
        watched!("facts end", id);
        on_tape!(self, |t| {
            let s = t.start(id as usize);
            (s as u32, t.end_with_start(id as usize, s) as u32)
        })
    }
    /// One past the node's subtree's last node.
    #[cfg_attr(feature = "tape-reads", track_caller)]
    pub fn subtree_end(&self, id: u32) -> u32 {
        watched!("facts subtree_end", id);
        on_tape!(self, |t| t.subtree_end(id as usize) as u32)
    }
    /// The tree's depth: nodes on the longest root-to-leaf path.
    pub fn depth(&self) -> u32 {
        if self.is_empty() {
            return 0;
        }
        on_tape!(self, |t| {
            let (mut open, mut deepest): (Vec<usize>, usize) = (Vec::new(), 0);
            for i in 0..t.len() {
                while open.last().is_some_and(|&end| end <= i) {
                    open.pop();
                }
                open.push(t.subtree_end(i));
                deepest = deepest.max(open.len());
            }
            deepest as u32
        })
    }
    /// The node's children, in order.
    pub fn children(&self, id: u32) -> Vec<u32> {
        on_tape!(self, |t| rerail_tape_format::tape::children(&t, id as usize).map(|c| c as u32).collect())
    }
    /// `id`'s children in order, each handed to `f`, read off the tape with no list made (a probe's).
    pub fn each_child(&self, id: u32, mut f: impl FnMut(u32)) {
        on_tape!(self, |t| rerail_tape_format::tape::children(&t, id as usize).for_each(|c| f(c as u32)))
    }
    /// Whether `id` has no children.
    pub fn is_leaf(&self, id: u32) -> bool {
        on_tape!(self, |t| t.subtree_end(id as usize) == id as usize + 1)
    }
    /// `id`'s anonymous children in source order: the tokens the tape stores (the grammar's implied
    /// ones are not, RER-131).
    pub fn tokens(&self, id: u32) -> Vec<Token> {
        self.children(id)
            .into_iter()
            .filter(|&c| !self.named(c))
            .map(|c| {
                let (start, end) = self.span(c);
                Token { kind: self.kind(c), field: self.field(c), start, end }
            })
            .collect()
    }
    /// A token's text.
    pub fn token_text(&self, token: &Token) -> &str {
        &self.source[token.start as usize..token.end as usize]
    }
    /// The nodes whose code is in the set `codes` makes of the tape's table, by id: a scan of the codes, 64 nodes at
    /// a time. The set is a kind's or a class's numbers from the table's index (`CodeTable::of_kind`), not a test of
    /// every entry: on format 16 the table is the language's (decision 0336 §7).
    fn matching(&self, codes: Codes) -> Vec<u32> {
        use rerail_tape_format::tape::CodeSet;
        if self.is_empty() {
            return Vec::new();
        }
        on_tape!(self, |t| {
            let table = t.table();
            // A kind's or a class's set is the table's own, borrowed (decision 0336 §7: on format 16 the table is the
            // language's, so a set made per document would cost a table's index per probe).
            let made;
            let set: &CodeSet = match codes {
                Codes::Kind(code) if code == ERROR_SYMBOL => {
                    let numbers = |s: &CodeSet| s.numbers().unwrap_or_default().to_vec();
                    made = CodeSet::of_numbers([table.of_kind(code), table.errors(), table.of_kind(0)].into_iter().flat_map(numbers));
                    &made
                }
                Codes::Kind(code) => table.of_kind(code),
                Codes::Class(code) => table.of_class(code),
                Codes::Faults => table.faults(),
            };
            if set.is_empty() {
                return Vec::new();
            }
            let held = self.held.get_or_init(|| {
                let mut codes: Vec<u32> = (0..t.len()).map(|i| t.code(i)).collect();
                codes.sort_unstable();
                codes.dedup();
                let (mut kinds, mut classes) = (Vec::new(), Vec::new());
                for &c in &codes {
                    let e = table.entry(c);
                    if e.flags & rerail_tape_format::CODE_ERROR == 0 {
                        kinds.push(e.kind);
                    }
                    classes.extend_from_slice(&e.classes);
                }
                for v in [&mut kinds, &mut classes] {
                    v.sort_unstable();
                    v.dedup();
                }
                Held { codes: codes.into_boxed_slice(), kinds: kinds.into_boxed_slice(), classes: classes.into_boxed_slice() }
            });
            let meets = match codes {
                Codes::Kind(code) if code != ERROR_SYMBOL => held.kinds.binary_search(&code).is_ok(),
                Codes::Class(code) => held.classes.binary_search(&code).is_ok(),
                // From the smaller side: the set's numbers searched among the held, or each held code tested.
                _ => match set.numbers() {
                    Some(numbers) if numbers.len() < held.codes.len() => numbers.iter().any(|n| held.codes.binary_search(n).is_ok()),
                    _ => held.codes.iter().any(|&c| set.contains(c)),
                },
            };
            if !meets {
                return Vec::new();
            }
            let mut out = Vec::new();
            t.match_groups(set, |base, mut m| {
                while m != 0 {
                    out.push((base + m.trailing_zeros() as usize) as u32);
                    m &= m - 1;
                }
            });
            out
        })
    }
    /// The nodes of kind `kind` (an atom), by id.
    pub fn of_kind(&self, kind: u32) -> Vec<u32> {
        if kind_tag(kind) != self.tag {
            return Vec::new();
        }
        // `atom`'s inverse: the codes of the kind with no ERROR flag; for slot 0 (ERROR's atom, which kind 0's shares)
        // also every ERROR code and kind 0's.
        self.matching(Codes::Kind(kind_code(kind)))
    }
    /// Nodes of the abstract class `class` (a supertype's kind atom), by id.
    pub fn of_class(&self, class: u32) -> Vec<u32> {
        if kind_tag(class) != self.tag {
            return Vec::new();
        }
        self.matching(Codes::Class(kind_code(class)))
    }
    /// Each kind atom the document holds and its nodes, and each class atom and its nodes, each sorted by atom.
    pub fn histogram(&self) -> (Vec<(u32, u32)>, Vec<(u32, u32)>) {
        if self.is_empty() {
            return (Vec::new(), Vec::new());
        }
        on_tape!(self, |t| {
            // Counted by the codes the document holds, not a counter per code of the table: on format 16 the table is
            // the language's (decision 0336 §7).
            let mut count: rustc_hash::FxHashMap<u32, u32> = rustc_hash::FxHashMap::default();
            for i in 0..t.len() {
                *count.entry(t.code(i)).or_insert(0) += 1;
            }
            let (mut kinds, mut classes) = (std::collections::BTreeMap::new(), std::collections::BTreeMap::new());
            for (&code, &n) in &count {
                let e = t.table().entry(code);
                *kinds.entry(self.atom(e)).or_insert(0u32) += n;
                for &class in &e.classes {
                    *classes.entry(kind_atom(self.tag, class)).or_insert(0u32) += n;
                }
            }
            (kinds.into_iter().collect(), classes.into_iter().collect())
        })
    }
    /// The kind atoms the document holds, ascending.
    pub fn kinds(&self) -> Vec<u32> {
        self.histogram().0.into_iter().map(|(k, _)| k).collect()
    }
    /// The ERROR and MISSING nodes, ascending.
    pub fn errors(&self) -> &[u32] {
        self.errors.get_or_init(|| self.matching(Codes::Faults))
    }
    /// Whether a node is an ERROR or MISSING.
    pub fn syntax_errors(&self) -> bool {
        !self.errors().is_empty()
    }
    /// Whether `id` or a node above it is an ERROR or MISSING.
    pub fn under_error(&self, id: u32) -> bool {
        self.errors().iter().take_while(|&&e| e <= id).any(|&e| id < self.subtree_end(e))
    }
    /// Whether `id`, a node above it or a node under it is an ERROR or MISSING.
    pub fn touches_error(&self, id: u32) -> bool {
        let end = self.subtree_end(id);
        self.errors().iter().any(|&e| (id..end).contains(&e)) || self.under_error(id)
    }
    #[cfg_attr(feature = "tape-reads", track_caller)]
    pub fn text(&self, id: u32) -> &str {
        let (start, end) = self.span(id);
        &self.source[start as usize..end as usize]
    }
    pub fn named(&self, id: u32) -> bool {
        self.flags(id) & 1 != 0
    }
    #[cfg_attr(feature = "tape-reads", track_caller)]
    pub fn parent(&self, id: u32) -> Option<u32> {
        watched!("facts parent", id);
        on_tape!(self, |t| t.parent(id as usize).map(|p| p as u32))
    }
    /// The nodes `a..b` in preorder, each with its parent: from the subtrees open around it, read in the same pass,
    /// where its parent is in the range; else read off the tape (on format 15, a search back). So a walk over a
    /// subtree reads one parent off the tape, not one a node.
    pub fn with_parents(&self, a: u32, b: u32) -> impl Iterator<Item = (u32, Option<u32>)> + '_ {
        let mut open: Vec<(u32, u32)> = Vec::new();
        (a..b).map(move |id| {
            while open.last().is_some_and(|&(_, end)| end <= id) {
                open.pop();
            }
            let parent = match open.last() {
                Some(&(p, _)) => Some(p),
                None => self.parent(id),
            };
            open.push((id, self.subtree_end(id)));
            (id, parent)
        })
    }
    /// The node's parent, `NONE` for the root.
    /// `id`'s parent, `a` an ancestor of it the caller knows (`Tape::parent_within`: on format 15 the search starts
    /// there).
    pub fn parent_within(&self, id: u32, a: u32) -> Option<u32> {
        on_tape!(self, |t| t.parent_within(id as usize, a as usize).map(|p| p as u32))
    }
    pub fn parent_or_none(&self, id: u32) -> u32 {
        self.parent(id).unwrap_or(NONE)
    }
    /// Resident bytes the facts hold of their own: none past the struct, as they copy nothing off the tape.
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
    }
    /// The resident bytes of the tape they read.
    pub fn tape_bytes(&self) -> usize {
        self.tape.as_ref().map_or(0, |t| t.bytes())
    }
}

/// The digest of a file's core facts: the source their spans read text from and the tape they read, as stored, so
/// equal digests mean equal facts (the core derivation's output, `families`). Cutoff on an edit happens in the
/// narrower families that read these, not here.
pub fn digest(f: &Facts) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"rerail.core-facts/v2\0");
    hash.update((f.source.len() as u64).to_le_bytes());
    hash.update(f.source.as_bytes());
    let tape = f.tape.as_ref().and_then(|t| t.image().to_bytes().ok()).unwrap_or_default();
    hash.update((tape.len() as u64).to_le_bytes());
    hash.update(&tape);
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, (f.source.len() + tape.len()) as u64);
    hash.finalize().into()
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)

/// The relations a program in `core.rr`'s shape declares (the engine's names, decision 0141, less those decision
/// 0174's rewrite adds), as the rule language reads them; registered at load (`install_core_names`). Empty without one.
pub fn core_names() -> &'static std::collections::BTreeSet<String> {
    static EMPTY: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    CORE_NAMES.get().map_or(&EMPTY, |names| names())
}

static CORE_NAMES: std::sync::OnceLock<fn() -> &'static std::collections::BTreeSet<String>> = std::sync::OnceLock::new();

/// Registers what reads `core.rr`'s names (once, at load).
pub fn install_core_names(names: fn() -> &'static std::collections::BTreeSet<String>) {
    let _ = CORE_NAMES.set(names);
}

/// A column's name as the planner and SQL read it: a positional name (`c1`, as the engine names a derived
/// relation's rows) of a relation named like a base one (`input_columns`) is that column's base name. Such a
/// relation is derived where an attribute is answered on request (`value`, `node_text`: decision 0174), and handed
/// on from the chunks it is the cross-file part's input, read as `value(n, v)`.
pub fn input_column_name(rel: &str, name: &str) -> Option<&'static str> {
    let i: usize = name.strip_prefix('c')?.parse().ok()?;
    input_columns(rel)?.get(i).copied()
}

/// The columns of the planner's base relations (T's input schema). Others are `c0`, `c1`, ….
pub fn input_columns(rel: &str) -> Option<&'static [&'static str]> {
    Some(match rel {
        "node" => &["d", "n", "kind", "start", "end_"],
        "line" => &["n", "l"],
        "doc" => &["d", "path", "module"],
        "child" => &["p", "c", "idx"],
        "field" => &["p", "f", "c"],
        "text" | "name" | "str" | "strprefix" | "node_text" | "comment" | "structural_before" | "structural_after" => &["n", "s"],
        // W's values (`facts::vtab::syntax::Extra`).
        "value" | "static_value" | "rr_value_lit" => &["n", "v"],
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
        "spread" | "rr_long_text_node" => &["n"],
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
        "end_point" => &["n", "p"],
        "tree_digest" | "def_digest" => &["n", "s"],
        // core.rr's tokens (`facts::vtab::syntax::tree`).
        "prev_token" => &["n", "s"],
        "token" | "rr_token_anon" => &["p", "i", "s", "start", "end_"],
        "rr_token_named" => &["p", "i", "c", "start", "end_"],
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
        rel if crate::code_rule_dsl::logic::front::ast::is_transduced(rel) => &["n", "a"],
        rel if rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) => &["s"],
        // An `ast` matcher's matched nodes (decision 0106).
        rel if rel.starts_with(ast_matches::PREFIX) => ast_matches::columns(rel),
        // A `match_all` regex's matches (`logic::strings::as_relations`, `facts::vtab::syntax::Extra`).
        rel if rel.starts_with(crate::code_rule_dsl::logic::strings::MATCHES) => &["sp", "m"],
        rel if rel.starts_with(crate::code_rule_dsl::logic::strings::GROUPS) => &["sp", "m", "g", "gsp"],
        // A registered family's lazy relations (`facts::vtab::syntax::Families`: Python's `py.*`).
        rel if rel.starts_with("py_") => crate::code_rule_dsl::facts::vtab::syntax::family_columns(rel)?,
        _ => return None,
    })
}

/// The executor of a binding (the run's lanes: every lane under the device binding, the cores' otherwise), registered
/// at load (`install_executors`); without one, a fresh executor of the machine's lanes, made only then (making it
/// compiles the GPU's kernels and builds the Neural Engine's lane).
pub fn executor_for(device: bool) -> std::sync::Arc<rerail_accel::Executor> {
    /// The machine's executor where none is registered (decision 0442: the machine's).
    #[derive(Default)]
    struct Own(std::sync::OnceLock<std::sync::Arc<rerail_accel::Executor>>);
    match EXECUTORS.get() {
        Some(executor) => executor(device),
        None => std::sync::Arc::clone(crate::code_rule_dsl::logic::profile::machine::<Own>().0.get_or_init(|| std::sync::Arc::new(rerail_accel::Executor::new()))),
    }
}

static EXECUTORS: std::sync::OnceLock<fn(bool) -> std::sync::Arc<rerail_accel::Executor>> = std::sync::OnceLock::new();

/// Registers the run's executors (once, at load).
pub fn install_executors(executor: fn(bool) -> std::sync::Arc<rerail_accel::Executor>) {
    let _ = EXECUTORS.set(executor);
}

/// The relations CPython's facts bind (the corpus's names for the verdict, `ast.unparse`, `ast.dump` and their
/// hashes) and their columns: base relations the planner reads by name.
pub mod cpython_relations {
    /// The relation the corpus binds for CPython's verdict (core.rr's `syntax_error(d, l, msg)`).
    pub const SYNTAX_ERROR: &str = "cpython_syntax_error";
    /// W's `literal_json` (`cpython::literal`).
    pub const LITERAL_JSON: &str = "literal_json";
    /// core.rr's `@py_unparse(n)` (`cpython::unparse`).
    pub const UNPARSE: &str = "cpython_unparse";
    /// core.rr's `@ast_dump(n)` (`cpython::dump`).
    pub const AST_DUMP: &str = "cpython_ast_dump";
    /// W's `dump_sha256`, `body_sha256` and `signature_sha256` (`cpython::dump`).
    pub const DUMP_SHA256: &str = "dump_sha256";
    pub const BODY_SHA256: &str = "body_sha256";
    pub const SIGNATURE_SHA256: &str = "signature_sha256";

    /// A printed relation's columns.
    pub fn columns(rel: &str) -> &'static [&'static str] {
        match rel {
            UNPARSE | AST_DUMP => &["n", "s"],
            DUMP_SHA256 => &["n", "h"],
            _ => &["f", "h"],
        }
    }
}

/// The relations a program's `ast` atoms bind (`rr_ast_<k>`, `rr_ast_<k>_<m>`, decision 0107) and their columns.
pub mod ast_matches {
    /// The prefix of a matcher's relation.
    pub const PREFIX: &str = "rr_ast_";

    /// A matcher relation's columns: the root `n`, then one a capture.
    pub(crate) const COLUMNS: [&str; 16] = ["n", "x1", "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13", "x14", "x15"];

    /// The most captures an atom may name.
    pub const MOST_CAPTURES: usize = COLUMNS.len() - 1;

    /// The columns of matcher relation `rel` (`rr_ast_<k>` or `rr_ast_<k>_<m>`).
    pub fn columns(rel: &str) -> &'static [&'static str] {
        let width = rel.strip_prefix(PREFIX).and_then(|rest| rest.split_once('_')).and_then(|(_, m)| m.parse::<usize>().ok()).unwrap_or(0);
        &COLUMNS[..=width.min(MOST_CAPTURES)]
    }
}
/// Python's execution contexts, by `ContextKind` order: `py_exec_context.kind`'s names, which a rule names.
pub const PYTHON_CONTEXTS: [&str; 5] = ["module", "sync_function", "async_function", "class", "lambda"];
