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
    /// The file table as bytes (`store` arrays): the paths joined by newlines, the tags, and each file's source and
    /// image ranges, so a later run takes the capture's files without walking it.
    pub fn write_table(&self, out: &mut Vec<u8>) {
        let paths: Vec<&str> = self.files.iter().map(|f| f.path.as_str()).collect();
        crate::store::put(out, paths.join("\n").as_bytes());
        crate::store::put(out, &self.files.iter().map(|f| f.tag).collect::<Vec<u8>>());
        let r: Vec<u64> = self.files.iter().flat_map(|f| [f.source.start, f.source.end, f.image.start, f.image.end]).map(|x| x as u64).collect();
        crate::store::put(out, &r);
    }

    /// The capture at `path`, mapped, with the file table `write_table` wrote at `*at` in `b`.
    pub fn with_table(path: &std::path::Path, b: &'static [u8], at: &mut usize) -> Result<Self, String> {
        let bytes = crate::store::map(path)?;
        let paths = crate::store::view::<u8>(b, at);
        let tags = crate::store::view::<u8>(b, at);
        let r = crate::store::view::<u64>(b, at);
        let paths = std::str::from_utf8(paths).map_err(|e| e.to_string())?;
        let files = paths
            .split('\n')
            .zip(tags)
            .zip(r.chunks(4))
            .map(|((p, &tag), r)| CapturedFile { path: p.to_owned(), tag, source: r[0] as usize..r[1] as usize, image: r[2] as usize..r[3] as usize })
            .collect();
        Ok(Self { bytes, files })
    }

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
    /// Each tape's placement and view, read from its header when the tape is first read (a run on a saved index
    /// reads only the tapes it uses).
    tapes: Vec<std::cell::OnceCell<(Placement, TapeView)>>,
    tag: u8,
    /// The last document's inner nodes' trailing distances, in close order (`ClosesTrail`, for `dumpcols`).
    pub trailers: std::cell::RefCell<Vec<u32>>,
    rank: Dict,
    cells: Cells,
    /// The symbol relation: each node's symbol (its row's rank), derived once from the tape in tape order and kept
    /// here as the one place the symbols live; laid out as the joined text the suffix array sorts (rank + 1 per
    /// node, 0 after each document). `starts[d]`: where document `d` begins in it.
    pub joined: crate::store::Arr<u32>,
    pub starts: crate::store::Arr<u32>,
}

/// The dictionary of rows: the base's distinct rows sorted (their rank is their place), owned or mapped from a saved
/// index as two halves, and the rows later batches added, by first seen. A lookup is a binary search of the base, so a
/// run on a saved index reads only the entries it looks up.
#[derive(Clone, Default)]
pub struct Dict {
    base: DictBase,
    added: HashMap<u128, u32, Fx>,
}

#[derive(Clone)]
enum DictBase {
    Owned(std::sync::Arc<Vec<u128>>),
    Mapped(&'static [u64], &'static [u64]),
}

impl Default for DictBase {
    fn default() -> Self {
        DictBase::Owned(Default::default())
    }
}

