//! Format 16 (decision 0156 §1), written and read in place (decision 0293), block-major and written in one pass
//! (decision 0434). Ported from the lab (`native/lab/tape16`: `f16`, `f16w`, `code8`, `repeat`), which measured it.
//!
//! A tape is five arrays over a document's nodes in preorder, at widths fixed for the language, in blocks of `Z`
//! nodes at a fixed stride (each block its own five columns, the last block at its own rows):
//!
//! - **codes**: a byte a node, the code's number in the language's table, numbered most frequent first. Below the
//!   cutoff the byte is the number; past it, a prefix byte and a low part in the block's side list;
//! - **slots**: an inner node's `size << 1`; a leaf's `distance << 2 | 1` to the next node's previous sibling, or,
//!   on every `K`-th sibling, `distance << 2 | 3` to the next node's parent;
//! - **gaps**: each start less the previous node's; **lengths**: each end less its start;
//! - **checkpoints**: the absolute start of every `K`-th node, at the document's width.
//!
//! A value too wide for its array is the all-ones value, and its value is in the side list of its 256-node block.
//! Each array's side list is kept in parts of `PART` nodes (one a block), after the blocks, with each part's start.

use crate::reads::f16 as reads;
use crate::tape::{CodeSet, CodeTable, Entry, Tape};
use rerail_clif::view::TapeView;
use std::collections::HashMap;

/// Every `K`-th sibling names its parent; every `K`-th node's start is a checkpoint.
pub const K: usize = 8;
/// Nodes a side-list block covers (an escape's offset is a byte).
pub const BLOCK: usize = 256;
/// Nodes a side-list part covers: one fixed block at rest.
pub const PART: usize = 1024;

/// A language's fixed choices: its code table by number, the cutoff, the slot, gap and length widths, and the
/// revision its tapes are marked with (0156 §1.0).
pub struct Lang {
    table: CodeTable,
    index: HashMap<Entry, u32>,
    /// Numbers below `cut` are their byte; past it a prefix byte and a low part.
    cut: u32,
    /// Numbers each prefix byte covers, and the low part's bytes.
    per: u32,
    low_w: usize,
    /// Slots, gaps, lengths, in bytes.
    widths: [usize; 3],
    revision: u8,
    /// Each number's kind and field: what a presence's bits are (decision 0434 §4).
    kind_field: Vec<(u16, u16)>,
}

impl Lang {
    /// `table` by number (most frequent first), the widths in bytes, and the revision (1–254).
    pub fn new(table: CodeTable, widths: [usize; 3], revision: u8) -> Result<Self, String> {
        if revision == 0 || revision == 255 {
            return Err(format!("revision {revision} is not a tape revision"));
        }
        if widths.iter().any(|&w| !(1..=8).contains(&w)) {
            return Err(format!("widths {widths:?} out of range"));
        }
        let n = table.len() as u32;
        let cut = if n <= 256 { 256 } else { 255 };
        let per = if cut >= 256 { 1 } else { n - cut };
        let index = table.entries().iter().enumerate().map(|(k, e)| (e.clone(), k as u32)).collect();
        let kind_field = table.entries().iter().map(|e| (e.kind, e.field)).collect();
        Ok(Lang { table, index, cut, per, low_w: if per <= 256 { 1 } else { 2 }, widths, revision, kind_field })
    }
    /// The words of a presence's kinds' and fields' bitmaps: one bit a kind and a field the table names.
    pub fn presence_words(&self) -> (usize, usize) {
        let kinds = self.kind_field.iter().map(|&(k, _)| usize::from(k) + 1).max().unwrap_or(0);
        let fields = self.kind_field.iter().map(|&(_, f)| usize::from(f) + 1).max().unwrap_or(0);
        (kinds.div_ceil(64), fields.div_ceil(64))
    }
    pub fn table(&self) -> &CodeTable {
        &self.table
    }
    /// `entry`'s number, if the table has it.
    pub fn number(&self, entry: &Entry) -> Option<u32> {
        self.index.get(entry).copied()
    }
    /// What a reader elsewhere (the device's, `device_tape.h`) needs of the language to read its tapes in
    /// place: the cutoff, the numbers a prefix byte covers, and the slot, gap and length widths.
    pub fn device(&self) -> (u32, u32, [usize; 3]) {
        (self.cut, self.per, self.widths)
    }
    pub fn revision(&self) -> u8 {
        self.revision
    }

    /// The choices as text, as the lab writes them and the crate ships them (`tables/<tag>.tsv`): the revision, the
    /// widths, then each code in number order: kind, field, flags, alias (`-`, or `named:name`), classes (`-`, or
    /// ids joined by commas).
    pub fn to_text(&self, comment: &str) -> String {
        let mut out = String::new();
        for line in comment.lines() {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&format!("revision\t{}\nwidths\t{}\t{}\t{}\n", self.revision, self.widths[0], self.widths[1], self.widths[2]));
        for e in self.table.entries() {
            let alias = e.alias.as_ref().map_or("-".to_owned(), |(name, named)| format!("{}:{name}", u8::from(*named)));
            let classes = if e.classes.is_empty() { "-".to_owned() } else { e.classes.iter().map(u16::to_string).collect::<Vec<_>>().join(",") };
            out.push_str(&format!("code\t{}\t{}\t{}\t{alias}\t{classes}\n", e.kind, e.field, e.flags));
        }
        out
    }

    /// The choices from their text (`to_text`).
    pub fn from_text(text: &str) -> Result<Self, String> {
        let (mut revision, mut widths, mut entries) = (None, None, Vec::new());
        for (k, line) in text.lines().enumerate() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = || format!("line {}: {line}", k + 1);
            let c: Vec<&str> = line.split('\t').collect();
            let num = |s: &str| s.parse::<u64>().map_err(|_| bad());
            match c[0] {
                "revision" if c.len() == 2 => revision = Some(num(c[1])? as u8),
                "widths" if c.len() == 4 => widths = Some([num(c[1])? as usize, num(c[2])? as usize, num(c[3])? as usize]),
                "code" if c.len() == 6 => {
                    let alias = match c[4] {
                        "-" => None,
                        a => {
                            let (named, name) = a.split_once(':').ok_or_else(bad)?;
                            Some((name.to_owned(), named == "1"))
                        }
                    };
                    let classes = if c[5] == "-" { Vec::new() } else { c[5].split(',').map(|x| x.parse::<u16>().map_err(|_| bad())).collect::<Result<_, _>>()? };
                    entries.push(Entry { kind: num(c[1])? as u16, field: num(c[2])? as u16, flags: num(c[3])? as u8, alias, classes });
                }
                _ => return Err(bad()),
            }
        }
        Lang::new(CodeTable::new(entries), widths.ok_or("no widths")?, revision.ok_or("no revision")?)
    }
}

/// The choices format 16 ships for the language of tape tag `tag` (`tables/<tag>.tsv`, written by the lab's
/// `tape16 table`), read once; none for a language without them, whose documents keep format 15 (0293 §3).
pub fn lang(tag: u8) -> Option<&'static Lang> {
    static LANGS: [std::sync::OnceLock<Option<Lang>>; 256] = [const { std::sync::OnceLock::new() }; 256];
    LANGS[usize::from(tag)]
        .get_or_init(|| {
            let text = tables::text(tag)?;
            Some(Lang::from_text(text).unwrap_or_else(|e| panic!("format 16's table for tag {tag}: {e}")))
        })
        .as_ref()
}

/// The shipped tables' text, by tape tag.
mod tables {
    include!("../tables/index.rs");
}

/// A stored format 16 tape's magic (its first 6 bytes are every stored tape's: the store reads the language tag
/// after them, `LANGUAGE`). Block-major (decision 0434).
pub const MAGIC16: &[u8; 8] = b"TMPTAPB6";
/// A stored 16z tape's magic.
pub const MAGIC16Z: &[u8; 8] = b"TMPTAPBZ";

/// Where a stored format 16 or 16z tape's header holds each value. A 16z tape's header is the header of the format
/// 16 tape it decodes to, but for its magic and its own fields (`Z`).
mod head {
    /// The language's tape tag, a byte; then the revision mark and the checkpoints' width.
    pub const LANGUAGE: usize = 8;
    pub const MARK: usize = 9;
    pub const CW: usize = 10;
    /// The source's newlines, u32: its newline index's count (`newlines`).
    pub const NEWLINES: usize = 12;
    /// The source's length, u64; the node count, u64.
    /// Where format 15's header states it too (`header::SOURCE_LEN`): a reader of either finds it there.
    pub const SOURCE_LEN: usize = crate::header::SOURCE_LEN;
    pub const NODES: usize = 24;
    /// The side lists' value widths, a byte each; then the slot, gap and length widths.
    pub const VW: usize = 32;
    pub const WIDTHS: usize = 36;
    /// The blocks' stride, then block 0's start, u32s.
    pub const STRIDE: usize = 40;
    pub const BLOCKS: usize = 44;
    /// The tail's starts, u32s: the four side lists, their part starts (an array a list), the newline index, and
    /// the tape's end.
    pub const TAIL: usize = 48;
    /// A 16z tape's own, u32s (0 in a format 16 tape): where its part starts, checkpoints, newline index, piece
    /// starts, piece lengths and piece kinds lie, and its end. Its payload follows its header.
    pub const Z: usize = 88;
}
/// The tail's starts and a 16z tape's own fields (`head::TAIL`, `head::Z`).
const TAILS: usize = 10;
const ZS: usize = 7;
/// A format 16 header's length: block 0 starts here.
const HEAD16: usize = (head::Z + 4 * ZS).next_multiple_of(8);
/// The most nodes a tape holds: a slot (`4 n + 3`) fits a u32, and so does every read's node index.
const MAX_NODES: usize = 1 << 30;
/// A block's columns: codes, slots, gaps, lengths, checkpoints.
#[cfg(any(test, feature = "reference"))]
const COLUMNS: usize = 5;

fn u64_at(b: &[u8], at: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(b.get(at..at + 8).ok_or("a short tape header")?.try_into().expect("8 bytes")))
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

#[cfg(any(test, feature = "reference"))]
fn put_u32(b: &mut [u8], at: usize, v: usize) -> Result<(), String> {
    let v = u32::try_from(v).map_err(|_| "a format 16 tape past 4 GiB: its header's offsets are u32s".to_owned())?;
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    Ok(())
}

/// The bytes a block's row holds before column `r` (0 codes, 1 slots, 2 gaps, 3 lengths, 4 checkpoints): a block of
/// R rows holds column r at `R * before(r)` from its start.
fn before(w: [usize; 3], r: usize) -> usize {
    [0, 1, 1 + w[0], 1 + w[0] + w[1], 1 + w[0] + w[1] + w[2]][r]
}

/// Column `r`'s cell width, the checkpoints' a `K`-th row's.
fn cell(w: [usize; 3], cw: usize, r: usize) -> usize {
    [1, w[0], w[1], w[2], cw][r]
}

/// A full block's bytes: `Z` rows and `Z / K` checkpoints.
fn stride_of(w: [usize; 3], cw: usize) -> usize {
    Z * before(w, 4) + Z / K * cw
}

/// One document's format 16 tape, held as stored (`as_bytes`) and read where it lies (decision 0434): the header,
/// blocks of `Z` nodes at a fixed stride (each its codes, slots, gaps, lengths and checkpoints, the last block at
/// its own rows), then the tail: the four side lists (codes' low parts, then escaped slots, gaps, lengths), their
/// part starts, and the newline index. The last block, each side list and the newline index are followed by 8 bytes
/// of room for a word's load. It is read through the tape's reads (decision 0310) over its view, which it keeps but
/// for its language's choices (`reader`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tape16 {
    pub mark: u8,
    p: Placement,
    source_len: u64,
    /// As stored: its own, or placed in a slab's room (decision 0353 step 4a).
    buf: crate::home::Home,
    /// Its view without its language's choices (`Placement::frame`), or none for a tape or a source past 4 GiB,
    /// which the reads' u32 offsets do not reach.
    frame: Option<TapeView>,
}

impl Tape16 {
    pub fn len(&self) -> usize {
        self.p.n
    }
    pub fn is_empty(&self) -> bool {
        self.p.n == 0
    }
    /// The bytes it takes in memory: its home's (the tape, and its presence after it).
    pub fn bytes(&self) -> usize {
        self.buf.len()
    }
    /// The source's length.
    pub fn source_len(&self) -> u64 {
        self.source_len
    }
    /// Its home's bytes: the tape (`as_bytes`), then its presence where it has one (`presence::at`). What a reader's
    /// offsets count from on the host, and what a room holds.
    pub fn extent(&self) -> &[u8] {
        &self.buf
    }
    /// Its presence's bytes in its home (`presence`), if it has one.
    pub fn presence_bytes(&self) -> Option<&[u8]> {
        (self.p.presence != 0).then(|| &self.buf[self.p.presence..])
    }
    /// The language tag its header names (`set_tag`).
    pub fn tag(&self) -> u8 {
        self.buf[head::LANGUAGE]
    }
    /// Names the language's tape tag in its header.
    pub fn set_tag(&mut self, tag: u8) {
        self.buf.own_mut()[head::LANGUAGE] = tag;
    }
    /// The tape as stored, without its presence.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.p.end]
    }
    /// Where its bytes are.
    pub fn home(&self) -> &crate::home::Home {
        &self.buf
    }
    /// Where its parts lie.
    pub fn placement(&self) -> &Placement {
        &self.p
    }

    /// The stored tape, held as it is: its header and part starts are checked against its length.
    pub fn from_bytes(buf: impl Into<crate::home::Home>) -> Result<Self, String> {
        let buf: crate::home::Home = buf.into();
        let p = placement(&buf)?;
        let source_len = u64_at(&buf, head::SOURCE_LEN)?;
        Ok(Tape16 { mark: buf[head::MARK], p, source_len, frame: p.frame_of(source_len), buf })
    }

    /// Part `j`'s start in side list `k` (one more at the end).
    #[cfg(any(test, feature = "reference"))]
    fn part_start(&self, k: usize, j: usize) -> usize {
        u32_at(&self.buf, self.p.parts[k] + 4 * j) as usize
    }

    /// Block `b`'s column `r` (`Placement::column`) and its bytes.
    #[cfg(any(test, feature = "reference"))]
    fn column(&self, b: usize, r: usize) -> &[u8] {
        let at = self.p.column(b, r);
        let rows = self.p.rows(b);
        let len = if r == 4 { rows.div_ceil(K) * self.p.cw } else { rows * cell(self.p.widths, self.p.cw, r) };
        &self.buf[at..at + len]
    }

    /// Read with `lang`'s choices: the tape's mark is `lang`'s revision and its widths are `lang`'s, else it is
    /// rewritten before it is read.
    pub fn reader<'a>(&'a self, lang: &'a Lang) -> Result<F16<'a>, String> {
        if self.mark != lang.revision || self.p.widths != lang.widths {
            return Err(format!("a tape of revision {} (widths {:?}), read with revision {} ({:?})", self.mark, self.p.widths, lang.revision, lang.widths));
        }
        let frame = self.frame.ok_or("a format 16 tape whose source is past 4 GiB: the reads' offsets do not reach it")?;
        presence::of_lang(self.presence_bytes(), lang)?;
        Ok(F16 { bytes: &self.buf, view: bind(frame, lang)?, lang })
    }
}

/// Where a format 16 tape's parts lie, as its header says, for a reader that reads them there without this crate
/// (the device's, `device_tape.h`): block 0's start and the blocks' stride (`column`), each side list's start, each
/// one's part starts (u32s, a part a block and one more at the end), the newline index, the end; the node count, the
/// checkpoints' width, the side lists' value widths and the slot, gap and length widths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub n: usize,
    /// The source's length its header states: a view's spans are held to it (decision 0463).
    pub source_len: u64,
    pub cw: usize,
    pub widths: [usize; 3],
    pub vw: [usize; 4],
    /// The source's newlines (it has one line more).
    pub newlines: u32,
    pub blocks: usize,
    pub stride: usize,
    pub side: [usize; 4],
    pub parts: [usize; 4],
    pub lines: usize,
    pub end: usize,
    /// Where the tape's presence lies in its home, after the tape (`presence::at`); 0 where the home holds the tape
    /// alone. Not the tape's layout: `fits` compares the layout without it.
    pub presence: usize,
}

impl Placement {
    /// The placement of a tape of `n` nodes whose block 0 starts at `blocks`, with these widths, side lists `sides`
    /// bytes long and a newline index `lines` bytes long of `newlines`, of a source `source_len` long.
    #[allow(clippy::too_many_arguments)]
    fn of(blocks: usize, n: usize, cw: usize, widths: [usize; 3], vw: [usize; 4], sides: [usize; 4], lines: usize, newlines: u32, source_len: u64) -> Self {
        let mut p = Placement { n, source_len, cw, widths, vw, newlines, blocks, stride: stride_of(widths, cw), side: [0; 4], parts: [0; 4], lines: 0, end: 0, presence: 0 };
        let count = p.block_count();
        let mut at = p.blocks_end() + 8;
        for (k, len) in sides.iter().enumerate() {
            p.side[k] = at;
            at += len + 8;
        }
        at = at.next_multiple_of(4);
        for k in 0..4 {
            p.parts[k] = at;
            at += 4 * (count + 1);
        }
        p.lines = at;
        p.end = at + lines + 8;
        p
    }

    /// The blocks: `Z` nodes each, the last fewer.
    pub fn block_count(&self) -> usize {
        self.n.div_ceil(Z)
    }
    /// Block `b`'s rows: `Z`, or the last block's nodes.
    pub fn rows(&self, b: usize) -> usize {
        Z.min(self.n - b * Z)
    }
    /// Where block `b`'s column `r` (0 codes, 1 slots, 2 gaps, 3 lengths, 4 checkpoints) starts.
    pub fn column(&self, b: usize, r: usize) -> usize {
        self.blocks + b * self.stride + self.rows(b) * before(self.widths, r)
    }
    /// One past the last block's columns.
    fn blocks_end(&self) -> usize {
        match self.block_count() {
            0 => self.blocks,
            count => self.column(count - 1, 4) + self.rows(count - 1).div_ceil(K) * self.cw,
        }
    }

    /// The view a tape's reads take (decision 0310), the tape at byte `base` of what the view's offsets count from
    /// (0: the tape's own bytes).
    pub fn view(&self, lang: &Lang, base: usize) -> Result<TapeView, String> {
        // Each block's columns at `lang`'s widths: the tape's own, which its stride and blocks were checked against
        // when it was read in (`header`, `fits`), so a read of a node of the tape lies in its block (decision 0463;
        // its walks give only nodes of the tape).
        let (_, _, w) = lang.device();
        if w != self.widths {
            return Err(format!("a format 16 tape of widths {:?}, read with widths {w:?}", self.widths));
        }
        bind(self.frame(base)?, lang)
    }

