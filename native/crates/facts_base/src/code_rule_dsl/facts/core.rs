//! Core node facts from the tape (docs/final-shape/03-architecture.md §1, §5; 05 Phase 2), adapted
//! from facts-a's per-content syntactic facts (the facts-engine branch, `spikes/facts-a/src/facts.rs`):
//! columns indexed by preorder node id, built from Rerail's own tape so ids, kinds, fields and
//! spans equal the matcher's (preorder transducer ids over every tree-sitter node, named and
//! anonymous).
//!
//! facts-a parsed Python only and interned kind names in a process-wide table. Here any dialect's
//! tape is read, and a kind is the atom `tape tag · 1024 + kind code`, so kinds of different
//! dialects never collide and no table is needed. The abstract classes the tape records
//! (Tree-sitter supertypes, tape v8) get postings beside the concrete kinds.

use rerail_blocks::vector::Vector;
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

/// The columns of a registered family's lazy relation (`facts::vtab::syntax::family_columns`, rerail-facts'), which
/// the boot registers with the families; none registered: no family relation.
pub static FAMILY_COLUMNS: rerail_registry::Slot<fn(&str) -> Option<&'static [&'static str]>> =
    rerail_registry::Slot::new("a family's relation's columns (facts::core::FAMILY_COLUMNS)");

static VALUE_READER: std::sync::OnceLock<&'static dyn ValueReader> = std::sync::OnceLock::new();

/// Registers the reader of values (once, at load).
pub fn install_value_reader(reader: &'static dyn ValueReader) {
    let _ = VALUE_READER.set(reader);
}

/// The registered reader of values; `None` where no engine registered one.
pub fn value_reader() -> Option<&'static dyn ValueReader> {
    VALUE_READER.get().copied()
}

/// The dialect of `file`'s label.
fn file_language(file: &dyn SourceFile) -> Option<crate::code_rule_dsl::language::GrammarDialect> {
    crate::code_rule_dsl::language::dialect_by_label(file.language())
}

/// The tape tag of the grammar labelled `label` (`build_in_wave_with_tape`'s key's), if it has one.
pub fn label_tag(label: &str) -> Option<i64> {
    crate::code_rule_dsl::language::dialect_by_label(label).and_then(tape_tag).map(i64::from)
}

/// Whether this process has queued the tape of a text whose SHA-256 is `digest` in the grammar `file` names, for the
/// store (`build_in_wave_with_tape` then reads it, not parses it): decision 0431 R6's count of the texts parsed.
pub fn tape_queued(cache: &crate::content_store::ContentStore, file: &dyn SourceFile, digest: &[u8; 32], fragment: Option<Fragment>) -> bool {
    use crate::code_rule_dsl::build_ids::grammar_version;
    let tag = match fragment {
        Some(f) => Some(i64::from(f.tag)),
        None => file_language(file).and_then(tape_tag).map(i64::from),
    };
    let Some(tag) = tag else { return false };
    let fv = i64::from(crate::code_rule_dsl::tape::formats::stored_version());
    cache.queued_tape(&queued_tape(digest, tag, grammar_version(), fv)).is_some()
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
    errors: std::sync::OnceLock<Vector<u32>>,
    /// Its index's bytes (decision 0656 step 4, `DocIndex`), built on the first read that needs it where its tape's
    /// home holds none (a format 15 tape, one stored before layout 8); a home's own is read where it lies
    /// (`FactsIn::index`).
    index: std::sync::OnceLock<Vec<u8>>,
    /// Computed only for a demanded Python syntax verdict; shared by every projection of this tape. `Err`: `ast.parse`
    /// raises another exception than `SyntaxError` (`facts::vtab::syntax::python_syntax`).
    pub python_syntax: std::sync::OnceLock<Result<Option<(u32, String)>, String>>,
    /// The file's programs (decision 0431 §7, step 6): each embedding its dialect declares, made at its parse or read
    /// with its stored tape, and kept with it. `None` where they are not known (a tape stored before step 6, a
    /// program's own facts): the batch makes them from the tape.
    pub programs: Option<Arc<[crate::code_rule_dsl::tape::DerivedImage]>>,
    /// Whether the document's syntax verdict is its tape's (its ERROR and MISSING nodes), not its language's own
    /// check of a file: a program no file of its language is, as a rule's pattern is (decision 0495 §3,
    /// `embed::Embedder::tape_verdict`).
    pub tape_verdict: bool,
}

/// An anonymous child: a token the tape stores ([`Facts::tokens`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Token {
    pub kind: u32,
    pub field: u16,
    pub start: u32,
    pub end: u32,
}

#[allow(clippy::large_enum_variant)]
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

