//! Format 16 (decision 0156 §1), written and read in place (decision 0293). Ported from the lab
//! (`native/lab/tape16`: `f16`, `f16w`, `code8`, `repeat`), which measured it.
//!
//! A tape is five arrays over a document's nodes in preorder, at widths fixed for the language:
//!
//! - **codes**: a byte a node, the code's number in the language's table, numbered most frequent first. Below the
//!   cutoff the byte is the number; past it, a prefix byte and a low part in the block's side list;
//! - **slots**: an inner node's `size << 1`; a leaf's `distance << 2 | 1` to the next node's previous sibling, or,
//!   on every `K`-th sibling, `distance << 2 | 3` to the next node's parent;
//! - **gaps**: each start less the previous node's; **lengths**: each end less its start;
//! - **checkpoints**: the absolute start of every `K`-th node, at the document's width.
//!
//! A value too wide for its array is the all-ones value, and its value is in the side list of its 256-node block.
//! Each array's side list is kept in parts of `PART` nodes, one a fixed block at rest (0156 §1.5, §2.1), with
//! each part's start (the block index's entry).

use crate::generated_reads::f16 as reads;
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
        Ok(Lang { table, index, cut, per, low_w: if per <= 256 { 1 } else { 2 }, widths, revision })
    }
    pub fn table(&self) -> &CodeTable {
        &self.table
    }
    /// `entry`'s number, if the table has it.
    pub fn number(&self, entry: &Entry) -> Option<u32> {
        self.index.get(entry).copied()
    }
    /// What a reader elsewhere (the device's, `metal_tape.metal`) needs of the language to read its tapes in
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

/// One array's side list as the writer lays it: its parts end to end (8 bytes of room past them), each part's
/// start (one more at the end), and a value's bytes.
struct Side {
    bytes: Vec<u8>,
    base: Vec<u32>,
    vw: usize,
}

/// A stored format 16 tape's magic (its first 6 bytes are every stored tape's: the store reads the language tag
/// after them, `LANGUAGE`).
pub const MAGIC16: &[u8; 8] = b"TMPTAP16";
/// A stored 16z tape's magic.
pub const MAGIC16Z: &[u8; 8] = b"TMPTAP6Z";

/// Where a stored format 16 or 16z tape's header holds each value. A 16z tape's header is the header of the format
/// 16 tape it decodes to, but for its magic.
mod head {
    /// The language's tape tag, a byte; then the revision mark and the checkpoints' width.
    pub const LANGUAGE: usize = 8;
    pub const MARK: usize = 9;
    pub const CW: usize = 10;
    /// The source's newlines, u32: its newline index's count (`newlines`).
    pub const NEWLINES: usize = 12;
    /// The source's length, u64; the node count, u64.
    pub const SOURCE_LEN: usize = 16;
    pub const NODES: usize = 24;
    /// The side lists' value widths, a byte each; then the slot, gap and length widths.
    pub const VW: usize = 32;
    pub const WIDTHS: usize = 36;
    /// Each region's start in the format 16 tape, a u32, and one more at the end (decision 0336): a region is from
    /// its start to the next, its last 8 bytes room for a word's load.
    pub const STARTS: usize = 40;
}
/// A format 16 header's length; each side list's part starts follow it (`Placement::parts`).
const HEAD16: usize = head::STARTS + 4 * (REGIONS + 1);

fn u64_at(b: &[u8], at: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(b.get(at..at + 8).ok_or("a short tape header")?.try_into().expect("8 bytes")))
}

fn u32s(b: &[u8], at: usize, count: usize) -> Result<Vec<u32>, String> {
    let bytes = at.checked_add(4 * count).and_then(|end| b.get(at..end)).ok_or("a short tape header")?;
    Ok(bytes.as_chunks::<4>().0.iter().map(|&w| u32::from_le_bytes(w)).collect())
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

/// Where the regions of `n` nodes start when the first is at `first`, and the end: from the widths, the side
/// lists' lengths and the newline index's. Each region is followed by 8 bytes of room. A tape's first region is past its header and part
/// starts (`first_region`); a decoded 16z block's is at 0.
fn layout(first: usize, n: usize, w: [usize; 3], cw: usize, sides: [usize; 4], lines: usize) -> [usize; REGIONS + 1] {
    let len = [n, n * w[0], n * w[1], n * w[2], n.div_ceil(K) * cw, sides[0], sides[1], sides[2], sides[3], lines];
    let mut at = [first; REGIONS + 1];
    for r in 0..REGIONS {
        at[r + 1] = at[r] + len[r] + 8;
    }
    at
}

/// Where a tape of `n` nodes' first region starts: past the header and the side lists' part starts.
fn first_region(n: usize) -> Option<usize> {
    Some(HEAD16.checked_add(n.div_ceil(PART).checked_add(1)?.checked_mul(16)?)?.next_multiple_of(8))
}

/// One document's format 16 tape, held as stored (`as_bytes`) and read where it lies: the header (each region's
/// start among its values), each side list's part starts, then the regions: codes, slots, gaps, lengths,
/// checkpoints, and the four side lists (codes' low parts, then escaped slots, gaps, lengths), each followed by 8
/// bytes of room for a word's load. It is read through the tape's reads (decision 0310) over its view, which it
/// keeps but for its language's choices (`reader`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tape16 {
    pub mark: u8,
    p: Placement,
    source_len: u64,
    buf: Vec<u8>,
    /// Its view without its language's choices (`Placement::frame`), or none for a source past 4 GiB, which the
    /// reads' u32 offsets do not reach.
    frame: Option<TapeView>,
}

impl Tape16 {
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
    /// The source's length.
    pub fn source_len(&self) -> u64 {
        self.source_len
    }
    /// The language tag its header names (`set_tag`).
    pub fn tag(&self) -> u8 {
        self.buf[head::LANGUAGE]
    }
    /// Names the language's tape tag in its header.
    pub fn set_tag(&mut self, tag: u8) {
        self.buf[head::LANGUAGE] = tag;
    }
    /// The tape as stored.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
    /// Region `r` with its 8 bytes of room.
    fn region(&self, r: usize) -> &[u8] {
        &self.buf[self.p.at[r]..self.p.at[r + 1]]
    }

    /// The stored tape, held as it is: its header and part starts are checked against its length.
    pub fn from_bytes(buf: Vec<u8>) -> Result<Self, String> {
        let p = placement(&buf)?;
        let source_len = u64_at(&buf, head::SOURCE_LEN)?;
        Ok(Tape16 { mark: buf[head::MARK], p, source_len, buf, frame: p.frame_of(source_len) })
    }

    /// Part `j`'s start in side list `k`'s region (one more at the end).
    fn part_start(&self, k: usize, j: usize) -> usize {
        self.p.part_start(&self.buf, k, j)
    }