    /// The view but for its language's choices (`bind` adds them): the tape's own widths, and where its parts lie.
    pub fn frame(&self, base: usize) -> Result<TapeView, String> {
        let at = |offset: usize| u32::try_from(base + offset).map_err(|_| "a tape placed past 4 GiB".to_owned());
        let mut widths = 0;
        for (k, w) in [self.cw, self.vw[0], self.vw[1], self.vw[2], self.vw[3]].into_iter().enumerate() {
            widths |= nibble(w)? << (12 + 4 * k);
        }
        let row = |r: usize| before(self.widths, r) as u32;
        Ok(TapeView {
            nodes: u32::try_from(self.n).map_err(|_| "a tape past 4 G nodes".to_owned())?,
            source_len: u32::try_from(self.source_len).map_err(|_| "a source past 4 GiB".to_owned())?,
            format: 16,
            widths,
            columns: [at(self.blocks)?, row(1), row(2), row(3), row(4)],
            stride: u32::try_from(self.stride).map_err(|_| "a stride past 4 GiB".to_owned())?,
            side: [at(self.side[0])?, at(self.side[1])?, at(self.side[2])?, at(self.side[3])?],
            parts: [at(self.parts[0])?, at(self.parts[1])?, at(self.parts[2])?, at(self.parts[3])?],
            lines: at(self.lines)?,
            line_count: self.newlines.checked_add(1).ok_or("a source past 4 G lines")?,
            presence: if self.presence == 0 { 0 } else { at(self.presence)? },
            ..Default::default()
        })
    }

    /// The frame of a tape of a source `source_len` long, at its own bytes, if the reads reach both.
    fn frame_of(&self, source_len: u64) -> Option<TapeView> {
        self.frame(0).ok().filter(|_| u32::try_from(source_len).is_ok())
    }

    /// The header of a tape placed so (`magic` its kind), its 16z fields 0, into `buf`'s first `HEAD16` bytes.
    #[cfg(any(test, feature = "reference"))]
    fn head_into(&self, buf: &mut [u8], magic: &[u8; 8], tag: u8, mark: u8, source_len: u64) -> Result<(), String> {
        buf[..HEAD16].fill(0);
        buf[..8].copy_from_slice(magic);
        buf[head::LANGUAGE] = tag;
        buf[head::MARK] = mark;
        buf[head::CW] = self.cw as u8;
        buf[head::NEWLINES..head::NEWLINES + 4].copy_from_slice(&self.newlines.to_le_bytes());
        buf[head::SOURCE_LEN..head::SOURCE_LEN + 8].copy_from_slice(&source_len.to_le_bytes());
        buf[head::NODES..head::NODES + 8].copy_from_slice(&(self.n as u64).to_le_bytes());
        for k in 0..4 {
            buf[head::VW + k] = self.vw[k] as u8;
        }
        for k in 0..3 {
            buf[head::WIDTHS + k] = self.widths[k] as u8;
        }
        put_u32(buf, head::STRIDE, self.stride)?;
        put_u32(buf, head::BLOCKS, self.blocks)?;
        let tail = [self.side[0], self.side[1], self.side[2], self.side[3], self.parts[0], self.parts[1], self.parts[2], self.parts[3], self.lines, self.end];
        for (t, &at) in tail.iter().enumerate() {
            put_u32(buf, head::TAIL + 4 * t, at)?;
        }
        Ok(())
    }
}

/// A width as a view's nibble.
fn nibble(w: usize) -> Result<u32, String> {
    if w <= 15 { Ok(w as u32) } else { Err("a width past a nibble".to_owned()) }
}

/// `frame` with `lang`'s choices: the cutoff, the numbers a prefix byte covers, the slot, gap and length widths.
fn bind(mut frame: TapeView, lang: &Lang) -> Result<TapeView, String> {
    let (cut, per, w) = lang.device();
    frame.widths |= nibble(w[0])? | nibble(w[1])? << 4 | nibble(w[2])? << 8;
    (frame.cut, frame.per) = (cut, per);
    Ok(frame)
}

/// The placement a format 16 header says (a 16z tape's: of the tape it decodes to), its values in range. That it
/// is the layout its lengths give is checked with its part starts (`fits`).
fn header(buf: &[u8]) -> Result<Placement, String> {
    if buf.len() < HEAD16 {
        return Err("a short tape header".into());
    }
    let n = usize::try_from(u64_at(buf, head::NODES)?).ok().filter(|&n| n < MAX_NODES).ok_or("a tape of too many nodes")?;
    let byte = |at: usize| usize::from(buf[at]);
    let cw = byte(head::CW);
    let vw: [usize; 4] = std::array::from_fn(|k| byte(head::VW + k));
    let widths: [usize; 3] = std::array::from_fn(|k| byte(head::WIDTHS + k));
    if cw > 8 || vw.iter().any(|&w| w > 8) || widths.iter().any(|&w| !(1..=8).contains(&w)) {
        return Err("a format 16 tape's widths are out of range".into());
    }
    let word = |at: usize| u32_at(buf, at) as usize;
    let tail: [usize; TAILS] = std::array::from_fn(|t| word(head::TAIL + 4 * t));
    let p = Placement {
        n,
        source_len: u64_at(buf, head::SOURCE_LEN)?,
        cw,
        widths,
        vw,
        newlines: u32_at(buf, head::NEWLINES),
        blocks: word(head::BLOCKS),
        stride: word(head::STRIDE),
        side: [tail[0], tail[1], tail[2], tail[3]],
        parts: [tail[4], tail[5], tail[6], tail[7]],
        lines: tail[8],
        end: tail[9],
        presence: 0,
    };
    if p.blocks != HEAD16 || p.stride != stride_of(widths, cw) || p.end < p.lines + 8 {
        return Err("a format 16 header that does not fit itself".into());
    }
    Ok(p)
}

/// The side lists' lengths, from their part starts in `buf` at `parts` (each from 0, in order, the last its
/// length), for `count` blocks.
fn side_lengths(buf: &[u8], parts: [usize; 4], count: usize) -> Result<[usize; 4], String> {
    let mut lens = [0; 4];
    for k in 0..4 {
        let end = parts[k].checked_add(4 * (count + 1)).filter(|&e| e <= buf.len()).ok_or("a short tape: its part starts")?;
        let b = |j: usize| u32_at(buf, parts[k] + 4 * j);
        if b(0) != 0 || (1..=count).any(|j| b(j - 1) > b(j)) {
            return Err(format!("side list {k}'s part starts do not fit it"));
        }
        debug_assert!(end <= buf.len());
        lens[k] = b(count) as usize;
    }
    Ok(lens)
}

/// `p` is the layout its own lengths give: the side lists' (from their part starts, in `buf` at `parts`) and the
/// newline index's.
fn fits(p: &Placement, buf: &[u8], parts: [usize; 4]) -> Result<[usize; 4], String> {
    let sides = side_lengths(buf, parts, p.block_count())?;
    let mut laid = Placement::of(p.blocks, p.n, p.cw, p.widths, p.vw, sides, p.end - 8 - p.lines, p.newlines, p.source_len);
    laid.presence = p.presence;
    if laid != *p {
        return Err("a format 16 header that does not fit its parts".into());
    }
    Ok(sides)
}

/// The placement of the stored format 16 tape `buf`, its header and part starts checked against its length as
/// `Tape16::from_bytes` checks them, without copying a byte.
pub fn placement(buf: &[u8]) -> Result<Placement, String> {
    if buf.len() < HEAD16 || &buf[..8] != MAGIC16 {
        return Err("not a format 16 tape".into());
    }
    let mut p = header(buf)?;
    if buf.len() < p.end {
        return Err(format!("a format 16 tape of {} bytes, its header says {}", buf.len(), p.end));
    }
    p.presence = presence::find(buf, p.end, p.block_count())?;
    if buf[head::Z..HEAD16].iter().any(|&b| b != 0) {
        return Err("a format 16 tape with a 16z tape's fields".into());
    }
    fits(&p, buf, p.parts)?;
    check_lines(&buf[p.lines..p.end - 8], &p, u64_at(buf, head::SOURCE_LEN)?)?;
    Ok(p)
}

/// The newline index `lines` fits its source and holds the header's count of newlines.
fn check_lines(lines: &[u8], p: &Placement, source_len: u64) -> Result<(), String> {
    crate::newlines::check(lines, source_len)?;
    if crate::newlines::count(lines) != Some(p.newlines) {
        return Err("a newline index whose count is not its header's".into());
    }
    Ok(())
}

/// The width in bytes a value up to `max` takes (at least 1).
fn bytes_for(max: u64) -> usize {
    (((64 - max.leading_zeros()) as usize).div_ceil(8)).max(1)
}

#[inline(always)]
fn mask(w: usize) -> u64 {
    if w >= 8 { u64::MAX } else { (1u64 << (8 * w)) - 1 }
}

// ---- the writer ------------------------------------------------------------------------------------------

/// The writer whose ops are the CLIF ones (decision 0434 §6).
pub mod ops;
#[cfg(any(test, feature = "reference"))]
pub mod reference;

/// The writer: the ops in CLIF, through their Rust (decision 0434 §6, R16).
pub use ops::{Document, Writer};

/// A node's cells as `write` takes them: its code's number in the language's table, its span, and one past its
/// subtree's last node.
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub number: u32,
    pub start: u64,
    pub end: u64,
    pub subtree_end: usize,
}

/// A tape the writer made: format 16, or 16z.
pub enum Written {
    F16(Tape16),
    F16Z(Tape16Z),
}

/// A stored format 16 or 16z tape's presence, read in place in its stored bytes (`buf`, its home as stored: the tape,
/// then its presence) by the tape's read (`tape_has_kind`), through its view's `presence` (no other read of the tape
/// is asked here). What a planner asks of a tape it has not loaded.
pub struct StoredPresence<'a> {
    buf: &'a [u8],
    view: TapeView,
}

impl<'a> StoredPresence<'a> {
    /// The presence `buf` holds after its tape, checked to fill it, of `lang`'s kinds and fields; `None` where it holds
    /// none (format 15, or a tape stored without one) or is no format 16 or 16z tape's home.
    pub fn of(buf: &'a [u8], lang: &Lang) -> Option<Self> {
        let p = header(buf).ok()?;
        let end = if buf.starts_with(MAGIC16) {
            p.end
        } else if buf.starts_with(MAGIC16Z) {
            u32_at(buf, head::Z + 4 * ZEND) as usize
        } else {
            return None;
        };
        if end > buf.len() {
            return None;
        }
        let at = presence::find(buf, end, p.block_count()).ok().filter(|&at| at != 0)?;
        presence::of_lang(Some(&buf[at..]), lang).ok()?;
        Some(StoredPresence { buf, view: TapeView { presence: u32::try_from(at).ok()?, ..Default::default() } })
    }
    /// `Tape::has_kind`: block `block` (`u32::MAX`: the document) may have a node of kind `kind`.
    pub fn has_kind(&self, block: u32, kind: u16) -> bool {
        reads::has_kind(self.buf, &self.view, block, u32::from(kind)) != 0
    }
}

/// What a writer's `finish` gives: the tape, its presence laid in its home after it (decision 0434 §4, step 6).
pub struct Finished {
    pub tape: Written,
}

impl Written {
    /// Its home's bytes: the tape, then its presence.
    /// Its bytes, where it owns them (a writer's tape, not a placed one): for the writer to write the next tape in
    /// (`ops::Writer::recycle`).
    pub fn into_own(self) -> Option<Vec<u8>> {
        let home = match self {
            Written::F16(t) => t.buf,
            Written::F16Z(t) => t.buf,
        };
        match home {
            crate::home::Home::Own(v) => Some(v),
            crate::home::Home::Placed(_) => None,
        }
    }
    pub fn extent(&self) -> &[u8] {
        match self {
            Written::F16(t) => t.extent(),
            Written::F16Z(t) => t.extent(),
        }
    }
    /// Its presence's bytes in its home, if it has one.
    pub fn presence_bytes(&self) -> Option<&[u8]> {
        match self {
            Written::F16(t) => t.presence_bytes(),
            Written::F16Z(t) => t.presence_bytes(),
        }
    }
    /// Its presence, read out of its home: a test's oracle (the engine reads it in place, `tape_has_kind`).
    #[cfg(any(test, feature = "reference"))]
    pub fn presence(&self) -> Presence {
        self.presence_bytes().map(presence::read).unwrap_or_default()
    }
}

/// A tape's presence in its home (decision 0434 §4, step 6): after the tape, at its end rounded up to 8 bytes, a
/// header of four u32s (its kinds' words, its fields' words, its presences, 0), then the document's words (kinds',
/// then fields') and each block's after them. Not the tape's layout: the tape's reads never reach it, and its stored
/// bytes (`as_bytes`) end before it. Read in place by `tape_has_kind` (decision 0310), on the host and the device.
pub mod presence {
    use super::{Presence, u32_at};

    /// The header's bytes.
    pub const HEAD: usize = 16;

    /// Where a tape `end` bytes long lays its presence.
    pub fn at(end: usize) -> usize {
        end.next_multiple_of(8)
    }

    /// `p`'s bytes, laid after the tape in `out` (`out.len()` its end): the writer's last write, from what it
    /// accumulated.
    pub fn lay(out: &mut Vec<u8>, p: &Presence) {
        let words = p.kind_words + p.field_words;
        let count = 1 + p.blocks.len().checked_div(words).unwrap_or(0);
        lay_words(out, (p.kind_words, p.field_words), count, &mut p.doc.iter().chain(&p.blocks).copied());
    }

    /// `count` presences of these words (`kind_words`, `field_words`) each, laid after the tape in `out`.
    pub fn lay_words(out: &mut Vec<u8>, (kind_words, field_words): (usize, usize), count: usize, words: &mut dyn Iterator<Item = u64>) {
        out.resize(at(out.len()), 0);
        for v in [kind_words, field_words, count, 0] {
            out.extend_from_slice(&(v as u32).to_le_bytes());
        }
        for w in words.take((kind_words + field_words) * count) {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }

    /// Where the presence in the home `buf` of a tape `end` bytes long and of `blocks` blocks lies, checked to fill
    /// the home past the tape: 0 where the home holds the tape alone. One presence, the document's, or one more than
    /// the blocks.
    pub(super) fn find(buf: &[u8], end: usize, blocks: usize) -> Result<usize, String> {
        if buf.len() == end {
            return Ok(0);
        }
        let at = at(end);
        let bad = || "a tape's presence does not fit its home".to_owned();
        if buf.len() < at + HEAD || buf[end..at].iter().any(|&b| b != 0) {
            return Err(bad());
        }
        let w = |k: usize| u32_at(buf, at + 4 * k) as usize;
        let (words, count) = (w(0) + w(1), w(2));
        if words == 0 || w(3) != 0 || (count != 1 && count != 1 + blocks) || buf.len() - at - HEAD != 8 * words * count {
            return Err(bad());
        }
        Ok(at)
    }

    /// The presence `bytes` (`find`'s) holds `lang`'s kinds' and fields' words (`Lang::presence_words`).
    pub(super) fn of_lang(bytes: Option<&[u8]>, lang: &super::Lang) -> Result<(), String> {
        match bytes {
            Some(b) if (u32_at(b, 0) as usize, u32_at(b, 4) as usize) != lang.presence_words() => Err("a tape's presence of another language's kinds and fields".into()),
            _ => Ok(()),
        }
    }

    /// The presence `bytes` (`find`'s) as a `Presence`: a test's oracle.
    #[cfg(any(test, feature = "reference"))]
    pub fn read(bytes: &[u8]) -> Presence {
        let w = |k: usize| u32_at(bytes, 4 * k) as usize;
        let word = |k: usize| u64::from_le_bytes(bytes[HEAD + 8 * k..HEAD + 8 * k + 8].try_into().expect("8 bytes"));
        let (kind_words, field_words, count) = (w(0), w(1), w(2));
        let words = kind_words + field_words;
        Presence { kind_words, field_words, doc: (0..words).map(word).collect(), blocks: (words..words * count).map(word).collect() }
    }
}

/// The kinds and fields a tape's nodes have, as bitmaps over its language's kinds and fields (`Lang::presence_words`):
/// the document's, and, where the writer was asked for them, each block's (decision 0434 §4). Not part of the tape:
/// the store keeps it beside the document, to leave out what an anchor cannot touch before the tape is read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presence {
    /// Words of the kinds' bitmap, then of the fields'.
    pub kind_words: usize,
    pub field_words: usize,
    /// The document's: its kinds' words, then its fields'.
    pub doc: Vec<u64>,
    /// Each block's, the same layout one after another; empty unless asked for.
    pub blocks: Vec<u64>,
}

impl Presence {
    fn words(&self) -> usize {
        self.kind_words + self.field_words
    }
    /// Whether the document has a node of kind `kind`.
    pub fn has_kind(&self, kind: u16) -> bool {
        bit(&self.doc[..self.kind_words], kind)
    }
    /// Whether the document has a node in field `field`.
    pub fn has_field(&self, field: u16) -> bool {
        bit(&self.doc[self.kind_words..], field)
    }
    /// Block `b`'s kinds' and fields' words, where blocks were asked for.
    pub fn block(&self, b: usize) -> Option<(&[u64], &[u64])> {
        let w = self.words();
        let words = self.blocks.get(b * w..(b + 1) * w)?;
        Some(words.split_at(self.kind_words))
    }
}

fn bit(words: &[u64], k: u16) -> bool {
    words.get(usize::from(k) / 64).is_some_and(|w| w >> (k % 64) & 1 != 0)
}

/// Format 16, or 16z where `z`, written over a document's `n` nodes in preorder by the writer's ops (`Writer`):
/// `node(i)` is node `i`'s cells, asked once each, in order; a node closes when the pass reaches its subtree's end.
/// `Err` where a node's values break the tape's bounds: no tape is made of it.
/// A tape writer's ops (decision 0434 §2): the writer's (`Writer`, the ops in CLIF), or the reference writer's.
pub(crate) trait WriterOps: Sized {
    fn open(&mut self, number: u32, start: u64) -> usize;
    fn close(&mut self, end: u64);
    // `leaf`, `retag` and `set_start`: the tests' (tier 2's, by R17).
    #[cfg_attr(not(test), allow(dead_code))]
    fn leaf(&mut self, number: u32, start: u64, end: u64) -> usize;
    fn commit(&mut self);
    #[cfg_attr(not(test), allow(dead_code))]
    fn retag(&mut self, i: usize, number: u32);
    #[cfg_attr(not(test), allow(dead_code))]
    fn set_start(&mut self, i: usize, start: u64);
    fn finish(self) -> Result<Finished, String>;
}

