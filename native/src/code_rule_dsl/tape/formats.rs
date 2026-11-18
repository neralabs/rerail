//! Decision 0293: a document's tape through the one API (`rerail_tape_format::tape`), in the format the run stores
//! (`RERAIL_TAPE`), read in place:
//!
//! - **15** (the default): format 15's columns, as the parsers write them;
//! - **16**: format 16, written from them where a file's documents are made (`FileImage::single`, `embedding`),
//!   until the parsers write it; the store keeps it, and a load reads it as stored;
//! - **16z**: format 16 in fixed LZ4 blocks, a block decoded where a read falls in it.
//!
//! A language without format 16's table (`f16::lang`), or an image with a code the table lacks, keeps format 15,
//! and the run says so once a language.

use super::{Code, Columns15, FileImage, Grammar, Held, SENTINEL, TAIL_FROM_START, TapeDoc, TapeImage};
use crate::code_rule_dsl::language::GrammarDialect;
use rerail_tape_format::{Redo, Writer};
use rerail_tape_format::f16::{self, Lang};
use rerail_tape_format::tape::{AnyTape, CodeTable, Entry, F15, F15Columns};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// The format a run stores and reads its documents' tapes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    F15,
    F16,
    F16Z,
}

/// Format 16's and 16z's stored layout: 2 since decision 0336 (the header holds each region's start; 16z's blocks
/// are one piece an array, raw, LZ4 or a repeat, its checkpoints raw). Layout 1 had keys 16 and 1016.
const F16_LAYOUT: u32 = 2;

impl Format {
    pub const ALL: [Format; 3] = [Format::F15, Format::F16, Format::F16Z];

    /// `RERAIL_TAPE`: `15` (the default until 0156 is decided), `16` or `16z`. Another value stops the run, so a
    /// measurement never reads a format it did not ask for.
    pub fn of_run() -> Format {
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(format) = AS_RUN.with(std::cell::Cell::get) {
            return format;
        }
        static RUN: OnceLock<Format> = OnceLock::new();
        *RUN.get_or_init(|| match std::env::var("RERAIL_TAPE").as_deref() {
            Err(_) | Ok("" | "15") => Format::F15,
            Ok("16") => Format::F16,
            Ok("16z") => Format::F16Z,
            Ok(other) => panic!("RERAIL_TAPE={other}: the formats are 15, 16 and 16z"),
        })
    }