/// The files a run parsed (`record_parse`): the run's own count (decision 0442), always kept, for the tests that
/// hold a warm run to parsing only what changed (Q332, `warm_tests`).
#[derive(Default)]
pub struct Parses(parking_lot::Mutex<std::collections::BTreeMap<(&'static str, String), usize>>);

/// The files the run under way parsed, by the path that parsed them (`direct`: `build_with_tape`; `wave`, `cached`:
/// a wave's build that found no tape) and their language.
pub fn parses() -> std::collections::BTreeMap<(&'static str, String), usize> {
    crate::code_rule_dsl::logic::profile::local::<Parses>().0.lock().clone()
}

fn record_parse(file: &dyn SourceFile, reader: &'static str) {
    use crate::code_rule_dsl::prof;
    *crate::code_rule_dsl::logic::profile::local::<Parses>().0.lock().entry((reader, file.language().to_owned())).or_default() += 1;
    // A measurement aid: `RERAIL_PARSE_TRACE` prints where each parse is asked for.
    if std::env::var_os("RERAIL_PARSE_TRACE").is_some() {
        eprintln!("RERAIL_PARSE_TRACE {reader} {} (inside waves: {}):\n{}", file.language(), crate::code_rule_dsl::logic::profile::inside_waves(), std::backtrace::Backtrace::force_capture());
    }
    prof::count("source_tape.parses", 1);
    if prof::on() {
        use sha2::{Digest as _, Sha256};
        let digest = Sha256::digest(file.source().as_bytes()).into();
        prof::count_named(format!("source_tape.input.{reader}.{}.{}", file.language(), crate::content_store::observe::hex(&digest)), 1);
    }
}

/// [`build`], with the tape the facts were read off where `keep` (decision 0106).
pub fn build_with_tape(file: &dyn SourceFile, keep: bool) -> Result<(Built, Option<TapeDoc>), String> {
    use crate::code_rule_dsl::{lines::Terminators, tape::{Grammar, TapeImage}};
    let language = file_language(file).ok_or_else(|| format!("{} has no grammar", file.language()))?;
    let grammar = Grammar::embedded(language)?;
    let parse = crate::code_rule_dsl::logic::profile::span(|| "tape parse".into());
    record_parse(file, "direct");
    let Some(image) = TapeImage::parse(&grammar, file.source(), &|| false) else { return Ok((Built::Unparsed, None)); };
    let image = image?;
    drop(parse);
    let (built, doc) = facts_of_image(file, &grammar, image.primary, crate::code_rule_dsl::lines::terminators(language), keep)?;
    Ok((with_programs(built, &grammar, image.derived)?, doc))
}

/// `built`'s facts with the file's programs (decision 0431 §7, step 6): `derived`, those its parse made over the tape
/// (`FileImage::embedding`), and those of each embedding that reads the scope (`call-argument`, decision 0470), made
/// over a document over the facts' own tape, read in place. The batch reads them off the facts.
fn with_programs(built: Built, grammar: &Arc<crate::code_rule_dsl::tape::Grammar>, derived: Vec<crate::code_rule_dsl::tape::DerivedImage>) -> Result<Built, String> {
    let Built::Facts(mut facts) = built else { return Ok(built) };
    let mut programs = derived;
    let mut vias: Vec<crate::code_rule_dsl::language::Via> = Vec::new();
    for &(via, _) in grammar.lang.embeds() {
        if vias.contains(&via) {
            continue;
        }
        vias.push(via);
        let embedder = crate::code_rule_dsl::embed::embedder(via);
        if !embedder.reads_scope() {
            continue;
        }
        let Some(tape) = facts.tape.as_ref() else { continue };
        // The host read in a batch of its own: its 16z blocks decoded into a cache made for it.
        let doc = TapeDoc::over(Arc::clone(tape), Arc::clone(grammar), Arc::clone(&facts.lines)).alone();
        for program in embedder.build_in_doc(&doc, &|| false) {
            let image = crate::code_rule_dsl::tape::DerivedImage::encode(via, program, 0, grammar, &facts.source, &|| false)?;
            programs.push(image);
        }
    }
    facts.programs = Some(programs.into());
    Ok(Built::Facts(facts))
}

/// A fragment's parse (decision 0466): the node kind it starts at, and the tag its tapes are stored under (never its
/// dialect's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fragment {
    pub start: &'static str,
    pub tag: u8,
}

impl Fragment {
    /// The fragment the registry names `name`.
    pub fn named(name: &str) -> Option<Self> {
        let (_, start) = crate::code_rule_dsl::language::fragment(name)?;
        Some(Fragment { start, tag: crate::code_rule_dsl::language::fragment_tag(name)? })
    }
}

/// [`build`] of `file` as a fragment (decision 0466): parsed from its start by its dialect's hand-written parser, its
/// tape's root one node of that kind. `Unparsed` where the parser refuses the text.
pub fn build_fragment(file: &dyn SourceFile, fragment: Fragment) -> Result<Built, String> {
    use crate::code_rule_dsl::{lines::Terminators, tape::{Grammar, TapeImage}};
    let language = file_language(file).ok_or_else(|| format!("{} has no grammar", file.language()))?;
    let grammar = Grammar::embedded(language)?;
    let Some(image) = TapeImage::parse_from(&grammar, file.source(), fragment.start, &|| false) else { return Ok(Built::Unparsed) };
    facts_of_image(file, &grammar, image?.primary, crate::code_rule_dsl::lines::terminators(language), false).map(|(built, _)| built)
}

/// [`build_with_tape`] of an embedded program (decision 0431 §4), read off the tape or tree its embedder parsed
/// (`embed::Parse`), not parsed again.
/// A tree's placeholders `segments` omits are left out as it is encoded; a lean parser's tape left them out as it
/// was written (`Parse::read_program`).
pub fn build_parsed(file: &dyn SourceFile, parse: crate::code_rule_dsl::embed::Parse, segments: &crate::code_rule_dsl::embed::segments::SegmentMap, keep: bool) -> Result<(Built, Option<TapeDoc>), String> {
    use crate::code_rule_dsl::{lines::Terminators, tape::{Grammar, TapeImage}};
    let language = file_language(file).ok_or_else(|| format!("{} has no grammar", file.language()))?;
    let grammar = Grammar::embedded(language)?;
    let primary = match parse {
        crate::code_rule_dsl::embed::Parse::Tape(image) => image,
        crate::code_rule_dsl::embed::Parse::Tree(tree) => TapeImage::encode_tree_image(tree.root_node(), &grammar, file.source(), Some(segments), &|| false)?,
    };
    facts_of_image(file, &grammar, primary, crate::code_rule_dsl::lines::terminators(language), keep)
}

/// The facts of `file` read off its parsed tape `primary`, with the tape where `keep`.
fn facts_of_image(file: &dyn SourceFile, grammar: &Arc<crate::code_rule_dsl::tape::Grammar>, primary: crate::code_rule_dsl::tape::TapeImage, terminators: crate::code_rule_dsl::lines::Terminators, keep: bool) -> Result<(Built, Option<TapeDoc>), String> {
    // Where the device reads tapes, the parse's in a slab's room too (decision 0353 step 4a): a guest's, a run's
    // without a store.
    let primary = placed_where_read(primary, grammar)?;
    let lines = Arc::new(primary.lines(file.source(), terminators, true));
    let doc = kept_tape(keep, file, grammar, &primary, &lines)?;
    let _facts = crate::code_rule_dsl::logic::profile::span(|| "facts from tape".into());
    let facts = read_off(doc.as_ref(), file, grammar, primary, lines)?;
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

/// A parsed tape's stored bytes in their home and the image read there (decision 0353 step 4a): written straight
/// into a slab's room, which the store's writer and every reader take. A host whose tape is not placed (no device,
/// or the pool refused the room) keeps the parser's image, and the writer its bytes.
/// A tape still queued for the writer, read where it is: the room this process placed it in (decision 0353 step 4a),
/// else its bytes read into a tape of its own.
// copy (0353 step 4a): a queued tape not in a room (a frame of a gathered segment, decompressed) is read into a vector.
fn queued_image(stored: &crate::content_store::ObjectBytes, grammar: &Arc<crate::code_rule_dsl::tape::Grammar>) -> Result<crate::code_rule_dsl::tape::TapeImage, String> {
    use crate::code_rule_dsl::tape::TapeImage;
    if let crate::content_store::ObjectBytes::Tape(held) = stored
        && let Ok(home) = (Arc::clone(held) as Arc<dyn std::any::Any + Send + Sync>).downcast::<rerail_tape_format::home::Home>()
        && home.placed()
    {
        return TapeImage::from_home((*home).clone(), grammar);
    }
    crate::code_rule_dsl::prof::count("tape.queued_read_apart", 1);
    const APART: rerail_prof::copies::Site = rerail_prof::copies::Site::new(17, "4a", "a queued tape not in a room, read into a vector (queued_image)");
    APART.time(|| (TapeImage::from_bytes(stored, grammar), stored.len()))
}

/// A parse's tape in a slab's room where a device reads tapes (`placed_parse`), else as the parser made it.
fn placed_where_read(image: crate::code_rule_dsl::tape::TapeImage, grammar: &Arc<crate::code_rule_dsl::tape::Grammar>) -> Result<crate::code_rule_dsl::tape::TapeImage, String> {
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    if crate::code_rule_dsl::tape::placed::device().is_some() {
        return placed_parse(image, grammar).map(|(_, image)| image);
    }
    let _ = grammar;
    Ok(image)
}

fn placed_parse(image: crate::code_rule_dsl::tape::TapeImage, grammar: &Arc<crate::code_rule_dsl::tape::Grammar>) -> Result<(crate::content_store::TapeBytes, crate::code_rule_dsl::tape::TapeImage), String> {
    use crate::code_rule_dsl::tape::TapeImage;
    let home = image.place_stored()?;
    if !home.placed() {
        return Ok((Arc::new(home), image));
    }
    let mut primary = TapeImage::from_home(home.clone(), grammar)?;
    primary.source_digest = image.source_digest;
    Ok((Arc::new(home), primary))
}

/// Restore a tape already built by the source phase, including tapes awaiting publication.
pub fn restore_tape(file: &dyn SourceFile, cache: &crate::content_store::ContentStore, digest: [u8; 32]) -> crate::content_store::StoreResult<Option<TapeDoc>> {
    use crate::code_rule_dsl::{lines::Terminators, tape::{Grammar, TapeImage, formats}};
    use crate::content_store::{HashKey, StoreError};
    let language = file_language(file).ok_or_else(|| StoreError::Invalid(format!("{} has no grammar", file.language())))?;
    let tag = i64::from(tape_tag(language).ok_or_else(|| StoreError::Invalid("the dialect has no tape tag".into()))?);
    let grammar_version = crate::code_rule_dsl::build_ids::grammar_version();
    let format = i64::from(formats::stored_version());
    let bytes = match cache.queued_tape(&queued_tape(&digest, tag, grammar_version, format)) {
        Some(bytes) => bytes?,
        None => {
            let Some(source) = cache.source_by_hash(HashKey::from_sha256_digest(digest))? else { return Ok(None) };
            let Some(frame) = cache.tape_frames_for(&[source], tag, grammar_version, format)?.into_iter().next().flatten()
                else { return Ok(None) };
            frame.bytes()?
        }
    };
    let grammar = Arc::new(Grammar::new(language).map_err(StoreError::Invalid)?);
    let primary = TapeImage::from_bytes(&bytes, &grammar).map_err(StoreError::Invalid)?;
    let lines = Arc::new(primary.lines(file.source(), crate::code_rule_dsl::lines::terminators(language), false));
    kept_tape(true, file, &grammar, &primary, &lines).map_err(StoreError::Invalid)
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
    build_in_wave_as(file, None, Some(cache), digest, found, named, keep)
}

/// [`build_in_wave_with_tape`] from the stored tape `found` alone, with no store: a document made when first read
/// (`syntax::Pending`, Q332), whose tape frame was read with the run's and is held, as the store need not be.
pub fn build_with_frame(file: &dyn SourceFile, found: &crate::content_store::TapeFrame, keep: bool) -> Result<(Built, Option<TapeDoc>), String> {
    build_in_wave_as(file, None, None, None, Some(found), true, keep)
}

/// [`build_in_wave`] of `file` as a fragment (decision 0466): its tape stored and found under the fragment's tag.
pub fn build_fragment_in_wave(
    file: &dyn SourceFile,
    fragment: Fragment,
    cache: &crate::content_store::ContentStore,
    digest: Option<[u8; 32]>,
    found: Option<&crate::content_store::TapeFrame>,
    named: bool,
) -> Result<Built, String> {
    build_in_wave_as(file, Some(fragment), Some(cache), digest, found, named, false).map(|(built, _)| built)
}

/// [`build_in_wave_with_tape`], of a whole file or a fragment. With no store (`cache`), a tape not `found` is parsed
/// and not queued.
fn build_in_wave_as(
    file: &dyn SourceFile,
    fragment: Option<Fragment>,
    cache: Option<&crate::content_store::ContentStore>,
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
    let tag = match fragment {
        Some(f) => i64::from(f.tag),
        None => i64::from(tape_tag(language).ok_or("the dialect has no tape tag")?),
    };
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
    // The programs the parse made (decision 0431 step 6); a whole file's only, not a fragment's.
    let mut derived: Option<Vec<tape::DerivedImage>> = None;
    // A parsed tape, queued for the store once its programs are known, with them.
    let mut queue: Option<(HashKey, crate::content_store::TapeBytes)> = None;
    // Stored: decompressed (or read from its segment) straight into its one home, a slab's room, and read there
    // (decision 0353 step 4a).
    let placed = match found {
        Some(frame) => {
            let _decode = crate::code_rule_dsl::logic::profile::span(|| "tape decompress".into());
            let len = frame.raw_len().map_err(|e| e.to_string())?;
            Some(tape::placed::place(len, |out| frame.copy_into(out)).map_err(|e| e.to_string())?)
        }
        None => None,
    };
    // Still queued for the writer: read where it is (gathered, placed, or a compressed frame).
    let kept: Option<crate::content_store::ObjectBytes> = match placed {
        Some(_) => None,
        None => match cache {
            Some(cache) => cache.queued_tape(&key).transpose().map_err(|e| e.to_string())?,
            None => None,
        },
    };
    let primary = match (placed, kept) {
        (Some(home), _) => {
            crate::code_rule_dsl::logic::profile::decision("input tape cache", || "hit".into());
            let _decode = crate::code_rule_dsl::logic::profile::span(|| "tape decode".into());
            TapeImage::from_home(home, &grammar)?
        }
        (None, Some(stored)) => {
            crate::code_rule_dsl::logic::profile::decision("input tape cache", || "hit".into());
            let _decode = crate::code_rule_dsl::logic::profile::span(|| "tape decode".into());
            queued_image(&stored, &grammar)?
        }
        (None, None) => {
            crate::code_rule_dsl::logic::profile::decision("input tape cache", || "miss".into());
            record_parse(file, "wave");
            let parse = crate::code_rule_dsl::logic::profile::span(|| "tape parse".into());
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, file.source().len() as u64);
            let parsed = match fragment {
                Some(f) => TapeImage::parse_from(&grammar, file.source(), f.start, &|| false),
                None => TapeImage::parse(&grammar, file.source(), &|| false),
            };
            let Some(image) = parsed else {
                return Ok((Built::Unparsed, None));
            };
            let image = image?;
            drop(parse);
            derived = Some(image.derived);
            let (tape, primary) = placed_parse(image.primary, &grammar)?;
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, (*tape).as_ref().len() as u64);
            // The run's gather takes the source and tape as they are (no copy), placed in a segment of the store's
            // when it is full or the wave ends, and read there until written (decision 0295 step 5). No wave waits for
            // the writer (decision 0104).
            queue = Some((HashKey::from_sha256_digest(digest), tape));
            primary
        }
    };
    let lines = Arc::new(primary.lines(file.source(), crate::code_rule_dsl::lines::terminators(language), true));
    let doc = kept_tape(keep, file, &grammar, &primary, &lines)?;
    let _facts = crate::code_rule_dsl::logic::profile::span(|| "facts from tape".into());
    let facts = read_off(doc.as_ref(), file, &grammar, primary, lines)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, facts.len() as u64);
    let built = match derived.filter(|_| fragment.is_none()) {
        Some(derived) => with_programs(Built::Facts(facts), &grammar, derived)?,
        // Found: the programs recorded with its tape, where they are known.
        None => {
            let mut facts = facts;
            if fragment.is_none() {
                facts.programs = found.and_then(crate::content_store::TapeFrame::programs).and_then(|stored| stored_programs(stored, language, file.source().len()));
            }
            Built::Facts(facts)
        }
    };
    // With no store (`build_with_frame`), a tape parsed is not queued.
    if let (Some(cache), Some((canonical, tape))) = (cache, queue) {
        cache.queue_source_and_tape(key, canonical, tag, gv, fv, file.source(), tape, named, queued_programs(&built)?).map_err(|e| e.to_string())?;
    }
    Ok((built, doc))
}