macro_rules! writer_ops {
    ($t:ty) => {
        writer_ops!($t, |w: $t| <$t>::finish(w));
    };
    // `finish` through `&mut`: a writer used again (`ops::Writer::begin`).
    ($t:ty, again) => {
        writer_ops!($t, |mut w: $t| <$t>::finish(&mut w));
    };
    ($t:ty, $finish:expr) => {
        impl WriterOps for $t {
            fn open(&mut self, number: u32, start: u64) -> usize {
                <$t>::open(self, number, start)
            }
            fn close(&mut self, end: u64) {
                <$t>::close(self, end)
            }
            fn leaf(&mut self, number: u32, start: u64, end: u64) -> usize {
                <$t>::leaf(self, number, start, end)
            }
            fn commit(&mut self) {
                <$t>::commit(self)
            }
            fn retag(&mut self, i: usize, number: u32) {
                <$t>::retag(self, i, number)
            }
            fn set_start(&mut self, i: usize, start: u64) {
                <$t>::set_start(self, i, start)
            }
            fn finish(self) -> Result<Finished, String> {
                ($finish)(self)
            }
        }
    };
}
writer_ops!(Writer, again);
#[cfg(any(test, feature = "reference"))]
writer_ops!(reference::Writer<'_>);
#[cfg(test)]
writer_ops!(ops::device::DeviceOps);

pub fn write(lang: &Lang, n: usize, source: &[u8], tag: u8, z: bool, node: impl FnMut(usize) -> Node) -> Result<Written, String> {
    write_with(Writer::new(lang, source, tag, z, false, n), n, node).map(|f| f.tape)
}

/// `write` by `w`'s ops.
pub(crate) fn write_with(mut w: impl WriterOps, n: usize, mut node: impl FnMut(usize) -> Node) -> Result<Finished, String> {
    // The open nodes' subtree ends and ends.
    let mut ends: Vec<(usize, u64)> = Vec::with_capacity(64);
    for i in 0..n {
        let x = node(i);
        while ends.last().is_some_and(|e| e.0 <= i) {
            w.close(ends.pop().expect("an open node").1);
        }
        if x.subtree_end <= i || x.subtree_end > n || ends.last().is_some_and(|e| x.subtree_end > e.0) {
            return Err(format!("node {i}'s cells break the tape's bounds"));
        }
        w.open(x.number, x.start);
        ends.push((x.subtree_end, x.end));
        w.commit();
    }
    while let Some((_, end)) = ends.pop() {
        w.close(end);
    }
    w.finish()
}

const NONE: u32 = u32::MAX;

/// Format 16 (or 16z where `z`) written from another tape `t`, each node's code mapped to the language's number
/// (`number`, by `t`'s code), in one pass (`write`): a tape a parser did not write in format 16 (decision 0293).
pub fn write_from<T: Tape>(lang: &Lang, t: &T, number: &[u32], source: &[u8], tag: u8, z: bool) -> Result<Written, String> {
    write(lang, t.len(), source, tag, z, nodes_of(t, number))
}

/// `write_from`, with the tape's presence, the document's and each block's (§4).
pub fn write_from_with_presence<T: Tape>(lang: &Lang, t: &T, number: &[u32], source: &[u8], tag: u8, z: bool) -> Result<Finished, String> {
    write_with(Writer::new(lang, source, tag, z, true, t.len()), t.len(), nodes_of(t, number))
}

/// `t`'s nodes in preorder, as `write` takes them.
pub(crate) fn nodes_of<'t, T: Tape>(t: &'t T, number: &'t [u32]) -> impl FnMut(usize) -> Node + 't {
    let mut s = 0;
    move |i| {
        s = t.start_next(i, s);
        Node { number: number[t.code(i) as usize], start: s, end: t.end_with_start(i, s), subtree_end: t.subtree_end(i) }
    }
}

/// Each of `table`'s codes as `lang`'s number: what `write_from` maps a tape's codes with. An error names a code the
/// language's table does not hold.
pub fn numbers(lang: &Lang, table: &CodeTable) -> Result<Vec<u32>, String> {
    table.entries().iter().map(|e| lang.number(e).ok_or_else(|| format!("code {e:?} is not in the language's table"))).collect()
}

// ---- the reader ------------------------------------------------------------------------------------------

/// The groups of up to `PART` nodes from `codes` (a part's code bytes) into `out`: bit j of group g set when node
/// `64 g + j` is in `set`. A byte below the cutoff is tested by the set's nibble tables, 64 at a time (`ByteSet`);
/// one past it is the prefix of a number whose low part is in the part's entries of side list 0, so when the set
/// holds a number past the cutoff the entries are walked in order (`part_codes`), each once, rather than searched
/// for a node at a time.
#[inline(always)]
fn part_groups(codes: &[u8], lang: &Lang, set: &CodeSet, byte: &ByteSet, tail: Option<(&[u8], &TapeView, usize)>, out: &mut [u64; PART / 64]) -> usize {
    let groups = codes.len().div_ceil(64);
    code_groups(codes, byte, out);
    if let Some((bytes, view, part)) = tail {
        let cut = lang.cut;
        part_codes(bytes, view, part, |local, low| {
            let number = cut + (u32::from(codes[local]) - cut) * lang.per + low;
            out[local / 64] |= u64::from(set.contains(number)) << (local % 64);
        });
    }
    groups
}

/// Each entry of part `part` of side list 0 (codes' low parts) of the tape in `bytes` read by `view`, in node
/// order: `f(node within the part, low part)`. The layout `f16_side` reads: the part's blocks' running entry
/// counts (u16 each), each entry's node's low byte, then the values; a part with none is empty.
fn part_codes(bytes: &[u8], view: &TapeView, part: usize, mut f: impl FnMut(usize, u32)) {
    let side = view.side[0] as usize;
    let (from, to) = (u32_at(bytes, view.parts[0] as usize + 4 * part), u32_at(bytes, view.parts[0] as usize + 4 * part + 4));
    // A part with no entries is no bytes (`part_into`).
    if from == to {
        return;
    }
    let start = side + from as usize;
    let blocks = (view.nodes as usize).div_ceil(BLOCK);
    let sub = PART / BLOCK;
    let nsub = sub.min(blocks - part * sub);
    let u16_at = |at: usize| usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
    let offsets = start + 2 * nsub;
    let values = offsets + u16_at(start + 2 * (nsub - 1));
    let vw = ((view.widths >> 16) & 15) as usize;
    let mask = if vw >= 4 { u32::MAX } else { (1u32 << (8 * vw)) - 1 };
    let mut lo = 0;
    for j in 0..nsub {
        let hi = u16_at(start + 2 * j);
        for e in lo..hi {
            let at = values + e * vw;
            let mut w = [0u8; 4];
            w[..vw.min(4)].copy_from_slice(&bytes[at..at + vw.min(4)]);
            f(j * BLOCK + usize::from(bytes[offsets + e]), u32::from_le_bytes(w) & mask);
        }
        lo = hi;
    }
}

/// A set's test a code byte at a time: membership of each number below the cutoff, and whether it holds any past.
fn byte_table(lang: &Lang, set: &CodeSet) -> (ByteSet, bool) {
    let cut = lang.cut as usize;
    let mut byte = [false; 256];
    set.mark_below(cut, &mut byte);
    (ByteSet::of(&byte), set.any_from(cut))
}

/// A set of byte values as simdjson's nibble tables (Hyperscan's "truffle"): byte `b` is in it where bit `b >> 4`
/// of `low[b & 15]` is set (`b` below 128), or bit `(b >> 4) - 8` of `high[b & 15]` (128 and above). Each half is
/// a nibble classification (`classify`), the high nibble's bit from a fixed table, so 64 codes are tested by four
/// lookups, not a lookup a code.
struct ByteSet {
    low: [u8; 16],
    high: [u8; 16],
}

/// The high nibble's bit in `ByteSet::low` (0 for a byte at 128 or above) and in `ByteSet::high`.
const NIBBLE_LOW: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 0, 0, 0, 0, 0, 0, 0, 0];
const NIBBLE_HIGH: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 4, 8, 16, 32, 64, 128];

impl ByteSet {
    fn of(member: &[bool; 256]) -> Self {
        let (mut low, mut high) = ([0u8; 16], [0u8; 16]);
        for (b, _) in member.iter().enumerate().filter(|(_, member)| **member) {
            let (lo, hi) = (b & 15, b >> 4);
            if hi < 8 {
                low[lo] |= 1 << hi;
            } else {
                high[lo] |= 1 << (hi - 8);
            }
        }
        Self { low, high }
    }
}

/// Bit `j` of `out[g]`: code byte `64 g + j` is in `set`, 64 codes a step (the last group's past `codes` clear).
#[inline(always)]
unsafe fn groups_of<S: rerail_simd8::Simd8x64>(codes: &[u8], set: &ByteSet, out: &mut [u64; PART / 64]) {
    for (g, chunk) in codes.chunks(64).enumerate() {
        // The last group's codes in a register's width (the bytes past them cleared below).
        let mut last = [0u8; 64];
        let block: &[u8; 64] = match <&[u8; 64]>::try_from(chunk) {
            Ok(block) => block,
            Err(_) => {
                last[..chunk.len()].copy_from_slice(chunk);
                &last
            }
        };
        // SAFETY: `dispatch!` runs this on a backend the CPU runs.
        let m = unsafe {
            let v = S::load(block);
            S::nonzero(S::or(S::classify(v, &set.low, &NIBBLE_LOW), S::classify(v, &set.high, &NIBBLE_HIGH)))
        };
        out[g] = if chunk.len() == 64 { m } else { m & ((1u64 << chunk.len()) - 1) };
    }
}

rerail_simd8::dispatch! {
    fn code_groups(codes: &[u8], set: &ByteSet, out: &mut [u64; PART / 64]) -> () = groups_of, on code_groups_on;
}

/// A format 16 tape read in place with its language's choices, through the tape's reads (`crate::reads`, written directly,
/// decision 0510) over its view.
#[derive(Clone, Copy)]
pub struct F16<'a> {
    bytes: &'a [u8],
    view: TapeView,
    lang: &'a Lang,
}

impl F16<'_> {
    /// The line (from 0) byte `x` of the source lies in: the newlines before it, by the tape's newline index.
    pub fn line(&self, x: u32) -> u32 {
        reads::line(self.bytes, &self.view, x)
    }
    /// Line `k`'s first byte (`k` up to the source's newlines).
    pub fn line_start(&self, k: u32) -> u32 {
        reads::line_start(self.bytes, &self.view, k)
    }
    /// The source's lines: one more than its newlines.
    pub fn line_count(&self) -> u32 {
        self.view.line_count
    }
    /// Block `b`'s code bytes: its first column.
    fn block_codes(&self, b: usize) -> &[u8] {
        let at = self.view.columns[0] as usize + b * self.view.stride as usize;
        &self.bytes[at..at + Z.min(self.view.nodes as usize - b * Z)]
    }
}

impl Tape for F16<'_> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.view.nodes as usize
    }
    /// By the tape's presence in its home (`tape_has_kind`, the kernels' read).
    fn has_kind(&self, block: u32, kind: u16) -> bool {
        reads::has_kind(self.bytes, &self.view, block, u32::from(kind)) != 0
    }
    #[inline(always)]
    fn table(&self) -> &CodeTable {
        &self.lang.table
    }
    #[inline(always)]
    fn code(&self, i: usize) -> u32 {
        reads::number(self.bytes, &self.view, i as u32)
    }
    #[inline(always)]
    fn subtree_end(&self, i: usize) -> usize {
        i + reads::size(self.bytes, &self.view, i as u32) as usize
    }
    /// The node before is the parent (an inner node), or a leaf naming the previous sibling or, on every `K`-th
    /// sibling, the parent: at most `K - 1` sibling steps.
    fn parent(&self, i: usize) -> Option<usize> {
        let p = reads::parent(self.bytes, &self.view, i as u32);
        (p != NONE).then_some(p as usize)
    }
    #[inline(always)]
    fn start(&self, i: usize) -> u64 {
        u64::from(reads::start(self.bytes, &self.view, i as u32))
    }
    #[inline(always)]
    fn end(&self, i: usize) -> u64 {
        u64::from(reads::end(self.bytes, &self.view, i as u32))
    }
    #[inline(always)]
    fn start_next(&self, i: usize, prev_start: u64) -> u64 {
        u64::from(reads::start_next(self.bytes, &self.view, i as u32, prev_start as u32))
    }
    #[inline(always)]
    fn end_with_start(&self, i: usize, start: u64) -> u64 {
        start + u64::from(reads::length(self.bytes, &self.view, i as u32))
    }
    /// The code bytes 64 at a time through a 256-entry table; a byte past the cutoff is decoded only when the set
    /// holds a number past it.
    fn match_groups(&self, set: &CodeSet, f: impl FnMut(usize, u64)) {
        self.match_groups_in(set, &|_| true, f);
    }
    fn match_groups_in(&self, set: &CodeSet, block: &dyn Fn(u32) -> bool, mut f: impl FnMut(usize, u64)) {
        let (byte, tail) = byte_table(self.lang, set);
        let mut groups = [0u64; PART / 64];
        for part in 0..(self.view.nodes as usize).div_ceil(PART) {
            if !block(part as u32) {
                continue;
            }
            let chunk = self.block_codes(part);
            let count = part_groups(chunk, self.lang, set, &byte, tail.then_some((self.bytes, &self.view, part)), &mut groups);
            for (g, &m) in groups[..count].iter().enumerate() {
                f(part * PART + 64 * g, m);
            }
        }
    }
}

// ---- format 16 in fixed blocks (16z) -----------------------------------------------------------------------

/// Nodes a block holds: one side-list part, so every read of a node but a parent's sibling steps stays in its
/// block.
pub const Z: usize = PART;

/// The pieces a 16z block is stored in, one an array: codes, slots, gaps, lengths. Each is the block's column of
/// the array, then the array's side list's part for the block (0156 §2.1: one decode gives both).
const PIECES: usize = 4;

/// How a piece is stored (its kind byte): as its bytes, as one LZ4 block, or as a repeat of another piece.
const RAW: u8 = 0;
const LZ4: u8 = 1;
const REPEAT: u8 = 2;

/// The fewest bytes a piece holds to be stored as a repeat (whose payload is the other piece's index, 4 bytes).
#[cfg(any(test, feature = "reference"))]
const REPEAT_MIN: usize = 8;
/// The fewest bytes a piece holds to be offered to LZ4.
#[cfg(any(test, feature = "reference"))]
const LZ4_MIN: usize = 16;

/// A 16z tape's own regions (`head::Z`): its part starts, checkpoints, newline index, piece starts, piece lengths,
/// piece kinds, and its end.
const ZPARTS: usize = 0;
const ZCKPT: usize = 1;
const ZLINES: usize = 2;
const ZSTARTS: usize = 3;
const ZLENS: usize = 4;
const ZKINDS: usize = 5;
const ZEND: usize = 6;

/// One document's format 16 tape stored in fixed blocks of `Z` nodes (0156 §2.1; decision 0434 §3), held as
/// stored (`as_bytes`):
///
/// - the header of the format 16 tape it decodes to, its magic `MAGIC16Z`, and where its own regions lie;
/// - the payload: each block's pieces, in the order the blocks became final;
/// - the side lists' part starts (as the format 16 tape's);
/// - the checkpoints, raw (absolute starts do not compress), in node order, with their room;
/// - the newline index, raw, with its room (read in place: `newlines`);
/// - the piece index: each piece's start in the payload and its length (u32s), then each one's kind (a byte), by
///   piece (`PIECES` a block, in block order).
///
/// Block `b`'s piece `q` decodes to array `q`'s column in block `b` and side list `q`'s part `b`. A block decodes
/// alone, where a read falls, and the tape is never laid out whole (decision 0353 step 4b): the CPU's `F16Z` decodes
/// it into its thread's slots, the reads written once into a cache entry (`view`).
#[derive(Clone, Debug)]
pub struct Tape16Z {
    /// Which tape its decoded blocks are, in the thread's cache (a clone holds the same bytes).
    id: u64,
    pub mark: u8,
    /// The placement of the format 16 tape it decodes to.
    p: Placement,
    source_len: u64,
    /// As stored: its own, or placed in a slab's room (decision 0353 step 4a).
    buf: crate::home::Home,
    /// Where its own regions lie (`ZPARTS` …).
    zat: [usize; ZS],
}

