//! Source-revision-bound rec6 preorder tape and transient ast-grep document.
//! No parser's tree survives encoding; restoration never invokes a parser. Every tape holds one
//! source, one grammar and one tree, and passes the same checks. A tree's syntax errors are its
//! ERROR and MISSING records: a Blade template's lexer errors are ERROR leaves, zero-width at their
//! offset, and a template Forte does not parse is a `document` holding one ERROR. A file that
//! embeds code (`embed`) is several documents (`FileImage`): its own tape, and beside it each
//! program it derives (`DerivedImage`), with a tape of its own over the program's text and the
//! segments that map that text back to the document it was derived from. The store keeps each
//! derived tape as a row of its own (`file_documents`); no tape holds another.
use crate::code_rule_dsl::embed::segments::SegmentMap;
use crate::code_rule_dsl::embed::Program;
use crate::code_rule_dsl::language::{GrammarDialect, KindCatalog, Via};
use crate::code_rule_dsl::lines::LineTable;
use crate::code_rule_dsl::lines::Terminators;
use ast_grep_core::Position;
use ast_grep_core::source::{Doc, Edit, SgNode};
use ast_grep_language::LanguageExt;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::ops::Range;
use std::sync::{Arc, Weak};
use rerail_tape_format::tape::{Entry, Tape};
use tree_sitter::{Language, Node as TsNode, TreeCursor};

/// v9np (decision 0025): structure-only, regular columns, no parent column; 10: supertype
/// memberships folded into the codes; later formats: `rerail_tape_format::FORMAT_VERSION`.
pub const TAPE_FORMAT_VERSION: u32 = rerail_tape_format::FORMAT_VERSION;
/// The format's constants are `rerail_tape_format`'s: the parsers write tapes with its writer
/// (decision 0140), and an image is one byte string however it was made.
pub use rerail_tape_format::{CODE_ERROR, CODE_MISSING, COLUMN_ALIGN, ERROR_CODE, FIELD_LIMIT, HEADER_BYTES, KIND_LIMIT, MAGIC as TAPE_MAGIC, SENTINEL, TAIL_FROM_START, header, padded};
pub use rerail_tape_format::CODE_EXTRA;
/// The language tag (`language::tape_tag`) a stored tape image names in its header, if `bytes`
/// starts like one (any format version): the store picks a frame's dictionary by it.
pub fn stored_tape_tag(bytes: &[u8]) -> Option<u8> {
    (bytes.len() > header::LANGUAGE && bytes.starts_with(&TAPE_MAGIC[..6])).then(|| bytes[header::LANGUAGE])
}
pub const NONE: u32 = u32::MAX;
pub const CHECKPOINT_LIMIT: usize = 0x00ff_ffff;

// P's index columns, read by `prefilter`'s `nera_tree` (the §17 SQL fallback, tests only so far).
#[allow(dead_code)]
pub mod index;
pub mod navigation;
#[allow(unused_imports)]
pub use index::IndexImage;
pub mod wire;
pub mod tokens;
mod token_automata;
// tokens_tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
// cratify: `token_readers` is rerail-w's (it mounts it)
pub mod resident;
pub mod validated;
/// A document's tape through one API, in the run's format (decision 0293).
pub mod formats;
pub mod placed;
pub mod token_census;
/// Python's tape from the lean parser (`rerail_python_lean`).
pub mod python_lean;
/// JavaScript's, TypeScript's and TSX's tapes from the lean parser (`rerail_ecma_lean`).
pub mod ecma_lean;
/// Rust's tape from the lean parser (`rerail_rust_lean`).
pub mod rust_lean;
/// Java's tape from the lean parser (`rerail_java_lean`).
pub mod java_lean;
/// Go's tape from the lean parser (`rerail_go_lean`).
pub mod go_lean;
/// C's and C++'s tapes from the lean parser (`rerail_c_lean`).
pub mod c_lean;
/// C#'s tape from the lean parser (`rerail_csharp_lean`).
pub mod csharp_lean;
/// Kotlin's tape from the lean parser (`rerail_kotlin_lean`).
pub mod kotlin_lean;
/// Swift's tape from the lean parser (`rerail_swift_lean`).
pub mod swift_lean;
/// Scala's tape from the lean parser (`rerail_scala_lean`).
pub mod scala_lean;
/// PHP's tape from the lean parser (`rerail_php_lean`).
pub mod php_lean;
/// HTML's tape from the lean parser (`rerail_html_lean`).
pub mod html_lean;
/// CSS's tape from the lean parser (`rerail_css_lean`).
pub mod css_lean;
/// Markdown's tape from the lean parser (`rerail_markdown_lean`).
pub mod markdown_lean;
/// Markdown's inline texts' tapes from the lean parser (`rerail_markdown_inline_lean`).
pub mod markdown_inline_lean;
/// SQL's tape from the lean parser (`rerail_sql_lean`).
pub mod sql_lean;
/// Vue's tape from the lean parser (`rerail_vue_lean`).
pub mod vue_lean;
pub use wire::index_span;
use wire::{CHECKPOINT_STRIDE, WIDTH};
pub const ERROR_SYMBOL: u16 = u16::MAX;
// QueryCursor uses the same fixed capacity in the pinned Tree-sitter query runtime.
pub const MAX_NODE_SUPERTYPES: usize = 8;

const _: () = {
    assert!(
        std::mem::size_of::<TreeCursor<'static>>()
            == std::mem::size_of::<tree_sitter::ffi::TSTreeCursor>()
    );
    assert!(
        std::mem::align_of::<TreeCursor<'static>>()
            == std::mem::align_of::<tree_sitter::ffi::TSTreeCursor>()
    );
};

unsafe extern "C" {
    fn ts_tree_cursor_current_status(
        cursor: *const tree_sitter::ffi::TSTreeCursor,
        field_id: *mut tree_sitter::ffi::TSFieldId,
        has_later_siblings: *mut bool,
        has_later_named_siblings: *mut bool,
        can_have_later_siblings_with_this_field: *mut bool,
        supertypes: *mut tree_sitter::ffi::TSSymbol,
        supertype_count: *mut std::ffi::c_uint,
    );
}

/// Reads the abstract symbols on the current cursor path from the pinned Tree-sitter runtime.
#[cfg(any(test, feature = "test-hooks"))]
pub fn cursor_supertypes(cursor: &TreeCursor<'_>) -> ([u16; MAX_NODE_SUPERTYPES], usize) {
    let (supertypes, count, _) = cursor_status(cursor);
    (supertypes, count)
}

/// The abstract symbols on the current cursor path and the node's field, in one call. The field
/// is `ts_tree_cursor_current_field_id`'s except that that one stops at an extra node and this one
/// walks past it, so an extra node's field is taken as none.
fn cursor_status(cursor: &TreeCursor<'_>) -> ([u16; MAX_NODE_SUPERTYPES], usize, u16) {
    let mut field = 0;
    let mut later = false;
    let mut later_named = false;
    let mut later_field = false;
    let mut supertypes = [0; MAX_NODE_SUPERTYPES];
    let mut count = MAX_NODE_SUPERTYPES as std::ffi::c_uint;
    // SAFETY: the pinned wrapper layout is asserted above. All output storage remains live, and
    // Tree-sitter writes no more than `count` symbols.
    unsafe {
        ts_tree_cursor_current_status(
            std::ptr::from_ref(cursor).cast(),
            &mut field,
            &mut later,
            &mut later_named,
            &mut later_field,
            supertypes.as_mut_ptr(),
            &mut count,
        );
    }
    (supertypes, count as usize, field)
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// The largest table capacity `TapeImage::from_bytes` reserved on this thread since the last
    /// `take_table_reservation`.
    static TABLE_RESERVATION: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-hooks"))]
fn note_table_reservation(capacity: usize) {
    TABLE_RESERVATION.set(TABLE_RESERVATION.get().max(capacity));
}

/// The largest alias or override table capacity reserved since the last call, then resets it.
#[cfg(any(test, feature = "test-hooks"))]
pub fn take_table_reservation() -> usize {
    TABLE_RESERVATION.replace(0)
}

/// Deterministic grammar-wide codes: kind code is the dialect's kind id, field code its field id
/// (zero means no field), both as `GrammarDialect::kinds` numbers them.
pub struct Grammar {
    pub lang: GrammarDialect,
    pub kinds: KindCatalog,
    /// Each kind code's name and whether it is named, read once: Tree-sitter answers a name
    /// by measuring and validating its C string on every call, and `kind` is the matcher's
    /// most frequent question.
    named: Vec<(Option<&'static str>, bool)>,
    /// The field names and their codes, sorted by name, read once: Tree-sitter answers a field's code by comparing
    /// the name with each field's in turn, and the summaries ask it at every `field` (Q51).
    fields: Vec<(&'static str, u16)>,
}

impl Grammar {
    pub fn dialect(&self) -> GrammarDialect { self.lang }
    /// The name of field `code` (M's facts-a, which read the private catalog directly).
    #[allow(dead_code)]
    pub fn field_name(&self, code: u16) -> Option<&'static str> {
        self.kinds.field_name(code)
    }

    /// The tree-sitter language of this grammar's dialect (Tempo's accessor; the packed runtime's
    /// Metal tests compile queries against it).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn language(&self) -> Language {
        self.lang.get_ts_language()
    }

    pub fn field_code(&self, name: &str) -> Option<u16> {
        self.fields.binary_search_by(|(field, _)| (*field).cmp(name)).ok().map(|at| self.fields[at].1)
    }
    pub fn has_field(&self, name: &str) -> bool {
        self.field_code(name).is_some()
    }
    pub fn new(lang: GrammarDialect) -> Result<Arc<Self>, String> {
        let kinds = lang.kinds();
        if kinds.kind_count() > KIND_LIMIT || kinds.field_count() >= FIELD_LIMIT {
            return Err("grammar exceeds rec6 kind/field code width".into());
        }
        let named = (0..kinds.kind_count())
            .map(|code| {
                let code = code as u16;
                (kinds.kind_name(code), kinds.kind_is_named(code))
            })
            .collect();
        let mut fields: Vec<(&'static str, u16)> = (1..=kinds.field_count() as u16).filter_map(|code| Some((kinds.field_name(code)?, code))).collect();
        fields.sort_unstable();
        // A name of two codes keeps its first, as the grammar's own lookup answers.
        fields.dedup_by_key(|(name, _)| *name);
        Ok(Arc::new(Self { lang, kinds, named, fields }))
    }

    /// The grammar of a dialect, built once per process and shared by every document of it: built per document, a
    /// run over 300 files held 61 MB of grammars at its peak (Q74), and built each one's kind table again.
    pub fn embedded(lang: GrammarDialect) -> Result<Arc<Self>, String> {
        static GRAMMARS: std::sync::LazyLock<
            std::sync::Mutex<std::collections::HashMap<GrammarDialect, Arc<Grammar>>>,
        > = std::sync::LazyLock::new(Default::default);
        let mut grammars = GRAMMARS.lock().map_err(|_| "grammar cache poisoned")?;
        if let Some(grammar) = grammars.get(&lang) {
            return Ok(Arc::clone(grammar));
        }
        let grammar = Self::new(lang)?;
        grammars.insert(lang, Arc::clone(&grammar));
        Ok(grammar)
    }

    pub fn kind_name(&self, code: u16) -> Option<&'static str> {
        match self.named.get(usize::from(code)) {
            Some((name, _)) => *name,
            // ERROR's code lies past the grammar's symbols.
            None => self.kinds.kind_name(code),
        }
    }

    pub fn kind_is_named(&self, code: u16) -> bool {
        match self.named.get(usize::from(code)) {
            Some((_, named)) => *named,
            None => self.kinds.kind_is_named(code),
        }
    }
}

/// Why a file its dialect's own parser reads has no tape (`TreeParser`).
#[derive(Debug)]
pub enum TemplateError {
    /// The parser stopped at a limit (Forte's nesting depth, its step budget) on the file or on a
    /// template it holds: that says nothing about its syntax, and nothing was read. Why, as the parser says.
    Parser(&'static str),
    /// Interrupted, or no tape could be encoded.
    Encode(String),
}

/// A parser's own kinds: its kind names by id (from 1), its first anonymous id, and its field names by id.
pub type OwnTable = (&'static [&'static str], u16, &'static [&'static str]);

/// A dialect whose files a language parses itself, not with tree-sitter (Blade's Forte), or with a parser
/// standing in for tree-sitter's (JSON's simdjson kernels). The crate that owns the parser registers each
/// implementation for its dialect as the engine loads (`install_parsers`, a `ParserImpl` each); the tape
/// reads the file through the one the router takes (`tree_parser`, `TapeImage::parse`).
pub trait TreeParser: Send + Sync {
    /// The file's documents: `None` where the parser leaves the file to tree-sitter, `Err(Parser)` where
    /// it stopped at a limit before it read the file.
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>>;

    /// Its own kinds, where its trees are not a tree-sitter grammar's.
    fn kinds(&self) -> Option<OwnTable> {
        None
    }

    /// The kinds a rule's `kind` names, where it names more than its own.
    fn kind_names(&self, _kind: &str) -> Option<&'static [&'static str]> {
        None
    }

    /// Its patterns, where they are its own trees: else they are tapes, as documents are (`pattern_doc`).
    fn build_pattern(
        &self,
        _builder: &ast_grep_core::matcher::PatternBuilder,
    ) -> Option<Result<ast_grep_core::matcher::Pattern, ast_grep_core::matcher::PatternError>> {
        None
    }
}

/// One implementation of a dialect's parser, as its language registers it (`install_parsers`). A dialect
/// may have several (JSON's: a kernel for each CPU family, simdjson's DOM parser); the router reads its
/// files with the best one this host runs, and a file that one leaves alone goes to tree-sitter.
pub struct ParserImpl {
    pub dialect: GrammarDialect,
    /// Its name, for the profile and the route's listing (`parser_routes`).
    pub name: &'static str,
    /// Its place among the dialect's implementations: the lowest this host runs is taken. The order is
    /// the registering language's, from its measurements (JSON's: decision 0148).
    pub rank: u32,
    /// Whether this host runs it: its CPU's instructions, its build's.
    pub runs: bool,
    pub parser: &'static dyn TreeParser,
}

/// The parser router: every implementation registered, and for each dialect the one taken.
struct Router {
    registered: Vec<ParserImpl>,
    /// Each dialect's implementation, an index into `registered`.
    chosen: Vec<(GrammarDialect, usize)>,
}

static ROUTER: std::sync::OnceLock<Router> = std::sync::OnceLock::new();

/// The engine's parsers, by dialect; the first call holds. Each dialect's files are read by the
/// lowest-ranked implementation this host runs (the first registered, among equals).
pub fn install_parsers(registered: Vec<ParserImpl>) {
    let mut chosen: Vec<(GrammarDialect, usize)> = Vec::new();
    for (i, r) in registered.iter().enumerate() {
        if !r.runs {
            continue;
        }
        match chosen.iter_mut().find(|(dialect, _)| *dialect == r.dialect) {
            Some((_, best)) if registered[*best].rank <= r.rank => {}
            Some((_, best)) => *best = i,
            None => chosen.push((r.dialect, i)),
        }
    }
    let _ = ROUTER.set(Router { registered, chosen });
}

/// `dialect`'s own parser, where its language registered one this host runs: the router's choice.
pub fn tree_parser(dialect: GrammarDialect) -> Option<&'static dyn TreeParser> {
    let router = ROUTER.get()?;
    router.chosen.iter().find(|(lang, _)| *lang == dialect).map(|&(_, i)| router.registered[i].parser)
}

/// Every registered implementation, in registration order, and whether the router took it.
pub fn parser_routes() -> Vec<(&'static ParserImpl, bool)> {
    let Some(router) = ROUTER.get() else { return Vec::new() };
    router.registered.iter().enumerate().map(|(i, r)| (r, router.chosen.iter().any(|&(_, c)| c == i))).collect()
}

impl From<String> for TemplateError {
    fn from(error: String) -> Self {
        Self::Encode(error)
    }
}


/// Owns only compact structure and a binding to the exact source and grammar.
/// The first derived class id (`TapeDoc::with_labels`), above every grammar's kinds and supertypes
/// (`KIND_LIMIT`). A wave's distinct labels take the ids from here to `u16::MAX`: 0x4000 leaves
/// room for 49,151 of them (a whole Python pack's; 0xf000 held 4,095). The Metal sources spell it
/// `16384u`.
pub const DERIVED_CLASS_BASE: u16 = 0x4000;
const _: () = assert!(DERIVED_CLASS_BASE as usize >= KIND_LIMIT);

