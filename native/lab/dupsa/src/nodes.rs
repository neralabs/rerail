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
    /// The capture file, mapped: the tapes are read where the file holds them.
    pub bytes: &'static [u8],
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
        let bytes = crate::store::map(path)?;
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
    rank: HashMap<u128, u32, Fx>,
    cells: Cells,
    /// The symbol relation: each node's symbol (its row's rank), derived once from the tape in tape order and kept
    /// here as the one place the symbols live; laid out as the joined text the suffix array sorts (rank + 1 per
    /// node, 0 after each document). `starts[d]`: where document `d` begins in it.
    pub joined: Vec<u32>,
    starts: Vec<u32>,
}

/// A hasher for the rows (one multiply a word): SipHash's resistance to chosen keys buys nothing on a tape's rows.
#[derive(Default, Clone, Copy)]
pub struct Fx(u64);
impl std::hash::Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, b: &[u8]) {
        for &x in b {
            self.write_u64(u64::from(x));
        }
    }
    #[inline(always)]
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    #[inline(always)]
    fn write_u128(&mut self, x: u128) {
        self.write_u64(x as u64);
        self.write_u64((x >> 64) as u64);
    }
}
impl std::hash::BuildHasher for Fx {
    type Hasher = Fx;
    fn build_hasher(&self) -> Fx {
        Fx(0)
    }
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
    /// The language's tapes, with no symbols yet.
    fn open(capture: &'a Capture, tag: u8, cells: Cells, rank: HashMap<u128, u32, Fx>) -> Result<Self, String> {
        let lang = f16::lang(tag).ok_or_else(|| format!("tape tag {tag} has no format 16 table"))?;
        let mut docs = Self { capture, files: Vec::new(), placements: Vec::new(), views: Vec::new(), rank, cells, joined: Vec::new(), starts: Vec::new() };
        for (k, f) in capture.files.iter().enumerate().filter(|(_, f)| f.tag == tag) {
            let image = &capture.bytes[f.image.clone()];
            let p = f16::placement(image).map_err(|e| format!("{}: {e}", f.path))?;
            let v = p.view(lang, 0).map_err(|e| format!("{}: {e}", f.path))?;
            docs.files.push(k);
            docs.placements.push(p);
            docs.views.push(v);
        }
        Ok(docs)
    }

    /// Each document's rows read once, in tape order, ranked by `rank_of` into the symbol relation.
    fn derive(&mut self, mut rank_of: impl FnMut(&mut HashMap<u128, u32, Fx>, u128) -> u32) {
        let total: usize = self.placements.iter().map(|p| p.n + 1).sum();
        let mut joined = Vec::with_capacity(total);
        let mut starts = Vec::with_capacity(self.files.len());
        let mut rank = std::mem::take(&mut self.rank);
        for d in 0..self.files.len() {
            starts.push(joined.len() as u32);
            self.rows(d, |v| joined.push(rank_of(&mut rank, v) + 1));
            joined.push(0);
        }
        self.rank = rank;
        self.joined = joined;
        self.starts = starts;
    }

    pub fn new(capture: &'a Capture, tag: u8, cells: Cells) -> Result<Self, String> {
        let mut docs = Self::open(capture, tag, cells, HashMap::default())?;
        // Each row's first-seen id, then the ids renumbered in the rows' value order.
        docs.derive(|m, v| {
            let next = m.len() as u32;
            *m.entry(v).or_insert(next)
        });
        let mut values: Vec<(u128, u32)> = docs.rank.iter().map(|(&v, &r)| (v, r)).collect();
        values.sort_unstable();
        let mut to = vec![0u32; values.len()];
        for (r, &(_, first)) in values.iter().enumerate() {
            to[first as usize] = r as u32;
        }
        for x in docs.joined.iter_mut().filter(|x| **x != 0) {
            *x = to[*x as usize - 1] + 1;
        }
        docs.rank = values.into_iter().enumerate().map(|(r, (v, _))| (v, r as u32)).collect();
        Ok(docs)
    }

    /// A later batch's documents (new versions), ranked by `prev`'s dictionary and its own new rows after it, so that
    /// a row has one rank in every batch.
    pub fn extending(capture: &'a Capture, tag: u8, cells: Cells, prev: &NodeDocs) -> Result<Self, String> {
        let mut docs = Self::open(capture, tag, cells, prev.rank.clone())?;
        docs.derive(|m, v| {
            let next = m.len() as u32;
            *m.entry(v).or_insert(next)
        });
        Ok(docs)
    }

    /// The dictionary's rows in rank order, for a later run to rank by (`with_values`) without reading every row.
    pub fn values(&self) -> Vec<u128> {
        let mut v = vec![0u128; self.rank.len()];
        for (&x, &r) in &self.rank {
            v[r as usize] = x;
        }
        v
    }

