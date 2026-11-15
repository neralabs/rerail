//! Literal-pool LZ (0156 §2, T1), and the same coder with references into decoded output (LZ4's rule), so
//! the two differ only in what a copy may read.
//!
//! A compressed block:
//!
//! ```text
//! header       varints: decoded length, sequences, literal bytes, K, checkpoints
//! checkpoints  every K sequences: output offset, literal offset, sequence offset (u32 each)
//! sequences    token (literal run length << 4 | op), varint extensions; op 0 none, 1 run, 2 copy
//! literals     the block's literal bytes, contiguous
//! ```
//!
//! In `Refs::Literals` a copy reads the pool: the dictionary, then this block's literals. It never reads
//! decoded output, so any byte decodes in one hop (`read_at`) and copies have no order between them.
//! In `Refs::Output` a copy reads the dictionary, then the decoded output so far (classic LZ).
//! In `Refs::Replay` an op-3 sequence replays sequences `[i - dist, i - dist + count)`: the same output
//! again, read from the compressed form.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refs {
    Literals,
    Output,
    /// `Literals`, and a copy may also replay a run of earlier sequences (op 3): a pointer into the
    /// compressed block, not into its output. Replayed sequences are never replays themselves, so any byte
    /// is still at most two hops away (the user's "pointers to compressed content").
    Replay,
}

pub(crate) const MIN_MATCH: usize = 4;
pub(crate) const MIN_RUN: usize = 8;
const HASH_BITS: u32 = 16;
pub(crate) const CHAIN: usize = 16;

pub(crate) fn hash4(b: &[u8]) -> usize {
    let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    (v.wrapping_mul(2654435761) >> (32 - HASH_BITS)) as usize
}

/// A dictionary and its match index, built once and shared by every block (as on the device).
pub struct Dict {
    pub bytes: Vec<u8>,
    pub(crate) head: Vec<u32>,
    pub(crate) prev: Vec<u32>,
}

pub(crate) const NIL: u32 = u32::MAX;

impl Dict {
    pub fn new(bytes: Vec<u8>) -> Self {
        let mut head = vec![NIL; 1 << HASH_BITS];
        let mut prev = vec![NIL; bytes.len()];
        for i in 0..bytes.len().saturating_sub(3) {
            let h = hash4(&bytes[i..]);
            prev[i] = head[h];
            head[h] = i as u32;
        }
        Dict { bytes, head, prev }
    }

    pub fn empty() -> Self {
        Dict { bytes: Vec::new(), head: vec![NIL; 1 << HASH_BITS], prev: Vec::new() }
    }
}

pub(crate) fn varint(out: &mut Vec<u8>, mut v: usize) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

pub(crate) fn read_varint(b: &[u8], at: &mut usize) -> usize {
    let mut v = 0usize;
    let mut s = 0;
    loop {
        let c = b[*at];
        *at += 1;
        v |= usize::from(c & 0x7f) << s;
        if c < 0x80 {
            return v;
        }
        s += 7;
    }
}

/// The pool's byte `i`: the dictionary's, then `tail`'s (literals or output).
#[inline]
pub(crate) fn pool_at(dict: &[u8], tail: &[u8], i: usize) -> u8 {
    if i < dict.len() { dict[i] } else { tail[i - dict.len()] }
}

struct Seq {
    lits: usize,
    op: u8,
    len: usize,
    arg: usize,
}

/// The output length of replay sequence `i` (the sum of the sequences it replays).
fn replay_len(seqs: &[Seq], i: usize) -> usize {
    let s = &seqs[i];
    (i - s.arg..i - s.arg + s.len).map(|t| seqs[t].lits + seqs[t].len).sum()
}