impl Dict {
    pub fn mapped(lo: &'static [u64], hi: &'static [u64]) -> Self {
        Self { base: DictBase::Mapped(lo, hi), added: HashMap::default() }
    }
    fn base_len(&self) -> usize {
        match &self.base {
            DictBase::Owned(v) => v.len(),
            DictBase::Mapped(lo, _) => lo.len(),
        }
    }
    fn base_at(&self, i: usize) -> u128 {
        match &self.base {
            DictBase::Owned(v) => v[i],
            DictBase::Mapped(lo, hi) => u128::from(lo[i]) | u128::from(hi[i]) << 64,
        }
    }
    pub fn len(&self) -> usize {
        self.base_len() + self.added.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[inline]
    pub fn get(&self, v: u128) -> Option<u32> {
        let (mut lo, mut hi) = (0, self.base_len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.base_at(mid).cmp(&v) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid as u32),
            }
        }
        self.added.get(&v).copied()
    }
    /// The row's rank, a new one (the next) if it is not in the dictionary.
    pub fn id(&mut self, v: u128) -> u32 {
        if let Some(r) = self.get(v) {
            return r;
        }
        let next = self.len() as u32;
        self.added.insert(v, next);
        next
    }
    /// The rows in rank order.
    pub fn values(&self) -> Vec<u128> {
        let mut out: Vec<u128> = (0..self.base_len()).map(|i| self.base_at(i)).collect();
        out.resize(self.len(), 0);
        for (&v, &r) in &self.added {
            out[r as usize] = v;
        }
        out
    }
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
    /// The stored row's values with escapes resolved: today's format, like for like (derived in tape order only).
    Decoded,
    /// `Decoded`, as if a leaf's slot always held its next node's previous sibling's size, never the parent's
    /// distance (the every-8th-sibling parent step moved out of the row).
    Sibling,
    /// `Sibling` with `Indent`'s gap and lengths.
    SiblingIndent,
    /// `Local`, plus each node's count of subtrees that close right after it: the window's exact shape, read from the
    /// sizes in order (balanced parentheses' closes), not stored.
    Shape,
    /// `Shape`'s structure (leaf bit and closes) with the stored gaps, and a leaf's length only: the format with sizes
    /// replaced by closes, indentation kept.
    Closes,
    /// `Closes`, with each inner node's end kept where it closes: for every subtree closing after a node, its end less
    /// its last node's end (a block's `}` past its last named node), hashed into the node's symbol.
    ClosesTrail,
}

impl<'a> NodeDocs<'a> {
    /// The language's tapes, with no symbols yet.
    fn open(capture: &'a Capture, tag: u8, cells: Cells, rank: Dict) -> Result<Self, String> {
        f16::lang(tag).ok_or_else(|| format!("tape tag {tag} has no format 16 table"))?;
        let files: Vec<usize> = capture.files.iter().enumerate().filter(|(_, f)| f.tag == tag).map(|(k, _)| k).collect();
        let tapes = files.iter().map(|_| std::cell::OnceCell::new()).collect();
        Ok(Self { capture, files, tapes, tag, trailers: Default::default(), rank, cells, joined: Vec::new().into(), starts: Vec::new().into() })
    }

    /// The language's tapes, with no dictionary and no symbols: for reading rows in order (`each_row`).
    pub fn bare(capture: &'a Capture, tag: u8, cells: Cells) -> Result<Self, String> {
        Self::open(capture, tag, cells, Dict::default())
    }

    /// Document `d`'s rows in tape order, as this variant packs them.
    pub fn each_row(&self, d: usize, f: impl FnMut(u128)) {
        self.rows(d, f)
    }

    /// Document `d`'s tape's placement and view.
    pub fn tape(&self, d: usize) -> &(Placement, TapeView) {
        self.tapes[d].get_or_init(|| {
            let f = &self.capture.files[self.files[d]];
            let p = f16::placement(self.image(d)).unwrap_or_else(|e| panic!("{}: {e}", f.path));
            let v = p.view(f16::lang(self.tag).unwrap(), 0).unwrap_or_else(|e| panic!("{}: {e}", f.path));
            (p, v)
        })
    }

    /// Each document's rows read once, in tape order, ranked by `rank_of` into the symbol relation.
    fn derive(&mut self, mut rank_of: impl FnMut(&mut Dict, u128) -> u32) {
        let total: usize = (0..self.files.len()).map(|d| self.tape(d).0.n + 1).sum();
        let mut joined = Vec::with_capacity(total);
        let mut starts = Vec::with_capacity(self.files.len());
        let mut rank = std::mem::take(&mut self.rank);
        for d in 0..self.files.len() {
            starts.push(joined.len() as u32);
            self.rows(d, |v| joined.push(rank_of(&mut rank, v) + 1));
            joined.push(0);
        }
        self.rank = rank;
        self.joined = joined.into();
        self.starts = starts.into();
    }

