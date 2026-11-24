//! Decision 0293: a document's tape through the one API (`rerail_tape_format::tape`), in the format the run stores
//! (`RERAIL_TAPE`), read in place:
//!
//! - **16** (the default since 2026-11-17, 0156): format 16, written by every parser as it parses (the lean parsers'
//!   writer, `lean`; a tree-sitter walk's ops, `TreeOps`; simdjson's events); the store keeps it, and a load reads it
//!   as stored;
//! - **16z**: format 16 in fixed LZ4 blocks, a block decoded where a read falls in it.
//!
//! Every language has its table, made from its grammar (decision 0631). A node whose code the table lacks, or past
//! format 16's bounds, is an error: there is no other format to keep (format 15 is removed, 0631).

use rerail_blocks::vector::Vector;
use super::{Code, FileImage, Grammar, Held, TapeDoc, TapeImage, Text};
use crate::code_rule_dsl::language::GrammarDialect;
use rerail_tape_format::{Redo, Writer};
use rerail_tape_format::f16::{self, Lang};
use rerail_tape_format::tape::{AnyTape, Entry};
use std::sync::Arc;

/// The format a run stores and reads its documents' tapes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    F16,
    F16Z,
}

/// Format 16's and 16z's stored layout: 3 since decision 0336 §6 (a newline index after the side lists, its count
/// in the header; 16z holds it raw after the checkpoints). Layout 2 (0336: region starts in the header; 16z's blocks
/// one piece an array, raw, LZ4 or a repeat) had keys 216 and 1216; layout 1, 16 and 1016. Layout 5 (decision 0434
/// §4, step 6): the tape's presence stored after it, in its home. Layout 6: a MISSING node sets the presence's
/// fault bit (`ERROR_CODE`'s) beside its kind's, so a presence says which blocks hold a syntax error. Layout 8
/// (decision 0656 step 3): the tape's auxiliary index after its presence, in its home (`lay_aux`). Layout 9: its
/// SIBLINGS of named children (version 2) and its NAMED section. Layout 10: its COUNTS' strays (version 2).
const F16_LAYOUT: u32 = 10;

impl Format {
    pub const ALL: [Format; 2] = [Format::F16, Format::F16Z];

    /// `RERAIL_TAPE`: `16` (the default since the user took 0156, 2026-11-17) or `16z`. Another value stops the run,
    /// so a measurement never reads a format it did not ask for.
    pub fn of_run() -> Format {
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(format) = AS_RUN.with(std::cell::Cell::get) {
            return format;
        }
        rerail_prof::code_rule_dsl::logic::profile::setting(|| match std::env::var("RERAIL_TAPE").as_deref() {
            Err(_) | Ok("" | "16") => Format::F16,
            Ok("16z") => Format::F16Z,
            Ok(other) => panic!("RERAIL_TAPE={other}: the formats are 16 and 16z (format 15 is removed, decision 0631)"),
        })
    }

    /// The tape format version the store keys a tape by when a run stores this format: a store holds each
    /// document once per format, and a run finds only its own. Each carries its layout (`F16_LAYOUT`): a store
    /// built before a layout changed reads cold, not wrong.
    pub fn store_version(self) -> u32 {
        match self {
            Format::F16 => 16 + 100 * F16_LAYOUT,
            Format::F16Z => 1016 + 100 * F16_LAYOUT,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::F16 => "16",
            Format::F16Z => "16z",
        }
    }
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// The format this thread's run stores, where a test sets it (`as_run`).
    static AS_RUN: std::cell::Cell<Option<Format>> = const { std::cell::Cell::new(None) };
}

/// `f` as a run that stores `format`, on this thread: a test's, so that one process writes every format.
#[cfg(any(test, feature = "test-hooks"))]
pub fn as_run<R>(format: Format, f: impl FnOnce() -> R) -> R {
    let before = AS_RUN.with(|r| r.replace(Some(format)));
    let out = f();
    AS_RUN.with(|r| r.set(before));
    out
}

