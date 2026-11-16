//! Decision 0293: a document's tape through the one API (`rerail_tape_format::tape`), in the format the run reads
//! (`RERAIL_TAPE`), read in place:
//!
//! - **15** (the default): the image's own columns;
//! - **16**: format 16, written from them once an image (0293's interim: until the parsers write it, and until
//!   the store keeps it beside format 15);
//! - **16z**: format 16 in fixed LZ4 blocks, a block decoded where a read falls in it.
//!
//! A language without format 16's table (`f16::lang`), or an image with a code the table lacks, is read as format
//! 15, and the run says so once a language.

use super::{SENTINEL, TAIL_FROM_START, TapeDoc, TapeImage};
use rerail_tape_format::f16::{self, Lang, Tape16, Tape16Z};
use rerail_tape_format::tape::{AnyTape, CodeTable, Entry, F15, F15Columns};
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};

/// The format a run reads its documents' tapes in.
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
}

/// A document's tape where its image is held: format 15's columns, and each format a run asks for made from them
/// once (the image's code table as the API's, and format 16 as written; none where it is read as format 15). The
/// document (`TapeDoc`) and its facts (`Facts::tape`) hold the same one.
pub struct DocTape {
    image: Arc<TapeImage>,
    table: OnceLock<CodeTable>,
    f16: OnceLock<Option<(&'static Lang, Tape16)>>,
    f16z: OnceLock<Option<(&'static Lang, Tape16Z)>>,
}

impl TapeImage {
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
                    alias: (c.alias != SENTINEL).then(|| self.kind_aliases[usize::from(c.alias)].clone()),
                    classes: c.classes().to_vec(),
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

/// Format 16 written from `image` (format 15, read through `table`), or why it is not: the run says so once a
/// language.
fn written(image: &TapeImage, table: &CodeTable) -> Option<(&'static Lang, Tape16)> {
    static SAID: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];
    let tag = crate::code_rule_dsl::language::tape_tag(image.language)?;
    let why = match f16::lang(tag) {
        None => "its language has no format 16 table".to_owned(),
        Some(lang) => match f16::numbers(lang, table) {
            Ok(number) => return Some((lang, f16::write_from(lang, &F15::over(image.columns(), table), &number, image.source_len as u64))),
            Err(e) => e,
        },
    };
    if !SAID[usize::from(tag)].swap(true, Ordering::Relaxed) {
        eprintln!("rerail: RERAIL_TAPE asks for format 16; {:?} documents are read as format 15: {why}", image.language);
    }
    None
}

impl DocTape {
    pub fn new(image: Arc<TapeImage>) -> Self {
        DocTape { image, table: OnceLock::new(), f16: OnceLock::new(), f16z: OnceLock::new() }
    }

    pub fn image(&self) -> &TapeImage {
        &self.image
    }

    /// The bytes it holds: the image's columns and code table, and each format made from them.
    pub fn bytes(&self) -> usize {
        let image = &*self.image;
        let columns = [&image.code, &image.size, &image.base, &image.off, &image.tail].iter().map(|c| c.len()).sum::<usize>();
        let made = self.f16.get().and_then(Option::as_ref).map_or(0, |(_, t)| t.bytes()) + self.f16z.get().and_then(Option::as_ref).map_or(0, |(_, t)| t.bytes());
        columns + image.codes.len() * std::mem::size_of::<super::Code>() + made
    }

    /// The tape, read in place in the run's format (`RERAIL_TAPE`, decision 0293). One a pass: a 16z tape keeps the
    /// blocks it decoded.
    pub fn tape(&self) -> AnyTape<'_> {
        self.tape_in(Format::of_run())
    }

    /// The tape in `format`, or format 15 where the language has no format 16.
    pub fn tape_in(&self, format: Format) -> AnyTape<'_> {
        let image = &*self.image;
        let table = self.table.get_or_init(|| image.code_table());
        let held = match format {
            Format::F15 => None,
            Format::F16 => self.f16.get_or_init(|| written(image, table)).as_ref().map(|(lang, t)| AnyTape::F16(t.reader(lang).expect("written with its language"))),
            Format::F16Z => self
                .f16z
                .get_or_init(|| written(image, table).map(|(lang, t)| (lang, t.blocks(lang))))
                .as_ref()
                .map(|(lang, t)| AnyTape::F16Z(t.reader(lang).expect("written with its language"))),
        };
        held.unwrap_or_else(|| AnyTape::F15(F15::over(image.columns(), table)))
    }
}

impl TapeDoc {
    /// This document's tape, read in place in the run's format (`DocTape::tape`).
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
