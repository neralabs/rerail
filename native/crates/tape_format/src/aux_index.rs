//! A tape's auxiliary index (decision 0656): what the format does not hold yet, made once from a tape and its source, kept
//! beside the tape as its presence is, and read in place: by these reads in Rust, and by the same reads in UKL
//! (`<ukl/reads/aux_index.h>`, `<ukl/reads/aux_index_tape.h>`) on every device and on the CPU's vector path. A test
//! (`rerail-ukl-vec`'s `aux_index_tests`) holds every UKL read equal to these on every node, leaf, term and code.
//!
//! The auxiliary index is generic: each section covers its whole document, whatever the rules are. The rules choose which
//! sections a run needs (`Sections`), never which keys a section holds. Its sections:
//!
//! - **LEAVES**: which nodes are leaves, a bitmap per 256-node block, with the leaves before each block: a node's
//!   leaf rank and a rank's node.
//! - **NAMES**: one byte per leaf, its text's term in the document, the most frequent first (0156 §6's names level).
//!   255 is a prefix: the term is in the escapes, by leaf, a run for each 256 leaves.
//! - **VOCAB**: each term's 64-bit hash (`text_hash`) and where its first occurrence lies in the source (no text is
//!   copied), and the terms in hash order, for a lookup by a string.
//! - **SIBLINGS**: every 8th named child of a family (its 8th, 16th, ...), as (parent, node) in order: a child's
//!   place among its named siblings, and a parent's i-th named child, from one in at most 7 named steps of the tape
//!   (the tokens between stepped over). The places the engine reads are among named children.
//! - **COUNTS**: each code's count in the document, by code.
//! - **RARE**: the nodes of each rare code (at most one node in 256), by code, in preorder.
//! - **NAMED**: which of the language's codes are named (a bit a code), as the grammar says: the tape holds a code,
//!   not whether its node is named, and SIBLINGS and its reads count named children.
//!
//! The layout: a header, a directory of sections (kind, version, parameters, offset, length), then each section,
//! 16-byte aligned, starting with four u32s of its own. Every value lies at its own alignment; all are little-endian.
//! A reader finds a section by its kind, and an auxiliary index without it answers `None` (or `NONE`) for its reads; a newer
//! section's kind is skipped by a reader that does not know it. A section is a pure function of the tape, the source
//! and its version, so one can be added to an auxiliary index without the others made again.

use crate::tape::Tape;

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
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Leaves, Kind::Names, Kind::Vocab, Kind::Siblings, Kind::Counts, Kind::Rare, Kind::Named];
    /// The section's version, in its directory entry: a reader refuses a version it does not read. SIBLINGS' 2: of
    /// named children (1 was of every child).
    pub fn version(self) -> u16 {
        match self {
            Kind::Siblings => 2,
            _ => 1,
        }
    }
}

/// The sections an auxiliary index is made with. NAMES and VOCAB are made together (a name is a term's number), and both
/// need LEAVES (a leaf's rank); SIBLINGS needs NAMED (its reads step over the unnamed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sections(u32);