/// This thread runs as a run that stores `format` until it ends: a test of one format's image (16z's, say) whatever
/// the process's run stores. Each test has a thread of its own.
#[cfg(any(test, feature = "test-hooks"))]
pub fn this_thread_stores(format: Format) {
    AS_RUN.with(|r| r.set(Some(format)));
}

/// The tape format version the store keys this run's tapes by (`Format::store_version`).
pub fn stored_version() -> u32 {
    Format::of_run().store_version()
}

impl Code {
    /// The code as the API's entry, with no alias (a parser's code; an image's aliases are its own, `code_table`).
    pub fn entry(&self) -> Entry {
        let kind_mask = (1u16 << rerail_tape_format::KIND_BITS) - 1;
        Entry {
            kind: self.kf & kind_mask,
            field: self.kf >> rerail_tape_format::KIND_BITS,
            flags: self.flags,
            alias: None,
            // A count past the slots (a damaged image) reads as that many classes, which a restore refuses.
            classes: self.classes.get(..usize::from(self.class_count)).map_or_else(|| vec![0; usize::from(self.class_count)], <[u16]>::to_vec),
        }
    }
}

/// The run's format 16 target for `language`, its lines ending at `terminators` (its ecosystem's, decision 0612), each
/// of the parser's keys numbered in the language's table through its code (`number`).
fn sixteen(language: GrammarDialect, terminators: crate::code_rule_dsl::lines::Terminators, number: impl Fn(&'static Lang, u32) -> Option<u32> + Send + Sync + 'static) -> Result<rerail_tape_format::Sixteen, String> {
    let (lang, tag, z) = run_16(language)?;
    Ok(rerail_tape_format::Sixteen { lang, terminators, tag, z, blocks: true, number: Arc::new(move |key| number(lang, key)) })
}

/// The language's format 16 table, its tag and whether the run stores 16z; `Err` for a language with no tape tag or
/// no table (every tape tag has one: decision 0631).
pub fn run_16(language: GrammarDialect) -> Result<(&'static Lang, u8, bool), String> {
    let tag = crate::code_rule_dsl::language::tape_tag(language).ok_or_else(|| format!("{language:?} has no tape tag"))?;
    let lang = f16::lang(tag).ok_or_else(|| format!("{language:?} (tape tag {tag}) has no format 16 table"))?;
    Ok((lang, tag, Format::of_run() == Format::F16Z))
}

/// The cursor walk's format 16 output (`tape::TreeOut`; decision 0434 §2): each node given to format 16's ops as the
/// walk opens and closes it, its number the language's for its code, looked up once a code a document. A code the
/// table lacks, or a node past format 16's bounds, makes `finish` an error.
pub(super) struct TreeOps {
    lang: &'static Lang,
    w: f16::Writer,
    /// The open nodes' ends.
    ends: Vec<u32>,
    numbers: rustc_hash::FxHashMap<Code, u32>,
    /// The document's contextual aliases, numbered by first use (a code's `alias`).
    aliases: Vec<(String, bool)>,
    bad: Option<String>,
}

impl TreeOps {
    /// The run's format 16 output for a tree of `language` over `source`, of about `nodes` nodes (`run_16`).
    pub(super) fn of_run(language: GrammarDialect, source: &[u8], nodes: usize) -> Result<Self, String> {
        Self::of_run_as(language, source, nodes, true)
    }

    /// `of_run`, its blocks packed (16z) only where `packed` and the run stores 16z: a tape read where it is written
    /// and never stored (a rule's pattern) is not packed to be unpacked at its next read.
    pub(super) fn of_run_as(language: GrammarDialect, source: &[u8], nodes: usize, packed: bool) -> Result<Self, String> {
        let (lang, tag, z) = run_16(language)?;
        let z = z && packed;
        let w = f16::Writer::new(lang, source, crate::code_rule_dsl::lines::terminators(language), tag, z, true, nodes);
        Ok(TreeOps { lang, w, ends: Vec::with_capacity(64), numbers: Default::default(), aliases: Vec::new(), bad: None })
    }

    /// `v`'s number in the language's table, a member of `classes`: `Err` names a code the table lacks.
    fn number(&mut self, v: &super::Visit, classes: &[u16]) -> Result<u32, String> {
        let alias = match &v.kind_alias {
            None => super::SENTINEL,
            Some(a) => match self.aliases.iter().position(|x| x == a) {
                Some(k) => k as u16,
                None => {
                    self.aliases.push(a.clone());
                    (self.aliases.len() - 1) as u16
                }
            },
        };
        // A code's classes ascending, each once, as a table's entries hold them.
        let mut sorted = [0u16; super::MAX_NODE_SUPERTYPES];
        let mut count = 0;
        for &c in classes {
            if !sorted[..count].contains(&c) {
                *sorted.get_mut(count).ok_or("too many supertype memberships")? = c;
                count += 1;
            }
        }
        sorted[..count].sort_unstable();
        let flags = (u8::from(v.extra) * super::CODE_EXTRA) | (u8::from(v.error) * super::CODE_ERROR) | (u8::from(v.missing) * super::CODE_MISSING);
        let code = Code { kf: v.kf, flags, alias, class_count: count as u8, classes: sorted };
        if let Some(&n) = self.numbers.get(&code) {
            return Ok(n);
        }
        let mut entry = code.entry();
        entry.alias = v.kind_alias.clone();
        let n = self.lang.number(&entry).ok_or_else(|| format!("code {entry:?} is not in the language's table"))?;
        self.numbers.insert(code, n);
        Ok(n)
    }

    /// The tape, or why the nodes are not a format 16 tape.
    pub(super) fn finish(mut self) -> Result<f16::Finished, String> {
        match self.bad {
            Some(why) => Err(why),
            None => self.w.finish(),
        }
    }
}

impl super::TreeOut for TreeOps {
    fn open(&mut self, v: super::Visit, _parent: Option<u32>, classes: &[u16]) -> Result<u32, String> {
        let index = self.w.len() as u32;
        let number = if self.bad.is_some() {
            0
        } else {
            self.number(&v, classes).unwrap_or_else(|why| {
                self.bad = Some(why);
                0
            })
        };
        self.w.open(number, u64::from(v.start));
        // A walk never edits a node it wrote: each block is final once its nodes close, and 16z's pieces are
        // compressed then, not at the end.
        self.w.commit();
        self.ends.push(v.end);
        Ok(index)
    }

    fn close(&mut self) {
        if let Some(end) = self.ends.pop() {
            self.w.close(u64::from(end));
        }
    }
}

/// The sink a lean parser's writer is given (`rerail_*_lean::tape_into`; decisions 0293, 0434): the tape in the run's
/// format, 16 or 16z. The writer passes its nodes to format 16's ops as the parse commits them, each key numbered in
/// the language's table by the names its crate gives it (`lean_named`). A key the table lacks, or nodes past format
/// 16's bounds, are an error.
pub struct Lean<'a> {
    grammar: &'a Grammar,
    source: &'a str,
    budget: &'a dyn Fn() -> bool,
    target: Result<rerail_tape_format::Sixteen, String>,
    /// The program ranges its tape leaves out (`omitting`).
    omit: Vector<[u32; 2]>,
}

