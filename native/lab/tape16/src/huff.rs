//! An entropy stage for the LZ codecs. It uses static canonical Huffman tables shared per language, like the
//! dictionaries, so no table travels with a block. There is one table for sequence bytes and one for
//! literals.
//!
//! Each checkpoint keeps the bit offset of its sequences and of its literals. A segment therefore decodes
//! on its own, and the GPU can Huffman-decode every segment in parallel before the LZ pass (`gpu`).
//!
//! An entropy-coded block:
//!
//! ```text
//! varints      decoded length, sequences, literal bytes, K, checkpoints, sequence bytes
//! checkpoints  output offset, literal offset, sequence byte offset, sequence bit, literal bit (u32 each)
//! varint       the sequence stream's bytes
//! streams      the sequence bits, then the literal bits (LSB first)
//! ```

use crate::lp::{self, read_varint, varint};

pub const MAX_BITS: u32 = 12;

#[derive(Clone)]
pub struct Table {
    pub len: [u8; 256],
    pub code: [u16; 256],
    /// Indexed by the next MAX_BITS bits: (symbol, length).
    pub lut: Vec<(u8, u8)>,
}

/// Code lengths for `counts` (every symbol gets one; unseen symbols count as 1), limited to MAX_BITS.
fn lengths(counts: &[u64; 256]) -> [u8; 256] {
    // Huffman by repeated merging of the two lightest, tracking each symbol's depth.
    let mut nodes: Vec<(u64, Vec<u8>)> = (0..256).map(|s| (counts[s] + 1, vec![s as u8])).collect();
    let mut len = [0u8; 256];
    while nodes.len() > 1 {
        nodes.sort_by(|a, b| b.0.cmp(&a.0));
        let (wa, a) = nodes.pop().unwrap();
        let (wb, b) = nodes.pop().unwrap();
        for &s in a.iter().chain(&b) {
            len[s as usize] += 1;
        }
        nodes.push((wa + wb, [a, b].concat()));
    }
    // Limit to MAX_BITS: clamp, then lengthen the shortest codes until Kraft's sum is at most 1.
    for l in &mut len {
        *l = (*l).clamp(1, MAX_BITS as u8);
    }
    let kraft = |len: &[u8; 256]| len.iter().map(|&l| 1u64 << (MAX_BITS - l as u32)).sum::<u64>();
    while kraft(&len) > 1 << MAX_BITS {
        // Lengthen the symbol with the longest code still below the limit and the least count.
        let s = (0..256).filter(|&s| len[s] < MAX_BITS as u8).max_by_key(|&s| (len[s], u64::MAX - counts[s])).unwrap();
        len[s] += 1;
    }
    len
}

impl Table {
    pub fn train(counts: &[u64; 256]) -> Self {
        let len = lengths(counts);
        // Canonical codes, by (length, symbol), bit-reversed for an LSB-first stream.
        let mut order: Vec<usize> = (0..256).collect();
        order.sort_by_key(|&s| (len[s], s));
        let mut code = [0u16; 256];
        let (mut c, mut prev) = (0u32, len[order[0]] as u32);
        for (i, &s) in order.iter().enumerate() {
            let l = len[s] as u32;
            if i > 0 {
                c = (c + 1) << (l - prev);
            }
            prev = l;
            code[s] = (c.reverse_bits() >> (32 - l)) as u16;
        }
        let mut lut = vec![(0u8, 0u8); 1 << MAX_BITS];
        for s in 0..256 {
            let l = len[s] as u32;
            let mut x = code[s] as u32;
            while x < 1 << MAX_BITS {
                lut[x as usize] = (s as u8, l as u8);
                x += 1 << l;
            }
        }
        Table { len, code, lut }
    }

    pub fn bits(&self, bytes: &[u8]) -> u64 {
        bytes.iter().map(|&b| u64::from(self.len[b as usize])).sum()
    }
}

struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
    total: u64,
}