/// The programs `stored` recorded with a file's tape (decision 0431 step 6), each tape placed in a room of its own as
/// the file's is (decision 0353 step 4a) and checked against the file (`parent`, `parent_len` bytes); `None` where one
/// does not check: the batch makes them again.
fn stored_programs(stored: &[crate::content_store::StoredProgram], parent: crate::code_rule_dsl::language::GrammarDialect, parent_len: usize) -> Option<Arc<[crate::code_rule_dsl::tape::DerivedImage]>> {
    use crate::code_rule_dsl::tape::{self, DerivedImage, StoredDerived, TapeImage};
    stored
        .iter()
        .map(|p| {
            let d = &p.document;
            if d.parent_ordinal != 0 {
                return None;
            }
            let dialect = crate::code_rule_dsl::language::from_tape_tag(u8::try_from(d.grammar_id? & 0xff).ok()?)?;
            let grammar = tape::Grammar::embedded(dialect).ok()?;
            let len = p.tape.raw_len().ok()?;
            let image = TapeImage::from_home(tape::placed::place(len, |out| p.tape.copy_into(out)).ok()?, &grammar).ok()?;
            let at = StoredDerived { ordinal: d.ordinal, parent: 0, role: d.role_id, via: d.via_id, text: &p.text, source_map: &d.source_map, host_errors: &d.host_errors, tape: &[], at: d.at };
            DerivedImage::checked(&at, 0, parent, parent_len, image).ok()
        })
        .collect::<Option<Vec<_>>>()
        .map(Into::into)
}

