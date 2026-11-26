//! Format 16 tape nodes as symbols: a node's symbol is its row as the uncompressed tape stores it (decision 0434):
//! its code byte, slot, gap and length cells, at the language's widths, escapes as written. The row is read where
//! the tape holds it, in the capture's bytes as loaded; nothing holds the rows a second time.
//!
//! Each language is its own suffix array (a code means nothing in another language's table). The rows' values are
//! ranked by a dictionary of the distinct rows (an interned value, written once where it is computed), so the
//! suffix array's alphabet is dense.
//!
//! Why rows resist local change: a code is the node's (kind, field, flags); a slot is an inner node's subtree size or
//! a leaf's step back to its sibling; a gap is its start less the previous node's start; a length is its own extent.
//! None holds an absolute place, so an edit changes the rows of the nodes it touches and their ancestors' sizes.

use crate::multi::Docs;
use rerail_tape_format::f16::{self, Placement, Z};
use rerail_tape_format::reads::{TapeView, f16 as reads};
use std::collections::HashMap;
use std::ops::Range;

/// A capture (`rerail-engine capture-tapes`): `T16CAP01`, then per file a path, a tape tag, the source and its
/// format 16 tape. The files' parts are ranges of the loaded capture.
pub struct Capture {
    pub bytes: Vec<u8>,
    pub files: Vec<CapturedFile>,
}

pub struct CapturedFile {
    pub path: String,
    pub tag: u8,
    pub source: Range<usize>,
    pub image: Range<usize>,
}

impl Capture {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !bytes.starts_with(b"T16CAP01") {
            return Err("not a T16CAP01 capture".into());
        }
        let mut at = 8;
        let mut files = Vec::new();
        let take = |at: &mut usize, n: usize| -> Result<Range<usize>, String> {
            let r = *at..*at + n;
            if r.end > bytes.len() {
                return Err("a short capture".into());
            }
            *at = r.end;
            Ok(r)
        };
        let u32_at = |r: Range<usize>| u32::from_le_bytes(bytes[r].try_into().unwrap()) as usize;
        while at < bytes.len() {
            let r = take(&mut at, 2)?;
            let n = u16::from_le_bytes(bytes[r].try_into().unwrap()) as usize;
            let path = String::from_utf8_lossy(&bytes[take(&mut at, n)?]).into_owned();
            let tag = bytes[take(&mut at, 1)?.start];
            let n = u32_at(take(&mut at, 4)?);
            let source = take(&mut at, n)?;
            let n = u32_at(take(&mut at, 4)?);
            let image = take(&mut at, n)?;
            files.push(CapturedFile { path, tag, source, image });
        }
        Ok(Self { bytes, files })
    }
}

/// One language's tapes as documents of rows.
pub struct NodeDocs<'a> {
    pub capture: &'a Capture,
    /// The capture's files of this language, by index.
    pub files: Vec<usize>,
    pub placements: Vec<Placement>,
    pub views: Vec<TapeView>,
    rank: HashMap<u128, u32>,
    cells: Cells,
}

/// Which cells of a row the symbol holds: all four, or the code and slot only (structure, no text extents), or
/// the row made indent-proof (`Indent`):
/// - a gap that crosses a line break is a flag and the previous node's distance to its line's end (the previous
///   line's own text), not the bytes to this start, which hold the indentation and blank lines;
/// - an inner node's length is left out (a multi-line node's extent holds every inner line's indentation); a
///   leaf's length is kept.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cells {
    Row,
    CodeSlot,
    Indent,
    /// `Indent`, with every slot's distance left out, its leaf tag kept:
    /// - an inner node's slot is its subtree size, different in each copy for a node that starts in a copy and ends
    ///   past it;
    /// - a leaf's slot steps back to its previous sibling, or to its parent on every K-th sibling, so it depends on
    ///   the sibling's index mod K and can reach outside the copy.
    Local,
}