    /// The tape format version the store keys a tape by when a run stores this format: a store holds each
    /// document once per format, and a run finds only its own. (A language that keeps format 15 in a run of
    /// format 16 is stored under 16, as format 15's bytes: the bytes say what they hold.) Format 16's and 16z's
    /// carry their layout (`F16_LAYOUT`): a store built before a layout changed reads cold, not wrong.
    pub fn store_version(self) -> u32 {
        match self {
            Format::F15 => super::TAPE_FORMAT_VERSION,
            Format::F16 => 16 + 100 * F16_LAYOUT,
            Format::F16Z => 1016 + 100 * F16_LAYOUT,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::F15 => "15",
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

/// The tape format version the store keys this run's tapes by (`Format::store_version`).
pub fn stored_version() -> u32 {
    Format::of_run().store_version()
}

impl Columns15 {
    /// Its code table as the API's: the entries format 15's reader takes from the image's bytes (`f15_columns`).
    pub fn code_table(&self) -> CodeTable {
        CodeTable::new(
            self.codes
                .iter()
                .map(|c| Entry {
                    // An alias past the table (a damaged image) reads as an empty name, which a restore refuses.
                    alias: (c.alias != SENTINEL).then(|| self.kind_aliases.get(usize::from(c.alias)).cloned().unwrap_or_default()),
                    ..c.entry()
                })
                .collect(),
        )
    }

    /// Its columns, as format 15's reader reads them.
    pub fn columns(&self) -> F15Columns<'_> {
        F15Columns {
            code: self.code(),
            size: self.size(),
            base: self.base(),
            off: self.off(),
            tail: self.tail(),
            widths: [self.widths[0], self.widths[1], self.widths[2] & !TAIL_FROM_START],
            from_start: self.widths[2] & TAIL_FROM_START != 0,
        }
    }

    /// The view the tape's reads take (decision 0310), its offsets from its bytes' first (`laid_out`): made from
    /// the columns' places, as cheap as the columns' slices.
    pub fn view(&self) -> rerail_clif::view::TapeView {
        let (_, at) = self.laid_out();
        let w = |k: usize| u32::from(self.widths[k]);
        let from_start = self.widths[2] & TAIL_FROM_START != 0;
        rerail_clif::view::TapeView {
            nodes: (self.code().len() / 2) as u32,
            format: 15,
            widths: w(0) | w(1) << 4 | u32::from(self.widths[2] & !TAIL_FROM_START) << 8 | u32::from(from_start) << 12,
            columns: at.map(|a| a as u32),
            ..Default::default()
        }
    }
}

/// The run says once a language that its documents keep format 15, and why.
fn keeps_15(language: GrammarDialect, tag: u8, why: &str) {
    static SAID: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];
    if !SAID[usize::from(tag)].swap(true, Ordering::Relaxed) {
        eprintln!("rerail: RERAIL_TAPE asks for format {}; {language:?} documents are read as format 15: {why}", Format::of_run().name());
    }
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

/// The finish a lean parser's writer is given (`rerail_*_lean::tape_into`; decision 0293): the tape in the run's
/// format, in one pass over the writer. Where the run stores format 16 or 16z, each code the parser's keys name
/// (`code_of`) is numbered in the language's table and the tape written from the writer's nodes
/// (`Writer::finish_16`): no format 15 column is made. Format 15's columns (`TapeImage::written`) where the run
/// stores 15, the language has no format 16 table, a code is not in it, or a node breaks format 16's bounds (said
/// once a language).
pub fn lean<'a>(
    grammar: &'a Grammar,
    source: &'a str,
    budget: &'a dyn Fn() -> bool,
    code_of: &'a dyn Fn(&u32) -> Result<Code, String>,
) -> impl FnMut(Writer) -> Result<Result<TapeImage, String>, Redo> + 'a {
    move |mut w| {
        let format = Format::of_run();
        let language = grammar.lang;
        if format != Format::F15
            && let Some(tag) = crate::code_rule_dsl::language::tape_tag(language)
        {
            match f16::lang(tag) {
                None => keeps_15(language, tag, "its language has no format 16 table"),
                Some(lang) => {
                    let codes: Vec<Code> = match w.code_keys().iter().map(code_of).collect() {
                        Ok(codes) => codes,
                        Err(e) => return Ok(Err(e)),
                    };
                    match codes.iter().map(|c| lang.number(&c.entry())).collect::<Option<Vec<u32>>>() {
                        None => keeps_15(language, tag, "a code its format 16 table lacks"),
                        Some(numbers) => match w.finish_16(lang, &numbers, source.len() as u64, tag, format == Format::F16Z)? {
                            Ok(written) => return Ok(TapeImage::written_16(written, language, source, budget)),
                            Err(e) => keeps_15(language, tag, &e),
                        },
                    }
                }
            }
        }
        let columns = w.finish_columns()?;
        let codes: Result<Vec<Code>, String> = columns.keys.iter().map(code_of).collect();
        Ok(codes.and_then(|codes| TapeImage::written(columns.code, columns.size, columns.base, columns.off, columns.tail, columns.widths, codes, None, grammar, source, budget)))
    }
}

impl TapeImage {
    /// This tape written in `format`, from format 15's columns (decision 0293's interim: the parsers write format
    /// 15, and format 16 is written from it where a file's documents are made). A tape already held in another
    /// format is kept as it is; one whose language has no format 16 table, or whose code the table lacks, keeps
    /// format 15, and the run says so once a language.
    pub fn in_format(self, format: Format) -> Self {
        let Held::F15(c) = &self.held else { return self };
        if format == Format::F15 {
            return self;
        }
        let Some(tag) = crate::code_rule_dsl::language::tape_tag(self.language) else { return self };
        let Some(lang) = f16::lang(tag) else {
            keeps_15(self.language, tag, "its language has no format 16 table");
            return self;
        };
        let table = c.table.get_or_init(|| c.code_table());
        let number = match f16::numbers(lang, table) {
            Ok(number) => number,
            Err(e) => {
                keeps_15(self.language, tag, &e);
                return self;
            }
        };
        // One pass over format 15's columns, to format 16 or straight to 16z's blocks.
        let held = match f16::write_from(lang, &F15::over(c.laid_out().0, c.view(), table), &number, self.source_len as u64, tag, format == Format::F16Z) {
            Ok(f16::Written::F16(t)) => Held::F16(Arc::new(t)),
            Ok(f16::Written::F16Z(t)) => Held::F16Z(Arc::new(t)),
            Err(e) => {
                keeps_15(self.language, tag, &e);
                return self;
            }
        };
        Self { held, ..self }
    }