    /// The tape from its regions (each with its 8 bytes of room) and side lists, laid as stored. `Err` for a tape
    /// past 4 GiB, which its header's u32 starts do not reach.
    #[allow(clippy::too_many_arguments)]
    fn laid(tag: u8, mark: u8, n: usize, cw: usize, source_len: u64, widths: [usize; 3], arrays: [Vec<u8>; 5], side: [Side; 4], lines: &[u8]) -> Result<Self, String> {
        let vw = std::array::from_fn(|k| side[k].vw);
        let newlines = crate::newlines::count(lines).ok_or("a newline index without its count")?;
        let p = Placement::of(n, cw, widths, vw, std::array::from_fn(|k| side[k].bytes.len() - 8), lines.len(), newlines)?;
        let mut buf = head_bytes(MAGIC16, tag, mark, &p, source_len, std::array::from_fn(|k| side[k].base.as_slice()))?;
        for region in arrays.iter().map(Vec::as_slice).chain(side.iter().map(|s| s.bytes.as_slice())) {
            buf.extend_from_slice(region);
        }
        buf.extend_from_slice(lines);
        buf.extend_from_slice(&[0; 8]);
        debug_assert_eq!(buf.len(), p.at[REGIONS]);
        Ok(Tape16 { mark, p, source_len, buf, frame: p.frame_of(source_len) })
    }

    /// Read with `lang`'s choices: the tape's mark is `lang`'s revision and its widths are `lang`'s, else it is
    /// rewritten before it is read.
    pub fn reader<'a>(&'a self, lang: &'a Lang) -> Result<F16<'a>, String> {
        if self.mark != lang.revision || self.p.widths != lang.widths {
            return Err(format!("a tape of revision {} (widths {:?}), read with revision {} ({:?})", self.mark, self.p.widths, lang.revision, lang.widths));
        }
        let frame = self.frame.ok_or("a format 16 tape whose source is past 4 GiB: the reads' offsets do not reach it")?;
        Ok(F16 { bytes: &self.buf, view: bind(frame, lang)?, lang })
    }
}

/// A format 16 header and each side list's part starts (`bases`, one more at the end), up to the first region: what
/// a format 16 tape and its 16z form both begin with, `magic` apart. `Err` for a tape past 4 GiB.
fn head_bytes(magic: &[u8; 8], tag: u8, mark: u8, p: &Placement, source_len: u64, bases: [&[u32]; 4]) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; HEAD16];
    buf[..8].copy_from_slice(magic);
    buf[head::LANGUAGE] = tag;
    buf[head::MARK] = mark;
    buf[head::CW] = p.cw as u8;
    buf[head::NEWLINES..head::NEWLINES + 4].copy_from_slice(&p.newlines.to_le_bytes());
    buf[head::SOURCE_LEN..head::SOURCE_LEN + 8].copy_from_slice(&source_len.to_le_bytes());
    buf[head::NODES..head::NODES + 8].copy_from_slice(&(p.n as u64).to_le_bytes());
    for k in 0..4 {
        buf[head::VW + k] = p.vw[k] as u8;
    }
    for k in 0..3 {
        buf[head::WIDTHS + k] = p.widths[k] as u8;
    }
    for (r, &at) in p.at.iter().enumerate() {
        let at = u32::try_from(at).map_err(|_| "a format 16 tape past 4 GiB: its header's starts are u32s".to_owned())?;
        buf[head::STARTS + 4 * r..head::STARTS + 4 * r + 4].copy_from_slice(&at.to_le_bytes());
    }
    let parts = p.n.div_ceil(PART) + 1;
    for base in bases {
        debug_assert_eq!(base.len(), parts);
        for b in base {
            buf.extend_from_slice(&b.to_le_bytes());
        }
    }
    buf.resize(p.at[0], 0);
    Ok(buf)
}

/// Where a stored format 16 tape's parts lie in its bytes, as its header says, for a reader that reads them there
/// without this crate (the device's, `metal_tape.metal`): each region's start (codes, slots, gaps, lengths,
/// checkpoints, the four side lists; one more at the end), each side list's part starts (u32s, a part's start and
/// one more at the end), the node count, the checkpoints' width, the side lists' value widths and the slot, gap
/// and length widths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub n: usize,
    pub cw: usize,
    pub at: [usize; REGIONS + 1],
    pub parts: [usize; 4],
    pub vw: [usize; 4],
    pub widths: [usize; 3],
    /// The source's newlines (it has one line more).
    pub newlines: u32,
}

impl Placement {
    /// The placement of a tape of `n` nodes with these widths, side lists `sides` bytes long and a newline index
    /// `lines` bytes long of `newlines`.
    fn of(n: usize, cw: usize, widths: [usize; 3], vw: [usize; 4], sides: [usize; 4], lines: usize, newlines: u32) -> Result<Self, String> {
        let first = first_region(n).ok_or("a tape length overflows")?;
        let parts = n.div_ceil(PART) + 1;
        Ok(Placement { n, cw, at: layout(first, n, widths, cw, sides, lines), parts: std::array::from_fn(|k| HEAD16 + 4 * k * parts), vw, widths, newlines })
    }

    /// Part `j`'s start in side list `k`'s region (one more at the end), in `buf`, the tape's bytes or its 16z
    /// form's (the same up to the first region).
    fn part_start(&self, buf: &[u8], k: usize, j: usize) -> usize {
        u32_at(buf, self.parts[k] + 4 * j) as usize
    }

    /// The view a tape's reads take (decision 0310), the tape at byte `base` of what the view's offsets count from
    /// (0: the tape's own bytes).
    pub fn view(&self, lang: &Lang, base: usize) -> Result<TapeView, String> {
        bind(self.frame(base)?, lang)
    }

    /// The view but for its language's choices (`bind` adds them): the tape's own widths, and where its parts lie.
    pub fn frame(&self, base: usize) -> Result<TapeView, String> {
        let at = |offset: usize| u32::try_from(base + offset).map_err(|_| "a tape placed past 4 GiB".to_owned());
        let mut widths = 0;
        for (k, w) in [self.cw, self.vw[0], self.vw[1], self.vw[2], self.vw[3]].into_iter().enumerate() {
            widths |= nibble(w)? << (12 + 4 * k);
        }
        Ok(TapeView {
            nodes: u32::try_from(self.n).map_err(|_| "a tape past 4 G nodes".to_owned())?,
            format: 16,
            widths,
            columns: [at(self.at[0])?, at(self.at[1])?, at(self.at[2])?, at(self.at[3])?, at(self.at[4])?],
            side: [at(self.at[5])?, at(self.at[6])?, at(self.at[7])?, at(self.at[8])?],
            parts: [at(self.parts[0])?, at(self.parts[1])?, at(self.parts[2])?, at(self.parts[3])?],
            lines: at(self.at[9])?,
            line_count: self.newlines.checked_add(1).ok_or("a source past 4 G lines")?,
            ..Default::default()
        })
    }

