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

/// Where a stored format 16 or 16z tape's header holds each value (format 15's `header` where they agree).
mod head {
    /// The language's tape tag, a byte; then the revision mark and the checkpoints' width.
    pub const LANGUAGE: usize = 8;
    pub const MARK: usize = 9;
    pub const CW: usize = 10;
    /// The source's length, u64; the node count, u64.
    pub const SOURCE_LEN: usize = 16;
    pub const NODES: usize = 24;
    /// The side lists' value widths, a byte each; then (16z) the slot, gap and length widths.
    pub const VW: usize = 32;
    pub const WIDTHS: usize = 36;
    /// Format 16: each region's length, u64. 16z: the compressed bytes' length, u64.
    pub const LENGTHS: usize = 40;
}
const HEAD16: usize = head::LENGTHS + 8 * REGIONS;
const HEAD16Z: usize = head::LENGTHS + 8;

fn u64_at(b: &[u8], at: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(b.get(at..at + 8).ok_or("a short tape header")?.try_into().expect("8 bytes")))
}

fn u32s(b: &[u8], at: usize, count: usize) -> Result<Vec<u32>, String> {
    let bytes = b.get(at..at + 4 * count).ok_or("a short block index")?;
    Ok(bytes.as_chunks::<4>().0.iter().map(|&w| u32::from_le_bytes(w)).collect())
}

/// One document's format 16 tape, held as stored (`as_bytes`) and read where it lies: the header, each side list's
/// part starts, then the regions: codes, slots, gaps, lengths, checkpoints, and the four side lists (codes' low
/// parts, then escaped slots, gaps, lengths), each followed by 8 bytes of room for a word's load. It is read through
/// the tape's reads (decision 0310) over its view, which it keeps but for its language's choices (`reader`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tape16 {
    pub mark: u8,
    n: usize,
    /// The checkpoints' width in bytes, from the source's length.
    cw: usize,
    source_len: u64,
    /// As stored: its own, or placed in a slab's room (decision 0353 step 4a).
    buf: crate::home::Home,
    /// Each region's start in `buf`, one more at the end.
    at: [usize; REGIONS + 1],
    vw: [usize; 4],
    /// Its view without its language's choices (`Placement::frame`), or none for a tape or a source past 4 GiB,
    /// which the reads' u32 offsets do not reach.
    frame: Option<TapeView>,
}

impl Tape16 {
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
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
    /// Names the language's tape tag in its header: the writer does not know it.
    pub fn set_tag(&mut self, tag: u8) {
        self.buf.own_mut()[head::LANGUAGE] = tag;
    }
    /// The tape as stored.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }
    /// Where its bytes are.
    pub fn home(&self) -> &crate::home::Home {
        &self.buf
    }
    /// Region `r` with its 8 bytes of room.
    fn region(&self, r: usize) -> &[u8] {
        &self.buf[self.at[r]..self.at[r + 1]]
    }

    /// The stored tape, held as it is: its header and part starts are checked against its length.
    pub fn from_bytes(buf: impl Into<crate::home::Home>) -> Result<Self, String> {
        let buf: crate::home::Home = buf.into();
        let p = placement(&buf)?;
        let source_len = u64_at(&buf, head::SOURCE_LEN)?;
        Ok(Tape16 { mark: buf[head::MARK], n: p.n, cw: p.cw, source_len, at: p.at, vw: p.vw, frame: p.frame_of(source_len), buf })
    }

    /// Part `j`'s start in side list `k`'s region (one more at the end).
    fn part_start(&self, k: usize, j: usize) -> usize {
        let at = HEAD16 + 4 * (k * (self.n.div_ceil(PART) + 1) + j);
        u32::from_le_bytes(self.buf[at..at + 4].try_into().expect("4 bytes")) as usize
    }