impl Bits {
    fn put(&mut self, code: u16, len: u8) {
        self.acc |= u64::from(code) << self.n;
        self.n += u32::from(len);
        self.total += u64::from(len);
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// `n` symbols from `stream` starting at bit `bit`, appended to `out`.
pub fn decode(stream: &[u8], bit: u64, n: usize, t: &Table, out: &mut Vec<u8>) {
    let mut pos = bit;
    for _ in 0..n {
        let byte = (pos >> 3) as usize;
        let mut w = [0u8; 8];
        let avail = stream.len().saturating_sub(byte).min(8);
        w[..avail].copy_from_slice(&stream[byte..byte + avail]);
        let x = (u64::from_le_bytes(w) >> (pos & 7)) as usize & ((1 << MAX_BITS) - 1);
        let (s, l) = t.lut[x];
        out.push(s);
        pos += u64::from(l);
    }
}

/// An LZ block (`lp::parts` layout) with its sequences and literals entropy-coded.
pub fn encode(c: &[u8], seq_t: &Table, lit_t: &Table) -> Vec<u8> {
    let p = lp::parts(c);
    let mut sb = Bits { out: Vec::new(), acc: 0, n: 0, total: 0 };
    let mut lb = Bits { out: Vec::new(), acc: 0, n: 0, total: 0 };
    // Bit offsets at each checkpoint's sequence byte and literal byte.
    let mut seq_bit = vec![0u32; p.checks.len()];
    let mut lit_bit = vec![0u32; p.checks.len()];
    let mut ck = 0;
    for (i, &b) in p.seqs.iter().enumerate() {
        while ck < p.checks.len() && p.checks[ck][2] as usize == i {
            seq_bit[ck] = sb.total as u32;
            ck += 1;
        }
        sb.put(seq_t.code[b as usize], seq_t.len[b as usize]);
    }
    while ck < p.checks.len() {
        seq_bit[ck] = sb.total as u32;
        ck += 1;
    }
    let mut ck = 0;
    for (i, &b) in p.lits.iter().enumerate() {
        while ck < p.checks.len() && p.checks[ck][1] as usize == i {
            lit_bit[ck] = lb.total as u32;
            ck += 1;
        }
        lb.put(lit_t.code[b as usize], lit_t.len[b as usize]);
    }
    while ck < p.checks.len() {
        lit_bit[ck] = lb.total as u32;
        ck += 1;
    }
    let (s, l) = (sb.finish(), lb.finish());
    let mut out = Vec::with_capacity(s.len() + l.len() + p.checks.len() * 20 + 24);
    for v in [p.len, p.nseq, p.lits.len(), p.k, p.checks.len(), p.seqs.len()] {
        varint(&mut out, v);
    }
    for (i, c) in p.checks.iter().enumerate() {
        for v in [c[0], c[1], c[2], seq_bit[i], lit_bit[i]] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    varint(&mut out, s.len());
    out.extend_from_slice(&s);
    out.extend_from_slice(&l);
    out
}

/// The parts of an entropy-coded block, for a decoder (CPU or GPU).
pub struct Coded<'a> {
    pub len: usize,
    pub nseq: usize,
    pub nlit: usize,
    pub k: usize,
    pub nseq_bytes: usize,
    /// Output offset, literal offset, sequence byte offset, sequence bit, literal bit.
    pub checks: Vec<[u32; 5]>,
    pub seq_bits: &'a [u8],
    pub lit_bits: &'a [u8],
}

pub fn coded(c: &[u8]) -> Coded<'_> {
    let mut at = 0;
    let v: Vec<usize> = (0..6).map(|_| read_varint(c, &mut at)).collect();
    let mut checks = Vec::with_capacity(v[4]);
    for _ in 0..v[4] {
        let g = |i: usize| u32::from_le_bytes(c[at + 4 * i..at + 4 * i + 4].try_into().unwrap());
        checks.push([g(0), g(1), g(2), g(3), g(4)]);
        at += 20;
    }
    let ns = read_varint(c, &mut at);
    Coded { len: v[0], nseq: v[1], nlit: v[2], k: v[3], nseq_bytes: v[5], checks, seq_bits: &c[at..at + ns], lit_bits: &c[at + ns..] }
}

/// The plain LZ block back (`lp::parts` layout), segment by segment from the checkpoints' bit offsets.
pub fn decode_block(c: &[u8], seq_t: &Table, lit_t: &Table) -> Vec<u8> {
    let p = coded(c);
    let (mut seqs, mut lits) = (Vec::with_capacity(p.nseq_bytes), Vec::with_capacity(p.nlit));
    for (i, ck) in p.checks.iter().enumerate() {
        let next = p.checks.get(i + 1);
        let s_end = next.map_or(p.nseq_bytes, |n| n[2] as usize);
        let l_end = next.map_or(p.nlit, |n| n[1] as usize);
        decode(p.seq_bits, u64::from(ck[3]), s_end - ck[2] as usize, seq_t, &mut seqs);
        decode(p.lit_bits, u64::from(ck[4]), l_end - ck[1] as usize, lit_t, &mut lits);
    }
    let mut out = Vec::with_capacity(seqs.len() + lits.len() + p.checks.len() * 12 + 16);
    for v in [p.len, p.nseq, p.nlit, p.k, p.checks.len()] {
        varint(&mut out, v);
    }
    for ck in &p.checks {
        for v in &ck[..3] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    varint(&mut out, seqs.len());
    out.extend_from_slice(&seqs);
    out.extend_from_slice(&lits);
    out
}

/// Byte counts of the sequences and the literals of LZ blocks, for training.
pub fn count(blocks: &[Vec<u8>], seq: &mut [u64; 256], lit: &mut [u64; 256]) {
    for c in blocks {
        let p = lp::parts(c);
        for &b in p.seqs {
            seq[b as usize] += 1;
        }
        for &b in p.lits {
            lit[b as usize] += 1;
        }
    }
}
