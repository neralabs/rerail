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

use super::{Columns15, FileImage, Held, SENTINEL, TAIL_FROM_START, TapeDoc, TapeImage};
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

impl Format {
    pub const ALL: [Format; 3] = [Format::F15, Format::F16, Format::F16Z];

    /// `RERAIL_TAPE`: `15` (the default until 0156 is decided), `16` or `16z`. Another value stops the run, so a
    /// measurement never reads a format it did not ask for.
    pub fn of_run() -> Format {
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
    /// format 16 is stored under 16, as format 15's bytes: the bytes say what they hold.)
    pub fn store_version(self) -> u32 {
        match self {
            Format::F15 => super::TAPE_FORMAT_VERSION,
            Format::F16 => 16,
            Format::F16Z => 1016,
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

/// The tape format version the store keys this run's tapes by (`Format::store_version`).
pub fn stored_version() -> u32 {
    Format::of_run().store_version()
}

impl Columns15 {
    /// Its code table as the API's: the entries format 15's reader takes from the image's bytes (`f15_columns`).
    pub fn code_table(&self) -> CodeTable {
        let kind_mask = (1u16 << rerail_tape_format::KIND_BITS) - 1;
        CodeTable::new(
            self.codes
                .iter()
                .map(|c| Entry {
                    kind: c.kf() & kind_mask,
                    field: c.kf() >> rerail_tape_format::KIND_BITS,
                    flags: c.flags,
                    // An alias past the table (a damaged image) reads as an empty name, which a restore refuses.
                    alias: (c.alias != SENTINEL).then(|| self.kind_aliases.get(usize::from(c.alias)).cloned().unwrap_or_default()),
                    // A count past the slots (a damaged image) reads as that many classes, which a restore refuses.
                    classes: c.classes.get(..usize::from(c.class_count)).map_or_else(|| vec![0; usize::from(c.class_count)], <[u16]>::to_vec),
                })
                .collect(),
        )
    }

    /// Its columns, as format 15's reader reads them.
    pub fn columns(&self) -> F15Columns<'_> {
        F15Columns {
            code: &self.code[..2 * self.node_count()],
            size: &self.size,
            base: &self.base,
            off: &self.off,
            tail: &self.tail,
            widths: [self.widths[0], self.widths[1], self.widths[2] & !TAIL_FROM_START],
            from_start: self.widths[2] & TAIL_FROM_START != 0,
        }
    }
}

/// The run says once a language that its documents keep format 15, and why.
fn keeps_15(image: &TapeImage, tag: u8, why: &str) {
    static SAID: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];
    if !SAID[usize::from(tag)].swap(true, Ordering::Relaxed) {
        eprintln!("rerail: RERAIL_TAPE asks for format {}; {:?} documents are read as format 15: {why}", Format::of_run().name(), image.language);
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
            keeps_15(&self, tag, "its language has no format 16 table");
            return self;
        };
        let table = c.table.get_or_init(|| c.code_table());
        let number = match f16::numbers(lang, table) {
            Ok(number) => number,
            Err(e) => {
                keeps_15(&self, tag, &e);
                return self;
            }
        };
        let mut t = f16::write_from(lang, &F15::over(c.columns(), table), &number, self.source_len as u64);
        t.set_tag(tag);
        let held = match format {
            Format::F16 => Held::F16(Arc::new(t)),
            _ => Held::F16Z(Arc::new(t.blocks(lang))),
        };
        Self { held, ..self }
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
    image: Arc<TapeImage>,
    other: [OnceLock<TapeImage>; 3],
}

impl DocTape {
    pub fn new(image: Arc<TapeImage>) -> Self {
        DocTape { image, other: Default::default() }
    }

    pub fn image(&self) -> &TapeImage {
        &self.image
    }

    /// The bytes it holds.
    pub fn bytes(&self) -> usize {
        self.image.estimated_bytes() + self.other.iter().filter_map(OnceLock::get).map(TapeImage::estimated_bytes).sum::<usize>()
    }

    /// The tape, read in place in the format it is held in.
    pub fn tape(&self) -> AnyTape<'_> {
        self.image.tape()
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