    pub fn new(capture: &'a Capture, tag: u8, cells: Cells) -> Result<Self, String> {
        let mut docs = Self::open(capture, tag, cells, Dict::default())?;
        // Each row's first-seen id (a hash map while the rows are found), then the ids renumbered in the rows' value
        // order, and the rows kept sorted as the dictionary's base.
        docs.derive(|m, v| {
            let next = m.added.len() as u32;
            *m.added.entry(v).or_insert(next)
        });
        let mut values: Vec<(u128, u32)> = docs.rank.added.drain().collect();
        values.sort_unstable();
        let mut to = vec![0u32; values.len()];
        for (r, &(_, first)) in values.iter().enumerate() {
            to[first as usize] = r as u32;
        }
        let crate::store::Arr::Owned(joined) = &mut docs.joined else { unreachable!("a relation just derived") };
        for x in joined.iter_mut().filter(|x| **x != 0) {
            *x = to[*x as usize - 1] + 1;
        }
        docs.rank = Dict { base: DictBase::Owned(std::sync::Arc::new(values.into_iter().map(|(v, _)| v).collect())), added: HashMap::default() };
        Ok(docs)
    }

    /// A later batch's documents (new versions), ranked by `prev`'s dictionary and its own new rows after it, so that
    /// a row has one rank in every batch.
    pub fn extending(capture: &'a Capture, tag: u8, cells: Cells, prev: &NodeDocs) -> Result<Self, String> {
        let mut docs = Self::open(capture, tag, cells, prev.rank.clone())?;
        docs.derive(Dict::id);
        Ok(docs)
    }

    /// The dictionary's rows in rank order, for a later run to rank by (`with_dict`) without reading every row.
    pub fn values(&self) -> Vec<u128> {
        self.rank.values()
    }

    /// The documents as a run before kept them: its dictionary (`Dict::mapped`) and its symbol relation and document
    /// starts, read in place.
    pub fn with_dict(capture: &'a Capture, tag: u8, cells: Cells, rank: Dict, joined: &'static [u32], starts: &'static [u32]) -> Result<Self, String> {
        let mut docs = Self::open(capture, tag, cells, rank)?;
        assert_eq!(starts.len(), docs.files.len(), "the saved relation is of these documents");
        docs.joined = crate::store::Arr::Mapped(joined);
        docs.starts = crate::store::Arr::Mapped(starts);
        Ok(docs)
    }