impl Lean<'_> {
    /// The sink of a program's tape that leaves out its placeholders `omit` (`SegmentMap::omissions`), as the parse
    /// writes it (`Writer::omitting`).
    pub fn omitting(mut self, omit: Vector<[u32; 2]>) -> Self {
        self.omit = omit;
        self
    }
}

/// The sink for a lean parser whose crate names its keys (`names`, decision 0631): each numbered in the language's
/// table by the grammar's names the table carries (`Lang::number_named`), as the crate's own tapes are (its `sink`).
pub fn lean_named<'a>(grammar: &'a Grammar, source: &'a str, budget: &'a dyn Fn() -> bool, names: fn(u32) -> Option<rerail_tape_format::KeyNames>) -> Lean<'a> {
    let target = sixteen(grammar.lang, crate::code_rule_dsl::lines::terminators(grammar.lang), move |lang, key| lang.number_named(&names(key)?));
    Lean { grammar, source, budget, target, omit: Vector::new() }
}

impl rerail_tape_format::Sink<Result<TapeImage, String>> for Lean<'_> {
    fn target(&self) -> rerail_tape_format::Target {
        self.target.clone()
    }

    fn omissions(&self) -> Vector<[u32; 2]> {
        self.omit.clone()
    }

    fn finish(&mut self, mut w: Writer) -> Result<Result<TapeImage, String>, Redo> {
        Ok(w.finish_16()?.and_then(|f| TapeImage::written_16(f, self.grammar.lang, self.source, None, self.budget)))
    }
}