    /// A lean parser's tape written in format 16 or 16z (`lean`), with its source's digest.
    pub fn written_16(written: f16::Written, language: GrammarDialect, source: &str, budget: &dyn Fn() -> bool) -> Result<Self, String> {
        let digest = super::source_digest(source, &budget)?;
        let held = match written {
            f16::Written::F16(t) => Held::F16(Arc::new(t)),
            f16::Written::F16Z(t) => Held::F16Z(Arc::new(t)),
        };
        Ok(TapeImage { held, language, source_len: source.len(), source_digest: Some(digest) })
    }

    /// The language's table, where the image is held in format 16.
    pub fn lang(&self) -> Option<&'static Lang> {
        match &self.held {
            Held::F15(_) => None,
            Held::F16(t) => f16::lang(t.tag()),
            Held::F16Z(t) => f16::lang(t.tag()),
        }
    }
}

/// A `.rerail` file's tape from its lean parser, in the run's format: format 16 or 16z written in one pass over the
/// writer, as `lean` writes the other languages', else format 15's image. One per file, moved once: its variants'
/// sizes do not matter.
#[allow(clippy::large_enum_variant)]
pub(super) enum RrTape {
    Image(Vec<u8>),
    Written(f16::Written),
}

/// The finish the `.rerail` parser's writer is given (`rerail_rr_syntax::lean::tape_into`): `lean`'s, for a parser whose
/// keys are the codes (`kf | flags << 16`, no supertypes).
struct Rr<'s>(&'s str);

impl rerail_tape_format::Finish for Rr<'_> {
    type Out = RrTape;
    fn finish<L: rerail_tape_format::Layout>(&mut self, mut w: Writer<L>) -> Result<RrTape, Redo> {
        let format = Format::of_run();
        let tag = rerail_tape_format::RR_TAG;
        if format != Format::F15 {
            match f16::lang(tag) {
                None => keeps_15(GrammarDialect::Rr, tag, "its language has no format 16 table"),
                Some(lang) => {
                    let kind_mask = (1u16 << rerail_tape_format::KIND_BITS) - 1;
                    let entry = |key: u32| {
                        let kf = (key & 0xffff) as u16;
                        Entry { kind: kf & kind_mask, field: kf >> rerail_tape_format::KIND_BITS, flags: (key >> 16 & 7) as u8, alias: None, classes: Vec::new() }
                    };
                    match w.code_keys().iter().map(|&k| lang.number(&entry(k))).collect::<Option<Vec<u32>>>() {
                        None => keeps_15(GrammarDialect::Rr, tag, "a code its format 16 table lacks"),
                        Some(numbers) => match w.finish_16(lang, &numbers, self.0.len() as u64, tag, format == Format::F16Z)? {
                            Ok(written) => return Ok(RrTape::Written(written)),
                            Err(e) => keeps_15(GrammarDialect::Rr, tag, &e),
                        },
                    }
                }
            }
        }
        w.finish(self.0.as_bytes()).map(RrTape::Image)
    }
}

/// `source`'s tape from the `.rerail` parser, in the run's format (`Rr`).
pub(super) fn rr_tape(source: &str) -> RrTape {
    rerail_rr_syntax::lean::tape_into(source, &mut Rr(source))
}

impl FileImage {
    /// Every document's tape in `format` (`TapeImage::in_format`).
    pub fn in_format(self, format: Format) -> Self {
        FileImage {
            primary: self.primary.in_format(format),
            derived: self.derived.into_iter().map(|d| super::DerivedImage { tape: d.tape.in_format(format), ..d }).collect(),
        }
    }
}

/// A document's tape as held (its image), shared by the document (`TapeDoc`) and the facts read off it
/// (`Facts::tape`). A test may read it in another format (`tape_in`): written from format 15 once.
pub struct DocTape {
    /// The image's reader, made once (`reader`). It borrows `image`, so it is declared first: dropped before it.
    reader: AnyTape<'static>,
    image: Arc<TapeImage>,
    other: [OnceLock<TapeImage>; 3],
    /// A 16z tape laid out as the format 16 tape it is the blocks of, once, for the readers that read format 16's
    /// arrays in place (`in_place`), as the device's slab lays it out.
    laid: OnceLock<Result<f16::Tape16, String>>,
    /// Each node's parent, kept once a read asks for one that no step knew (`parents`).
    parents: OnceLock<Box<[u32]>>,
}

impl DocTape {
    pub fn new(image: Arc<TapeImage>) -> Self {
        let reader = image.tape();
        // SAFETY: the reader borrows the image's bytes and code table, and the language tables (static). The image
        // is in the `Arc`'s allocation, which does not move, and is never changed once held; this holds the `Arc`
        // for as long as the reader, and `reader` lends it at the borrow of `self` only.
        let reader = unsafe { std::mem::transmute::<AnyTape<'_>, AnyTape<'static>>(reader) };
        DocTape { reader, image, other: Default::default(), laid: OnceLock::new(), parents: OnceLock::new() }
    }