impl<'a> NodeDocs<'a> {
    pub fn new(capture: &'a Capture, tag: u8, cells: Cells) -> Result<Self, String> {
        let lang = f16::lang(tag).ok_or_else(|| format!("tape tag {tag} has no format 16 table"))?;
        let mut docs = Self { capture, files: Vec::new(), placements: Vec::new(), views: Vec::new(), rank: HashMap::new(), cells };
        for (k, f) in capture.files.iter().enumerate().filter(|(_, f)| f.tag == tag) {
            let image = &capture.bytes[f.image.clone()];
            let p = f16::placement(image).map_err(|e| format!("{}: {e}", f.path))?;
            let v = p.view(lang, 0).map_err(|e| format!("{}: {e}", f.path))?;
            docs.files.push(k);
            docs.placements.push(p);
            docs.views.push(v);
        }
        // The distinct rows, ranked in their value's order.
        let mut distinct = std::collections::HashSet::new();
        for d in 0..docs.files.len() {
            for j in 0..docs.placements[d].n {
                distinct.insert(docs.value(d, j, cells));
            }
        }
        let mut values: Vec<u128> = distinct.into_iter().collect();
        values.sort_unstable();
        docs.rank = values.into_iter().enumerate().map(|(r, v)| (v, r as u32)).collect();
        Ok(docs)
    }

    pub fn image(&self, d: usize) -> &'a [u8] {
        &self.capture.bytes[self.capture.files[self.files[d]].image.clone()]
    }

    pub fn source(&self, d: usize) -> &'a [u8] {
        &self.capture.bytes[self.capture.files[self.files[d]].source.clone()]
    }

    pub fn path(&self, d: usize) -> &'a str {
        &self.capture.files[self.files[d]].path
    }

    /// Node `j`'s row in document `d`, its cells as stored (little-endian, at the tape's widths) packed into one value.
    #[inline(always)]
    pub fn value(&self, d: usize, j: usize, cells: Cells) -> u128 {
        let (t, p) = (self.image(d), &self.placements[d]);
        let (b, r) = (j / Z, j % Z);
        let cell = |col: usize, w: usize| -> u128 {
            let at = p.column(b, col) + r * w;
            let mut v = 0u128;
            for (k, &byte) in t[at..at + w].iter().enumerate() {
                v |= u128::from(byte) << (8 * k);
            }
            v
        };
        let [w0, w1, w2] = p.widths;
        let code_slot = cell(0, 1) | cell(1, w0) << 8;
        match cells {
            Cells::CodeSlot => code_slot,
            Cells::Row => code_slot | cell(2, w1) << 40 | cell(3, w2) << 72,
            Cells::Indent | Cells::Local => {
                let (v, i) = (&self.views[d], j as u32);
                let leaf = reads::slot(t, v, i) & 1 == 1;
                let len = if leaf { cell(3, w2) } else { 0 };
                let gap = if i == 0 {
                    0
                } else {
                    // A line break between the two starts, read in the source where it lies: the bytes between
                    // them (the newline index's `line` answers the same, at a rank and select a read).
                    let (s, p) = (reads::start(t, v, i), reads::start(t, v, i - 1));
                    match self.source(d)[p as usize..s as usize].iter().position(|&b| b == b'\n') {
                        None => u128::from(s - p),
                        Some(k) => 1 << 32 | (k as u128 + 1),
                    }
                };
                let code_slot = if cells == Cells::Local { cell(0, 1) | u128::from(leaf) << 8 } else { code_slot };
                code_slot | gap << 40 | len << 80
            }
        }
    }

    pub fn distinct(&self) -> usize {
        self.rank.len()
    }

    /// The source line (from 1) byte `x` lies on, by the tape's newline index.
    pub fn line(&self, d: usize, x: u32) -> u32 {
        reads::line(self.image(d), &self.views[d], x) + 1
    }

    /// The source span of nodes `first..end` of document `d`: from the first's start to the furthest end of a node
    /// whose subtree lies in the range (a node whose subtree runs past the range is cut by it).
    pub fn span(&self, d: usize, first: usize, end: usize) -> (u32, u32) {
        let (t, v) = (self.image(d), &self.views[d]);
        let start = reads::start(t, v, first as u32);
        let mut stop = start;
        for i in first..end {
            if i + reads::size(t, v, i as u32) as usize <= end {
                stop = stop.max(reads::end(t, v, i as u32));
            }
        }
        (start, stop)
    }
}