    /// The tape from its regions (each with its 8 bytes of room) and side lists, laid as stored.
    fn laid(mark: u8, n: usize, cw: usize, source_len: u64, arrays: [Vec<u8>; 5], side: [Side; 4]) -> Self {
        let parts = n.div_ceil(PART) + 1;
        let regions: Vec<&[u8]> = arrays.iter().map(Vec::as_slice).chain(side.iter().map(|s| s.bytes.as_slice())).collect();
        let mut buf = vec![0u8; HEAD16];
        buf[..8].copy_from_slice(MAGIC16);
        buf[head::MARK] = mark;
        buf[head::CW] = cw as u8;
        buf[head::SOURCE_LEN..head::SOURCE_LEN + 8].copy_from_slice(&source_len.to_le_bytes());
        buf[head::NODES..head::NODES + 8].copy_from_slice(&(n as u64).to_le_bytes());
        for (k, s) in side.iter().enumerate() {
            buf[head::VW + k] = s.vw as u8;
        }
        for (r, region) in regions.iter().enumerate() {
            buf[head::LENGTHS + 8 * r..head::LENGTHS + 8 * r + 8].copy_from_slice(&((region.len() - 8) as u64).to_le_bytes());
        }
        for s in &side {
            debug_assert_eq!(s.base.len(), parts);
            for b in &s.base {
                buf.extend_from_slice(&b.to_le_bytes());
            }
        }
        buf.resize(buf.len().next_multiple_of(8), 0);
        let mut at = [0usize; REGIONS + 1];
        at[0] = buf.len();
        for (r, region) in regions.iter().enumerate() {
            buf.extend_from_slice(region);
            at[r + 1] = buf.len();
        }
        let vw = std::array::from_fn(|k| side[k].vw);
        let p = Placement { n, cw, at, parts: std::array::from_fn(|k| HEAD16 + 4 * k * parts), vw };
        Tape16 { mark, n, cw, source_len, buf: buf.into(), at, vw, frame: p.frame_of(source_len) }
    }

    /// Read with `lang`'s choices: the tape's mark is `lang`'s revision, else it is rewritten before it is read.
    pub fn reader<'a>(&'a self, lang: &'a Lang) -> Result<F16<'a>, String> {
        if self.mark != lang.revision {
            return Err(format!("a tape of revision {}, read with revision {}", self.mark, lang.revision));
        }
        let frame = self.frame.ok_or("a format 16 tape, or its source, past 4 GiB: the reads' offsets do not reach it")?;
        Ok(F16 { bytes: &self.buf, view: bind(frame, lang)?, lang })
    }
}

/// Where a stored format 16 tape's parts lie in its bytes, for a reader that reads them there without this crate
/// (the device's, `metal_tape.metal`): each region's start (codes, slots, gaps, lengths, checkpoints, the four
/// side lists; one more at the end), each side list's part starts (u32s, a part's start and one more at the end),
/// the node count, the checkpoints' width and the side lists' value widths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub n: usize,
    pub cw: usize,
    pub at: [usize; REGIONS + 1],
    pub parts: [usize; 4],
    pub vw: [usize; 4],
}