    /// The frame of a tape of a source `source_len` long, at its own bytes, if the reads reach both.
    fn frame_of(&self, source_len: u64) -> Option<TapeView> {
        self.frame(0).ok().filter(|_| u32::try_from(source_len).is_ok())
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

/// The placement a format 16 header says (a 16z tape's: of the tape it decodes to), checked against itself: the
/// widths in range, the first region past the part starts, each fixed region as long as its nodes take, each region
/// with its room. The bytes past the header are not read.
fn header(buf: &[u8]) -> Result<Placement, String> {
    let n = usize::try_from(u64_at(buf, head::NODES)?).map_err(|_| "a tape length overflows")?;
    let bad = || "a format 16 header that does not fit itself".to_owned();
    let byte = |at: usize| usize::from(buf[at]);
    let cw = byte(head::CW);
    let vw: [usize; 4] = std::array::from_fn(|k| byte(head::VW + k));
    let widths: [usize; 3] = std::array::from_fn(|k| byte(head::WIDTHS + k));
    if cw > 8 || vw.iter().any(|&w| w > 8) || widths.iter().any(|&w| !(1..=8).contains(&w)) {
        return Err("a format 16 tape's widths are out of range".into());
    }
    let starts = u32s(buf, head::STARTS, REGIONS + 1)?;
    let at: [usize; REGIONS + 1] = std::array::from_fn(|r| starts[r] as usize);
    let fixed = [Some(n), n.checked_mul(widths[0]), n.checked_mul(widths[1]), n.checked_mul(widths[2]), n.div_ceil(K).checked_mul(cw)];
    if Some(at[0]) != first_region(n) || (0..REGIONS).any(|r| at[r + 1] < at[r].saturating_add(8)) || (0..5).any(|r| fixed[r].and_then(|l| l.checked_add(8)) != Some(at[r + 1] - at[r])) {
        return Err(bad());
    }
    let parts = n.div_ceil(PART) + 1;
    let newlines = u32s(buf, head::NEWLINES, 1)?[0];
    Ok(Placement { n, cw, at, parts: std::array::from_fn(|k| HEAD16 + 4 * k * parts), vw, widths, newlines })
}

/// Each side list's part starts in `buf` (from 0, in order, the last its length) fit the region `p` gives it.
fn check_parts(buf: &[u8], p: &Placement) -> Result<(), String> {
    let parts = p.n.div_ceil(PART) + 1;
    if buf.len() < p.at[0] {
        return Err("a short tape header".into());
    }
    for k in 0..4 {
        let b = |j: usize| p.part_start(buf, k, j);
        if b(0) != 0 || (1..parts).any(|j| b(j - 1) > b(j)) || b(parts - 1) + 8 != p.at[6 + k] - p.at[5 + k] {
            return Err(format!("side list {k}'s part starts do not fit it"));
        }
    }
    Ok(())
}

/// The placement of the stored format 16 tape `buf`, its header and part starts checked against its length as
/// `Tape16::from_bytes` checks them, without copying a byte.
pub fn placement(buf: &[u8]) -> Result<Placement, String> {
    if buf.len() < HEAD16 || &buf[..8] != MAGIC16 {
        return Err("not a format 16 tape".into());
    }
    let p = header(buf)?;
    if buf.len() != p.at[REGIONS] {
        return Err(format!("a format 16 tape of {} bytes, its header says {}", buf.len(), p.at[REGIONS]));
    }
    check_parts(buf, &p)?;
    check_lines(&buf[p.at[9]..p.at[10] - 8], &p, u64_at(buf, head::SOURCE_LEN)?)?;
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

/// A node's cells as `write` takes them: its code's number in the language's table, its span, and one past its
/// subtree's last node.
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub number: u32,
    pub start: u64,
    pub end: u64,
    pub subtree_end: usize,
}

/// A tape `write` made: format 16, or 16z.
pub enum Written {
    F16(Tape16),
    F16Z(Tape16Z),
}

/// One block's cells as the pass writes them (`Z` nodes; the last block fewer): the five arrays, and each side
/// list's entries as (node in the block, value), in node order.
struct Cells {
    codes: Vec<u8>,
    arrays: [Vec<u8>; 3],
    ckpt: Vec<u8>,
    side: [Vec<(u32, u64)>; 4],
}

/// Where `write` puts each finished block: format 16's regions and side lists, or 16z's pieces, with the checkpoints
/// and each side list's part starts (`base`, from the parts' lengths so far, `sides`).
enum Out {
    Plain { regions: [Vec<u8>; 5], side: [(Vec<u8>, Vec<u32>); 4] },
    Blocks { pieces: Pieces, ckpt: Vec<u8>, base: [Vec<u32>; 4], sides: [usize; 4], piece: Vec<u8> },
}

/// A block's side-list part as stored (`parts`, one part): for each of its `BLOCK`-node blocks a u16 end, then each
/// entry's node offset in its block, then the values at `vw` bytes. Nothing for a part with no entries.
fn part_into(out: &mut Vec<u8>, e: &[(u32, u64)], nb: usize, vw: usize) {
    if e.is_empty() {
        return;
    }
    let nsub = nb.div_ceil(BLOCK);
    let mut k = 0;
    for b in 0..nsub {
        while k < e.len() && (e[k].0 as usize) / BLOCK == b {
            k += 1;
        }
        out.extend_from_slice(&(k as u16).to_le_bytes());
    }
    out.extend(e.iter().map(|x| (x.0 as usize % BLOCK) as u8));
    for x in e {
        out.extend_from_slice(&x.1.to_le_bytes()[..vw]);
    }
}

/// Format 16, or 16z where `z`, written in one pass over a document's `n` nodes in preorder (0156 §7; decision
/// 0293): `node(i)` is node `i`'s cells, asked once each, in order. Each cell is written once: a node's code, gap,
/// length, checkpoint and (an inner node's) slot as it is reached, a leaf's slot as the node after it is. A block's
/// cells are final once the pass is past it: it is laid then (16z: compressed). No array of the nodes is made.
///
/// The side lists' value widths come from the document's bounds (a slot is at most `4n + 3`, a gap or a length the
/// source's length), so they are known before the first block. `Err` where a node's values break them (a start
/// before the one before it, a span past the source): no tape is made of it.
pub fn write(lang: &Lang, n: usize, source: &[u8], tag: u8, z: bool, mut node: impl FnMut(usize) -> Node) -> Result<Written, String> {
    let source_len = source.len() as u64;
    let lines = crate::newlines::index(source);
    let w = lang.widths;
    let esc = [mask(w[0]), mask(w[1]), mask(w[2])];
    let cw = bytes_for(source_len);
    let vw = [lang.low_w, bytes_for(4 * n as u64 + 3), bytes_for(source_len), bytes_for(source_len)];
    let new_cells = || Cells {
        codes: Vec::with_capacity(Z),
        arrays: [Vec::with_capacity(Z * w[0]), Vec::with_capacity(Z * w[1]), Vec::with_capacity(Z * w[2])],
        ckpt: Vec::with_capacity(Z / K * cw),
        side: Default::default(),
    };
    let mut out = if z {
        Out::Blocks { pieces: Pieces::new(), ckpt: Vec::with_capacity(n.div_ceil(K) * cw), base: Default::default(), sides: [0; 4], piece: Vec::new() }
    } else {
        Out::Plain {
            regions: [Vec::with_capacity(n + 8), Vec::with_capacity(n * w[0] + 8), Vec::with_capacity(n * w[1] + 8), Vec::with_capacity(n * w[2] + 8), Vec::with_capacity(n.div_ceil(K) * cw + 8)],
            side: Default::default(),
        }
    };
    // Value `v` of array `a` (0 slots, 1 gaps, 2 lengths) into the block's cell `j`, or its escape.
    let put = |c: &mut Cells, a: usize, j: usize, v: u64| {
        let v = if v >= esc[a] {
            c.side[a + 1].push((j as u32, v));
            esc[a]
        } else {
            v
        };
        c.arrays[a][j * w[a]..(j + 1) * w[a]].copy_from_slice(&v.to_le_bytes()[..w[a]]);
    };
    // A block's cells, laid where they go.
    let lay = |out: &mut Out, c: &mut Cells, nb: usize| {
        match out {
            Out::Plain { regions, side } => {
                regions[0].extend_from_slice(&c.codes);
                for a in 0..3 {
                    regions[1 + a].extend_from_slice(&c.arrays[a]);
                }
                regions[4].extend_from_slice(&c.ckpt);
                for k in 0..4 {
                    let (bytes, base) = &mut side[k];
                    base.push(bytes.len() as u32);
                    part_into(bytes, &c.side[k], nb, vw[k]);
                }
            }
            Out::Blocks { pieces, ckpt, base, sides, piece } => {
                ckpt.extend_from_slice(&c.ckpt);
                // Each array's piece: its bytes for the block, then its side list's part.
                for q in 0..PIECES {
                    piece.clear();
                    piece.extend_from_slice(if q == 0 { &c.codes } else { &c.arrays[q - 1] });
                    let fixed = piece.len();
                    part_into(piece, &c.side[q], nb, vw[q]);
                    base[q].push(sides[q] as u32);
                    sides[q] += piece.len() - fixed;
                    pieces.push(piece);
                }
            }
        }
        c.codes.clear();
        for a in &mut c.arrays {
            a.clear();
        }
        c.ckpt.clear();
        for s in &mut c.side {
            s.clear();
        }
    };
    let mut c = new_cells();
    // The open nodes: each its index, its children so far, its last child, its subtree's end.
    let mut open: Vec<(u32, u32, u32, usize)> = Vec::with_capacity(64);
    let mut prev_start = 0u64;
    for i in 0..n {
        let x = node(i);
        if x.start < prev_start || x.end < x.start || x.end > source_len || x.subtree_end <= i || x.subtree_end > n {
            return Err(format!("node {i}'s cells break the tape's bounds"));
        }
        while open.last().is_some_and(|o| o.3 <= i) {
            open.pop();
        }
        // The node before is a leaf unless it is the parent: its slot describes this node.
        if let Some(top) = open.last_mut() {
            let (idx, prev, p) = (top.1, top.2, top.0);
            top.1 = idx + 1;
            top.2 = i as u32;
            let before = i - 1;
            if before as u32 != p {
                let v = if (idx as usize).is_multiple_of(K) { (u64::from(i as u32 - p) << 2) | 3 } else { (u64::from(i as u32 - prev) << 2) | 1 };
                put(&mut c, 0, before % Z, v);
            }
        } else if i > 0 {
            return Err(format!("node {i} lies outside the root's subtree"));
        }
        // The block before is final once its last node's slot is: laid, and this node starts the next.
        if i > 0 && i.is_multiple_of(Z) {
            lay(&mut out, &mut c, Z);
        }
        let j = i % Z;
        let number = x.number;
        if number < lang.cut {
            c.codes.push(number as u8);
        } else {
            let t = number - lang.cut;
            c.codes.push((lang.cut + t / lang.per) as u8);
            c.side[0].push((j as u32, u64::from(t % lang.per)));
        }
        for (a, array) in c.arrays.iter_mut().enumerate() {
            array.resize((j + 1) * w[a], 0);
        }
        put(&mut c, 1, j, if i == 0 { 0 } else { x.start - prev_start });
        put(&mut c, 2, j, x.end - x.start);
        prev_start = x.start;
        if j.is_multiple_of(K) {
            c.ckpt.extend_from_slice(&x.start.to_le_bytes()[..cw]);
        }
        let size = x.subtree_end - i;
        if size > 1 {
            put(&mut c, 0, j, (size as u64) << 1);
        }
        open.push((i as u32, 0, NONE, x.subtree_end));
    }
    if n > 0 {
        // The last node in preorder is a leaf with nothing after it.
        put(&mut c, 0, (n - 1) % Z, 1);
        lay(&mut out, &mut c, n - (n - 1) / Z * Z);
    }
    let mark = lang.revision;
    Ok(match out {
        Out::Plain { regions, side } => {
            let arrays = regions.map(|mut r| {
                r.extend_from_slice(&[0; 8]);
                r
            });
            let mut k = 0;
            let side = side.map(|(mut bytes, mut base)| {
                base.push(bytes.len() as u32);
                bytes.extend_from_slice(&[0; 8]);
                k += 1;
                Side { bytes, base, vw: vw[k - 1] }
            });
            Written::F16(Tape16::laid(tag, mark, n, cw, source_len, w, arrays, side, &lines)?)
        }
        Out::Blocks { pieces, ckpt, mut base, sides, .. } => {
            for q in 0..4 {
                base[q].push(sides[q] as u32);
            }
            let newlines = crate::newlines::count(&lines).ok_or("a newline index without its count")?;
            let p = Placement::of(n, cw, w, vw, sides, lines.len(), newlines)?;
            let head = head_bytes(MAGIC16Z, tag, mark, &p, source_len, std::array::from_fn(|k| base[k].as_slice()))?;
            Written::F16Z(Tape16Z::laid(head, p, mark, source_len, &ckpt, &lines, pieces))
        }
    })
}

const NONE: u32 = u32::MAX;

/// Format 16 (or 16z where `z`) written from another tape `t`, each node's code mapped to the language's number
/// (`number`, by `t`'s code), in one pass (`write`): a tape a parser did not write in format 16 (decision 0293).
pub fn write_from<T: Tape>(lang: &Lang, t: &T, number: &[u32], source: &[u8], tag: u8, z: bool) -> Result<Written, String> {
    let mut s = 0;
    write(lang, t.len(), source, tag, z, |i| {
        s = t.start_next(i, s);
        Node { number: number[t.code(i) as usize], start: s, end: t.end_with_start(i, s), subtree_end: t.subtree_end(i) }
    })
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

/// A format 16 tape read in place with its language's choices, through the tape's reads (decision 0310's Rust
/// backend, `generated_reads`) over its view.
#[derive(Clone, Copy)]
pub struct F16<'a> {
    bytes: &'a [u8],
    view: TapeView,
    lang: &'a Lang,
}

impl F16<'_> {
    /// The line (from 0) byte `x` of the source lies in: the newlines before it, by the tape's newline index.
    pub fn line(&self, x: u32) -> u32 {
        reads::tape_line(self.bytes, &self.view, x)
    }
    /// Line `k`'s first byte (`k` up to the source's newlines).
    pub fn line_start(&self, k: u32) -> u32 {
        reads::tape_line_start(self.bytes, &self.view, k)
    }
    /// The source's lines: one more than its newlines.
    pub fn line_count(&self) -> u32 {
        self.view.line_count
    }
    /// Each node's code byte.
    fn codes(&self) -> &[u8] {
        let at = self.view.columns[0] as usize;
        &self.bytes[at..at + self.view.nodes as usize]
    }
}

impl Tape for F16<'_> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.view.nodes as usize
    }
    #[inline(always)]
    fn table(&self) -> &CodeTable {
        &self.lang.table
    }
    #[inline(always)]
    fn code(&self, i: usize) -> u32 {
        reads::tape_number(self.bytes, &self.view, i as u32)
    }
    #[inline(always)]
    fn subtree_end(&self, i: usize) -> usize {
        i + reads::tape_size(self.bytes, &self.view, i as u32) as usize
    }
    /// The node before is the parent (an inner node), or a leaf naming the previous sibling or, on every `K`-th
    /// sibling, the parent: at most `K - 1` sibling steps.
    fn parent(&self, i: usize) -> Option<usize> {
        let p = reads::tape_parent(self.bytes, &self.view, i as u32);
        (p != NONE).then_some(p as usize)
    }
    #[inline(always)]
    fn start(&self, i: usize) -> u64 {
        u64::from(reads::tape_start(self.bytes, &self.view, i as u32))
    }
    #[inline(always)]
    fn end(&self, i: usize) -> u64 {
        u64::from(reads::tape_end(self.bytes, &self.view, i as u32))
    }
    #[inline(always)]
    fn start_next(&self, i: usize, prev_start: u64) -> u64 {
        u64::from(reads::tape_start_next(self.bytes, &self.view, i as u32, prev_start as u32))
    }
    #[inline(always)]
    fn end_with_start(&self, i: usize, start: u64) -> u64 {
        start + u64::from(reads::tape_length(self.bytes, &self.view, i as u32))
    }
    /// The code bytes 64 at a time through a 256-entry table; a byte past the cutoff is decoded only when the set
    /// holds a number past it.
    fn match_groups(&self, set: &CodeSet, mut f: impl FnMut(usize, u64)) {
        let (byte, tail) = byte_table(self.lang, set);
        let codes = self.codes();
        let mut groups = [0u64; PART / 64];
        for (part, chunk) in codes.chunks(PART).enumerate() {
            let count = part_groups(chunk, self.lang, set, &byte, tail.then_some((self.bytes, &self.view, part)), &mut groups);
            for (g, &m) in groups[..count].iter().enumerate() {
                f(part * PART + 64 * g, m);
            }
        }
    }
}