pub fn compress(data: &[u8], dict: &Dict, refs: Refs, k: usize) -> Vec<u8> {
    let d = &dict.bytes[..];
    // The block's part of the pool: its literals (Literals) or its output so far (Output).
    let mut tail: Vec<u8> = Vec::with_capacity(data.len());
    let mut head = vec![NIL; 1 << HASH_BITS];
    let mut prev: Vec<u32> = Vec::with_capacity(data.len());
    let mut lits: Vec<u8> = Vec::new();
    let mut seqs: Vec<Seq> = Vec::new();
    let mut pending = 0usize;
    // Replay (op 3): every non-replay sequence's output start, length, and an index by its first 4 bytes.
    let (mut seq_out, mut seq_len, mut seq_plain) = (Vec::<usize>::new(), Vec::<usize>::new(), Vec::<bool>::new());
    let mut rhead = vec![NIL; 1 << HASH_BITS];
    let mut rprev: Vec<u32> = Vec::new();
    let mut out_at = 0usize;
    let record = |s: &Seq, n: usize, out_at: &mut usize, seq_out: &mut Vec<usize>, seq_len: &mut Vec<usize>, seq_plain: &mut Vec<bool>, rhead: &mut Vec<u32>, rprev: &mut Vec<u32>| {
        let i = seq_out.len();
        seq_out.push(*out_at);
        seq_len.push(n);
        seq_plain.push(s.op != 3);
        rprev.push(NIL);
        if s.op != 3 && *out_at + 4 <= data.len() {
            let h = hash4(&data[*out_at..]);
            rprev[i] = rhead[h];
            rhead[h] = i as u32;
        }
        *out_at += n;
    };
    let index = |tail: &Vec<u8>, head: &mut Vec<u32>, prev: &mut Vec<u32>| {
        // Index the pool position that now has 4 bytes after it.
        let q = tail.len();
        prev.resize(q, NIL);
        if q >= 4 {
            let i = q - 4;
            let h = hash4(&tail[i..]);
            prev[i] = head[h];
            head[h] = i as u32;
        }
    };
    let mut p = 0;
    while p < data.len() {
        let mut run = 1;
        while p + run < data.len() && data[p + run] == data[p] {
            run += 1;
        }
        if run >= MIN_RUN {
            let sq = Seq { lits: pending, op: 1, len: run, arg: data[p] as usize };
            record(&sq, sq.lits + sq.len, &mut out_at, &mut seq_out, &mut seq_len, &mut seq_plain, &mut rhead, &mut rprev);
            seqs.push(sq);
            pending = 0;
            if refs == Refs::Output {
                for &b in &data[p..p + run] {
                    tail.push(b);
                    index(&tail, &mut head, &mut prev);
                }
            }
            p += run;
            continue;
        }
        // The longest match among the dictionary's and the block's candidates.
        let (mut best, mut best_at) = (0usize, 0usize);
        if p + MIN_MATCH <= data.len() {
            let h = hash4(&data[p..]);
            let mut try_at = |at: usize, tail: &[u8]| {
                let total = d.len() + tail.len();
                let mut l = 0;
                while p + l < data.len() && at + l < total && pool_at(d, tail, at + l) == data[p + l] {
                    l += 1;
                }
                if l > best {
                    best = l;
                    best_at = at;
                }
            };
            let mut c = dict.head[h];
            for _ in 0..CHAIN {
                if c == NIL {
                    break;
                }
                try_at(c as usize, &tail);
                c = dict.prev[c as usize];
            }
            let mut c = head[h];
            for _ in 0..CHAIN {
                if c == NIL {
                    break;
                }
                try_at(d.len() + c as usize, &tail);
                c = prev[c as usize];
            }
        }
        // A replay: earlier whole sequences whose output is what comes next (from the sequence being formed).
        let (mut rbest, mut rat, mut rcount) = (0usize, 0usize, 0usize);
        if refs == Refs::Replay && p + 4 <= data.len() {
            let mut c = rhead[hash4(&data[p..])];
            for _ in 0..CHAIN {
                if c == NIL {
                    break;
                }
                let (mut t, mut covered) = (c as usize, 0usize);
                while t < seq_out.len() && seq_plain[t] {
                    let (o, n) = (seq_out[t], seq_len[t]);
                    if p + covered + n > data.len() || data[o..o + n] != data[p + covered..p + covered + n] {
                        break;
                    }
                    covered += n;
                    t += 1;
                }
                if covered > rbest {
                    (rbest, rat, rcount) = (covered, c as usize, t - c as usize);
                }
                c = rprev[c as usize];
            }
        }
        if (rcount >= 2 || (rcount == 1 && rbest >= 16)) && rbest > best {
            let sq = Seq { lits: pending, op: 3, len: rcount, arg: seq_out.len() - rat };
            pending = 0;
            record(&sq, sq.lits + rbest, &mut out_at, &mut seq_out, &mut seq_len, &mut seq_plain, &mut rhead, &mut rprev);
            seqs.push(sq);
            p += rbest;
            continue;
        }
        if best >= MIN_MATCH {
            let sq = Seq { lits: pending, op: 2, len: best, arg: best_at };
            record(&sq, sq.lits + sq.len, &mut out_at, &mut seq_out, &mut seq_len, &mut seq_plain, &mut rhead, &mut rprev);
            seqs.push(sq);
            pending = 0;
            if refs == Refs::Output {
                for &b in &data[p..p + best] {
                    tail.push(b);
                    index(&tail, &mut head, &mut prev);
                }
            }
            p += best;
        } else {
            lits.push(data[p]);
            pending += 1;
            tail.push(data[p]);
            index(&tail, &mut head, &mut prev);
            p += 1;
        }
    }
    if pending > 0 || seqs.is_empty() {
        seqs.push(Seq { lits: pending, op: 0, len: 0, arg: 0 });
    }
    // Sequences, with a checkpoint every k.
    let mut body = Vec::with_capacity(seqs.len() * 3);
    let mut checks: Vec<[u32; 3]> = Vec::new();
    let (mut out_off, mut lit_off) = (0usize, 0usize);
    for (i, s) in seqs.iter().enumerate() {
        if i % k == 0 {
            checks.push([out_off as u32, lit_off as u32, body.len() as u32]);
        }
        let lit_nib = s.lits.min(15);
        let op_nib = s.op;
        body.push((lit_nib << 4) as u8 | op_nib);
        if lit_nib == 15 {
            varint(&mut body, s.lits - 15);
        }
        match s.op {
            1 => {
                varint(&mut body, s.len - MIN_RUN);
                body.push(s.arg as u8);
            }
            2 => {
                varint(&mut body, s.len - MIN_MATCH);
                varint(&mut body, s.arg);
            }
            3 => {
                varint(&mut body, s.len - 1);
                varint(&mut body, s.arg);
            }
            _ => {}
        }
        out_off += s.lits + if s.op == 3 { replay_len(&seqs, i) } else { s.len };
        lit_off += s.lits;
    }
    let mut out = Vec::with_capacity(body.len() + lits.len() + checks.len() * 12 + 16);
    for v in [data.len(), seqs.len(), lits.len(), k, checks.len()] {
        varint(&mut out, v);
    }
    for c in &checks {
        for v in c {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    varint(&mut out, body.len());
    out.extend_from_slice(&body);
    out.extend_from_slice(&lits);
    out
}

pub struct Parts<'a> {
    pub len: usize,
    pub nseq: usize,
    pub k: usize,
    pub checks: Vec<[u32; 3]>,
    pub seqs: &'a [u8],
    pub lits: &'a [u8],
}

pub fn parts(c: &[u8]) -> Parts<'_> {
    let mut at = 0;
    let len = read_varint(c, &mut at);
    let nseq = read_varint(c, &mut at);
    let nlit = read_varint(c, &mut at);
    let k = read_varint(c, &mut at);
    let nck = read_varint(c, &mut at);
    let mut checks = Vec::with_capacity(nck);
    for _ in 0..nck {
        let g = |i: usize| u32::from_le_bytes(c[at + 4 * i..at + 4 * i + 4].try_into().unwrap());
        checks.push([g(0), g(1), g(2)]);
        at += 12;
    }
    let nbody = read_varint(c, &mut at);
    let seqs = &c[at..at + nbody];
    let lits = &c[at + nbody..at + nbody + nlit];
    Parts { len, nseq, k, checks, seqs, lits }
}