/// A tape image in v9np's columns (decision 0025; P decision 0003). Each node is a fixed-width
/// row across five columns, in preorder:
/// - `code`: u16, an index into `codes`, which folds the node's kind, field, flags (extra, error,
///   missing), contextual alias and supertype memberships into one code;
/// - `size`: the subtree's node count (2 or 4 bytes: `widths[0]`);
/// - `base` (one u32 per `CHECKPOINT_STRIDE` nodes, the block's least start) and `off`
///   (`widths[1]`): `start(i) = base[i / 16] + off[i]`;
/// - `tail` (`widths[2]`): `end(i) = start(last(i)) + tail[i]`, `last(i) = i + size(i) - 1`, or
///   `start(i) + tail[i]` under `TAIL_FROM_START`.
///
/// There is no parent column: `restore_tree` rebuilds parents from the sizes with a stack.
#[derive(Clone)]
pub struct TapeImage {
    /// The tape in the format the run stores (decision 0293 §4): format 15's columns as a parser writes them, or
    /// format 16 (or 16z) as stored, read where it lies.
    held: Held,
    pub language: GrammarDialect,
    pub source_len: usize,
    /// The source's SHA-256 where the image was encoded from it; not stored (v12), so `None` for
    /// an image read from bytes, whose store pairs it with its source.
    pub source_digest: Option<[u8; 32]>,
}

/// A tape as held, in one format.
#[derive(Clone)]
pub enum Held {
    F15(Columns15),
    F16(Arc<rerail_tape_format::f16::Tape16>),
    F16Z(Arc<rerail_tape_format::f16::Tape16Z>),
}

/// Format 15's columns (above), its code table, and the table as the tape API reads it (made once).
#[derive(Clone)]
pub struct Columns15 {
    /// The five columns (code, size, base, off, tail), from `base` in `bytes`, each from a `COLUMN_ALIGN` boundary
    /// and padded with zeros, as the stored image lays them after its header: one base addresses every column, as a
    /// reader of the tape in place takes it (decision 0310). A loaded image is placed: `bytes` is the stored image
    /// in its slab's room, read where it lies (decision 0353 step 4a); a parser's holds its columns alone.
    bytes: rerail_tape_format::home::Home,
    base: usize,
    /// Each column's start from `base`, and its length.
    at: [(usize, usize); 5],
    pub widths: [u8; 3],
    pub codes: Vec<Code>,
    /// Contextual Tree-sitter aliases whose displayed kind or named flag differs from the static
    /// language catalog. The numeric kind remains the native `kind_id` for matcher parity.
    pub kind_aliases: Vec<(String, bool)>,
    table: std::sync::OnceLock<rerail_tape_format::tape::CodeTable>,
}

/// A folded code: kind (10 bits, ERROR's is `ERROR_CODE`) and field (6 bits) as v8's `kf`, the
/// flags, the contextual alias (`SENTINEL`: none), and the node's query-independent supertype
/// memberships (abstract grammar symbols, ascending; unused slots zero). Memberships depend on
/// where the node sits, not on its kind alone, so they are the node's own, as the parser's
/// cursor reports them; nodes in the same context share a code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Code {
    kf: u16,
    pub flags: u8,
    pub alias: u16,
    pub class_count: u8,
    pub classes: [u16; MAX_NODE_SUPERTYPES],
}

impl Code {
    /// A code of `kind` under `field`, with no flags or alias, in the supertypes `classes` (ascending).
    pub fn plain(kind: u16, field: u16, classes: &[u16]) -> Result<Self, String> {
        if kind as usize >= KIND_LIMIT || field as usize >= FIELD_LIMIT || kind == ERROR_CODE || classes.len() > MAX_NODE_SUPERTYPES {
            return Err(format!("kind {kind}, field {field} or {} supertypes past the tape's codes", classes.len()));
        }
        let mut code = Code { kf: kind | (field << 10), flags: 0, alias: SENTINEL, class_count: classes.len() as u8, classes: [0; MAX_NODE_SUPERTYPES] };
        code.classes[..classes.len()].copy_from_slice(classes);
        Ok(code)
    }

    /// An `ERROR` node's code under `field` (a parser that recovers writes it as tree-sitter does).
    pub fn error(field: u16) -> Result<Self, String> {
        if field as usize >= FIELD_LIMIT {
            return Err(format!("field {field} past the tape's codes"));
        }
        Ok(Code { kf: ERROR_CODE | (field << 10), flags: CODE_ERROR, alias: SENTINEL, class_count: 0, classes: [0; MAX_NODE_SUPERTYPES] })
    }

    /// Its supertype memberships, ascending.
    pub fn classes(&self) -> &[u16] {
        &self.classes[..usize::from(self.class_count)]
    }

    /// Its kind and field, as v8's `kf`: the kind in the low 10 bits, the field above.
    pub fn kf(&self) -> u16 {
        self.kf
    }

    /// The code's bytes in the image: kf, flags, alias, class count, classes.
    fn stored_len(&self) -> usize {
        6 + 2 * usize::from(self.class_count)
    }
}

/// A little-endian value of `width` bytes (2 or 4) at row `i` of `column`.
fn cell(column: &[u8], i: usize, width: usize) -> u32 {
    let at = i * width;
    if width == 2 {
        u32::from(u16::from_le_bytes([column[at], column[at + 1]]))
    } else {
        u32::from_le_bytes(column[at..at + 4].try_into().expect("u32 width"))
    }
}

fn put_cell(column: &mut Vec<u8>, value: u32, width: usize) {
    column.extend_from_slice(&value.to_le_bytes()[..width]);
}

fn width_for(max: u32) -> usize {
    if max <= 0xffff { 2 } else { 4 }
}

/// Which of a file's documents one is: the file's own tree, or one derived from another of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentRole {
    /// The file's own tree, over the file's bytes.
    Primary,
    /// A program whose text is one contiguous, byte-exact slice of the document it was derived
    /// from: its source map is one copied segment over the whole text.
    Embedded,
    /// Any other derived program, written as well as copied (Blade's compiled view, a Vue
    /// component's script program).
    Generated,
}

impl DocumentRole {
    /// The role of a derived document of `text` that `source_map` maps to its parent.
    pub fn derived(text: &str, source_map: &SegmentMap) -> Self {
        if !text.is_empty()
            && source_map.len() == 1
            && source_map.copied_span(0..text.len()).is_some()
        {
            Self::Embedded
        } else {
            Self::Generated
        }
    }

    /// Its persisted code (`file_documents.role_id`).
    pub fn code(self) -> i64 {
        match self {
            Self::Primary => 0,
            Self::Embedded => 1,
            Self::Generated => 2,
        }
    }

    fn from_code(code: i64) -> Option<Self> {
        match code {
            0 => Some(Self::Primary),
            1 => Some(Self::Embedded),
            2 => Some(Self::Generated),
            _ => None,
        }
    }
}

/// An embedding's persisted identity (`file_documents.via_id`): a nonnegative integer read from
/// its registry name (`Via::id`), so it holds across builds that keep the name.
pub fn via_code(via: Via) -> i64 {
    let digest = Sha256::digest(via.id().as_bytes());
    i64::from_be_bytes(digest[..8].try_into().expect("digest width")) & i64::MAX
}

/// The embedding of `host`'s whose persisted identity is `code` (`via_code`).
fn via_of_code(host: GrammarDialect, code: i64) -> Option<Via> {
    host.embeds()
        .iter()
        .map(|&(via, _)| via)
        .find(|&via| via_code(via) == code)
}

/// A file's documents as parsed: its own tree, and the ones derived from it.
#[derive(Clone)]
pub struct FileImage {
    pub primary: TapeImage,
    /// In ordinal order (`DerivedImage::parent` counts the primary as 0, the first of these as 1).
    pub derived: Vec<DerivedImage>,
}

/// A document derived from another of the file's: its own text, grammar and tape, and how it maps
/// to the document it was derived from.
#[derive(Clone)]
pub struct DerivedImage {
    /// `Embedded` or `Generated`, never `Primary`.
    pub role: DocumentRole,
    /// The embedding that built it.
    pub via: Via,
    /// The document it was derived from: 0 the primary, `i` the `i`-th derived one. Embedders run
    /// on the primary only; the field makes nesting representable.
    pub parent: u32,
    pub text: Arc<String>,
    /// Where its text stands in its parent's (`embed::segments`).
    pub(super) source_map: Arc<SegmentMap>,
    /// The parent's bytes it cannot be read through (`embed::Program::host_errors`).
    pub(super) host_errors: Arc<Vec<(u32, u32)>>,
    /// Its tree over `text`, in its own dialect.
    pub tape: TapeImage,
}

impl DerivedImage {
    /// The image of `program`, which `via` built from document `parent` of `host`'s dialect over
    /// `source`.
    pub fn encode(
        via: Via,
        program: Program,
        parent: u32,
        host: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        if !host.lang.embeds().contains(&(via, program.dialect)) {
            return Err("an embedder built a program in a dialect its host does not embed".into());
        }
        if program
            .host_errors
            .iter()
            .any(|range| range.start > range.end || range.end > source.len())
            || program
                .host_errors
                .windows(2)
                .any(|pair| (pair[0].start, pair[0].end) > (pair[1].start, pair[1].end))
        {
            return Err(
                "an embedded program's host errors are not sorted ranges of the host".into(),
            );
        }
        let grammar = Grammar::embedded(program.dialect)?;
        let tape = match &program.parse {
            crate::code_rule_dsl::embed::Parse::Tree(tree) => TapeImage::encode_tree_image(
                tree.root_node(),
                &grammar,
                &program.text,
                Some(&program.segments),
                budget,
            )?,
            crate::code_rule_dsl::embed::Parse::Tape(tape) => tape.omitting(&program.segments, &grammar, &program.text, budget)?,
        };
        Ok(Self {
            role: DocumentRole::derived(&program.text, &program.segments),
            via,
            parent,
            host_errors: Arc::new(
                program
                    .host_errors
                    .iter()
                    .map(|range| (range.start as u32, range.end as u32))
                    .collect(),
            ),
            text: Arc::new(program.text),
            source_map: Arc::new(program.segments),
            tape,
        })
    }

    /// The dialect of its tree.
    pub fn dialect(&self) -> GrammarDialect {
        self.tape.language
    }

    /// Its source map's persisted form (`SegmentMap::write`).
    /// The program ranges its tape leaves out (`SegmentMap::omissions`), as little-endian `u32`
    /// pairs: with its text, all its tape depends on.
    pub fn omission_bytes(&self) -> Vec<u8> {
        self.source_map
            .omissions()
            .flat_map(|range| [range.start as u32, range.end as u32])
            .flat_map(u32::to_le_bytes)
            .collect()
    }

    pub fn source_map_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            SegmentMap::persisted_len(self.source_map.len()).unwrap_or_default(),
        );
        self.source_map.write(&mut bytes);
        bytes
    }

    /// Its host errors' persisted form: each range's start and end, little-endian `u32`s.
    pub fn host_error_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.host_errors.len() * 8);
        for &(start, end) in self.host_errors.iter() {
            bytes.extend_from_slice(&start.to_le_bytes());
            bytes.extend_from_slice(&end.to_le_bytes());
        }
        bytes
    }

    fn estimated_bytes(&self) -> usize {
        self.text.capacity()
            + self.source_map.estimated_bytes()
            + self.host_errors.capacity() * std::mem::size_of::<(u32, u32)>()
            + self.tape.estimated_bytes()
    }
}

/// A derived document as the store keeps it (its `file_documents` row, text and tape), before
/// `FileImage::read` checks it.
pub struct StoredDerived<'a> {
    pub ordinal: i64,
    pub parent: i64,
    pub role: i64,
    pub via: i64,
    pub text: &'a [u8],
    pub source_map: &'a [u8],
    pub host_errors: &'a [u8],
    pub tape: &'a [u8],
}

impl FileImage {
    /// The documents of a file that derives none.
    /// The documents of the host tape `primary` of `source`: itself, and the programs its
    /// embedders build from it, one per `via` its dialect embeds. The tape is read in place
    /// (`embed::host`), so a lean parser's host embeds what tree-sitter's does.
    pub fn embedding(primary: TapeImage, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Self, String> {
        let mut image = FileImage { primary, derived: Vec::new() };
        let embeds = grammar.lang.embeds();
        let mut programs = Vec::new();
        if let Some(root) = crate::code_rule_dsl::embed::host::Host::new(&image.primary, grammar).root() {
            for (index, &(via, _)) in embeds.iter().enumerate() {
                if embeds[..index].iter().any(|&(earlier, _)| earlier == via) {
                    continue;
                }
                if let Some(program) = crate::code_rule_dsl::embed::embedder(via).build(root, source, budget) {
                    programs.push((via, program));
                }
            }
        }
        for (via, program) in programs {
            image.derived.push(DerivedImage::encode(via, program, 0, grammar, source, &budget)?);
        }
        // Each document in the format the run stores (decision 0293 §4), once the embedders have read the host.
        Ok(image.in_format(formats::Format::of_run(), source))
    }

    /// The documents of a file that derives none, in the format the run stores (decision 0293 §4).
    pub fn single(primary: TapeImage, source: &str) -> Self {
        Self {
            primary,
            derived: Vec::new(),
        }
        .in_format(formats::Format::of_run(), source)
    }

    /// The documents the store keeps for a file: its tape `primary` in `grammar`'s dialect and
    /// the documents derived from it, in ordinal order from 1. Each derived document is checked
    /// as a tape of its own, and against the document it was derived from: an embedding that
    /// document's dialect has, a tape of that embedding's dialect over its text, a source map and
    /// host errors within both texts, and the role its map gives it.
    pub fn read(
        primary: &[u8],
        grammar: &Arc<Grammar>,
        derived: &[StoredDerived<'_>],
    ) -> Result<Self, String> {
        let mut image = Self {
            primary: TapeImage::from_bytes(primary, grammar)?,
            derived: Vec::with_capacity(derived.len()),
        };
        for (index, stored) in derived.iter().enumerate() {
            if usize::try_from(stored.ordinal).ok() != Some(index + 1) {
                return Err("a file's derived documents are not numbered from 1".into());
            }
            let parent = u32::try_from(stored.parent)
                .ok()
                .filter(|&parent| (parent as usize) <= index)
                .ok_or("a derived document's parent is not an earlier document of its file")?;
            let (parent_dialect, parent_len) = match parent {
                0 => (image.primary.language, image.primary.source_len),
                parent => {
                    let document = &image.derived[parent as usize - 1];
                    (document.tape.language, document.text.len())
                }
            };
            let via = via_of_code(parent_dialect, stored.via).ok_or("unsupported embedding")?;
            let text = std::str::from_utf8(stored.text)
                .map_err(|_| "derived document text is not UTF-8")?
                .to_owned();
            let dialect = stored
                .tape
                .get(8)
                .and_then(|&tag| super::language::from_tape_tag(tag))
                .ok_or("unsupported derived document language")?;
            if !parent_dialect.embeds().contains(&(via, dialect)) {
                return Err("a derived document's dialect is not one its parent embeds".into());
            }
            let tape = TapeImage::from_bytes(stored.tape, &Grammar::embedded(dialect)?)?;
            if tape.source_len != text.len() {
                return Err("a derived document's tape is not of its text".into());
            }
            let source_map = SegmentMap::read(stored.source_map, text.len(), parent_len)?;
            if !stored.host_errors.len().is_multiple_of(8) {
                return Err("truncated derived document host errors".into());
            }
            let host_errors = stored
                .host_errors
                .chunks_exact(8)
                .map(|pair| {
                    (
                        u32::from_le_bytes(pair[..4].try_into().expect("u32 width")),
                        u32::from_le_bytes(pair[4..].try_into().expect("u32 width")),
                    )
                })
                .collect::<Vec<_>>();
            if host_errors.windows(2).any(|pair| pair[0] > pair[1])
                || host_errors
                    .iter()
                    .any(|&(start, end)| start > end || end as usize > parent_len)
            {
                return Err("noncanonical derived document host errors".into());
            }
            let role = DocumentRole::from_code(stored.role)
                .filter(|&role| role == DocumentRole::derived(&text, &source_map))
                .ok_or("a derived document's role is not its source map's")?;
            image.derived.push(DerivedImage {
                role,
                via,
                parent,
                text: Arc::new(text),
                source_map: Arc::new(source_map),
                host_errors: Arc::new(host_errors),
                tape,
            });
        }
        Ok(image)
    }

    /// Every document's image written as the store keeps it and read back (`read`).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn round_trip(&self, grammar: &Arc<Grammar>) -> Result<Self, String> {
        let primary = self.primary.to_bytes()?;
        let parts = self
            .derived
            .iter()
            .map(|document| {
                Ok((
                    document.tape.to_bytes()?,
                    document.source_map_bytes(),
                    document.host_error_bytes(),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let stored = self
            .derived
            .iter()
            .zip(&parts)
            .enumerate()
            .map(
                |(index, (document, (tape, source_map, host_errors)))| StoredDerived {
                    ordinal: index as i64 + 1,
                    parent: i64::from(document.parent),
                    role: document.role.code(),
                    via: via_code(document.via),
                    text: document.text.as_bytes(),
                    source_map,
                    host_errors,
                    tape,
                },
            )
            .collect::<Vec<_>>();
        Self::read(&primary, grammar, &stored)
    }

    pub fn estimated_bytes(&self) -> usize {
        self.primary.estimated_bytes()
            + self
                .derived
                .iter()
                .map(DerivedImage::estimated_bytes)
                .sum::<usize>()
            + self.derived.capacity() * std::mem::size_of::<DerivedImage>()
    }
}

/// What tests restore (`TapeDoc::restore`): a file's documents, or a lone tape, the documents of
/// a file that derives none.
#[cfg(any(test, feature = "test-hooks"))]
pub trait Documents {
    fn file_image(&self) -> Cow<'_, FileImage>;
}

#[cfg(any(test, feature = "test-hooks"))]
impl Documents for FileImage {
    fn file_image(&self) -> Cow<'_, FileImage> {
        Cow::Borrowed(self)
    }
}

#[cfg(any(test, feature = "test-hooks"))]
impl Documents for TapeImage {
    fn file_image(&self) -> Cow<'_, FileImage> {
        // No source: the image is kept in the format it is held in.
        Cow::Owned(FileImage { primary: self.clone(), derived: Vec::new() })
    }
}

#[derive(Clone)]
pub struct Pending {
    pub kf: u16,
    pub parent: Option<u32>,
    pub start: u32,
    pub length: u32,
    pub size: u32,
    pub extra: bool,
    pub error: bool,
    pub missing: bool,
    pub kind_alias: Option<(String, bool)>,
}

pub fn checked(budget: &impl Fn() -> bool) -> Result<(), String> {
    if budget() {
        Err("tape operation interrupted".into())
    } else {
        Ok(())
    }
}

/// The store's identity of a tape (`HashKey::tape_identity`) whose source hashes to `source` and
/// whose stored bytes are `tape`.
pub fn tape_identity(source: &[u8; 32], tape: &[u8]) -> [u8; 32] {
    let key = crate::content_store::HashKey::tape_identity(crate::content_store::HashKey::from_sha256_digest(*source), tape);
    key.sha256_bytes().expect("a SHA-256 identity")
}

fn source_digest(source: &str, budget: &impl Fn() -> bool) -> Result<[u8; 32], String> {
    let mut digest = Sha256::new();
    for chunk in source.as_bytes().chunks(4096) {
        checked(budget)?;
        digest.update(chunk);
    }
    checked(budget)?;
    Ok(digest.finalize().into())
}

fn varint(mut value: u32, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 127) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 128;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

pub fn checkpoint(value: usize, bytes: &mut Vec<u8>) -> Result<(), String> {
    if value > CHECKPOINT_LIMIT {
        return Err("rec6 checkpoint overflow".into());
    }
    bytes.extend_from_slice(&(value as u32).to_le_bytes()[..3]);
    Ok(())
}

struct TapeReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> TapeReader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(length)
            .filter(|&end| end <= self.bytes.len())
            .ok_or("truncated tape image")?;
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("u32 width"),
        ))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("u64 width"),
        ))
    }

    fn length(&mut self) -> Result<usize, String> {
        usize::try_from(self.u64()?).map_err(|_| "tape image length overflow".into())
    }
}