/// `built`'s programs as the store's writer takes them with its tape (decision 0431 step 6); `None` where not known.
fn queued_programs(built: &Built) -> Result<Option<Arc<[crate::content_store::QueuedDocument]>>, String> {
    let Built::Facts(facts) = built else { return Ok(None) };
    let Some(programs) = facts.programs.as_ref() else { return Ok(None) };
    Ok(Some(programs.iter().map(crate::code_rule_dsl::tape::DerivedImage::queued).collect::<Result<Vec<_>, _>>()?.into()))
}

/// [`build_cached_as`], with the source's id as the store holds it after the build. A parsed tape is stored with its
/// source in one transaction (O95), so a source collected since the caller resolved `known` is stored again, under a
/// new id; `None` when the file did not parse and its source is not known.
pub(crate) fn build_cached_source(file: &dyn SourceFile, cache: &crate::content_store::ContentStore, known: Option<crate::content_store::SourceId>) -> Result<(Built, Option<crate::content_store::SourceId>), String> {
    build_cached_with_tape(file, cache, known, false).map(|(built, id, _)| (built, id))
}

/// [`build_cached_source`], with the tape the facts were read off (decision 0106).
pub fn build_cached_with_tape(
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
            let primary = queued_image(&held, &grammar)?;
            let lines = Arc::new(primary.lines(file.source(), crate::code_rule_dsl::lines::terminators(language), true));
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
    // The stored tape's frame, decompressed (or read from its segment) straight into its home, a slab's room
    // (decision 0353 step 4a).
    let stored = match source_id {
        Some(id) => cache.tape_frames_for(&[id], tag, gv, fv).map_err(e)?.remove(0),
        None => None,
    };
    drop(lookup);
    let grammar = tape::Grammar::embedded(language)?;
    // The programs the parse made (decision 0431 step 6).
    let mut derived: Option<Vec<tape::DerivedImage>> = None;
    let mut queue: Option<(crate::content_store::write_back::Key, HashKey, crate::content_store::TapeBytes)> = None;
    let primary = match stored {
        Some(frame) => {
            crate::code_rule_dsl::logic::profile::decision("input tape cache", || "hit".into());
            let _decode = crate::code_rule_dsl::logic::profile::span(|| "tape decode".into());
            let len = frame.raw_len().map_err(e)?;
            TapeImage::from_home(tape::placed::place(len, |out| frame.copy_into(out)).map_err(|e| e.to_string())?, &grammar)?
        }
        None => {
            crate::code_rule_dsl::logic::profile::decision("input tape cache", || "miss".into());
            record_parse(file, "cached");
            let parse = crate::code_rule_dsl::logic::profile::span(|| "tape parse".into());
            // Its own work: the source prepared for the parser (tree-sitter and the encoding charge their own).
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, file.source().len() as u64);
            let Some(image) = TapeImage::parse(&grammar, file.source(), &|| false) else {
                return Ok((Built::Unparsed, source_id, None));
            };
            let image = image?;
            drop(parse);
            derived = Some(image.derived);
            let (tape, primary) = placed_parse(image.primary, &grammar)?;
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, (*tape).as_ref().len() as u64);
            // Handed to the store's writer, behind the run (decision 0104), and read from memory until written
            // (`pending`, above): the source's id is the writer's, so none is returned for a source not stored yet.
            use sha2::Digest as _;
            let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
            // Queued as `build_in_wave` queues it: gathered in a run's waves, else compressed here and queued alone.
            queue = Some((queued_tape(&digest, tag, gv, fv), HashKey::from_sha256_digest(digest), tape));
            primary
        }
    };
    let lines = Arc::new(primary.lines(file.source(), crate::code_rule_dsl::lines::terminators(language), true));
    let doc = kept_tape(keep, file, &grammar, &primary, &lines)?;
    let _facts = crate::code_rule_dsl::logic::profile::span(|| "facts from tape".into());
    let facts = read_off(doc.as_ref(), file, &grammar, primary, lines)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, facts.len() as u64);
    let built = match derived {
        Some(derived) => with_programs(Built::Facts(facts), &grammar, derived)?,
        None => Built::Facts(facts),
    };
    if let Some((key, canonical, tape)) = queue {
        cache.queue_source_and_tape(key, canonical, tag, gv, fv, file.source(), tape, false, queued_programs(&built)?).map_err(|e| e.to_string())?;
    }
    Ok((built, source_id, doc))
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
        index: Default::default(),
        python_syntax: Default::default(),
        programs: None,
        tape_verdict: false,
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
    Facts { tag, source, lines, tape: Some(tape), grammar: Some(Arc::clone(grammar)), errors: Default::default(), index: Default::default(), python_syntax: Default::default(), programs: None, tape_verdict: false }
}