impl TapeImage {
    /// A parser's tape written in format 16 or 16z, with its presence and its source's digest: `digest`, where the
    /// parse computed it (simdjson's kernels), else computed here.
    pub fn written_16(written: f16::Finished, language: GrammarDialect, source: &str, digest: Option<[u8; 32]>, budget: &dyn Fn() -> bool) -> Result<Self, String> {
        let digest = match digest {
            Some(d) => d,
            None => super::source_digest(source, &budget)?,
        };
        let mut written = written;
        lay_aux(&mut written.tape, language, source, &digest)?;
        let held = match written.tape {
            f16::Written::F16(t) => Held::F16(Arc::new(t)),
            f16::Written::F16Z(t) => Held::F16Z(Arc::new(t)),
        };
        Ok(TapeImage { held, language, source_len: source.len(), source_digest: Some(digest) })
    }

    /// The language's table.
    pub fn lang(&self) -> Option<&'static Lang> {
        match &self.held {
            Held::F16(t) => f16::lang(t.tag()),
            Held::F16Z(t) => f16::lang(t.tag()),
        }
    }
}

/// Lays `tape`'s auxiliary index in its home after its presence (decision 0656 step 3): every section, built once
/// here, where the tape is written, and stored, placed and loaded with it, so every reader (the host's and the
/// device's) reads it where the tape lies. Keyed by the tape's content: its header holds the source digest's first
/// eight bytes, and the store keys the home by its layout (`F16_LAYOUT`). A tape without a presence (one no
/// producer of the engine writes) is left without one.
fn lay_aux(tape: &mut f16::Written, language: GrammarDialect, source: &str, digest: &[u8; 32]) -> Result<(), String> {
    if tape.presence_bytes().is_none() {
        return Ok(());
    }
    let key = u64::from_le_bytes(digest[..8].try_into().expect("8 bytes"));
    let lang = match &*tape {
        f16::Written::F16(t) => f16::lang(t.tag()),
        f16::Written::F16Z(t) => f16::lang(t.tag()),
    }
    .ok_or("a format 16 tape of a language without its table")?;
    let named = named_codes(lang.table(), language);
    let aux = match &*tape {
        f16::Written::F16(t) => rerail_tape_format::aux_index::build(&t.reader(lang)?, source.as_bytes(), key, &named, rerail_tape_format::aux_index::Sections::ALL)?,
        f16::Written::F16Z(t) => {
            let cache = f16::ZCache::of([t]);
            rerail_tape_format::aux_index::build(&t.reader(lang, Some(&cache))?, source.as_bytes(), key, &named, rerail_tape_format::aux_index::Sections::ALL)?
        }
    };
    tape.lay_aux(&aux)
}