// ---- format 16 in fixed blocks (16z) -----------------------------------------------------------------------

/// Nodes a fixed block holds: one side-list part, so every read of a node but a parent's sibling steps stays in
/// its block.
pub const Z: usize = PART;

/// The regions of a format 16 tape, in order: codes, slots, gaps, lengths, checkpoints, the four side lists, the
/// newline index.
const REGIONS: usize = 10;

/// The pieces a 16z block is stored in, one an array: codes, slots, gaps, lengths. Each is the block's bytes of
/// the array, then the array's side list's part for the block (0156 §2.1: one decode gives both).
const PIECES: usize = 4;

/// How a piece is stored (its kind byte): as its bytes, as one LZ4 block, or as a repeat of an earlier piece.
const RAW: u8 = 0;
const LZ4: u8 = 1;
const REPEAT: u8 = 2;

/// The fewest bytes a piece holds to be stored as a repeat (whose payload is the earlier piece's index, 4 bytes).
const REPEAT_MIN: usize = 8;
/// The fewest bytes a piece holds to be offered to LZ4.
const LZ4_MIN: usize = 16;

/// 16z's pieces as a writer adds them, in order: each stored as its bytes, as one LZ4 block (HC 9) when that
/// shrinks it by a tenth, or as a repeat of an earlier piece of the tape with the same bytes (0156 §2.1).
struct Pieces {
    payload: Vec<u8>,
    /// Each piece's start in the payload, one more at the end.
    offs: Vec<u32>,
    kinds: Vec<u8>,
    /// The pieces stored so far, by their bytes: what a repeat names.
    seen: HashMap<Vec<u8>, u32>,
}

