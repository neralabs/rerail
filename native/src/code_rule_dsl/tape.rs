//! Source-revision-bound rec6 preorder tape and transient ast-grep document.
//! No parser's tree survives encoding; restoration never invokes a parser. Every tape holds one
//! source, one grammar and one tree, and passes the same checks. A tree's syntax errors are its
//! ERROR and MISSING records: a Blade template's lexer errors are ERROR leaves, zero-width at their
//! offset, and a template Forte does not parse is a `document` holding one ERROR. A file that
//! embeds code (`embed`) is several documents (`FileImage`): its own tape, and beside it each
//! program it derives (`DerivedImage`), with a tape of its own over the program's text and the
//! segments that map that text back to the document it was derived from. The store keeps each
//! derived tape as a row of its own (`file_documents`); no tape holds another.
use crate::code_rule_dsl::blade::forte::tree::{BladeNode, BladeTree, parse_tree};
use crate::code_rule_dsl::blade::view::{ReadError, View};
use crate::code_rule_dsl::embed::segments::{FileRanges, SegmentMap};
use crate::code_rule_dsl::embed::{Embedder, Program};
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
use tree_sitter::{Language, Node as TsNode, TreeCursor};

/// v9np (decision 0025): structure-only, regular columns, no parent column; 10: supertype
/// memberships folded into the codes.
pub(super) const TAPE_FORMAT_VERSION: u32 = 11;
const TAPE_MAGIC: &[u8; 8] = b"TMPTAP9N";
/// The image header: magic, language, the three column widths, version, source length and
/// digest, and five table lengths: nodes, codes, the code table's bytes, aliases, the alias
/// table's bytes.
const HEADER_BYTES: usize = 56 + 5 * 8;
/// Each column starts at this alignment from the image's start (format 11), so a reader of the
/// image in place, as the device reads a resident tape (PIAT §20.2.3), reads aligned columns.
pub(super) const COLUMN_ALIGN: usize = 16;

/// `length` rounded up to `COLUMN_ALIGN`.
const fn padded(length: usize) -> usize {
    length.div_ceil(COLUMN_ALIGN) * COLUMN_ALIGN
}
/// `TapeImage::widths[2]`'s high bit: a tail is measured from the node's own start, not from
/// its last descendant's (for a tree where a node ends before its last descendant starts).
pub(in crate::code_rule_dsl) const TAIL_FROM_START: u8 = 0x80;
/// A code's flags.
const CODE_EXTRA: u8 = 1;
const CODE_ERROR: u8 = 2;
const CODE_MISSING: u8 = 4;
/// A named-only tape keeps all of the node's anonymous children (`tokens::fold`).
const CODE_TOKENS: u8 = 8;
const KIND_LIMIT: usize = 1024;
const FIELD_LIMIT: usize = 64;
const SENTINEL: u16 = u16::MAX;
const NONE: u32 = u32::MAX;
const CHECKPOINT_LIMIT: usize = 0x00ff_ffff;
/// Tree-sitter's ERROR symbol does not fit the 10-bit kind code. ERROR records store the
/// grammar's hidden end symbol, which never labels a visible node, and are listed in `error`.
const ERROR_CODE: u16 = 0;