/// The tape tag of a text-only document's facts (`text`): no dialect has it.
pub const TEXT: u8 = u8::MAX;

/// The facts of a file read as text only (core.rerail's `.scope text`): its source and lines, no tree. Its lines are W's
/// for a file with no language (`lines::terminators_of(None)`: `\r\n`, `\r`, `\n`, U+2028, U+2029).
pub fn text(source: Arc<String>) -> Facts {
    let lines = Arc::new(LineTable::new(&source, crate::code_rule_dsl::lines::terminators_of(None)));
    Facts { tag: TEXT, source, lines, tape: None, grammar: None, errors: std::sync::OnceLock::from(rerail_blocks::vector::Vector::new()), index: Default::default(), python_syntax: Default::default(), programs: None, tape_verdict: false }
}

/// The text document of parsed file `f` (decision 0512: a file the run reads for its programs): its source and line
/// table, shared, and no tree.
pub fn text_of(f: &Facts) -> Facts {
    Facts { tag: TEXT, source: Arc::clone(&f.source), lines: Arc::clone(&f.lines), tape: None, grammar: None, errors: std::sync::OnceLock::from(rerail_blocks::vector::Vector::new()), index: Default::default(), python_syntax: Default::default(), programs: None, tape_verdict: false }
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
        rerail_tape_format::with_tape!($f.facts.doc_tape().tape_with($f.cache), |$t| $body)
    };
}