impl Sections {
    pub const NONE: Sections = Sections(0);
    pub const ALL: Sections = Sections((1 << 8) - 2);
    pub fn of(kinds: &[Kind]) -> Sections {
        Sections(kinds.iter().fold(0, |m, k| m | 1 << *k as u32))
    }
    pub fn has(self, k: Kind) -> bool {
        self.0 & (1 << k as u32) != 0
    }
    /// With what each section needs.
    fn closed(self) -> Sections {
        let mut s = self;
        if s.has(Kind::Names) || s.has(Kind::Vocab) {
            s = Sections(s.0 | Sections::of(&[Kind::Leaves, Kind::Names, Kind::Vocab]).0);
        }
        if s.has(Kind::Siblings) {
            s = Sections(s.0 | Sections::of(&[Kind::Named]).0);
        }
        s
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

// ---- the writer ------------------------------------------------------------------------------------------

/// The bytes of an auxiliary index of `t` (over `source`, the document's), with `want`'s sections (and those they need).
/// `digest` is the tape's content id, as the store keys it, kept in the header so an auxiliary index is matched to its tape.
/// `named`: the language's named codes, a bit a code (`named_codes`), which NAMED holds and SIBLINGS counts by.
pub fn build<T: Tape + ?Sized>(t: &T, source: &[u8], digest: u64, named: &[u64], want: Sections) -> Result<Vec<u8>, String> {
    let n = t.len();
    if n >= NONE as usize || source.len() >= NONE as usize {
        return Err(format!("a document of {n} nodes and {} bytes: past an auxiliary index's u32s", source.len()));
    }
    let want = want.closed();
    let blocks = n.div_ceil(BLOCK as usize);
    let leaf = |i: usize| t.subtree_end(i) == i + 1;
    let is_named = |i: usize| bit(named, t.code(i));
    let leaves: Vec<u32> = (0..n).filter(|&i| leaf(i)).map(|i| i as u32).collect();
    let mut sections: Vec<(Kind, u32, Vec<u8>)> = Vec::new();
    let mut terms_count = 0u32;
    if want.has(Kind::Leaves) {
        let mut s = head([blocks as u32, (16 + blocks * 32) as u32, 0, 0]);
        let mut bases = Vec::with_capacity(blocks + 1);
        let mut before = 0u32;
        for b in 0..blocks {
            bases.push(before);
            let mut words = [0u64; 4];
            for i in b * BLOCK as usize..((b + 1) * BLOCK as usize).min(n) {
                if leaf(i) {
                    words[(i % 256) / 64] |= 1 << (i % 64);
                    before += 1;
                }
            }
            words.iter().for_each(|w| s.extend_from_slice(&w.to_le_bytes()));
        }
        bases.push(before);
        bases.iter().for_each(|x| s.extend_from_slice(&x.to_le_bytes()));
        sections.push((Kind::Leaves, 0, s));
    }
    if want.has(Kind::Names) {
        // Each leaf's text, by its first occurrence; then numbered by count, the most frequent first (ties by first).
        let mut first: std::collections::HashMap<&[u8], u32> = std::collections::HashMap::new();
        let mut terms: Vec<(u32, u32, u32)> = Vec::new(); // (count, start, len) by first occurrence
        let mut of_leaf = Vec::with_capacity(leaves.len());
        for &i in &leaves {
            let (s, e) = t.span(i as usize);
            let (s, e) = (s as usize, e as usize);
            let text = source.get(s..e).ok_or_else(|| format!("leaf {i}'s span {s}..{e} past the source's {} bytes", source.len()))?;
            let k = *first.entry(text).or_insert_with(|| {
                terms.push((0, s as u32, (e - s) as u32));
                terms.len() as u32 - 1
            });
            terms[k as usize].0 += 1;
            of_leaf.push(k);
        }
        let mut order: Vec<u32> = (0..terms.len() as u32).collect();
        order.sort_by_key(|&k| (std::cmp::Reverse(terms[k as usize].0), k));
        let mut number = vec![0u32; terms.len()];
        order.iter().enumerate().for_each(|(r, &k)| number[k as usize] = r as u32);
        terms_count = terms.len() as u32;
        // NAMES.
        let chunks = leaves.len().div_ceil(CHUNK as usize);
        let bases_at = 16 + leaves.len().next_multiple_of(4);
        let escapes_at = bases_at + 4 * (chunks + 1);
        let escaped: Vec<(u32, u32)> =
            of_leaf.iter().enumerate().map(|(l, &k)| (l as u32, number[k as usize])).filter(|&(_, x)| x >= u32::from(ESCAPE)).collect();
        let mut s = head([chunks as u32, bases_at as u32, escapes_at as u32, escaped.len() as u32]);
        s.extend(of_leaf.iter().map(|&k| number[k as usize].min(u32::from(ESCAPE)) as u8));
        s.resize(bases_at, 0);
        for c in 0..=chunks {
            let x = escaped.partition_point(|&(l, _)| l < c as u32 * CHUNK);
            s.extend_from_slice(&(x as u32).to_le_bytes());
        }
        escaped.iter().for_each(|&(l, x)| {
            s.extend_from_slice(&l.to_le_bytes());
            s.extend_from_slice(&x.to_le_bytes());
        });
        sections.push((Kind::Names, 0, s));
        // VOCAB.
        let hashes: Vec<u64> = order.iter().map(|&k| text_hash(&source[terms[k as usize].1 as usize..][..terms[k as usize].2 as usize])).collect();
        let by_hash_at = 16 + 16 * order.len();
        let mut s = head([order.len() as u32, 16, by_hash_at as u32, 0]);
        for (r, &k) in order.iter().enumerate() {
            s.extend_from_slice(&hashes[r].to_le_bytes());
            s.extend_from_slice(&terms[k as usize].1.to_le_bytes());
            s.extend_from_slice(&terms[k as usize].2.to_le_bytes());
        }
        let mut by_hash: Vec<u32> = (0..order.len() as u32).collect();
        by_hash.sort_by_key(|&r| (hashes[r as usize], r));
        by_hash.iter().for_each(|r| s.extend_from_slice(&r.to_le_bytes()));
        sections.push((Kind::Vocab, 0, s));
    }
    if want.has(Kind::Siblings) {
        let mut pairs = Vec::new();
        for p in 0..n {
            let end = t.subtree_end(p);
            let (mut c, mut k) = (p + 1, 0u32);
            while c < end {
                if is_named(c) {
                    if k > 0 && k % K == 0 {
                        pairs.push((p as u32, c as u32));
                    }
                    k += 1;
                }
                c = t.subtree_end(c);
            }
        }
        let mut s = head([pairs.len() as u32, 0, 0, 0]);
        pairs.iter().for_each(|&(p, c)| {
            s.extend_from_slice(&p.to_le_bytes());
            s.extend_from_slice(&c.to_le_bytes());
        });
        sections.push((Kind::Siblings, 0, s));
    }
    let counts = || {
        let mut m = std::collections::BTreeMap::<u32, u32>::new();
        (0..n).for_each(|i| *m.entry(t.code(i)).or_default() += 1);
        m
    };
    if want.has(Kind::Counts) {
        let m = counts();
        let mut s = head([m.len() as u32, 0, 0, 0]);
        m.iter().for_each(|(c, x)| {
            s.extend_from_slice(&c.to_le_bytes());
            s.extend_from_slice(&x.to_le_bytes());
        });
        sections.push((Kind::Counts, 0, s));
    }
    if want.has(Kind::Rare) {
        let limit = rare_limit(n);
        let rare: Vec<(u32, u32)> = counts().into_iter().filter(|&(_, x)| x <= limit).collect();
        let mut at = std::collections::HashMap::new();
        let mut start = 0u32;
        for &(c, x) in &rare {
            at.insert(c, start);
            start += x;
        }
        let mut nodes = vec![0u32; start as usize];
        for i in 0..n {
            if let Some(a) = at.get_mut(&t.code(i)) {
                nodes[*a as usize] = i as u32;
                *a += 1;
            }
        }
        let nodes_at = 16 + 8 * (rare.len() + 1);
        let mut s = head([rare.len() as u32, limit, nodes_at as u32, 0]);
        let mut start = 0u32;
        for &(c, x) in &rare {
            s.extend_from_slice(&c.to_le_bytes());
            s.extend_from_slice(&start.to_le_bytes());
            start += x;
        }
        s.extend_from_slice(&NONE.to_le_bytes());
        s.extend_from_slice(&start.to_le_bytes());
        nodes.iter().for_each(|x| s.extend_from_slice(&x.to_le_bytes()));
        sections.push((Kind::Rare, limit, s));
    }
    if want.has(Kind::Named) {
        let mut s = head([named.len() as u32, 0, 0, 0]);
        named.iter().for_each(|w| s.extend_from_slice(&w.to_le_bytes()));
        sections.push((Kind::Named, 0, s));
    }
    // The header, the directory, the sections.
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(sections.len() as u16).to_le_bytes());
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&(leaves.len() as u32).to_le_bytes());
    out.extend_from_slice(&(blocks as u32).to_le_bytes());
    out.extend_from_slice(&terms_count.to_le_bytes());
    out.extend_from_slice(&digest.to_le_bytes());
    let mut at = (HEADER + ENTRY * sections.len()).next_multiple_of(16);
    for (k, params, s) in &sections {
        out.extend_from_slice(&(*k as u16).to_le_bytes());
        out.extend_from_slice(&k.version().to_le_bytes());
        out.extend_from_slice(&params.to_le_bytes());
        out.extend_from_slice(&(at as u32).to_le_bytes());
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        at = (at + s.len()).next_multiple_of(16);
    }
    for (_, _, s) in &sections {
        out.resize(out.len().next_multiple_of(16), 0);
        out.extend_from_slice(s);
    }
    out.resize(out.len().next_multiple_of(16), 0);
    Ok(out)
}