    /// The tape's reader, made once: a node's reads dispatch on the format, and nothing is built a read.
    #[inline]
    pub fn reader(&self) -> &AnyTape<'_> {
        &self.reader
    }

    /// Each node's parent (`NONE` at the root), kept once a read asks for one that no step knew (`TapeNode::up`,
    /// `TapeDoc::transducer_parent`): one pass of the subtrees open around each node, a subtree end a node, as
    /// `TapeDoc::transducer_parents` reads them; 4 bytes a node.
    ///
    /// The tape stores no parent: format 15 searches back at most 64 nodes and then descends from the root over the
    /// siblings' subtree ends, format 16 steps back over the siblings' slots (0293 §8). The matcher asks for one at
    /// every step of a climb from a node the anchors or a binary search handed it (`NodeRef::find_near`, `inside`,
    /// a field's first child): a search a step where the records it read before held a load. Kept, a step is a
    /// load again, as a projection keeps the parents its walk finds (0293 §6, "kept on measurement"): the
    /// shipped Node.js built-in tables read a search per step on every file, 2.4 times main's time.
    pub fn parents(&self) -> &[u32] {
        self.parents.get_or_init(|| {
            use rerail_tape_format::tape::Tape;
            rerail_tape_format::with_tape!(&self.reader, |t| {
                let mut parents = Vec::with_capacity(t.len());
                let mut open: Vec<(u32, usize)> = Vec::new();
                for i in 0..t.len() {
                    while open.last().is_some_and(|&(_, end)| end <= i) {
                        open.pop();
                    }
                    parents.push(open.last().map_or(super::NONE, |&(p, _)| p));
                    open.push((i as u32, t.subtree_end(i)));
                }
                parents.into_boxed_slice()
            })
        })
    }

    pub fn image(&self) -> &TapeImage {
        &self.image
    }

    /// The bytes it holds.
    pub fn bytes(&self) -> usize {
        self.image.estimated_bytes()
            + self.other.iter().filter_map(OnceLock::get).map(TapeImage::estimated_bytes).sum::<usize>()
            + self.laid.get().and_then(|laid| laid.as_ref().ok()).map_or(0, f16::Tape16::bytes)
            + self.parents.get().map_or(0, |parents| 4 * parents.len())
    }

    /// The tape as the reads written once take it (decision 0310): the bytes every offset of its view counts
    /// from, and the view. Format 15's columns and format 16's arrays as held; a 16z tape's blocks laid out as
    /// format 16 once.
    pub fn in_place(&self) -> Result<(&[u8], rerail_clif::view::TapeView), String> {
        fn sixteen<'a>(t: &'a f16::Tape16, lang: Option<&Lang>) -> Result<(&'a [u8], rerail_clif::view::TapeView), String> {
            let lang = lang.ok_or("a format 16 tape of a language without its table")?;
            Ok((t.as_bytes(), f16::placement(t.as_bytes())?.view(lang, 0)?))
        }
        let lang = self.image.lang();
        match self.image.held() {
            Held::F15(c) => Ok((c.laid_out().0, c.view())),
            Held::F16(t) => sixteen(t, lang),
            Held::F16Z(z) => {
                let laid = self.laid.get_or_init(|| {
                    let mut bytes = vec![0; z.decoded_len()];
                    z.decode_into(&mut bytes)?;
                    f16::Tape16::from_bytes(bytes)
                });
                sixteen(laid.as_ref().map_err(Clone::clone)?, lang)
            }
        }
    }

    /// The tape, read in place in the format it is held in.
    pub fn tape(&self) -> AnyTape<'_> {
        self.reader
    }

    /// The tape in `format`: as held, or (a test's comparison of formats) written from format 15 once.
    pub fn tape_in(&self, format: Format) -> AnyTape<'_> {
        if format == self.image.format() {
            return self.image.tape();
        }
        let slot = Format::ALL.iter().position(|&f| f == format).expect("a format");
        self.other[slot].get_or_init(|| (*self.image).clone().in_format(format)).tape()
    }
}

impl TapeDoc {
    /// This document's tape, read in place in the format it is held in (`DocTape::tape`).
    pub fn tape(&self) -> AnyTape<'_> {
        self.held.tape()
    }

    /// This document's tape in `format` (`DocTape::tape_in`).
    pub fn tape_in(&self, format: Format) -> AnyTape<'_> {
        self.held.tape_in(format)
    }

    /// The tape it holds, shared with the facts read off it.
    pub fn doc_tape(&self) -> &Arc<DocTape> {
        &self.held
    }
}
