//! Decision 0293: a document's tape through the one API (`rerail_tape_format::tape`), in the format the run stores
//! (`RERAIL_TAPE`), read in place:
//!
//! - **16** (the default since 2026-11-17, 0156): format 16, written by every parser as it parses (the lean parsers'
//!   writer, `lean`; a tree-sitter walk's ops, `TreeOps`; simdjson's events); the store keeps it, and a load reads it
//!   as stored;
//! - **16z**: format 16 in fixed LZ4 blocks, a block decoded where a read falls in it.
//!
//! Every language has its table, made from its grammar (decision 0626). A node whose code the table lacks, or past
//! format 16's bounds, is an error: there is no other format to keep (format 15 is removed, 0626).

use super::{Code, FileImage, Grammar, Held, TapeDoc, TapeImage};
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
/// §4, step 6): the tape's presence stored after it, in its home.
const F16_LAYOUT: u32 = 5;

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
            Ok(other) => panic!("RERAIL_TAPE={other}: the formats are 16 and 16z (format 15 is removed, decision 0626)"),
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

/// The run's format 16 target for `language`, each of the parser's keys numbered in the language's table through
/// its code (`number`).
fn sixteen(language: GrammarDialect, number: impl Fn(&'static Lang, u32) -> Option<u32> + Send + Sync + 'static) -> Result<rerail_tape_format::Sixteen, String> {
    let (lang, tag, z) = run_16(language)?;
    Ok(rerail_tape_format::Sixteen { lang, terminators: crate::code_rule_dsl::lines::terminators(language), tag, z, blocks: true, number: Arc::new(move |key| number(lang, key)) })
}

/// The language's format 16 table, its tag and whether the run stores 16z; `Err` for a language with no tape tag or
/// no table (every tape tag has one: decision 0626).
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
        let (lang, tag, z) = run_16(language)?;
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
}

/// The sink for a lean parser whose crate names its keys (`names`, decision 0626): each numbered in the language's
/// table by the grammar's names the table carries (`Lang::number_named`), as the crate's own tapes are (its `sink`).
pub fn lean_named<'a>(grammar: &'a Grammar, source: &'a str, budget: &'a dyn Fn() -> bool, names: fn(u32) -> Option<rerail_tape_format::KeyNames>) -> Lean<'a> {
    let target = sixteen(grammar.lang, move |lang, key| lang.number_named(&names(key)?));
    Lean { grammar, source, budget, target }
}

impl rerail_tape_format::Sink<Result<TapeImage, String>> for Lean<'_> {
    fn target(&self) -> rerail_tape_format::Target {
        self.target.clone().map_or(rerail_tape_format::Target::F15, rerail_tape_format::Target::F16)
    }

    fn finish(&mut self, mut w: Writer) -> Result<Result<TapeImage, String>, Redo> {
        if let Err(e) = &self.target {
            return Ok(Err(e.clone()));
        }
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

/// The finish the `.rerail` parser's writer is given (`rerail_rr_syntax::lean::tape_into`): `lean`'s, for a parser whose
/// keys are the codes (`kf | flags << 16`, no supertypes).
struct Rr {
    target: Result<rerail_tape_format::Sixteen, String>,
}

impl rerail_tape_format::Finish for Rr {
    type Out = Result<f16::Finished, String>;
    fn target(&self) -> rerail_tape_format::Target {
        self.target.clone().map_or(rerail_tape_format::Target::F15, rerail_tape_format::Target::F16)
    }

    fn finish<L: rerail_tape_format::Layout>(&mut self, mut w: Writer<L>) -> Result<Self::Out, Redo> {
        if let Err(e) = &self.target {
            return Ok(Err(e.clone()));
        }
        w.finish_16()
    }
}

/// `source`'s tape from the `.rerail` parser, in the run's format (`Rr`).
pub(super) fn rr_tape(source: &str) -> Result<f16::Finished, String> {
    let kind_mask = (1u16 << rerail_tape_format::KIND_BITS) - 1;
    let entry = move |key: u32| {
        let kf = (key & 0xffff) as u16;
        Entry { kind: kf & kind_mask, field: kf >> rerail_tape_format::KIND_BITS, flags: (key >> 16 & 7) as u8, alias: None, classes: Vec::new() }
    };
    let target = sixteen(GrammarDialect::Rr, move |lang, key| lang.number(&entry(key)));
    rerail_rr_syntax::lean::tape_into(source, &mut Rr { target })
}

/// A document's tape as held (its image), shared by the document (`TapeDoc`) and the facts read off it
/// (`Facts::tape`).
pub struct DocTape {
    /// The image's reader, made once (`reader`). It borrows `image`, so it is declared first: dropped before it.
    reader: AnyTape<'static>,
    image: Arc<TapeImage>,
    /// The document's source.
    source: Arc<String>,
}

impl DocTape {
    pub fn new(image: Arc<TapeImage>, source: Arc<String>) -> Self {
        let reader = image.tape();
        // SAFETY: the reader borrows the image's bytes and code table, and the language tables (static). The image
        // is in the `Arc`'s allocation, which does not move, and is never changed once held; this holds the `Arc`
        // for as long as the reader, and `reader` lends it at the borrow of `self` only.
        let reader = unsafe { std::mem::transmute::<AnyTape<'_>, AnyTape<'static>>(reader) };
        DocTape { reader, image, source }
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
    }

    /// The tape as the reads written once take it (decision 0310): the bytes every offset of its view counts
    /// from, and the view. Format 16's arrays as held; a 16z tape's blocks as stored, which
    /// a reader reads through a `rerail_clif::view::Reader` of its own (a block decoded where a read falls, 0353
    /// step 4b).
    pub fn in_place(&self) -> Result<(&[u8], rerail_clif::view::TapeView), String> {
        fn sixteen<'a>(t: &'a f16::Tape16, lang: Option<&Lang>) -> Result<(&'a [u8], rerail_clif::view::TapeView), String> {
            let lang = lang.ok_or("a format 16 tape of a language without its table")?;
            Ok((t.as_bytes(), f16::placement(t.as_bytes())?.view(lang, 0)?))
        }
        let lang = self.image.lang();
        match self.image.held() {
            Held::F16(t) => sixteen(t, lang),
            Held::F16Z(z) => Ok((z.as_bytes(), z.view(lang.ok_or("a 16z tape of a language without its table")?)?)),
        }
    }

    /// The tape, read in place in the format it is held in.
    pub fn tape(&self) -> AnyTape<'_> {
        self.reader
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
}