thread_local! {
    /// Every token kept, as formats up to 11 stored them (named-only at rest came with format 12): only for
    /// `capture-tapes --all-tokens`, the tape lab's format-11 baseline (decision 0156, E10).
    static ALL_TOKENS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` with every token kept in the tapes this thread encodes from tree-sitter trees (format 11's nodes).
pub fn with_all_tokens<T>(f: impl FnOnce() -> T) -> T {
    ALL_TOKENS.set(true);
    let out = f();
    ALL_TOKENS.set(false);
    out
}

/// Appends `root`'s tree to `nodes` in preorder, `language` its grammar. An embedded program's
/// placeholders its segments mark (`omitted`) are left out, with their subtrees.
/// Tree-sitter's tree under `root` into `nodes`, in preorder. Where `filter` (the grammar's token
/// automata) is given, a token it implies is skipped as the walk meets it: it is never a record
/// (`tokens::TokenGrammar::stored_children`, RER-131).
fn encode_tree(
    nodes: &mut Vec<Pending>,
    supertypes: &mut Vec<(u32, u16)>,
    root: TsNode<'_>,
    language: &Language,
    filter: Option<&tokens::TokenGrammar>,
    omitted: Option<&SegmentMap>,
    budget: &impl Fn() -> bool,
) -> Result<(), String> {
    // Per kind id, the grammar's name for it and whether it is named, read once: a node whose
    // own name or namedness differs is a contextual alias.
    let mut catalog: Vec<Option<(Option<&str>, bool)>> = vec![None; language.node_kind_count()];
    // One cursor walks the tree in preorder. `parent` is the tape index of the node the cursor's
    // node is a child of; `above` holds the same for each node the cursor descended through.
    let mut cursor = root.walk();
    let mut parent = None;
    let mut above: Vec<Option<u32>> = Vec::new();
    // Which children of the cursor's node's parent are stored (`None`: all), and the cursor's
    // node's place among them; `levels` holds the same for each node the cursor descended through.
    let mut stored: Option<Vec<bool>> = None;
    let mut place = 0usize;
    let mut levels: Vec<(Option<Vec<bool>>, usize)> = Vec::new();
    let mut scratch = tokens::FilterScratch::default();
    let mut filter_cursor = filter.map(|_| root.walk());
    let mut visited = 0usize;
    loop {
        // Every 256 nodes, omitted ones included.
        if visited.is_multiple_of(256) {
            checked(budget)?;
        }
        visited += 1;
        let implied = stored.as_ref().is_some_and(|stored| !stored.get(place).copied().unwrap_or(true));
        // An implied token has no record or descendants. Do not reconstruct its
        // TSNode (including its source position) merely to discard it.
        let retained = (!implied).then(|| cursor.node()).filter(|node| !omitted.is_some_and(|segments| segments.omitted(node.byte_range())));
        if let Some(node) = retained {
            // The cursor's root is `root` itself, which has no field.
            // One status call gives the field and the supertypes (the field alone walks the stack again).
            let (classes, count, field) = cursor_status(&cursor);
            let extra = node.is_extra();
            let field = if extra { 0 } else { field };
            let index = encode_node(nodes, node, parent, field, extra, language, &mut catalog)?;
            for class in classes.into_iter().take(count) {
                if usize::from(class) >= KIND_LIMIT {
                    return Err("supertype symbol exceeds tape limit".into());
                }
                supertypes.push((index, class));
            }
            if cursor.goto_first_child() {
                above.push(parent);
                parent = Some(index);
                levels.push((stored.take(), place));
                stored = filter.and_then(|filter| filter.stored_children_in(node, filter_cursor.as_mut().expect("a cursor for the filter"), &mut scratch));
                place = 0;
                continue;
            }
        }
        // The next sibling, of this node or of its nearest ancestor that has one.
        loop {
            let Some(grandparent) = above.last().copied() else {
                return Ok(());
            };
            if cursor.goto_next_sibling() {
                place += 1;
                break;
            }
            cursor.goto_parent();
            above.pop();
            parent = grandparent;
            if let Some(mask) = stored.take() { scratch.recycle(mask); }
            (stored, place) = levels.pop().unwrap_or_default();
        }
    }
}

/// Appends `node` under `parent` as the child in `field`; its tape index.
fn encode_node<'l>(
    nodes: &mut Vec<Pending>,
    node: TsNode<'_>,
    parent: Option<u32>,
    field: u16,
    extra: bool,
    language: &'l Language,
    catalog: &mut [Option<(Option<&'l str>, bool)>],
) -> Result<u32, String> {
    let index = u32::try_from(nodes.len()).map_err(|_| "rec6 node count overflow")?;
    let code = node.kind_id();
    let kind = usize::from(code);
    // ts_node_is_error repeats ts_node_symbol. The pinned runtime defines it
    // as this comparison; read each node property once across the FFI boundary.
    let error = code == ERROR_SYMBOL;
    let missing = node.is_missing();
    let span = node.byte_range();
    if usize::from(field) >= FIELD_LIMIT {
        return Err(format!(
            "grammar field id {field} exceeds tape limit {FIELD_LIMIT}"
        ));
    }
    if error {
        if missing {
            return Err("unsupported error or missing grammar symbol".into());
        }
    } else if kind >= language.node_kind_count() {
        return Err(format!(
            "grammar symbol {kind} is not in the semantic catalog"
        ));
    } else if usize::from(code) >= KIND_LIMIT {
        return Err(format!(
            "grammar symbol {code} exceeds tape limit {KIND_LIMIT}"
        ));
    } else if code == ERROR_CODE {
        return Err("grammar symbol conflicts with the reserved tape error code".into());
    } else if missing && !span.is_empty() {
        return Err("missing grammar symbol has a non-empty source span".into());
    }
    let start = u32::try_from(span.start).map_err(|_| "node span overflow")?;
    let end = u32::try_from(span.end).map_err(|_| "node span overflow")?;
    let length = end.checked_sub(start).ok_or("inverted node span")?;
    let kind_alias = if error {
        None
    } else {
        let (name, named) = *catalog[kind].get_or_insert_with(|| {
            (
                language.node_kind_for_id(code),
                language.node_kind_is_named(code),
            )
        });
        // SAFETY: `node` is a live node of a tree in `language`, whose kind names are static
        // NUL-terminated strings.
        let own = unsafe { tree_sitter::ffi::ts_node_type(node.into_raw()) };
        // The same string when the same pointer; otherwise compared as Tree-sitter's `kind` would.
        let same_name = name.is_some_and(|name| {
            std::ptr::eq(name.as_ptr(), own.cast())
                || name.as_bytes() == unsafe { std::ffi::CStr::from_ptr(own) }.to_bytes()
        });
        (!same_name || named != node.is_named()).then(|| (node.kind().to_owned(), node.is_named()))
    };
    nodes.push(Pending {
        kf: if error { ERROR_CODE } else { code } | (field << 10),
        parent,
        start,
        length,
        size: 1,
        extra,
        error,
        missing,
        kind_alias,
    });
    Ok(index)
}

impl TapeImage {
    /// The tape as the store keeps it: format 15's bytes, or format 16's (or 16z's) as held.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(self.stored_len());
        self.emit(&mut |b| bytes.extend_from_slice(b))?;
        Ok(bytes)
    }

    /// The tape as the store keeps it (`to_bytes`), written straight into its home, a slab's room (decision 0353
    /// step 4a): the store's writer and every reader take it there.
    pub fn place_stored(&self) -> Result<rerail_tape_format::home::Home, String> {
        let len = self.stored_len();
        placed::place(len, |out| {
            let mut at = 0;
            self.emit(&mut |b| {
                out[at..at + b.len()].copy_from_slice(b);
                at += b.len();
            })
            .map_err(std::io::Error::other)?;
            debug_assert_eq!(at, len, "the stored length");
            Ok(())
        })
        .map_err(|e| e.to_string())
    }

    /// The length of the bytes the store keeps (`to_bytes`).
    pub fn stored_len(&self) -> usize {
        match &self.held {
            Held::F15(c) => {
                let alias_bytes = c.kind_aliases.iter().map(|(name, _)| 5 + name.len()).sum::<usize>();
                let code_bytes = c.codes.iter().map(Code::stored_len).sum::<usize>();
                HEADER_BYTES + c.columns_bytes().len() + code_bytes + alias_bytes
            }
            Held::F16(t) => t.as_bytes().len(),
            Held::F16Z(t) => t.as_bytes().len(),
        }
    }

    /// The bytes the store keeps, in order, to `put`.
    fn emit(&self, put: &mut dyn FnMut(&[u8])) -> Result<(), String> {
        match &self.held {
            Held::F15(c) => self.emit15(c, put),
            Held::F16(t) => {
                put(t.as_bytes());
                Ok(())
            }
            Held::F16Z(t) => {
                put(t.as_bytes());
                Ok(())
            }
        }
    }

    fn emit15(&self, c: &Columns15, put: &mut dyn FnMut(&[u8])) -> Result<(), String> {
        let language =
            super::language::tape_tag(self.language).ok_or("unsupported tape language")?;
        let alias_bytes = c.kind_aliases.iter().map(|(name, _)| 5 + name.len()).sum::<usize>();
        let code_bytes = c.codes.iter().map(Code::stored_len).sum::<usize>();
        let lengths = [c.node_count(), c.codes.len(), code_bytes, c.kind_aliases.len(), alias_bytes];
        put(TAPE_MAGIC);
        put(&[language]);
        put(&c.widths);
        put(&TAPE_FORMAT_VERSION.to_le_bytes());
        put(&(self.source_len as u64).to_le_bytes());
        for length in lengths {
            put(&(length as u64).to_le_bytes());
        }
        put(c.columns_bytes());
        for code in &c.codes {
            put(&code.kf.to_le_bytes());
            put(&[code.flags]);
            put(&code.alias.to_le_bytes());
            put(&[code.class_count]);
            for class in code.classes() {
                put(&class.to_le_bytes());
            }
        }
        for (name, named) in &c.kind_aliases {
            put(&[u8::from(*named)]);
            put(&u32::try_from(name.len()).map_err(|_| "contextual grammar alias length overflow")?.to_le_bytes());
            put(name.as_bytes());
        }
        Ok(())
    }

    /// A stored tape placed in a room (decision 0353 step 4a), read where it lies: format 15's columns, or format 16
    /// (or 16z) as stored. Nothing is copied.
    pub fn from_home(home: rerail_tape_format::home::Home, grammar: &Arc<Grammar>) -> Result<Self, String> {
        use rerail_tape_format::f16::{MAGIC16, MAGIC16Z};
        let magic = home.get(..8).ok_or("a short tape")?;
        if magic != MAGIC16 && magic != MAGIC16Z {
            return Self::read_image_in(&home, grammar, Some(&home));
        }
        Self::sixteen(home, grammar)
    }

    /// A stored tape, in the format it was stored in (its magic): format 15's columns, or format 16 (or 16z) held
    /// as stored.
    pub fn from_bytes(bytes: &[u8], grammar: &Arc<Grammar>) -> Result<Self, String> {
        use rerail_tape_format::f16::{MAGIC16, MAGIC16Z, Tape16, Tape16Z};
        let magic = bytes.get(..8).ok_or("a short tape")?;
        if magic != MAGIC16 && magic != MAGIC16Z {
            return Self::read_image(bytes, grammar);
        }
        Self::sixteen(bytes.to_vec().into(), grammar)
    }

    /// A format 16 or 16z tape held as stored, in `bytes`.
    fn sixteen(bytes: rerail_tape_format::home::Home, grammar: &Arc<Grammar>) -> Result<Self, String> {
        use rerail_tape_format::f16::{MAGIC16, Tape16, Tape16Z};
        let tag = super::language::tape_tag(grammar.lang).ok_or("unsupported tape language")?;
        if bytes.get(header::LANGUAGE) != Some(&tag) {
            return Err("tape image grammar mismatch".into());
        }
        let lang = rerail_tape_format::f16::lang(tag).ok_or("a format 16 tape of a language without its table")?;
        let held = if bytes[..8] == *MAGIC16 {
            let t = Tape16::from_bytes(bytes)?;
            t.reader(lang)?;
            Held::F16(Arc::new(t))
        } else {
            let t = Tape16Z::from_bytes(bytes)?;
            t.reader(lang)?;
            Held::F16Z(Arc::new(t))
        };
        let source_len = match &held {
            Held::F16(t) => t.source_len(),
            Held::F16Z(t) => t.source_len(),
            Held::F15(_) => unreachable!(),
        };
        let image = Self { held, language: grammar.lang, source_len: usize::try_from(source_len).map_err(|_| "source length overflow")?, source_digest: None };
        if image.node_count() == 0 {
            return Err("an empty tape".into());
        }
        Ok(image)
    }

    /// An image of format 15's columns (code, size, base, off, tail), laid out in one buffer.
    fn of15(columns: [&[u8]; 5], widths: [u8; 3], codes: Vec<Code>, kind_aliases: Vec<(String, bool)>, language: GrammarDialect, source_len: usize, source_digest: Option<[u8; 32]>) -> Self {
        let (bytes, at) = Columns15::laid(columns);
        Self { held: Held::F15(Columns15 { bytes: bytes.into(), base: 0, at, widths, codes, kind_aliases, table: Default::default() }), language, source_len, source_digest }
    }

    /// The tape as held.
    pub fn held(&self) -> &Held {
        &self.held
    }

    /// Whether its bytes are placed in a room (a slab's, decision 0353 step 4a), not its own.
    pub fn placed(&self) -> bool {
        self.room().is_some()
    }

    /// The room the tape is placed in (`placed`), which says where the device reads it.
    pub fn room(&self) -> Option<&std::sync::Arc<dyn rerail_tape_format::home::Room>> {
        match &self.held {
            Held::F15(c) => c.bytes.room(),
            Held::F16(t) => t.home().room(),
            Held::F16Z(t) => t.home().room(),
        }
    }

    /// Format 15's columns, where it is held in format 15.
    pub fn f15(&self) -> Option<&Columns15> {
        match &self.held {
            Held::F15(c) => Some(c),
            _ => None,
        }
    }

    /// Format 15's columns, or why a reader of them cannot read this image.
    pub fn columns15(&self) -> Result<&Columns15, String> {
        self.f15().ok_or_else(|| format!("a format {} tape where format 15's columns are read", self.format().name()))
    }

    /// The image a parser wrote as it parsed `source` (its bytes, the engine's: decision 0140), read as
    /// stored, with the source's digest (not stored since format 12), as `encode_rr`'s.
    pub fn parsed(bytes: &[u8], grammar: &Grammar, source: &str, budget: &impl Fn() -> bool) -> Result<Self, String> {
        let mut image = Self::read_image(bytes, grammar)?;
        image.source_digest = Some(source_digest(source, budget)?);
        Ok(image)
    }

    /// The image `bytes` hold, of a `grammar` file.
    fn read_image(bytes: &[u8], grammar: &Grammar) -> Result<Self, String> {
        Self::read_image_in(bytes, grammar, None)
    }

    /// A stored format 15 image (`read_image`), its columns read where they lie in `placed` (whose bytes are
    /// `bytes`, decision 0353 step 4a) or laid out in a buffer of their own.
    fn read_image_in(bytes: &[u8], grammar: &Grammar, placed: Option<&rerail_tape_format::home::Home>) -> Result<Self, String> {
        let mut reader = TapeReader { bytes, offset: 0 };
        if reader.take(8)? != TAPE_MAGIC {
            return Err("wrong tape image magic".into());
        }
        let language = super::language::from_tape_tag(reader.u8()?)
            .ok_or("unsupported tape image language")?;
        let widths: [u8; 3] = reader.take(3)?.try_into().expect("three widths");
        if !matches!(widths[0], 2 | 4) || !matches!(widths[1], 2 | 4) || !matches!(widths[2] & !TAIL_FROM_START, 2 | 4) {
            return Err("noncanonical tape image header".into());
        }
        if reader.u32()? != TAPE_FORMAT_VERSION {
            return Err("unsupported tape image version".into());
        }
        if language != grammar.lang {
            return Err("tape image grammar mismatch".into());
        }
        let source_len = usize::try_from(reader.u64()?).map_err(|_| "source length overflow")?;
        let nodes = reader.length()?;
        let code_count = reader.length()?;
        let code_bytes = reader.length()?;
        let alias_count = reader.length()?;
        let alias_bytes = reader.length()?;
        // Every count must be backed by the bytes before a table is reserved for it.
        let column = |width: usize| nodes.checked_mul(width).ok_or("tape image length overflow");
        let blocks = nodes.div_ceil(CHECKPOINT_STRIDE).checked_mul(4).ok_or("tape image length overflow")?;
        let lengths = [column(2)?, column(usize::from(widths[0]))?, blocks, column(usize::from(widths[1]))?, column(usize::from(widths[2] & !TAIL_FROM_START))?];
        let fixed = lengths
            .into_iter()
            .map(padded)
            .chain([code_bytes, alias_bytes])
            .try_fold(0_usize, |total, length| total.checked_add(length))
            .ok_or("tape image length overflow")?;
        if nodes == 0
            || nodes > u32::MAX as usize
            || code_count == 0
            || code_count > usize::from(u16::MAX) + 1
            || code_count.checked_mul(6).is_none_or(|minimum| minimum > code_bytes)
            || fixed != bytes.len() - reader.offset
        {
            return Err("invalid tape image table lengths".into());
        }
        if alias_count > usize::from(SENTINEL) || alias_count.checked_mul(5).is_none_or(|minimum| minimum > alias_bytes) {
            return Err("invalid contextual grammar alias count".into());
        }
        // Each column, then its padding to `COLUMN_ALIGN`, which must be zero.
        let columns_at = reader.offset;
        let mut columns: [&[u8]; 5] = [&[]; 5];
        for (column, length) in columns.iter_mut().zip(lengths) {
            *column = reader.take(length)?;
            if reader.take(padded(length) - length)?.iter().any(|&byte| byte != 0) {
                return Err("noncanonical tape column padding".to_owned());
            }
        }
        #[cfg(any(test, feature = "test-hooks"))]
        note_table_reservation(code_count);
        let codes_end = reader.offset + code_bytes;
        let mut codes = Vec::with_capacity(code_count);
        for _ in 0..code_count {
            let kf = u16::from_le_bytes(reader.take(2)?.try_into().expect("u16 width"));
            let flags = reader.u8()?;
            let alias = u16::from_le_bytes(reader.take(2)?.try_into().expect("u16 width"));
            let class_count = reader.u8()?;
            let allowed = CODE_EXTRA | CODE_ERROR | CODE_MISSING;
            if flags & !allowed != 0
                || (alias != SENTINEL && usize::from(alias) >= alias_count)
                || usize::from(class_count) > MAX_NODE_SUPERTYPES
            {
                return Err("noncanonical tape code".into());
            }
            let mut classes = [0_u16; MAX_NODE_SUPERTYPES];
            for slot in classes.iter_mut().take(usize::from(class_count)) {
                *slot = u16::from_le_bytes(reader.take(2)?.try_into().expect("u16 width"));
            }
            let code = Code { kf, flags, alias, class_count, classes };
            if code.classes().windows(2).any(|pair| pair[0] >= pair[1])
                || code.classes().iter().any(|&class| usize::from(class) >= grammar.kinds.kind_count())
            {
                return Err("noncanonical supertype membership".into());
            }
            codes.push(code);
        }
        if reader.offset != codes_end {
            return Err("noncanonical tape code table length".into());
        }
        let aliases_end = reader.offset + alias_bytes;
        #[cfg(any(test, feature = "test-hooks"))]
        note_table_reservation(alias_count);
        let mut kind_aliases = Vec::with_capacity(alias_count);
        for _ in 0..alias_count {
            let named = match reader.u8()? {
                0 => false,
                1 => true,
                _ => return Err("noncanonical contextual grammar alias flag".into()),
            };
            let length = usize::try_from(reader.u32()?)
                .map_err(|_| "contextual grammar alias length overflow")?;
            let name = std::str::from_utf8(reader.take(length)?)
                .map_err(|_| "contextual grammar alias is not UTF-8")?
                .to_owned();
            kind_aliases.push((name, named));
        }
        if reader.offset != aliases_end {
            return Err("noncanonical contextual grammar alias length".into());
        }
        if reader.offset != bytes.len() {
            return Err("trailing tape image bytes".into());
        }
        if let Some(home) = placed {
            // The columns as the stored image lays them, from `columns_at`: each padded to `COLUMN_ALIGN`.
            let mut at = [(0, 0); 5];
            let mut start = 0;
            for (k, column) in columns.iter().enumerate() {
                at[k] = (start, column.len());
                start = padded(start + column.len());
            }
            let c = Columns15 { bytes: home.clone(), base: columns_at, at, widths, codes, kind_aliases, table: Default::default() };
            return Ok(Self { held: Held::F15(c), language, source_len, source_digest: None });
        }
        Ok(Self::of15(columns, widths, codes, kind_aliases, language, source_len, None))
    }

    /// Parses `source` as `grammar`'s dialect and encodes its documents: with tree-sitter (a PHP
    /// file's view, `php_view`, against `source`), or a Blade template with Rerail's port of Forte
    /// (`encode_template`). `None` where the parser stopped before it read the file: tree-sitter
    /// gave up (`budget` interrupted it), or Forte reached a limit.
    pub fn parse(
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Option<Result<FileImage, String>> {
        // A dialect its language parses itself (Blade's Forte), or reads with a parser standing in for
        // tree-sitter's (JSON's Bitlex parser, which may leave the file to tree-sitter-json below).
        if let Some(parser) = tree_parser(grammar.lang)
            && !(ALL_TOKENS.get() && grammar.kinds.grammar().is_some())
            && let Some(parsed) = parser.parse(grammar, source, &budget)
        {
            return match parsed {
                Ok(image) => Some(Ok(image)),
                // The parser stopped at a limit: like tree-sitter giving up, it read nothing.
                Err(TemplateError::Parser(_)) => None,
                Err(TemplateError::Encode(error)) => Some(Err(error)),
            };
        }
        // A `.rerail` file: the rule engine's own parser builds its tree (decision 0061).
        if grammar.lang == GrammarDialect::Rr {
            return Some(Self::encode_rr(grammar, source, budget).map(|image| FileImage::single(image, source)));
        }
        let mut parser = tree_sitter::Parser::new();
        if let Err(error) = parser.set_language(&grammar.lang.get_ts_language()) {
            return Some(Err(error.to_string()));
        }
        let parsed = super::language::base(super::language::Family::of(grammar.lang)).parse_input(source);
        let bytes = parsed.as_bytes();
        let mut input = |offset: usize, _: tree_sitter::Point| &bytes[offset..];
        let mut progress = |_: &tree_sitter::ParseState| {
            if budget() {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        };
        let parse = crate::code_rule_dsl::logic::profile::span(|| "tree-sitter parse".into());
        let tree = parser.parse_with_options(
            &mut input,
            None,
            Some(tree_sitter::ParseOptions::new().progress_callback(&mut progress)),
        )?;
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, bytes.len() as u64);
        drop(parse);
        let _encode = crate::code_rule_dsl::logic::profile::span(|| "tape encode".into());
        if crate::code_rule_dsl::logic::profile::on() {
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, tree.root_node().descendant_count() as u64);
        }
        Some(Self::encode(tree.root_node(), grammar, source, budget))
    }

    /// The documents of a tree-sitter tree of `source`: its tape, and the programs its embedders
    /// build from the tape (`FileImage::embedding`). (Blade's compiled view is built from its
    /// Forte tree: `encode_blade`.)
    pub fn encode(
        root: TsNode<'_>,
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Result<FileImage, String> {
        FileImage::embedding(Self::encode_tree_image(root, grammar, source, None, budget)?, grammar, source, budget)
    }

    /// The tape of a tree-sitter tree of `source` alone, leaving out the placeholders `omitted`
    /// marks (an embedded program's).
    pub fn encode_tree_image(
        root: TsNode<'_>,
        grammar: &Grammar,
        source: &str,
        omitted: Option<&SegmentMap>,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        checked(budget)?;
        let Some(language) = grammar.kinds.grammar() else {
            return Err("a Blade template is encoded from its Forte tree".into());
        };
        if *root.language() != *language {
            return Err("parsed tree grammar mismatch".into());
        }
        if source.len() > u32::MAX as usize
            || root.start_byte() > root.end_byte()
            || root.end_byte() > source.len()
            || !source.is_char_boundary(root.start_byte())
            || !source.is_char_boundary(root.end_byte())
        {
            return Err("source exceeds tape range or does not match parsed root".into());
        }
        let mut nodes = Vec::<Pending>::new();
        let mut supertypes = Vec::new();
        let filter = if ALL_TOKENS.get() { None } else { tokens::TokenGrammar::of(grammar.lang, language) };
        encode_tree(&mut nodes, &mut supertypes, root, language, filter.as_deref(), omitted, budget)?;
        Self::finish(nodes, supertypes, grammar, source, budget)
    }

    /// The tape of a `.rerail` file: the tree `logic::rr::syntax` builds as it reads the statements, its comments as
    /// extras and each statement that does not parse an ERROR leaf (decision 0061). The lean parser writes it as
    /// the image's bytes as it parses (decision 0140), so nothing here packs a node: the image is read as stored.
    pub fn encode_rr(grammar: &Grammar, source: &str, budget: &impl Fn() -> bool) -> Result<Self, String> {
        checked(budget)?;
        if grammar.lang != GrammarDialect::Rr {
            return Err("only a .rerail file is encoded from its rule tree".into());
        }
        if source.len() > u32::MAX as usize {
            return Err("source exceeds tape range".into());
        }
        // In the run's format, written as the parser goes (`formats::rr_tape`). A parsed image carries its source's
        // digest, as `finish`'s does (not stored since format 12): the documents and the memo are keyed by it.
        match formats::rr_tape(source) {
            formats::RrTape::Image(bytes) => {
                let mut image = Self::read_image(&bytes, grammar)?;
                image.source_digest = Some(source_digest(source, budget)?);
                Ok(image)
            }
            formats::RrTape::Written(written) => Self::written_16(written, grammar.lang, source, budget),
        }
    }

    /// The image of `nodes`, in preorder with their parents set.
    pub fn finish(
        mut nodes: Vec<Pending>,
        mut supertypes: Vec<(u32, u16)>,
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        checked(budget)?;
        for index in (1..nodes.len()).rev() {
            if index % 256 == 0 {
                checked(budget)?;
            }
            let parent = nodes[index].parent.ok_or("orphan node")? as usize;
            let size = nodes[index].size;
            nodes[parent].size = nodes[parent]
                .size
                .checked_add(size)
                .ok_or("subtree overflow")?;
        }
        for (index, node) in nodes.iter().enumerate() {
            match node.parent {
                Some(parent) if (parent as usize) < index => {}
                None if index == 0 => {}
                _ => return Err("invalid preorder parent".into()),
            }
        }
        Self::pack(&nodes, std::mem::take(&mut supertypes), grammar, source, budget)
    }

    /// The image without the nodes, and their subtrees, whose spans `segments` omits (an embedded
    /// program's placeholders): the image of the tree `encode_tree_image` walks with the same
    /// `omitted`. The image itself where nothing is omitted.
    pub fn omitting(&self, segments: &SegmentMap, grammar: &Grammar, source: &str, budget: &impl Fn() -> bool) -> Result<Self, String> {
        // Read through the tape API, in the format the parser wrote (a lean parser's may be format 16, 0293 §7).
        let omitted = rerail_tape_format::with_tape!(self.tape(), |t| {
            (0..t.len()).any(|i| segments.omitted(t.start(i) as usize..t.end(i) as usize))
        });
        if !omitted {
            return Ok(self.clone());
        }
        let (nodes, supertypes) = rerail_tape_format::with_tape!(self.tape(), |t| Self::kept(&t, segments, budget))?;
        Self::finish(nodes, supertypes, grammar, source, budget)
    }

    /// The nodes of `t` that `segments` does not omit, nor any of their ancestors, as `finish` takes them.
    fn kept<T: Tape>(t: &T, segments: &SegmentMap, budget: &impl Fn() -> bool) -> Result<(Vec<Pending>, Vec<(u32, u16)>), String> {
        let count = t.len();
        let mut nodes = Vec::<Pending>::with_capacity(count);
        let mut supertypes = Vec::new();
        // Per open ancestor: its index in `nodes`, and where its subtree ends in `t`.
        let mut open: Vec<(u32, usize)> = Vec::new();
        let mut i = 0;
        while i < count {
            if i % 256 == 0 {
                checked(budget)?;
            }
            while open.last().is_some_and(|&(_, last)| last <= i) {
                open.pop();
            }
            let (start, end, until) = (t.start(i), t.end(i), t.subtree_end(i));
            if segments.omitted(start as usize..end as usize) {
                i = until;
                continue;
            }
            let code = t.table().entry(t.code(i));
            let index = u32::try_from(nodes.len()).map_err(|_| "rec6 node count overflow")?;
            let start = u32::try_from(start).map_err(|_| "span outside source")?;
            nodes.push(Pending {
                kf: code.kind | code.field << rerail_tape_format::KIND_BITS,
                parent: open.last().map(|&(at, _)| at),
                start,
                length: u32::try_from(end).map_err(|_| "span outside source")? - start,
                size: 1,
                extra: code.flags & CODE_EXTRA != 0,
                error: code.flags & CODE_ERROR != 0,
                missing: code.flags & CODE_MISSING != 0,
                kind_alias: None,
            });
            if code.alias.is_some() {
                return Err("a written tape's contextual alias".into());
            }
            supertypes.extend(code.classes.iter().map(|&class| (index, class)));
            open.push((index, until));
            i += 1;
        }
        Ok((nodes, supertypes))
    }

    /// An image from columns a parser wrote itself, in this format (codes numbered by first use, `base`
    /// per `CHECKPOINT_STRIDE` nodes, each of `size`, `off` and `tail` at its narrowest width): no node is
    /// packed again, and only the columns' lengths are checked (a code's range is the writer's: a node's code
    /// is numbered as the node is written). `digest`: the source's SHA-256, where the parser's pass computed
    /// it. Not generic, so it is compiled once, here.
    #[allow(clippy::too_many_arguments)]
    pub fn written(
        code: Vec<u8>,
        size: Vec<u8>,
        base: Vec<u8>,
        off: Vec<u8>,
        tail: Vec<u8>,
        widths: [u8; 3],
        codes: Vec<Code>,
        digest: Option<[u8; 32]>,
        grammar: &Grammar,
        source: &str,
        budget: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        let count = code.len() / 2;
        let [size_width, off_width, tail_width] = widths.map(|w| usize::from(w & !TAIL_FROM_START));
        if count == 0
            || code.len() % 2 != 0
            || ![size_width, off_width, tail_width].iter().all(|w| matches!(w, 2 | 4))
            || size.len() != count * size_width
            || off.len() != count * off_width
            || tail.len() != count * tail_width
            || base.len() != count.div_ceil(CHECKPOINT_STRIDE) * 4
            || codes.is_empty()
        {
            return Err("a written tape's columns do not agree".into());
        }
        // The parser's pass hashed the source as it read it, or it is hashed here.
        let digest = match digest {
            Some(digest) => digest,
            None => source_digest(source, &budget)?,
        };
        Ok(Self::of15([&code, &size, &base, &off, &tail], widths, codes, Vec::new(), grammar.lang, source.len(), Some(digest)))
    }

    /// A written tape whose codes name contextual aliases (`Code::alias`, an index into
    /// `kind_aliases`): a kind whose symbol's own name or namedness is not the node's, as
    /// `encode_node` finds it. The aliases numbered by first use, as `pack` numbers them.
    pub fn with_kind_aliases(mut self, kind_aliases: Vec<(String, bool)>) -> Result<Self, String> {
        // A parser writes format 15's columns (`written`); a tape held in another format came from a store.
        let Held::F15(c) = &mut self.held else { return Err("a written tape is held in format 15".into()) };
        if kind_aliases.len() >= usize::from(SENTINEL)
            || c.codes.iter().any(|code| code.alias != SENTINEL && usize::from(code.alias) >= kind_aliases.len())
        {
            return Err("a written tape's aliases do not agree with its codes".into());
        }
        c.kind_aliases = kind_aliases;
        // The table as the tape API reads it names the aliases: made again from them.
        c.table = Default::default();
        Ok(self)
    }

    /// The columns of `nodes`, in preorder with their sizes set: codes folded and numbered by
    /// first use, `base` per 16 nodes, and each column at its narrowest width.
    fn pack(
        nodes: &[Pending],
        mut supertypes: Vec<(u32, u16)>,
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let count = nodes.len();
        if count == 0 || u32::try_from(count).is_err() {
            return Err("tape node count out of range".into());
        }
        if nodes.iter().enumerate().any(|(i, n)| n.size == 0 || i + n.size as usize > count) {
            return Err("a subtree size out of range".into());
        }
        supertypes.sort_unstable();
        supertypes.dedup();
        let mut memberships = supertypes.iter().peekable();
        let mut kind_aliases = Vec::<(String, bool)>::new();
        let mut codes = Vec::<Code>::new();
        // One lookup per node: FxHash, not SipHash (4% of a parse).
        let mut code_ids = rustc_hash::FxHashMap::<Code, u16>::default();
        let mut code = Vec::with_capacity(count * 2);
        for (index, node) in nodes.iter().enumerate() {
            if index % 256 == 0 {
                checked(budget)?;
            }
            let alias = match &node.kind_alias {
                Some(alias) => {
                    let id = kind_aliases.iter().position(|candidate| candidate == alias).unwrap_or_else(|| {
                        kind_aliases.push(alias.clone());
                        kind_aliases.len() - 1
                    });
                    u16::try_from(id).ok().filter(|&id| id != SENTINEL).ok_or("too many contextual grammar aliases")?
                }
                None => SENTINEL,
            };
            let flags = u8::from(node.extra) * CODE_EXTRA | u8::from(node.error) * CODE_ERROR | u8::from(node.missing) * CODE_MISSING;
            let mut classes = [0_u16; MAX_NODE_SUPERTYPES];
            let mut class_count = 0_usize;
            while let Some(&(_, class)) = memberships.next_if(|&&(at, _)| at as usize == index) {
                *classes.get_mut(class_count).ok_or("too many supertype memberships")? = class;
                class_count += 1;
            }
            let folded = Code { kf: node.kf, flags, alias, class_count: class_count as u8, classes };
            let id = match code_ids.get(&folded) {
                Some(&id) => id,
                None => {
                    let id = u16::try_from(codes.len()).map_err(|_| "too many folded tape codes")?;
                    codes.push(folded);
                    code_ids.insert(folded, id);
                    id
                }
            };
            code.extend_from_slice(&id.to_le_bytes());
        }
        // Each block's least start, and whether every node ends at or after its last
        // descendant's start.
        let bases = nodes
            .chunks(CHECKPOINT_STRIDE)
            .map(|block| block.iter().map(|node| node.start).min().expect("nonempty block"))
            .collect::<Vec<_>>();
        let end = |node: &Pending| node.start.checked_add(node.length).ok_or("span end overflow");
        let mut from_last = true;
        let (mut max_off, mut max_tail, mut max_length) = (0_u32, 0_u32, 0_u32);
        for (index, node) in nodes.iter().enumerate() {
            let last = &nodes[index + node.size as usize - 1];
            match end(node)?.checked_sub(last.start) {
                Some(tail) => max_tail = max_tail.max(tail),
                None => from_last = false,
            }
            max_length = max_length.max(node.length);
            max_off = max_off.max(node.start - bases[index / CHECKPOINT_STRIDE]);
        }
        let size_width = width_for(count as u32);
        let off_width = width_for(max_off);
        let tail_width = width_for(if from_last { max_tail } else { max_length });
        let (mut size, mut off, mut tail) = (
            Vec::with_capacity(count * size_width),
            Vec::with_capacity(count * off_width),
            Vec::with_capacity(count * tail_width),
        );
        for (index, node) in nodes.iter().enumerate() {
            if index % 256 == 0 {
                checked(budget)?;
            }
            put_cell(&mut size, node.size, size_width);
            put_cell(&mut off, node.start - bases[index / CHECKPOINT_STRIDE], off_width);
            let from = if from_last { nodes[index + node.size as usize - 1].start } else { node.start };
            put_cell(&mut tail, end(node)? - from, tail_width);
        }
        if memberships.next().is_some() {
            return Err("supertype membership of no node".into());
        }
        checked(budget)?;
        let widths = [size_width as u8, off_width as u8, tail_width as u8 | if from_last { 0 } else { TAIL_FROM_START }];
        let base: Vec<u8> = bases.iter().flat_map(|base| base.to_le_bytes()).collect();
        Ok(Self::of15([&code, &size, &base, &off, &tail], widths, codes, kind_aliases, grammar.lang, source.len(), Some(source_digest(source, budget)?)))
    }

    /// The format it is held in.
    pub fn format(&self) -> formats::Format {
        match self.held {
            Held::F15(_) => formats::Format::F15,
            Held::F16(_) => formats::Format::F16,
            Held::F16Z(_) => formats::Format::F16Z,
        }
    }

    /// The tape, read in place in the format it is held in (decision 0293).
    pub fn tape(&self) -> rerail_tape_format::tape::AnyTape<'_> {
        use rerail_tape_format::tape::{AnyTape, F15};
        let lang = |tag: u8| rerail_tape_format::f16::lang(tag).expect("a format 16 tape is held with its language's table");
        match &self.held {
            Held::F15(c) => AnyTape::F15(F15::over(c.laid_out().0, c.view(), self.code_table())),
            Held::F16(t) => AnyTape::F16(t.reader(lang(t.tag())).expect("checked when held")),
            Held::F16Z(t) => AnyTape::F16Z(t.reader(lang(t.tag())).expect("checked when held")),
        }
    }

    /// Its code table, as the tape API numbers its codes: format 15's own, or the language's.
    pub fn code_table(&self) -> &rerail_tape_format::tape::CodeTable {
        match &self.held {
            Held::F15(c) => c.table.get_or_init(|| c.code_table()),
            _ => self.lang().expect("a format 16 tape is held with its language's table").table(),
        }
    }

    /// The preorders of nodes whose code carries `flag`.
    #[cfg(any(test, feature = "test-hooks"))]
    fn flagged(&self, flag: u8) -> Vec<u32> {
        use rerail_tape_format::tape::Tape;
        rerail_tape_format::with_tape!(self.tape(), |t| (0..t.len()).filter(|&i| t.table().entry(t.code(i)).flags & flag != 0).map(|i| i as u32).collect())
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn error_nodes(&self) -> Vec<u32> {
        self.flagged(CODE_ERROR)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn missing_nodes(&self) -> Vec<u32> {
        self.flagged(CODE_MISSING)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn extra_nodes(&self) -> Vec<u32> {
        self.flagged(CODE_EXTRA)
    }

    /// The supertype memberships the tape holds.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn membership_count(&self) -> usize {
        use rerail_tape_format::tape::Tape;
        rerail_tape_format::with_tape!(self.tape(), |t| (0..t.len()).map(|i| t.table().entry(t.code(i)).classes.len()).sum())
    }

    /// The nodes that carry a contextual alias.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn aliased_nodes(&self) -> usize {
        use rerail_tape_format::tape::Tape;
        rerail_tape_format::with_tape!(self.tape(), |t| (0..t.len()).filter(|&i| t.table().entry(t.code(i)).alias.is_some()).count())
    }

    pub fn node_count(&self) -> usize {
        match &self.held {
            Held::F15(c) => c.node_count(),
            Held::F16(t) => t.len(),
            Held::F16Z(t) => t.len(),
        }
    }

    /// Node `i`'s subtree size, itself included.
    pub fn subtree_size(&self, i: usize) -> u32 {
        use rerail_tape_format::tape::Tape;
        match &self.held {
            Held::F15(c) => c.size_at(i),
            _ => rerail_tape_format::with_tape!(self.tape(), |t| (t.subtree_end(i) - i) as u32),
        }
    }

    /// The bytes it holds.
    pub(super) fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.held {
                Held::F15(c) => c.estimated_bytes(),
                Held::F16(t) => t.bytes(),
                Held::F16Z(t) => t.bytes(),
            }
    }
}

/// Tests of format 15's columns read and break them on the image (a format 16 image has none: the test fails).
#[cfg(any(test, feature = "test-hooks"))]
impl std::ops::Deref for TapeImage {
    type Target = Columns15;
    fn deref(&self) -> &Columns15 {
        self.f15().expect("a format 15 image's columns")
    }
}

#[cfg(any(test, feature = "test-hooks"))]
impl std::ops::DerefMut for TapeImage {
    fn deref_mut(&mut self) -> &mut Columns15 {
        match &mut self.held {
            Held::F15(c) => {
                // A test changes the columns: the API's table is made again from them.
                c.table = Default::default();
                c
            }
            _ => panic!("a format 15 image's columns"),
        }
    }
}

impl Columns15 {
    pub const CODE: usize = 0;
    pub const SIZE: usize = 1;
    pub const BASE: usize = 2;
    pub const OFF: usize = 3;
    pub const TAIL: usize = 4;

    /// The columns laid out in one buffer, as the stored image lays them.
    fn laid(columns: [&[u8]; 5]) -> (Vec<u8>, [(usize, usize); 5]) {
        let mut bytes = Vec::with_capacity(columns.iter().map(|column| padded(column.len())).sum());
        let mut at = [(0, 0); 5];
        for (k, column) in columns.into_iter().enumerate() {
            at[k] = (bytes.len(), column.len());
            bytes.extend_from_slice(column);
            bytes.resize(padded(bytes.len()), 0);
        }
        // The reads' offsets are u32s: a source's columns are far smaller.
        assert!(u32::try_from(bytes.len()).is_ok(), "format 15's columns past 4 GiB");
        (bytes, at)
    }

    /// Column `k` (`CODE` … `TAIL`).
    pub fn column(&self, k: usize) -> &[u8] {
        let (start, length) = self.at[k];
        &self.bytes[self.base + start..self.base + start + length]
    }

    /// The columns, laid out from `base`, each padded.
    fn columns_bytes(&self) -> &[u8] {
        let (start, length) = self.at[Self::TAIL];
        &self.bytes[self.base..self.base + padded(start + length)]
    }

    pub fn code(&self) -> &[u8] {
        self.column(Self::CODE)
    }

    pub fn size(&self) -> &[u8] {
        self.column(Self::SIZE)
    }

    pub fn base(&self) -> &[u8] {
        self.column(Self::BASE)
    }

    pub fn off(&self) -> &[u8] {
        self.column(Self::OFF)
    }

    pub fn tail(&self) -> &[u8] {
        self.column(Self::TAIL)
    }

    /// The buffer every column lies in, and where each starts in it: what a reader in place takes.
    pub fn laid_out(&self) -> (&[u8], [usize; 5]) {
        (self.columns_bytes(), self.at.map(|(start, _)| start))
    }

    /// A test's change of column `k` (its bytes, or its length): the columns laid out again.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn edit(&mut self, k: usize, change: impl FnOnce(&mut Vec<u8>)) {
        let mut columns: Vec<Vec<u8>> = (0..5).map(|k| self.column(k).to_vec()).collect();
        change(&mut columns[k]);
        let (bytes, at) = Self::laid(std::array::from_fn(|k| columns[k].as_slice()));
        (self.bytes, self.base, self.at) = (bytes.into(), 0, at);
        self.table = Default::default();
    }

    pub fn size_width(&self) -> usize {
        usize::from(self.widths[0])
    }

    pub fn off_width(&self) -> usize {
        usize::from(self.widths[1])
    }

    pub fn tail_width(&self) -> usize {
        usize::from(self.widths[2] & !TAIL_FROM_START)
    }

    pub fn code_at(&self, i: usize) -> u16 {
        u16::from_le_bytes([self.code()[2 * i], self.code()[2 * i + 1]])
    }

    pub fn size_at(&self, i: usize) -> u32 {
        cell(self.size(), i, self.size_width())
    }

    pub fn start_at(&self, i: usize) -> u32 {
        cell(self.base(), i / CHECKPOINT_STRIDE, 4).saturating_add(cell(self.off(), i, self.off_width()))
    }

    pub fn tail_at(&self, i: usize) -> u32 {
        cell(self.tail(), i, self.tail_width())
    }

    pub fn node_count(&self) -> usize {
        self.code().len() / 2
    }

    /// The code table's stored bytes.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn code_table_bytes(&self) -> usize {
        self.codes.iter().map(Code::stored_len).sum()
    }

    fn estimated_bytes(&self) -> usize {
        // A placed image's bytes are its slab's, counted there.
        let own = match &self.bytes {
            rerail_tape_format::home::Home::Own(v) => v.capacity(),
            rerail_tape_format::home::Home::Placed(_) => 0,
        };
        own + self.codes.capacity() * std::mem::size_of::<Code>()
            + self.kind_aliases.capacity() * std::mem::size_of::<(String, bool)>()
            + self.kind_aliases.iter().map(|(name, _)| name.capacity()).sum::<usize>()
    }
}

/// E7 (draft decision 0156): how often readers take each part of a node, so format 16 knows which parts a
/// reader needs decoded and which it may derive or leave compressed. Off unless the `tape-reads` feature is
/// on; with it, a relaxed add per read and a table on stderr when a `rerail-engine` command ends.
#[cfg(feature = "tape-reads")]
pub mod reads {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    pub const PARTS: [&str; 15] = [
        "kind", "field", "flags", "named", "span", "points", "parent", "children", "prev", "size", "classes",
        "docs restored", "nodes restored", "facts docs", "facts nodes",
    ];
    static COUNTS: [AtomicU64; PARTS.len()] = [const { AtomicU64::new(0) }; PARTS.len()];

    pub fn hit(part: usize, n: u64) {
        COUNTS[part].fetch_add(n, Relaxed);
    }

    pub const fn part(name: &str) -> usize {
        let mut i = 0;
        while !eq(PARTS[i], name) {
            i += 1;
        }
        i
    }

    const fn eq(a: &str, b: &str) -> bool {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        let mut i = 0;
        while i < a.len() {
            if a[i] != b[i] {
                return false;
            }
            i += 1;
        }
        true
    }

    /// E17 (t39 step 9): how each read of a span, a parent or a subtree end lies against the reader's
    /// previous read of the same part on this thread: the same node, the next one, near (16 nodes or fewer
    /// away), or far (a point read). A slice of a column read whole is in order ("bulk", its length).
    pub const PATTERN_PARTS: [&str; 7] = [
        "tape span", "tape parent", "tape size", "facts start", "facts end", "facts parent", "facts subtree_end",
    ];
    pub const CLASSES: [&str; 5] = ["same", "next", "near", "far", "bulk"];
    static PATTERNS: [[AtomicU64; CLASSES.len()]; PATTERN_PARTS.len()] =
        [const { [const { AtomicU64::new(0) }; CLASSES.len()] }; PATTERN_PARTS.len()];
    thread_local! {
        static LAST: [std::cell::Cell<u32>; PATTERN_PARTS.len()] = const { [const { std::cell::Cell::new(u32::MAX) }; PATTERN_PARTS.len()] };
        /// Builders that decode (facts from a tape) are not readers: their reads are not counted.
        static PAUSED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    pub const fn pattern_part(name: &str) -> usize {
        let mut i = 0;
        while !eq(PATTERN_PARTS[i], name) {
            i += 1;
        }
        i
    }

    /// With `E17_SITES` set, each read is also counted by its caller's source location (slow: a lock a read).
    static SITES_ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("E17_SITES").is_some());
    type Site = (usize, &'static std::panic::Location<'static>);
    static SITES: std::sync::Mutex<Option<std::collections::HashMap<Site, [u64; 4]>>> = std::sync::Mutex::new(None);

    #[inline]
    #[track_caller]
    pub fn at(part: usize, node: u32) {
        if PAUSED.with(|p| p.get()) != 0 {
            return;
        }
        let last = LAST.with(|l| l[part].replace(node));
        let class = if node == last {
            0
        } else if last != u32::MAX && node == last.wrapping_add(1) {
            1
        } else if last != u32::MAX && node.abs_diff(last) <= 16 {
            2
        } else {
            3
        };
        PATTERNS[part][class].fetch_add(1, Relaxed);
        if *SITES_ON {
            let site = std::panic::Location::caller();
            SITES.lock().expect("the sites").get_or_insert_default().entry((part, site)).or_default()[class] += 1;
        }
    }

    #[inline]
    pub fn bulk(part: usize, n: usize) {
        if PAUSED.with(|p| p.get()) == 0 {
            PATTERNS[part][4].fetch_add(n as u64, Relaxed);
        }
    }

    pub struct Paused;
    impl Drop for Paused {
        fn drop(&mut self) {
            PAUSED.with(|p| p.set(p.get() - 1));
        }
    }
    /// Reads are not counted until the guard drops (a builder decoding a tape into facts).
    pub fn pause() -> Paused {
        PAUSED.with(|p| p.set(p.get() + 1));
        Paused
    }

    pub fn report() {
        let nodes = COUNTS[part("nodes restored")].load(Relaxed).max(1) as f64;
        eprintln!("tape reads (E7):");
        for (name, count) in PARTS.iter().zip(&COUNTS) {
            let count = count.load(Relaxed);
            eprintln!("  {name:<16} {count:>14} {:>10.3} per restored node", count as f64 / nodes);
        }
        let facts = COUNTS[part("facts nodes")].load(Relaxed).max(1) as f64;
        eprintln!("read patterns (E17), per facts node ({} facts nodes):", facts as u64);
        eprintln!("  {:<18} {}", "", CLASSES.iter().map(|c| format!("{c:>12}")).collect::<String>());
        for (name, row) in PATTERN_PARTS.iter().zip(&PATTERNS) {
            eprintln!("  {name:<18} {}", row.iter().map(|c| format!("{:>12.4}", c.load(Relaxed) as f64 / facts)).collect::<String>());
        }
        eprintln!("read patterns (E17), counts:");
        for (name, row) in PATTERN_PARTS.iter().zip(&PATTERNS) {
            eprintln!("  {name:<18} {}", row.iter().map(|c| format!("{:>12}", c.load(Relaxed))).collect::<String>());
        }
        if let Some(sites) = SITES.lock().expect("the sites").take() {
            let mut sites: Vec<_> = sites.into_iter().collect();
            sites.sort_by_key(|(_, c)| std::cmp::Reverse(c.iter().sum::<u64>()));
            eprintln!("read patterns (E17), top sites (same, next, near, far):");
            for ((part, site), c) in sites.iter().take(40) {
                eprintln!("  {:<18} {:>11} {:>11} {:>11} {:>11}  {site}", PATTERN_PARTS[*part], c[0], c[1], c[2], c[3]);
            }
        }
    }
}

/// Counts a read of `$part` (`reads::PATTERN_PARTS`) of node `$node`, by where it lies (E17).
macro_rules! read_at {
    ($part:literal, $node:expr) => {
        #[cfg(feature = "tape-reads")]
        {
            const PART: usize = reads::pattern_part($part);
            reads::at(PART, $node as u32);
        }
    };
}

/// Counts a read of `$part` (`reads::PARTS`), `$n` times.
macro_rules! read {
    ($part:literal) => {
        read!($part, 1)
    };
    ($part:literal, $n:expr) => {
        #[cfg(feature = "tape-reads")]
        {
            const PART: usize = reads::part($part);
            reads::hit(PART, $n as u64);
        }
    };
}

/// A node's kind as the matchers name it: the code's, or `ERROR_SYMBOL` for an ERROR node.
#[inline]
fn kind_of(e: &Entry) -> u16 {
    if e.flags & CODE_ERROR != 0 { ERROR_SYMBOL } else { e.kind }
}

/// Whether a node of code `e` is named: its alias's flag, else its kind's.
#[inline]
fn named_of(grammar: &Grammar, e: &Entry) -> bool {
    e.alias.as_ref().map_or_else(|| grammar.kind_is_named(kind_of(e)), |alias| alias.1)
}

/// A node whose parent the step that reached it did not know (`TapeNode::up`).
const UNKNOWN: u32 = u32::MAX - 1;

#[derive(Clone)]
pub struct TapeDoc {
    // P's: the source's digest, read by P's anchor corpus (`matcher::p_anchors`, test-only).
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    source_digest: [u8; 32],
    pub source: Arc<String>,
    grammar: Arc<Grammar>,
    lines: Arc<LineTable>,
    points: Arc<Points>,
    /// Derived classes (`with_labels`): a bitset over the nodes per class from
    /// `DERIVED_CLASS_BASE`.
    labels: Arc<Vec<Vec<u64>>>,
    /// Text tests read from the term index (`with_texts`, `prefilter::terms`, `R[tid]`): per
    /// slot key (`Micro::slot_key`), a bit per node.
    texts: Arc<std::collections::BTreeMap<String, Arc<Vec<u64>>>>,
    /// A host's guests (`embed`); the host a guest reads links to them weakly, as they hold it.
    guests: Guests,
    /// A guest's link to the file it is embedded in.
    pub host: Option<Arc<HostLink>>,
    /// The image this document was restored from, and its stored bytes' SHA-256 once asked
    /// (`residency::slabs::admit_docs`, decision 0032): what the device reads in place.
    pub image: Arc<TapeImage>,
    digest: Arc<std::sync::OnceLock<[u8; 32]>>,
    /// The document's path, as a run's matchers read it (`with_path`: what a `filePath` test inside a matcher tests).
    path: Option<Arc<str>>,
    /// The image's tape through the one API, in each format a run reads (`tape()`, decision 0293): the same image.
    held: Arc<formats::DocTape>,
}

#[derive(Clone)]
enum Guests {
    Owned(Arc<Vec<Guest>>),
    Linked(Weak<Vec<Guest>>),
}

/// A program a file embeds, as a document of its own (`embed`).
#[derive(Clone)]
pub struct Guest {
    pub via: Via,
    pub doc: TapeDoc,
}

/// What a guest document reads of the file it is embedded in: the embedding, the segments that
/// map its text to the file's bytes, the file's bytes it cannot be read through, and the host
/// document (which links back to its guests weakly).
pub struct HostLink {
    pub(in crate::code_rule_dsl) via: Via,
    pub segments: Arc<SegmentMap>,
    pub(in crate::code_rule_dsl) host_errors: Arc<Vec<(u32, u32)>>,
    pub host: TapeDoc,
    /// Each node's extent in the program (`node_extents`), by preorder index.
    pub extents: Vec<(u32, u32)>,
}

/// The program bytes each node of a guest's records stands for: from the first of its leaves that
/// stands for file bytes to the last (`SegmentMap::stands_for`); a node none of whose leaves does,
/// from its first leaf to its last. A node's span may run past those leaves: a root's holds the
/// white space after its last statement, and a statement ending in a `;` its embedder wrote over
/// no bytes holds the white space before it, which stands for nothing of the node's.
///
/// The tape stores no implied token (RER-131, `tokens`): `echo` and `;` a Blade echo is written as,
/// the `;` marker Vue's script and template programs start with. Their text is in the gaps a node's
/// stored children leave in its span, the source `structural_before` and `structural_after` hold
/// (core.rerail): comments are nodes, so a gap is implied tokens and white space only, and each run of
/// it that is not white space (`implied`) is read as a leaf. Which tokens a run holds does not
/// matter here (`TokenGrammar::gaps` would say): only whether its bytes stand for the file's.
fn node_extents(doc: &TapeDoc, segments: &SegmentMap) -> Vec<(u32, u32)> {
    rerail_tape_format::with_tape!(doc.tape(), |t| extents_of(&t, doc.source.as_bytes(), segments))
}

/// `node_extents` over the tape, read in place: one pass in preorder, each node's extent joined into its parent's
/// as its subtree closes (the subtrees open around a node are its ancestors).
fn extents_of<T: rerail_tape_format::tape::Tape>(t: &T, text: &[u8], segments: &SegmentMap) -> Vec<(u32, u32)> {
    const NONE_YET: (u32, u32) = (u32::MAX, 0);
    let join = |into: &mut (u32, u32), (start, end): (u32, u32)| {
        *into = (into.0.min(start), into.1.max(end));
    };
    // The implied tokens of `start..end`, a gap between a node's stored children (a
    // `structural_before`/`structural_after` source): its runs of text that are not white space.
    let implied = |start: u32, end: u32, leaf: &mut dyn FnMut(u32, u32)| {
        let gap = text.get(start as usize..end as usize).unwrap_or_default();
        let mut at = 0;
        while at < gap.len() {
            let Some(from) = gap[at..].iter().position(|b| !b.is_ascii_whitespace()) else { break };
            let from = at + from;
            let to = gap[from..].iter().position(|b| b.is_ascii_whitespace()).map_or(gap.len(), |n| from + n);
            leaf(start + from as u32, start + to as u32);
            at = to;
        }
    };
    let n = t.len();
    // Per node: the extent of its leaves standing for bytes, and of all its leaves.
    let mut standing = vec![NONE_YET; n];
    let mut all = vec![NONE_YET; n];
    // A closed subtree's extents, joined into its parent's (the subtree open around it).
    let close = |node: usize, open: &[(usize, usize)], standing: &mut [(u32, u32)], all: &mut [(u32, u32)]| {
        if let Some(&(parent, _)) = open.last() {
            let (leaves, standing_leaves) = (all[node], standing[node]);
            join(&mut all[parent], leaves);
            if standing_leaves != NONE_YET {
                join(&mut standing[parent], standing_leaves);
            }
        }
    };
    let mut open: Vec<(usize, usize)> = Vec::new();
    for index in 0..n {
        while let Some(&(node, end)) = open.last() {
            if end > index {
                break;
            }
            open.pop();
            close(node, &open, &mut standing, &mut all);
        }
        let (start, end) = (t.start(index) as u32, t.end(index) as u32);
        let subtree_end = t.subtree_end(index);
        if subtree_end == index + 1 {
            all[index] = (start, end);
            if segments.stands_for(start as usize..end as usize) {
                standing[index] = (start, end);
            }
        } else {
            // The gaps its stored children leave in its span.
            let mut at = start;
            let mut gaps = Vec::new();
            for child in rerail_tape_format::tape::children(t, index) {
                let (c_start, c_end) = (t.start(child) as u32, t.end(child) as u32);
                if c_start > at {
                    gaps.push((at, c_start));
                }
                at = at.max(c_end);
            }
            if end > at {
                gaps.push((at, end));
            }
            for (from, to) in gaps {
                implied(from, to, &mut |from, to| {
                    join(&mut all[index], (from, to));
                    if segments.stands_for(from as usize..to as usize) {
                        join(&mut standing[index], (from, to));
                    }
                });
            }
        }
        open.push((index, subtree_end));
    }
    while let Some((node, _)) = open.pop() {
        close(node, &open, &mut standing, &mut all);
    }
    standing
        .into_iter()
        .zip(all)
        .map(|(standing, all)| if standing == NONE_YET { all } else { standing })
        .collect()
}

pub type KindPostings = rustc_hash::FxHashMap<u16, Vec<u32>>;

/// A node's row and column, as tree-sitter's `TSPoint` counts them: only `\n` ends a row. The
/// evaluator's line table follows the linter a port mirrors (a lone `\r` ends a Ruff line), so
/// positions keep their own table, built on first use: violations place spans with the line table,
/// and nothing but parity with the native tree reads a tape node's position.
struct Points {
    source: Arc<String>,
    rows: std::sync::OnceLock<LineTable>,
}

impl Points {
    fn point(&self, byte: u32) -> Position {
        let byte = byte as usize;
        let (row, column) = self
            .rows
            .get_or_init(|| LineTable::tree_sitter(&self.source))
            .point(byte);
        Position::new(row, column, byte)
    }
}

/// Immutable source-bound tape input. Expanded navigation exists only while
/// this file is packed or its result page is normalized.
#[derive(Clone)]
pub struct TapeSource {
    pub source: Arc<String>,
    pub grammar: Arc<Grammar>,
    pub image: Arc<FileImage>,
    pub lines: Arc<LineTable>,
}

impl TapeSource {
    pub fn restore(&self, budget: &impl Fn() -> bool) -> Result<TapeDoc, String> {
        TapeDoc::restore_with_lines(
            Arc::clone(&self.source),
            Arc::clone(&self.grammar),
            &self.image,
            Arc::clone(&self.lines),
            budget,
        )
    }
}

impl TapeDoc {
    /// P's: the digest of the source the tape was built from (P's SQL anchor engine,
    /// test-only). W's tape keeps no digest, so it is computed.
    /// G's: the shared source (G's literal anchors, test-only).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn transducer_source_arc(&self) -> Arc<String> {
        Arc::clone(&self.source)
    }

    /// The documents `image` encodes over `source`: the file's own, with the guests derived from
    /// it.
    pub fn restore_with_lines(
        source: Arc<String>,
        grammar: Arc<Grammar>,
        image: &FileImage,
        lines: Arc<LineTable>,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let mut host = Self::restore_tree(source, grammar, &image.primary, lines, budget)?;
        if image.derived.is_empty() {
            return Ok(host);
        }
        // A guest reads the file's own document as its host; a document derived from a guest
        // has no runtime home, so its file is parsed again rather than read in part.
        if image.derived.iter().any(|document| document.parent != 0) {
            return Err("a document derived from a derived document is not restored".into());
        }
        let mut failure = None;
        let guests = Arc::new_cyclic(|guests: &Weak<Vec<Guest>>| {
            let linked = Self {
                guests: Guests::Linked(guests.clone()),
                ..host.clone()
            };
            let mut restored = Vec::with_capacity(image.derived.len());
            for guest in &image.derived {
                match Self::restore_guest(&linked, guest, budget) {
                    Ok(doc) => restored.push(Guest {
                        via: guest.via,
                        doc,
                    }),
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            restored
        });
        if let Some(error) = failure {
            return Err(error);
        }
        host.guests = Guests::Owned(guests);
        Ok(host)
    }

    /// The guest document `guest` encodes, derived from `host`.
    fn restore_guest(
        host: &Self,
        guest: &DerivedImage,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let grammar = Grammar::embedded(guest.tape.language)?;
        // Positions read through the segments slice both texts: a map that does not fit them is
        // refused here, so the tape is encoded again, never read wrong.
        guest.source_map.check(&guest.text, &host.source)?;
        let lines = Arc::new(LineTable::new(&guest.text, Terminators::of(grammar.lang)));
        let mut doc =
            Self::restore_tree(Arc::clone(&guest.text), grammar, &guest.tape, lines, budget)?;
        checked(budget)?;
        doc.host = Some(Arc::new(HostLink {
            via: guest.via,
            segments: Arc::clone(&guest.source_map),
            host_errors: Arc::clone(&guest.host_errors),
            host: host.clone(),
            extents: node_extents(&doc, &guest.source_map),
        }));
        Ok(doc)
    }

    /// The document of `image`'s own tree over `source`.
    fn restore_tree(
        source: Arc<String>,
        grammar: Arc<Grammar>,
        image: &TapeImage,
        lines: Arc<LineTable>,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        read!("docs restored");
        read!("nodes restored", image.node_count());
        // The image is checked node by node and held as it is: the document reads its tape in place (decision
        // 0293), so nothing is copied out of it.
        let (digest, _) = validated::visit(&source, &grammar, image, budget, |_, _| {})?;
        checked(budget)?;
        let image = Arc::new(image.clone());
        let held = Arc::new(formats::DocTape::new(Arc::clone(&image), Arc::clone(&source)));
        Ok(Self {
            held,
            source_digest: digest,
            points: Arc::new(Points {
                source: Arc::clone(&source),
                rows: std::sync::OnceLock::new(),
            }),
            source,
            grammar,
            lines,
            labels: Arc::new(Vec::new()),
            texts: Arc::default(),
            guests: Guests::Owned(Arc::new(Vec::new())),
            host: None,

            image,
            digest: Arc::default(),
            path: None,
        })
    }

    /// Ranges of ERROR nodes and MISSING tokens that no ERROR node contains, as
    /// `SyntaxErrors::collect` finds them on the native tree.
    pub fn syntax_error_ranges(&self) -> Vec<Range<usize>> {
        self.syntax_error_nodes().map(|(_, range)| range).collect()
    }

    /// The nodes `syntax_error_ranges` reads, each with its preorder index.
    pub fn syntax_error_nodes(&self) -> impl Iterator<Item = (usize, Range<usize>)> + '_ {
        use rerail_tape_format::tape::Tape;
        let mut found = Vec::new();
        rerail_tape_format::with_tape!(self.tape(), |t| {
            let mut i = 0;
            while i < t.len() {
                if t.table().entry(t.code(i)).flags & (CODE_ERROR | CODE_MISSING) != 0 {
                    found.push((i, t.start(i) as usize..t.end(i) as usize));
                    i = t.subtree_end(i);
                } else {
                    i += 1;
                }
            }
        });
        found.into_iter()
    }

    /// A host's guests (`embed`), in registry order; for the host a guest reads, the same ones.
    /// `None` once the host that owns them is gone.
    pub fn guests(&self) -> Option<Arc<Vec<Guest>>> {
        match &self.guests {
            Guests::Owned(guests) => Some(Arc::clone(guests)),
            Guests::Linked(guests) => guests.upgrade(),
        }
    }

    /// Whether this is a guest document: a program a file embeds.
    pub fn is_guest(&self) -> bool {
        self.host.is_some()
    }

    /// Whether `other` is this document restored once (a clone, a host a guest reads as this
    /// host): the same tape.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.held, &other.held)
    }

    /// The embedding that built this guest; `None` for a host.
    pub fn via(&self) -> Option<Via> {
        self.host.as_ref().map(|link| link.via)
    }

    /// The host document this guest is embedded in.
    pub fn host(&self) -> Option<&Self> {
        self.host.as_ref().map(|link| &link.host)
    }

    /// Its dialect's grammar.
    pub fn grammar(&self) -> &Arc<Grammar> {
        &self.grammar
    }

    /// This document's own text, which its spans index.
    pub fn document_source(&self) -> &Arc<String> {
        &self.source
    }

    /// The file's source: a guest's host's, else this document's own.
    pub fn file_source(&self) -> &Arc<String> {
        self.host
            .as_ref()
            .map_or(&self.source, |link| &link.host.source)
    }

    /// The file's line table: a guest's host's, else this document's own.
    pub fn file_lines(&self) -> &Arc<LineTable> {
        self.host
            .as_ref()
            .map_or(&self.lines, |link| &link.host.lines)
    }

    /// The file bytes a guest cannot be read through (`embed::Program::host_errors`); none for a
    /// host.
    pub fn host_errors(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.host
            .iter()
            .flat_map(|link| link.host_errors.iter())
            .map(|&(start, end)| start as usize..end as usize)
    }

    /// The dialect of the file this document is of: a guest's host's, else its own.
    pub fn file_dialect(&self) -> GrammarDialect {
        self.host
            .as_ref()
            .map_or(self.grammar.lang, |link| link.host.grammar.lang)
    }

    /// The documents `image` encodes over `source`, with a fresh line table: a file's documents,
    /// or a lone tape (a file that derives none).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn restore(
        source: Arc<String>,
        grammar: Arc<Grammar>,
        image: &impl Documents,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let lines = Arc::new(LineTable::new(&source, Terminators::of(grammar.lang)));
        Self::restore_with_lines(source, grammar, &image.file_image(), lines, budget)
    }

    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub fn index_source_digest(&self) -> &[u8; 32] {
        &self.source_digest
    }

    /// The source's line table, shared with the evaluator that restored this document.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn line_table(&self) -> &LineTable {
        &self.lines
    }

    pub fn transducer_len(&self) -> usize {
        self.t().len()
    }

    /// The tape's reader, built once (`DocTape::reader`): a read at a time, dispatched on the format.
    #[inline]
    fn t(&self) -> &rerail_tape_format::tape::AnyTape<'_> {
        self.held.reader()
    }

    /// Node `node`'s code.
    #[inline]
    fn entry(&self, node: u32) -> &Entry {
        let t = self.t();
        t.table().entry(t.code(node as usize))
    }

    /// This document's tape identity, the store's key for it (`HashKey::tape_identity`: its image
    /// as stored and its source's hash), computed once.
    pub fn tape_digest(&self) -> Result<[u8; 32], String> {
        if let Some(digest) = self.digest.get() {
            return Ok(*digest);
        }
        let digest = tape_identity(&self.source_digest, &self.image.to_bytes()?);
        Ok(*self.digest.get_or_init(|| digest))
    }

    /// `tape_digest`, and the image as stored when computing it serialized the image (the digest
    /// not known yet): admission copies those bytes, not a second serialization.
    pub fn tape_digest_and_image(&self) -> Result<([u8; 32], Option<Vec<u8>>), String> {
        if let Some(digest) = self.digest.get() {
            return Ok((*digest, None));
        }
        let image = self.image.to_bytes()?;
        let digest = tape_identity(&self.source_digest, &image);
        Ok((*self.digest.get_or_init(|| digest), Some(image)))
    }

    /// The SHA-256 of this document's source.
    pub fn source_digest(&self) -> &[u8; 32] {
        &self.source_digest
    }

    /// This document's tape identity, known where it was restored from stored bytes
    /// (`tape_identity`, the residency's key): admission then reads it from the store.
    pub fn stored_as(self, digest: [u8; 32]) -> Self {
        let _ = self.digest.set(digest);
        self
    }

    /// The SHA-256 of this document's stored bytes, the residency's key (decision 0032).
    pub fn resident_key(&self) -> Result<[u8; 32], String> {
        self.tape_digest()
    }

    /// This document's image as stored (`image`, when the caller has it already), its source and
    /// its grammar: what residency admits.
    pub fn resident_parts(&self, image: Option<Vec<u8>>) -> Result<(Vec<u8>, &str, &Arc<Grammar>), String> {
        let image = match image {
            Some(image) => image,
            None => self.image.to_bytes()?,
        };
        Ok((image, self.source.as_str(), &self.grammar))
    }

    pub fn transducer_kind(&self, node: u32) -> u16 {
        read!("kind");
        kind_of(self.entry(node))
    }

    fn node_is_named(&self, node: u32) -> bool {
        read!("named");
        named_of(&self.grammar, self.entry(node))
    }

    pub fn transducer_field(&self, node: u32) -> u16 {
        read!("field");
        self.entry(node).field
    }

    pub fn transducer_flags(&self, node: u32) -> u8 {
        read!("flags");
        let e = self.entry(node);
        u8::from(named_of(&self.grammar, e))
            | (u8::from(e.flags & CODE_EXTRA != 0) << 1)
            | (u8::from(e.flags & CODE_MISSING != 0) << 2)
            | (u8::from(e.flags & CODE_ERROR != 0) << 3)
    }

    pub fn transducer_span(&self, node: u32) -> Range<usize> {
        read!("span");
        read_at!("tape span", node);
        let t = self.t();
        t.start(node as usize) as usize..t.end(node as usize) as usize
    }

    /// The grammar's named kinds, ERROR included, ascending: what a rule ranges over.
    pub(in crate::code_rule_dsl) fn named_kinds(&self) -> Vec<u16> {
        let mut kinds: Vec<u16> = (0..self.grammar.named.len() as u16).filter(|&kind| self.grammar.kind_is_named(kind)).collect();
        kinds.push(ERROR_SYMBOL);
        kinds
    }

    /// Whether the node is named (its alias's, else its kind's).
    pub fn transducer_is_named(&self, node: u32) -> bool {
        self.node_is_named(node)
    }

    pub fn transducer_parent(&self, node: u32) -> Option<u32> {
        read!("parent");
        read_at!("tape parent", node);
        self.t().parent(node as usize).map(|p| p as u32)
    }

    /// Each node's parent, in preorder, from the subtrees open around it: a pass that reads no parent, where
    /// `transducer_parent` reads one (format 15 searches for it, decision 0293).
    pub fn transducer_parents(&self) -> impl Iterator<Item = Option<u32>> + '_ {
        let t = self.t();
        let mut open: Vec<(u32, usize)> = Vec::new();
        (0..t.len()).map(move |i| {
            while open.last().is_some_and(|&(_, end)| end <= i) {
                open.pop();
            }
            let parent = open.last().map(|&(p, _)| p);
            open.push((i as u32, t.subtree_end(i)));
            parent
        })
    }

    /// The node's children, in order.
    pub fn transducer_children(&self, node: u32) -> impl Iterator<Item = u32> + '_ {
        rerail_tape_format::tape::children(self.t(), node as usize).map(|c| {
            read!("children");
            c as u32
        })
    }

    /// Whether the tokens the tape leaves out right after `parent`'s child `child` (its index among
    /// the children) begin with `word` (`Doc::implied_after`, RER-131).
    pub fn transducer_implied_after(&self, parent: u32, child: usize, word: &[&str]) -> bool {
        self.implied_after(&self.root_node().at(parent, UNKNOWN), child, word) == Some(true)
    }

    pub fn transducer_descendants(&self, node: u32) -> Range<u32> {
        read!("size");
        read_at!("tape size", node);
        node + 1..self.t().subtree_end(node as usize) as u32
    }

    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub fn transducer_next_sibling(&self, node: u32, named: bool) -> Option<u32> {
        let t = self.t();
        let parent_end = t.subtree_end(t.parent(node as usize)?);
        let mut next = t.next_sibling(node as usize, parent_end);
        while let Some(n) = next {
            read!("children");
            if !named || self.node_is_named(n as u32) {
                return Some(n as u32);
            }
            next = t.next_sibling(n, parent_end);
        }
        None
    }

    /// The whole source, for the Metal backend's concatenated buffer.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub fn transducer_source(&self) -> &[u8] {
        self.source.as_bytes()
    }

    /// A node's source slice. Restoring the tape validates every span as a UTF-8 boundary.
    pub fn transducer_text_str(&self, node: u32) -> &str {
        &self.source[self.transducer_span(node)]
    }

    /// A node's source bytes, for the packed CPU and Metal backends.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub fn transducer_text(&self, node: u32) -> &[u8] {
        &self.source.as_bytes()[self.transducer_span(node)]
    }

    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub fn transducer_is_named_leaf(&self, node: u32) -> bool {
        self.node_is_named(node) && self.t().first_child(node as usize).is_none()
    }

    /// A collision-free encoding of the recursive kind and child structure used by ast-grep's
    /// repeated-capture equality when neither side is a named leaf.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub fn transducer_structural_key(&self, node: u32) -> String {
        fn encode(tape: &TapeDoc, node: u32, bytes: &mut Vec<u8>) {
            bytes.extend_from_slice(&tape.transducer_kind(node).to_le_bytes());
            let children: Vec<u32> = tape.transducer_children(node).collect();
            bytes.extend_from_slice(&(children.len() as u32).to_le_bytes());
            for child in children {
                encode(tape, child, bytes);
            }
        }
        let mut bytes = Vec::new();
        encode(self, node, &mut bytes);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut key = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            key.push(char::from(HEX[usize::from(byte >> 4)]));
            key.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        key
    }

    /// The abstract grammar classes (supertypes) each node belongs to, as `(node, kind)` sorted.
    /// The nodes whose code passes `admits` (its kind, as `transducer_kind`, and its classes), in order, onto `out`:
    /// the codes tested once each, the nodes found from the tape's code bytes 64 at a time (`Tape::match_groups`).
    pub fn transducer_nodes_where(&self, admits: impl Fn(u16, &[u16]) -> bool, out: &mut Vec<u32>) {
        let t = self.t();
        let set = t.table().set(|e| admits(kind_of(e), &e.classes));
        t.match_groups(&set, |base, mut m| {
            while m != 0 {
                out.push((base + m.trailing_zeros() as usize) as u32);
                m &= m - 1;
            }
        });
    }

    pub fn transducer_supertypes(&self) -> impl Iterator<Item = (u32, u16)> + '_ {
        (0..self.transducer_len() as u32).flat_map(move |node| {
            read!("classes", self.entry(node).classes.len());
            self.entry(node).classes.iter().map(move |&class| (node, class))
        })
    }

    /// The derived classes (`with_labels`): per class from `DERIVED_CLASS_BASE`, a bitset over the
    /// nodes. The packed batch carries them as bitsets over its nodes, which every backend tests
    /// directly (`PackedBatch::derived`).
    pub fn transducer_labels(&self) -> &[Vec<u64>] {
        &self.labels
    }

    /// The labels, shared (`transducer_labels`).
    pub fn transducer_labels_shared(&self) -> Arc<Vec<Vec<u64>>> {
        Arc::clone(&self.labels)
    }

    // The packed runtime's class test (Phase 4).
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub fn transducer_has_supertype(&self, node: u32, kind: u16) -> bool {
        read!("classes");
        if kind >= DERIVED_CLASS_BASE {
            return self
                .labels
                .get(usize::from(kind - DERIVED_CLASS_BASE))
                .and_then(|bits| bits.get(node as usize / 64))
                .is_some_and(|word| word >> (node % 64) & 1 != 0);
        }
        self.entry(node).classes.binary_search(&kind).is_ok()
    }

    /// This tape with derived node classes: a lowered matcher's label passes (06-first-slice S1),
    /// one bitset over the nodes per class from `DERIVED_CLASS_BASE`, which `SUPERTYPE` tests as it
    /// tests a grammar class.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub fn with_labels(&self, labels: Arc<Vec<Vec<u64>>>) -> Self {
        Self {
            labels,
            ..self.clone()
        }
    }

    /// This tape read at `path`: what a `filePath` test inside a matcher tests (`matcher::Fact::nth_bits`).
    pub fn with_path(&self, path: &str) -> Self {
        Self { path: Some(Arc::from(path)), ..self.clone() }
    }

    /// The path the run reads this tape at (`with_path`), if one was given.
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// This tape with its text tests' rows from the term index (`prefilter::terms`).
    pub fn with_texts(&self, texts: std::collections::BTreeMap<String, Arc<Vec<u64>>>) -> Self {
        Self {
            texts: Arc::new(texts),
            ..self.clone()
        }
    }

    /// A text test's row from the term index, by its slot key (`with_texts`).
    pub fn transducer_text_row(&self, key: &str) -> Option<&Arc<Vec<u64>>> {
        self.texts.get(key)
    }

    /// The node at a preorder ordinal: `root.adopt(doc.node(ordinal))` is the node ast-grep
    /// matches on.
    pub fn node(&self, ordinal: u32) -> TapeNode<'_> {
        self.root_node().at(ordinal, if ordinal == 0 { NONE } else { UNKNOWN })
    }

    #[cfg(feature = "code-rule-profile")]
    pub fn navigation_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.held.bytes()
    }
}

#[derive(Clone, Copy)]
pub struct TapeNode<'a> {
    tape: &'a rerail_tape_format::tape::AnyTape<'a>,
    grammar: &'a Grammar,
    points: &'a Points,
    index: u32,
    /// The node's parent, where the step that reached it knew it (a child's, a walk's, a climb's): `NONE` at the
    /// root, `UNKNOWN` where it was not known, and the tape is asked.
    up: u32,
}

impl<'a> TapeNode<'a> {
    /// P's: the field this node fills in its parent (P's SQL anchor engine, test-only).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn incoming_field(&self) -> u16 {
        self.entry().field
    }

    /// Whether this is the first direct child in `field`, as `child_by_field_id` selects it.
    pub fn is_first_field(&self, field: u16) -> bool {
        if field == 0 || self.entry().field != field {
            return false;
        }
        let parent = self.up();
        parent == NONE || self.siblings_before(parent).all(|c| self.tape.table().entry(self.tape.code(c)).field != field)
    }

    #[inline]
    fn entry(&self) -> &'a Entry {
        self.tape.table().entry(self.tape.code(self.index as usize))
    }
    #[inline]
    fn start(&self) -> u32 {
        self.tape.start(self.index as usize) as u32
    }
    #[inline]
    fn end(&self) -> u32 {
        self.tape.end(self.index as usize) as u32
    }
    /// The node at `index`, its parent `up` (`UNKNOWN` where not known).
    #[inline]
    fn at(&self, index: u32, up: u32) -> Self {
        Self { index, up, ..*self }
    }
    /// The parent's id (`NONE` at the root): as the step that reached this node knew it, else the tape's.
    fn up(&self) -> u32 {
        if self.up != UNKNOWN {
            return self.up;
        }
        read!("parent");
        read_at!("tape parent", self.index);
        self.tape.parent(self.index as usize).map_or(NONE, |p| p as u32)
    }
    /// The children of `parent` before this node, in order.
    fn siblings_before(&self, parent: u32) -> impl Iterator<Item = usize> + 'a {
        let (tape, index) = (self.tape, self.index as usize);
        rerail_tape_format::tape::children(tape, parent as usize).take_while(move |&c| c < index)
    }
    /// The structurally adjacent sibling, for traversal; `SgNode::next` follows Tree-sitter.
    pub fn next_sibling(&self) -> Option<Self> {
        read!("children");
        let parent = self.up();
        if parent == NONE {
            return None;
        }
        let end = self.tape.subtree_end(parent as usize);
        self.tape.next_sibling(self.index as usize, end).map(|n| self.at(n as u32, parent))
    }
    fn prev_sibling(&self) -> Option<Self> {
        read!("prev");
        let parent = self.up();
        if parent == NONE {
            return None;
        }
        self.siblings_before(parent).last().map(|p| self.at(p as u32, parent))
    }
    /// Where ast-grep's `next_all`/`prev_all` start: a cursor on the parent (the root for the
    /// root) moved to the first child that ends after this node's start byte.
    ///
    /// Sibling ends never decrease (restore checks it), so the children that end after the start
    /// are a suffix of the parent's. The walk steps from this node (the root's first child for
    /// the root) to that suffix's first child: back over earlier siblings that also end after the
    /// start, or forward past zero-width ones that end at it. For a node with width, whose earlier
    /// siblings end at or before its start, that is one or two steps, not a scan from the
    /// parent's first child.
    fn walk_anchor(&self) -> Option<Self> {
        let start = self.start();
        let ends_after = |node: &Self| node.end() > start;
        let mut anchor = match self.parent() {
            Some(_) => *self,
            None => self.children().next()?,
        };
        if ends_after(&anchor) {
            while let Some(prev) = anchor.prev_sibling().filter(ends_after) {
                anchor = prev;
            }
            return Some(anchor);
        }
        loop {
            anchor = anchor.next_sibling()?;
            if ends_after(&anchor) {
                return Some(anchor);
            }
        }
    }
}

/// A node's children, read off the tape as they are asked for. How many are left is counted only when asked
/// (`size_hint`): a search among them (`child_by_field_id`) counted every child first, then walked them again.
struct Children<'a> {
    node: TapeNode<'a>,
    next: Option<usize>,
    end: usize,
}
impl<'a> Iterator for Children<'a> {
    type Item = TapeNode<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        let index = self.next?;
        read!("children");
        self.next = self.node.tape.next_sibling(index, self.end);
        Some(self.node.at(index as u32, self.node.index))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let tape = self.node.tape;
        let remaining = std::iter::successors(self.next, |&at| tape.next_sibling(at, self.end)).count();
        (remaining, Some(remaining))
    }
}
impl ExactSizeIterator for Children<'_> {}

impl<'a> SgNode<'a> for TapeNode<'a> {
    fn parent(&self) -> Option<Self> {
        let parent = self.up();
        (parent != NONE).then(|| self.at(parent, UNKNOWN))
    }
    /// The parent, then (where asked further) the rest of the path, from one descent from the root
    /// (`Tape::path`): each ancestor with its own parent known, so a climb reads no parent.
    fn ancestors(&self, _root: Self) -> impl Iterator<Item = Self> {
        let first = self.parent();
        let me = *self;
        let mut path: Option<Vec<usize>> = None;
        let mut given = 0usize;
        std::iter::from_fn(move || {
            if given == 0 {
                given = 1;
                return first;
            }
            let path = path.get_or_insert_with(|| {
                read!("parent");
                let mut path = Vec::new();
                me.tape.path(me.index as usize, &mut path);
                path
            });
            let at = path.len().checked_sub(1 + given)?;
            given += 1;
            Some(me.at(path[at] as u32, if at == 0 { NONE } else { path[at - 1] as u32 }))
        })
    }
    fn children(&self) -> impl ExactSizeIterator<Item = Self> {
        let (tape, index) = (self.tape, self.index as usize);
        let end = tape.subtree_end(index);
        Children { node: *self, next: tape.first_child(index), end }
    }
    /// The subtree in preorder, each node with its parent: the subtrees open around it.
    fn dfs(&self) -> impl Iterator<Item = Self> {
        let start = self.index as usize;
        read!("size");
        read_at!("tape size", self.index);
        let me = *self;
        let end = self.tape.subtree_end(start);
        let mut open: Vec<(u32, usize)> = Vec::new();
        (start..end).map(move |index| {
            if index == start {
                open.push((me.index, end));
                return me;
            }
            while open.last().is_some_and(|&(_, until)| until <= index) {
                open.pop();
            }
            let parent = open.last().expect("the subtree's root is open").0;
            open.push((index as u32, me.tape.subtree_end(index)));
            me.at(index as u32, parent)
        })
    }
    fn child(&self, nth: usize) -> Option<Self> {
        self.children().nth(nth)
    }
    // Tree-sitter's next sibling is byte-positioned: it skips later siblings that end at or
    // before this node's end, i.e. zero-width siblings sitting at that end.
    fn next(&self) -> Option<Self> {
        let end = self.end();
        let mut next = self.next_sibling();
        while let Some(node) = next {
            if node.end() > end {
                return Some(node);
            }
            next = node.next_sibling();
        }
        None
    }
    fn prev(&self) -> Option<Self> {
        self.prev_sibling()
    }
    fn next_all(&self) -> impl Iterator<Item = Self> {
        let mut next = self.walk_anchor().and_then(|anchor| anchor.next_sibling());
        std::iter::from_fn(move || {
            let node = next?;
            next = node.next_sibling();
            Some(node)
        })
    }
    /// The siblings before the anchor, nearest first: read forward from the parent's first child once.
    fn prev_all(&self) -> impl Iterator<Item = Self> {
        let before: Vec<Self> = self
            .walk_anchor()
            .map(|anchor| {
                let parent = anchor.up();
                if parent == NONE { Vec::new() } else { anchor.siblings_before(parent).map(|c| anchor.at(c as u32, parent)).collect() }
            })
            .unwrap_or_default();
        before.into_iter().rev()
    }
    fn kind(&self) -> Cow<'_, str> {
        read!("kind");
        let e = self.entry();
        Cow::Borrowed(e.alias.as_ref().map_or_else(|| self.grammar.kind_name(kind_of(e)).expect("validated grammar symbol"), |alias| alias.0.as_str()))
    }
    fn kind_id(&self) -> u16 {
        read!("kind");
        kind_of(self.entry())
    }
    fn node_id(&self) -> usize {
        self.index as usize
    }
    fn range(&self) -> Range<usize> {
        read!("span");
        read_at!("tape span", self.index);
        self.start() as usize..self.end() as usize
    }
    fn start_pos(&self) -> Position {
        read!("points");
        self.points.point(self.start())
    }
    fn end_pos(&self) -> Position {
        read!("points");
        self.points.point(self.end())
    }
    fn is_named(&self) -> bool {
        read!("named");
        named_of(self.grammar, self.entry())
    }
    fn is_named_leaf(&self) -> bool {
        self.children().all(|child| !child.is_named())
    }
    fn is_extra(&self) -> bool {
        read!("flags");
        self.entry().flags & CODE_EXTRA != 0
    }
    fn is_error(&self) -> bool {
        read!("flags");
        self.entry().flags & CODE_ERROR != 0
    }
    fn is_missing(&self) -> bool {
        read!("flags");
        self.entry().flags & CODE_MISSING != 0
    }
    fn field(&self, name: &str) -> Option<Self> {
        let field = self.grammar.field_code(name)?;
        self.child_by_field_id(field)
    }
    fn field_children(&self, field_id: Option<u16>) -> impl Iterator<Item = Self> {
        self.children().filter(move |child| {
            read!("field");
            field_id.is_some_and(|field| field != 0 && child.entry().field == field)
        })
    }
    fn child_by_field_id(&self, field_id: u16) -> Option<Self> {
        if field_id == 0 {
            None
        } else {
            #[cfg(feature = "code-rule-profile")]
            let mut examined = 0;
            let found = self.children().find(|child| {
                #[cfg(feature = "code-rule-profile")]
                {
                    examined += 1;
                }
                read!("field");
                child.entry().field == field_id
            });
            #[cfg(feature = "code-rule-profile")]
            navigation::record(examined, found.is_some());
            found
        }
    }
}

impl Doc for TapeDoc {
    type Source = String;
    type Lang = GrammarDialect;
    type Node<'a> = TapeNode<'a>;
    fn get_lang(&self) -> &GrammarDialect {
        &self.grammar.lang
    }
    fn get_source(&self) -> &String {
        &self.source
    }
    fn do_edit(&mut self, _edit: &Edit<String>) -> Result<(), String> {
        Err("replace the source-bound tape after editing".into())
    }
    fn root_node(&self) -> Self::Node<'_> {
        TapeNode { tape: self.held.reader(), grammar: &self.grammar, points: &self.points, index: 0, up: NONE }
    }
    fn get_node_text<'a>(&'a self, node: &Self::Node<'a>) -> Cow<'a, str> {
        Cow::Borrowed(&self.source[node.range()])
    }
    /// The implied word the tape leaves out in the gap after `parent`'s child `child`
    /// (`tokens::stored_gaps`: the word before the next named child, or after the last), RER-131.
    fn implied_after<'a>(&'a self, parent: &TapeNode<'a>, child: usize, word: &[&str]) -> Option<bool> {
        let grammar = tokens::TokenGrammar::of_dialect(self.grammar.lang)?;
        // `stored_gaps` reads an ast-grep node: the parent's, rebuilt over a root of this document.
        let root = ast_grep_core::AstGrep::doc(self.clone());
        let parent = root.adopt(root.root().get_inner_node().at(parent.index, parent.up));
        let (children, gaps) = tokens::stored_gaps(&grammar, &parent)?;
        let next = (child + 1..children.len()).find(|&at| children[at].is_named() && !children[at].is_extra());
        let gap = next.map_or(&gaps.end, |at| &gaps.before[at]);
        Some(gap.len() >= word.len() && gap.iter().zip(word).all(|((text, _), token)| text == token))
    }
}


/// The children of `node` a tape of `lang` stores, in order: tree-sitter's, the tokens the grammar
/// implies left out (`tokens::TokenGrammar::stored_children`). For tests that compare a tape with
/// the tree it was written from.
#[cfg(any(test, feature = "test-hooks"))]
pub fn stored_children_for_test<'t>(lang: GrammarDialect, node: TsNode<'t>) -> Vec<(TsNode<'t>, u16)> {
    let grammar = Grammar::new(lang).unwrap();
    let stored = grammar.kinds.grammar().and_then(|language| tokens::TokenGrammar::of(lang, language)).and_then(|filter| filter.stored_children(node));
    let mut cursor = node.walk();
    let mut children = Vec::new();
    if cursor.goto_first_child() {
        loop {
            children.push((cursor.node(), cursor.field_id().map_or(0, |id| id.get())));
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    match stored {
        None => children,
        Some(stored) => children.into_iter().zip(stored).filter_map(|(child, kept)| kept.then_some(child)).collect(),
    }
}

/// A pattern's tree as a tape: tree-sitter's parse of `source`, written as a document is
/// (`TapeImage::encode_tree_image`), so the tokens the grammar implies are left out of patterns as
/// they are of documents (RER-131). With it, per record, what tree-sitter's node says that the tape
/// does not ([`PatternShape`]).
pub fn pattern_doc(lang: GrammarDialect, source: &str) -> Result<(TapeDoc, PatternShape), String> {
    use ast_grep_core::language::Language as _;
    let grammar = Grammar::new(lang)?;
    let tree = ast_grep_core::tree_sitter::StrDoc::try_new(source, lang)?;
    let root = tree.tree.root_node();
    let image = TapeImage::encode_tree_image(root, &grammar, source, None, &|| false)?;
    let filter = grammar.kinds.grammar().and_then(|language| tokens::TokenGrammar::of(lang, language));
    // In the encoder's preorder: a record per stored node.
    let mut ends: Vec<Option<Vec<String>>> = Vec::new();
    let mut single = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let at = ends.len();
        ends.push(None);
        let stored = filter.as_deref().and_then(|filter| filter.stored_children(node));
        // `is_single_node` (ast-grep's `pattern.rs`) over tree-sitter's node, whose first child
        // the tape holds (the walk descends to it).
        single.push(
            match node.child_count() {
                1 => true,
                2 => node.child(1).is_some_and(|second| second.is_missing() || second.kind().is_empty()),
                _ => false,
            } && stored.as_ref().is_none_or(|stored| stored[0]),
        );
        let mut cursor = node.walk();
        let children: Vec<TsNode<'_>> = node.children(&mut cursor).collect();
        let kept: Vec<TsNode<'_>> = match &stored {
            None => children.clone(),
            Some(stored) => children.iter().zip(stored).filter_map(|(child, &kept)| kept.then_some(*child)).collect(),
        };
        if let Some(stored) = &stored {
            // The implied tokens after the last stored child: the pattern ends its children with
            // them, as one goal (`PatternNode::children_end`), which an ellipsis before it leaves.
            let last = stored.iter().rposition(|&kept| kept);
            let word: Vec<String> = children.iter().zip(stored).skip(last.map_or(0, |last| last + 1))
                .filter(|(_, kept)| !**kept)
                .filter_map(|(child, _)| child.utf8_text(source.as_bytes()).ok().map(str::to_owned))
                .collect();
            if !word.is_empty() {
                ends[at] = Some(word);
            }
        }
        stack.extend(kept.into_iter().rev());
    }
    let source = Arc::new(source.to_owned());
    let lines = Arc::new(LineTable::new(&source, Terminators::of(lang)));
    let doc = TapeDoc::restore_tree(source, grammar, &image, lines, &|| false)?;
    if ends.len() != doc.transducer_len() {
        return Err("a pattern's records disagree with its tree".into());
    }
    Ok((doc, PatternShape { ends, single }))
}

/// What a pattern's tree-sitter nodes say that its tape leaves out, per record
/// (`ast_grep_core::matcher::PatternShape`, `native/vendor/ast-grep-core-0.51.0/PATCHES.md`).
pub struct PatternShape {
    /// The implied tokens' texts, in order, where the node ended in implied tokens after its last
    /// child: its pattern ends its children there.
    pub(crate) ends: Vec<Option<Vec<String>>>,
    /// Whether tree-sitter's node is single (`is_single_node`): one child, or one and a MISSING
    /// or empty one. A tape's node can have one child where tree-sitter's had tokens too.
    pub(crate) single: Vec<bool>,
}

/// The tape document a node of any document is of, if a tape's.
pub fn tape_doc<'t, D: Doc>(node: &ast_grep_core::Node<'t, D>) -> Option<&'t TapeDoc> {
    let doc: &'t dyn std::any::Any = node.get_doc();
    doc.downcast_ref::<TapeDoc>()
}

// P's tape spike (v7, v8, v9np), for O8's pre-lock measurements.
#[allow(dead_code, unused_imports)]
pub mod v9np;

// cratify: `tests` is rerail-engine's (it mounts it)

// cratify: `corpus_parity` is rerail-lang-php's (it mounts it)