#[inline]
fn next_seq(seqs: &[u8], at: &mut usize) -> Seq {
    let t = seqs[*at];
    *at += 1;
    let mut lits = (t >> 4) as usize;
    if lits == 15 {
        lits += read_varint(seqs, at);
    }
    let op = t & 15;
    let (len, arg) = match op {
        1 => {
            let l = read_varint(seqs, at) + MIN_RUN;
            let b = seqs[*at] as usize;
            *at += 1;
            (l, b)
        }
        2 => {
            let l = read_varint(seqs, at) + MIN_MATCH;
            (l, read_varint(seqs, at))
        }
        3 => {
            let l = read_varint(seqs, at) + 1;
            (l, read_varint(seqs, at))
        }
        _ => (0, 0),
    };
    Seq { lits, op, len, arg }
}

/// One non-replay sequence's output (its literals at `li`, then its op), each byte passed to `put`.
#[inline]
fn emit(s: &Seq, li: usize, lits: &[u8], d: &[u8], mut put: impl FnMut(u8)) {
    for &b in &lits[li..li + s.lits] {
        put(b);
    }
    match s.op {
        1 => (0..s.len).for_each(|_| put(s.arg as u8)),
        2 => (0..s.len).for_each(|j| put(pool_at(d, lits, s.arg + j))),
        _ => {}
    }
}

