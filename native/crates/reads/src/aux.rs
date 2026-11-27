//! An auxiliary index's reads, in place (decision 0656): its layout's numbers, its sections' kinds, a term's hash and
//! `AuxIndex`, the reads the tape's place and child reads take it by. The writer and the tape-walking reads are
//! `rerail_tape_format::aux_index`'s, which re-exports these.

/// The auxiliary index's first four bytes.
pub const MAGIC: u32 = u32::from_le_bytes(*b"RAX1");
/// The layout's version: the header's and the directory's.
pub const VERSION: u16 = 1;
/// No node, leaf or term.
pub const NONE: u32 = u32::MAX;
/// Nodes a LEAVES block covers, as format 16's blocks.
pub const BLOCK: u32 = 256;
/// Leaves an escapes run covers.
pub const CHUNK: u32 = 256;
/// The names byte that says the term is in the escapes: terms 0..255 are their own byte.
pub const ESCAPE: u8 = 255;
/// The header's bytes.
pub const HEADER: usize = 32;
/// A directory entry's bytes.
pub const ENTRY: usize = 16;
/// The siblings a SIBLINGS entry is between: every K-th child.
pub const K: u32 = 8;

/// A section's kind, its directory key. New kinds take new numbers; a number is never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Kind {
    Leaves = 1,
    Names = 2,
    Vocab = 3,
    Siblings = 4,
    Counts = 5,
    Rare = 6,
    Named = 7,
    Comments = 8,
}

impl Kind {
    pub const ALL: [Kind; 8] = [Kind::Leaves, Kind::Names, Kind::Vocab, Kind::Siblings, Kind::Counts, Kind::Rare, Kind::Named, Kind::Comments];
    /// The section's version, in its directory entry: a reader refuses a version it does not read. SIBLINGS' 2: of
    /// named children (1 was of every child). COUNTS' 2: with its strays (1 had none).
    pub const fn version(self) -> u16 {
        match self {
            Kind::Siblings | Kind::Counts => 2,
            _ => 1,
        }
    }
}

/// A text's hash, as the auxiliary index keys a term: eight bytes at a time (little-endian, the last zero-filled), each
/// folded in by a multiply, the length first and a finish after. `ukl_aux_text_hash` is the same in UKL.
pub fn text_hash(text: &[u8]) -> u64 {
    let mut h = 0x9E37_79B9_7F4A_7C15u64 ^ (text.len() as u64).wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    for c in text.chunks(8) {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 29;
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^ (h >> 33)
}

// The numbers the tape's CLIF read of a place (`tape_place`) reads the index by.
const _: () = assert!(
    crate::view::aux::SIBLINGS == Kind::Siblings as u16
        && crate::view::aux::NAMED == Kind::Named as u16
        && crate::view::aux::K == K
        && crate::view::aux::SIBLINGS_VERSION == Kind::Siblings.version()
        && crate::view::aux::NAMED_VERSION == Kind::Named.version()
        && crate::view::aux::COMMENTS == Kind::Comments as u16
        && crate::view::aux::COMMENTS_VERSION == Kind::Comments.version()
);

// ---- the reads -------------------------------------------------------------------------------------------

/// An auxiliary index read in place.
#[derive(Clone, Copy)]
pub struct AuxIndex<'a> {
    b: &'a [u8],
    /// Each kind's section offset (0: none), by kind.
    at: [u32; 9],
}

