//! Source-revision-bound rec6 preorder tape and transient ast-grep document.
//! No parser's tree survives encoding; restoration never invokes a parser. Every tape holds one
//! source, one grammar and one tree, and passes the same checks. A tree's syntax errors are its
//! ERROR and MISSING records: a Blade template's lexer errors are ERROR leaves, zero-width at their
//! offset, and a template Forte does not parse is a `document` holding one ERROR. A file that
//! embeds code (`embed`) is several documents (`FileImage`): its own tape, and beside it each
//! program it derives (`DerivedImage`), with a tape of its own over the program's text and the
//! segments that map that text back to the document it was derived from. The store keeps each
//! derived tape as a row of its own (`file_documents`); no tape holds another.
use rerail_blocks::vector::Vector;
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
use rerail_tape_format::tape::{CodeSet, EntryRef, Tape};
use tree_sitter::{Language, Node as TsNode, TreeCursor};

/// The format's constants are `rerail_tape_format`'s: the parsers write tapes with its writer
/// (decision 0140), and an image is one byte string however it was made.
pub use rerail_tape_format::{CODE_ERROR, CODE_MISSING, COLUMN_ALIGN, ERROR_CODE, FIELD_LIMIT, HEADER_BYTES, KIND_LIMIT, MAGIC as TAPE_MAGIC, SENTINEL, TAIL_FROM_START, header, padded};
pub use rerail_tape_format::CODE_EXTRA;
/// Format 16's writer and tables, for the parsers the engine's crates compile (JSON's kernels: `encode_16`).
pub use rerail_tape_format::f16;
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
// cratify: `token_readers` is rerail-workspace's (it mounts it)
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

    /// The tree-sitter language of this grammar's dialect (the predecessor's accessor; the packed runtime's
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
        // Read-locked for a grammar built already: every parse worker asks it for each document (0477).
        static GRAMMARS: std::sync::LazyLock<
            std::sync::RwLock<std::collections::HashMap<GrammarDialect, Arc<Grammar>>>,
        > = std::sync::LazyLock::new(Default::default);
        if let Some(grammar) = GRAMMARS.read().map_err(|_| "grammar cache poisoned")?.get(&lang) {
            return Ok(Arc::clone(grammar));
        }
        let mut grammars = GRAMMARS.write().map_err(|_| "grammar cache poisoned")?;
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

    /// Whether `parse` builds the file's programs itself (Blade's compiled view). Otherwise the router builds them over
    /// its tape, for a dialect that embeds others (`TapeImage::parse`).
    fn embeds_itself(&self) -> bool {
        false
    }

    /// The node kinds it parses a text from besides a whole file (decision 0466): a fragment's start.
    fn starts(&self) -> &'static [&'static str] {
        &[]
    }

    /// `source` parsed from the node kind `start` (one of `starts`), the tape's root one such node with none above
    /// it: `None` where the parser refuses the text (there is no tree-sitter fallback for a fragment), `Some(Err)`
    /// where the tape cannot be made.
    fn parse_from(&self, _grammar: &Grammar, _source: &str, _start: &str, _budget: &dyn Fn() -> bool) -> Option<Result<TapeImage, String>> {
        None
    }

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

/// `parser`'s documents of `source` (`TreeParser::parse`), with the programs a dialect that embeds others holds, built
/// over the parser's tape (`embed::host` reads it in place), unless the parser builds them itself: a file that embeds
/// code is read by its lean parser too. Every caller of a dialect's own parser reads a file through it.
pub fn parse_own(parser: &dyn TreeParser, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
    let parsed = parser.parse(grammar, source, budget)?;
    if parser.embeds_itself() || grammar.lang.embeds().is_empty() {
        return Some(parsed);
    }
    Some(parsed.and_then(|image| FileImage::embedding(image.primary, grammar, source, budget).map_err(TemplateError::Encode)))
}

/// `dialect`'s own parser, where its language registered one this host runs: the router's choice.
pub fn tree_parser(dialect: GrammarDialect) -> Option<&'static dyn TreeParser> {
    let router = ROUTER.get()?;
    router.chosen.iter().find(|(lang, _)| *lang == dialect).map(|&(_, i)| router.registered[i].parser)
}

/// The node kinds `dialect`'s hand-written parser parses a fragment from (decision 0466): none for a dialect only
/// tree-sitter parses.
pub fn starts(dialect: GrammarDialect) -> &'static [&'static str] {
    tree_parser(dialect).map_or(&[], |p| p.starts())
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

/// A document's tape: format 16 (or 16z) as its parser wrote it or the store kept it, read where it lies (decisions
/// 0156, 0293, 0434). Each node's code is a number in its language's table (made from its grammar, decision 0626).
#[derive(Clone)]
pub struct TapeImage {
    /// The tape in the format the run stores (decision 0293 §4).
    held: Held,
    pub language: GrammarDialect,
    pub source_len: usize,
    /// The source's SHA-256 where the image was encoded from it; not stored (v12), so `None` for
    /// an image read from bytes, whose store pairs it with its source.
    pub source_digest: Option<[u8; 32]>,
}

/// A stored tape's presence, read in place in its stored bytes (`f16::StoredPresence`).
pub use rerail_tape_format::f16::StoredPresence;

/// The presence the stored bytes `bytes` of a tape of tape tag `tag` hold after the tape, read in place; `None` where
/// they hold none.
pub fn stored_presence(bytes: &[u8], tag: u8) -> Option<StoredPresence<'_>> {
    StoredPresence::of(bytes, rerail_tape_format::f16::lang(tag)?)
}