impl Tape16Z {
    pub fn len(&self) -> usize {
        self.p.n
    }
    pub fn is_empty(&self) -> bool {
        self.p.n == 0
    }
    /// The bytes it takes in memory: as stored.
    pub fn bytes(&self) -> usize {
        self.buf.len()
    }
    pub fn source_len(&self) -> u64 {
        self.source_len
    }
    pub fn tag(&self) -> u8 {
        self.buf[head::LANGUAGE]
    }
    /// The tape as stored, without its presence.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.zat[ZEND]]
    }
    /// Its home's bytes: the tape, then its presence where it has one (as `Tape16::extent`).
    pub fn extent(&self) -> &[u8] {
        &self.buf
    }
    /// Its presence's bytes in its home, if it has one.
    pub fn presence_bytes(&self) -> Option<&[u8]> {
        (self.p.presence != 0).then(|| &self.buf[self.p.presence..])
    }
    /// Where its bytes are.
    pub fn home(&self) -> &crate::home::Home {
        &self.buf
    }

    /// The stored tape, held as it is: its header, its regions and its piece index are checked against its length,
    /// and each piece against where it decodes to. No piece is decoded.
    pub fn from_bytes(buf: impl Into<crate::home::Home>) -> Result<Self, String> {
        let buf: crate::home::Home = buf.into();
        if buf.len() < HEAD16 || &buf[..8] != MAGIC16Z {
            return Err("not a 16z tape".into());
        }
        let p = header(&buf)?;
        let zat: [usize; ZS] = std::array::from_fn(|t| u32_at(&buf, head::Z + 4 * t) as usize);
        let (count, n) = (p.block_count(), p.n);
        // Its nodes numbered in a u32 (the device's), its blocks within a cache key's 22 bits (`(id << 22) | block`).
        if u32::try_from(n).is_err() || count >= 1 << 22 {
            return Err("a 16z tape too long for its reads".into());
        }
        let pieces = PIECES * count;
        let lines = p.end - 8 - p.lines;
        let bad = || "a 16z tape's regions do not fit it".to_owned();
        let at_parts = zat[ZPARTS];
        if at_parts < HEAD16
            || !at_parts.is_multiple_of(4)
            || zat[ZCKPT] != at_parts + 16 * (count + 1)
            || zat[ZLINES] != zat[ZCKPT] + n.div_ceil(K) * p.cw + 8
            || zat[ZSTARTS] != (zat[ZLINES] + lines + 8).next_multiple_of(4)
            || zat[ZLENS] != zat[ZSTARTS] + 4 * pieces
            || zat[ZKINDS] != zat[ZLENS] + 4 * pieces
            || zat[ZEND] != zat[ZKINDS] + pieces
            || zat[ZEND] > buf.len()
        {
            return Err(bad());
        }
        // Its presence, after it in its home, where it has one.
        let mut p = p;
        p.presence = presence::find(&buf, zat[ZEND], count)?;
        fits(&p, &buf, std::array::from_fn(|k| at_parts + 4 * k * (count + 1)))?;
        let source_len = u64_at(&buf, head::SOURCE_LEN)?;
        check_lines(&buf[zat[ZLINES]..zat[ZLINES] + lines], &p, source_len)?;
        let t = Tape16Z { id: next_id(), mark: buf[head::MARK], p, source_len, buf, zat };
        let payload = at_parts - HEAD16;
        for j in 0..pieces {
            let (s, l) = t.span(j);
            if s.checked_add(l).is_none_or(|e| e > payload) {
                return Err(format!("a 16z tape's piece {j} lies past its payload"));
            }
        }
        for j in 0..pieces {
            let want = t.decoded(j);
            let (kind, bytes) = t.piece(j);
            let fits = match kind {
                RAW => bytes.len() == want,
                // Within what LZ4 decodes its bytes to (255 bytes a byte at most): the side lengths the part starts
                // state size every cache entry, so they are bounded by the bytes stored, not taken on trust
                // (decision 0463).
                LZ4 => !bytes.is_empty() && want as u64 <= 255 * bytes.len() as u64 + 64,
                REPEAT => {
                    let other = if bytes.len() == 4 { u32_at(bytes, 0) as usize } else { usize::MAX };
                    other < pieces && other != j && t.kind(other) != REPEAT && t.decoded(other) == want
                }
                _ => false,
            };
            if !fits {
                return Err(format!("a 16z tape's piece {j} does not fit it"));
            }
        }
        Ok(t)
    }

    /// The length of the format 16 tape it decodes to (`decode_into`, a test's).
    #[cfg(test)]
    fn decoded_len(&self) -> usize {
        self.p.end
    }

    /// Part `j`'s start in side list `k` (one more at the end).
    fn part_start(&self, k: usize, j: usize) -> usize {
        u32_at(&self.buf, self.zat[ZPARTS] + 4 * (k * (self.p.block_count() + 1) + j)) as usize
    }

    /// Piece `j`'s start in the payload and its length.
    fn span(&self, j: usize) -> (usize, usize) {
        (u32_at(&self.buf, self.zat[ZSTARTS] + 4 * j) as usize, u32_at(&self.buf, self.zat[ZLENS] + 4 * j) as usize)
    }

    fn kind(&self, j: usize) -> u8 {
        self.buf[self.zat[ZKINDS] + j]
    }

    /// Where block `b`'s piece `q` decodes to in the format 16 tape: the array's column in the block, and its side
    /// list's part.
    fn dest(&self, b: usize, q: usize) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        let w = cell(self.p.widths, self.p.cw, q);
        let fixed = self.p.column(b, q);
        let side = self.p.side[q];
        (fixed..fixed + self.p.rows(b) * w, side + self.part_start(q, b)..side + self.part_start(q, b + 1))
    }

    /// Piece `j`'s decoded length.
    fn decoded(&self, j: usize) -> usize {
        let (fixed, side) = self.dest(j / PIECES, j % PIECES);
        fixed.len() + side.len()
    }

    /// Piece `j`'s kind and its stored bytes.
    fn piece(&self, j: usize) -> (u8, &[u8]) {
        let (s, l) = self.span(j);
        (self.kind(j), &self.buf[HEAD16 + s..HEAD16 + s + l])
    }

    /// Piece `j`'s decoded bytes: its stored bytes where it is raw, else decoded into `scratch`.
    fn piece_bytes<'a>(&'a self, j: usize, scratch: &'a mut Vec<u8>) -> Result<&'a [u8], String> {
        let (kind, bytes) = self.piece(j);
        match kind {
            RAW => Ok(bytes),
            LZ4 => {
                let ((s, l), len) = (self.span(j), self.decoded(j));
                scratch.resize(len + 8, 0);
                decode_block(&self.buf, HEAD16 + s, HEAD16 + s + l, scratch, len)?;
                Ok(&scratch[..len])
            }
            // Another piece that is not a repeat (`from_bytes`).
            _ => self.piece_bytes(u32_at(bytes, 0) as usize, scratch),
        }
    }

    /// The format 16 tape's parts that are not in pieces, into `out` (a test's: a 16z tape is read a block at a time,
    /// never laid out whole, decision 0353 step 4b) (`decoded_len` bytes): its header, each
    /// block's checkpoints, the side lists' part starts, the newline index, and the room after the last block, each
    /// side list and the newline index. With every block (`decode_block_into`), `out` is the format 16 tape
    /// (`decode_into`).
    #[cfg(test)]
    fn decode_head_into(&self, out: &mut [u8]) -> Result<(), String> {
        let p = &self.p;
        if out.len() != p.end {
            return Err(format!("{} bytes for a format 16 tape of {}", out.len(), p.end));
        }
        out[..HEAD16].copy_from_slice(&self.buf[..HEAD16]);
        out[..8].copy_from_slice(MAGIC16);
        out[head::Z..HEAD16].fill(0);
        let ckpt = self.zat[ZCKPT];
        for b in 0..p.block_count() {
            let (at, len) = (p.column(b, 4), p.rows(b).div_ceil(K) * p.cw);
            let from = ckpt + b * (Z / K) * p.cw;
            out[at..at + len].copy_from_slice(&self.buf[from..from + len]);
        }
        let end = p.blocks_end();
        out[end..end + 8].fill(0);
        for k in 0..4 {
            let at = p.side[k] + self.part_start(k, p.block_count());
            out[at..at + 8].fill(0);
        }
        let pad = p.side[3] + self.part_start(3, p.block_count()) + 8;
        out[pad..p.parts[0]].fill(0);
        out[p.parts[0]..p.lines].copy_from_slice(&self.buf[self.zat[ZPARTS]..self.zat[ZCKPT]]);
        let lines = p.end - 8 - p.lines;
        out[p.lines..p.lines + lines].copy_from_slice(&self.buf[self.zat[ZLINES]..self.zat[ZLINES] + lines]);
        out[p.end - 8..].fill(0);
        Ok(())
    }

    /// Block `b` decoded where it lies in the format 16 tape `out` (`decoded_len` bytes): each of its pieces, its
    /// array's column and its side part. No other block is read; `scratch` holds an LZ4 piece while it is copied.
    #[cfg(test)]
    fn decode_block_into(&self, b: usize, out: &mut [u8], scratch: &mut Vec<u8>) -> Result<(), String> {
        if out.len() != self.decoded_len() || b >= self.p.block_count() {
            return Err(format!("block {b} into {} bytes, of a format 16 tape of {}", out.len(), self.decoded_len()));
        }
        for q in 0..PIECES {
            let (fixed, side) = self.dest(b, q);
            let bytes = self.piece_bytes(PIECES * b + q, scratch)?;
            let (f, s) = bytes.split_at(fixed.len());
            out[fixed].copy_from_slice(f);
            out[side].copy_from_slice(s);
        }
        Ok(())
    }

    /// The format 16 tape it is, into `out` (`decoded_len` bytes): its head, then each block where it lies.
    #[cfg(test)]
    fn decode_into(&self, out: &mut [u8]) -> Result<(), String> {
        self.decode_head_into(out)?;
        let mut scratch = Vec::new();
        for b in 0..self.p.block_count() {
            self.decode_block_into(b, out, &mut scratch)?;
        }
        Ok(())
    }

    /// The bytes block `b` takes decoded in a cache entry, as the reads written once lay it (`rerail_clif::reads`,
    /// `z_decode`): column `q` at `nb * C[q]`, `C[q]` the least that clears the piece before (its column, then its
    /// side part) and 16 bytes of room; the checkpoints, 16 bytes of room, then each side list's part's starts.
    fn entry_bytes(&self, b: usize) -> usize {
        let nb = self.p.rows(b);
        let mut c = 0;
        for q in 0..PIECES {
            let side = self.part_start(q, b + 1) - self.part_start(q, b);
            c = (nb * c + nb * cell(self.p.widths, self.p.cw, q) + side + 16).div_ceil(nb);
        }
        (nb * c + nb.div_ceil(K) * self.p.cw + 16).next_multiple_of(8) + 32 + 8
    }

    /// The view the reads written once take of this tape as stored (decision 0353 step 4b,
    /// `rerail_clif::view::F16Z_FORMAT`): its piece index, side lists' part starts, checkpoints and payload where
    /// they lie in `as_bytes`, its newline index (raw: format 16's line reads read it in place), format 16's widths
    /// and `lang`'s choices, its id, and the size of a cache entry that holds its largest block decoded. A reader
    /// reads it through a cache, which the view is given (`view::Reader`, or the device's).
    pub fn view(&self, lang: &Lang) -> Result<TapeView, String> {
        use rerail_clif::view::{F16Z_BLOCK, F16Z_FORMAT, F16Z_PIECES, z};
        const _: () = assert!(F16Z_BLOCK as usize == Z && F16Z_PIECES as usize == PIECES);
        const _: () = assert!(z::RAW == RAW && z::LZ4 == LZ4 && z::REPEAT == REPEAT);
        if self.mark != lang.revision || self.p.widths != lang.widths {
            return Err(format!("a tape of revision {}, read with revision {}", self.mark, lang.revision));
        }
        let largest = (0..self.p.block_count()).map(|b| self.entry_bytes(b)).max().unwrap_or(0);
        let entry = (z::SLOT_BYTES as usize + largest + z::SLACK).next_power_of_two();
        let mut view = bind(self.p.frame(0)?, lang)?;
        let at = |offset: usize| u32::try_from(offset).map_err(|_| "a tape past 4 GiB".to_owned());
        view.format = F16Z_FORMAT;
        view.columns = [at(self.zat[ZSTARTS])?, at(self.zat[ZPARTS])?, at(self.zat[ZCKPT])?, at(entry)?, 0];
        view.stride = at(HEAD16)?;
        view.lines = at(self.zat[ZLINES])?;
        view.side = [0, 0, self.id as u32, (self.id >> 32) as u32];
        view.parts = [0; 4];
        Ok(view)
    }

    /// Read with `lang`'s choices (its revision and widths).
    pub fn reader<'a>(&'a self, lang: &'a Lang) -> Result<F16Z<'a>, String> {
        if self.mark != lang.revision || self.p.widths != lang.widths {
            return Err(format!("a tape of revision {} (widths {:?}), read with revision {} ({:?})", self.mark, self.p.widths, lang.revision, lang.widths));
        }
        presence::of_lang(self.presence_bytes(), lang)?;
        Ok(F16Z { t: self, lang })
    }
}

fn next_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A decoded block: its tape and number, its bytes (a format 16 tape of its nodes: its block, then each side list's
/// one part and their starts), and its view, with its language's choices; and the room an LZ4 piece is decoded in.
#[derive(Clone, Default)]
struct Slot {
    b: Option<(u64, usize)>,
    raw: Vec<u8>,
    view: TapeView,
    scratch: Vec<u8>,
}

#[derive(Clone, Default)]
struct Cache {
    slots: [Slot; 2],
    /// The slot a miss replaces: the one not read last.
    next: usize,
}

thread_local! {
    /// The blocks this thread decoded last, whichever tape they are of: a reader is made per read, and keeps none.
    static BLOCKS: std::cell::RefCell<Cache> = std::cell::RefCell::default();
}

/// A 16z tape read a block at a time: the block a read falls in is decoded to a format 16 tape of its nodes (the
/// thread keeps two, so a parent's sibling steps back over a block's start do not decode again) and read in place.
#[derive(Clone, Copy)]
pub struct F16Z<'a> {
    t: &'a Tape16Z,
    lang: &'a Lang,
}

impl F16Z<'_> {
    /// `f` over block `b`'s bytes and view, decoding it on a miss. `f` reads no other 16z tape.
    #[inline(always)]
    fn with<R>(&self, b: usize, f: impl FnOnce(&[u8], &TapeView) -> R) -> R {
        BLOCKS.with(|cache| {
            let mut c = cache.borrow_mut();
            let key = Some((self.t.id, b));
            let s = if c.slots[0].b == key {
                0
            } else if c.slots[1].b == key {
                1
            } else {
                let s = c.next;
                self.decode(b, &mut c.slots[s]);
                s
            };
            c.next = s ^ 1;
            let slot = &c.slots[s];
            f(&slot.raw, &slot.view)
        })
    }
    /// Block `b` as a format 16 tape of its nodes: one block at 0, its pieces' columns and side parts, its
    /// checkpoints, the room, and each side list's one part.
    #[cold]
    #[inline(never)]
    fn decode(&self, b: usize, slot: &mut Slot) {
        let t = self.t;
        let nb = t.p.rows(b);
        let sides: [usize; 4] = std::array::from_fn(|k| t.part_start(k, b + 1) - t.part_start(k, b));
        // No newline index: a block's view answers no line (the whole tape's does).
        let q = Placement::of(0, nb, t.p.cw, t.p.widths, t.p.vw, sides, 0, 0, t.source_len);
        slot.raw.clear();
        slot.raw.resize(q.end + 32, 0);
        let mut scratch = std::mem::take(&mut slot.scratch);
        for (r, &side) in sides.iter().enumerate() {
            let at = q.column(0, r);
            let fixed = nb * cell(t.p.widths, t.p.cw, r);
            let bytes = t.piece_bytes(PIECES * b + r, &mut scratch).expect("a 16z block decodes to its length");
            slot.raw[at..at + fixed].copy_from_slice(&bytes[..fixed]);
            slot.raw[q.side[r]..q.side[r] + side].copy_from_slice(&bytes[fixed..]);
        }
        slot.scratch = scratch;
        let (at, len) = (q.column(0, 4), nb.div_ceil(K) * t.p.cw);
        let from = t.zat[ZCKPT] + b * (Z / K) * t.p.cw;
        slot.raw[at..at + len].copy_from_slice(&t.buf[from..from + len]);
        // Each side list's one part: from 0 to its length.
        for (k, &len) in sides.iter().enumerate() {
            slot.raw[q.parts[k] + 4..q.parts[k] + 8].copy_from_slice(&(len as u32).to_le_bytes());
        }
        slot.view = q.frame(0).and_then(|frame| bind(frame, self.lang)).expect("a block's view: its widths are its tape's, checked when read");
        slot.b = Some((t.id, b));
    }
}

impl Tape for F16Z<'_> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.t.p.n
    }
    /// The presence lies in the 16z tape's home, read where it lies through the tape's own view (`tape_has_kind`).
    fn has_kind(&self, block: u32, kind: u16) -> bool {
        let view = self.t.view(self.lang).expect("a 16z tape's view: its reader's language is its own");
        reads::has_kind(&self.t.buf, &view, block, u32::from(kind)) != 0
    }
    #[inline(always)]
    fn table(&self) -> &CodeTable {
        &self.lang.table
    }
    #[inline(always)]
    fn code(&self, i: usize) -> u32 {
        self.with(i / Z, |r, v| reads::number(r, v, (i % Z) as u32))
    }
    #[inline(always)]
    fn subtree_end(&self, i: usize) -> usize {
        i + self.with(i / Z, |r, v| reads::size(r, v, (i % Z) as u32)) as usize
    }
    /// As format 16's, each step in the block it falls in (a parent's sibling steps cross blocks' starts).
    fn parent(&self, i: usize) -> Option<usize> {
        let mut j = i;
        while j > 0 {
            let v = self.with((j - 1) / Z, |r, v| reads::slot(r, v, ((j - 1) % Z) as u32));
            if v & 1 == 0 {
                return Some(j - 1);
            }
            if v & 2 != 0 {
                return Some(j - (v >> 2) as usize);
            }
            j -= (v >> 2) as usize;
        }
        None
    }
    #[inline(always)]
    fn start(&self, i: usize) -> u64 {
        u64::from(self.with(i / Z, |r, v| reads::start(r, v, (i % Z) as u32)))
    }
    #[inline(always)]
    fn end(&self, i: usize) -> u64 {
        u64::from(self.with(i / Z, |r, v| reads::end(r, v, (i % Z) as u32)))
    }
    #[inline(always)]
    fn start_next(&self, i: usize, prev_start: u64) -> u64 {
        u64::from(self.with(i / Z, |r, v| reads::start_next(r, v, (i % Z) as u32, prev_start as u32)))
    }
    #[inline(always)]
    fn end_with_start(&self, i: usize, start: u64) -> u64 {
        start + u64::from(self.with(i / Z, |r, v| reads::length(r, v, (i % Z) as u32)))
    }
    /// A block at a time: each decoded once, its code bytes read as format 16's are. The block's groups are taken
    /// before `f` is called, so `f` may read the tape.
    fn match_groups(&self, set: &CodeSet, f: impl FnMut(usize, u64)) {
        self.match_groups_in(set, &|_| true, f);
    }
    fn match_groups_in(&self, set: &CodeSet, block: &dyn Fn(u32) -> bool, mut f: impl FnMut(usize, u64)) {
        let (byte, tail) = byte_table(self.lang, set);
        let mut groups = [0u64; Z / 64];
        for b in 0..self.t.p.n.div_ceil(Z) {
            // A block left out is not decompressed.
            if !block(b as u32) {
                continue;
            }
            // A block is one part, its side lists' part 0.
            let count = self.with(b, |r, v| {
                let at = v.columns[0] as usize;
                part_groups(&r[at..at + v.nodes as usize], self.lang, set, &byte, tail.then_some((r, v, 0)), &mut groups)
            });
            for (g, &m) in groups[..count].iter().enumerate() {
                f(b * Z + 64 * g, m);
            }
        }
    }
}

/// The LZ4 piece at `buf[from..to]` decoded into `dst[..len]` by the decoder written directly (`reads::lz4`: the
/// reads' `z_lz4` in Rust, the JIT's and the kernels' decoder, decision 0353 step 4b). A piece that does not decode to
/// its length is a corrupt tape.
fn decode_block(buf: &[u8], from: usize, to: usize, dst: &mut [u8], len: usize) -> Result<(), String> {
    if from > to || to > buf.len() || to > u32::MAX as usize {
        return Err(format!("a 16z piece at {from}..{to} of a tape of {} bytes", buf.len()));
    }
    if dst.len() < len {
        return Err(format!("a 16z piece's room of {} bytes for {len}", dst.len()));
    }
    let n = crate::reads::lz4(&buf[from..to], dst, len);
    if n == len { Ok(()) } else { Err(format!("a 16z piece decodes to {n} bytes, not {len}")) }
}

/// LZ4's block format (`lz4-sys`), compressed: HC level 4 in the reference writer (0156 §2.1 chose 9; decision 0434
/// §6 takes 4, which the writer's ops in CLIF port exactly, and write by). Decoded by `decode_block`.
#[cfg(any(test, feature = "reference"))]
mod lz4 {
    use std::os::raw::c_char;

    /// HC's level: 8 chain entries a position.
    #[cfg(any(test, feature = "reference"))]
    pub const LEVEL: i32 = 4;

    #[cfg(any(test, feature = "reference"))]
    pub fn compress(src: &[u8]) -> Vec<u8> {
        compress_at(src, LEVEL)
    }