impl Placement {
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

/// The placement of the stored format 16 tape `buf`, its header and part starts checked against its length as
/// `Tape16::from_bytes` checks them, without copying a byte.
pub fn placement(buf: &[u8]) -> Result<Placement, String> {
    if buf.len() < HEAD16 || &buf[..8] != MAGIC16 {
        return Err("not a format 16 tape".into());
    }
    let n = usize::try_from(u64_at(buf, head::NODES)?).map_err(|_| "a tape length overflows")?;
    let parts = n.div_ceil(PART) + 1;
    let mut at = [0usize; REGIONS + 1];
    let starts = HEAD16.checked_add(4 * 4 * parts).ok_or("a tape length overflows")?;
    at[0] = starts.next_multiple_of(8);
    for r in 0..REGIONS {
        let len = usize::try_from(u64_at(buf, head::LENGTHS + 8 * r)?).map_err(|_| "a tape length overflows")?;
        at[r + 1] = at[r].checked_add(len).and_then(|a| a.checked_add(8)).ok_or("a tape length overflows")?;
    }
    if buf.len() != at[REGIONS] {
        return Err(format!("a format 16 tape of {} bytes, its header says {}", buf.len(), at[REGIONS]));
    }
    let part = |k: usize| HEAD16 + 4 * k * parts;
    for k in 0..4 {
        let b = buf[part(k)..part(k) + 4 * parts].as_chunks::<4>().0;
        let b = |j: usize| u32::from_le_bytes(b[j]);
        if (1..parts).any(|j| b(j - 1) > b(j)) || b(parts - 1) as usize + 8 != at[6 + k] - at[5 + k] {
            return Err(format!("side list {k}'s part starts do not fit it"));
        }
    }
    let vw = std::array::from_fn(|k| usize::from(buf[head::VW + k]));
    if vw.iter().any(|&w| w > 8) || usize::from(buf[head::CW]) > 8 {
        return Err("a format 16 tape's widths are out of range".into());
    }
    Ok(Placement { n, cw: usize::from(buf[head::CW]), at, parts: std::array::from_fn(part), vw })
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

/// Where `write` puts each finished block: format 16's regions and side lists, or 16z's compressed blocks.
enum Out {
    Plain { regions: [Vec<u8>; 5], side: [(Vec<u8>, Vec<u32>); 4] },
    Blocks { comp: Vec<u8>, at: Vec<u32>, sides: Vec<[u32; 4]>, raw: Vec<u8> },
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
pub fn write(lang: &Lang, n: usize, source_len: u64, tag: u8, z: bool, mut node: impl FnMut(usize) -> Node) -> Result<Written, String> {
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
        Out::Blocks { comp: Vec::new(), at: vec![0], sides: Vec::with_capacity(n.div_ceil(Z)), raw: Vec::new() }
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
            Out::Blocks { comp, at, sides, raw } => {
                raw.clear();
                let mut region = |bytes: &[u8]| {
                    raw.extend_from_slice(bytes);
                    raw.extend_from_slice(&[0; 8]);
                };
                region(&c.codes);
                for a in 0..3 {
                    region(&c.arrays[a]);
                }
                region(&c.ckpt);
                let mut lens = [0u32; 4];
                let mut part = Vec::new();
                for k in 0..4 {
                    part.clear();
                    part_into(&mut part, &c.side[k], nb, vw[k]);
                    lens[k] = part.len() as u32;
                    region(&part);
                }
                debug_assert_eq!(raw.len(), layout(nb, w, cw, lens)[REGIONS]);
                comp.extend_from_slice(&lz4::compress(raw));
                at.push(comp.len() as u32);
                sides.push(lens);
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
        for a in 0..3 {
            c.arrays[a].resize((j + 1) * w[a], 0);
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
            let mut t = Tape16::laid(mark, n, cw, source_len, arrays, side);
            t.set_tag(tag);
            Written::F16(t)
        }
        Out::Blocks { comp, at, sides, .. } => Written::F16Z(Tape16Z::laid(mark, tag, n, cw, source_len, w, vw, comp, at, sides)),
    })
}

const NONE: u32 = u32::MAX;

/// Format 16 (or 16z where `z`) written from another tape `t`, each node's code mapped to the language's number
/// (`number`, by `t`'s code), in one pass (`write`): a tape a parser did not write in format 16 (decision 0293).
pub fn write_from<T: Tape>(lang: &Lang, t: &T, number: &[u32], source_len: u64, tag: u8, z: bool) -> Result<Written, String> {
    let mut s = 0;
    write(lang, t.len(), source_len, tag, z, |i| {
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

/// One group of up to 64 nodes from `base` of a run whose code bytes are `codes`: those whose code is in the set
/// (`byte` below the cutoff, `tail` when the set holds a number past it; `number` a node's code past it).
#[inline(always)]
fn group(codes: &[u8], cut: usize, set: &CodeSet, byte: &[bool; 256], tail: bool, base: usize, number: impl Fn(usize) -> u32) -> u64 {
    let mut m = 0u64;
    for j in 0..64.min(codes.len() - base) {
        let b = codes[base + j] as usize;
        let hit = if b < cut { byte[b] } else { tail && set.contains(number(base + j)) };
        m |= u64::from(hit) << j;
    }
    m
}

/// A set's test a code byte at a time: membership of each number below the cutoff, and whether it holds any past.
fn byte_table(lang: &Lang, set: &CodeSet) -> ([bool; 256], bool) {
    let cut = lang.cut as usize;
    let mut byte = [false; 256];
    for (b, m) in byte.iter_mut().enumerate().take(cut) {
        *m = set.contains(b as u32);
    }
    (byte, set.by_number().iter().skip(cut).any(|&m| m))
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
        for base in (0..codes.len()).step_by(64) {
            f(base, group(codes, self.lang.cut as usize, set, &byte, tail, base, |i| self.code(i)));
        }
    }
}

// ---- format 16 in fixed blocks (16z) -----------------------------------------------------------------------

/// Nodes a fixed block holds: one side-list part, so every read of a node but a parent's sibling steps stays in
/// its block.
pub const Z: usize = PART;

/// The regions of a block decoded, in order: codes, slots, gaps, lengths, checkpoints, the four side parts. Each is
/// followed by 8 bytes of room for a word's load.
const REGIONS: usize = 9;

/// Where each region of a block of `nb` nodes starts, and the end: from the widths and its side parts' lengths.
fn layout(nb: usize, w: [usize; 3], cw: usize, sides: [u32; 4]) -> [usize; REGIONS + 1] {
    let len = [nb, nb * w[0], nb * w[1], nb * w[2], nb.div_ceil(K) * cw, sides[0] as usize, sides[1] as usize, sides[2] as usize, sides[3] as usize];
    let mut at = [0usize; REGIONS + 1];
    for r in 0..REGIONS {
        at[r + 1] = at[r] + len[r] + 8;
    }
    at
}

/// One document's format 16 tape in fixed LZ4 blocks of `Z` nodes (0156 §2.1), held as stored (`as_bytes`): the
/// header, the block index (each block's start in the compressed bytes, one more at the end; each block's side
/// parts' lengths), then the compressed blocks. A block decodes to the tape's own bytes for its nodes, read in
/// place as format 16 is. The index is read in when the tape is.
#[derive(Clone, Debug)]
pub struct Tape16Z {
    /// Which tape its decoded blocks are, in the thread's cache (a clone holds the same bytes).
    id: u64,
    pub mark: u8,
    n: usize,
    cw: usize,
    source_len: u64,
    widths: [usize; 3],
    vw: [usize; 4],
    /// As stored, its blocks compressed: its own, or placed in a slab's room (decision 0353 step 4a).
    buf: crate::home::Home,
    /// Where the compressed blocks start in `buf`.
    comp: usize,
    /// Each block's start in the compressed bytes, one more at the end.
    at: Vec<u32>,
    /// Each block's side parts' lengths.
    sides: Vec<[u32; 4]>,
}

impl Tape16 {
    /// This tape in fixed blocks, each compressed alone (no dictionary yet: 0293, not done).
    pub fn blocks(&self, lang: &Lang) -> Tape16Z {
        let (n, w, cw) = (self.n, lang.widths, self.cw);
        let blocks = n.div_ceil(Z);
        let (mut comp, mut at, mut sides) = (Vec::new(), vec![0u32], Vec::with_capacity(blocks));
        let mut raw = Vec::new();
        for b in 0..blocks {
            let (i0, nb) = (b * Z, Z.min(n - b * Z));
            let part = |k: usize| (self.part_start(k, b), self.part_start(k, b + 1));
            let lens: [u32; 4] = std::array::from_fn(|k| {
                let (p0, p1) = part(k);
                (p1 - p0) as u32
            });
            raw.clear();
            let mut region = |bytes: &[u8]| {
                raw.extend_from_slice(bytes);
                raw.extend_from_slice(&[0; 8]);
            };
            region(&self.region(0)[i0..i0 + nb]);
            region(&self.region(1)[i0 * w[0]..(i0 + nb) * w[0]]);
            region(&self.region(2)[i0 * w[1]..(i0 + nb) * w[1]]);
            region(&self.region(3)[i0 * w[2]..(i0 + nb) * w[2]]);
            let c0 = i0 / K;
            region(&self.region(4)[c0 * cw..(c0 + nb.div_ceil(K)) * cw]);
            for k in 0..4 {
                let (p0, p1) = part(k);
                region(&self.region(5 + k)[p0..p1]);
            }
            debug_assert_eq!(raw.len(), layout(nb, w, cw, lens)[REGIONS]);
            comp.extend_from_slice(&lz4::compress(&raw));
            at.push(comp.len() as u32);
            sides.push(lens);
        }
        Tape16Z::laid(self.mark, self.tag(), n, cw, self.source_len, w, self.vw, comp, at, sides)
    }
}

impl Tape16Z {
    /// The tape from its compressed blocks, its block index and its choices, laid as stored.
    #[allow(clippy::too_many_arguments)]
    fn laid(mark: u8, tag: u8, n: usize, cw: usize, source_len: u64, w: [usize; 3], vw: [usize; 4], comp: Vec<u8>, at: Vec<u32>, sides: Vec<[u32; 4]>) -> Self {
        let mut buf = vec![0u8; HEAD16Z];
        buf[..8].copy_from_slice(MAGIC16Z);
        buf[head::LANGUAGE] = tag;
        buf[head::MARK] = mark;
        buf[head::CW] = cw as u8;
        buf[head::SOURCE_LEN..head::SOURCE_LEN + 8].copy_from_slice(&source_len.to_le_bytes());
        buf[head::NODES..head::NODES + 8].copy_from_slice(&(n as u64).to_le_bytes());
        for k in 0..4 {
            buf[head::VW + k] = vw[k] as u8;
        }
        for k in 0..3 {
            buf[head::WIDTHS + k] = w[k] as u8;
        }
        buf[head::LENGTHS..head::LENGTHS + 8].copy_from_slice(&(comp.len() as u64).to_le_bytes());
        for x in at.iter().chain(sides.iter().flatten()) {
            buf.extend_from_slice(&x.to_le_bytes());
        }
        let start = buf.len();
        buf.extend_from_slice(&comp);
        Tape16Z { id: next_id(), mark, n, cw, source_len, widths: w, vw, buf: buf.into(), comp: start, at, sides }
    }
}

impl Tape16Z {
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// The bytes it takes in memory: as stored, and the block index read in.
    pub fn bytes(&self) -> usize {
        self.buf.len() + 4 * self.at.len() + 16 * self.sides.len()
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
    /// Where its bytes are.
    pub fn home(&self) -> &crate::home::Home {
        &self.buf
    }

    /// The stored tape, held as it is: its header and block index are checked against its length.
    pub fn from_bytes(buf: impl Into<crate::home::Home>) -> Result<Self, String> {
        let buf: crate::home::Home = buf.into();
        if buf.len() < HEAD16Z || &buf[..8] != MAGIC16Z {
            return Err("not a 16z tape".into());
        }
        let n = usize::try_from(u64_at(&buf, head::NODES)?).map_err(|_| "a tape length overflows")?;
        let blocks = n.div_ceil(Z);
        let at = u32s(&buf, HEAD16Z, blocks + 1)?;
        let sides: Vec<[u32; 4]> = u32s(&buf, HEAD16Z + 4 * (blocks + 1), 4 * blocks)?.as_chunks::<4>().0.to_vec();
        let comp = HEAD16Z + 4 * (blocks + 1) + 16 * blocks;
        let len = u64_at(&buf, head::LENGTHS)?;
        if (buf.len() - comp.min(buf.len())) as u64 != len || at.first() != Some(&0) || at.last().map(|&e| u64::from(e)) != Some(len) || at.windows(2).any(|w| w[0] > w[1]) {
            return Err("a 16z tape's block index does not fit it".into());
        }
        let widths = std::array::from_fn(|k| usize::from(buf[head::WIDTHS + k]));
        let vw = std::array::from_fn(|k| usize::from(buf[head::VW + k]));
        Ok(Tape16Z { id: next_id(), mark: buf[head::MARK], n, cw: usize::from(buf[head::CW]), source_len: u64_at(&buf, head::SOURCE_LEN)?, widths, vw, buf, comp, at, sides })
    }

    /// The view the reads written once take of this tape as stored (decision 0353 step 4b,
    /// `rerail_clif::view::F16Z_FORMAT`): its block index, side lengths and compressed bytes where they lie in
    /// `as_bytes`, format 16's widths and `lang`'s choices, its id, and the size of a cache entry that holds its
    /// largest block decoded. A reader reads it through a cache, which the view is given (`view::Reader`, or the
    /// device's).
    pub fn view(&self, lang: &Lang) -> Result<TapeView, String> {
        use rerail_clif::view::{F16Z_BLOCK, F16Z_FORMAT, z};
        const _: () = assert!(F16Z_BLOCK as usize == Z);
        if self.mark != lang.revision || self.widths != lang.widths {
            return Err(format!("a tape of revision {}, read with revision {}", self.mark, lang.revision));
        }
        let blocks = self.n.div_ceil(Z);
        let largest = (0..blocks).map(|b| layout(Z.min(self.n - b * Z), self.widths, self.cw, self.sides[b])[REGIONS]).max().unwrap_or(0);
        let entry = (z::SLOT_BYTES as usize + largest + z::SLACK).next_power_of_two();
        let frame = Placement { n: self.n, cw: self.cw, at: [0; REGIONS + 1], parts: [0; 4], vw: self.vw }.frame(0)?;
        let mut view = bind(frame, lang)?;
        let at = |offset: usize| u32::try_from(offset).map_err(|_| "a tape past 4 GiB".to_owned());
        view.format = F16Z_FORMAT;
        view.columns = [at(HEAD16Z)?, at(HEAD16Z + 4 * (blocks + 1))?, at(self.comp)?, at(entry)?, 0];
        (view.side[2], view.side[3]) = (self.id as u32, (self.id >> 32) as u32);
        Ok(view)
    }

    /// Read with `lang`'s choices (its revision and widths).
    pub fn reader<'a>(&'a self, lang: &'a Lang) -> Result<F16Z<'a>, String> {
        if self.mark != lang.revision || self.widths != lang.widths {
            return Err(format!("a tape of revision {}, read with revision {}", self.mark, lang.revision));
        }
        Ok(F16Z { t: self, lang })
    }
}

fn next_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A decoded block: its tape and number, its bytes (its regions, then each side list's part starts: one part each),
/// and its view, with its language's choices.
#[derive(Clone, Default)]
struct Slot {
    b: Option<(u64, usize)>,
    raw: Vec<u8>,
    view: TapeView,
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

/// A 16z tape read a block at a time: the block a read falls in is decoded to the tape's own bytes for it (the
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
    #[cold]
    #[inline(never)]
    fn decode(&self, b: usize, slot: &mut Slot) {
        let t = self.t;
        let nb = Z.min(t.n - b * Z);
        let at = layout(nb, t.widths, t.cw, t.sides[b]);
        let end = at[REGIONS];
        slot.raw.resize(end + 32, 0);
        lz4::decompress(&t.buf[t.comp + t.at[b] as usize..t.comp + t.at[b + 1] as usize], &mut slot.raw[..end]);
        // Each side list's one part: from 0 to its length.
        for k in 0..4 {
            slot.raw[end + 8 * k..end + 8 * k + 4].copy_from_slice(&0u32.to_le_bytes());
            slot.raw[end + 8 * k + 4..end + 8 * k + 8].copy_from_slice(&t.sides[b][k].to_le_bytes());
        }
        let p = Placement { n: nb, cw: t.cw, at, parts: std::array::from_fn(|k| end + 8 * k), vw: t.vw };
        slot.view = p.frame(0).and_then(|frame| bind(frame, self.lang)).expect("a block's view: its widths are its tape's, checked when read");
        slot.b = Some((t.id, b));
    }
}

impl Tape for F16Z<'_> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.t.n
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
        let cut = self.lang.cut as usize;
        let mut groups = [0u64; Z / 64];
        for b in 0..self.t.n.div_ceil(Z) {
            let count = self.with(b, |r, v| {
                let at = v.columns[0] as usize;
                let codes = &r[at..at + v.nodes as usize];
                for (g, base) in groups.iter_mut().zip((0..codes.len()).step_by(64)) {
                    *g = group(codes, cut, set, &byte, tail, base, |j| reads::tape_number(r, v, j as u32));
                }
                codes.len().div_ceil(64)
            });
            for (g, &m) in groups[..count].iter().enumerate() {
                f(b * Z + 64 * g, m);
            }
        }
    }
}

/// LZ4's block format (the lab's `lz4-sys`, `LZ4_compress_default` and `LZ4_decompress_safe`).
mod lz4 {
    use std::os::raw::c_char;

    pub fn compress(src: &[u8]) -> Vec<u8> {
        let n = i32::try_from(src.len()).expect("a block under 2 GB");
        // SAFETY: the bound is LZ4's own for `n` bytes; both buffers are as long as the lengths given.
        unsafe {
            let cap = lz4_sys::LZ4_compressBound(n);
            let mut out = vec![0u8; cap as usize];
            let k = lz4_sys::LZ4_compress_default(src.as_ptr() as *const c_char, out.as_mut_ptr() as *mut c_char, n, cap);
            assert!(k > 0, "LZ4 compresses a block");
            out.truncate(k as usize);
            out
        }
    }

    /// `src` into `dst`, which is exactly the block's length.
    pub fn decompress(src: &[u8], dst: &mut [u8]) {
        // SAFETY: LZ4 reads at most `src.len()` bytes and writes at most `dst.len()`.
        let k = unsafe { lz4_sys::LZ4_decompress_safe(src.as_ptr() as *const c_char, dst.as_mut_ptr() as *mut c_char, src.len() as i32, dst.len() as i32) };
        assert_eq!(k as usize, dst.len(), "a block decodes to its length");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::{F15, children, f15_columns};

    /// Format 16 written from `t` in one pass, and 16z written the same way, which is the format 16 tape's blocks.
    fn sixteen<T: Tape>(lang: &Lang, t: &T, number: &[u32], source_len: u64) -> Tape16 {
        let Ok(Written::F16(plain)) = write_from(lang, t, number, source_len, 0, false) else { panic!("format 16") };
        let Ok(Written::F16Z(z)) = write_from(lang, t, number, source_len, 0, true) else { panic!("16z") };
        assert_eq!(z.as_bytes(), plain.blocks(lang).as_bytes(), "16z written directly is format 16's blocks");
        let p = placement(plain.as_bytes()).unwrap();
        assert_eq!((p.n, p.cw, p.at, p.vw), (plain.n, plain.cw, plain.at, plain.vw), "the placement is the reader's");
        plain
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
        same(&f15, &t.blocks(&lang).reader(&lang).unwrap(), &number);
        // As stored, and read back where it lies.
        let mut tagged = t.clone();
        tagged.set_tag(7);
        let back = Tape16::from_bytes(tagged.as_bytes().to_vec()).unwrap();
        assert_eq!((back.tag(), back.source_len()), (7, 2_000));
        same(&f15, &back.reader(&lang).unwrap(), &number);
        let z = Tape16Z::from_bytes(tagged.blocks(&lang).as_bytes().to_vec()).unwrap();
        assert_eq!((z.tag(), z.source_len()), (7, 2_000));
        same(&f15, &z.reader(&lang).unwrap(), &number);
        let mut short = tagged.as_bytes().to_vec();
        short.pop();
        assert!(Tape16::from_bytes(short).is_err());
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
        assert!((0..4).all(|k| t.part_start(k, t.n.div_ceil(PART)) > 0), "every side list has entries");
        same(&plain, &t.reader(&lang).unwrap(), &number);
        let set = table.set(|e| e.kind % 97 == 3 || e.kind > 590);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        plain.match_groups(&set, |base, m| a.push((base, m)));
        t.reader(&lang).unwrap().match_groups(&set, |base, m| b.push((base, m)));
        assert_eq!(a, b, "a set past the cutoff");
        // The same tape in fixed blocks: every read decodes the block it falls in.
        let z = Tape16Z::from_bytes(t.blocks(&lang).as_bytes().to_vec()).unwrap();
        assert!(z.len() > 6 * Z, "several blocks");
        assert_eq!(Tape16::from_bytes(t.as_bytes().to_vec()).unwrap(), t);
        same(&plain, &z.reader(&lang).unwrap(), &number);
        let mut c = Vec::new();
        z.reader(&lang).unwrap().match_groups(&set, |base, m| c.push((base, m)));
        assert_eq!(a, c, "a set past the cutoff, in blocks");
        // A bulk answer that reads the tape inside the matches' callback.
        let (mut near, mut near_z) = (Vec::new(), Vec::new());
        crate::tape::nearest_all(&t.reader(&lang).unwrap(), &set, |_| true, &mut near);
        crate::tape::nearest_all(&z.reader(&lang).unwrap(), &set, |_| true, &mut near_z);
        assert_eq!(near, near_z, "nearest in blocks");
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
        let Ok(Written::F16Z(z)) = write_from(&lang, &plain, &(0..600).collect::<Vec<_>>(), pos + 2, 0, true) else { panic!("16z") };
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
        let Ok(Written::F16Z(z)) = write_from(&lang, &plain, &(0..600).collect::<Vec<_>>(), pos + 2, 0, true) else { panic!("16z") };
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
}