    /// The at-rest structure check: each document encoded as closes (subtrees ending after each node), the length of
    /// every one-node subtree, an inner node's end as its distance past its last node's end, and the gaps; then
    /// decoded by one pass with a stack into format 16's slots and lengths, compared with the tape's. Returns
    /// (nodes, slot mismatches, length mismatches, decode nanoseconds).
    pub fn rest_check(&self) -> (u64, u64, u64, u128) {
        let (mut nodes, mut bad_slot, mut bad_len, mut ns) = (0u64, 0u64, 0u64, 0u128);
        for d in 0..self.files.len() {
            let (t, v) = (self.image(d), &self.tape(d).1);
            let n = self.tape(d).0.n;
            // Encode, from the tape's reads.
            let size: Vec<u32> = (0..n as u32).map(|i| reads::size(t, v, i)).collect();
            let mut prev = 0u32;
            let starts: Vec<u32> = (0..n as u32)
                .map(|i| {
                    prev = reads::start_next(t, v, i, prev);
                    prev
                })
                .collect();
            let gaps: Vec<u32> = (0..n).map(|i| if i == 0 { starts[0] } else { starts[i] - starts[i - 1] }).collect();
            let (mut closes, mut len1, mut trail) = (vec![0u8; n], Vec::new(), Vec::new());
            let mut stack: Vec<(u32, u32)> = Vec::new();
            for k in 0..n {
                stack.push((k as u32, k as u32 + size[k]));
                if size[k] == 1 {
                    len1.push(reads::length(t, v, k as u32));
                }
                let last_end = reads::end(t, v, k as u32);
                while let Some(&(j, e)) = stack.last() {
                    if e as usize > k + 1 {
                        break;
                    }
                    stack.pop();
                    closes[k] += 1;
                    if size[j as usize] > 1 {
                        trail.push(reads::end(t, v, j) - last_end);
                    }
                }
            }
            // Decode: format 16's slots and lengths from the at-rest arrays alone.
            let t0 = std::time::Instant::now();
            let (mut slots, mut lens) = (vec![0u64; n], vec![0u32; n]);
            // Frames: (node, its start, children so far, last child).
            let mut open: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(64);
            let (mut start, mut li, mut ti) = (0u32, 0usize, 0usize);
            for k in 0..n {
                let i = k as u32;
                start = start.wrapping_add(gaps[k]);
                if let Some(f) = open.last_mut() {
                    let (p, idx, last) = (f.0, f.2, f.3);
                    f.2 += 1;
                    f.3 = i;
                    if i - 1 != p {
                        slots[k - 1] = if idx % f16::K as u32 == 0 { u64::from(i - p) << 2 | 3 } else { u64::from(i - last) << 2 | 1 };
                    }
                }
                let c = closes[k];
                if c == 0 {
                    open.push((i, start, 0, 0));
                    continue;
                }
                // Node k is a one-node subtree; the c - 1 frames above it close too.
                let end = start + len1[li];
                li += 1;
                lens[k] = end - start;
                for _ in 1..c {
                    let (j, js, ..) = open.pop().unwrap();
                    slots[j as usize] = u64::from(i + 1 - j) << 1;
                    lens[j as usize] = end + trail[ti] - js;
                    ti += 1;
                }
            }
            ns += t0.elapsed().as_nanos();
            nodes += n as u64;
            for k in 0..n {
                let i = k as u32;
                // A one-node inner subtree's slot is its size (2); a leaf's is set by the node after it.
                let want = reads::slot(t, v, i);
                let got = if size[k] == 1 && want & 1 == 0 { want } else { slots[k] };
                bad_slot += u64::from(got != want && k + 1 != n);
                bad_len += u64::from(lens[k] != reads::length(t, v, i));
            }
        }
        (nodes, bad_slot, bad_len, ns)
    }

    /// The variants derived from the tree in order: sizes, closes and previous siblings by one pass with a stack.
    fn rows_derived(&self, d: usize, mut f: impl FnMut(u128)) {
        let (t, v, src) = (self.image(d), &self.tape(d).1, self.source(d));
        let n = self.tape(d).0.n;
        let size: Vec<u32> = (0..n as u32).map(|i| reads::size(t, v, i)).collect();
        // closes[k]: subtrees ending right after node k; next_prev[k]: the size of node k + 1's previous sibling.
        let (mut closes, mut next_prev) = (vec![0u32; n], vec![0u32; n]);
        // trail[k]: a hash of the trailing distances of the inner subtrees closing right after node k, innermost first.
        let mut trail = vec![0u64; n];
        self.trailers.borrow_mut().clear();
        let mut stack: Vec<(u32, u32)> = Vec::new();
        for k in 0..=n {
            let (mut c, mut outer) = (0, None);
            while let Some(&(j, e)) = stack.last() {
                if e as usize > k {
                    break;
                }
                stack.pop();
                c += 1;
                outer = Some(j);
                if size[j as usize] > 1 {
                    let t_end = reads::end(t, v, j);
                    let last_end = reads::end(t, v, k as u32 - 1);
                    let x = t_end.saturating_sub(last_end);
                    self.trailers.borrow_mut().push(x);
                    trail[k - 1] = (trail[k - 1] ^ u64::from(x)).wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(17);
                }
            }
            if k > 0 {
                closes[k - 1] = c;
                if let Some(j) = outer {
                    next_prev[k - 1] = k as u32 - j;
                }
            }
            if k < n {
                stack.push((k as u32, k as u32 + size[k]));
            }
        }
        let indent = matches!(self.cells, Cells::SiblingIndent | Cells::Shape);
        let mut prev = 0u32;
        for j in 0..n {
            let i = j as u32;
            let raw = reads::slot(t, v, i);
            let leaf = raw & 1 == 1;
            let code = u128::from(reads::number(t, v, i));
            let s = reads::start_next(t, v, i, prev).min(v.source_len);
            let length = u128::from(reads::length(t, v, i));
            let slot: u128 = match self.cells {
                Cells::Decoded => u128::from(raw),
                Cells::Sibling | Cells::SiblingIndent if leaf => u128::from(next_prev[j]) << 2 | 1,
                Cells::Sibling | Cells::SiblingIndent => u128::from(raw),
                // Closes below 2^11; the trailing distances' hash in the field's 22 bits above them.
                Cells::ClosesTrail => u128::from(leaf) | u128::from(closes[j].min(2047)) << 1 | u128::from(trail[j] & 0x3f_ffff) << 12,
                _ => u128::from(leaf) | u128::from(closes[j]) << 1,
            };
            let closes_form = matches!(self.cells, Cells::Closes | Cells::ClosesTrail);
            let (gap, len) = if !indent {
                (u128::from(s - prev), if closes_form && !leaf { 0 } else { length })
            } else {
                let gap = if j == 0 {
                    0
                } else {
                    match src[prev as usize..s as usize].iter().position(|&c| c == b'\n') {
                        None => u128::from(s - prev),
                        Some(k) => 1 << 32 | (k as u128 + 1),
                    }
                };
                (gap, if leaf { length } else { 0 })
            };
            prev = s;
            f(code | slot << 16 | gap << 50 | len << 84);
        }
    }