impl Docs for NodeDocs<'_> {
    fn count(&self) -> usize {
        self.files.len()
    }
    fn doc_len(&self, d: usize) -> usize {
        self.placements[d].n
    }
    #[inline(always)]
    fn at(&self, d: usize, j: usize) -> u32 {
        self.rank[&self.value(d, j, self.cells)]
    }
    fn upper(&self) -> u32 {
        self.rank.len().saturating_sub(1) as u32
    }
}

/// What a line of tape leaves is keyed by (`tape_lines`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LineKey {
    /// Each leaf's code and its token's text: Jeff's normalisation done by tokens.
    Text,
    /// Each leaf's code and length: renamed identifiers and changed literals of the same length still match.
    Shape,
}

/// The hybrid: Jeff's units (the lines that count, numbered by an interned key), each line's key made from the tape
/// instead of its text: the leaves that start on it, in order, comments (any kind whose name ends in `comment`, with
/// its subtree) left out. A line with no leaf left does not count, so blank and comment lines drop as in Jeff's rule,
/// and so do the lines inside a block comment.
pub fn tape_lines(capture: &Capture, key: LineKey) -> crate::lines::LineDocs {
    use crate::lines::LineDocs;
    let mut ids_of: HashMap<Vec<u8>, u32> = HashMap::new();
    let mut docs = LineDocs { paths: Vec::new(), ids: Vec::new(), lines: Vec::new(), distinct: 0 };
    for f in &capture.files {
        let Some(lang) = f16::lang(f.tag) else { continue };
        let t = &capture.bytes[f.image.clone()];
        let src = &capture.bytes[f.source.clone()];
        let Ok(v) = f16::placement(t).and_then(|p| p.view(lang, 0)) else { continue };
        let comment = |num: u32| {
            let kind = lang.table().entry(num).kind;
            lang.names().kind_name(kind).is_some_and(|n| n.ends_with("comment"))
        };
        let (mut ids, mut lines) = (Vec::new(), Vec::new());
        let mut cur: Vec<u8> = Vec::new();
        let mut cur_line = 0u32;
        let mut flush = |cur: &mut Vec<u8>, line: u32, ids: &mut Vec<u32>, lines: &mut Vec<u32>| {
            if !cur.is_empty() {
                let next = ids_of.len() as u32;
                ids.push(*ids_of.entry(std::mem::take(cur)).or_insert(next));
                lines.push(line);
            }
        };
        let mut j = 0u32;
        while j < v.nodes {
            let num = reads::number(t, &v, j);
            if comment(num) {
                j += reads::size(t, &v, j).max(1);
                continue;
            }
            if reads::slot(t, &v, j) & 1 == 1 {
                let (s, e) = (reads::start(t, &v, j), reads::end(t, &v, j));
                if e > s {
                    let line = reads::line(t, &v, s) + 1;
                    if line != cur_line {
                        flush(&mut cur, cur_line, &mut ids, &mut lines);
                        cur_line = line;
                    }
                    cur.extend_from_slice(&num.to_le_bytes());
                    cur.extend_from_slice(&(e - s).to_le_bytes());
                    if key == LineKey::Text {
                        cur.extend_from_slice(&src[s as usize..e as usize]);
                    }
                }
            }
            j += 1;
        }
        flush(&mut cur, cur_line, &mut ids, &mut lines);
        docs.paths.push(f.path.clone());
        docs.ids.push(ids);
        docs.lines.push(lines);
    }
    docs.distinct = ids_of.len() as u32;
    docs
}