/// The most nodes a rare code has in a document of `n` nodes.
pub fn rare_limit(n: usize) -> u32 {
    (n as u32 / 256).max(1)
}

/// Bit `code` of `words` (0 past them).
fn bit(words: &[u64], code: u32) -> bool {
    words.get((code / 64) as usize).is_some_and(|w| (w >> (code % 64)) & 1 != 0)
}

/// The named codes of a code table whose entry `c` is named where `named(c)`: a bit a code, as `build` takes them.
pub fn named_codes(codes: usize, named: impl Fn(u32) -> bool) -> Vec<u64> {
    let mut w = vec![0u64; codes.div_ceil(64)];
    (0..codes as u32).filter(|&c| named(c)).for_each(|c| w[(c / 64) as usize] |= 1 << (c % 64));
    w
}

fn head(w: [u32; 4]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

// ---- the reads -------------------------------------------------------------------------------------------

/// An auxiliary index read in place.
#[derive(Clone, Copy)]
pub struct AuxIndex<'a> {
    b: &'a [u8],
    /// Each kind's section offset (0: none), by kind.
    at: [u32; 8],
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
        let mut at = [0u32; 8];
        for e in 0..count {
            let d = HEADER + ENTRY * e;
            let (kind, version, off, len) = (u16_at(b, d), u16_at(b, d + 2), u32_at(b, d + 8), u32_at(b, d + 12));
            if off as usize % 16 != 0 || off as usize + len as usize > b.len() || len < 16 {
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

    pub fn bytes(&self) -> &'a [u8] {
        self.b
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
        if !self.has(Kind::Siblings) {
            return (0, NONE);
        }
        let first = self.family(parent);
        let (mut lo, mut hi) = (first, self.family(parent.saturating_add(1)));
        // The last entry of the family whose node is at most `node`.
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.pair(mid).1 <= node { lo = mid + 1 } else { hi = mid }
        }
        if lo == first { (0, NONE) } else { (lo - first, self.pair(lo - 1).1) }
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
    /// Each code present and its count, by code.
    pub fn counts(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        let (s, n) = if self.has(Kind::Counts) { (self.sec(Kind::Counts) + 16, self.hw(Kind::Counts, 0)) } else { (0, 0) };
        (0..n as usize).map(move |k| (u32_at(self.b, s + 8 * k), u32_at(self.b, s + 8 * k + 4)))
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

/// `node`'s place among its named siblings: the named children of its parent before it (0 for the first, and for
/// the root). From the last every-K-th named sibling at or before it, the tape's siblings stepped to it, at most K - 1
/// named ones and the tokens between. `ukl_aux_place` is the same in UKL.
pub fn place<T: Tape + ?Sized>(t: &T, aux: &AuxIndex, node: u32) -> u32 {
    let Some(p) = t.parent(node as usize) else { return 0 };
    let (k, from) = match aux.eighth_at_or_before(p as u32, node) {
        (_, NONE) => (0, p as u32 + 1),
        (k, e) => (k * K, e),
    };
    let (mut c, mut i) = (from as usize, k);
    while c != node as usize {
        i += u32::from(aux.named(t.code(c)));
        c = t.subtree_end(c);
    }
    i
}

/// `parent`'s named child `i`, or `None`: from its every-K-th named child, the siblings stepped past at most K - 1
/// named ones. `ukl_aux_child` is the same in UKL.
pub fn child<T: Tape + ?Sized>(t: &T, aux: &AuxIndex, parent: u32, i: u32) -> Option<u32> {
    let end = t.subtree_end(parent as usize);
    let mut c = if i < K { parent as usize + 1 } else { aux.eighth(parent, i / K) as usize };
    if c == NONE as usize {
        return None;
    }
    let mut left = i % K;
    while c < end {
        if aux.named(t.code(c)) {
            if left == 0 {
                return Some(c as u32);
            }
            left -= 1;
        }
        c = t.subtree_end(c);
    }
    None
}

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

/// A document for the auxiliary index's tests, here and in the UKL reads' (`rerail-ukl-vec`): nodes `(code, start, end,
/// subtree end)` in preorder over its source. Families of every width (to 300 children, so SIBLINGS has entries,
/// nested), leaves named from a vocabulary of 700 identifiers at Zipf-like frequencies (so some escape), and rare
/// and common codes.
#[cfg(any(test, feature = "test-hooks"))]
pub mod sample {
    use crate::tape::{CodeTable, Entry, Tape};

    pub struct Doc {
        pub nodes: Vec<(u32, u64, u64, usize)>,
        pub source: Vec<u8>,
        pub table: CodeTable,
    }

    impl Doc {
        pub fn new() -> Doc {
            let entries: Vec<Entry> = (0..64u16).map(|k| Entry { kind: k, field: 0, flags: 0, alias: None, classes: Vec::new() }).collect();
            let mut d = Doc { nodes: Vec::new(), source: Vec::new(), table: CodeTable::new(entries) };
            let mut x = 0x2545_F491u32;
            let mut rnd = move || {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x
            };
            let at = d.open(0);
            for f in 0..60u32 {
                let width = [1, 3, 7, 8, 9, 15, 16, 17, 40, 300][(f % 10) as usize];
                let fam = d.open(1 + f % 5);
                for c in 0..width {
                    if c % 23 == 5 {
                        let inner = d.open(6 + c % 3);
                        for _ in 0..(c % 11) {
                            d.leaf(&mut rnd);
                        }
                        d.close(inner);
                    } else {
                        d.leaf(&mut rnd);
                    }
                }
                d.close(fam);
            }
            d.close(at);
            d
        }
        fn open(&mut self, code: u32) -> usize {
            self.nodes.push((code, self.source.len() as u64, 0, 0));
            self.nodes.len() - 1
        }
        fn close(&mut self, i: usize) {
            self.nodes[i].2 = self.source.len() as u64;
            self.nodes[i].3 = self.nodes.len();
            self.source.push(b'\n');
        }
        fn leaf(&mut self, rnd: &mut impl FnMut() -> u32) {
            // A term from 700, the low ones far more often; its code by the term (some codes rare).
            let r = rnd();
            let term = if r % 4 == 0 { (r >> 8) % 700 } else { (r >> 8) % 40 };
            let code = if term % 97 == 3 { 40 + term % 7 } else { 10 + term % 20 };
            let start = self.source.len() as u64;
            let name = format!("n{}_{}", term, "x".repeat((term % 13) as usize));
            self.source.extend_from_slice(name.as_bytes());
            self.nodes.push((code, start, self.source.len() as u64, self.nodes.len() + 1));
            self.source.push(b' ');
        }
    }

    /// The sample's named codes: all but those `c % 5 == 2` (leaves among them, so families hold tokens a place
    /// steps over, and inner nodes too).
    pub fn named() -> Vec<u64> {
        super::named_codes(64, |c| c % 5 != 2)
    }

    impl Default for Doc {
        fn default() -> Self {
            Doc::new()
        }
    }

    impl Tape for Doc {
        fn len(&self) -> usize {
            self.nodes.len()
        }
        fn table(&self) -> &CodeTable {
            &self.table
        }
        fn code(&self, i: usize) -> u32 {
            self.nodes[i].0
        }
        fn subtree_end(&self, i: usize) -> usize {
            self.nodes[i].3
        }
        fn parent(&self, i: usize) -> Option<usize> {
            (0..i).rev().find(|&p| self.nodes[p].3 > i)
        }
        fn start(&self, i: usize) -> u64 {
            self.nodes[i].1
        }
        fn end(&self, i: usize) -> u64 {
            self.nodes[i].2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every read answers as the document says: each node's leaf rank and back, each leaf's term and its text, each
    /// term found by its text and its leaves, each node's place and each parent's children, each code's count and
    /// its rare nodes.
    #[test]
    fn the_aux_index_reads_every_node_as_the_document_says() {
        let d = sample::Doc::new();
        let b = build(&d, &d.source, 7, &sample::named(), Sections::ALL).unwrap();
        let aux = AuxIndex::read(&b).unwrap();
        assert_eq!((aux.nodes() as usize, aux.digest()), (d.len(), 7));
        assert!(Kind::ALL.iter().all(|&k| aux.has(k)));
        let mut rank = 0;
        let mut seen: std::collections::HashMap<&[u8], u32> = std::collections::HashMap::new();
        let mut per_term: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
        for i in 0..d.len() as u32 {
            if d.subtree_end(i as usize) == i as usize + 1 {
                assert_eq!(aux.leaf_rank(i), rank, "node {i}'s rank");
                assert_eq!(aux.leaf_node(rank), i, "leaf {rank}'s node");
                let text = &d.source[d.start(i as usize) as usize..d.end(i as usize) as usize];
                let t = aux.term_of_node(i);
                assert_eq!(aux.term_text(t, &d.source), Some(text), "node {i}'s term");
                assert_eq!(*seen.entry(text).or_insert(t), t, "one term a text");
                per_term.entry(t).or_default().push(i);
                rank += 1;
            } else {
                assert_eq!((aux.leaf_rank(i), aux.term_of_node(i)), (NONE, NONE), "inner node {i}");
            }
        }
        assert_eq!(aux.leaves(), rank);
        assert!(aux.terms() > 300 && aux.terms() as usize == seen.len(), "{} terms: some escape", aux.terms());
        // Most frequent first.
        let counts: Vec<usize> = (0..aux.terms()).map(|t| per_term[&t].len()).collect();
        assert!(counts.windows(2).all(|w| w[0] >= w[1]), "terms by frequency");
        for (&text, &t) in &seen {
            assert_eq!(aux.term_of_text(text, &d.source), t);
            assert_eq!(aux.term_by_hash(text_hash(text)), t);
            assert_eq!(aux.nodes_with_term(t).collect::<Vec<_>>(), per_term[&t], "term {t}'s nodes");
        }
        assert_eq!(aux.term_of_text(b"absent", &d.source), NONE);
        // Places and children, among the named: a token's place is the named siblings before it.
        let named = sample::named();
        let (mut wide, mut tokens) = (0, 0);
        for p in 0..d.len() as u32 {
            let mut c = p as usize + 1;
            let mut k = 0;
            while c < d.subtree_end(p as usize) {
                assert_eq!(place(&d, &aux, c as u32), k, "node {c}'s place");
                if bit(&named, d.code(c)) {
                    assert_eq!(child(&d, &aux, p, k), Some(c as u32), "{p}'s child {k}");
                    k += 1;
                } else {
                    tokens += 1;
                }
                c = d.subtree_end(c);
            }
            assert_eq!(child(&d, &aux, p, k), None, "{p} has {k} named children");
            wide += usize::from(k > 200);
        }
        assert!(wide > 0 && tokens > 100, "{wide} wide families, {tokens} tokens");
        assert_eq!(place(&d, &aux, 0), 0);
        assert!((0..64).all(|c| aux.named(c) == bit(&named, c)));
        // Counts and rare codes.
        let mut by_code = std::collections::BTreeMap::<u32, Vec<u32>>::new();
        (0..d.len() as u32).for_each(|i| by_code.entry(d.code(i as usize)).or_default().push(i));
        assert_eq!(aux.counts().collect::<Vec<_>>(), by_code.iter().map(|(c, v)| (*c, v.len() as u32)).collect::<Vec<_>>());
        let limit = rare_limit(d.len());
        let mut rare = 0;
        for c in 0..64 {
            let want = by_code.get(&c).cloned().unwrap_or_default();
            match aux.rare(c) {
                Some(it) => {
                    assert!(want.len() as u32 <= limit);
                    assert_eq!(it.collect::<Vec<_>>(), want, "code {c}'s rare nodes");
                    rare += usize::from(!want.is_empty());
                }
                None => assert!(want.len() as u32 > limit, "code {c} of {} nodes is rare", want.len()),
            }
        }
        assert!(rare > 0);
    }

    /// An auxiliary index of a format 16 tape is the same bytes as one of the document it was written from: the auxiliary index is a
    /// function of what the tape reads, whatever the tape's format.
    #[test]
    fn a_format_16_tapes_aux_index_is_its_documents() {
        let d = sample::Doc::new();
        let lang = crate::f16::Lang::new(d.table.clone(), [1, 1, 1], 1).unwrap();
        let number: Vec<u32> = crate::f16::numbers(&lang, &d.table).unwrap();
        let Ok(crate::f16::Written::F16(t)) = crate::f16::write_from(&lang, &d, &number, &d.source, crate::newlines::Terminators::Lf, 0, false) else { panic!("format 16") };
        let r = t.reader(&lang).unwrap();
        // The tape's codes are the language's numbers: the same where the table is in order.
        assert!((0..d.len()).all(|i| r.code(i) == number[d.code(i) as usize]));
        let identity = number.iter().enumerate().all(|(k, &x)| k as u32 == x);
        // The named codes, as the tape numbers them.
        let named = sample::named();
        let named_r = named_codes(number.iter().max().map_or(0, |&m| m as usize + 1), |x| number.iter().position(|&y| y == x).is_some_and(|c| bit(&named, c as u32)));
        let (a, b) = (build(&d, &d.source, 1, &named, Sections::ALL).unwrap(), build(&r, &d.source, 1, &named_r, Sections::ALL).unwrap());
        if identity {
            assert_eq!(a, b);
        } else {
            let only = Sections::of(&[Kind::Leaves, Kind::Names, Kind::Vocab]);
            assert_eq!(build(&d, &d.source, 1, &named, only).unwrap(), build(&r, &d.source, 1, &named_r, only).unwrap());
            let (a, b) = (AuxIndex::read(&a).unwrap(), AuxIndex::read(&b).unwrap());
            assert!((0..d.len() as u32).all(|p| (1..40).all(|k| a.eighth(p, k) == b.eighth(p, k))), "the same every-8th named children");
        }
    }

    /// Decision 0656 step 3: a tape's auxiliary index is laid in its home after its presence, and a home read back
    /// (format 16 and 16z) holds the same tape, the same presence and the same auxiliary index, read in place. One
    /// is laid once, after a presence; a home whose index does not read is refused.
    #[test]
    fn a_tapes_home_holds_its_aux_index_after_its_presence() {
        use crate::f16::{Tape16, Tape16Z, Written};
        let d = sample::Doc::new();
        let lang = crate::f16::Lang::new(d.table.clone(), [1, 1, 1], 1).unwrap();
        let number: Vec<u32> = crate::f16::numbers(&lang, &d.table).unwrap();
        for z in [false, true] {
            let mut w = crate::f16::write_from_with_presence(&lang, &d, &number, &d.source, crate::newlines::Terminators::Lf, 0, z).unwrap().tape;
            let (tape, presence) = (match &w { Written::F16(t) => t.as_bytes().to_vec(), Written::F16Z(t) => t.as_bytes().to_vec() }, w.presence_bytes().unwrap().to_vec());
            assert!(w.aux_bytes().is_none());
            let aux = build(&d, &d.source, 7, &sample::named(), Sections::ALL).unwrap();
            w.lay_aux(&aux).unwrap();
            assert!(w.lay_aux(&aux).is_err(), "laid once");
            let home = w.extent().to_vec();
            let held: Box<dyn Fn(&[u8]) -> Result<(Vec<u8>, Vec<u8>, Option<Vec<u8>>), String>> = if z {
                Box::new(|b| Tape16Z::from_bytes(crate::Vector::from(b)).map(|t| (t.as_bytes().to_vec(), t.presence_bytes().unwrap().to_vec(), t.aux_bytes().map(<[u8]>::to_vec))))
            } else {
                Box::new(|b| Tape16::from_bytes(crate::Vector::from(b)).map(|t| (t.as_bytes().to_vec(), t.presence_bytes().unwrap().to_vec(), t.aux_bytes().map(<[u8]>::to_vec))))
            };
            let (t, p, a) = held(&home).unwrap();
            assert_eq!(t, tape, "z {z}: the tape");
            assert_eq!(p[..presence.len()], presence[..], "z {z}: the presence");
            assert!(p[presence.len()..].iter().all(|&b| b == 0) && p.len() - presence.len() < 16 && p.len() % 16 == 0);
            assert_eq!(a.as_deref(), Some(&aux[..]), "z {z}: the auxiliary index");
            // The presence alone reads as before; a broken index is refused.
            let at = home.len() - aux.len();
            assert!(held(&home[..at - (p.len() - presence.len())]).is_ok());
            let mut bad = home.clone();
            bad[at] ^= 1;
            assert!(held(&bad).is_err(), "z {z}: a home whose index does not read");
        }
    }

    /// Sections are chosen: one made without others reads `NONE` for theirs; names bring their vocabulary and leaves.
    #[test]
    fn an_aux_index_without_a_section_answers_none_for_it() {
        let d = sample::Doc::new();
        let b = build(&d, &d.source, 0, &sample::named(), Sections::of(&[Kind::Names])).unwrap();
        let aux = AuxIndex::read(&b).unwrap();
        assert!(aux.has(Kind::Leaves) && aux.has(Kind::Vocab) && !aux.has(Kind::Siblings) && !aux.has(Kind::Counts));
        assert_eq!((aux.eighth(0, 1), aux.count(10)), (NONE, NONE));
        assert!(aux.rare(40).is_none());
        let b = build(&d, &d.source, 0, &sample::named(), Sections::of(&[Kind::Counts])).unwrap();
        let aux = AuxIndex::read(&b).unwrap();
        assert_eq!((aux.leaf_rank(1), aux.term_of_text(b"n0_", &d.source)), (NONE, NONE));
        assert!(AuxIndex::read(&b[..8]).is_err());
    }
}