impl Pieces {
    fn new() -> Self {
        Pieces { payload: Vec::new(), offs: vec![0], kinds: Vec::new(), seen: HashMap::new() }
    }

    fn push(&mut self, piece: &[u8]) {
        let index = self.kinds.len() as u32;
        let kind = match self.seen.get(piece) {
            Some(&earlier) => {
                self.payload.extend_from_slice(&earlier.to_le_bytes());
                REPEAT
            }
            None => {
                if piece.len() >= REPEAT_MIN {
                    self.seen.insert(piece.to_vec(), index);
                }
                let z = if piece.len() >= LZ4_MIN { lz4::compress(piece) } else { Vec::new() };
                if !z.is_empty() && 10 * z.len() <= 9 * piece.len() {
                    self.payload.extend_from_slice(&z);
                    LZ4
                } else {
                    self.payload.extend_from_slice(piece);
                    RAW
                }
            }
        };
        self.kinds.push(kind);
        self.offs.push(self.payload.len() as u32);
    }
}

impl Tape16 {
    /// This tape stored in fixed blocks (16z), each piece compressed alone (no dictionary yet: 0293, not done).
    pub fn blocks(&self) -> Tape16Z {
        let (n, w) = (self.p.n, self.p.widths);
        let mut pieces = Pieces::new();
        let mut piece = Vec::new();
        for b in 0..n.div_ceil(Z) {
            let (i0, nb) = (b * Z, Z.min(n - b * Z));
            for q in 0..PIECES {
                let wq = [1, w[0], w[1], w[2]][q];
                piece.clear();
                piece.extend_from_slice(&self.region(q)[i0 * wq..(i0 + nb) * wq]);
                piece.extend_from_slice(&self.region(5 + q)[self.part_start(q, b)..self.part_start(q, b + 1)]);
                pieces.push(&piece);
            }
        }
        let mut head = self.buf[..self.p.at[0]].to_vec();
        head[..8].copy_from_slice(MAGIC16Z);
        Tape16Z::laid(head, self.p, self.mark, self.source_len, &self.buf[self.p.at[4]..self.p.at[5] - 8], &self.buf[self.p.at[9]..self.p.at[10] - 8], pieces)
    }
}