    /// Document `d`'s rows in tape order (`value`'s), each read once: starts by the running sum of gaps
    /// (`start_next`, a checkpoint every K nodes), the line breaks by one scan of the source.
    fn rows(&self, d: usize, mut f: impl FnMut(u128)) {
        let n = self.tape(d).0.n;
        if matches!(self.cells, Cells::Decoded | Cells::Sibling | Cells::SiblingIndent | Cells::Shape | Cells::Closes | Cells::ClosesTrail) {
            return self.rows_derived(d, f);
        }
        if !matches!(self.cells, Cells::Indent | Cells::Local) {
            (0..n).for_each(|j| f(self.value(d, j, self.cells)));
            return;
        }
        let (t, p, v, src) = (self.image(d), &self.tape(d).0, &self.tape(d).1, self.source(d));
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
        let (t, p) = (self.image(d), &self.tape(d).0);
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
            Cells::Decoded | Cells::Sibling | Cells::SiblingIndent | Cells::Shape | Cells::Closes | Cells::ClosesTrail => {
                unreachable!("a derived variant is read in tape order (rows_derived), never at a point")
            }
            Cells::CodeSlot => code_slot,
            Cells::Row => code_slot | cell(2, w1) << 40 | cell(3, w2) << 72,
            Cells::Indent | Cells::Local => {
                let (v, i) = (&self.tape(d).1, j as u32);
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
        reads::line(self.image(d), &self.tape(d).1, x) + 1
    }

    /// The source span of nodes `first..end` of document `d`: from the first's start to the furthest end of a node
    /// whose subtree lies in the range (a node whose subtree runs past the range is cut by it).
    pub fn span(&self, d: usize, first: usize, end: usize) -> (u32, u32) {
        let (t, v) = (self.image(d), &self.tape(d).1);
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
        if self.starts.is_empty() {
            return self.tape(d).0.n;
        }
        let end = self.starts.get(d + 1).map_or(self.joined.len(), |&e| e as usize);
        end - self.starts[d] as usize - 1
    }
    #[inline(always)]
    fn at(&self, d: usize, j: usize) -> u32 {
        if self.joined.is_empty() {
            return self.rank.get(self.value(d, j, self.cells)).expect("a row in the dictionary");
        }
        self.joined[self.starts[d] as usize + j] - 1
    }
    fn joined(&self) -> Option<&[u32]> {
        (!self.joined.is_empty()).then_some(&self.joined[..])
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
        let (t, v) = (self.image(d), &self.tape(d).1);
        let kind = lang.table().entry(reads::number(t, v, j as u32)).kind;
        let name = lang.names().kind_name(kind).unwrap_or("?").to_owned();
        (name, reads::line(t, v, reads::start(t, v, j as u32)) + 1)
    }
}