impl<'a> AuxIndex<'a> {
    /// The auxiliary index `b` holds, its header and directory checked.
    pub fn read(b: &'a [u8]) -> Result<Self, String> {
        if b.len() < HEADER || u32_at(b, 0) != MAGIC {
            return Err("no auxiliary index".into());
        }
        if u16_at(b, 4) != VERSION {
            return Err(format!("an auxiliary index of version {}, not {VERSION}", u16_at(b, 4)));
        }
        let count = u16_at(b, 6) as usize;
        if b.len() < HEADER + ENTRY * count {
            return Err("an auxiliary index's directory past its bytes".into());
        }
        let mut at = [0u32; 9];
        for e in 0..count {
            let d = HEADER + ENTRY * e;
            let (kind, version, off, len) = (u16_at(b, d), u16_at(b, d + 2), u32_at(b, d + 8), u32_at(b, d + 12));
            if !(off as usize).is_multiple_of(16) || off as usize + len as usize > b.len() || len < 16 {
                return Err(format!("section {kind}'s bytes {off}+{len} out of the auxiliary index's {}", b.len()));
            }
            match Kind::ALL.iter().find(|k| **k as u16 == kind) {
                Some(k) if k.version() == version => at[kind as usize] = off,
                Some(_) => return Err(format!("section {kind} of version {version}")),
                None => {}
            }
        }
        Ok(AuxIndex { b, at })
    }

    /// Its sections' offsets, by kind, as `read` found them.
    pub fn sections_at(&self) -> [u32; 9] {
        self.at
    }
    /// The index `b` holds whose sections lie at `at` (`sections_at` of a `read` of these very bytes): nothing read
    /// again.
    pub fn at(b: &'a [u8], at: [u32; 9]) -> Self {
        AuxIndex { b, at }
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.b
    }
    /// The bytes the index spans, as its writer lays it: its header and directory, and each section, each padded to
    /// 16. A home's index ends its home (`presence::find`).
    pub fn extent(&self) -> usize {
        let count = u16_at(self.b, 6) as usize;
        (0..count).fold((HEADER + ENTRY * count).next_multiple_of(16), |end, e| {
            let d = HEADER + ENTRY * e;
            end.max((u32_at(self.b, d + 8) as usize + u32_at(self.b, d + 12) as usize).next_multiple_of(16))
        })
    }
    pub fn nodes(&self) -> u32 {
        u32_at(self.b, 8)
    }
    pub fn leaves(&self) -> u32 {
        u32_at(self.b, 12)
    }
    pub fn terms(&self) -> u32 {
        u32_at(self.b, 20)
    }
    pub fn digest(&self) -> u64 {
        u64_at(self.b, 24)
    }
    pub fn has(&self, k: Kind) -> bool {
        self.at[k as usize] != 0
    }
    /// Word `w` of section `k`'s head.
    fn hw(&self, k: Kind, w: usize) -> u32 {
        u32_at(self.b, self.at[k as usize] as usize + 4 * w)
    }
    fn sec(&self, k: Kind) -> usize {
        self.at[k as usize] as usize
    }

    // LEAVES.