/// The named codes of a tape's table, a bit a code (`aux_index::named_codes`): as a node's flags say it is named
/// (`Facts::flags`), its alias's word where it has one, else its grammar's for its kind (ERROR's for an error code).
/// What an auxiliary index is built with, by the writer (`lay_aux`) and where a home holds none (`Facts::index`).
pub fn named_codes(table: &rerail_tape_format::tape::CodeTable, language: GrammarDialect) -> Vec<u64> {
    let kinds = language.kinds();
    rerail_tape_format::aux_index::named_codes(table.len(), |c| {
        let e = table.entry(c);
        match &e.alias {
            Some(alias) => alias.1,
            None => kinds.kind_is_named(if e.flags & rerail_tape_format::CODE_ERROR != 0 { super::ERROR_SYMBOL } else { e.kind }),
        }
    })
}

/// `source`'s tape from the `.rerail` parser, in the run's format: the parser's own sink (`lean::Sink16`, its keys
/// numbered in the language's table), with each block's presence, through `lean::tape_to` (compiled in the parser's
/// crate).
pub(super) fn rr_tape(source: &str) -> Result<f16::Finished, String> {
    let (_, _, z) = run_16(GrammarDialect::Rr)?;
    rerail_rr_syntax::lean::tape_to(source, z, true)
}

/// A document's tape as held (its image), shared by the document (`TapeDoc`) and the facts read off it
/// (`Facts::tape`).
pub struct DocTape {
    /// The image's reader, made once (`reader`). It borrows `image` and `cache`, so it is declared first: dropped
    /// before them.
    reader: AnyTape<'static>,
    image: Arc<TapeImage>,
    /// The cache a 16z tape's blocks are decoded into: its batch's (`TapeDoc::in_batch`); none outside a batch.
    cache: Option<Arc<f16::ZCache>>,
    /// The document's source.
    source: Arc<Text>,

}

impl DocTape {
    pub fn new(image: Arc<TapeImage>, source: Arc<Text>) -> Self {
        Self::in_batch(image, source, None)
    }

    /// `new`, a 16z tape's blocks decoded into `cache`, its batch's.
    pub fn in_batch(image: Arc<TapeImage>, source: Arc<Text>, cache: Option<Arc<f16::ZCache>>) -> Self {
        let reader = image.tape_in(cache.as_deref());
        // SAFETY: the reader borrows the image's bytes and code table, the cache, and the language tables (static).
        // The image and the cache are in their `Arc`s' allocations, which do not move, and are never changed once
        // held (the cache is written through its atomics only); this holds the `Arc`s for as long as the reader, and
        // `reader` lends it at the borrow of `self` only.
        let reader = unsafe { std::mem::transmute::<AnyTape<'_>, AnyTape<'static>>(reader) };
        DocTape { reader, image, cache, source }
    }


    /// The tape's reader, made once: a node's reads dispatch on the format, and nothing is built a read.
    #[inline]
    pub fn reader(&self) -> &AnyTape<'_> {
        &self.reader
    }

    /// The image, shared: a document over the same tape holds it (`TapeDoc::over`).
    pub fn image_arc(&self) -> &Arc<TapeImage> {
        &self.image
    }

    /// The cache its 16z tape's blocks are decoded into: its batch's, where it is read in one.
    pub fn cache(&self) -> Option<&Arc<f16::ZCache>> {
        self.cache.as_ref()
    }

    /// The source the tape is over.
    pub fn source(&self) -> &Arc<Text> {
        &self.source
    }

    pub fn image(&self) -> &TapeImage {
        &self.image
    }

    /// The bytes it holds.
    pub fn bytes(&self) -> usize {
        self.image.estimated_bytes()

    }

    /// The tape as the reads written once take it (decision 0310): the bytes every offset of its view counts
    /// from, and the view. Its home's bytes (`extent`): the tape, its presence and its auxiliary index, which its view
    /// names (`presence`) and its reads read in place (`tape_has_kind`, `tape_place`). Format 16's arrays as held; a
    /// 16z tape's blocks as stored, which a reader reads through its batch's cache (`ZCache::view_for`: a block
    /// decoded where a read falls, 0353 step 4b).
    pub fn in_place(&self) -> Result<(&[u8], rerail_clif::view::TapeView), String> {
        let lang = self.image.lang();
        match self.image.held() {
            Held::F16(t) => Ok((t.extent(), t.placement().view(lang.ok_or("a format 16 tape of a language without its table")?, 0)?)),
            Held::F16Z(z) => Ok((z.extent(), z.view(lang.ok_or("a 16z tape of a language without its table")?)?)),
        }
    }

    /// The tape, read in place in the format it is held in.
    pub fn tape(&self) -> AnyTape<'_> {
        self.reader
    }

    /// The tape, a 16z tape's blocks decoded into `cache`, its batch's: a read of it in that batch.
    pub fn tape_with<'a>(&'a self, cache: Option<&'a f16::ZCache>) -> AnyTape<'a> {
        // Its own cache where it holds one: its batch's, or its own batch's where its blocks did not fit the batch's
        // entries (`TapeDoc::in_batch`). `cache` is for a document held outside every batch.
        match (&self.reader, cache) {
            (AnyTape::F16Z(_), Some(_)) if self.cache.is_none() => self.image.tape_in(cache),
            _ => self.reader,
        }
    }
}