/// One document's format 16 tape stored in fixed blocks of `Z` nodes (0156 §2.1), held as stored (`as_bytes`):
///
/// - the header and side lists' part starts of the format 16 tape it decodes to, its magic `MAGIC16Z`;
/// - the checkpoints, raw (absolute starts do not compress), with their room;
/// - the newline index, raw, with its room (read in place: `newlines`);
/// - the piece index: each piece's start in the payload (a u32, one more at the end), then each one's kind (a byte);
/// - the payload.
///
/// Block `b`'s piece `q` (`PIECES` a block) decodes to array `q`'s bytes for the block's nodes and side list `q`'s
/// part `b`, each where it lies in the format 16 tape: a block decodes in place, alone (`decode_block_into`). The
/// CPU reads a block decoded on its own (`F16Z`).
#[derive(Clone, Debug)]
pub struct Tape16Z {
    /// Which tape its decoded blocks are, in the thread's cache (a clone holds the same bytes).
    id: u64,
    pub mark: u8,
    /// The placement of the format 16 tape it decodes to.
    p: Placement,
    source_len: u64,
    buf: Vec<u8>,
    /// Where the piece index's starts, its kinds and the payload begin in `buf`.
    ix: usize,
    kinds: usize,
    payload: usize,
}

/// One 16z piece's decode (`Tape16Z::piece_jobs`): its stored bytes (in the tape as stored), LZ4 or raw, decoding
/// to `fixed` (its array's bytes for its block's nodes) then `side` (its side list's part), in the format 16 tape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PieceJob {
    pub src: std::ops::Range<usize>,
    pub lz4: bool,
    pub fixed: std::ops::Range<usize>,
    pub side: std::ops::Range<usize>,
}

/// Where a 16z tape's piece index, its kinds and its payload begin, and the number of pieces, from the placement of
/// the tape it decodes to: past the head (up to the first region), the checkpoints and the newline index.
fn index_at(p: &Placement) -> (usize, usize, usize, usize) {
    let count = PIECES * p.n.div_ceil(Z);
    let ix = (lines_at(p) + p.at[10] - p.at[9]).next_multiple_of(4);
    let kinds = ix + 4 * (count + 1);
    (ix, kinds, (kinds + count).next_multiple_of(4), count)
}

/// Where a 16z tape's newline index lies: past its head and its checkpoints.
fn lines_at(p: &Placement) -> usize {
    p.at[0] + p.at[5] - p.at[4]
}

impl Tape16Z {
    /// The tape from its head (`head_bytes`, magic `MAGIC16Z`), its checkpoints, its newline index and its pieces,
    /// laid as stored.
    fn laid(head: Vec<u8>, p: Placement, mark: u8, source_len: u64, ckpt: &[u8], lines: &[u8], pieces: Pieces) -> Self {
        let (ix, kinds, payload, count) = index_at(&p);
        debug_assert_eq!((head.len(), ckpt.len() + 8, lines.len() + 8, pieces.kinds.len()), (p.at[0], p.at[5] - p.at[4], p.at[10] - p.at[9], count));
        let mut buf = head;
        buf.extend_from_slice(ckpt);
        buf.extend_from_slice(&[0; 8]);
        buf.extend_from_slice(lines);
        buf.extend_from_slice(&[0; 8]);
        buf.resize(ix, 0);
        for o in &pieces.offs {
            buf.extend_from_slice(&o.to_le_bytes());
        }
        buf.extend_from_slice(&pieces.kinds);
        buf.resize(payload, 0);
        buf.extend_from_slice(&pieces.payload);
        Tape16Z { id: next_id(), mark, p, source_len, buf, ix, kinds, payload }
    }

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
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// The stored tape, held as it is: its header, part starts and piece index are checked against its length, and
    /// each piece against where it decodes to. No piece is decoded.
    pub fn from_bytes(buf: Vec<u8>) -> Result<Self, String> {
        if buf.len() < HEAD16 || &buf[..8] != MAGIC16Z {
            return Err("not a 16z tape".into());
        }
        let p = header(&buf)?;
        check_parts(&buf, &p)?;
        let (ix, kinds, payload, count) = index_at(&p);
        if buf.len() < payload {
            return Err("a 16z tape's piece index does not fit it".into());
        }
        let source_len = u64_at(&buf, head::SOURCE_LEN)?;
        let l = lines_at(&p);
        check_lines(&buf[l..l + p.at[10] - p.at[9] - 8], &p, source_len)?;
        let t = Tape16Z { id: next_id(), mark: buf[head::MARK], p, source_len, buf, ix, kinds, payload };
        let off = |j: usize| u32_at(&t.buf, ix + 4 * j) as usize;
        if off(0) != 0 || off(count) != t.buf.len() - payload || (0..count).any(|j| off(j) > off(j + 1)) {
            return Err("a 16z tape's piece index does not fit it".into());
        }
        for j in 0..count {
            let want = t.decoded(j);
            let (kind, bytes) = t.piece(j);
            let fits = match kind {
                RAW => bytes.len() == want,
                LZ4 => !bytes.is_empty(),
                REPEAT => {
                    let earlier = if bytes.len() == 4 { u32_at(bytes, 0) as usize } else { usize::MAX };
                    earlier < j && t.buf[kinds + earlier] != REPEAT && t.decoded(earlier) == want
                }
                _ => false,
            };
            if !fits {
                return Err(format!("a 16z tape's piece {j} does not fit it"));
            }
        }
        Ok(t)
    }

    /// The length of the format 16 tape it decodes to (`decode_into`).
    pub fn decoded_len(&self) -> usize {
        self.p.at[REGIONS]
    }