// P's index columns, read by `prefilter`'s `nera_tree` (the §17 SQL fallback, tests only so far).
#[allow(dead_code)]
mod index;
#[allow(unused_imports)]
pub(in crate::code_rule_dsl) use index::IndexImage;
mod wire;
pub(in crate::code_rule_dsl) mod tokens;
#[cfg(test)]
mod tokens_tests;
pub(in crate::code_rule_dsl) mod resident;
pub(super) use wire::index_span;
use wire::{CHECKPOINT_STRIDE, WIDTH};
const ERROR_SYMBOL: u16 = u16::MAX;
// QueryCursor uses the same fixed capacity in the pinned Tree-sitter query runtime.
const MAX_NODE_SUPERTYPES: usize = 8;

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
#[cfg(test)]
fn cursor_supertypes(cursor: &TreeCursor<'_>) -> ([u16; MAX_NODE_SUPERTYPES], usize) {
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

#[cfg(test)]
thread_local! {
    /// The largest table capacity `TapeImage::from_bytes` reserved on this thread since the last
    /// `take_table_reservation`.
    static TABLE_RESERVATION: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn note_table_reservation(capacity: usize) {
    TABLE_RESERVATION.set(TABLE_RESERVATION.get().max(capacity));
}

/// The largest alias or override table capacity reserved since the last call, then resets it.
#[cfg(test)]
pub(super) fn take_table_reservation() -> usize {
    TABLE_RESERVATION.replace(0)
}

/// Deterministic grammar-wide codes: kind code is the dialect's kind id, field code its field id
/// (zero means no field), both as `GrammarDialect::kinds` numbers them.
pub(super) struct Grammar {
    lang: GrammarDialect,
    kinds: KindCatalog,
    /// Each kind code's name and whether it is named, read once: Tree-sitter answers a name
    /// by measuring and validating its C string on every call, and `kind` is the matcher's
    /// most frequent question.
    named: Vec<(Option<&'static str>, bool)>,
}

impl Grammar {
    /// The name of field `code` (M's facts-a, which read the private catalog directly).
    #[allow(dead_code)]
    pub(super) fn field_name(&self, code: u16) -> Option<&'static str> {
        self.kinds.field_name(code)
    }

    /// The tree-sitter language of this grammar's dialect (Tempo's accessor; the packed runtime's
    /// Metal tests compile queries against it).
    #[cfg(test)]
    pub(super) fn language(&self) -> Language {
        self.lang.get_ts_language()
    }

    #[cfg(test)]
    pub(super) fn field_code(&self, name: &str) -> Option<u16> {
        self.kinds.field_id(name)
    }
    #[cfg(test)]
    pub(super) fn has_field(&self, name: &str) -> bool {
        self.kinds.field_id(name).is_some()
    }
    pub(super) fn new(lang: GrammarDialect) -> Result<Arc<Self>, String> {
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
        Ok(Arc::new(Self { lang, kinds, named }))
    }

    /// The grammar of an embedded program's dialect, built once per process.
    fn embedded(lang: GrammarDialect) -> Result<Arc<Self>, String> {
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

    fn kind_name(&self, code: u16) -> Option<&'static str> {
        match self.named.get(usize::from(code)) {
            Some((name, _)) => *name,
            // ERROR's code lies past the grammar's symbols.
            None => self.kinds.kind_name(code),
        }
    }

    fn kind_is_named(&self, code: u16) -> bool {
        match self.named.get(usize::from(code)) {
            Some((_, named)) => *named,
            None => self.kinds.kind_is_named(code),
        }
    }
}

/// Why a Blade template has no tape (`TapeImage::encode_template`).
#[derive(Debug)]
pub(crate) enum TemplateError {
    /// Forte stopped at a limit (its nesting depth, its step budget) on the template or on a
    /// template it holds: that says nothing about its syntax, and nothing was read.
    Parser(crate::code_rule_dsl::blade::forte::ParseError),
    /// Interrupted, or no tape could be encoded.
    Encode(String),
}

impl From<String> for TemplateError {
    fn from(error: String) -> Self {
        Self::Encode(error)
    }
}

/// Owns only compact structure and a binding to the exact source and grammar.
/// The first derived class id (`TapeDoc::with_labels`), above every grammar's kinds and supertypes.
pub(super) const DERIVED_CLASS_BASE: u16 = 0xf000;

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
pub(super) struct TapeImage {
    code: Vec<u8>,
    size: Vec<u8>,
    base: Vec<u8>,
    off: Vec<u8>,
    tail: Vec<u8>,
    widths: [u8; 3],
    codes: Vec<Code>,
    /// Contextual Tree-sitter aliases whose displayed kind or named flag differs from the static
    /// language catalog. The numeric kind remains the native `kind_id` for matcher parity.
    kind_aliases: Vec<(String, bool)>,
    language: GrammarDialect,
    source_len: usize,
    source_digest: [u8; 32],
}

/// A folded code: kind (10 bits, ERROR's is `ERROR_CODE`) and field (6 bits) as v8's `kf`, the
/// flags, the contextual alias (`SENTINEL`: none), and the node's query-independent supertype
/// memberships (abstract grammar symbols, ascending; unused slots zero). Memberships depend on
/// where the node sits, not on its kind alone, so they are the node's own, as the parser's
/// cursor reports them; nodes in the same context share a code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Code {
    kf: u16,
    flags: u8,
    alias: u16,
    class_count: u8,
    classes: [u16; MAX_NODE_SUPERTYPES],
}

impl Code {
    fn classes(&self) -> &[u16] {
        &self.classes[..usize::from(self.class_count)]
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
pub(super) enum DocumentRole {
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
    fn derived(text: &str, source_map: &SegmentMap) -> Self {
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
    pub(super) fn code(self) -> i64 {
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
pub(super) fn via_code(via: Via) -> i64 {
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
pub(super) struct FileImage {
    pub(super) primary: TapeImage,
    /// In ordinal order (`DerivedImage::parent` counts the primary as 0, the first of these as 1).
    pub(super) derived: Vec<DerivedImage>,
}

/// A document derived from another of the file's: its own text, grammar and tape, and how it maps
/// to the document it was derived from.
#[derive(Clone)]
pub(super) struct DerivedImage {
    /// `Embedded` or `Generated`, never `Primary`.
    pub(super) role: DocumentRole,
    /// The embedding that built it.
    pub(super) via: Via,
    /// The document it was derived from: 0 the primary, `i` the `i`-th derived one. Embedders run
    /// on the primary only; the field makes nesting representable.
    pub(super) parent: u32,
    pub(super) text: Arc<String>,
    /// Where its text stands in its parent's (`embed::segments`).
    pub(super) source_map: Arc<SegmentMap>,
    /// The parent's bytes it cannot be read through (`embed::Program::host_errors`).
    pub(super) host_errors: Arc<Vec<(u32, u32)>>,
    /// Its tree over `text`, in its own dialect.
    pub(super) tape: TapeImage,
}

impl DerivedImage {
    /// The image of `program`, which `via` built from document `parent` of `host`'s dialect over
    /// `source`.
    fn encode(
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
        let tape = TapeImage::encode_tree_image(
            program.root(),
            &grammar,
            &program.text,
            Some(&program.segments),
            budget,
        )?;
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
    pub(super) fn dialect(&self) -> GrammarDialect {
        self.tape.language
    }

    /// Its source map's persisted form (`SegmentMap::write`).
    /// The program ranges its tape leaves out (`SegmentMap::omissions`), as little-endian `u32`
    /// pairs: with its text, all its tape depends on.
    pub(super) fn omission_bytes(&self) -> Vec<u8> {
        self.source_map
            .omissions()
            .flat_map(|range| [range.start as u32, range.end as u32])
            .flat_map(u32::to_le_bytes)
            .collect()
    }

    pub(super) fn source_map_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            SegmentMap::persisted_len(self.source_map.len()).unwrap_or_default(),
        );
        self.source_map.write(&mut bytes);
        bytes
    }

    /// Its host errors' persisted form: each range's start and end, little-endian `u32`s.
    pub(super) fn host_error_bytes(&self) -> Vec<u8> {
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
pub(super) struct StoredDerived<'a> {
    pub(super) ordinal: i64,
    pub(super) parent: i64,
    pub(super) role: i64,
    pub(super) via: i64,
    pub(super) text: &'a [u8],
    pub(super) source_map: &'a [u8],
    pub(super) host_errors: &'a [u8],
    pub(super) tape: &'a [u8],
}

impl FileImage {
    /// The documents of a file that derives none.
    pub(super) fn single(primary: TapeImage) -> Self {
        Self {
            primary,
            derived: Vec::new(),
        }
    }

    /// The documents the store keeps for a file: its tape `primary` in `grammar`'s dialect and
    /// the documents derived from it, in ordinal order from 1. Each derived document is checked
    /// as a tape of its own, and against the document it was derived from: an embedding that
    /// document's dialect has, a tape of that embedding's dialect over its text, a source map and
    /// host errors within both texts, and the role its map gives it.
    pub(super) fn read(
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
    #[cfg(test)]
    pub(super) fn round_trip(&self, grammar: &Arc<Grammar>) -> Result<Self, String> {
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

    pub(super) fn estimated_bytes(&self) -> usize {
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
#[cfg(test)]
pub(super) trait Documents {
    fn file_image(&self) -> Cow<'_, FileImage>;
}

#[cfg(test)]
impl Documents for FileImage {
    fn file_image(&self) -> Cow<'_, FileImage> {
        Cow::Borrowed(self)
    }
}

#[cfg(test)]
impl Documents for TapeImage {
    fn file_image(&self) -> Cow<'_, FileImage> {
        Cow::Owned(FileImage::single(self.clone()))
    }
}

#[derive(Clone)]
struct Pending {
    kf: u16,
    parent: Option<u32>,
    start: u32,
    length: u32,
    size: u32,
    extra: bool,
    error: bool,
    missing: bool,
    kind_alias: Option<(String, bool)>,
    /// A named-only tape keeps all of this node's anonymous children (`tokens::fold`).
    tokens_kept: bool,
}

fn checked(budget: &impl Fn() -> bool) -> Result<(), String> {
    if budget() {
        Err("tape operation interrupted".into())
    } else {
        Ok(())
    }
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

fn checkpoint(value: usize, bytes: &mut Vec<u8>) -> Result<(), String> {
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

/// Appends `root`'s tree to `nodes` in preorder, `language` its grammar. An embedded program's
/// placeholders its segments mark (`omitted`) are left out, with their subtrees.
fn encode_tree(
    nodes: &mut Vec<Pending>,
    supertypes: &mut Vec<(u32, u16)>,
    root: TsNode<'_>,
    language: &Language,
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
    let mut visited = 0usize;
    loop {
        // Every 256 nodes, omitted ones included.
        if visited.is_multiple_of(256) {
            checked(budget)?;
        }
        visited += 1;
        let node = cursor.node();
        let index = if omitted.is_some_and(|segments| segments.omitted(node.byte_range())) {
            None
        } else {
            // The cursor's root is `root` itself, which has no field.
            // One status call gives the field and the supertypes (the field alone walks the stack again).
            let (classes, count, field) = cursor_status(&cursor);
            let field = if node.is_extra() { 0 } else { field };
            let index = encode_node(nodes, node, parent, field, language, &mut catalog)?;
            for class in classes.into_iter().take(count) {
                if usize::from(class) >= KIND_LIMIT {
                    return Err("supertype symbol exceeds tape limit".into());
                }
                supertypes.push((index, class));
            }
            Some(index)
        };
        if let Some(index) = index
            && cursor.goto_first_child()
        {
            above.push(parent);
            parent = Some(index);
            continue;
        }
        // The next sibling, of this node or of its nearest ancestor that has one.
        loop {
            let Some(grandparent) = above.last().copied() else {
                return Ok(());
            };
            if cursor.goto_next_sibling() {
                break;
            }
            cursor.goto_parent();
            above.pop();
            parent = grandparent;
        }
    }
}

/// Appends `node` under `parent` as the child in `field`; its tape index.
fn encode_node<'l>(
    nodes: &mut Vec<Pending>,
    node: TsNode<'_>,
    parent: Option<u32>,
    field: u16,
    language: &'l Language,
    catalog: &mut [Option<(Option<&'l str>, bool)>],
) -> Result<u32, String> {
    let index = u32::try_from(nodes.len()).map_err(|_| "rec6 node count overflow")?;
    let kind = usize::from(node.kind_id());
    let error = node.is_error();
    if usize::from(field) >= FIELD_LIMIT {
        return Err(format!(
            "grammar field id {field} exceeds tape limit {FIELD_LIMIT}"
        ));
    }
    let code = node.kind_id();
    if error {
        if code != ERROR_SYMBOL || node.is_missing() {
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
    } else if node.is_missing() && !node.byte_range().is_empty() {
        return Err("missing grammar symbol has a non-empty source span".into());
    }
    let span = node.byte_range();
    let start = u32::try_from(span.start).map_err(|_| "node span overflow")?;
    let end = u32::try_from(span.end).map_err(|_| "node span overflow")?;
    let length = end.checked_sub(start).ok_or("inverted node span")?;
    let kind_alias = if error {
        None
    } else {
        let (name, named) = *catalog[kind].get_or_insert_with(|| {
            (
                language.node_kind_for_id(node.kind_id()),
                language.node_kind_is_named(node.kind_id()),
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
        extra: node.is_extra(),
        error,
        missing: node.is_missing(),
        kind_alias,
        tokens_kept: false,
    });
    Ok(index)
}

/// Where a Blade template's syntax error (`BladeTree::error_ranges`) sits in its tree as an ERROR
/// leaf: under the deepest node whose span holds it with bytes on both sides (a zero-width error
/// strictly inside), after the children that end at or before its start. Siblings then still end
/// in byte order. The node's id and the child index the leaf goes before.
fn error_place(root: BladeNode<'_>, error: &Range<usize>) -> (usize, usize) {
    let mut node = root;
    loop {
        let inner = node.children().find(|child| {
            let range = child.range();
            range.start <= error.start
                && error.end <= range.end
                && range.start < error.end
                && error.start < range.end
        });
        match inner {
            Some(child) => node = child,
            None => break,
        }
    }
    let before = node
        .children()
        .take_while(|child| child.range().end <= error.start)
        .count();
    (node.node_id(), before)
}

/// Appends a Blade template's tree (`blade::forte::tree`) to `nodes` in preorder, its kinds and
/// fields the tree's, and its syntax errors `errors` (as Blade reads them, `View::read`) as ERROR
/// leaves (`error_place`).
fn encode_blade_tree(
    nodes: &mut Vec<Pending>,
    tree: &BladeTree,
    errors: &[Range<usize>],
    budget: &impl Fn() -> bool,
) -> Result<(), String> {
    enum Item<'t> {
        Node(BladeNode<'t>),
        Error(Range<usize>),
    }
    let mut errors = errors
        .iter()
        .map(|error| {
            let (node, before) = error_place(tree.root(), error);
            (node, before, error.clone())
        })
        .collect::<Vec<_>>();
    errors.sort_by_key(|(node, before, error)| (*node, *before, error.start, error.end));
    let mut stack = vec![(Item::Node(tree.root()), None)];
    let mut visited = 0usize;
    while let Some((item, parent)) = stack.pop() {
        if visited.is_multiple_of(256) {
            checked(budget)?;
        }
        visited += 1;
        let index = u32::try_from(nodes.len()).map_err(|_| "rec6 node count overflow")?;
        let node = match item {
            Item::Error(range) => {
                let start = u32::try_from(range.start).map_err(|_| "node span overflow")?;
                let end = u32::try_from(range.end).map_err(|_| "node span overflow")?;
                nodes.push(Pending {
                    kf: ERROR_CODE,
                    parent,
                    start,
                    length: end.checked_sub(start).ok_or("inverted node span")?,
                    size: 1,
                    extra: false,
                    error: true,
                    missing: false,
                    kind_alias: None,
                    tokens_kept: false,
                });
                continue;
            }
            Item::Node(node) => node,
        };
        let field = node.field_id();
        let code = node.kind_id();
        if code == ERROR_CODE
            || usize::from(code) >= KIND_LIMIT
            || usize::from(field) >= FIELD_LIMIT
        {
            return Err(format!(
                "Blade kind {code} or field {field} exceeds the tape's codes"
            ));
        }
        let range = node.range();
        let start = u32::try_from(range.start).map_err(|_| "node span overflow")?;
        let end = u32::try_from(range.end).map_err(|_| "node span overflow")?;
        nodes.push(Pending {
            kf: code | (field << 10),
            parent,
            start,
            length: end.checked_sub(start).ok_or("inverted node span")?,
            size: 1,
            extra: false,
            error: false,
            missing: false,
            kind_alias: None,
            tokens_kept: false,
        });
        let id = node.node_id();
        let here = errors.partition_point(|(node, ..)| *node < id)
            ..errors.partition_point(|(node, ..)| *node <= id);
        let mut placed = errors[here].iter().peekable();
        let mut items = Vec::with_capacity(node.children().len() + placed.len());
        for (position, child) in node.children().enumerate() {
            while let Some((_, _, error)) = placed.next_if(|(_, before, _)| *before <= position) {
                items.push(Item::Error(error.clone()));
            }
            items.push(Item::Node(child));
        }
        items.extend(placed.map(|(_, _, error)| Item::Error(error.clone())));
        stack.extend(items.into_iter().rev().map(|item| (item, Some(index))));
    }
    Ok(())
}

impl TapeImage {
    pub(super) fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let language =
            super::language::tape_tag(self.language).ok_or("unsupported tape language")?;
        let alias_bytes = self.kind_aliases.iter().map(|(name, _)| 5 + name.len()).sum::<usize>();
        let code_bytes = self.codes.iter().map(Code::stored_len).sum::<usize>();
        let lengths = [self.node_count(), self.codes.len(), code_bytes, self.kind_aliases.len(), alias_bytes];
        let mut bytes = Vec::with_capacity(
            HEADER_BYTES
                + [&self.code, &self.size, &self.base, &self.off, &self.tail].iter().map(|column| padded(column.len())).sum::<usize>()
                + code_bytes
                + alias_bytes,
        );
        bytes.extend_from_slice(TAPE_MAGIC);
        bytes.push(language);
        bytes.extend_from_slice(&self.widths);
        bytes.extend_from_slice(&TAPE_FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(self.source_len as u64).to_le_bytes());
        bytes.extend_from_slice(&self.source_digest);
        for length in lengths {
            bytes.extend_from_slice(&(length as u64).to_le_bytes());
        }
        debug_assert_eq!(bytes.len() % COLUMN_ALIGN, 0);
        for column in [&self.code, &self.size, &self.base, &self.off, &self.tail] {
            bytes.extend_from_slice(column);
            bytes.resize(padded(bytes.len()), 0);
        }
        for code in &self.codes {
            bytes.extend_from_slice(&code.kf.to_le_bytes());
            bytes.push(code.flags);
            bytes.extend_from_slice(&code.alias.to_le_bytes());
            bytes.push(code.class_count);
            for class in code.classes() {
                bytes.extend_from_slice(&class.to_le_bytes());
            }
        }
        for (name, named) in &self.kind_aliases {
            bytes.push(u8::from(*named));
            bytes.extend_from_slice(
                &u32::try_from(name.len())
                    .map_err(|_| "contextual grammar alias length overflow")?
                    .to_le_bytes(),
            );
            bytes.extend_from_slice(name.as_bytes());
        }
        Ok(bytes)
    }

    pub(super) fn from_bytes(bytes: &[u8], grammar: &Arc<Grammar>) -> Result<Self, String> {
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
        let source_digest: [u8; 32] = reader.take(32)?.try_into().expect("digest width");
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
        let mut columns = lengths.into_iter().map(|length| {
            let column = reader.take(length)?.to_vec();
            if reader.take(padded(length) - length)?.iter().any(|&byte| byte != 0) {
                return Err("noncanonical tape column padding".to_owned());
            }
            Ok(column)
        });
        let mut next = || columns.next().expect("five columns");
        let (code, size, base, off, tail) = (next()?, next()?, next()?, next()?, next()?);
        drop(columns);
        #[cfg(test)]
        note_table_reservation(code_count);
        let codes_end = reader.offset + code_bytes;
        let mut codes = Vec::with_capacity(code_count);
        for _ in 0..code_count {
            let kf = u16::from_le_bytes(reader.take(2)?.try_into().expect("u16 width"));
            let flags = reader.u8()?;
            let alias = u16::from_le_bytes(reader.take(2)?.try_into().expect("u16 width"));
            let class_count = reader.u8()?;
            if flags & !(CODE_EXTRA | CODE_ERROR | CODE_MISSING) != 0
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
        #[cfg(test)]
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
        Ok(Self { code, size, base, off, tail, widths, codes, kind_aliases, language, source_len, source_digest })
    }

    /// Parses `source` as `grammar`'s dialect and encodes its documents: with tree-sitter (a PHP
    /// file's view, `php_view`, against `source`), or a Blade template with Rerail's port of Forte
    /// (`encode_template`). `None` where the parser stopped before it read the file: tree-sitter
    /// gave up (`budget` interrupted it), or Forte reached a limit.
    pub(super) fn parse(
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Option<Result<FileImage, String>> {
        // A build without `lang-blade` routes no file to Blade (`GrammarDialect::built`), so Forte is not linked.
        if cfg!(feature = "lang-blade") && grammar.lang == GrammarDialect::Blade {
            return match Self::encode_template(grammar, source, budget) {
                Ok(image) => Some(Ok(image)),
                // Forte stopped at a limit: like tree-sitter giving up, it read nothing.
                Err(TemplateError::Parser(_)) => None,
                Err(TemplateError::Encode(error)) => Some(Err(error)),
            };
        }
        // A `.rr` file: the rule engine's own parser builds its tree (decision 0061).
        if grammar.lang == GrammarDialect::Rr {
            return Some(Self::encode_rr(grammar, source, budget).map(FileImage::single));
        }
        let mut parser = tree_sitter::Parser::new();
        if let Err(error) = parser.set_language(&grammar.lang.get_ts_language()) {
            return Some(Err(error.to_string()));
        }
        let parsed = super::language::adapter::parse_input(grammar.lang, source);
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
    /// build from the tree, each one per `via`. (Blade's compiled view is built from its Forte
    /// tree: `encode_blade`.)
    pub(super) fn encode(
        root: TsNode<'_>,
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Result<FileImage, String> {
        let mut image = FileImage::single(Self::encode_tree_image(
            root, grammar, source, None, budget,
        )?);
        let embeds = grammar.lang.embeds();
        for (index, &(via, _)) in embeds.iter().enumerate() {
            if embeds[..index].iter().any(|&(earlier, _)| earlier == via) {
                continue;
            }
            crate::code_rule_dsl::language::with_embedder!(via, E => {
                if let Some(program) = <E as Embedder>::build(root, source) {
                    image
                        .derived
                        .push(DerivedImage::encode(via, program, 0, grammar, source, budget)?);
                }
            });
        }
        Ok(image)
    }

    /// The tape of a tree-sitter tree of `source` alone, leaving out the placeholders `omitted`
    /// marks (an embedded program's).
    fn encode_tree_image(
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
        encode_tree(&mut nodes, &mut supertypes, root, language, omitted, budget)?;
        Self::finish(nodes, supertypes, grammar, source, budget)
    }

    /// The documents of a Blade template, parsed with Rerail's port of Forte (`encode_blade`).
    pub(super) fn encode_template(
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Result<FileImage, TemplateError> {
        checked(budget)?;
        let tree = parse_tree(source.as_bytes()).map_err(TemplateError::Parser)?;
        Self::encode_blade(&tree, grammar, source, budget)
    }

    /// The documents of a Blade template from its Forte tree (`tree::parse_tree`): the
    /// template's tree, its syntax errors as ERROR leaves, and the document its embedded PHP
    /// derives (`blade::view`, the `compiled-view` embedding).
    pub(super) fn encode_blade(
        tree: &BladeTree,
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Result<FileImage, TemplateError> {
        checked(budget)?;
        if grammar.lang != GrammarDialect::Blade {
            return Err("only a Blade template is encoded from a Forte tree"
                .to_owned()
                .into());
        }
        if source.len() > u32::MAX as usize {
            return Err("source exceeds tape range".to_owned().into());
        }
        // The template's syntax errors are ERROR leaves where Blade reads them (`View::read`).
        let read = View::read(tree, source, budget).map_err(|error| match error {
            ReadError::Parser(error) => TemplateError::Parser(error),
            ReadError::Interrupted => TemplateError::Encode("tape operation interrupted".into()),
        })?;
        let mut nodes = Vec::<Pending>::new();
        encode_blade_tree(&mut nodes, tree, &read.syntax_errors, budget)?;
        let mut image = FileImage::single(Self::finish(nodes, Vec::new(), grammar, source, budget)?);
        if let Some(view) = read.view {
            image.derived.push(DerivedImage::encode(
                Via::CompiledView,
                view.into_program(),
                0,
                grammar,
                source,
                budget,
            )?);
        }
        Ok(image)
    }

    /// The tape of a `.rr` file: the tree `logic::rr::syntax` builds as it reads the statements, its comments as
    /// extras and each statement that does not parse an ERROR leaf (decision 0061).
    pub(super) fn encode_rr(grammar: &Grammar, source: &str, budget: &impl Fn() -> bool) -> Result<Self, String> {
        checked(budget)?;
        if grammar.lang != GrammarDialect::Rr {
            return Err("only a .rr file is encoded from its rule tree".into());
        }
        if source.len() > u32::MAX as usize {
            return Err("source exceeds tape range".into());
        }
        let nodes = crate::code_rule_dsl::logic::rr::tape(source)
            .into_iter()
            .map(|node| {
                let error = node.is_error();
                if !error && (usize::from(node.kind) >= KIND_LIMIT || usize::from(node.field) >= FIELD_LIMIT) {
                    return Err(format!("rr kind {} or field {} exceeds the tape's codes", node.kind, node.field));
                }
                Ok(Pending {
                    kf: if error { ERROR_CODE } else { node.kind | (node.field << 10) },
                    parent: node.parent,
                    start: node.start,
                    length: node.end.checked_sub(node.start).ok_or("inverted node span")?,
                    size: 1,
                    extra: node.extra,
                    error,
                    missing: false,
                    kind_alias: None,
                    tokens_kept: false,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Self::finish(nodes, Vec::new(), grammar, source, budget)
    }

    /// The image of `nodes`, in preorder with their parents set.
    fn finish(
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
            let flags = u8::from(node.extra) * CODE_EXTRA | u8::from(node.error) * CODE_ERROR | u8::from(node.missing) * CODE_MISSING | u8::from(node.tokens_kept) * CODE_TOKENS;
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
        Ok(Self {
            code,
            size,
            base: bases.iter().flat_map(|base| base.to_le_bytes()).collect(),
            off,
            tail,
            widths: [size_width as u8, off_width as u8, tail_width as u8 | if from_last { 0 } else { TAIL_FROM_START }],
            codes,
            kind_aliases,
            language: grammar.lang,
            source_len: source.len(),
            source_digest: source_digest(source, budget)?,
        })
    }

    fn size_width(&self) -> usize {
        usize::from(self.widths[0])
    }

    fn off_width(&self) -> usize {
        usize::from(self.widths[1])
    }

    fn tail_width(&self) -> usize {
        usize::from(self.widths[2] & !TAIL_FROM_START)
    }

    fn code_at(&self, i: usize) -> u16 {
        u16::from_le_bytes([self.code[2 * i], self.code[2 * i + 1]])
    }

    fn size_at(&self, i: usize) -> u32 {
        cell(&self.size, i, self.size_width())
    }

    fn start_at(&self, i: usize) -> u32 {
        cell(&self.base, i / CHECKPOINT_STRIDE, 4).saturating_add(cell(&self.off, i, self.off_width()))
    }

    fn tail_at(&self, i: usize) -> u32 {
        cell(&self.tail, i, self.tail_width())
    }

    /// The preorders of nodes whose code carries `flag`.
    #[cfg(test)]
    fn flagged(&self, flag: u8) -> Vec<u32> {
        (0..self.node_count()).filter(|&i| self.codes[usize::from(self.code_at(i))].flags & flag != 0).map(|i| i as u32).collect()
    }

    #[cfg(test)]
    pub(super) fn error_nodes(&self) -> Vec<u32> {
        self.flagged(CODE_ERROR)
    }

    #[cfg(test)]
    pub(super) fn missing_nodes(&self) -> Vec<u32> {
        self.flagged(CODE_MISSING)
    }

    #[cfg(test)]
    pub(super) fn extra_nodes(&self) -> Vec<u32> {
        self.flagged(CODE_EXTRA)
    }

    /// The supertype memberships the tape holds.
    #[cfg(test)]
    pub(super) fn membership_count(&self) -> usize {
        (0..self.node_count()).map(|i| usize::from(self.codes[usize::from(self.code_at(i))].class_count)).sum()
    }

    /// The code table's stored bytes.
    #[cfg(test)]
    pub(super) fn code_table_bytes(&self) -> usize {
        self.codes.iter().map(Code::stored_len).sum()
    }

    /// The nodes that carry a contextual alias.
    #[cfg(test)]
    pub(super) fn aliased_nodes(&self) -> usize {
        (0..self.node_count()).filter(|&i| self.codes[usize::from(self.code_at(i))].alias != SENTINEL).count()
    }

    pub(super) fn node_count(&self) -> usize {
        self.code.len() / 2
    }

    /// Node `i`'s subtree size, itself included (the `size` column).
    pub(super) fn subtree_size(&self, i: usize) -> u32 {
        self.size_at(i)
    }

    pub(super) fn estimated_bytes(&self) -> usize {
        self.code.capacity()
            + self.size.capacity()
            + self.base.capacity()
            + self.off.capacity()
            + self.tail.capacity()
            + self.codes.capacity() * std::mem::size_of::<Code>()
            + self.kind_aliases.capacity() * std::mem::size_of::<(String, bool)>()
            + self.kind_aliases.iter().map(|(name, _)| name.capacity()).sum::<usize>()
            + std::mem::size_of::<Self>()
    }
}

#[derive(Clone, Copy)]
struct Rec {
    kind: u16,
    field: u16,
    start: u32,
    end: u32,
    size: u32,
    parent: u32,
    first: u32,
    next: u32,
    prev: u32,
    children: u32,
    extra: bool,
    error: bool,
    missing: bool,
    /// The first child under its field (P's `rec` flag bit 4); read by `index_image`.
    #[allow(dead_code)]
    first_field: bool,
}

#[derive(Clone)]
pub(super) struct TapeDoc {
    // P's: the source's digest, read by P's anchor corpus (`matcher::p_anchors`, test-only).
    #[cfg_attr(not(test), allow(dead_code))]
    source_digest: [u8; 32],
    source: Arc<String>,
    grammar: Arc<Grammar>,
    data: Arc<Vec<Rec>>,
    lines: Arc<LineTable>,
    points: Arc<Points>,
    kind_aliases: Arc<Vec<(String, bool)>>,
    kind_overrides: Arc<Vec<(u32, u16)>>,
    supertypes: Arc<Vec<(u32, u16)>>,
    /// Derived classes (`with_labels`): a bitset over the nodes per class from
    /// `DERIVED_CLASS_BASE`.
    labels: Arc<Vec<Vec<u64>>>,
    /// Text tests read from the term index (`with_texts`, `prefilter::terms`, `R[tid]`): per
    /// slot key (`Micro::slot_key`), a bit per node.
    texts: Arc<std::collections::BTreeMap<String, Arc<Vec<u64>>>>,
    /// A host's guests (`embed`); the host a guest reads links to them weakly, as they hold it.
    guests: Guests,
    /// A guest's link to the file it is embedded in.
    host: Option<Arc<HostLink>>,
    /// The image this document was restored from, and its stored bytes' SHA-256 once asked
    /// (`residency::slabs::admit_docs`, decision 0032): what the device reads in place.
    image: Arc<TapeImage>,
    digest: Arc<std::sync::OnceLock<[u8; 32]>>,
}

#[derive(Clone)]
enum Guests {
    Owned(Arc<Vec<Guest>>),
    Linked(Weak<Vec<Guest>>),
}

/// A program a file embeds, as a document of its own (`embed`).
#[derive(Clone)]
pub(crate) struct Guest {
    pub(crate) via: Via,
    pub(crate) doc: TapeDoc,
}

/// What a guest document reads of the file it is embedded in: the embedding, the segments that
/// map its text to the file's bytes, the file's bytes it cannot be read through, and the host
/// document (which links back to its guests weakly).
struct HostLink {
    via: Via,
    segments: Arc<SegmentMap>,
    host_errors: Arc<Vec<(u32, u32)>>,
    host: TapeDoc,
    /// Each node's extent in the program (`node_extents`), by preorder index.
    extents: Vec<(u32, u32)>,
}

/// The program bytes each node of a guest's records stands for: from the first of its leaves that
/// stands for file bytes to the last (`SegmentMap::stands_for`); a node none of whose leaves does,
/// from its first leaf to its last. A node's span may run past those leaves: a root's holds the
/// white space after its last statement, and a statement ending in a `;` its embedder wrote over
/// no bytes holds the white space before it, which stands for nothing of the node's.
fn node_extents(nodes: &[Rec], segments: &SegmentMap) -> Vec<(u32, u32)> {
    const NONE_YET: (u32, u32) = (u32::MAX, 0);
    let join = |into: &mut (u32, u32), (start, end): (u32, u32)| {
        *into = (into.0.min(start), into.1.max(end));
    };
    // Per node: the extent of its leaves standing for bytes, and of all its leaves.
    let mut standing = vec![NONE_YET; nodes.len()];
    let mut all = vec![NONE_YET; nodes.len()];
    for index in (0..nodes.len()).rev() {
        let rec = &nodes[index];
        if rec.children == 0 {
            all[index] = (rec.start, rec.end);
            if segments.stands_for(rec.start as usize..rec.end as usize) {
                standing[index] = (rec.start, rec.end);
            }
        }
        if rec.parent != NONE {
            let (leaves, standing_leaves) = (all[index], standing[index]);
            join(&mut all[rec.parent as usize], leaves);
            if standing_leaves != NONE_YET {
                join(&mut standing[rec.parent as usize], standing_leaves);
            }
        }
    }
    standing
        .into_iter()
        .zip(all)
        .map(|(standing, all)| if standing == NONE_YET { all } else { standing })
        .collect()
}

pub(super) type KindPostings = rustc_hash::FxHashMap<u16, Vec<u32>>;

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
pub(super) struct TapeSource {
    pub source: Arc<String>,
    pub grammar: Arc<Grammar>,
    pub image: Arc<FileImage>,
    pub lines: Arc<LineTable>,
}

impl TapeSource {
    pub(super) fn restore(&self, budget: &impl Fn() -> bool) -> Result<TapeDoc, String> {
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
    #[cfg(test)]
    pub(super) fn transducer_source_arc(&self) -> Arc<String> {
        Arc::clone(&self.source)
    }

    /// The documents `image` encodes over `source`: the file's own, with the guests derived from
    /// it.
    pub(super) fn restore_with_lines(
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
            extents: node_extents(&doc.data, &guest.source_map),
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
        checked(budget)?;
        if image.language != grammar.lang
            || image.source_len != source.len()
            || image.source_digest != source_digest(&source, budget)?
        {
            return Err("tape grammar or source revision mismatch".into());
        }
        let count = image.node_count();
        let widths_valid = [image.widths[0], image.widths[1], image.widths[2] & !TAIL_FROM_START].iter().all(|width| matches!(width, 2 | 4));
        if count == 0
            || count > u32::MAX as usize
            || !widths_valid
            || image.code.len() != count * 2
            || image.size.len() != count * image.size_width()
            || image.base.len() != count.div_ceil(CHECKPOINT_STRIDE) * 4
            || image.off.len() != count * image.off_width()
            || image.tail.len() != count * image.tail_width()
        {
            return Err("malformed v9np tape".into());
        }
        for code in &image.codes {
            let kind = code.kf & 1023;
            let field = code.kf >> 10;
            let error = code.flags & CODE_ERROR != 0;
            if code.flags & !(CODE_EXTRA | CODE_ERROR | CODE_MISSING | CODE_TOKENS) != 0
                || (error && (kind != ERROR_CODE || code.flags & CODE_MISSING != 0))
                || (!error && (usize::from(kind) >= grammar.kinds.kind_count() || !grammar.kinds.kind_is_visible(kind)))
                || usize::from(field) > grammar.kinds.field_count()
                || (field != 0 && grammar.kinds.field_name(field).is_none())
                || (code.alias != SENTINEL && image.kind_aliases.get(usize::from(code.alias)).is_none_or(|alias| alias.0.is_empty()))
            {
                return Err("unknown v9np grammar code".into());
            }
            if usize::from(code.class_count) > MAX_NODE_SUPERTYPES
                || code.classes().windows(2).any(|pair| pair[0] >= pair[1])
                || code.classes[usize::from(code.class_count).min(MAX_NODE_SUPERTYPES)..].iter().any(|&unused| unused != 0)
                || code.classes().iter().any(|&class| usize::from(class) >= grammar.kinds.kind_count() || !grammar.kinds.kind_is_supertype(class))
            {
                return Err("invalid abstract node class entry".into());
            }
        }
        let from_start = image.widths[2] & TAIL_FROM_START != 0;
        let mut nodes: Vec<Rec> = Vec::with_capacity(count);
        let mut kind_overrides = Vec::new();
        let mut supertypes = Vec::new();
        // Parents rebuilt from the sizes: the enclosing subtree on the stack.
        let mut parent_stack: Vec<(usize, u64)> = Vec::new();
        let mut unwound = 0usize;
        for index in 0..count {
            if index % 256 == 0 {
                checked(budget)?;
            }
            let code = *image.codes.get(usize::from(image.code_at(index))).ok_or("unknown v9np code")?;
            let field = code.kf >> 10;
            let error = code.flags & CODE_ERROR != 0;
            let missing = code.flags & CODE_MISSING != 0;
            let kind = if error { ERROR_SYMBOL } else { code.kf & 1023 };
            if code.alias != SENTINEL {
                kind_overrides.push((index as u32, code.alias));
            }
            supertypes.extend(code.classes().iter().map(|&class| (index as u32, class)));
            let size = image.size_at(index);
            if size == 0 || index.checked_add(size as usize).is_none_or(|end| end > count) {
                return Err("invalid subtree boundary".into());
            }
            let start = image.start_at(index);
            let from = if from_start { start } else { image.start_at(index + size as usize - 1) };
            let end = from.checked_add(image.tail_at(index)).ok_or("span end overflow")?;
            if end < start
                || (missing && (start != end || size != 1))
                || end as usize > source.len()
                || !source.is_char_boundary(start as usize)
                || !source.is_char_boundary(end as usize)
            {
                return Err("span outside source or UTF-8 boundary".into());
            }
            while parent_stack
                .last()
                .is_some_and(|&(parent, _)| index >= parent + nodes[parent].size as usize)
            {
                if unwound.is_multiple_of(256) {
                    checked(budget)?;
                }
                unwound += 1;
                parent_stack.pop();
            }
            let parent = if index == 0 {
                if field != 0 || size as usize != count {
                    return Err("invalid tape root".into());
                }
                NONE
            } else {
                let enclosing = parent_stack.last().ok_or("orphan tape node")?.0;
                if start < nodes[enclosing].start || end > nodes[enclosing].end {
                    return Err("invalid tape parent or span".into());
                }
                enclosing as u32
            };
            let first_field = field != 0
                && parent_stack.last_mut().is_some_and(|(_, fields)| {
                    let bit = 1_u64 << field;
                    let first = *fields & bit == 0;
                    *fields |= bit;
                    first
                });
            nodes.push(Rec {
                kind,
                field,
                start,
                end,
                size,
                parent,
                first: NONE,
                next: NONE,
                prev: NONE,
                children: 0,
                extra: code.flags & CODE_EXTRA != 0,
                error,
                missing,
                first_field,
            });
            parent_stack.push((index, 0));
        }
        let mut last = vec![NONE; count];
        for index in 1..count {
            if index % 256 == 0 {
                checked(budget)?;
            }
            let parent = nodes[index].parent as usize;
            if nodes[parent].first == NONE {
                nodes[parent].first = index as u32;
            }
            let prev = last[parent];
            // Siblings sit in byte order, so their ends never decrease (`walk_anchor` steps
            // from a node on that).
            if prev != NONE && nodes[prev as usize].end > nodes[index].end {
                return Err("tape siblings out of byte order".into());
            }
            nodes[index].prev = prev;
            if prev != NONE {
                nodes[prev as usize].next = index as u32;
            }
            last[parent] = index as u32;
            nodes[parent].children += 1;
        }
        checked(budget)?;
        Ok(Self {
            source_digest: image.source_digest,
            points: Arc::new(Points {
                source: Arc::clone(&source),
                rows: std::sync::OnceLock::new(),
            }),
            source,
            grammar,
            data: Arc::new(nodes),
            lines,
            kind_aliases: Arc::new(image.kind_aliases.clone()),
            kind_overrides: Arc::new(kind_overrides),
            supertypes: Arc::new(supertypes),
            labels: Arc::new(Vec::new()),
            texts: Arc::default(),
            guests: Guests::Owned(Arc::new(Vec::new())),
            host: None,
            image: Arc::new(image.clone()),
            digest: Arc::default(),
        })
    }

    /// Ranges of ERROR nodes and MISSING tokens that no ERROR node contains, as
    /// `SyntaxErrors::collect` finds them on the native tree.
    pub(super) fn syntax_error_ranges(&self) -> Vec<Range<usize>> {
        self.syntax_error_nodes().map(|(_, range)| range).collect()
    }

    /// The nodes `syntax_error_ranges` reads, each with its preorder index.
    pub(super) fn syntax_error_nodes(&self) -> impl Iterator<Item = (usize, Range<usize>)> + '_ {
        let mut skip_until = 0usize;
        self.data
            .iter()
            .enumerate()
            .filter_map(move |(index, rec)| {
                if index < skip_until || !(rec.error || rec.missing) {
                    return None;
                }
                skip_until = index + rec.size as usize;
                Some((index, rec.start as usize..rec.end as usize))
            })
    }

    /// A host's guests (`embed`), in registry order; for the host a guest reads, the same ones.
    /// `None` once the host that owns them is gone.
    pub(crate) fn guests(&self) -> Option<Arc<Vec<Guest>>> {
        match &self.guests {
            Guests::Owned(guests) => Some(Arc::clone(guests)),
            Guests::Linked(guests) => guests.upgrade(),
        }
    }

    /// Whether this is a guest document: a program a file embeds.
    pub(crate) fn is_guest(&self) -> bool {
        self.host.is_some()
    }

    /// Whether `other` is this document restored once (a clone, a host a guest reads as this
    /// host): the same records.
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }

    /// The embedding that built this guest; `None` for a host.
    pub(crate) fn via(&self) -> Option<Via> {
        self.host.as_ref().map(|link| link.via)
    }

    /// The host document this guest is embedded in.
    pub(crate) fn host(&self) -> Option<&Self> {
        self.host.as_ref().map(|link| &link.host)
    }

    /// This document's own text, which its spans index.
    pub(super) fn document_source(&self) -> &Arc<String> {
        &self.source
    }

    /// The file's source: a guest's host's, else this document's own.
    pub(super) fn file_source(&self) -> &Arc<String> {
        self.host
            .as_ref()
            .map_or(&self.source, |link| &link.host.source)
    }

    /// The file's line table: a guest's host's, else this document's own.
    pub(super) fn file_lines(&self) -> &Arc<LineTable> {
        self.host
            .as_ref()
            .map_or(&self.lines, |link| &link.host.lines)
    }

    /// The file bytes a guest cannot be read through (`embed::Program::host_errors`); none for a
    /// host.
    pub(crate) fn host_errors(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.host
            .iter()
            .flat_map(|link| link.host_errors.iter())
            .map(|&(start, end)| start as usize..end as usize)
    }

    /// The dialect of the file this document is of: a guest's host's, else its own.
    pub(super) fn file_dialect(&self) -> GrammarDialect {
        self.host
            .as_ref()
            .map_or(self.grammar.lang, |link| link.host.grammar.lang)
    }

    /// The documents `image` encodes over `source`, with a fresh line table: a file's documents,
    /// or a lone tape (a file that derives none).
    #[cfg(test)]
    pub(super) fn restore(
        source: Arc<String>,
        grammar: Arc<Grammar>,
        image: &impl Documents,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let lines = Arc::new(LineTable::new(&source, Terminators::of(grammar.lang)));
        Self::restore_with_lines(source, grammar, &image.file_image(), lines, budget)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn index_source_digest(&self) -> &[u8; 32] {
        &self.source_digest
    }

    /// The source's line table, shared with the evaluator that restored this document.
    #[cfg(test)]
    pub(super) fn line_table(&self) -> &LineTable {
        &self.lines
    }

    pub(super) fn transducer_len(&self) -> usize {
        self.data.len()
    }

    /// The SHA-256 of this document's image as stored (the store's key for it), computed once.
    pub(super) fn tape_digest(&self) -> Result<[u8; 32], String> {
        if let Some(digest) = self.digest.get() {
            return Ok(*digest);
        }
        let digest = <[u8; 32]>::from(Sha256::digest(self.image.to_bytes()?));
        Ok(*self.digest.get_or_init(|| digest))
    }

    /// This document's tape digest, known where it was restored from stored bytes (the SHA-256 of
    /// those bytes, the residency's key): admission then reads it from the store, not from here.
    pub(crate) fn stored_as(self, digest: [u8; 32]) -> Self {
        let _ = self.digest.set(digest);
        self
    }

    /// The SHA-256 of this document's stored bytes, the residency's key (decision 0032).
    pub(crate) fn resident_key(&self) -> Result<[u8; 32], String> {
        self.tape_digest()
    }

    /// This document's image as stored, its source and its grammar: what residency admits.
    pub(super) fn resident_parts(&self) -> Result<(Vec<u8>, &str, &Arc<Grammar>), String> {
        Ok((self.image.to_bytes()?, self.source.as_str(), &self.grammar))
    }

    pub(super) fn transducer_kind(&self, node: u32) -> u16 {
        self.data[node as usize].kind
    }

    fn kind_alias(&self, node: u32) -> Option<&(String, bool)> {
        self.kind_overrides
            .binary_search_by_key(&node, |&(index, _)| index)
            .ok()
            .map(|slot| &self.kind_aliases[self.kind_overrides[slot].1 as usize])
    }

    fn node_is_named(&self, node: u32) -> bool {
        self.kind_alias(node).map_or_else(
            || self.grammar.kind_is_named(self.data[node as usize].kind),
            |alias| alias.1,
        )
    }

    pub(super) fn transducer_field(&self, node: u32) -> u16 {
        self.data[node as usize].field
    }

    pub(super) fn transducer_flags(&self, node: u32) -> u8 {
        let rec = &self.data[node as usize];
        u8::from(self.node_is_named(node))
            | (u8::from(rec.extra) << 1)
            | (u8::from(rec.missing) << 2)
            | (u8::from(rec.error) << 3)
    }

    pub(super) fn transducer_span(&self, node: u32) -> Range<usize> {
        let rec = &self.data[node as usize];
        rec.start as usize..rec.end as usize
    }

    /// Whether the node is named (its alias's, else its kind's).
    pub(super) fn transducer_is_named(&self, node: u32) -> bool {
        self.node_is_named(node)
    }

    pub(super) fn transducer_parent(&self, node: u32) -> Option<u32> {
        let parent = self.data[node as usize].parent;
        (parent != NONE).then_some(parent)
    }

    pub(super) fn transducer_children(&self, node: u32) -> impl Iterator<Item = u32> + '_ {
        let mut next = self.data[node as usize].first;
        std::iter::from_fn(move || {
            if next == NONE {
                return None;
            }
            let current = next;
            next = self.data[current as usize].next;
            Some(current)
        })
    }

    pub(super) fn transducer_descendants(&self, node: u32) -> Range<u32> {
        let rec = &self.data[node as usize];
        node + 1..node + rec.size
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn transducer_next_sibling(&self, node: u32, named: bool) -> Option<u32> {
        let mut next = self.data[node as usize].next;
        while next != NONE {
            if !named || self.node_is_named(next) {
                return Some(next);
            }
            next = self.data[next as usize].next;
        }
        None
    }

    /// The whole source, for the Metal backend's concatenated buffer.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn transducer_source(&self) -> &[u8] {
        self.source.as_bytes()
    }

    /// A node's source slice. Restoring the tape validates every span as a UTF-8 boundary.
    pub(super) fn transducer_text_str(&self, node: u32) -> &str {
        &self.source[self.transducer_span(node)]
    }

    /// A node's source bytes, for the packed CPU and Metal backends.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn transducer_text(&self, node: u32) -> &[u8] {
        &self.source.as_bytes()[self.transducer_span(node)]
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn transducer_is_named_leaf(&self, node: u32) -> bool {
        self.node_is_named(node) && self.data[node as usize].first == NONE
    }

    /// A collision-free encoding of the recursive kind and child structure used by ast-grep's
    /// repeated-capture equality when neither side is a named leaf.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn transducer_structural_key(&self, node: u32) -> String {
        fn encode(tape: &TapeDoc, node: u32, bytes: &mut Vec<u8>) {
            bytes.extend_from_slice(&tape.transducer_kind(node).to_le_bytes());
            let children = tape.transducer_children(node).collect::<Vec<_>>();
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
    pub(super) fn transducer_supertypes(&self) -> impl Iterator<Item = (u32, u16)> + '_ {
        self.supertypes.iter().copied()
    }

    /// The derived classes (`with_labels`): per class from `DERIVED_CLASS_BASE`, a bitset over the
    /// nodes. The packed batch carries them as bitsets over its nodes, which every backend tests
    /// directly (`PackedBatch::derived`).
    pub(super) fn transducer_labels(&self) -> &[Vec<u64>] {
        &self.labels
    }

    /// The labels, shared (`transducer_labels`).
    pub(super) fn transducer_labels_shared(&self) -> Arc<Vec<Vec<u64>>> {
        Arc::clone(&self.labels)
    }

    // The packed runtime's class test (Phase 4).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn transducer_has_supertype(&self, node: u32, kind: u16) -> bool {
        if kind >= DERIVED_CLASS_BASE {
            return self
                .labels
                .get(usize::from(kind - DERIVED_CLASS_BASE))
                .and_then(|bits| bits.get(node as usize / 64))
                .is_some_and(|word| word >> (node % 64) & 1 != 0);
        }
        self.supertypes.binary_search(&(node, kind)).is_ok()
    }

    /// This tape with derived node classes: a lowered matcher's label passes (06-first-slice S1),
    /// one bitset over the nodes per class from `DERIVED_CLASS_BASE`, which `SUPERTYPE` tests as it
    /// tests a grammar class.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn with_labels(&self, labels: Arc<Vec<Vec<u64>>>) -> Self {
        Self {
            labels,
            ..self.clone()
        }
    }

    /// This tape with its text tests' rows from the term index (`prefilter::terms`).
    pub(super) fn with_texts(&self, texts: std::collections::BTreeMap<String, Arc<Vec<u64>>>) -> Self {
        Self {
            texts: Arc::new(texts),
            ..self.clone()
        }
    }

    /// A text test's row from the term index, by its slot key (`with_texts`).
    pub(super) fn transducer_text_row(&self, key: &str) -> Option<&Arc<Vec<u64>>> {
        self.texts.get(key)
    }

    /// The node at a preorder ordinal: `root.adopt(doc.node(ordinal))` is the node ast-grep
    /// matches on.
    pub(super) fn node(&self, ordinal: u32) -> TapeNode<'_> {
        self.root_node().at(ordinal)
    }

    #[cfg(feature = "code-rule-profile")]
    pub(super) fn navigation_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.data.capacity() * std::mem::size_of::<Rec>()
            + self.kind_aliases.capacity() * std::mem::size_of::<(String, bool)>()
            + self
                .kind_aliases
                .iter()
                .map(|(name, _)| name.capacity())
                .sum::<usize>()
            + (self.kind_overrides.capacity() + self.supertypes.capacity())
                * std::mem::size_of::<(u32, u16)>()
            + self.image.estimated_bytes()
    }
}

#[derive(Clone, Copy)]
pub(super) struct TapeNode<'a> {
    nodes: &'a [Rec],
    grammar: &'a Grammar,
    points: &'a Points,
    kind_aliases: &'a [(String, bool)],
    kind_overrides: &'a [(u32, u16)],
    index: u32,
}

impl<'a> TapeNode<'a> {
    /// P's: the field this node fills in its parent (P's SQL anchor engine, test-only).
    #[cfg(test)]
    pub(super) fn incoming_field(&self) -> u16 {
        self.rec().field
    }

    /// Whether this is the first direct child in `field`, as `child_by_field_id` selects it.
    pub(super) fn is_first_field(&self, field: u16) -> bool {
        field != 0 && self.rec().field == field && self.rec().first_field
    }

    fn rec(&self) -> &Rec {
        &self.nodes[self.index as usize]
    }
    fn at(&self, index: u32) -> Self {
        Self { index, ..*self }
    }
    fn kind_alias(&self) -> Option<&'a (String, bool)> {
        self.kind_overrides
            .binary_search_by_key(&self.index, |&(index, _)| index)
            .ok()
            .map(|slot| &self.kind_aliases[self.kind_overrides[slot].1 as usize])
    }
    /// The structurally adjacent sibling, for traversal; `SgNode::next` follows Tree-sitter.
    pub(super) fn next_sibling(&self) -> Option<Self> {
        (self.rec().next != NONE).then(|| self.at(self.rec().next))
    }
    fn prev_sibling(&self) -> Option<Self> {
        (self.rec().prev != NONE).then(|| self.at(self.rec().prev))
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
        let start = self.rec().start;
        let ends_after = |node: &Self| node.rec().end > start;
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

struct Children<'a> {
    node: TapeNode<'a>,
    next: u32,
    remaining: usize,
}
impl<'a> Iterator for Children<'a> {
    type Item = TapeNode<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next == NONE {
            return None;
        }
        let index = self.next;
        self.next = self.node.nodes[index as usize].next;
        self.remaining -= 1;
        Some(self.node.at(index))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl ExactSizeIterator for Children<'_> {}

impl<'a> SgNode<'a> for TapeNode<'a> {
    fn parent(&self) -> Option<Self> {
        (self.rec().parent != NONE).then(|| self.at(self.rec().parent))
    }
    fn ancestors(&self, _root: Self) -> impl Iterator<Item = Self> {
        let mut ancestor = self.parent();
        std::iter::from_fn(move || {
            let next = ancestor?;
            ancestor = next.parent();
            Some(next)
        })
    }
    fn children(&self) -> impl ExactSizeIterator<Item = Self> {
        Children {
            node: *self,
            next: self.rec().first,
            remaining: self.rec().children as usize,
        }
    }
    fn dfs(&self) -> impl Iterator<Item = Self> {
        let start = self.index;
        let end = start + self.rec().size;
        (start..end).map(move |index| self.at(index))
    }
    fn child(&self, nth: usize) -> Option<Self> {
        self.children().nth(nth)
    }
    // Tree-sitter's next sibling is byte-positioned: it skips later siblings that end at or
    // before this node's end, i.e. zero-width siblings sitting at that end.
    fn next(&self) -> Option<Self> {
        let end = self.rec().end;
        let mut next = self.next_sibling();
        while let Some(node) = next {
            if node.rec().end > end {
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
    fn prev_all(&self) -> impl Iterator<Item = Self> {
        let mut prev = self.walk_anchor().and_then(|anchor| anchor.prev_sibling());
        std::iter::from_fn(move || {
            let node = prev?;
            prev = node.prev_sibling();
            Some(node)
        })
    }
    fn kind(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.kind_alias().map_or_else(
            || {
                self.grammar
                    .kind_name(self.rec().kind)
                    .expect("validated grammar symbol")
            },
            |alias| alias.0.as_str(),
        ))
    }
    fn kind_id(&self) -> u16 {
        self.rec().kind
    }
    fn node_id(&self) -> usize {
        self.index as usize
    }
    fn range(&self) -> Range<usize> {
        self.rec().start as usize..self.rec().end as usize
    }
    fn start_pos(&self) -> Position {
        self.points.point(self.rec().start)
    }
    fn end_pos(&self) -> Position {
        self.points.point(self.rec().end)
    }
    fn is_named(&self) -> bool {
        self.kind_alias().map_or_else(
            || self.grammar.kind_is_named(self.rec().kind),
            |alias| alias.1,
        )
    }
    fn is_named_leaf(&self) -> bool {
        self.children().all(|child| !child.is_named())
    }
    fn is_extra(&self) -> bool {
        self.rec().extra
    }
    fn is_error(&self) -> bool {
        self.rec().error
    }
    fn is_missing(&self) -> bool {
        self.rec().missing
    }
    fn field(&self, name: &str) -> Option<Self> {
        let field = self.grammar.kinds.field_id(name)?;
        self.child_by_field_id(field)
    }
    fn field_children(&self, field_id: Option<u16>) -> impl Iterator<Item = Self> {
        self.children().filter(move |child| {
            field_id.is_some_and(|field| field != 0 && child.rec().field == field)
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
                child.rec().field == field_id
            });
            #[cfg(feature = "code-rule-profile")]
            super::profile::navigation::record(examined, found.is_some());
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
        TapeNode {
            nodes: &self.data,
            grammar: &self.grammar,
            points: &self.points,
            kind_aliases: &self.kind_aliases,
            kind_overrides: &self.kind_overrides,
            index: 0,
        }
    }
    fn get_node_text<'a>(&'a self, node: &Self::Node<'a>) -> Cow<'a, str> {
        Cow::Borrowed(&self.source[node.range()])
    }
}

/// A host's bytes are the file's; a guest's program text stands for the file ranges its segments
/// map it to (`SegmentMap::file_ranges`), and a guest node for those of its extent, from its first
/// leaf standing for file bytes to its last (`node_extents`), white space between its ranges
/// included, as a tree's node holds the white space between its tokens (`FileRanges::bridged`).
impl crate::code_rule_dsl::origin::Origin for TapeDoc {
    fn of_node(&self, id: usize, range: Range<usize>) -> Option<FileRanges> {
        Some(match &self.host {
            None => FileRanges::one(range),
            Some(link) => {
                let (start, end) = *link.extents.get(id)?;
                link.segments
                    .file_ranges(start as usize..end as usize)
                    .bridged(&link.host.source)
            }
        })
    }

    fn of_range(&self, range: Range<usize>) -> Option<FileRanges> {
        Some(match &self.host {
            None => FileRanges::one(range),
            Some(link) => link.segments.file_ranges(range).bridged(&link.host.source),
        })
    }

    fn bounds(&self, id: usize, range: Range<usize>) -> Option<Range<usize>> {
        Some(match &self.host {
            None => range,
            Some(link) => {
                let (start, end) = *link.extents.get(id)?;
                link.segments.bounds(start as usize..end as usize)
            }
        })
    }

    fn copied(&self, range: Range<usize>) -> Option<Range<usize>> {
        match &self.host {
            None => Some(range),
            Some(link) => link.segments.copied_span(range),
        }
    }

    fn copies(&self, range: Range<usize>) -> bool {
        self.host
            .as_ref()
            .is_none_or(|link| link.segments.copies(range))
    }
}

/// The tape document a node of any document is of, if a tape's.
pub(crate) fn tape_doc<'t, D: Doc>(node: &ast_grep_core::Node<'t, D>) -> Option<&'t TapeDoc> {
    let doc: &'t dyn std::any::Any = node.get_doc();
    doc.downcast_ref::<TapeDoc>()
}

// P's tape spike (v7, v8, v9np), for O8's pre-lock measurements.
#[allow(dead_code, unused_imports)]
pub(crate) mod v9np;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod corpus_parity;