impl TapeDoc {
    /// This document's tape, read in place in the format it is held in (`DocTape::tape`).
    pub fn tape(&self) -> AnyTape<'_> {
        self.held.tape()
    }

    /// The tape it holds, shared with the facts read off it.
    pub fn doc_tape(&self) -> &Arc<DocTape> {
        &self.held
    }

    /// The document read in its batch: a 16z tape's blocks decoded into `cache`, the batch's (the device's reads
    /// share it). The document itself stays its own, read in every batch it is in, each through its own cache; a
    /// tape of another format is read as held, and the document is returned as it is.
    /// A tape whose blocks the batch's entries do not hold (a document made when first read, after its batch's cache
    /// was sized: `syntax::Pending`) is read in a batch of its own (`alone`).
    pub fn in_batch(&self, cache: &Arc<f16::ZCache>) -> TapeDoc {
        if self.held.image.format() != Format::F16Z {
            return self.clone();
        }
        if self.z_tape().is_some_and(|t| cache.entry_size() < t.entry_size()) {
            return self.alone();
        }
        let held = DocTape::in_batch(Arc::clone(&self.held.image), Arc::clone(&self.held.source), Some(Arc::clone(cache)));
        TapeDoc { held: Arc::new(held), ..self.clone() }
    }

    /// Its 16z tape, where it holds one: a batch's cache is made for these (`f16::ZCache::of`).
    pub fn z_tape(&self) -> Option<&f16::Tape16Z> {
        self.held.image.z_tape()
    }

    /// The document read in a batch of its own (a reader outside every batch): a 16z tape's blocks decoded into a
    /// cache made for it, where it is in no batch already (`in_one_batch`).
    pub fn alone(&self) -> TapeDoc {
        in_one_batch(std::slice::from_ref(self)).into_owned().swap_remove(0)
    }

    /// Whether it is a 16z document read in no batch whose cache holds its blocks (`in_batch`).
    fn unbatched(&self) -> bool {
        self.z_tape().is_some_and(|t| self.held.cache.as_ref().is_none_or(|c| c.entry_size() < t.entry_size()))
    }
}

/// `files` read in one batch: a reader of documents outside every batch (the transducer's) makes one of the 16z
/// documents it reads that are in none, their blocks decoded into one cache made for them (`TapeDoc::in_batch`). The
/// files as they are where each is in one already.
pub fn in_one_batch(files: &[TapeDoc]) -> std::borrow::Cow<'_, [TapeDoc]> {
    if !files.iter().any(TapeDoc::unbatched) {
        return std::borrow::Cow::Borrowed(files);
    }
    let cache = Arc::new(f16::ZCache::of(files.iter().filter(|d| d.unbatched()).filter_map(TapeDoc::z_tape)));
    std::borrow::Cow::Owned(files.iter().map(|d| if d.unbatched() { d.in_batch(&cache) } else { d.clone() }).collect())
}