    /// `src` at HC level `level`.
    #[cfg(any(test, feature = "reference"))]
    pub fn compress_at(src: &[u8], level: i32) -> Vec<u8> {
        let n = i32::try_from(src.len()).expect("a piece under 2 GB");
        // SAFETY: the bound is LZ4's own for `n` bytes; both buffers are as long as the lengths given.
        unsafe {
            let cap = lz4_sys::LZ4_compressBound(n);
            let mut out = vec![0u8; cap as usize];
            let k = lz4_sys::LZ4_compress_HC(src.as_ptr() as *const c_char, out.as_mut_ptr() as *mut c_char, n, cap, level);
            assert!(k > 0, "LZ4 compresses a piece");
            out.truncate(k as usize);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::{F15, children, f15_columns};

    /// Format 16 written from `t` in one pass, and 16z written the same way, which is the format 16 tape's blocks.
    fn sixteen<T: Tape>(lang: &Lang, t: &T, number: &[u32], source_len: u64) -> Tape16 {
        let source = source_of(source_len);
        let Ok(Written::F16(plain)) = write_from(lang, t, number, &source, 0, false) else { panic!("format 16") };
        let Ok(Written::F16Z(z)) = write_from(lang, t, number, &source, 0, true) else { panic!("16z") };
        // The writer's blocks finish as their nodes close (the root's last); `blocks` stores them in order. Both are
        // the format 16 tape's blocks.
        let blocks = plain.blocks();
        let mut from_blocks = vec![0u8; blocks.decoded_len()];
        blocks.decode_into(&mut from_blocks).unwrap();
        assert_eq!(from_blocks, plain.as_bytes(), "format 16's blocks decode to it");
        let mut decoded = vec![0u8; z.decoded_len()];
        z.decode_into(&mut decoded).unwrap();
        assert_eq!(decoded, plain.as_bytes(), "16z decodes to format 16's bytes");
        // Each block decodes where it lies, alone and in any order.
        let mut by_block = vec![0xa5u8; z.decoded_len()];
        z.decode_head_into(&mut by_block).unwrap();
        let mut scratch = Vec::new();
        for b in (0..z.len().div_ceil(Z)).rev() {
            z.decode_block_into(b, &mut by_block, &mut scratch).unwrap();
        }
        assert_eq!(by_block, plain.as_bytes(), "16z's blocks decode in place");
        let p = placement(plain.extent()).unwrap();
        assert_eq!(p, plain.p, "the placement is the reader's");
        compact(&p);
        lines_answer(&plain.reader(lang).unwrap(), &source);
        plain
    }

    /// Block-major (decision 0434, R3): block 0 past the header, the blocks at their stride, the last at its own
    /// rows and no padding, the tail right after it.
    fn compact(p: &Placement) {
        assert_eq!((p.blocks, p.stride), (HEAD16, Z * (1 + p.widths[0] + p.widths[1] + p.widths[2]) + Z / K * p.cw));
        let count = p.block_count();
        if count == 0 {
            return;
        }
        let (last, rows) = (count - 1, p.n - (count - 1) * Z);
        let at = p.blocks + last * p.stride;
        let w = [1, p.widths[0], p.widths[1], p.widths[2]];
        let mut col = at;
        for (r, &wr) in w.iter().enumerate() {
            assert_eq!(p.column(last, r), col, "the last block's column {r}");
            col += rows * wr;
        }
        assert_eq!(p.column(last, 4), col);
        assert_eq!(p.side[0], col + rows.div_ceil(K) * p.cw + 8, "the tail follows the last block's columns");
    }

    /// A source `len` bytes long whose newlines come both spread and in runs.
    fn source_of(len: u64) -> Vec<u8> {
        (0..len).map(|i| if i % 41 == 40 || ((i / 500) % 7 == 3 && i % 3 == 0) { b'\n' } else { b'x' }).collect()
    }

    /// Every byte's line and every line's start, as counting the source's newlines says.
    fn lines_answer(t: &F16<'_>, source: &[u8]) {
        let starts: Vec<u32> = std::iter::once(0).chain(source.iter().enumerate().filter(|(_, b)| **b == b'\n').map(|(i, _)| i as u32 + 1)).collect();
        assert_eq!(t.line_count() as usize, starts.len(), "lines");
        for (k, &s) in starts.iter().enumerate() {
            assert_eq!(t.line_start(k as u32), s, "line {k}'s start");
        }
        let mut line = 0;
        for x in 0..=source.len() {
            assert_eq!(t.line(x as u32), line, "byte {x}'s line");
            if source.get(x) == Some(&b'\n') {
                line += 1;
            }
        }
    }

    /// The line reads over sources whose newlines are dense, sparse, bunched and absent: a tape of one node.
    #[test]
    fn format_16_answers_lines() {
        let (image, _) = crate::tape::tests::sample();
        let (columns, table) = f15_columns(&image).unwrap();
        let f15 = F15::new(&image, columns, &table).unwrap();
        let lang = lang_of(&table);
        let number = numbers(&lang, &table).unwrap();
        let span = f15.end(0);
        for mut source in crate::newlines::tests::sources() {
            source.resize(source.len().max(span as usize), b'y');
            let Ok(Written::F16(t)) = write_from(&lang, &f15, &number, &source, 0, false) else { panic!("format 16") };
            let back = Tape16::from_bytes(t.as_bytes().to_vec()).unwrap();
            lines_answer(&back.reader(&lang).unwrap(), &source);
            let Ok(Written::F16Z(z)) = write_from(&lang, &f15, &number, &source, 0, true) else { panic!("16z") };
            let z = Tape16Z::from_bytes(z.as_bytes().to_vec()).unwrap();
            let mut decoded = vec![0u8; z.decoded_len()];
            z.decode_into(&mut decoded).unwrap();
            assert_eq!(decoded, t.as_bytes(), "16z holds the newline index");
        }
    }

    /// A language whose table is the tape's own, numbered as it is, at one byte a value.
    fn lang_of(table: &CodeTable) -> Lang {
        Lang::new(table.clone(), [1, 1, 1], 1).unwrap()
    }

    fn same<A: Tape, B: Tape>(a: &A, b: &B, number: &[u32]) {
        assert_eq!(a.len(), b.len());
        let mut spans = (Vec::new(), Vec::new());
        a.spans(&mut spans.0);
        b.spans(&mut spans.1);
        assert_eq!(spans.0, spans.1, "spans in order");
        for i in 0..a.len() {
            assert_eq!(number[a.code(i) as usize], b.code(i), "code of {i}");
            assert_eq!(a.subtree_end(i), b.subtree_end(i), "subtree of {i}");
            assert_eq!(a.parent(i), b.parent(i), "parent of {i}");
            assert_eq!((a.start(i), a.end(i)), (b.start(i), b.end(i)), "span of {i}");
            assert_eq!(children(a, i).collect::<Vec<_>>(), children(b, i).collect::<Vec<_>>(), "children of {i}");
        }
    }

    /// Format 16 written from format 15 answers every question as format 15 does.
    #[test]
    fn format_16_answers_as_format_15() {
        let (image, _) = crate::tape::tests::sample();
        let (columns, table) = f15_columns(&image).unwrap();
        let f15 = F15::new(&image, columns, &table).unwrap();
        let lang = lang_of(&table);
        let number = numbers(&lang, &table).unwrap();
        let t = sixteen(&lang, &f15, &number, 2_000);
        same(&f15, &t.reader(&lang).unwrap(), &number);
        same(&f15, &t.blocks().reader(&lang).unwrap(), &number);
        // As stored, and read back where it lies.
        let mut tagged = t.clone();
        tagged.set_tag(7);
        let back = Tape16::from_bytes(tagged.as_bytes().to_vec()).unwrap();
        assert_eq!((back.tag(), back.source_len()), (7, 2_000));
        same(&f15, &back.reader(&lang).unwrap(), &number);
        let z = Tape16Z::from_bytes(tagged.blocks().as_bytes().to_vec()).unwrap();
        assert_eq!((z.tag(), z.source_len()), (7, 2_000));
        same(&f15, &z.reader(&lang).unwrap(), &number);
        let mut short = tagged.as_bytes().to_vec();
        short.pop();
        assert!(Tape16::from_bytes(short).is_err());
    }

    /// A byte set's nibble tables (`ByteSet`) find the codes the set holds, on every backend this CPU runs, for sets
    /// on either side of 128 and a run that ends inside a group.
    #[test]
    fn byte_sets_find_their_codes_on_every_backend() {
        let codes: Vec<u8> = (0..1_000u32).map(|i| (i * 37 % 256) as u8).collect();
        let sets: [fn(usize) -> bool; 5] = [|b| b % 3 == 0, |b| b >= 200, |b| b == 0, |b| b == 255, |_| true];
        for member in sets {
            let table: [bool; 256] = std::array::from_fn(member);
            let set = ByteSet::of(&table);
            let mut want = [0u64; PART / 64];
            for (i, &code) in codes.iter().enumerate() {
                want[i / 64] |= u64::from(table[code as usize]) << (i % 64);
            }
            for backend in rerail_simd8::Backend::ALL {
                let mut got = [0u64; PART / 64];
                if code_groups_on(backend, &codes, &set, &mut got).is_some() {
                    assert_eq!(got, want, "{backend:?}");
                }
            }
        }
    }

    /// Wide values escape to the side lists, and codes past the cutoff keep their low parts there: a long, flat
    /// and deep tree over a table of 600 codes, with gaps, lengths and sizes past a byte.
    #[test]
    fn escapes_and_codes_past_the_cutoff_read_from_their_side_lists() {
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let plain = Plain(&nodes, &table);
        let number: Vec<u32> = (0..600).collect();
        let t = sixteen(&lang, &plain, &number, pos + 2);
        assert!((0..4).all(|k| t.part_start(k, t.p.n.div_ceil(PART)) > 0), "every side list has entries");
        same(&plain, &t.reader(&lang).unwrap(), &number);
        let set = table.set(|e| e.kind % 97 == 3 || e.kind > 590);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        plain.match_groups(&set, |base, m| a.push((base, m)));
        t.reader(&lang).unwrap().match_groups(&set, |base, m| b.push((base, m)));
        assert_eq!(a, b, "a set past the cutoff");
        // The same tape in fixed blocks: every read decodes the block it falls in.
        let z = Tape16Z::from_bytes(t.blocks().as_bytes().to_vec()).unwrap();
        assert!(z.len() > 6 * Z, "several blocks");
        assert_eq!(Tape16::from_bytes(t.extent().to_vec()).unwrap(), t);
        same(&plain, &z.reader(&lang).unwrap(), &number);
        let mut c = Vec::new();
        z.reader(&lang).unwrap().match_groups(&set, |base, m| c.push((base, m)));
        assert_eq!(a, c, "a set past the cutoff, in blocks");
        // A part whose codes are all below the cutoff has no entries in side list 0 (no bytes): its groups come from
        // its bytes alone, in format 16 and in blocks.
        let mut low = nodes.clone();
        for node in &mut low[PART..2 * PART] {
            node.0 %= 200;
        }
        let plain_low = Plain(&low, &table);
        let t_low = sixteen(&lang, &plain_low, &number, pos + 2);
        assert_eq!(t_low.part_start(0, 1), t_low.part_start(0, 2), "the second part lists no codes");
        let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
        plain_low.match_groups(&set, |base, m| a.push((base, m)));
        t_low.reader(&lang).unwrap().match_groups(&set, |base, m| b.push((base, m)));
        Tape16Z::from_bytes(t_low.blocks().as_bytes().to_vec()).unwrap().reader(&lang).unwrap().match_groups(&set, |base, m| c.push((base, m)));
        assert_eq!((&a, &a), (&b, &c), "a part with no codes past the cutoff");
        // A bulk answer that reads the tape inside the matches' callback.
        let (mut near, mut near_z) = (Vec::new(), Vec::new());
        crate::tape::nearest_all(&t.reader(&lang).unwrap(), &set, |_| true, &mut near);
        crate::tape::nearest_all(&z.reader(&lang).unwrap(), &set, |_| true, &mut near_z);
        assert_eq!(near, near_z, "nearest in blocks");
    }

    /// 16z stores a piece as its bytes, as LZ4, or as a repeat of an earlier one, and refuses a stored tape whose
    /// pieces do not fit it: a root over 6,000 alike leaves, whose blocks' codes, gaps and lengths repeat.
    #[test]
    fn sixteen_z_pieces_are_raw_lz4_or_repeats() {
        let entries: Vec<Entry> = (0..4u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let n = 6_001;
        let mut nodes: Vec<(u32, u64, u64, usize)> = (1..n as u64).map(|i| (1 + (i % 3) as u32, 4 * i, 4 * i + 2, 1)).collect();
        nodes.insert(0, (0, 0, 4 * n as u64, n));
        let plain = Plain(&nodes, &table);
        let t = sixteen(&lang, &plain, &[0, 1, 2, 3], 4 * n as u64);
        let z = t.blocks();
        let kinds = &z.as_bytes()[z.zat[ZKINDS]..z.zat[ZKINDS] + PIECES * n.div_ceil(Z)];
        assert!(kinds.contains(&REPEAT) && kinds.contains(&LZ4), "kinds {kinds:?}");
        same(&plain, &z.reader(&lang).unwrap(), &[0, 1, 2, 3]);
        // A repeat that names itself, and a piece index past the payload, are refused.
        let j = kinds.iter().position(|&k| k == REPEAT).unwrap();
        let mut bad = z.as_bytes().to_vec();
        let at = HEAD16 + z.span(j).0;
        bad[at..at + 4].copy_from_slice(&(j as u32).to_le_bytes());
        assert!(Tape16Z::from_bytes(bad).is_err());
        let mut short = z.as_bytes().to_vec();
        short.pop();
        assert!(Tape16Z::from_bytes(short).is_err());
    }

    /// A root over 3,000 children; every 7th child holds a chain 9 deep. Spans grow past one byte's gap. Each
    /// node: its code, start, end and subtree size; and the source's length less 2.
    fn escaped_nodes() -> (Vec<(u32, u64, u64, usize)>, u64) {
        let mut nodes: Vec<(u32, u64, u64, usize)> = vec![(0, 0, 0, 0)];
        let mut pos = 0u64;
        for c in 0..3_000u32 {
            let deep = if c % 7 == 0 { 9 } else { 0 };
            let at = nodes.len();
            for d in 0..=deep {
                nodes.push(((c * 31 + d) % 600, pos, 0, 0));
                pos += if c % 50 == 0 { 400 } else { 3 };
            }
            pos += 10;
            for (d, k) in (at..nodes.len()).enumerate() {
                nodes[k].2 = pos - d as u64;
                nodes[k].3 = nodes.len() - k;
            }
        }
        nodes[0] = (599, 0, pos + 1, nodes.len());
        (nodes, pos)
    }

    /// Nodes read straight from their cells: a reference.
    struct Plain<'a>(&'a [(u32, u64, u64, usize)], &'a CodeTable);
    impl Tape for Plain<'_> {
        fn len(&self) -> usize {
            self.0.len()
        }
        fn table(&self) -> &CodeTable {
            self.1
        }
        fn code(&self, i: usize) -> u32 {
            self.0[i].0
        }
        fn subtree_end(&self, i: usize) -> usize {
            i + self.0[i].3
        }
        fn parent(&self, i: usize) -> Option<usize> {
            (0..i).rev().find(|&j| self.subtree_end(j) > i)
        }
        fn start(&self, i: usize) -> u64 {
            self.0[i].1
        }
        fn end(&self, i: usize) -> u64 {
            self.0[i].2
        }
    }

    /// Every read of every node equals `t`'s, read where the tape lies.
    fn same_reads<T: Tape>(reads: &rerail_clif::jit::Reads, t: &T, tape: &[u8], view: &rerail_clif::view::TapeView) {
        const NONE: u32 = u32::MAX;
        let none = |v: Option<usize>| v.map_or(NONE, |v| v as u32);
        for i in 0..t.len() {
            let end = t.subtree_end(i);
            assert_eq!(reads.one("tape_size", tape, view, i) as usize, end - i, "size of {i}");
            assert_eq!(reads.one("tape_end_of", tape, view, i) as usize, end, "end of {i}'s subtree");
            assert_eq!(reads.one("tape_number", tape, view, i), t.code(i), "code of {i}");
            assert_eq!(u64::from(reads.one("tape_start", tape, view, i)), t.start(i), "start of {i}");
            assert_eq!(u64::from(reads.one("tape_end", tape, view, i)), t.end(i), "end of {i}");
            let parent = t.parent(i);
            assert_eq!(reads.one("tape_parent", tape, view, i), none(parent), "parent of {i}");
            let next = parent.and_then(|p| Some(end).filter(|&n| n < t.subtree_end(p)));
            assert_eq!(reads.one("tape_next", tape, view, i), none(next), "next of {i}");
            if let Some(p) = parent {
                assert_eq!(reads.two("tape_next_in", tape, view, i, t.subtree_end(p) as u32), none(next), "next of {i} in its parent");
            }
            let kids = crate::tape::children(t, i).collect::<Vec<_>>();
            assert_eq!(reads.one("tape_first_child", tape, view, i), none(kids.first().copied()), "first child of {i}");
            assert_eq!(reads.one("tape_child_count", tape, view, i) as usize, kids.len(), "children of {i}");
            for k in 0..=kids.len().min(12) {
                assert_eq!(reads.two("tape_child", tape, view, i, k as u32), none(kids.get(k).copied()), "child {k} of {i}");
            }
        }
    }

    /// The CLIF reads (decision 0310) read every node as this crate's readers do: format 16 with every side list
    /// in use, and format 15; the format's numbers they are written for are this crate's.
    #[test]
    fn the_clif_reads_read_every_node_as_the_crate_does() {
        use rerail_clif::view;
        assert_eq!((view::F16_K, view::F16_BLOCK, view::F16_PART, view::F16_SUB), (K as u32, BLOCK as u32, PART as u32, (PART / BLOCK) as u32));
        assert_eq!(1 << view::F15_STRIDE_SHIFT, crate::CHECKPOINT_STRIDE);
        let reads = rerail_clif::jit::Reads::new();
        // Format 16, every side list in use.
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let plain = Plain(&nodes, &table);
        let t = sixteen(&lang, &plain, &(0..600).collect::<Vec<_>>(), pos + 2);
        let v = placement(t.as_bytes()).unwrap().view(&lang, 0).unwrap();
        same_reads(&reads, &t.reader(&lang).unwrap(), t.as_bytes(), &v);
        // 16z, the same nodes in blocks (decision 0353 step 4b): each read decodes its block into the reader's slot.
        let Ok(Written::F16Z(z)) = write_from(&lang, &plain, &(0..600).collect::<Vec<_>>(), &source_of(pos + 2), 0, true) else { panic!("16z") };
        assert!(z.len() > 3 * Z, "the tape spans blocks");
        let reader = rerail_clif::view::Reader::new(&z.view(&lang).unwrap(), z.as_bytes().as_ptr());
        // SAFETY: the reader's view, which this thread alone reads.
        same_reads(&reads, &z.reader(&lang).unwrap(), z.as_bytes(), unsafe { &*reader.view() });
        // Format 15.
        let (image, _) = crate::tape::tests::sample();
        let (columns, table) = crate::tape::f15_columns(&image).unwrap();
        let f15 = crate::tape::F15::new(&image, columns, &table).unwrap();
        let v = crate::tape::f15_view(&image, 0).unwrap();
        same_reads(&reads, &f15, &image, &v);
    }

    /// The Rust reads (`crate::reads`, written directly, decision 0510) read every node, byte and line as the CLIF
    /// reads do through the JIT: format 16 with every side list in use, and format 15.
    #[test]
    fn the_rust_reads_are_the_jits() {
        use crate::reads::{f15, f16};
        let jit = rerail_clif::jit::Reads::new();
        // SAFETY: the read's signature: the tape's base, its view, a node; a u64.
        let slot: extern "C" fn(*const u8, *const TapeView, u32) -> u64 = unsafe { std::mem::transmute(jit.function("tape_slot")) };
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let t = sixteen(&lang, &Plain(&nodes, &table), &(0..600).collect::<Vec<_>>(), pos + 2);
        let (b, v) = (t.as_bytes(), placement(t.as_bytes()).unwrap().view(&lang, 0).unwrap());
        let mut prev = 0;
        for i in 0..v.nodes {
            let n = i as usize;
            assert_eq!(f16::size(b, &v, i), jit.one("tape_size", b, &v, n), "size of {i}");
            assert_eq!(f16::number(b, &v, i), jit.one("tape_number", b, &v, n), "number of {i}");
            assert_eq!(f16::start_stated(b, &v, i), jit.one("tape_start_stated", b, &v, n), "stated start of {i}");
            assert_eq!(f16::start(b, &v, i), jit.one("tape_start", b, &v, n), "start of {i}");
            assert_eq!(f16::end(b, &v, i), jit.one("tape_end", b, &v, n), "end of {i}");
            assert_eq!(f16::length(b, &v, i), jit.one("tape_length", b, &v, n), "length of {i}");
            assert_eq!(f16::parent(b, &v, i), jit.one("tape_parent", b, &v, n), "parent of {i}");
            assert_eq!(f16::slot(b, &v, i), slot(b.as_ptr(), &v, i), "slot of {i}");
            assert_eq!(f16::start_next(b, &v, i, prev), jit.two("tape_start_next", b, &v, n, prev), "next start of {i}");
            prev = f16::start(b, &v, i);
        }
        for x in 0..=v.source_len {
            assert_eq!(f16::line(b, &v, x), jit.one("tape_line", b, &v, x as usize), "line of byte {x}");
        }
        for k in 0..v.line_count {
            assert_eq!(f16::line_start(b, &v, k), jit.one("tape_line_start", b, &v, k as usize), "line {k}'s start");
        }
        for block in (0..v.nodes.div_ceil(Z as u32)).chain([u32::MAX]) {
            for kind in 0..640 {
                assert_eq!(f16::has_kind(b, &v, block, kind), jit.two("tape_has_kind", b, &v, block as usize, kind), "kind {kind} in block {block}");
            }
        }
        // Format 15.
        let (image, _) = crate::tape::tests::sample();
        let v = crate::tape::f15_view(&image, 0).unwrap();
        for i in 0..v.nodes {
            let n = i as usize;
            assert_eq!(f15::size(&image, &v, i), jit.one("tape_size", &image, &v, n), "size of {i}");
            assert_eq!(f15::number(&image, &v, i), jit.one("tape_number", &image, &v, n), "number of {i}");
            assert_eq!(f15::start_stated(&image, &v, i), jit.one("tape_start_stated", &image, &v, n), "stated start of {i}");
            assert_eq!(f15::end_stated(&image, &v, i), jit.one("tape_end_stated", &image, &v, n), "stated end of {i}");
            assert_eq!(f15::start(&image, &v, i), jit.one("tape_start", &image, &v, n), "start of {i}");
            assert_eq!(f15::end(&image, &v, i), jit.one("tape_end", &image, &v, n), "end of {i}");
            assert_eq!(f15::parent(&image, &v, i), jit.one("tape_parent", &image, &v, n), "parent of {i}");
        }
    }

    /// Decision 0463: a 16z tape's reads stay within it and its cache, whatever its blocks hold. A side length past
    /// what its block's bytes decode to is refused when the tape is read in. A block whose literals claim more
    /// bytes than it has is decoded no further than its own: the tape lies against a page no read may touch, and
    /// every node is read. A node past the tape's last reads the cache's dead entry, a failure counted. The cache
    /// is found as it was made (`Spec::check`), and its guard overwritten is found.
    #[cfg(unix)]
    #[test]
    fn a_16z_tapes_reads_stay_within_its_bytes() {
        use rerail_clif::view::cache;
        let reads = rerail_clif::jit::Reads::new();
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let plain = Plain(&nodes, &table);
        let Ok(Written::F16Z(z)) = write_from(&lang, &plain, &(0..600).collect::<Vec<_>>(), &source_of(pos + 2), 0, true) else { panic!("16z") };
        let (n, blocks) = (z.len(), z.len().div_ceil(Z));
        // A piece that states more bytes decoded than its stored bytes can give: refused. Its decoded length is
        // its column's and its side part's, which size every cache entry (decision 0463).
        let packed = (0..PIECES * blocks).rev().find(|&j| z.kind(j) == LZ4).expect("an LZ4 piece");
        let mut stated = z.as_bytes().to_vec();
        let at = z.zat[ZLENS] + 4 * packed;
        stated[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        let refused = Tape16Z::from_bytes(stated).expect_err("refused");
        assert!(refused.contains(&format!("piece {packed} does not fit")), "{refused}");
        // The last LZ4 piece's literals made to claim more bytes than the piece has, within its decoded room.
        let mut bytes = z.as_bytes().to_vec();
        let (start, len) = z.span(packed);
        let (from, to) = (HEAD16 + start, HEAD16 + start + len);
        let room = z.decoded(packed);
        let claim = (to - from + 64).min(room - 16);
        assert!(claim > to - from, "a piece that compresses: {} bytes for {room}", to - from);
        let extra = claim - 15;
        let mut block = vec![0xF0u8];
        block.extend(std::iter::repeat_n(255u8, extra / 255));
        block.push((extra % 255) as u8);
        assert!(block.len() < to - from, "the claim fits the piece's bytes");
        bytes[from..from + block.len()].copy_from_slice(&block);
        let guarded = Guarded::new(&bytes);
        let tape = guarded.at;
        let reader = rerail_clif::view::Reader::new(&z.view(&lang).unwrap(), tape);
        let f = ["tape_number", "tape_start", "tape_end", "tape_parent", "tape_size", "tape_next", "tape_child_count", "tape_first_child", "tape_length"].map(|name| reads.function(name));
        let read = |i: u32| {
            for &g in &f {
                // SAFETY: a read's signature: the tape's base, its view, a node.
                let g: extern "C" fn(*const u8, *const TapeView, u32) -> u32 = unsafe { std::mem::transmute(g) };
                std::hint::black_box(g(tape, reader.view(), i));
            }
        };
        for i in 0..n as u32 {
            read(i);
        }
        assert_eq!(reader.counts().expect("a 16z cache").failed, 0, "every node of the tape found its block");
        // Nodes past the last: the dead entry, each a failure.
        for i in n as u32..(blocks + 3) as u32 * Z as u32 {
            read(i);
        }
        read(u32::MAX);
        assert!(reader.counts().expect("a 16z cache").failed > 0, "a node past the tape counted");
        drop(reader);
        drop(guarded);
        // A cache overwritten past its slack, or in its shape, is found.
        let spec = cache::Spec::new(3, 4, 4096);
        let mut buf = vec![0u8; spec.bytes()];
        spec.init(&mut buf).unwrap();
        assert_eq!(cache::Spec::check(&buf), Ok(spec));
        let end = buf.len() - 1;
        buf[end] ^= 1;
        assert!(cache::Spec::check(&buf).unwrap_err().contains("guard"));
        buf[end] ^= 1;
        buf[cache::DEAD as usize] ^= 8;
        assert!(cache::Spec::check(&buf).unwrap_err().contains("overwritten"));
    }

    /// `bytes` placed so their last lies against a page no read may touch (64 KiB: a page or several, on every
    /// host), at `at`; unmapped when dropped.
    struct Guarded {
        map: *mut u8,
        span: usize,
        at: *mut u8,
    }

    unsafe extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut u8;
        fn mprotect(addr: *mut u8, len: usize, prot: i32) -> i32;
        fn munmap(addr: *mut u8, len: usize) -> i32;
    }

    impl Guarded {
        fn new(bytes: &[u8]) -> Guarded {
            const GUARD: usize = 1 << 16;
            #[cfg(target_os = "macos")]
            const ANON: i32 = 0x1000;
            #[cfg(not(target_os = "macos"))]
            const ANON: i32 = 0x20;
            let span = bytes.len().next_multiple_of(GUARD) + GUARD;
            // SAFETY: a fresh private mapping of `span` bytes; its last `GUARD` bytes made unreadable.
            let map = unsafe { mmap(std::ptr::null_mut(), span, 3, 2 | ANON, -1, 0) };
            assert!(!map.is_null() && map as isize != -1, "a mapping");
            assert_eq!(unsafe { mprotect(map.add(span - GUARD), GUARD, 0) }, 0, "a page no read may touch");
            // SAFETY: the readable part of the mapping holds the bytes, ending at the unreadable page.
            let at = unsafe { map.add(span - GUARD - bytes.len()) };
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), at, bytes.len()) };
            Guarded { map, span, at }
        }
    }

    impl Drop for Guarded {
        fn drop(&mut self) {
            // SAFETY: the mapping `new` made, no read of it left.
            unsafe { munmap(self.map, self.span) };
        }
    }

    /// Decision 0463: a node's span, from the tape's gaps and lengths, is held to the source its header states:
    /// a tape whose spans claim more than its source reads none past it, in format 16 or 16z, and none reversed.
    #[test]
    fn a_tapes_spans_stay_within_its_source() {
        let reads = rerail_clif::jit::Reads::new();
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let plain = Plain(&nodes, &table);
        let source = pos / 2;
        let f = ["tape_start", "tape_end"].map(|name| reads.function(name));
        // SAFETY: a read's signature: the tape's base, its view, a node.
        let [start, end] = f.map(|g| unsafe { std::mem::transmute::<_, extern "C" fn(*const u8, *const TapeView, u32) -> u32>(g) });
        // The source's last byte against a page no read may touch: each node's span read byte by byte, as a
        // kernel's text test reads it (`same_text`, `text_is`).
        let text = Guarded::new(&vec![b'x'; source as usize]);
        let check = |tape: *const u8, view: *const TapeView, n: usize| {
            let mut held = 0;
            let mut sum = 0u64;
            for i in 0..n as u32 {
                let (s, e) = (start(tape, view, i), end(tape, view, i));
                assert!(s as u64 <= source && s <= e && e as u64 <= source, "node {i}: {s}..{e} of a source of {source}");
                for k in s..e {
                    // SAFETY: what the test asks: the span, read where the source lies.
                    sum += u64::from(unsafe { text.at.add(k as usize).read_volatile() });
                }
                held += usize::from(e as u64 == source);
            }
            assert!(held > 1 && sum > 0, "spans past the source, held to its end");
        };
        // Written over the whole source, then read with a shorter one stated: a tape held with the wrong source, or
        // corrupt. (Its header's length is checked against its newline index when it is read in; the view's is
        // what the reads hold spans to.)
        let Ok(Written::F16(t)) = write_from(&lang, &plain, &(0..600).collect::<Vec<_>>(), &source_of(pos + 2), 0, false) else { panic!("16") };
        let mut view = placement(t.as_bytes()).unwrap().view(&lang, 0).unwrap();
        assert_eq!(u64::from(view.source_len), pos + 2);
        view.source_len = source as u32;
        check(t.as_bytes().as_ptr(), &raw const view, t.len());
        let Ok(Written::F16Z(z)) = write_from(&lang, &plain, &(0..600).collect::<Vec<_>>(), &source_of(pos + 2), 0, true) else { panic!("16z") };
        let mut view = z.view(&lang).unwrap();
        view.source_len = source as u32;
        let reader = rerail_clif::view::Reader::new(&view, z.as_bytes().as_ptr());
        check(z.as_bytes().as_ptr(), reader.view(), z.len());
    }

    /// The device's kernel: every read of node `i` (`DEVICE_READS` of them) into its slots, from the reads as UKL
    /// (`READS_HEADER`: `<ukl/reads/tape.h>`, or `tape_z.h`, which reads 16z too), the view read where its buffer holds
    /// it. `parentEnd[i]` is the end of `i`'s parent's subtree (`tape_next_in`'s bound).
    const DEVICE_KERNEL: &str = r#"