use rerail_tape_format::tape::Tape as _;

impl Facts {
    /// The dialect's grammar, where the facts have a tape.
    pub fn grammar(&self) -> Option<&Arc<crate::code_rule_dsl::tape::Grammar>> {
        self.grammar.as_ref()
    }

    fn doc_tape(&self) -> &crate::code_rule_dsl::tape::formats::DocTape {
        self.tape.as_deref().expect("a node read from facts with no tape (a text-only document has no nodes)")
    }
    /// The atom of a code's kind: ERROR's where it is one.
    fn atom(&self, e: rerail_tape_format::tape::EntryRef<'_>) -> u32 {
        kind_atom(self.tag, if e.flags & rerail_tape_format::CODE_ERROR != 0 { ERROR_SYMBOL } else { e.kind })
    }
    /// Whether a node is an ERROR or MISSING: the document's fault bit, which the parse wrote (decision 0434 §4), read
    /// without its nodes; a tape without presence (format 15's) scanned for them, in a batch of its own.
    pub fn has_syntax_error(&self) -> bool {
        use rerail_tape_format::tape::Tape as _;
        match self.tape.as_deref() {
            None => false,
            Some(t) if t.image().has_presence() => {
                rerail_tape_format::with_tape!(t.tape_with(None), |t| t.has_kind(u32::MAX, rerail_tape_format::ERROR_CODE))
            }
            Some(_) => self.alone(|f| f.syntax_errors()),
        }
    }
    /// `f` over the facts read in a batch of their own (a caller outside every batch, `FactsIn`): a 16z tape's blocks
    /// decoded into a cache made for them.
    pub fn alone<R>(&self, f: impl FnOnce(FactsIn<'_>) -> R) -> R {
        let cache = self.own_cache();
        f(self.read_in(cache.as_ref()))
    }
    /// A cache for its 16z tape's blocks alone (a batch of its own), where it holds one.
    pub fn own_cache(&self) -> Option<rerail_tape_format::f16::ZCache> {
        self.z_tape().map(|t| rerail_tape_format::f16::ZCache::of([t]))
    }
    /// The facts read in a batch: a 16z tape's blocks decoded into `cache`, the batch's (`FactsIn`).
    pub fn read_in<'a>(&'a self, cache: Option<&'a rerail_tape_format::f16::ZCache>) -> FactsIn<'a> {
        FactsIn { facts: self, cache }
    }
    /// Its 16z tape, where it holds one: what a batch's cache is made for (`ZCache::of`).
    pub fn z_tape(&self) -> Option<&rerail_tape_format::f16::Tape16Z> {
        self.tape.as_deref().and_then(|t| t.image().z_tape())
    }
    pub fn len(&self) -> usize {
        self.tape.as_ref().map_or(0, |t| t.image().node_count())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
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

/// The facts read in a batch (`Facts::read_in`): a 16z tape's blocks decoded into the batch's cache (`ZCache`, the
/// block the device's reads share). Every read of a node goes through one, its batch's cache named at the read (the
/// user's choice, 2026-11-21: a document is in many batches, so no cache is fixed in it); a tape of another format
/// needs none.
#[derive(Clone, Copy)]
pub struct FactsIn<'a> {
    pub facts: &'a Facts,
    cache: Option<&'a rerail_tape_format::f16::ZCache>,
}

impl std::ops::Deref for FactsIn<'_> {
    type Target = Facts;
    fn deref(&self) -> &Facts {
        self.facts
    }
}

impl<'a> FactsIn<'a> {
    #[cfg_attr(feature = "tape-reads", track_caller)]
    pub fn start_line(&self, id: u32) -> u32 {
        self.lines.line(self.start(id) as usize) as u32 + 1
    }
    /// The tape's codes (its table's numbers, what `kind` and `field` read a node's of): none without a tape.
    pub fn code_count(&self) -> usize {
        if self.tape.is_none() {
            return 0;
        }
        on_tape!(self, |t| t.table().len())
    }
    /// Code `code`'s kind atom and field: what `kind` and `field` answer for a node of that code (the views' derived
    /// table of the tape's codes, decision 0353 step 4c).
    pub fn code_entry(&self, code: u32) -> (u32, u16) {
        on_tape!(self, |t| {
            let e = t.table().entry(code);
            (self.atom(e), e.field)
        })
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
            let (mut open, mut deepest): (Vector<usize>, usize) = (rerail_blocks::vector::Vector::new(), 0);
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
    pub fn children(&self, id: u32) -> Vector<u32> {
        on_tape!(self, |t| rerail_tape_format::tape::children(&t, id as usize).map(|c| c as u32).collect())
    }
    /// `id`'s children in order, each handed to `f`, read off the tape with no list made (a probe's).
    pub fn each_child(&self, id: u32, mut f: impl FnMut(u32)) {
        on_tape!(self, |t| rerail_tape_format::tape::children(&t, id as usize).for_each(|c| f(c as u32)))
    }
    /// `id`'s children in order, each handed to `f` with its field code (`field`), read in one pass of the tape: a
    /// probe of a field by its parent tests each child's field with no dispatch on the tape's format a child.
    pub fn each_child_field(&self, id: u32, mut f: impl FnMut(u32, u16)) {
        on_tape!(self, |t| {
            let table = t.table();
            rerail_tape_format::tape::children(&t, id as usize).for_each(|c| f(c as u32, table.entry(t.code(c)).field))
        })
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
    pub fn token_text(&self, token: &Token) -> &'a str {
        &self.facts.source[token.start as usize..token.end as usize]
    }
    /// The document's index (decision 0656 step 4): its tape home's, read where it lies, else built once for it
    /// (`DocIndex::build`). Every count, listed code, text test and place the facts answer from it goes through it.
    pub fn index(&self) -> rerail_tape_format::doc_index::DocIndex<'a> {
        use rerail_tape_format::doc_index::DocIndex;
        let facts: &'a Facts = self.facts;
        if let Some(x) = facts.tape.as_deref().and_then(|t| t.image().aux_bytes()).and_then(|b| DocIndex::read(b).ok()) {
            return x;
        }
        let bytes = facts.index.get_or_init(|| {
            let _span = crate::code_rule_dsl::logic::profile::span(|| "a document's index, built (its home holds none)".into());
            let grammar = facts.grammar.as_ref().expect("a document's index, of facts with a tape");
            on_tape!(self, |t| {
                let named = crate::code_rule_dsl::tape::formats::named_codes(t.table(), grammar.lang);
                DocIndex::build(&t, facts.source.as_bytes(), &named).unwrap_or_else(|e| panic!("a document's index: {e}"))
            })
        });
        DocIndex::read(bytes).expect("the index built")
    }
    /// Named node `id`'s place among its parent's named children (0 for the first, and the root's), read in the
    /// document's index (`DocIndex::place`).
    pub fn place(&self, id: u32) -> u32 {
        let index = self.index();
        on_tape!(self, |t| index.place(&t, id))
    }
    /// The nodes whose code is in the set `codes` makes of the tape's table, by id. The set is a kind's or a class's
    /// numbers from the table's index (`CodeTable::of_kind`), not a test of every entry: on format 16 the table is the
    /// language's (decision 0336 §7). The document's index (`index`) answers whether it holds one, and lists the nodes
    /// of a code few nodes have; else a scan of the codes, 64 nodes at a time.
    fn matching(&self, codes: Codes) -> Vector<u32> {
        use rerail_tape_format::tape::CodeSet;
        if self.is_empty() {
            return rerail_blocks::vector::Vector::new();
        }
        let index = self.index();
        on_tape!(self, |t| {
            let table = t.table();
            // A kind's or a class's set is the table's own, borrowed (decision 0336 §7: on format 16 the table is the
            // language's, so a set made per document would cost a table's index per probe).
            // ERROR's: its kind's codes, the errors and kind 0's, in one sorted run in a block.
            let mut made: Vector<u32> = Vector::new();
            let set: CodeSet<'_> = match codes {
                Codes::Kind(code) if code == ERROR_SYMBOL => {
                    for s in [table.of_kind(code), table.errors(), table.of_kind(0)] {
                        // copy: derived: the union of three of the table's sets, written once as ERROR's.
                        made.extend_from_slice(s.numbers().unwrap_or_default());
                    }
                    made.sort_unstable();
                    made.dedup();
                    CodeSet::Few(&made)
                }
                Codes::Kind(code) => table.of_kind(code),
                Codes::Class(code) => table.of_class(code),
                Codes::Faults => table.faults(),
            };
            if set.is_empty() {
                return rerail_blocks::vector::Vector::new();
            }
            // Whether the document holds one, from the smaller side: the set's numbers searched among the codes it
            // holds, or each code it holds tested.
            let numbers = set.numbers();
            let meets = match numbers {
                Some(numbers) if (numbers.len() as u32) < index.distinct() => numbers.iter().any(|&n| index.holds(n)),
                _ => index.counts().any(|(c, _)| set.contains(c)),
            };
            if !meets {
                return rerail_blocks::vector::Vector::new();
            }
            // Codes few nodes have: their nodes as the index lists them, no scan.
            if let Some(numbers) = numbers
                && numbers.iter().all(|&n| index.listed(n).is_some())
            {
                let mut out: Vector<u32> = numbers.iter().flat_map(|&n| index.listed(n).into_iter().flatten()).collect();
                out.sort_unstable();
                return out;
            }
            let mut out = rerail_blocks::vector::Vector::new();
            t.match_groups(&set, |base, mut m| {
                while m != 0 {
                    out.push((base + m.trailing_zeros() as usize) as u32);
                    m &= m - 1;
                }
            });
            out
        })
    }
    /// The nodes of kind `kind` (an atom), by id.
    pub fn of_kind(&self, kind: u32) -> Vector<u32> {
        if kind_tag(kind) != self.tag {
            return rerail_blocks::vector::Vector::new();
        }
        // `atom`'s inverse: the codes of the kind with no ERROR flag; for slot 0 (ERROR's atom, which kind 0's shares)
        // also every ERROR code and kind 0's.
        self.matching(Codes::Kind(kind_code(kind)))
    }
    /// Nodes of the abstract class `class` (a supertype's kind atom), by id.
    pub fn of_class(&self, class: u32) -> Vector<u32> {
        if kind_tag(class) != self.tag {
            return rerail_blocks::vector::Vector::new();
        }
        self.matching(Codes::Class(kind_code(class)))
    }
    /// Each kind atom the document holds and its nodes, and each class atom and its nodes, each sorted by atom.
    pub fn histogram(&self) -> (Vec<(u32, u32)>, Vec<(u32, u32)>) {
        if self.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let index = self.index();
        on_tape!(self, |t| {
            // Counted by the codes the document holds, as its index counts them, not a counter per code of the table:
            // on format 16 the table is the language's (decision 0336 §7).
            let (mut kinds, mut classes) = (std::collections::BTreeMap::new(), std::collections::BTreeMap::new());
            for (code, n) in index.counts() {
                let e = t.table().entry(code);
                *kinds.entry(self.atom(e)).or_insert(0u32) += n;
                for &class in e.classes {
                    *classes.entry(kind_atom(self.tag, class)).or_insert(0u32) += n;
                }
            }
            (kinds.into_iter().collect(), classes.into_iter().collect())
        })
    }
    /// The kind atoms the document holds, ascending.
    pub fn kinds(&self) -> Vector<u32> {
        self.histogram().0.into_iter().map(|(k, _)| k).collect()
    }
    /// The ERROR and MISSING nodes, ascending: as the tape's writer listed them in its home (`TapeImage::faults`),
    /// read there; a format 15 tape's found once by its codes.
    pub fn errors(&self) -> &'a [u32] {
        if let Some(listed) = self.facts.tape.as_deref().and_then(|t| t.image().faults()) {
            return listed;
        }
        self.facts.errors.get_or_init(|| self.matching(Codes::Faults))
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
    pub fn text(&self, id: u32) -> &'a str {
        let (start, end) = self.span(id);
        &self.facts.source[start as usize..end as usize]
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
    pub fn with_parents(self, a: u32, b: u32) -> impl Iterator<Item = (u32, Option<u32>)> + 'a {
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

/// The relations a program in `core.rerail`'s shape declares (the engine's names, decision 0141, less those decision
/// 0174's rewrite adds), as the rule language reads them; registered at load (`install_core_names`). Empty without one.
pub fn core_names() -> &'static std::collections::BTreeSet<String> {
    static EMPTY: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    CORE_NAMES.get().map_or(&EMPTY, |names| names())
}