    /// The documents ranked by a dictionary a run before kept (its rows in rank order).
    pub fn with_values(capture: &'a Capture, tag: u8, cells: Cells, values: &[u128]) -> Result<Self, String> {
        let rank = values.iter().enumerate().map(|(r, &v)| (v, r as u32)).collect();
        let mut docs = Self::open(capture, tag, cells, rank)?;
        docs.derive(|m, v| m[&v]);
        Ok(docs)
    }

    /// Document `d`'s rows in tape order (`value`'s), each read once: starts by the running sum of gaps
    /// (`start_next`, a checkpoint every K nodes), the line breaks by one scan of the source.
    fn rows(&self, d: usize, mut f: impl FnMut(u128)) {
        let n = self.placements[d].n;
        if !matches!(self.cells, Cells::Indent | Cells::Local) {
            (0..n).for_each(|j| f(self.value(d, j, self.cells)));
            return;
        }
        let (t, p, v, src) = (self.image(d), &self.placements[d], &self.views[d], self.source(d));
        let [w0, _, w2] = p.widths;
        let cell = |b: usize, r: usize, col: usize, w: usize| -> u128 {
            let at = p.column(b, col) + r * w;
            let mut x = 0u128;
            for (k, &byte) in t[at..at + w].iter().enumerate() {
                x |= u128::from(byte) << (8 * k);
            }
            x
        };
        let mut prev = 0u32;
        for j in 0..n {
            let (b, r, i) = (j / Z, j % Z, j as u32);
            let leaf = reads::slot(t, v, i) & 1 == 1;
            let len = if leaf { cell(b, r, 3, w2) } else { 0 };
            let s = reads::start_next(t, v, i, prev).min(v.source_len);
            let gap = if j == 0 {
                0
            } else {
                match src[prev as usize..s as usize].iter().position(|&c| c == b'\n') {
                    None => u128::from(s - prev),
                    Some(k) => 1 << 32 | (k as u128 + 1),
                }
            };
            prev = s;
            let code_slot = if self.cells == Cells::Local { cell(b, r, 0, 1) | u128::from(leaf) << 8 } else { cell(b, r, 0, 1) | cell(b, r, 1, w0) << 8 };
            f(code_slot | gap << 40 | len << 80);
        }
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
        self.joined[self.starts[d] as usize + j] - 1
    }
    fn joined(&self) -> Option<&[u32]> {
        Some(&self.joined)
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

/// The leaves `tape_lines` keys each counted line by, for one captured file: (line, [(kind name, token text)]).
pub fn tape_line_tokens(capture: &Capture, file: usize) -> Vec<(u32, Vec<(String, String)>)> {
    let f = &capture.files[file];
    let mut out: Vec<(u32, Vec<(String, String)>)> = Vec::new();
    let Some(lang) = f16::lang(f.tag) else { return out };
    let t = &capture.bytes[f.image.clone()];
    let src = &capture.bytes[f.source.clone()];
    let Ok(v) = f16::placement(t).and_then(|p| p.view(lang, 0)) else { return out };
    let name = |num: u32| {
        let kind = lang.table().entry(num).kind;
        lang.names().kind_name(kind).unwrap_or("?").to_owned()
    };
    let mut j = 0u32;
    while j < v.nodes {
        let num = reads::number(t, &v, j);
        if name(num).ends_with("comment") {
            j += reads::size(t, &v, j).max(1);
            continue;
        }
        if reads::slot(t, &v, j) & 1 == 1 {
            let (s, e) = (reads::start(t, &v, j), reads::end(t, &v, j));
            if e > s {
                let line = reads::line(t, &v, s) + 1;
                let tok = (name(num), String::from_utf8_lossy(&src[s as usize..e as usize]).into_owned());
                match out.last_mut() {
                    Some((l, toks)) if *l == line => toks.push(tok),
                    _ => out.push((line, vec![tok])),
                }
            }
        }
        j += 1;
    }
    out
}

/// Node `j`'s kind name and source line (from 1), for `explain`.
impl NodeDocs<'_> {
    pub fn kind_and_line(&self, d: usize, j: usize) -> (String, u32) {
        let f = &self.capture.files[self.files[d]];
        let lang = f16::lang(f.tag).expect("a table");
        let (t, v) = (self.image(d), &self.views[d]);
        let kind = lang.table().entry(reads::number(t, v, j as u32)).kind;
        let name = lang.names().kind_name(kind).unwrap_or("?").to_owned();
        (name, reads::line(t, v, reads::start(t, v, j as u32)) + 1)
    }
}