#define CLIF_VIEW_SPACE UKL_GLOBAL
#include READS_HEADER

typedef struct ReadArgs {
    uint32_t n;
    uint32_t pad;
} ReadArgs;
UKL_REFLECT_STRUCT(ReadArgs, n, pad)

DEFINE_KERNEL2(tests, t_reads,
               ARGS,     ReadArgs, args,      ,
               RWBUFFER, uint8_t,  tape,      "[bytes]",
               ROBUFFER, uint32_t, view,      "[29]",
               ROBUFFER, uint32_t, parentEnd, "[n]",
               RWBUFFER, uint32_t, out,       "[n * 23]",
               GID0,     uint32_t, i,         )
{
    if (i >= args.n)
        return;
    RWBUFFER(uint32_t) o = out + i * 23u;
    o[0] = clif_tape_size(tape, view, i);
    o[1] = clif_tape_end_of(tape, view, i);
    o[2] = clif_tape_number(tape, view, i);
    o[3] = clif_tape_start(tape, view, i);
    o[4] = clif_tape_end(tape, view, i);
    o[5] = clif_tape_parent(tape, view, i);
    o[6] = clif_tape_next(tape, view, i);
    o[7] = clif_tape_next_in(tape, view, i, parentEnd[i]);
    o[8] = clif_tape_first_child(tape, view, i);
    o[9] = clif_tape_child_count(tape, view, i);
    for (uint32_t k = 0u; k <= 12u; k++)
        o[10u + k] = clif_tape_child(tape, view, i, k);
}
"#;
    const DEVICE_READS: usize = 23;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ReadArgs {
        n: u32,
        pad: u32,
    }
    unsafe impl rerail_gpu::Pod for ReadArgs {}

    /// `t`'s every read of every node, as the kernel lays them out; `parent_end` each node's parent's subtree's end.
    fn reads_of<T: Tape>(t: &T) -> (Vec<u32>, Vec<u32>) {
        const NONE: u32 = u32::MAX;
        let none = |v: Option<usize>| v.map_or(NONE, |v| v as u32);
        let mut want = Vec::with_capacity(t.len() * DEVICE_READS);
        let mut parent_end = Vec::with_capacity(t.len());
        for i in 0..t.len() {
            let end = t.subtree_end(i);
            let parent = t.parent(i);
            let next = parent.and_then(|p| Some(end).filter(|&n| n < t.subtree_end(p)));
            let kids = crate::tape::children(t, i).collect::<Vec<_>>();
            parent_end.push(parent.map_or(0, |p| t.subtree_end(p) as u32));
            want.extend([(end - i) as u32, end as u32, t.code(i), t.start(i) as u32, t.end(i) as u32, none(parent), none(next)]);
            // `tape_next_in` is read only under a parent.
            want.push(if parent.is_some() { none(next) } else { NONE });
            want.extend([none(kids.first().copied()), kids.len() as u32]);
            want.extend((0..=12).map(|k| none(kids.get(k).copied())));
        }
        (want, parent_end)
    }

    /// The device's reads of every node of `t`, whose bytes are `tape` (with a cache after them for 16z) and whose
    /// view is `view`, equal `t`'s own.
    fn same_device_reads<T: Tape>(program: &rerail_gpu::kernels::Program, what: &str, t: &T, tape: &[u8], view: &TapeView) {
        use rerail_gpu::kernels::{alloc, buffer_of, launch, read};
        let shared = rerail_gpu::shared::get().expect("a device");
        let mut q = rerail_gpu::device::ComputeQueue::new(&shared.queue);
        let n = t.len();
        let started = std::time::Instant::now();
        let (want, parent_end) = reads_of(t);
        // The bytes, readable through the word that holds the last one.
        let mut bytes = tape.to_vec();
        bytes.resize(tape.len().next_multiple_of(8) + 8, 0);
        // SAFETY: the view is plain u32s.
        let words = unsafe { std::slice::from_raw_parts(std::ptr::from_ref(view).cast::<u32>(), size_of::<TapeView>() / 4) };
        let (tb, vb, pb) = (buffer_of(&bytes).unwrap(), buffer_of(words).unwrap(), buffer_of(&parent_end).unwrap());
        let out = alloc::<u32>(n * DEVICE_READS).unwrap();
        launch(&q, program.function("t_reads").unwrap())
            .value("args", &ReadArgs { n: n as u32, pad: 0 })
            .buffer("tape", &tb)
            .buffer("view", &vb)
            .buffer("parentEnd", &pb)
            .buffer("out", &out)
            .run(&[n.div_ceil(64) as u64], &[64])
            .unwrap();
        q.flush().await_().unwrap();
        let got: Vec<u32> = read(&out, n * DEVICE_READS);
        eprintln!("{}: {what}: {n} nodes read in {:.2} s", rerail_gpu::device::backend(), started.elapsed().as_secs_f64());
        const NAMES: [&str; 10] = ["size", "end of subtree", "code", "start", "end", "parent", "next", "next in parent", "first child", "child count"];
        for i in 0..n {
            let (g, w) = (&got[i * DEVICE_READS..][..DEVICE_READS], &want[i * DEVICE_READS..][..DEVICE_READS]);
            for k in 0..DEVICE_READS {
                if k == 7 && t.parent(i).is_none() {
                    continue;
                }
                let name = NAMES.get(k).map_or_else(|| format!("child {}", k - 10), |s| s.to_string());
                assert_eq!(g[k], w[k], "{}: {what}: node {i}: {name}", rerail_gpu::device::backend());
            }
        }
    }

    /// Decision 0400 step 6 (0310 §4's test, per backend): the reads as UKL, compiled for this process's GPU (Metal
    /// on macOS, CUDA on Linux), read every node as this crate's readers do: format 16 with every side list in use,
    /// 16z through a cache the device's threads share (sized for the whole tape, so no read finds it full), and
    /// format 15. `<ukl/reads/tape.h>` reads 16 and 15; `tape_z.h` reads all three. Without a device it passes,
    /// saying so.
    #[test]
    fn the_device_reads_read_every_node_as_the_crate_does() {
        use rerail_clif::view::cache;
        if rerail_gpu::shared::get().is_none() {
            eprintln!("no device: the reads as UKL did not run");
            return;
        }
        let program = |header: &str| {
            let text: &'static str = Box::leak(DEVICE_KERNEL.replace("READS_HEADER", &format!("<ukl/reads/{header}>")).into_boxed_str());
            let name: &'static str = Box::leak(format!("tape_reads_{header}").into_boxed_str());
            let started = std::time::Instant::now();
            let p = rerail_gpu::kernels::program(name, text).unwrap_or_else(|e| panic!("{}: {header}: {e}", rerail_gpu::device::backend()));
            eprintln!("{}: {header} compiled in {:.2} s", rerail_gpu::device::backend(), started.elapsed().as_secs_f64());
            p
        };
        let (plain_reads, z_reads) = (program("tape.h"), program("tape_z.h"));
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let plain = Plain(&nodes, &table);
        // Format 16.
        let t = sixteen(&lang, &plain, &(0..600).collect::<Vec<_>>(), pos + 2);
        let v = placement(t.as_bytes()).unwrap().view(&lang, 0).unwrap();
        for (reads, what) in [(plain_reads, "16, tape.h"), (z_reads, "16, tape_z.h")] {
            same_device_reads(reads, what, &t.reader(&lang).unwrap(), t.as_bytes(), &v);
        }
        // 16z: the tape, then its cache (each block decoded once into it), the view naming the cache by its offset.
        let Ok(Written::F16Z(z)) = write_from(&lang, &plain, &(0..600).collect::<Vec<_>>(), &source_of(pos + 2), 0, true) else { panic!("16z") };
        let mut view = z.view(&lang).unwrap();
        let blocks = z.p.block_count() as u32;
        let spec = cache::Spec::new(3, blocks + 1, view.columns[3] as usize);
        let at = z.as_bytes().len().next_multiple_of(64);
        let mut bytes = z.as_bytes().to_vec();
        bytes.resize(at + spec.bytes(), 0);
        spec.init(&mut bytes[at..]).unwrap();
        (view.side[0], view.side[1], view.columns[3], view.columns[4]) = (at as u32, 0, spec.entry_bytes() as u32, 1);
        same_device_reads(z_reads, "16z, tape_z.h", &z.reader(&lang).unwrap(), &bytes, &view);
        // Format 15.
        let (image, _) = crate::tape::tests::sample();
        let (columns, table) = crate::tape::f15_columns(&image).unwrap();
        let f15 = crate::tape::F15::new(&image, columns, &table).unwrap();
        let v = crate::tape::f15_view(&image, 0).unwrap();
        for (reads, what) in [(plain_reads, "15, tape.h"), (z_reads, "15, tape_z.h")] {
            same_device_reads(reads, what, &f15, &image, &v);
        }
    }

    /// Decision 0353 step 4b: a rolling cache shared by threads (`view::cache`, its stamps checked after each read)
    /// reads a 16z tape as this crate does while it turns: eight threads read nodes at random through one cache of
    /// three generations of four blocks, over a tape of more blocks than it holds. A read wrong only where the
    /// cache counted a failure (no room: the dead entry read).
    #[test]
    fn a_shared_cache_reads_16z_as_the_crate_does() {
        use rerail_clif::view::cache;
        let reads = rerail_clif::jit::Reads::new();
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let plain = Plain(&nodes, &table);
        let Ok(Written::F16Z(z)) = write_from(&lang, &plain, &(0..600).collect::<Vec<_>>(), &source_of(pos + 2), 0, true) else { panic!("16z") };
        let t = z.reader(&lang).unwrap();
        let n = t.len();
        let want: Vec<[u32; 4]> = (0..n).map(|i| [t.code(i), t.start(i) as u32, t.end(i) as u32, t.parent(i).map_or(u32::MAX, |p| p as u32)]).collect();
        let mut view = z.view(&lang).unwrap();
        let spec = cache::Spec::new(3, 4, view.columns[3] as usize).with_stats();
        let mut buf = vec![0u128; spec.bytes().div_ceil(16)];
        // SAFETY: the buffer's bytes, 16-aligned.
        let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), spec.bytes()) };
        spec.init(bytes).unwrap();
        let rel = (bytes.as_ptr() as i64).wrapping_sub(z.as_bytes().as_ptr() as i64);
        (view.side[0], view.side[1], view.columns[3], view.columns[4]) = (rel as u32, (rel >> 32) as u32, spec.entry_bytes() as u32, 1);
        let cache_at = bytes.as_ptr() as usize;
        let failed = || unsafe { (*((cache_at + cache::FAILED as usize) as *const std::sync::atomic::AtomicU32)).load(std::sync::atomic::Ordering::Relaxed) };
        let f = ["tape_number", "tape_start", "tape_end", "tape_parent"].map(|name| reads.function(name) as usize);
        let (tape, v) = (z.as_bytes().as_ptr() as usize, std::ptr::from_ref(&view) as usize);
        let wrong: usize = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8u64)
                .map(|seed| {
                    let want = &want;
                    scope.spawn(move || {
                        let mut x = seed * 0x9E37_79B9 + 1;
                        let mut wrong = 0;
                        for _ in 0..40_000 {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                            let i = (x % n as u64) as u32;
                            for (k, &g) in f.iter().enumerate() {
                                // SAFETY: a read's signature: the tape's base, its view, a node.
                                let g: extern "C" fn(*const u8, *const TapeView, u32) -> u32 = unsafe { std::mem::transmute(g) };
                                if g(tape as *const u8, v as *const TapeView, i) != want[i as usize][k] {
                                    wrong += 1;
                                }
                            }
                        }
                        wrong
                    })
                })
                .collect();
            threads.into_iter().map(|t| t.join().unwrap()).sum()
        });
        // SAFETY: the cache's header, its threads done.
        let counts = unsafe { cache::Counts::read(cache_at as *const u8) };
        eprintln!("{counts:?}, hit ratio {:.3}, wrong {wrong}", counts.hit_ratio());
        assert!(counts.rotated > 100 && counts.promoted > 0 && counts.hits > 0, "the cache turned: {counts:?}");
        assert!(wrong <= failed() as usize, "{wrong} reads wrong, {} failures", failed());
        drop(buf);
    }

    /// Every shipped table loads, and its text reads back to the same choices.
    #[test]
    fn shipped_tables_load_and_read_back() {
        let tags: Vec<u8> = (0..=255).filter(|&t| tables::text(t).is_some()).collect();
        assert!(!tags.is_empty(), "no shipped tables");
        for tag in tags {
            let l = lang(tag).unwrap();
            assert!(!l.table().is_empty(), "tag {tag}");
            let back = Lang::from_text(&l.to_text("again")).unwrap();
            assert_eq!(back.table().entries(), l.table().entries(), "tag {tag}");
            assert_eq!((back.widths, back.revision, back.cut), (l.widths, l.revision, l.cut), "tag {tag}");
        }
        assert!(lang(255).is_none());
    }

    /// A tape of another revision is not read.
    #[test]
    fn another_revision_is_refused() {
        let (image, _) = crate::tape::tests::sample();
        let (columns, table) = f15_columns(&image).unwrap();
        let f15 = F15::new(&image, columns, &table).unwrap();
        let lang = lang_of(&table);
        let t = sixteen(&lang, &f15, &numbers(&lang, &table).unwrap(), 2_000);
        let other = Lang::new(table.clone(), [1, 1, 1], 2).unwrap();
        assert!(t.reader(&other).is_err());
    }

    /// A tape of one block is the regions end to end (decision 0434, R2): each column at the node count's offset
    /// from the header, the tail after.
    #[test]
    fn one_block_is_the_regions() {
        let (image, _) = crate::tape::tests::sample();
        let (columns, table) = f15_columns(&image).unwrap();
        let f15 = F15::new(&image, columns, &table).unwrap();
        let lang = lang_of(&table);
        let t = sixteen(&lang, &f15, &numbers(&lang, &table).unwrap(), 2_000);
        let (p, n) = (t.p, t.len());
        assert_eq!(p.block_count(), 1);
        for r in 0..COLUMNS {
            assert_eq!(p.column(0, r), HEAD16 + n * before(p.widths, r), "column {r}");
        }
    }

    /// Writes `nodes` (code, start, end, size) by the ops, committing every `every` nodes.
    fn by_ops(w: &mut impl WriterOps, nodes: &[(u32, u64, u64, usize)], every: usize) {
        let mut ends: Vec<(usize, u64)> = Vec::new();
        for (i, &(code, start, end, size)) in nodes.iter().enumerate() {
            while ends.last().is_some_and(|e| e.0 <= i) {
                w.close(ends.pop().unwrap().1);
            }
            if size == 1 {
                w.leaf(code, start, end);
            } else {
                w.open(code, start);
                ends.push((i + size, end));
            }
            if i % every == 0 {
                w.commit();
            }
        }
        while let Some((_, end)) = ends.pop() {
            w.close(end);
        }
    }

    /// The ops write the tape `write` writes from the same nodes, in 16 and 16z (decision 0434, R5); with the nodes
    /// hinted, the format 16 tape is the writer's own room, the last block moved down and the tail appended in
    /// place (R4); the presence is the nodes' kinds and fields, each block's and the document's (R10).
    #[test]
    fn ops_write_the_tape() {
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k % 300, field: k % 7, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let source = source_of(pos + 2);
        let number: Vec<u32> = (0..600).collect();
        let Ok(Written::F16(want)) = write_from(&lang, &Plain(&nodes, &table), &number, &source, 0, false) else { panic!("format 16") };
        let mut in_place = 0;
        for (z, every) in [(false, 1), (false, 1_000_000), (true, 1), (true, 97)] {
            let mut w = Writer::new(&lang, &source, 0, z, true, nodes.len());
            by_ops(&mut w, &nodes, every);
            let room = w.buffer(rerail_clif::writer::part::OUT);
            let (room, cap) = (room.as_ptr(), room.len());
            let f = w.finish().unwrap();
            match &f.tape {
                Written::F16(t) => {
                    assert_eq!(t.as_bytes(), want.as_bytes(), "16 by the ops");
                    // A tape within the room the nodes' hint made is the room; one whose side lists outgrow it
                    // moves as its buffer grows, wherever the allocator puts it (an escaped tape's: the hint is
                    // the nodes', not their side lists').
                    if t.as_bytes().len() <= cap {
                        assert_eq!(t.as_bytes().as_ptr(), room, "the tape is the writer's room");
                        in_place += 1;
                    }
                }
                Written::F16Z(t) => {
                    let mut d = vec![0u8; t.decoded_len()];
                    t.decode_into(&mut d).unwrap();
                    assert_eq!(d, want.as_bytes(), "16z by the ops (commit every {every})");
                }
            }
            let p = &f.tape.presence();
            let mut doc = vec![0u64; p.kind_words + p.field_words];
            for b in 0..nodes.len().div_ceil(Z) {
                let mut want = vec![0u64; p.kind_words + p.field_words];
                for &(code, ..) in &nodes[b * Z..nodes.len().min((b + 1) * Z)] {
                    let e = &table.entries()[code as usize];
                    want[usize::from(e.kind) / 64] |= 1 << (e.kind % 64);
                    want[p.kind_words + usize::from(e.field) / 64] |= 1 << (e.field % 64);
                }
                let (kinds, fields) = p.block(b).unwrap();
                assert_eq!((kinds, fields), want.split_at(p.kind_words), "block {b}'s presence");
                for (d, x) in doc.iter_mut().zip(&want) {
                    *d |= x;
                }
            }
            assert_eq!(p.doc, doc, "the document's presence");
            assert!(p.has_kind(299) && !p.has_kind(300) && p.has_field(6), "its bits");
            // The tape's read (`tape_has_kind`, the kernels' too) answers each block's and the document's, where it lies.
            let has = |b: u32, kind: u16| match &f.tape {
                Written::F16(t) => t.reader(&lang).unwrap().has_kind(b, kind),
                Written::F16Z(t) => t.reader(&lang).unwrap().has_kind(b, kind),
            };
            let blocks = nodes.len().div_ceil(Z) as u32;
            for kind in (0..320u16).chain([400, 2000]) {
                for b in 0..blocks {
                    let want = p.block(b as usize).is_some_and(|(kinds, _)| kinds.get(usize::from(kind) / 64).is_some_and(|w| w >> (kind % 64) & 1 != 0));
                    assert_eq!(has(b, kind), want, "block {b}, kind {kind} (16z {z})");
                }
                assert_eq!(has(u32::MAX, kind), p.has_kind(kind) && usize::from(kind) < 64 * p.kind_words, "the document, kind {kind}");
                assert_eq!(has(blocks, kind), has(u32::MAX, kind), "past the last block: the document's");
            }
        }
        assert_eq!(in_place, 0, "the escaped tape's side lists outgrow the hint's room");
        // A tape of few escapes (its side lists within the hint's `nodes / 2` bytes) is the writer's own room (R4).
        let n = 3000usize;
        let mut plain: Vec<(u32, u64, u64, usize)> = (1..n as u64).map(|i| (1 + (i % 3) as u32, 4 * i, 4 * i + 2, 1)).collect();
        plain.insert(0, (0, 0, 4 * n as u64, n));
        let source = source_of(4 * n as u64);
        for every in [1, 1_000_000] {
            let mut w = Writer::new(&lang, &source, 0, false, false, plain.len());
            by_ops(&mut w, &plain, every);
            let room = w.buffer(rerail_clif::writer::part::OUT).as_ptr();
            let Ok(Finished { tape: Written::F16(t), .. }) = w.finish() else { panic!("format 16") };
            assert_eq!(t.as_bytes().as_ptr(), room, "the tape is the writer's room (commit every {every})");
        }
    }

    /// The edits that reach a node before the window (decision 0434 §2): each inner node opened with another code
    /// and the node before's start, then retagged and its start set once its first child is open, gives the tape of
    /// the nodes written right, in 16 and 16z, escaped codes and gaps among them (R6).
    #[test]
    fn retag_and_set_start() {
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k % 300, field: k % 7, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let source = source_of(pos + 2);
        let number: Vec<u32> = (0..600).collect();
        let Ok(Written::F16(want)) = write_from(&lang, &Plain(&nodes, &table), &number, &source, 0, false) else { panic!("format 16") };
        for z in [false, true] {
            let mut w = Writer::new(&lang, &source, 0, z, false, nodes.len());
            let mut ends: Vec<(usize, u64)> = Vec::new();
            // The inner node whose first child is next: its code and start, given late.
            let mut late: Option<(usize, u32, u64)> = None;
            for (i, &(code, start, end, size)) in nodes.iter().enumerate() {
                while ends.last().is_some_and(|e| e.0 <= i) {
                    w.close(ends.pop().unwrap().1);
                }
                if size == 1 {
                    w.leaf(code, start, end);
                } else {
                    let early = if i == 0 { start } else { nodes[i - 1].1 };
                    w.open((code + 301) % 600, early);
                    ends.push((i + size, end));
                }
                if let Some((p, code, start)) = late.take() {
                    w.retag(p, code);
                    w.set_start(p, start);
                }
                if size > 1 {
                    late = Some((i, code, start));
                }
                w.commit();
            }
            while let Some((_, end)) = ends.pop() {
                w.close(end);
            }
            match w.finish().unwrap().tape {
                Written::F16(t) => assert_eq!(t.as_bytes(), want.as_bytes(), "16 retagged"),
                Written::F16Z(t) => {
                    let mut d = vec![0u8; t.decoded_len()];
                    t.decode_into(&mut d).unwrap();
                    assert_eq!(d, want.as_bytes(), "16z retagged");
                }
            }
        }
    }

    /// A 16z writer's block is final once its nodes are closed, the node after it has opened and the frontier is
    /// past it: the blocks under the open root finish as the parse leaves them, the root's last, and a final block's
    /// room serves the next (decision 0434, R9).
    #[test]
    fn blocks_finish_when_closed() {
        let entries: Vec<Entry> = (0..4u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let n = 3 * Z + 5;
        let source = source_of(4 * n as u64);
        let mut w = reference::Writer::new(&lang, &source, 0, true, false, n);
        w.open(0, 0);
        for i in 1..n {
            w.leaf(1 + (i % 3) as u32, 4 * i as u64, 4 * i as u64 + 2);
            if i == 2 * Z {
                assert_eq!(&w.done[..3], &[false, true, false], "block 1 is final once node {i} opens; block 0 holds the root");
            }
            w.commit();
            if i == 3 * Z {
                assert_eq!(&w.done[..4], &[false, true, true, false]);
                assert_eq!((w.free.len(), w.base.len()), (1, 4), "a final block's room serves the next");
            }
        }
        w.close(4 * n as u64);
        assert!(w.done[0], "the root's block is final at its close");
        let Ok(Finished { tape: Written::F16Z(z), .. }) = w.finish() else { panic!("16z") };
        let mut nodes: Vec<(u32, u64, u64, usize)> = (1..n as u64).map(|i| (1 + (i % 3) as u32, 4 * i, 4 * i + 2, 1)).collect();
        nodes.insert(0, (0, 0, 4 * n as u64, n));
        same(&Plain(&nodes, &table), &z.reader(&lang).unwrap(), &[0, 1, 2, 3]);
    }

    /// A node that breaks the tape's bounds makes no tape: a start before the one before it, an end past the source,
    /// a node after the root's close, a node left open.
    #[test]
    fn broken_nodes_make_no_tape() {
        let entries: Vec<Entry> = (0..4u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let source = source_of(100);
        let cases: [fn(&mut Writer); 4] = [
            |w| {
                w.open(0, 10);
                w.leaf(1, 5, 6);
                w.close(20);
            },
            |w| {
                w.open(0, 0);
                w.close(101);
            },
            |w| {
                w.leaf(0, 0, 10);
                w.leaf(1, 10, 20);
            },
            |w| {
                w.open(0, 0);
                w.leaf(1, 0, 1);
            },
        ];
        for (k, case) in cases.into_iter().enumerate() {
            let mut w = Writer::new(&lang, &source, 0, false, false, 4);
            case(&mut w);
            assert!(w.finish().is_err(), "case {k}");
        }
    }


    /// `nodes` written by `w`'s ops, a commit every `every` nodes; with `late`, each inner node opened with another
    /// code and the node before's start, then retagged and its start set once its first child is open.
    fn by_ops_of(w: &mut impl WriterOps, nodes: &[(u32, u64, u64, usize)], every: usize, late: bool) {
        let mut ends: Vec<(usize, u64)> = Vec::new();
        let mut held: Option<(usize, u32, u64)> = None;
        for (i, &(code, start, end, size)) in nodes.iter().enumerate() {
            while ends.last().is_some_and(|e| e.0 <= i) {
                w.close(ends.pop().unwrap().1);
            }
            if size == 1 {
                w.leaf(code, start, end);
            } else if late {
                let early = if i == 0 { start } else { nodes[i - 1].1 };
                w.open((code + 301) % 600, early);
                ends.push((i + size, end));
            } else {
                w.open(code, start);
                ends.push((i + size, end));
            }
            if let Some((p, code, start)) = held.take() {
                w.retag(p, code);
                w.set_start(p, start);
            }
            if late && size > 1 {
                held = Some((i, code, start));
            }
            if i % every == 0 {
                w.commit();
            }
        }
        while let Some((_, end)) = ends.pop() {
            w.close(end);
        }
    }

    /// Decision 0434 §6, R16: the crate's writer is the ops in CLIF, the hand-written one the tests' oracle: a node
    /// that breaks the tape's bounds is the ops' failure, kept as its code and node (the reference keeps a message),
    /// and `write_from` writes the reference's tape, in 16 and 16z.
    #[test]
    fn the_writer_is_the_clif_ops() {
        use rerail_clif::writer::fail;
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k % 300, field: k % 7, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let source = source_of(pos + 2);
        let mut w = Writer::new(&lang, &source, 0, false, false, 4);
        w.open(1, 10);
        w.leaf(2, 5, 6);
        assert_eq!(w.failure(), Some((fail::BOUNDS, 1)), "a start before the one before it, at node 1");
        assert!(w.finish().is_err());
        let number: Vec<u32> = (0..600).collect();
        for z in [false, true] {
            let bytes = |w: Written| match w {
                Written::F16(t) => t.as_bytes().to_vec(),
                Written::F16Z(t) => t.as_bytes().to_vec(),
            };
            let got = bytes(write_from(&lang, &Plain(&nodes, &table), &number, &source, 0, z).unwrap());
            let want = bytes(reference::write_from(&lang, &Plain(&nodes, &table), &number, &source, 0, z).unwrap());
            assert!(got == want, "16z {z}: the writer's tape is the reference's");
        }
    }

    /// Decision 0434 §6, R13: the writer's ops in CLIF, through their Rust, write the reference writer's tape and
    /// presence from the same ops: escaped codes and values, late closes, a commit at every node or none, `retag`
    /// and `set_start`, a presence a block; and a tape of one node, and of none.
    #[test]
    fn the_clif_ops_write_the_references_tape() {
        writes_the_references_tape(|lang, source, z, blocks, nodes| Writer::new(lang, source, 3, z, blocks, nodes / 3));
    }

    /// One writer, used again for every case of `writes_the_references_tape` in turn (`ops::Writer::begin`): its
    /// parts kept from the document before (16 and 16z, larger and smaller, with a presence a block and without),
    /// each tape and presence the reference writer's. A byte the ops read before they write shows, as `begin` fills
    /// every part kept with `FRESH` in tests.
    #[test]
    fn a_writer_used_again_writes_the_references_tape() {
        struct Again {
            w: Option<Writer>,
            home: std::rc::Rc<std::cell::RefCell<Option<Writer>>>,
        }
        impl WriterOps for Again {
            fn open(&mut self, number: u32, start: u64) -> usize {
                self.w.as_mut().unwrap().open(number, start)
            }
            fn close(&mut self, end: u64) {
                self.w.as_mut().unwrap().close(end)
            }
            fn leaf(&mut self, number: u32, start: u64, end: u64) -> usize {
                self.w.as_mut().unwrap().leaf(number, start, end)
            }
            fn commit(&mut self) {
                self.w.as_mut().unwrap().commit()
            }
            fn retag(&mut self, i: usize, number: u32) {
                self.w.as_mut().unwrap().retag(i, number)
            }
            fn set_start(&mut self, i: usize, start: u64) {
                self.w.as_mut().unwrap().set_start(i, start)
            }
            fn finish(mut self) -> Result<Finished, String> {
                let mut w = self.w.take().unwrap();
                let f = Writer::finish(&mut w);
                *self.home.borrow_mut() = Some(w);
                f
            }
        }
        let home = std::rc::Rc::new(std::cell::RefCell::new(None::<Writer>));
        let used = std::cell::Cell::new(0);
        writes_the_references_tape(|lang, source, z, blocks, nodes| {
            let w = match home.borrow_mut().take() {
                Some(mut w) => {
                    used.set(used.get() + 1);
                    w.begin(lang, source, 3, z, blocks, nodes / 3);
                    w
                }
                None => Writer::new(lang, source, 3, z, blocks, nodes / 3),
            };
            Again { w: Some(w), home: home.clone() }
        });
        assert!(used.get() > 50, "the writer used again: {} times", used.get());
    }

    /// A tape given back (`ops::Writer::recycle`) is the next document's room: the next tape is written in it, in
    /// place, and is the tape a new writer writes.
    #[test]
    fn a_tape_given_back_is_the_next_tapes_room() {
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k % 300, field: k % 7, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let source = source_of(pos + 2);
        let mut w = Writer::new(&lang, &source, 0, false, true, nodes.len());
        by_ops(&mut w, &nodes, 1);
        let first = Writer::finish(&mut w).expect("the first tape").tape;
        let want = first.extent().to_vec();
        let at = first.extent().as_ptr();
        w.recycle(first);
        w.begin(&lang, &source, 0, false, true, nodes.len());
        by_ops(&mut w, &nodes, 1);
        let again = Writer::finish(&mut w).expect("the second tape").tape;
        assert_eq!(again.extent(), &want[..], "the tape written in the room given back");
        assert_eq!(again.extent().as_ptr(), at, "the tape is the room given back");
    }

    /// Decision 0434 §6, R15: the writer's ops compiled by the JIT write the reference writer's tape and presence:
    /// R13's cases.
    #[test]
    fn the_jit_ops_write_the_references_tape() {
        let jit = ops::Jit::leaked();
        writes_the_references_tape(|lang, source, z, blocks, nodes| {
            let mut w = Writer::new(lang, source, 3, z, blocks, nodes / 3);
            w.jit = Some(jit);
            w
        });
    }

    /// Decision 0434 §6, R15: the writer's ops in UKL (`<ukl/writer.h>`), on the process's device, write the reference
    /// writer's tape and presence: R13's cases. Without a device it passes, saying so.
    #[test]
    fn the_device_writes_the_references_tape() {
        if rerail_gpu::shared::get().is_none() {
            eprintln!("no device: the writer's ops in UKL did not run");
            return;
        }
        writes_the_references_tape(|lang, source, z, blocks, nodes| ops::device::DeviceOps::new(lang, source, 3, z, blocks, nodes / 3));
    }

    /// R13's cases: `make(lang, source, z, blocks, nodes)`'s writer and the reference, from the same ops.
    fn writes_the_references_tape<W: WriterOps>(make: impl Fn(&Lang, &[u8], bool, bool, usize) -> W) {
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k % 300, field: k % 7, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let source = source_of(pos + 2);
        let small = [(3u32, 0u64, 5u64, 1usize)];
        // A tape whose blocks repeat: 16z's repeats.
        let n = 5000usize;
        let mut rep: Vec<(u32, u64, u64, usize)> = (1..n as u64).map(|i| (1 + (i % 3) as u32, 4 * i, 4 * i + 2, 1)).collect();
        rep.insert(0, (0, 0, 4 * n as u64, n));
        let rep_source = source_of(4 * n as u64);
        // Random trees: every alignment of the tail, escaped gaps and lengths among plain ones.
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut rng = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        fn tree(rng: &mut impl FnMut() -> u64, depth: u32, at: &mut u64, out: &mut Vec<(u32, u64, u64, usize)>, max: usize) {
            let i = out.len();
            out.push((0, 0, 0, 0));
            let (code, start) = ((rng() % 600) as u32, *at);
            let children = if depth < 7 && out.len() < max { rng() % 6 } else { 0 };
            for _ in 0..children {
                *at += if rng().is_multiple_of(23) { 70_000 + rng() % 1000 } else { rng() % 9 };
                tree(rng, depth + 1, at, out, max);
            }
            *at += if rng().is_multiple_of(31) { 70_000 } else { rng() % 5 };
            out[i] = (code, start, *at, out.len() - i);
        }
        let mut randoms = Vec::new();
        for max in [1, 2, 3, 5, 9, 40, 300, 1023, 1024, 1025, 3000] {
            let (mut at, mut t) = (0u64, Vec::new());
            tree(&mut rng, 0, &mut at, &mut t, max);
            randoms.push((t, source_of(at + 10)));
        }
        let mut kinds = [0usize; 3];
        type Case<'a> = (&'a str, &'a [(u32, u64, u64, usize)], &'a Vec<u8>);
        let mut cases: Vec<Case> = vec![("escaped", &nodes[..], &source), ("repeating", &rep[..], &rep_source), ("one node", &small[..], &source), ("none", &[][..], &source)];
        cases.extend(randoms.iter().map(|(t, src)| ("random", &t[..], src)));
        for (label, nodes, source) in cases {
            for (z, every, late, blocks) in [(false, 1, false, true), (false, 1_000_000, false, false), (false, 97, false, true), (false, 1, true, false), (false, 13, true, true), (true, 1, false, true), (true, 1_000_000, false, false), (true, 13, true, true)] {
                if late && nodes.len() < 2 {
                    continue;
                }
                let mut r = reference::Writer::new(&lang, source, 3, z, blocks, nodes.len());
                by_ops_of(&mut r, nodes, every, late);
                let mut c = make(&lang, source, z, blocks, nodes.len());
                by_ops_of(&mut c, nodes, every, late);
                let what = format!("{label}: 16z {z}, commit every {every}, late {late}, blocks {blocks}");
                match (r.finish(), c.finish()) {
                    (Ok(r), Ok(c)) => {
                        match (&r.tape, &c.tape) {
                            (Written::F16(rt), Written::F16(ct)) => assert_eq!(ct.as_bytes(), rt.as_bytes(), "{what}: the tape"),
                            (Written::F16Z(rt), Written::F16Z(ct)) => {
                                let (rb, cb) = (rt.as_bytes(), ct.as_bytes());
                                assert_eq!(cb.len(), rb.len(), "{what}: the 16z tape's length");
                                if let Some(k) = (0..rb.len()).find(|&k| rb[k] != cb[k]) {
                                    panic!("{what}: the 16z tapes differ from byte {k} of {}", rb.len());
                                }
                                for &k in &rb[rt.zat[ZKINDS]..rt.zat[ZEND]] {
                                    kinds[usize::from(k)] += 1;
                                }
                            }
                            _ => panic!("{what}: the formats"),
                        }
                        assert_eq!(c.tape.presence_bytes(), r.tape.presence_bytes(), "{what}: the presence");
                    }
                    (Err(_), Err(_)) => assert!(nodes.is_empty(), "{what}: both failed"),
                    (r, c) => panic!("{what}: the reference {:?}, the ops {:?}", r.map(|_| ()), c.map(|_| ())),
                }
            }
        }
        assert!(kinds.iter().all(|&k| k > 0), "16z's pieces of every kind (raw, LZ4, repeat): {kinds:?}");
    }

    /// Decision 0434 §6, R14: an op that breaks the tape's bounds fails in the ops as in the reference, and the
    /// writer makes no tape: `broken_nodes_make_no_tape`'s cases, and an edit of a final block.
    #[test]
    fn the_clif_ops_fail_as_the_reference() {
        use rerail_clif::writer::fail;
        let entries: Vec<Entry> = (0..4u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let source = source_of(5000);
        // Each case calls its ops through `op(which, a, b)`, with its first failure.
        type Case = fn(&mut dyn FnMut(u8, u64, u64));
        let cases: [(Case, u32); 6] = [
            (|w| { w(0, 10, 0); w(1, 5, 6); w(2, 20, 0); }, fail::BOUNDS),
            (|w| { w(0, 0, 0); w(2, 5001, 0); }, fail::BOUNDS),
            (|w| { w(1, 0, 10); w(1, 10, 20); }, fail::OUTSIDE),
            (|w| { w(0, 0, 0); w(1, 0, 1); }, fail::UNCLOSED),
            (|w| { w(2, 0, 0); }, fail::NO_OPEN),
            (|w| {
                // Block 1 holds no open node: final once the frontier passes it.
                w(0, 0, 0);
                for k in 0..2100 { w(1, k, k + 1); }
                w(3, 0, 0);
                w(4, 1500, 1);
            }, fail::FINAL),
        ];
        for (k, (case, code)) in cases.into_iter().enumerate() {
            let mut r = reference::Writer::new(&lang, &source, 0, false, false, 4);
            let mut c = Writer::new(&lang, &source, 0, false, false, 4);
            // 0 open, 1 leaf, 2 close, 3 commit, 4 retag.
            case(&mut |op, a, b| match op {
                0 => { r.open(1, a); c.open(1, a); }
                1 => { r.leaf(2, a, b); c.leaf(2, a, b); }
                2 => { r.close(a); c.close(a); }
                3 => { r.commit(); c.commit(); }
                _ => { r.retag(a as usize, b as u32); c.retag(a as usize, b as u32); }
            });
            let (rf, cf) = (r.finish(), c.finish());
            assert!(rf.is_err(), "case {k}: the reference fails");
            let Err(e) = cf else { panic!("case {k}: the ops fail") };
            assert!(e.starts_with(&format!("the writer's ops failed: {code:#x} ")), "case {k}: the ops' first failure is {code:#x}: {e}");
        }
    }

    /// Decision 0434 §6, R14: a part with no room fails its op as full, naming the part, and the op writes nothing
    /// past the part's capacity and its 8 bytes of room: the tape's blocks, the open nodes, a block's entries, the
    /// laid side lists, the side areas given back.
    #[test]
    fn a_full_writer_writes_nothing_past_its_parts() {
        use rerail_clif::writer::{fail, part};
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k % 300, field: k % 7, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let source = source_of(pos + 2);
        let mut cases = 0;
        for (k, cap) in [(part::OUT, 5000), (part::OUT, 30_000), (part::OPEN, 24), (part::TABLE, 16), (part::LAID, 100), (part::LAID_AT, 0), (part::FREE, 0)] {
            let mut c = Writer::new(&lang, &source, 0, false, false, 0);
            c.fix(k, cap, 0xa5);
            by_ops_of(&mut c, &nodes, 1, false);
            let failure = c.failure();
            assert_eq!(failure.map(|f| f.0), Some(fail::FULL + k), "part {k} at {cap} bytes: full");
            assert!(c.buffer(k)[cap + 8..].iter().all(|&b| b == 0xa5), "part {k} at {cap} bytes: nothing written past its room");
            assert!(c.finish().is_err(), "part {k} at {cap} bytes: no tape");
            cases += 1;
        }
        assert_eq!(cases, 7);
    }

    /// Decision 0434 §6: the writer's LZ4 compressor in CLIF writes liblz4's `LZ4_compress_HC(.., 4)` bytes, for
    /// pieces of every kind (empty, short, random, runs, a tape's columns, longer than a chain's 64 KB reach), each
    /// through one compressor whose tables earlier pieces left, as the writer reuses them.
    #[test]
    fn the_clif_lz4_is_liblz4_hc4() {
        let entries: Vec<Entry> = (0..600u16).map(|k| Entry { kind: k % 300, field: k % 7, flags: 0, alias: None, classes: Vec::new() }).collect();
        let table = CodeTable::new(entries);
        let lang = lang_of(&table);
        let (nodes, pos) = escaped_nodes();
        let source = source_of(pos + 2);
        let number: Vec<u32> = (0..600).collect();
        let Ok(Written::F16(tape)) = write_from(&lang, &Plain(&nodes, &table), &number, &source, 0, false) else { panic!("format 16") };
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut random = |n: usize, alphabet: u64| -> Vec<u8> {
            (0..n)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    (x % alphabet) as u8
                })
                .collect()
        };
        let mut inputs: Vec<Vec<u8>> = vec![Vec::new(), vec![7], b"abcabcabcabc".to_vec(), b"abcabcabcabcabcabc".to_vec(), vec![0; 13], vec![0; 5000]];
        for n in [14, 31, 100, 1000, 4096, 20_000] {
            inputs.push(random(n, 256));
            inputs.push(random(n, 4));
        }
        let mut runs = Vec::new();
        for k in 0..3000u32 {
            runs.extend(std::iter::repeat_n((k % 17) as u8, (k % 23) as usize));
            runs.extend_from_slice(&k.to_le_bytes()[..(k % 3) as usize]);
        }
        inputs.push(runs);
        let t = tape.as_bytes();
        for k in 0..12 {
            inputs.push(t[k * t.len() / 12..(k + 1) * t.len() / 12].to_vec());
        }
        inputs.push(t.to_vec());
        let mut far = random(70_000, 256);
        far.extend_from_within(..30_000);
        inputs.push(far);
        let mut z = ops::Lz::new();
        let (mut shrunk, mut differs) = (0, 0);
        for round in 0..2 {
            for (k, src) in inputs.iter().enumerate() {
                let want = if src.is_empty() { vec![0] } else { lz4::compress(src) };
                let got = z.compress(src);
                assert_eq!(got, want, "input {k} ({} bytes), round {round}", src.len());
                shrunk += usize::from(got.len() < src.len());
                // A level the port is not: HC 9's bytes differ on some inputs, so the test tells levels apart.
                if src.len() > 1000 {
                    let hc9 = lz4::compress_at(src, 9);
                    differs += usize::from(hc9 != got);
                }
            }
        }
        assert!(shrunk >= 2 * 20 && differs > 0, "{shrunk} inputs compressed, {differs} unlike HC 9");
    }

    /// The decoder (`reads::lz4`, written directly) decodes what LZ4 compressed: literal runs of every length mod 8,
    /// matches at every offset under 8 and far ones, and lengths past 15 (their extension bytes); a piece cut short
    /// decodes to less than its length. And the JIT's (`tape_lz4`, the reads' in CLIF, which widens an offset under 8
    /// as LZ4's reference decoder does) decodes each the same: as many bytes, the same bytes (decision 0517).
    #[test]
    fn the_reads_decoder_decodes_what_lz4_compressed() {
        let jit = rerail_clif::jit::Reads::new();
        type Lz4 = extern "C" fn(*const u8, *const TapeView, u32, u32, u64, u64) -> u64;
        // SAFETY: the read's signature: the tape's base, its view, the piece's bounds, the destination and its length.
        let tape_lz4: Lz4 = unsafe { std::mem::transmute(jit.function("tape_lz4")) };
        let jits = |buf: &[u8], from: usize, to: usize, len: usize| {
            let mut dst = vec![0u8; len + 8];
            let n = tape_lz4(buf.as_ptr(), &TapeView::default(), from as u32, to as u32, dst.as_mut_ptr() as u64, len as u64) as usize;
            dst.truncate(n);
            dst
        };
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for case in 0..400 {
            let mut raw = Vec::new();
            while raw.len() < 64 + case * 37 {
                match next() % 4 {
                    // A run repeating its last `off` bytes (a match at an offset under 8, or far).
                    0 | 1 if raw.len() > 16 => {
                        let off = if next() % 2 == 0 { 1 + (next() % 7) as usize } else { 8 + (next() % 9) as usize };
                        let off = off.min(raw.len());
                        for _ in 0..(4 + next() % 300) {
                            raw.push(raw[raw.len() - off]);
                        }
                    }
                    // Literals.
                    _ => raw.extend((0..1 + next() % 40).map(|_| next() as u8)),
                }
            }
            let packed = lz4::compress(&raw);
            let mut buf = vec![0xaau8; 5];
            buf.extend_from_slice(&packed);
            let mut dst = vec![0u8; raw.len() + 8];
            decode_block(&buf, 5, buf.len(), &mut dst, raw.len()).unwrap();
            assert_eq!(&dst[..raw.len()], &raw[..], "case {case}");
            assert_eq!(jits(&buf, 5, buf.len(), raw.len()), raw, "case {case}: the JIT's decode");
            // Cut short: fewer bytes, never past them, and refused.
            let cut = 5 + packed.len() / 2;
            let mut dst = vec![0u8; raw.len() + 8];
            assert!(decode_block(&buf[..cut], 5, cut, &mut dst, raw.len()).is_err(), "case {case}: a piece cut short is refused");
            let n = crate::reads::lz4(&buf[5..cut], &mut dst, raw.len());
            assert!(n < raw.len(), "case {case}: a piece cut short decodes less");
            assert_eq!(jits(&buf[..cut], 5, cut, raw.len()), &dst[..n], "case {case}: the JIT's decode of the piece cut short");
        }
    }
}