    /// Where block `b`'s piece `q` decodes to in the format 16 tape: the array's bytes for its nodes, and its side
    /// list's part.
    fn dest(&self, b: usize, q: usize) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        let w = [1, self.p.widths[0], self.p.widths[1], self.p.widths[2]][q];
        let (i0, nb) = (b * Z, Z.min(self.p.n - b * Z));
        let fixed = self.p.at[q] + i0 * w;
        let side = self.p.at[5 + q];
        (fixed..fixed + nb * w, side + self.p.part_start(&self.buf, q, b)..side + self.p.part_start(&self.buf, q, b + 1))
    }

    /// Piece `j`'s decoded length.
    fn decoded(&self, j: usize) -> usize {
        let (fixed, side) = self.dest(j / PIECES, j % PIECES);
        fixed.len() + side.len()
    }

    /// Every piece's decode as a device kernel takes it (decision 0336 §8), in order: where its stored bytes lie in
    /// `as_bytes` (a repeat's, the earlier piece's), whether they are LZ4, and where they decode to in the format 16
    /// tape. With `decode_head_into`, the pieces make the tape.
    pub fn piece_jobs(&self) -> impl Iterator<Item = PieceJob> + '_ {
        let start = |j: usize| self.payload + u32_at(&self.buf, self.ix + 4 * j) as usize;
        (0..PIECES * self.p.n.div_ceil(Z)).map(move |j| {
            let k = if self.buf[self.kinds + j] == REPEAT { u32_at(&self.buf, start(j)) as usize } else { j };
            let (fixed, side) = self.dest(j / PIECES, j % PIECES);
            PieceJob { src: start(k)..start(k + 1), lz4: self.buf[self.kinds + k] == LZ4, fixed, side }
        })
    }

    /// Piece `j`'s kind and its stored bytes.
    fn piece(&self, j: usize) -> (u8, &[u8]) {
        let at = |j: usize| self.payload + u32_at(&self.buf, self.ix + 4 * j) as usize;
        (self.buf[self.kinds + j], &self.buf[at(j)..at(j + 1)])
    }

    /// Piece `j`'s decoded bytes: its stored bytes where it is raw, else decoded into `scratch`.
    fn piece_bytes<'a>(&'a self, j: usize, scratch: &'a mut Vec<u8>) -> Result<&'a [u8], String> {
        let (kind, bytes) = self.piece(j);
        match kind {
            RAW => Ok(bytes),
            LZ4 => {
                scratch.resize(self.decoded(j), 0);
                lz4::decompress(bytes, scratch)?;
                Ok(scratch)
            }
            // An earlier piece that is not a repeat (`from_bytes`).
            _ => self.piece_bytes(u32_at(bytes, 0) as usize, scratch),
        }
    }

    /// The format 16 tape's parts that are not in blocks, into `out` (`decoded_len` bytes): its header and part
    /// starts, its checkpoints, its newline index, and each region's room. With every block (`decode_block_into`), `out` is the format
    /// 16 tape (`decode_into`).
    pub fn decode_head_into(&self, out: &mut [u8]) -> Result<(), String> {
        if out.len() != self.decoded_len() {
            return Err(format!("{} bytes for a format 16 tape of {}", out.len(), self.decoded_len()));
        }
        let at = &self.p.at;
        out[..at[0]].copy_from_slice(&self.buf[..at[0]]);
        out[..8].copy_from_slice(MAGIC16);
        out[at[4]..at[5]].copy_from_slice(&self.buf[at[0]..at[0] + at[5] - at[4]]);
        let l = lines_at(&self.p);
        out[at[9]..at[10]].copy_from_slice(&self.buf[l..l + at[10] - at[9]]);
        for r in 0..REGIONS {
            out[at[r + 1] - 8..at[r + 1]].fill(0);
        }
        Ok(())
    }

    /// Block `b` decoded where it lies in the format 16 tape `out` (`decoded_len` bytes): each of its pieces, its
    /// array's bytes and its side part. No other block is read; `scratch` holds an LZ4 piece while it is copied.
    pub fn decode_block_into(&self, b: usize, out: &mut [u8], scratch: &mut Vec<u8>) -> Result<(), String> {
        if out.len() != self.decoded_len() || b >= self.p.n.div_ceil(Z) {
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
    pub fn decode_into(&self, out: &mut [u8]) -> Result<(), String> {
        self.decode_head_into(out)?;
        let mut scratch = Vec::new();
        for b in 0..self.p.n.div_ceil(Z) {
            self.decode_block_into(b, out, &mut scratch)?;
        }
        Ok(())
    }

    /// Read with `lang`'s choices (its revision and widths).
    pub fn reader<'a>(&'a self, lang: &'a Lang) -> Result<F16Z<'a>, String> {
        if self.mark != lang.revision || self.p.widths != lang.widths {
            return Err(format!("a tape of revision {} (widths {:?}), read with revision {} ({:?})", self.mark, self.p.widths, lang.revision, lang.widths));
        }
        Ok(F16Z { t: self, lang })
    }
}

fn next_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A decoded block: its tape and number, its bytes (its regions, then each side list's part starts: one part each),
/// and its view, with its language's choices; and the room an LZ4 piece is decoded in.
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

/// A 16z tape read a block at a time: the block a read falls in is decoded to format 16's bytes for its nodes (the
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
    /// Block `b` as a format 16 tape of its nodes: its pieces' arrays and side parts, its checkpoints, each region's
    /// room, and each side list's one part.
    #[cold]
    #[inline(never)]
    fn decode(&self, b: usize, slot: &mut Slot) {
        let t = self.t;
        let nb = Z.min(t.p.n - b * Z);
        let sides: [usize; 4] = std::array::from_fn(|k| t.p.part_start(&t.buf, k, b + 1) - t.p.part_start(&t.buf, k, b));
        // No newline index: a block's view answers no line (the whole tape's does).
        let at = layout(0, nb, t.p.widths, t.p.cw, sides, 0);
        let end = at[REGIONS];
        slot.raw.clear();
        slot.raw.resize(end + 32, 0);
        let mut scratch = std::mem::take(&mut slot.scratch);
        for q in 0..PIECES {
            let fixed = at[q + 1] - 8 - at[q];
            let bytes = t.piece_bytes(PIECES * b + q, &mut scratch).expect("a 16z block decodes to its length");
            slot.raw[at[q]..at[q] + fixed].copy_from_slice(&bytes[..fixed]);
            slot.raw[at[5 + q]..at[5 + q] + sides[q]].copy_from_slice(&bytes[fixed..]);
        }
        slot.scratch = scratch;
        let (c0, cn) = (b * Z / K * t.p.cw, nb.div_ceil(K) * t.p.cw);
        slot.raw[at[4]..at[4] + cn].copy_from_slice(&t.buf[t.p.at[0] + c0..t.p.at[0] + c0 + cn]);
        // Each side list's one part: from 0 to its length.
        for (k, &len) in sides.iter().enumerate() {
            slot.raw[end + 8 * k + 4..end + 8 * k + 8].copy_from_slice(&(len as u32).to_le_bytes());
        }
        let p = Placement { n: nb, cw: t.p.cw, at, parts: std::array::from_fn(|k| end + 8 * k), vw: t.p.vw, widths: t.p.widths, newlines: 0 };
        slot.view = p.frame(0).and_then(|frame| bind(frame, self.lang)).expect("a block's view: its widths are its tape's, checked when read");
        slot.b = Some((t.id, b));
    }
}

impl Tape for F16Z<'_> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.t.p.n
    }
    #[inline(always)]
    fn table(&self) -> &CodeTable {
        &self.lang.table
    }
    #[inline(always)]
    fn code(&self, i: usize) -> u32 {
        self.with(i / Z, |r, v| reads::tape_number(r, v, (i % Z) as u32))
    }
    #[inline(always)]
    fn subtree_end(&self, i: usize) -> usize {
        i + self.with(i / Z, |r, v| reads::tape_size(r, v, (i % Z) as u32)) as usize
    }
    /// As format 16's, each step in the block it falls in (a parent's sibling steps cross blocks' starts).
    fn parent(&self, i: usize) -> Option<usize> {
        let mut j = i;
        while j > 0 {
            let v = self.with((j - 1) / Z, |r, v| reads::tape_slot(r, v, ((j - 1) % Z) as u32));
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
        u64::from(self.with(i / Z, |r, v| reads::tape_start(r, v, (i % Z) as u32)))
    }
    #[inline(always)]
    fn end(&self, i: usize) -> u64 {
        u64::from(self.with(i / Z, |r, v| reads::tape_end(r, v, (i % Z) as u32)))
    }
    #[inline(always)]
    fn start_next(&self, i: usize, prev_start: u64) -> u64 {
        u64::from(self.with(i / Z, |r, v| reads::tape_start_next(r, v, (i % Z) as u32, prev_start as u32)))
    }
    #[inline(always)]
    fn end_with_start(&self, i: usize, start: u64) -> u64 {
        start + u64::from(self.with(i / Z, |r, v| reads::tape_length(r, v, (i % Z) as u32)))
    }
    /// A block at a time: each decoded once, its code bytes read as format 16's are. The block's groups are taken
    /// before `f` is called, so `f` may read the tape.
    fn match_groups(&self, set: &CodeSet, mut f: impl FnMut(usize, u64)) {
        let (byte, tail) = byte_table(self.lang, set);
        let mut groups = [0u64; Z / 64];
        for b in 0..self.t.p.n.div_ceil(Z) {
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

/// LZ4's block format (`lz4-sys`): HC level 9 to write (0156 §2.1), `LZ4_decompress_safe` to read.
mod lz4 {
    use std::os::raw::c_char;

    pub fn compress(src: &[u8]) -> Vec<u8> {
        let n = i32::try_from(src.len()).expect("a piece under 2 GB");
        // SAFETY: the bound is LZ4's own for `n` bytes; both buffers are as long as the lengths given.
        unsafe {
            let cap = lz4_sys::LZ4_compressBound(n);
            let mut out = vec![0u8; cap as usize];
            let k = lz4_sys::LZ4_compress_HC(src.as_ptr() as *const c_char, out.as_mut_ptr() as *mut c_char, n, cap, 9);
            assert!(k > 0, "LZ4 compresses a piece");
            out.truncate(k as usize);
            out
        }
    }

    /// `src` into `dst`, which is exactly the piece's length.
    pub fn decompress(src: &[u8], dst: &mut [u8]) -> Result<(), String> {
        let (Ok(n), Ok(cap)) = (i32::try_from(src.len()), i32::try_from(dst.len())) else { return Err("a 16z piece past 2 GB".into()) };
        // SAFETY: LZ4 reads at most `src.len()` bytes and writes at most `dst.len()`.
        let k = unsafe { lz4_sys::LZ4_decompress_safe(src.as_ptr() as *const c_char, dst.as_mut_ptr() as *mut c_char, n, cap) };
        if k == cap { Ok(()) } else { Err(format!("a 16z piece decodes to {k} bytes, not {cap}")) }
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
        assert_eq!(z.as_bytes(), plain.blocks().as_bytes(), "16z written directly is format 16's blocks");
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
        // The pieces as a device decodes them: the head, then each piece from its stored bytes alone.
        let mut by_job = vec![0u8; z.decoded_len()];
        z.decode_head_into(&mut by_job).unwrap();
        for job in z.piece_jobs() {
            let stored = &z.as_bytes()[job.src.clone()];
            let mut out = vec![0u8; job.fixed.len() + job.side.len()];
            if job.lz4 { lz4::decompress(stored, &mut out).unwrap() } else { out.copy_from_slice(stored) }
            by_job[job.fixed.clone()].copy_from_slice(&out[..job.fixed.len()]);
            by_job[job.side.clone()].copy_from_slice(&out[job.fixed.len()..]);
        }
        assert_eq!(by_job, plain.as_bytes(), "16z's piece jobs decode to format 16's bytes");
        let p = placement(plain.as_bytes()).unwrap();
        assert_eq!(p, plain.p, "the placement is the reader's");
        // The header holds each region's start: the placement is read, not summed.
        let starts = u32s(plain.as_bytes(), head::STARTS, REGIONS + 1).unwrap();
        assert!(starts.iter().zip(p.at).all(|(&s, a)| s as usize == a), "the header's starts are the regions'");
        lines_answer(&plain.reader(lang).unwrap(), &source);
        plain
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
            let z = Tape16Z::from_bytes(z.into_bytes()).unwrap();
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
        let z = Tape16Z::from_bytes(tagged.blocks().into_bytes()).unwrap();
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
        let z = Tape16Z::from_bytes(t.blocks().into_bytes()).unwrap();
        assert!(z.len() > 6 * Z, "several blocks");
        assert_eq!(Tape16::from_bytes(t.as_bytes().to_vec()).unwrap(), t);
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
        Tape16Z::from_bytes(t_low.blocks().into_bytes()).unwrap().reader(&lang).unwrap().match_groups(&set, |base, m| c.push((base, m)));
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
        let kinds = &z.as_bytes()[z.kinds..z.kinds + PIECES * n.div_ceil(Z)];
        assert!(kinds.contains(&REPEAT) && kinds.contains(&LZ4), "kinds {kinds:?}");
        same(&plain, &z.reader(&lang).unwrap(), &[0, 1, 2, 3]);
        // A repeat that names itself, and a piece index past the payload, are refused.
        let j = kinds.iter().position(|&k| k == REPEAT).unwrap();
        let mut bad = z.as_bytes().to_vec();
        let at = z.payload + u32_at(&bad, z.ix + 4 * j) as usize;
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
        // Format 15.
        let (image, _) = crate::tape::tests::sample();
        let (columns, table) = crate::tape::f15_columns(&image).unwrap();
        let f15 = crate::tape::F15::new(&image, columns, &table).unwrap();
        let v = crate::tape::f15_view(&image, 0).unwrap();
        same_reads(&reads, &f15, &image, &v);
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
}