/// The kind a presence records for a node of kind `kind`: its own, an ERROR node's `ERROR_CODE` (the tape's folded
/// code, decision 0434 §4). The kernels' `presence_kind`.
pub fn presence_kind(kind: u16) -> u16 {
    if kind == ERROR_SYMBOL { ERROR_CODE } else { kind }
}

/// Nodes a block's presence covers (`f16::PART`): block `b` is nodes `PRESENCE_BLOCK * b ..`.
pub const PRESENCE_BLOCK: usize = rerail_tape_format::f16::PART;

/// A tape as held, in one format.
#[derive(Clone)]
pub enum Held {
    F16(Arc<rerail_tape_format::f16::Tape16>),
    F16Z(Arc<rerail_tape_format::f16::Tape16Z>),
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
    /// The parent's bytes of the node it stands at, where they are not its text's (`embed::Program::at`: a Python
    /// string's program stands at the string, around its content); `None`: the innermost node over its text.
    pub at: Option<(u32, u32)>,
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
        let tape = match program.parse {
            crate::code_rule_dsl::embed::Parse::Tree(tree) => TapeImage::encode_tree_image(
                tree.root_node(),
                &grammar,
                &program.text,
                Some(&program.segments),
                budget,
            )?,
            // The parse left the placeholders out as it wrote the tape (`Parse::read_program`).
            crate::code_rule_dsl::embed::Parse::Tape(tape) => tape,
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
            at: program.at.map(|at| (at.start as u32, at.end as u32)),
            text: Arc::new(program.text),
            source_map: Arc::new(program.segments),
            tape,
        })
    }

    /// A stored derived document (`stored`, its tape `tape` read apart), checked as a tape of its own and against
    /// document `parent` it was derived from (of `parent_dialect`, `parent_len` bytes): an embedding that document's
    /// dialect has, a tape of that embedding's dialect over its text, a source map and host errors within both texts,
    /// and the role its map gives it.
    pub fn checked(stored: &StoredDerived<'_>, parent: u32, parent_dialect: GrammarDialect, parent_len: usize, tape: TapeImage) -> Result<Self, String> {
        let via = via_of_code(parent_dialect, stored.via).ok_or("unsupported embedding")?;
        let text = std::str::from_utf8(stored.text)
            .map_err(|_| "derived document text is not UTF-8")?
            .to_owned();
        let dialect = tape.language;
        if !parent_dialect.embeds().contains(&(via, dialect)) {
            return Err("a derived document's dialect is not one its parent embeds".into());
        }
        if tape.source_len != text.len() {
            return Err("a derived document's tape is not of its text".into());
        }
        let source_map = SegmentMap::read(stored.source_map, text.len(), parent_len)?;
        if !stored.host_errors.len().is_multiple_of(8) {
            return Err("truncated derived document host errors".into());
        }
        let host_errors = stored
            .host_errors
            .as_chunks::<8>()
            .0
            .iter()
            .map(|pair| {
                let (start, end) = pair.split_at(4);
                (u32::from_le_bytes(start.try_into().expect("u32 width")), u32::from_le_bytes(end.try_into().expect("u32 width")))
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
        Ok(DerivedImage {
            role,
            via,
            parent,
            text: Arc::new(text),
            source_map: Arc::new(source_map),
            host_errors: Arc::new(host_errors),
            tape,
            at: stored.at.filter(|&(start, end)| start <= end && end as usize <= parent_len),
        })
    }

    /// Its tape's grammar id in the store (`file_documents`' tapes): its dialect's tape tag, marked derived (bit 8), with
    /// a fingerprint of the placeholders its source map omits in the bits above (0 where it omits none). A derived
    /// tape is its text's tree less those placeholders: the same text under two maps omitting different ones has two
    /// tapes, and none need be the tape a file of that text parses to.
    pub fn stored_grammar_id(&self) -> i64 {
        derived_grammar_id(self.dialect(), &self.omission_bytes())
    }

    /// Queued for the store with its file's tape (decision 0431 step 6), as `file_documents` keeps it.
    pub fn queued(&self) -> Result<crate::content_store::QueuedDocument, String> {
        Ok(crate::content_store::QueuedDocument {
            parent_ordinal: i64::from(self.parent),
            role_id: self.role.code(),
            via_id: via_code(self.via),
            source: Arc::clone(&self.text),
            grammar_id: self.stored_grammar_id(),
            tape: self.tape.to_bytes()?,
            source_map: self.source_map_bytes(),
            host_errors: self.host_error_bytes(),
            at: self.at,
        })
    }

    pub fn source_map(&self) -> &Arc<SegmentMap> {
        &self.source_map
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

/// The store's grammar id of a derived tape in `dialect` that omits the placeholders `omissions` names
/// (`DerivedImage::omission_bytes`): see `DerivedImage::stored_grammar_id`.
pub fn derived_grammar_id(dialect: GrammarDialect, omissions: &[u8]) -> i64 {
    let fingerprint = if omissions.is_empty() {
        0
    } else {
        let digest: [u8; 32] = Sha256::digest(omissions).into();
        u64::from_be_bytes(digest[..8].try_into().expect("digest width")) >> 17
    };
    ((fingerprint as i64) << 9) | (1 << 8) | i64::from(crate::code_rule_dsl::language::tape_tag(dialect).expect("a tape tag"))
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
    /// The parent's bytes it stands at (`DerivedImage::at`).
    pub at: Option<(u32, u32)>,
}

impl FileImage {
    /// A cache for its documents' 16z tapes' blocks, one batch's (decision 0628 §6): a reader outside every batch reads
    /// the file's documents in it. None where no tape is 16z.
    pub fn z_cache(&self) -> Option<Arc<rerail_tape_format::f16::ZCache>> {
        let tapes: Vec<&rerail_tape_format::f16::Tape16Z> = std::iter::once(&self.primary).chain(self.derived.iter().map(|d| &d.tape)).filter_map(TapeImage::z_tape).collect();
        (!tapes.is_empty()).then(|| Arc::new(rerail_tape_format::f16::ZCache::of(tapes)))
    }

    /// The documents of a file that derives none.
    /// The documents of the host tape `primary` of `source`: itself, and the programs its
    /// embedders build from it, one per `via` its dialect embeds. The tape is read in place
    /// (`embed::host`), so a lean parser's host embeds what tree-sitter's does.
    pub fn embedding(primary: TapeImage, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Self, String> {
        let mut image = FileImage { primary, derived: Vec::new() };
        let embeds = grammar.lang.embeds();
        let mut programs = Vec::new();
        // The host read in a batch of its own: its 16z blocks decoded into a cache made for it.
        let cache = image.primary.own_cache();
        if let Some(root) = crate::code_rule_dsl::embed::host::Host::new(&image.primary, grammar, cache.as_ref()).root() {
            for (index, &(via, _)) in embeds.iter().enumerate() {
                if embeds[..index].iter().any(|&(earlier, _)| earlier == via) {
                    continue;
                }
                programs.extend(crate::code_rule_dsl::embed::embedder(via).build_all(root, source, budget).into_iter().map(|p| (via, p)));
            }
        }
        for (via, program) in programs {
            image.derived.push(DerivedImage::encode(via, program, 0, grammar, source, &budget)?);
        }
        Ok(image)
    }

    /// The documents of a file that derives none.
    pub fn single(primary: TapeImage, _source: &str) -> Self {
        Self { primary, derived: Vec::new() }
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
            let dialect = stored
                .tape
                .get(8)
                .and_then(|&tag| super::language::from_tape_tag(tag))
                .ok_or("unsupported derived document language")?;
            let tape = TapeImage::from_bytes(stored.tape, &Grammar::embedded(dialect)?)?;
            image.derived.push(DerivedImage::checked(stored, parent, parent_dialect, parent_len, tape)?);
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
                    at: document.at,
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

/// A tree-sitter node as the cursor walk reads it (`visit`): its code's kind and field, its span and flags, and its
/// contextual alias.
#[derive(Clone)]
pub struct Visit {
    pub kf: u16,
    pub start: u32,
    pub end: u32,
    pub extra: bool,
    pub error: bool,
    pub missing: bool,
    pub kind_alias: Option<(String, bool)>,
}

/// Where a walk writes a tree's nodes, in preorder (decision 0434 §2): format 16's ops (`formats::TreeOps`).
pub trait TreeOut {
    /// Node `v` under `parent` (an index this gave), open, a member of the supertypes `classes`: its index.
    fn open(&mut self, v: Visit, parent: Option<u32>, classes: &[u16]) -> Result<u32, String>;
    /// The innermost open node's subtree is written.
    fn close(&mut self);
}

/// Tree-sitter's tree under `root` written to `out` in preorder, `language` its grammar. An embedded
/// program's placeholders its segments mark (`omitted`) are left out, with their subtrees. Where
/// `filter` (the grammar's token automata) is given, a token it implies is skipped as the walk meets
/// it: it is never a record (`tokens::TokenGrammar::stored_children`, RER-131).
fn encode_tree(
    out: &mut (impl TreeOut + ?Sized),
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
            let classes = &classes[..count];
            if classes.iter().any(|&class| usize::from(class) >= KIND_LIMIT) {
                return Err("supertype symbol exceeds tape limit".into());
            }
            let index = out.open(visit(node, field, extra, language, &mut catalog)?, parent, classes)?;
            if cursor.goto_first_child() {
                above.push(parent);
                parent = Some(index);
                levels.push((stored.take(), place));
                stored = filter.and_then(|filter| filter.stored_children_in(node, filter_cursor.as_mut().expect("a cursor for the filter"), &mut scratch));
                place = 0;
                continue;
            }
            out.close();
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
            out.close();
            above.pop();
            parent = grandparent;
            if let Some(mask) = stored.take() { scratch.recycle(mask); }
            (stored, place) = levels.pop().unwrap_or_default();
        }
    }
}

/// `node` as the child in `field`, checked against the tape's limits.
fn visit<'l>(
    node: TsNode<'_>,
    field: u16,
    extra: bool,
    language: &'l Language,
    catalog: &mut [Option<(Option<&'l str>, bool)>],
) -> Result<Visit, String> {
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
    end.checked_sub(start).ok_or("inverted node span")?;
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
    Ok(Visit { kf: if error { ERROR_CODE } else { code } | (field << 10), start, end, extra, error, missing, kind_alias })
}

impl TapeImage {
    /// The tape as the store keeps it: format 16's (or 16z's) bytes as held.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(self.stored_len());
        self.emit(&mut |b| bytes.extend_from_slice(b))?;
        Ok(bytes)
    }

    /// The tape as the store keeps it (`to_bytes`), written straight into its home, a slab's room (decision 0353
    /// step 4a): the store's writer and every reader take it there.
    // copy (0353 step 4a): the parsers write their columns into a vector of their own, which goes once its stored
    // bytes are in their room. It goes when a parser writes into the room (decision 0353).
    pub fn place_stored(&self) -> Result<rerail_tape_format::home::Home, String> {
        const INTO_ROOM: rerail_prof::copies::Site = rerail_prof::copies::Site::new(17, "4a", "a parser's tape copied into its room (place_stored)");
        let len = self.stored_len();
        INTO_ROOM.time(|| (self.place_stored_in(len), len))
    }

    fn place_stored_in(&self, len: usize) -> Result<rerail_tape_format::home::Home, String> {
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
            Held::F16(t) => t.extent().len(),
            Held::F16Z(t) => t.extent().len(),
        }
    }

    /// The bytes the store keeps, in order, to `put`.
    fn emit(&self, put: &mut dyn FnMut(&[u8])) -> Result<(), String> {
        match &self.held {
            // The tape's home: the tape, and its presence after it (decision 0434 §4, step 6), stored and placed as one.
            Held::F16(t) => {
                put(t.extent());
                Ok(())
            }
            Held::F16Z(t) => {
                put(t.extent());
                Ok(())
            }
        }
    }

    /// Whether the tape's home holds its presence (decision 0434 §4, step 6).
    pub fn has_presence(&self) -> bool {
        match &self.held {
            Held::F16(t) => t.presence_bytes().is_some(),
            Held::F16Z(t) => t.presence_bytes().is_some(),
        }
    }

    /// The tape's auxiliary index in its home, after its presence (decision 0656 step 3), read where it lies.
    pub fn aux_bytes(&self) -> Option<&[u8]> {
        match &self.held {
            Held::F16(t) => t.aux_bytes(),
            Held::F16Z(t) => t.aux_bytes(),
        }
    }

    /// A stored tape placed in a room (decision 0353 step 4a), read where it lies: format 16 (or 16z) as stored.
    /// Nothing is copied.
    pub fn from_home(home: rerail_tape_format::home::Home, grammar: &Arc<Grammar>) -> Result<Self, String> {
        Self::sixteen(home, grammar)
    }

    /// A stored tape, in the format it was stored in (its magic): format 16 (or 16z) held as stored.
    pub fn from_bytes(bytes: &[u8], grammar: &Arc<Grammar>) -> Result<Self, String> {
        Self::sixteen(rerail_tape_format::Vector::from(bytes).into(), grammar)
    }

    /// A format 16 or 16z tape held as stored, in `bytes`: any other bytes (a format 15 image's included) are
    /// refused.
    fn sixteen(bytes: rerail_tape_format::home::Home, grammar: &Arc<Grammar>) -> Result<Self, String> {
        use rerail_tape_format::f16::{MAGIC16, MAGIC16Z, Tape16, Tape16Z};
        let magic = bytes.get(..8).ok_or("a short tape")?;
        if magic != MAGIC16 && magic != MAGIC16Z {
            return Err("not a format 16 or 16z tape".into());
        }
        let tag = super::language::tape_tag(grammar.lang).ok_or("unsupported tape language")?;
        let lang = rerail_tape_format::f16::lang(tag).ok_or("a format 16 tape of a language without its table")?;
        let held = if bytes[..8] == *MAGIC16 {
            let t = Tape16::from_bytes(bytes)?;
            if t.tag() != tag {
                return Err("tape image grammar mismatch".into());
            }
            t.reader(lang)?;
            Held::F16(Arc::new(t))
        } else {
            let t = Tape16Z::from_bytes(bytes)?;
            if t.tag() != tag {
                return Err("tape image grammar mismatch".into());
            }
            // Its language's choices checked: no read yet (a read is its batch's, through its cache).
            t.reader(lang, None)?;
            Held::F16Z(Arc::new(t))
        };
        let source_len = match &held {
            Held::F16(t) => t.source_len(),
            Held::F16Z(t) => t.source_len(),
        };
        let image = Self { held, language: grammar.lang, source_len: usize::try_from(source_len).map_err(|_| "source length overflow")?, source_digest: None };
        if image.node_count() == 0 {
            return Err("an empty tape".into());
        }
        Ok(image)
    }

    /// The tape as held.
    pub fn held(&self) -> &Held {
        &self.held
    }

    /// A cache for its 16z tape's blocks, a batch of its own (a reader outside every batch makes it for the tape it
    /// reads), where it is held so.
    pub fn own_cache(&self) -> Option<rerail_tape_format::f16::ZCache> {
        self.z_tape().map(|t| rerail_tape_format::f16::ZCache::of([t]))
    }

    /// Its ERROR and MISSING nodes, in preorder, as its writer listed them in its home (`f16::presence::faults`):
    /// format 16's and 16z's, where the home holds a presence.
    pub fn faults(&self) -> Option<&[u32]> {
        match &self.held {
            Held::F16(t) => t.faults(),
            Held::F16Z(t) => t.faults(),
        }
    }

    /// Its 16z tape, where it is held so.
    pub fn z_tape(&self) -> Option<&rerail_tape_format::f16::Tape16Z> {
        match &self.held {
            Held::F16Z(t) => Some(t),
            _ => None,
        }
    }

    /// Whether its bytes are placed in a room (a slab's, decision 0353 step 4a), not its own.
    pub fn placed(&self) -> bool {
        self.room().is_some()
    }

    /// The room the tape is placed in (`placed`), which says where the device reads it.
    pub fn room(&self) -> Option<&std::sync::Arc<dyn rerail_tape_format::home::Room>> {
        match &self.held {
            Held::F16(t) => t.home().room(),
            Held::F16Z(t) => t.home().room(),
        }
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
            && let Some(parsed) = parse_own(parser, grammar, source, &budget)
        {
            return match parsed {
                Ok(image) => Some(Ok(image)),
                // The parser stopped at a limit: like tree-sitter giving up, it read nothing.
                Err(TemplateError::Parser(_)) => None,
                Err(TemplateError::Encode(error)) => Some(Err(error)),
            };
        }
        // A `.rerail` file: the rule engine's own parser builds its tree (decision 0061), and its embedders the programs
        // it holds (decision 0495: its JSON objects, regexes and patterns).
        if grammar.lang == GrammarDialect::Rr {
            return Some(Self::encode_rr(grammar, source, budget).and_then(|image| FileImage::embedding(image, grammar, source, budget)));
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

    /// `source` parsed from the node kind `start` by `grammar`'s hand-written parser (decision 0466): `None` where its
    /// dialect has no parser that starts there. A text the parser refuses is one `ERROR` node over it (`refused`):
    /// no tree is made up for it, and a rule's `.on guest_error` sees it where it stands.
    pub fn parse_from(grammar: &Grammar, source: &str, start: &str, budget: &impl Fn() -> bool) -> Option<Result<FileImage, String>> {
        let parser = tree_parser(grammar.lang).filter(|p| p.starts().contains(&start))?;
        let image = parser.parse_from(grammar, source, start, &budget).unwrap_or_else(|| Self::refused(grammar, source, budget));
        Some(image.map(|image| FileImage::single(image, source)))
    }

    /// A refused fragment's tape (decision 0466): one `ERROR` node over the text less the blanks at its ends.
    pub fn refused(grammar: &Grammar, source: &str, budget: &impl Fn() -> bool) -> Result<Self, String> {
        let start = source.len() - source.trim_start().len();
        let end = source.trim_end().len().max(start);
        let kf = Code::error(0)?.kf();
        let visit = Visit { kf, start: start as u32, end: end as u32, extra: false, error: true, missing: false, kind_alias: None };
        Self::walked(grammar, source, 1, budget, |out| {
            out.open(visit.clone(), None, &[])?;
            out.close();
            Ok(())
        })
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
        Self::encode_tree_as(root, grammar, source, omitted, budget, true)
    }

    /// `encode_tree_image`, its blocks packed only where `packed` (`formats::TreeOps::of_run_as`).
    fn encode_tree_as(
        root: TsNode<'_>,
        grammar: &Grammar,
        source: &str,
        omitted: Option<&SegmentMap>,
        budget: &impl Fn() -> bool,
        packed: bool,
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
        let filter = if ALL_TOKENS.get() { None } else { tokens::TokenGrammar::of(grammar.lang, language) };
        Self::walked_as(grammar, source, root.descendant_count(), budget, packed, |out| encode_tree(out, root, language, filter.as_deref(), omitted, budget))
    }

    /// The tape of a `.rerail` file: the tree `logic::rr::syntax` builds as it reads the statements, its comments as
    /// extras and each statement that does not parse an ERROR leaf (decision 0061). The lean parser passes its nodes
    /// to format 16's ops as it parses (decisions 0140, 0434), so nothing here packs a node.
    pub fn encode_rr(grammar: &Grammar, source: &str, budget: &impl Fn() -> bool) -> Result<Self, String> {
        checked(budget)?;
        if grammar.lang != GrammarDialect::Rr {
            return Err("only a .rerail file is encoded from its rule tree".into());
        }
        if source.len() > u32::MAX as usize {
            return Err("source exceeds tape range".into());
        }
        // In the run's format, written as the parser goes (`formats::rr_tape`), with its source's digest (not stored
        // since format 12): the documents and the memo are keyed by it.
        Self::written_16(formats::rr_tape(source)?, grammar.lang, source, None, budget)
    }

    /// The tape `walk` writes in preorder, in the run's format (decision 0434 §2): by format 16's ops. `nodes` is about
    /// the nodes `walk` writes. A code the language's table lacks, or nodes past format 16's bounds, are an error.
    pub fn walked(
        grammar: &Grammar,
        source: &str,
        nodes: usize,
        budget: &impl Fn() -> bool,
        walk: impl FnOnce(&mut dyn TreeOut) -> Result<(), String>,
    ) -> Result<Self, String> {
        Self::walked_as(grammar, source, nodes, budget, true, walk)
    }

    /// `walked`, its blocks packed only where `packed` (`formats::TreeOps::of_run_as`).
    fn walked_as(
        grammar: &Grammar,
        source: &str,
        nodes: usize,
        budget: &impl Fn() -> bool,
        packed: bool,
        walk: impl FnOnce(&mut dyn TreeOut) -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut ops = formats::TreeOps::of_run_as(grammar.lang, source.as_bytes(), nodes, packed)?;
        walk(&mut ops)?;
        Self::written_16(ops.finish()?, grammar.lang, source, None, budget)
    }

    /// `source`'s lines as `terminators` end them (decision 0612): the tape's line index, read where it lies, where
    /// the tape is format 16's made with those terminators (its language's); else an index of their own, placed
    /// where the device reads tapes when `placed`.
    pub fn lines(&self, source: &str, terminators: crate::code_rule_dsl::lines::Terminators, placed: bool) -> LineTable {
        let same = crate::code_rule_dsl::lines::terminators(self.language) == terminators && self.source_len == source.len();
        match &self.held {
            Held::F16(t) if same => LineTable::of_f16(t),
            Held::F16Z(t) if same => LineTable::of_f16z(t),
            _ if placed => LineTable::placed(source, terminators),
            _ => LineTable::new(source, terminators),
        }
    }

    /// The format it is held in.
    pub fn format(&self) -> formats::Format {
        match self.held {
            Held::F16(_) => formats::Format::F16,
            Held::F16Z(_) => formats::Format::F16Z,
        }
    }

    /// The tape, read in place in the format it is held in (decision 0293). A 16z tape's reads need its batch's
    /// cache (`tape_in`): this one's reach none.
    pub fn tape(&self) -> rerail_tape_format::tape::AnyTape<'_> {
        self.tape_in(None)
    }

    /// The tape, a 16z tape's blocks decoded into `cache`, its batch's (`rerail_tape_format::f16::ZCache`).
    pub fn tape_in<'a>(&'a self, cache: Option<&'a rerail_tape_format::f16::ZCache>) -> rerail_tape_format::tape::AnyTape<'a> {
        use rerail_tape_format::tape::AnyTape;
        let lang = |tag: u8| rerail_tape_format::f16::lang(tag).expect("a format 16 tape is held with its language's table");
        match &self.held {
            Held::F16(t) => AnyTape::F16(t.reader(lang(t.tag())).expect("checked when held")),
            Held::F16Z(t) => AnyTape::F16Z(t.reader(lang(t.tag()), cache).expect("checked when held")),
        }
    }

    /// Its code table, as the tape API numbers its codes: the language's.
    pub fn code_table(&self) -> &rerail_tape_format::tape::CodeTable {
        self.lang().expect("a format 16 tape is held with its language's table").table()
    }

    /// The preorders of nodes whose code carries `flag`.
    #[cfg(any(test, feature = "test-hooks"))]
    fn flagged(&self, flag: u8) -> Vector<u32> {
        use rerail_tape_format::tape::Tape;
        rerail_tape_format::with_tape!(self.tape(), |t| (0..t.len()).filter(|&i| t.table().entry(t.code(i)).flags & flag != 0).map(|i| i as u32).collect())
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn error_nodes(&self) -> Vector<u32> {
        self.flagged(CODE_ERROR)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn missing_nodes(&self) -> Vector<u32> {
        self.flagged(CODE_MISSING)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn extra_nodes(&self) -> Vector<u32> {
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
            Held::F16(t) => t.len(),
            Held::F16Z(t) => t.len(),
        }
    }

    /// Node `i`'s subtree size, itself included.
    pub fn subtree_size(&self, i: usize) -> u32 {
        use rerail_tape_format::tape::Tape;
        rerail_tape_format::with_tape!(self.tape(), |t| (t.subtree_end(i) - i) as u32)
    }

    /// The bytes it holds.
    pub(super) fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.held {
                Held::F16(t) => t.bytes(),
                Held::F16Z(t) => t.bytes(),
            }
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
    /// A run's reads (decision 0442: the run's): each part's, each pattern's, and with `E17_SITES` each site's.
    #[derive(Default)]
    struct Reads {
        counts: [AtomicU64; PARTS.len()],
        patterns: [[AtomicU64; CLASSES.len()]; PATTERN_PARTS.len()],
        sites: std::sync::Mutex<Option<std::collections::HashMap<Site, [u64; 4]>>>,
    }

    fn reads() -> std::sync::Arc<Reads> {
        rerail_prof::code_rule_dsl::logic::profile::local::<Reads>()
    }

    pub fn hit(part: usize, n: u64) {
        reads().counts[part].fetch_add(n, Relaxed);
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
    type Site = (usize, &'static std::panic::Location<'static>);

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
        let reads = reads();
        reads.patterns[part][class].fetch_add(1, Relaxed);
        if rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("E17_SITES").is_some()) {
            let site = std::panic::Location::caller();
            reads.sites.lock().expect("the sites").get_or_insert_default().entry((part, site)).or_default()[class] += 1;
        }
    }

    #[inline]
    pub fn bulk(part: usize, n: usize) {
        if PAUSED.with(|p| p.get()) == 0 {
            reads().patterns[part][4].fetch_add(n as u64, Relaxed);
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
        let reads = reads();
        let nodes = reads.counts[part("nodes restored")].load(Relaxed).max(1) as f64;
        eprintln!("tape reads (E7):");
        for (name, count) in PARTS.iter().zip(&reads.counts) {
            let count = count.load(Relaxed);
            eprintln!("  {name:<16} {count:>14} {:>10.3} per restored node", count as f64 / nodes);
        }
        let facts = reads.counts[part("facts nodes")].load(Relaxed).max(1) as f64;
        eprintln!("read patterns (E17), per facts node ({} facts nodes):", facts as u64);
        eprintln!("  {:<18} {}", "", CLASSES.iter().map(|c| format!("{c:>12}")).collect::<String>());
        for (name, row) in PATTERN_PARTS.iter().zip(&reads.patterns) {
            eprintln!("  {name:<18} {}", row.iter().map(|c| format!("{:>12.4}", c.load(Relaxed) as f64 / facts)).collect::<String>());
        }
        eprintln!("read patterns (E17), counts:");
        for (name, row) in PATTERN_PARTS.iter().zip(&reads.patterns) {
            eprintln!("  {name:<18} {}", row.iter().map(|c| format!("{:>12}", c.load(Relaxed))).collect::<String>());
        }
        if let Some(sites) = reads.sites.lock().expect("the sites").take() {
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
fn kind_of(e: EntryRef<'_>) -> u16 {
    if e.flags & CODE_ERROR != 0 { ERROR_SYMBOL } else { e.kind }
}

/// Whether a node of code `e` is named: its alias's flag, else its kind's.
#[inline]
fn named_of(grammar: &Grammar, e: EntryRef<'_>) -> bool {
    e.alias.map_or_else(|| grammar.kind_is_named(kind_of(e)), |alias| alias.1)
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
    labels: Arc<Vec<Vector<u64>>>,
    /// Text tests read from the term index (`with_texts`, `prefilter::terms`, `R[tid]`): per
    /// slot key (`Micro::slot_key`), a bit per node.
    texts: Arc<std::collections::BTreeMap<String, Arc<Vector<u64>>>>,
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
}

pub use rerail_tape_format::f16::ZCache;

/// A cache for `tapes`' 16z blocks: a batch of the documents a caller reads (`TapeDoc::in_batch`).
pub fn cache_of<'a>(tapes: impl IntoIterator<Item = &'a rerail_tape_format::f16::Tape16Z>) -> rerail_tape_format::f16::ZCache {
    rerail_tape_format::f16::ZCache::of(tapes)
}

pub type KindPostings = rustc_hash::FxHashMap<u16, Vector<u32>>;

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
            .get_or_init(|| LineTable::new(&self.source, crate::code_rule_dsl::lines::Terminators::Lf))
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
    /// The document over `held`, a tape the facts were read off and checked then (`facts::core`): the same image,
    /// read in place, with nothing copied or checked again (decision 0353). An embedder that reads the host through
    /// the scope table reads it so (decision 0470).
    pub fn over(held: Arc<formats::DocTape>, grammar: Arc<Grammar>, lines: Arc<LineTable>) -> Self {
        let image = Arc::clone(held.image_arc());
        let source = Arc::clone(held.source());
        Self {
            source_digest: image.source_digest.unwrap_or_default(),
            points: Arc::new(Points { source: Arc::clone(&source), rows: std::sync::OnceLock::new() }),
            held,
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
        }
    }

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
        // The file's documents are read outside every batch until one takes them (`TapeDoc::in_batch`): a batch of
        // their own, one cache for their 16z tapes' blocks (decision 0628 §6).
        let cache = image.z_cache();
        let mut host = Self::restore_tree(source, grammar, &image.primary, lines, cache.clone(), budget)?;
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
                match Self::restore_guest(&linked, guest, cache.clone(), budget) {
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
        cache: Option<Arc<rerail_tape_format::f16::ZCache>>,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let grammar = Grammar::embedded(guest.tape.language)?;
        // Positions read through the segments slice both texts: a map that does not fit them is
        // refused here, so the tape is encoded again, never read wrong.
        guest.source_map.check(&guest.text, &host.source)?;
        let lines = Arc::new(guest.tape.lines(&guest.text, crate::code_rule_dsl::lines::terminators(grammar.lang), false));
        let mut doc =
            Self::restore_tree(Arc::clone(&guest.text), grammar, &guest.tape, lines, cache, budget)?;
        checked(budget)?;
        doc.host = Some(Arc::new(HostLink {
            via: guest.via,
            segments: Arc::clone(&guest.source_map),
            host_errors: Arc::clone(&guest.host_errors),
            host: host.clone(),
        }));
        Ok(doc)
    }

    /// The document of `image`'s own tree over `source`, a 16z tape's blocks decoded into `cache`: its file's batch's.
    fn restore_tree(
        source: Arc<String>,
        grammar: Arc<Grammar>,
        image: &TapeImage,
        lines: Arc<LineTable>,
        cache: Option<Arc<rerail_tape_format::f16::ZCache>>,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        read!("docs restored");
        read!("nodes restored", image.node_count());
        // The image is checked node by node and held as it is: the document reads its tape in place (decision
        // 0293), so nothing is copied out of it.
        let (digest, _) = validated::visit(&source, &grammar, image, budget, |_, _| {})?;
        checked(budget)?;
        let image = Arc::new(image.clone());
        let held = Arc::new(formats::DocTape::in_batch(Arc::clone(&image), Arc::clone(&source), cache));
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

    /// Whether the document has an ERROR node or a MISSING token: whether its writer listed a fault in its home
    /// (`TapeImage::faults`: no node read, no block decoded); else its presence's fault bit (`presence::kind_word`);
    /// for a tape with no presence (format 15's), its nodes read.
    pub fn has_syntax_error(&self) -> bool {
        use rerail_tape_format::tape::Tape;
        if let Some(faults) = self.doc_tape().image().faults() {
            return !faults.is_empty();
        }
        if self.has_presence() {
            self.tape().has_kind(u32::MAX, ERROR_CODE)
        } else {
            self.syntax_error_nodes().next().is_some()
        }
    }

    /// The nodes `syntax_error_ranges` reads, each with its preorder index: the outermost of the faults its writer
    /// listed in its home (`TapeImage::faults`), each read in its block alone (in 16z, the only blocks decoded); a
    /// format 15 tape's in the blocks whose presence has the fault bit (`presence::kind_word`).
    pub fn syntax_error_nodes(&self) -> impl Iterator<Item = (usize, Range<usize>)> + '_ {
        use rerail_tape_format::tape::Tape;
        let mut found = Vec::new();
        let listed = self.doc_tape().image().faults();
        rerail_tape_format::with_tape!(self.tape(), |t| {
            let n = t.len();
            if let Some(faults) = listed {
                // A fault inside one found is not one: `skip` passes its subtree.
                let mut skip = 0;
                for &i in faults {
                    let i = i as usize;
                    if i < skip || i >= n {
                        continue;
                    }
                    found.push((i, t.start(i) as usize..t.end(i) as usize));
                    skip = t.subtree_end(i);
                }
            }
            // Format 15: none where the document's presence has no fault bit. A node inside a fault found is not one:
            // `i` passes its subtree, into the blocks after it too.
            let blocks = if listed.is_none() && t.has_kind(u32::MAX, ERROR_CODE) { n.div_ceil(PRESENCE_BLOCK) } else { 0 };
            let mut i = 0;
            for b in 0..blocks {
                let end = ((b + 1) * PRESENCE_BLOCK).min(n);
                if i >= end || !t.has_kind(u32::try_from(b).unwrap_or(u32::MAX - 1), ERROR_CODE) {
                    continue;
                }
                i = i.max(b * PRESENCE_BLOCK);
                while i < end {
                    if t.table().entry(t.code(i)).flags & (CODE_ERROR | CODE_MISSING) != 0 {
                        found.push((i, t.start(i) as usize..t.end(i) as usize));
                        i = t.subtree_end(i);
                    } else {
                        i += 1;
                    }
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
        let lines = Arc::new(LineTable::new(&source, crate::code_rule_dsl::lines::terminators(grammar.lang)));
        Self::restore_with_lines(source, grammar, &image.file_image(), lines, budget)
    }

    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    pub fn index_source_digest(&self) -> &[u8; 32] {
        &self.source_digest
    }

    /// The tree-sitter row (`TSPoint`'s, from 0) holding `byte`, as a native tree's node positions give it.
    pub fn row(&self, byte: usize) -> usize {
        self.points.point(byte as u32).line()
    }

    /// The source's line table, shared with the evaluator that restored this document: the one line index every
    /// reader reads in place (decision 0612), the batch's resident and copied lines among them (`packed_batch`).
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
    fn entry(&self, node: u32) -> EntryRef<'_> {
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

    /// Whether a node of the document may have kind `kind` (`transducer_kind`'s), by its image's presence (decision
    /// 0434 §4), read before any node is: `false` only where the writer saw no node of that kind. An error node's
    /// kind is `ERROR_SYMBOL`, which no presence holds, so it is always possible.
    pub fn may_have_kind(&self, kind: u16) -> bool {
        use rerail_tape_format::tape::Tape;
        rerail_tape_format::with_tape!(self.tape(), |t| t.has_kind(u32::MAX, presence_kind(kind)))
    }

    /// Whether block `b` (nodes `PRESENCE_BLOCK * b ..`) may hold a node of kind `kind`: as `may_have_kind`, by the
    /// block's presence where the tape holds it, else the document's (the same read as the kernels', `tape_has_kind`).
    pub fn block_may_have_kind(&self, b: usize, kind: u16) -> bool {
        use rerail_tape_format::tape::Tape;
        rerail_tape_format::with_tape!(self.tape(), |t| t.has_kind(u32::try_from(b).unwrap_or(u32::MAX - 1), presence_kind(kind)))
    }

    /// `transducer_nodes_where` in the blocks `block` admits alone (`block_may_have_kind` tells them): a block left
    /// out has none of its codes read (the kernels' `rule_may_in`, the same read).
    pub fn transducer_nodes_where_in(&self, admits: impl Fn(u16, &[u16]) -> bool, block: &dyn Fn(u32) -> bool, out: &mut Vector<u32>) {
        let t = self.t();
        // The codes' set, a bit a code, in a block.
        let mut bits = rerail_blocks::vector![0u32; t.table().set_words()];
        t.table().set_into(|e| admits(kind_of(e), e.classes), &mut bits);
        let set = CodeSet::Dense(&bits, t.table().len() as u32);
        t.match_groups_in(&set, block, |base, mut m| {
            while m != 0 {
                out.push((base + m.trailing_zeros() as usize) as u32);
                m &= m - 1;
            }
        });
    }

    /// Whether the tape's home holds its presence (decision 0434 §4, step 6): a format 16 or 16z writer's.
    pub fn has_presence(&self) -> bool {
        self.image.has_presence()
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
    pub(in crate::code_rule_dsl) fn named_kinds(&self) -> Vector<u16> {
        let mut kinds: Vector<u16> = (0..self.grammar.named.len() as u16).filter(|&kind| self.grammar.kind_is_named(kind)).collect();
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
            let children: Vector<u32> = tape.transducer_children(node).collect();
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
    pub fn transducer_nodes_where(&self, admits: impl Fn(u16, &[u16]) -> bool, out: &mut Vector<u32>) {
        let t = self.t();
        // The codes' set, a bit a code, in a block.
        let mut bits = rerail_blocks::vector![0u32; t.table().set_words()];
        t.table().set_into(|e| admits(kind_of(e), e.classes), &mut bits);
        let set = CodeSet::Dense(&bits, t.table().len() as u32);
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
    pub fn transducer_labels(&self) -> &[Vector<u64>] {
        &self.labels
    }

    /// The labels, shared (`transducer_labels`).
    pub fn transducer_labels_shared(&self) -> Arc<Vec<Vector<u64>>> {
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
    pub fn with_labels(&self, labels: Arc<Vec<Vector<u64>>>) -> Self {
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
    pub fn with_texts(&self, texts: std::collections::BTreeMap<String, Arc<Vector<u64>>>) -> Self {
        Self {
            texts: Arc::new(texts),
            ..self.clone()
        }
    }

    /// A text test's row from the term index, by its slot key (`with_texts`).
    pub fn transducer_text_row(&self, key: &str) -> Option<&Arc<Vector<u64>>> {
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
    fn entry(&self) -> EntryRef<'a> {
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
    /// The parent, then its parent, and so on: a parent read each, at most `K - 1` sibling steps in format 16. A
    /// climb mostly stops a few levels up (the nearest function or class), where a descent from the root
    /// (`Tape::path`) read every sibling before the path at each level: in a module of hundreds of statements, the
    /// reference graph's climbs from its parameters were a sixth of its time (0465, Q334).
    fn ancestors(&self, _root: Self) -> impl Iterator<Item = Self> {
        std::iter::successors(self.parent(), Self::parent)
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
        Cow::Borrowed(e.alias.map_or_else(|| self.grammar.kind_name(kind_of(e)).expect("validated grammar symbol"), |alias| alias.0))
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
        // The end from the start in hand (format 16: the start plus the length), not a second search for the start.
        let start = self.tape.start(self.index as usize);
        start as usize..self.tape.end_with_start(self.index as usize, start) as usize
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
    // Read here, where it is written, and never stored: format 16, its blocks not packed whatever the run stores.
    let image = TapeImage::encode_tree_as(root, &grammar, source, None, &|| false, false)?;
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
    let lines = Arc::new(LineTable::new(&source, crate::code_rule_dsl::lines::terminators(lang)));
    let cache = image.own_cache().map(Arc::new);
    let doc = TapeDoc::restore_tree(source, grammar, &image, lines, cache, &|| false)?;
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

// cratify: `tests` is rerail-engine's (it mounts it)

// cratify: `corpus_parity` is rerail-lang-php's (it mounts it)
