//! Decision 0293: a document's tape through the one API (`rerail_tape_format::tape`), in the format the run stores
//! (`RERAIL_TAPE`), read in place:
//!
//! - **16** (the default since 2026-11-17, 0156): format 16, written by the lean parsers directly (`lean`), else
//!   from format 15's columns where a file's documents are made (`FileImage::single`, `embedding`); the store
//!   keeps it, and a load reads it as stored;
//! - **15**: format 15's columns, as the other parsers write them;
//! - **16z**: format 16 in fixed LZ4 blocks, a block decoded where a read falls in it.
//!
//! A language without format 16's table (`f16::lang`), or an image with a code the table lacks, keeps format 15,
//! and the profile counts it (`tape.keeps_15.<language>`); a run that set `RERAIL_TAPE` is told once a language.

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

/// Format 16's and 16z's stored layout: 3 since decision 0336 §6 (a newline index after the side lists, its count
/// in the header; 16z holds it raw after the checkpoints). Layout 2 (0336: region starts in the header; 16z's blocks
/// one piece an array, raw, LZ4 or a repeat) had keys 216 and 1216; layout 1, 16 and 1016. Layout 5 (decision 0434
/// §4, step 6): the tape's presence stored after it, in its home.
const F16_LAYOUT: u32 = 5;

impl Format {
    pub const ALL: [Format; 3] = [Format::F15, Format::F16, Format::F16Z];