    /// Whether `node` is a leaf.
    pub fn is_leaf(&self, node: u32) -> bool {
        self.leaf_rank(node) != NONE
    }
    /// `node`'s rank among the leaves, or `NONE` (an inner node, or no LEAVES).
    pub fn leaf_rank(&self, node: u32) -> u32 {
        if !self.has(Kind::Leaves) || node >= self.nodes() {
            return NONE;
        }
        let s = self.sec(Kind::Leaves);
        let (b, bit) = ((node / BLOCK) as usize, node % BLOCK);
        let words = s + 16 + 32 * b;
        let w = u64_at(self.b, words + 8 * (bit / 64) as usize);
        if (w >> (bit % 64)) & 1 == 0 {
            return NONE;
        }
        let mut r = u32_at(self.b, s + self.hw(Kind::Leaves, 1) as usize + 4 * b);
        for k in 0..(bit / 64) as usize {
            r += u64_at(self.b, words + 8 * k).count_ones();
        }
        r + (w & ((1u64 << (bit % 64)) - 1)).count_ones()
    }
    /// The node of leaf `rank`, or `NONE`.
    pub fn leaf_node(&self, rank: u32) -> u32 {
        if !self.has(Kind::Leaves) || rank >= self.leaves() {
            return NONE;
        }
        let s = self.sec(Kind::Leaves);
        let (blocks, bases) = (self.hw(Kind::Leaves, 0), s + self.hw(Kind::Leaves, 1) as usize);
        // The last block whose leaves before it are at most `rank`.
        let (mut lo, mut hi) = (0u32, blocks);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if u32_at(self.b, bases + 4 * mid as usize) <= rank { lo = mid } else { hi = mid }
        }
        let mut r = rank - u32_at(self.b, bases + 4 * lo as usize);
        for k in 0..4u32 {
            let w = u64_at(self.b, s + 16 + 32 * lo as usize + 8 * k as usize);
            let c = w.count_ones();
            if r < c {
                return lo * BLOCK + 64 * k + select64(w, r);
            }
            r -= c;
        }
        NONE
    }

    // NAMES and VOCAB.

    /// Leaf `leaf`'s term, or `NONE`.
    pub fn term_of_leaf(&self, leaf: u32) -> u32 {
        if !self.has(Kind::Names) || leaf >= self.leaves() {
            return NONE;
        }
        let s = self.sec(Kind::Names);
        let x = self.b[s + 16 + leaf as usize];
        if x != ESCAPE {
            return u32::from(x);
        }
        let (bases, escapes) = (s + self.hw(Kind::Names, 1) as usize, s + self.hw(Kind::Names, 2) as usize);
        let c = (leaf / CHUNK) as usize;
        let (mut lo, mut hi) = (u32_at(self.b, bases + 4 * c), u32_at(self.b, bases + 4 * c + 4));
        while lo < hi {
            let mid = (lo + hi) / 2;
            let l = u32_at(self.b, escapes + 8 * mid as usize);
            if l == leaf {
                return u32_at(self.b, escapes + 8 * mid as usize + 4);
            }
            if l < leaf { lo = mid + 1 } else { hi = mid }
        }
        NONE
    }
    /// `node`'s term (a leaf's), or `NONE`.
    pub fn term_of_node(&self, node: u32) -> u32 {
        match self.leaf_rank(node) {
            NONE => NONE,
            l => self.term_of_leaf(l),
        }
    }
    /// Term `term`'s hash, and its first occurrence's start and length in the source.
    pub fn term(&self, term: u32) -> Option<(u64, u32, u32)> {
        if !self.has(Kind::Vocab) || term >= self.terms() {
            return None;
        }
        let e = self.sec(Kind::Vocab) + self.hw(Kind::Vocab, 1) as usize + 16 * term as usize;
        Some((u64_at(self.b, e), u32_at(self.b, e + 8), u32_at(self.b, e + 12)))
    }
    /// Term `term`'s text, in `source`.
    pub fn term_text<'s>(&self, term: u32, source: &'s [u8]) -> Option<&'s [u8]> {
        self.term(term).and_then(|(_, s, l)| source.get(s as usize..(s + l) as usize))
    }
    /// The first position, in hash order, whose term's hash is at least `h`.
    pub fn hash_lower_bound(&self, h: u64) -> u32 {
        let (s, n) = (self.sec(Kind::Vocab), self.terms());
        let by = s + self.hw(Kind::Vocab, 2) as usize;
        let (mut lo, mut hi) = (0u32, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.term(u32_at(self.b, by + 4 * mid as usize)).map_or(u64::MAX, |t| t.0) < h { lo = mid + 1 } else { hi = mid }
        }
        lo
    }
    /// The term at position `k` in hash order.
    pub fn by_hash(&self, k: u32) -> u32 {
        if !self.has(Kind::Vocab) || k >= self.terms() {
            return NONE;
        }
        u32_at(self.b, self.sec(Kind::Vocab) + self.hw(Kind::Vocab, 2) as usize + 4 * k as usize)
    }
    /// The first term (in hash order) whose hash is `h`, or `NONE`: a lookup by a string's hash.
    pub fn term_by_hash(&self, h: u64) -> u32 {
        if !self.has(Kind::Vocab) {
            return NONE;
        }
        let k = self.hash_lower_bound(h);
        match self.by_hash(k) {
            NONE => NONE,
            t if self.term(t).is_some_and(|x| x.0 == h) => t,
            _ => NONE,
        }
    }
    /// The term whose text is `text`, or `NONE` (a document without it): the hash, then the text compared in `source`.
    pub fn term_of_text(&self, text: &[u8], source: &[u8]) -> u32 {
        if !self.has(Kind::Vocab) {
            return NONE;
        }
        let h = text_hash(text);
        let mut k = self.hash_lower_bound(h);
        while k < self.terms() {
            let t = self.by_hash(k);
            if self.term(t).is_none_or(|x| x.0 != h) {
                break;
            }
            if self.term_text(t, source) == Some(text) {
                return t;
            }
            k += 1;
        }
        NONE
    }
    /// The leaves whose term is `term`, in order.
    pub fn leaves_with_term(&self, term: u32) -> impl Iterator<Item = u32> + '_ {
        let names: &[u8] = if self.has(Kind::Names) { &self.b[self.sec(Kind::Names) + 16..][..self.leaves() as usize] } else { &[] };
        let byte = term.min(u32::from(ESCAPE)) as u8;
        memchr::memchr_iter(byte, names).map(|l| l as u32).filter(move |&l| term < u32::from(ESCAPE) || self.term_of_leaf(l) == term)
    }
    /// The nodes whose text is term `term`'s, in preorder.
    pub fn nodes_with_term(&self, term: u32) -> impl Iterator<Item = u32> + '_ {
        self.leaves_with_term(term).map(|l| self.leaf_node(l))
    }

    // NAMED and SIBLINGS.

    /// Whether code `code` is named (false where it is past the table, or no NAMED).
    pub fn named(&self, code: u32) -> bool {
        if !self.has(Kind::Named) || code / 64 >= self.hw(Kind::Named, 0) {
            return false;
        }
        (u64_at(self.b, self.sec(Kind::Named) + 16 + 8 * (code / 64) as usize) >> (code % 64)) & 1 != 0
    }

    /// The node of `parent`'s named child `K * k` (k ≥ 1), or `NONE`.
    pub fn eighth(&self, parent: u32, k: u32) -> u32 {
        if !self.has(Kind::Siblings) || k == 0 {
            return NONE;
        }
        let first = self.family(parent);
        let at = first + k - 1;
        if at < self.hw(Kind::Siblings, 0) && self.pair(at).0 == parent { self.pair(at).1 } else { NONE }
    }
    /// The last of `parent`'s every-K-th named children at or before `node`: `(k, its node)`, or `(0, NONE)` where none
    /// is (`node` is before the K-th, or no SIBLINGS).
    pub fn eighth_at_or_before(&self, parent: u32, node: u32) -> (u32, u32) {
        match self.entry_at_or_before(parent, node) {
            None => (0, NONE),
            Some((k, e)) => (k, self.pair(e).1),
        }
    }
    /// The SIBLINGS entry of the last of `parent`'s every-K-th named children at or before `node`: `(k, the entry)`.
    fn entry_at_or_before(&self, parent: u32, node: u32) -> Option<(u32, u32)> {
        if !self.has(Kind::Siblings) {
            return None;
        }
        let first = self.family(parent);
        let (mut lo, mut hi) = (first, self.family(parent.saturating_add(1)));
        // The last entry of the family whose node is at most `node`.
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.pair(mid).1 <= node { lo = mid + 1 } else { hi = mid }
        }
        (lo != first).then(|| (lo - first, lo - 1))
    }
    /// The first entry whose parent is at least `parent`.
    fn family(&self, parent: u32) -> u32 {
        let (mut lo, mut hi) = (0u32, self.hw(Kind::Siblings, 0));
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.pair(mid).0 < parent { lo = mid + 1 } else { hi = mid }
        }
        lo
    }
    fn pair(&self, i: u32) -> (u32, u32) {
        let at = self.sec(Kind::Siblings) + 16 + 8 * i as usize;
        (u32_at(self.b, at), u32_at(self.b, at + 4))
    }

    // COMMENTS.

    /// Whether code `code` is a comment (false where it is past the table, or no COMMENTS).
    pub fn comment(&self, code: u32) -> bool {
        if !self.has(Kind::Comments) || code / 64 >= self.hw(Kind::Comments, 0) {
            return false;
        }
        (u64_at(self.b, self.sec(Kind::Comments) + 16 + 8 * (code / 64) as usize) >> (code % 64)) & 1 != 0
    }
    /// The comments among `parent`'s named children before its every-K-th named child at or before `node`: `(k, its
    /// node, the comments before it)`, or `None` where none is (`node` before the K-th, or no COMMENTS).
    pub fn eighth_past_comments(&self, parent: u32, node: u32) -> Option<(u32, u32, u32)> {
        if !self.has(Kind::Comments) {
            return None;
        }
        let (k, e) = self.entry_at_or_before(parent, node)?;
        let at = self.sec(Kind::Comments) + self.hw(Kind::Comments, 2) as usize + 4 * e as usize;
        Some((k, self.pair(e).1, u32_at(self.b, at)))
    }

    // COUNTS and RARE.

    /// The nodes of code `code`, or `NONE` (no COUNTS).
    pub fn count(&self, code: u32) -> u32 {
        if !self.has(Kind::Counts) {
            return NONE;
        }
        let (s, n) = (self.sec(Kind::Counts) + 16, self.hw(Kind::Counts, 0));
        let (mut lo, mut hi) = (0u32, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let c = u32_at(self.b, s + 8 * mid as usize);
            if c == code {
                return u32_at(self.b, s + 8 * mid as usize + 4);
            }
            if c < code { lo = mid + 1 } else { hi = mid }
        }
        0
    }
    /// How many named nodes hang from a parent that is not named, or `NONE` (no COUNTS).
    pub fn strays(&self) -> u32 {
        if !self.has(Kind::Counts) {
            return NONE;
        }
        self.hw(Kind::Counts, 1)
    }
    /// Each code present and its count, by code.
    pub fn counts(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        (0..self.distinct()).map(move |k| self.count_at(k))
    }
    /// How many codes are present (0 without COUNTS).
    pub fn distinct(&self) -> u32 {
        if self.has(Kind::Counts) { self.hw(Kind::Counts, 0) } else { 0 }
    }
    /// The `k`-th code present (by code) and its count; `k` below `distinct`.
    pub fn count_at(&self, k: u32) -> (u32, u32) {
        let at = self.sec(Kind::Counts) + 16 + 8 * k as usize;
        (u32_at(self.b, at), u32_at(self.b, at + 4))
    }
    /// The positions in RARE's nodes of code `code`'s, or `None` where the code is not rare (or no RARE): a code
    /// absent from the document is rare, with no nodes.
    pub fn rare_range(&self, code: u32) -> Option<(u32, u32)> {
        if !self.has(Kind::Rare) {
            return None;
        }
        let s = self.sec(Kind::Rare);
        let n = self.hw(Kind::Rare, 0);
        let (mut lo, mut hi) = (0u32, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let c = u32_at(self.b, s + 16 + 8 * mid as usize);
            if c == code {
                return Some((u32_at(self.b, s + 20 + 8 * mid as usize), u32_at(self.b, s + 28 + 8 * mid as usize)));
            }
            if c < code { lo = mid + 1 } else { hi = mid }
        }
        if self.has(Kind::Counts) && self.count(code) > 0 { None } else { Some((0, 0)) }
    }
    /// RARE's node at position `k`.
    pub fn rare_node(&self, k: u32) -> u32 {
        let s = self.sec(Kind::Rare);
        u32_at(self.b, s + self.hw(Kind::Rare, 2) as usize + 4 * k as usize)
    }
    /// Code `code`'s nodes, in preorder, where it is rare.
    pub fn rare(&self, code: u32) -> Option<impl Iterator<Item = u32> + '_> {
        self.rare_range(code).map(|(a, b)| (a..b).map(|k| self.rare_node(k)))
    }
}

// ---- reads of the tape and the auxiliary index together ---------------------------------------------------------

fn select64(mut w: u64, r: u32) -> u32 {
    for _ in 0..r {
        w &= w - 1;
    }
    w.trailing_zeros()
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().expect("two bytes"))
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("four bytes"))
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("eight bytes"))
}