/// Where sequence `t` starts in the body and the literals: its checkpoint, then at most K−1 sequences.
fn locate(p: &Parts, t: usize) -> (usize, usize) {
    let [_, li, at] = p.checks[t / p.k];
    let (mut at, mut li) = (at as usize, li as usize);
    for _ in 0..t % p.k {
        li += next_seq(p.seqs, &mut at).lits;
    }
    (at, li)
}

pub fn decompress(c: &[u8], dict: &Dict, refs: Refs) -> Vec<u8> {
    let p = parts(c);
    let d = &dict.bytes[..];
    let mut out = Vec::with_capacity(p.len);
    let (mut at, mut li) = (0usize, 0usize);
    let mut starts: Vec<(usize, usize)> = Vec::new();
    for i in 0..p.nseq {
        if refs == Refs::Replay {
            starts.push((at, li));
        }
        let s = next_seq(p.seqs, &mut at);
        match (refs, s.op) {
            (Refs::Output, 2) => {
                out.extend_from_slice(&p.lits[li..li + s.lits]);
                for j in 0..s.len {
                    let b = if s.arg + j < d.len() { d[s.arg + j] } else { out[s.arg + j - d.len()] };
                    out.push(b);
                }
            }
            (_, 3) => {
                out.extend_from_slice(&p.lits[li..li + s.lits]);
                for t in i - s.arg..i - s.arg + s.len {
                    let (mut ta, tl) = starts[t];
                    let ts = next_seq(p.seqs, &mut ta);
                    emit(&ts, tl, p.lits, d, |b| out.push(b));
                }
            }
            _ => emit(&s, li, p.lits, d, |b| out.push(b)),
        }
        li += s.lits;
    }
    out
}

/// `len` bytes at `offset` of the decoded block, read without decoding the rest: the checkpoint before it,
/// then at most K sequences, each byte one hop into the pool (two for a replay). Not for `Refs::Output`.
pub fn read_at(c: &[u8], dict: &Dict, offset: usize, len: usize, out: &mut Vec<u8>) {
    let p = parts(c);
    let d = &dict.bytes[..];
    let ck = match p.checks.binary_search_by(|c| (c[0] as usize).cmp(&offset)) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    let [o, li, at] = p.checks[ck];
    let (mut o, mut li, mut at) = (o as usize, li as usize, at as usize);
    let end = offset + len;
    out.clear();
    let mut i = ck * p.k;
    while o < end && i < p.nseq {
        let s = next_seq(p.seqs, &mut at);
        let mut q = o;
        let mut put = |b: u8| {
            if q >= offset && q < end {
                out.push(b);
            }
            q += 1;
        };
        if s.op == 3 {
            for &b in &p.lits[li..li + s.lits] {
                put(b);
            }
            for t in i - s.arg..i - s.arg + s.len {
                let (mut ta, tl) = locate(&p, t);
                let ts = next_seq(p.seqs, &mut ta);
                emit(&ts, tl, p.lits, d, &mut put);
            }
        } else {
            emit(&s, li, p.lits, d, &mut put);
        }
        o = q;
        li += s.lits;
        i += 1;
    }
}

/// Order-0 entropy of the sequences and the literals, in bytes: what an entropy stage with shared static
/// tables would reach (an estimate for T1's "literal-pool LZ + rANS" row, not a coder).
pub fn entropy_bytes(c: &[u8]) -> f64 {
    let p = parts(c);
    let h = |b: &[u8]| -> f64 {
        if b.is_empty() {
            return 0.0;
        }
        let mut f = [0usize; 256];
        for &x in b {
            f[x as usize] += 1;
        }
        let n = b.len() as f64;
        f.iter().filter(|&&c| c > 0).map(|&c| -(c as f64) * (c as f64 / n).log2()).sum::<f64>() / 8.0
    };
    h(p.seqs) + h(p.lits) + (p.checks.len() * 12) as f64 + 6.0
}