static CORE_NAMES: std::sync::OnceLock<fn() -> &'static std::collections::BTreeSet<String>> = std::sync::OnceLock::new();

/// Registers what reads `core.rerail`'s names (once, at load).
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
        // CPython's verdict (`facts::vtab::syntax::cpython`; core.rerail's `syntax_error(d, l, msg)`).
        "cpython_syntax_error" => &["d", "l", "msg"],
        // Ruff's string annotations and their trees (`facts::vtab::syntax::annotation`).
        "string_annotation" => &["s"],
        "parsed" => &["s", "grammar", "root"],
        // W's pythonAst `literal_json` (`facts::vtab::syntax::cpython`).
        "literal_json" => &["n", "v"],
        // core.rerail's @py_unparse(n) and @ast_dump(n), and W's hashes of dumps (`facts::vtab::syntax::cpython`).
        "cpython_unparse" | "cpython_ast_dump" => &["n", "s"],
        "dump_sha256" => &["n", "h"],
        "body_sha256" | "signature_sha256" => &["f", "h"],
        "intval" => &["n", "v"],
        "op_at" => &["n", "i", "s"],
        "op" | "op_t" => &["n", "s"],
        // `core.rerail`'s (`facts::vtab::syntax`, decision 0046).
        "syntax_error" => &["d", "l", "msg"],
        "spread" | "rr_long_text_node" => &["n"],
        "next_sibling" => &["a", "b"],
        "visit_index" => &["n", "i"],
        "node_order" => &["n", "k"],
        "ast_depth" => &["n", "k"],
        "def_unit" => &["n", "u"],
        "line_count" => &["d", "k"],
        "end_line" => &["n", "l"],
        "col" => &["n", "c"],
        "rr_report_location" => &["n", "line", "column", "end_line", "end_column"],
        "docof" => &["n", "d"],
        "source" => &["d", "sp"],
        "src_line" => &["d", "l", "sp"],
        "str_part" => &["lit", "i", "v", "at", "spec"],
        "str_part_word" => &["lit", "i", "w", "at", "spec"],
        "str_part_start" => &["lit", "i", "l", "c"],
        "first_token" | "last_token" => &["n", "t"],
        "token_before" | "token_after" => &["x", "t"],
        "end_point" => &["n", "p"],
        "tree_digest" | "def_digest" => &["n", "s"],
        // core.rerail's tokens (`facts::vtab::syntax::tree`).
        "prev_token" => &["n", "s"],
        "token" | "rr_token_anon" => &["p", "i", "s", "start", "end_"],
        "rr_token_named" => &["p", "i", "c", "start", "end_"],
        "rx_span" => &["p", "c", "j", "g", "m"],
        "ast_match" => &["p", "n", "g", "c"],
        // Decision 0055's (`facts::vtab::syntax::guests`).
        "doc_grammar" => &["d", "g"],
        // The document underneath a file's program and its grammar (decision 0431 §4), for the lowering's findings
        // alone: a rule names a program by its file and place (`parsed`).
        "rr_program" => &["d", "file", "grammar"],
        "tape_of" => &["d", "t"],
        "sub_tape" => &["at", "grammar", "t"],
        "host_node" => &["n", "h"],
        "guest_of" => &["n", "s"],
        "guest_place" => &["n", "s", "l", "c"],
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
        "unpackaged" | "tape_error" => &["d"],
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
        // A `match_all` regex's matches (`preparation::text::TextRelations`, `facts::vtab::syntax::Extra`).
        rel if rel.starts_with(crate::code_rule_dsl::logic::strings::MATCHES) => &["sp", "m"],
        rel if rel.starts_with(crate::code_rule_dsl::logic::strings::GROUPS) => &["sp", "m", "g", "gsp"],
        // A registered family's lazy relations (`facts::vtab::syntax::Families`: Python's `py.*`).
        rel if rel.starts_with("py_") => FAMILY_COLUMNS.try_get()?(rel)?,
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
    /// The relation the corpus binds for CPython's verdict (core.rerail's `syntax_error(d, l, msg)`).
    pub const SYNTAX_ERROR: &str = "cpython_syntax_error";
    /// W's `literal_json` (`cpython::literal`).
    pub const LITERAL_JSON: &str = "literal_json";
    /// core.rerail's `@py_unparse(n)` (`cpython::unparse`).
    pub const UNPARSE: &str = "cpython_unparse";
    /// core.rerail's `@ast_dump(n)` (`cpython::dump`).
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