    /// `RERAIL_TAPE`: `16` (the default since the user took 0156, 2026-11-17), `15` or `16z`. Another value stops
    /// the run, so a measurement never reads a format it did not ask for.
    pub fn of_run() -> Format {
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(format) = AS_RUN.with(std::cell::Cell::get) {
            return format;
        }
        rerail_prof::code_rule_dsl::logic::profile::setting(|| match std::env::var("RERAIL_TAPE").as_deref() {
            Err(_) | Ok("" | "16") => Format::F16,
            Ok("15") => Format::F15,
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

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// The format 15 images this thread converted to format 16 or 16z (`TapeImage::in_format`): a parser's tape
    /// made in format 15 first, which no format 16 run's parser does (decision 0434, R12).
    static CONVERTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The format 15 images this thread has converted (`CONVERTED`).
#[cfg(any(test, feature = "test-hooks"))]
pub fn converted() -> usize {
    CONVERTED.get()
}

/// `f` as a run that stores `format`, on this thread: a test's, so that one process writes every format.
#[cfg(any(test, feature = "test-hooks"))]
pub fn as_run<R>(format: Format, f: impl FnOnce() -> R) -> R {
    let before = AS_RUN.with(|r| r.replace(Some(format)));
    let out = f();
    AS_RUN.with(|r| r.set(before));
    out
}

/// This thread runs as a run that stores `format` until it ends: a test of one format's image (format 15's columns,
/// say) whatever the process's run stores. Each test has a thread of its own.
#[cfg(any(test, feature = "test-hooks"))]
pub fn this_thread_stores(format: Format) {
    AS_RUN.with(|r| r.set(Some(format)));
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

    /// Its columns, as format 15's reader reads them, over a source of `source_len` bytes (its image's).
    pub fn columns(&self, source_len: u64) -> F15Columns<'_> {
        F15Columns {
            code: self.code(),
            size: self.size(),
            base: self.base(),
            off: self.off(),
            tail: self.tail(),
            widths: [self.widths[0], self.widths[1], self.widths[2] & !TAIL_FROM_START],
            from_start: self.widths[2] & TAIL_FROM_START != 0,
            source_len,
        }
    }

    /// The view the tape's reads take (decision 0310), its offsets from its bytes' first (`laid_out`): made from
    /// the columns' places, as cheap as the columns' slices; its spans held to the source's `source_len` bytes (its
    /// image's: decision 0463).
    pub fn view(&self, source_len: usize) -> rerail_clif::view::TapeView {
        let (_, at) = self.laid_out();
        let w = |k: usize| u32::from(self.widths[k]);
        let from_start = self.widths[2] & TAIL_FROM_START != 0;
        rerail_clif::view::TapeView {
            nodes: (self.code().len() / 2) as u32,
            format: 15,
            widths: w(0) | w(1) << 4 | u32::from(self.widths[2] & !TAIL_FROM_START) << 8 | u32::from(from_start) << 12,
            columns: at.map(|a| a as u32),
            source_len: u32::try_from(source_len).unwrap_or(u32::MAX),
            ..Default::default()
        }
    }
}

/// A language whose documents keep format 15, and why: said once a language where the run asked for its format
/// (`RERAIL_TAPE`), and counted in the profile always (format 16 is the default, so a run that asked nothing is
/// not told).
pub fn keeps_15(language: GrammarDialect, tag: u8, why: &str) {
    static SAID: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];
    if !SAID[usize::from(tag)].swap(true, Ordering::Relaxed) {
        crate::code_rule_dsl::prof::count_named(format!("tape.keeps_15.{language:?}"), 1);
        if std::env::var_os("RERAIL_TAPE").is_none() {
            return;
        }
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

/// The run's format 16 target for `language`, each of the parser's keys numbered in the language's table through
/// its code (`number`): `None` where the run stores format 15, or the language has no format 16 table (said once).
fn sixteen(language: GrammarDialect, number: impl Fn(&'static Lang, u32) -> Option<u32> + Send + Sync + 'static) -> Option<rerail_tape_format::Sixteen> {
    let (lang, tag, z) = run_16(language)?;
    Some(rerail_tape_format::Sixteen { lang, tag, z, blocks: true, number: Arc::new(move |key| number(lang, key)) })
}

/// The language's format 16 table, its tag and whether the run stores 16z: `None` where the run stores format 15, or
/// the language has no format 16 table (said once).
pub fn run_16(language: GrammarDialect) -> Option<(&'static Lang, u8, bool)> {
    let format = Format::of_run();
    if format == Format::F15 {
        return None;
    }
    let tag = crate::code_rule_dsl::language::tape_tag(language)?;
    let Some(lang) = f16::lang(tag) else {
        keeps_15(language, tag, "its language has no format 16 table");
        return None;
    };
    Some((lang, tag, format == Format::F16Z))
}

/// The cursor walk's format 16 output (`tape::TreeOut`; decision 0434 §2): each node given to format 16's ops as the
/// walk opens and closes it, its number the language's for its code, looked up once a code a document. A code the
/// table lacks, or a node past format 16's bounds, makes `finish` `None` (said once a language): the walk is run
/// again for format 15.
pub(super) struct TreeOps {
    language: GrammarDialect,
    lang: &'static Lang,
    tag: u8,
    w: f16::Writer,
    /// The open nodes' ends.
    ends: Vec<u32>,
    numbers: rustc_hash::FxHashMap<Code, u32>,
    /// The document's contextual aliases, numbered by first use (a code's `alias`).
    aliases: Vec<(String, bool)>,
    bad: Option<String>,
}

impl TreeOps {
    /// The run's format 16 output for a tree of `language` over `source`, of about `nodes` nodes: `None` where the
    /// run stores format 15 (`run_16`).
    pub(super) fn of_run(language: GrammarDialect, source: &[u8], nodes: usize) -> Option<Self> {
        let (lang, tag, z) = run_16(language)?;
        let w = f16::Writer::new(lang, source, tag, z, true, nodes);
        Some(TreeOps { language, lang, tag, w, ends: Vec::with_capacity(64), numbers: Default::default(), aliases: Vec::new(), bad: None })
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
        // A code's classes ascending, each once, as format 15's codes hold them.
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

    /// The tape, or `None` where the nodes are not a format 16 tape (said once a language).
    pub(super) fn finish(self) -> Option<f16::Finished> {
        let why = match self.bad {
            Some(why) => why,
            None => match self.w.finish() {
                Ok(f) => return Some(f),
                Err(e) => e,
            },
        };
        keeps_15(self.language, self.tag, &why);
        None
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
/// format. Where the run stores format 16 or 16z, the writer passes its nodes to format 16's ops as the parse commits
/// them, each key numbered in the language's table through its code (`code_of`): no format 15 column is made. Format
/// 15's columns (`TapeImage::written`) where the run stores 15, the language has no format 16 table, a code is not in
/// it, or the nodes break format 16's bounds (said once a language).
pub struct Lean<'a> {
    grammar: &'a Grammar,
    source: &'a str,
    budget: &'a dyn Fn() -> bool,
    code_of: Arc<dyn Fn(&u32) -> Result<Code, String> + Send + Sync>,
    target: Option<rerail_tape_format::Sixteen>,
    /// `finish` asks for the parse again, in format 15 (`Sink::again`).
    again: bool,
}

pub fn lean<'a>(grammar: &'a Grammar, source: &'a str, budget: &'a dyn Fn() -> bool, code_of: impl Fn(&u32) -> Result<Code, String> + Send + Sync + 'static) -> Lean<'a> {
    let code_of: Arc<dyn Fn(&u32) -> Result<Code, String> + Send + Sync> = Arc::new(code_of);
    let codes = code_of.clone();
    let target = sixteen(grammar.lang, move |lang, key| codes(&key).ok().and_then(|c| lang.number(&c.entry())));
    Lean { grammar, source, budget, code_of, target, again: false }
}

impl rerail_tape_format::Sink<Result<TapeImage, String>> for Lean<'_> {
    fn target(&self) -> rerail_tape_format::Target {
        self.target.clone().map_or(rerail_tape_format::Target::F15, rerail_tape_format::Target::F16)
    }

    fn again(&mut self) -> bool {
        std::mem::take(&mut self.again)
    }

    fn finish(&mut self, mut w: Writer) -> Result<Result<TapeImage, String>, Redo> {
        let language = self.grammar.lang;
        if let Some(t) = &self.target {
            match w.finish_16()? {
                Ok(f) => return Ok(TapeImage::written_16(f, language, self.source, None, self.budget)),
                Err(e) => keeps_15(language, t.tag, &e),
            }
            // The window gave back the nodes passed on: the parse runs again for format 15 (`again`).
            self.target = None;
            if !w.holds_all() {
                self.again = true;
                return Err(Redo(w.widths()));
            }
        }
        let columns = w.finish_columns()?;
        let codes: Result<Vec<Code>, String> = columns.keys.iter().map(|k| (self.code_of)(k)).collect();
        Ok(codes.and_then(|codes| TapeImage::written(columns.code, columns.size, columns.base, columns.off, columns.tail, columns.widths, codes, None, self.grammar, self.source, self.budget)))
    }
}

impl TapeImage {
    /// This tape written in `format`, from format 15's columns (decision 0293's interim: the parsers write format
    /// 15, and format 16 is written from it where a file's documents are made). A tape already held in another
    /// format is kept as it is; one whose language has no format 16 table, or whose code the table lacks, keeps
    /// format 15, and the run says so once a language.
    /// `source` is its document's: format 16 holds its newline index.
    pub fn in_format(self, format: Format, source: &[u8]) -> Self {
        let Held::F15(c) = &self.held else { return self };
        debug_assert_eq!(source.len(), self.source_len, "a tape's source");
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
        #[cfg(any(test, feature = "test-hooks"))]
        CONVERTED.set(CONVERTED.get() + 1);
        let f = match f16::write_from_with_presence(lang, &F15::over(c.laid_out().0, c.view(self.source_len), table), &number, source, tag, format == Format::F16Z) {
            Ok(f) => f,
            Err(e) => {
                keeps_15(self.language, tag, &e);
                return self;
            }
        };
        let held = match f.tape {
            f16::Written::F16(t) => Held::F16(Arc::new(t)),
            f16::Written::F16Z(t) => Held::F16Z(Arc::new(t)),
        };
        Self { held, ..self }
    }

    /// A lean parser's tape written in format 16 or 16z (`lean`), with its presence and its source's digest: `digest`,
    /// where the parse computed it (simdjson's kernels), else computed here.
    pub fn written_16(written: f16::Finished, language: GrammarDialect, source: &str, digest: Option<[u8; 32]>, budget: &dyn Fn() -> bool) -> Result<Self, String> {
        let digest = match digest {
            Some(d) => d,
            None => super::source_digest(source, &budget)?,
        };
        let held = match written.tape {
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
/// writer, as `lean` writes the other languages', else format 15's image.
pub(super) enum RrTape {
    Image(Vec<u8>),
    Written(Box<f16::Finished>),
}

/// The finish the `.rerail` parser's writer is given (`rerail_rr_syntax::lean::tape_into`): `lean`'s, for a parser whose
/// keys are the codes (`kf | flags << 16`, no supertypes).
struct Rr<'s> {
    source: &'s str,
    /// Format 16 failed: format 15's image, parsed again.
    f15: bool,
}

impl rerail_tape_format::Finish for Rr<'_> {
    type Out = RrTape;
    fn target(&self) -> rerail_tape_format::Target {
        if self.f15 {
            return rerail_tape_format::Target::F15;
        }
        let kind_mask = (1u16 << rerail_tape_format::KIND_BITS) - 1;
        let entry = move |key: u32| {
            let kf = (key & 0xffff) as u16;
            Entry { kind: kf & kind_mask, field: kf >> rerail_tape_format::KIND_BITS, flags: (key >> 16 & 7) as u8, alias: None, classes: Vec::new() }
        };
        sixteen(GrammarDialect::Rr, move |lang, key| lang.number(&entry(key))).map_or(rerail_tape_format::Target::F15, rerail_tape_format::Target::F16)
    }

    fn finish<L: rerail_tape_format::Layout>(&mut self, mut w: Writer<L>) -> Result<RrTape, Redo> {
        if w.writes_16() {
            match w.finish_16()? {
                Ok(f) => return Ok(RrTape::Written(Box::new(f))),
                Err(e) => keeps_15(GrammarDialect::Rr, rerail_tape_format::RR_TAG, &e),
            }
            // As `Lean`'s: the parse runs again for format 15.
            self.f15 = true;
            if !w.holds_all() {
                return Err(Redo(w.widths()));
            }
        }
        w.finish(self.source.as_bytes()).map(RrTape::Image)
    }
}

/// `source`'s tape from the `.rerail` parser, in the run's format (`Rr`).
pub(super) fn rr_tape(source: &str) -> RrTape {
    rerail_rr_syntax::lean::tape_into(source, &mut Rr { source, f15: false })
}

impl FileImage {
    /// Every document's tape in `format` (`TapeImage::in_format`): the primary's of `source`, a derived one's of its
    /// text.
    pub fn in_format(self, format: Format, source: &str) -> Self {
        FileImage {
            primary: self.primary.in_format(format, source.as_bytes()),
            derived: self.derived.into_iter().map(|d| super::DerivedImage { tape: d.tape.in_format(format, d.text.as_bytes()), ..d }).collect(),
        }
    }
}

/// A document's tape as held (its image), shared by the document (`TapeDoc`) and the facts read off it
/// (`Facts::tape`). A test may read it in another format (`tape_in`): written from format 15 once.
pub struct DocTape {
    /// The image's reader, made once (`reader`). It borrows `image`, so it is declared first: dropped before it.
    reader: AnyTape<'static>,
    image: Arc<TapeImage>,
    /// The document's source, which another format's tape is written over (`tape_in`: format 16 holds its newlines).
    source: Arc<String>,
    other: [OnceLock<TapeImage>; 3],
}

impl DocTape {
    pub fn new(image: Arc<TapeImage>, source: Arc<String>) -> Self {
        let reader = image.tape();
        // SAFETY: the reader borrows the image's bytes and code table, and the language tables (static). The image
        // is in the `Arc`'s allocation, which does not move, and is never changed once held; this holds the `Arc`
        // for as long as the reader, and `reader` lends it at the borrow of `self` only.
        let reader = unsafe { std::mem::transmute::<AnyTape<'_>, AnyTape<'static>>(reader) };
        DocTape { reader, image, source, other: Default::default() }
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

    /// The source the tape is over.
    pub fn source(&self) -> &Arc<String> {
        &self.source
    }

    pub fn image(&self) -> &TapeImage {
        &self.image
    }

    /// The bytes it holds.
    pub fn bytes(&self) -> usize {
        self.image.estimated_bytes()
            + self.other.iter().filter_map(OnceLock::get).map(TapeImage::estimated_bytes).sum::<usize>()
    }

    /// The tape as the reads written once take it (decision 0310): the bytes every offset of its view counts
    /// from, and the view. Format 15's columns and format 16's arrays as held; a 16z tape's blocks as stored, which
    /// a reader reads through a `rerail_clif::view::Reader` of its own (a block decoded where a read falls, 0353
    /// step 4b).
    pub fn in_place(&self) -> Result<(&[u8], rerail_clif::view::TapeView), String> {
        fn sixteen<'a>(t: &'a f16::Tape16, lang: Option<&Lang>) -> Result<(&'a [u8], rerail_clif::view::TapeView), String> {
            let lang = lang.ok_or("a format 16 tape of a language without its table")?;
            Ok((t.as_bytes(), f16::placement(t.as_bytes())?.view(lang, 0)?))
        }
        let lang = self.image.lang();
        match self.image.held() {
            Held::F15(c) => Ok((c.laid_out().0, c.view(self.image.source_len))),
            Held::F16(t) => sixteen(t, lang),
            Held::F16Z(z) => Ok((z.as_bytes(), z.view(lang.ok_or("a 16z tape of a language without its table")?)?)),
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
        self.other[slot].get_or_init(|| (*self.image).clone().in_format(format, self.source.as_bytes())).tape()
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
