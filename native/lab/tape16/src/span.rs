//! Span LZ: copies point at compressed input.
//!
//! A copy here names a run of earlier *compressed* sequences: in this block, or in any earlier block of the
//! file. Decoding it re-runs those sequences from their compressed bytes. That is the user's "LZ4 that
//! copies compressed input blocks to the output". Decoding needs only compressed input, which is all
//! resident, and never another thread's output. So every checkpoint segment of every block decodes in
//! parallel, and a block may still share whole runs with the blocks before it.
//!
//! A sequence is literal-pool LZ's (0156 §2), plus one op:
//!
//! ```text
//! op 4  span   varint back   0: this block; b: the block b before this one
//!              varint n − 1  how many sequences the span re-runs
//!              varint body   back = 0: distance back from this token; else that block's body offset
//!              varint lit    back = 0: distance back from this sequence's literal offset; else that
//!                            block's literal offset
//!              varint out    the span's output length (a reader skips it without expanding it)
//! ```
//!
//! A span may name spans in turn, down to `depth` levels. The decoder therefore keeps a stack of at most
//! `depth` entries, never an unbounded one. Ops 1 and 2 in a re-run sequence read their own block's literal
//! pool (the dictionary, then that block's literals).
//!
//! The block layout is literal-pool LZ's (`lp::parts`): header, checkpoints every K sequences,
//! sequences, literals.

use crate::lp::{self, CHAIN, Dict, MIN_MATCH, MIN_RUN, NIL, hash4, pool_at, read_varint, varint};

/// A span covers at least this many bytes (its token costs about 6).
const MIN_SPAN: usize = 12;
const SPAN_HASH_BITS: u32 = 16;

#[derive(Clone, Copy)]
struct Seq {
    lits: usize,
    op: u8,
    /// Op 1: run length; op 2: copy length; op 4: sequences spanned.
    len: usize,
    /// Op 1: the byte; op 2: the pool offset.
    arg: usize,
    /// Op 4: the target block (absolute), its first sequence, and the output length.
    block: usize,
    first: usize,
    out: usize,
}

/// One compressed block's sequences, as later blocks see them.
#[derive(Default)]
struct Table {
    out_at: Vec<usize>,
    out_len: Vec<usize>,
    depth: Vec<u8>,
    body_at: Vec<usize>,
    lit_at: Vec<usize>,
}

/// The file's spans so far: every block's table and a hash of each sequence's first 4 output bytes.
struct Index<'a> {
    data: Vec<&'a [u8]>,
    tables: Vec<Table>,
    head: Vec<u32>,
    prev: Vec<u32>,
    /// Global sequence id → (block, sequence).
    at: Vec<(u32, u32)>,
}

impl<'a> Index<'a> {
    fn add(&mut self, b: usize, s: usize) {
        let t = &self.tables[b];
        let (o, n) = (t.out_at[s], t.out_len[s]);
        let d = self.data[b];
        let g = self.at.len() as u32;
        self.at.push((b as u32, s as u32));
        self.prev.push(NIL);
        if n >= 4 && o + 4 <= d.len() {
            let h = hash4(&d[o..]) & ((1 << SPAN_HASH_BITS) - 1);
            self.prev[g as usize] = self.head[h];
            self.head[h] = g;
        }
    }
}

/// Compresses a file's blocks in order. A span may reach any earlier block when `cross`, else only its own.
pub fn compress_file(blocks: &[Vec<u8>], dict: &Dict, k: usize, depth: u8, cross: bool) -> Vec<Vec<u8>> {
    let mut ix = Index { data: Vec::new(), tables: Vec::new(), head: vec![NIL; 1 << SPAN_HASH_BITS], prev: Vec::new(), at: Vec::new() };
    let mut out = Vec::with_capacity(blocks.len());
    for (b, data) in blocks.iter().enumerate() {
        if !cross {
            ix.head.fill(NIL);
        }
        ix.data.push(data);
        ix.tables.push(Table::default());
        out.push(compress_block(b, &mut ix, dict, k, depth));
    }
    out
}

fn compress_block(b: usize, ix: &mut Index, dict: &Dict, k: usize, max_depth: u8) -> Vec<u8> {
    let data = ix.data[b];
    let d = &dict.bytes[..];
    let mut lits: Vec<u8> = Vec::new();
    let mut lhead = vec![NIL; 1 << 16];
    let mut lprev: Vec<u32> = Vec::new();
    let mut seqs: Vec<Seq> = Vec::new();
    let mut pending = 0usize;
    let mut out_at = 0usize;
    let push_lit = |lits: &mut Vec<u8>, lhead: &mut Vec<u32>, lprev: &mut Vec<u32>, c: u8| {
        lits.push(c);
        lprev.resize(lits.len(), NIL);
        if lits.len() >= 4 {
            let i = lits.len() - 4;
            let h = hash4(&lits[i..]);
            lprev[i] = lhead[h];
            lhead[h] = i as u32;
        }
    };
    let mut p = 0;
    while p < data.len() {
        let mut run = 1;
        while p + run < data.len() && data[p + run] == data[p] {
            run += 1;
        }
        // The longest literal-pool copy (dictionary, then this block's literals).
        let (mut best, mut best_at) = (0usize, 0usize);
        if p + MIN_MATCH <= data.len() {
            let h = hash4(&data[p..]);
            let mut try_at = |at: usize| {
                let total = d.len() + lits.len();
                let mut l = 0;
                while p + l < data.len() && at + l < total && pool_at(d, &lits, at + l) == data[p + l] {
                    l += 1;
                }
                if l > best {
                    (best, best_at) = (l, at);
                }
            };
            for (head, prev, base) in [(&dict.head, &dict.prev, 0usize), (&lhead, &lprev, d.len())] {
                let mut c = head[h];
                for _ in 0..CHAIN {
                    if c == NIL {
                        break;
                    }
                    try_at(base + c as usize);
                    c = prev.get(c as usize).copied().unwrap_or(NIL);
                }
            }
        }
        // The longest span: consecutive earlier sequences whose output is what comes next.
        let (mut sbest, mut sblock, mut sfirst, mut sn, mut sdepth) = (0usize, 0usize, 0usize, 0usize, 0u8);
        if p + 4 <= data.len() {
            let mut g = ix.head[hash4(&data[p..]) & ((1 << SPAN_HASH_BITS) - 1)];
            for _ in 0..CHAIN {
                if g == NIL {
                    break;
                }
                let (tb, ts) = (ix.at[g as usize].0 as usize, ix.at[g as usize].1 as usize);
                let (t, td) = (&ix.tables[tb], ix.data[tb]);
                let (mut s, mut covered, mut dep) = (ts, 0usize, 0u8);
                while s < t.out_at.len() && t.depth[s] < max_depth {
                    let (o, n) = (t.out_at[s], t.out_len[s]);
                    if p + covered + n > data.len() || td[o..o + n] != data[p + covered..p + covered + n] {
                        break;
                    }
                    covered += n;
                    dep = dep.max(t.depth[s]);
                    s += 1;
                }
                if covered > sbest {
                    (sbest, sblock, sfirst, sn, sdepth) = (covered, tb, ts, s - ts, dep);
                }
                g = ix.prev[g as usize];
            }
        }
        let (seq, depth, advance) = if run >= MIN_RUN && run >= sbest {
            (Seq { lits: pending, op: 1, len: run, arg: data[p] as usize, block: 0, first: 0, out: 0 }, 0, run)
        } else if sbest >= MIN_SPAN && sbest > best + 2 {
            (Seq { lits: pending, op: 4, len: sn, arg: 0, block: sblock, first: sfirst, out: sbest }, sdepth + 1, sbest)
        } else if best >= MIN_MATCH {
            (Seq { lits: pending, op: 2, len: best, arg: best_at, block: 0, first: 0, out: 0 }, 0, best)
        } else {
            push_lit(&mut lits, &mut lhead, &mut lprev, data[p]);
            pending += 1;
            p += 1;
            continue;
        };
        pending = 0;
        let s = seqs.len();
        let t = &mut ix.tables[b];
        t.out_at.push(out_at);
        t.out_len.push(seq.lits + advance);
        t.depth.push(depth);
        out_at += seq.lits + advance;
        seqs.push(seq);
        ix.add(b, s);
        p += advance;
    }
    if pending > 0 || seqs.is_empty() {
        seqs.push(Seq { lits: pending, op: 0, len: 0, arg: 0, block: 0, first: 0, out: 0 });
        let t = &mut ix.tables[b];
        t.out_at.push(out_at);
        t.out_len.push(pending);
        t.depth.push(0);
        // The last sequence is not indexed: nothing in this block follows it, and later blocks rarely start
        // with a lone run of literals.
    }
    // Sequences, with a checkpoint every k, and each sequence's body and literal offsets for later spans.
    let mut body = Vec::with_capacity(seqs.len() * 3);
    let mut checks: Vec<[u32; 3]> = Vec::new();
    let (mut o, mut li) = (0usize, 0usize);
    for (i, s) in seqs.iter().enumerate() {
        if i % k == 0 {
            checks.push([o as u32, li as u32, body.len() as u32]);
        }
        let here = body.len();
        ix.tables[b].body_at.push(here);
        ix.tables[b].lit_at.push(li);
        let nib = s.lits.min(15);
        body.push((nib << 4) as u8 | s.op);
        if nib == 15 {
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
            4 => {
                let t = &ix.tables[s.block];
                varint(&mut body, b - s.block);
                varint(&mut body, s.len - 1);
                if s.block == b {
                    varint(&mut body, here - t.body_at[s.first]);
                    varint(&mut body, li - t.lit_at[s.first]);
                } else {
                    varint(&mut body, t.body_at[s.first]);
                    varint(&mut body, t.lit_at[s.first]);
                }
                varint(&mut body, s.out);
            }
            _ => {}
        }
        o += s.lits + match s.op {
            4 => s.out,
            0 => 0,
            _ => s.len,
        };
        li += s.lits;
    }
    let mut c = Vec::with_capacity(body.len() + lits.len() + checks.len() * 12 + 16);
    for v in [data.len(), seqs.len(), lits.len(), k, checks.len()] {
        varint(&mut c, v);
    }
    for ck in &checks {
        for v in ck {
            c.extend_from_slice(&v.to_le_bytes());
        }
    }
    varint(&mut c, body.len());
    c.extend_from_slice(&body);
    c.extend_from_slice(&lits);
    c
}

/// One decoded sequence header, with where a span's target is.
struct Tok {
    lits: usize,
    op: u8,
    len: usize,
    arg: usize,
    /// Op 4: target block, body offset, literal offset, output length.
    block: usize,
    body: usize,
    lit: usize,
    out: usize,
}

fn tok(seqs: &[u8], at: &mut usize, b: usize, li: usize) -> Tok {
    let here = *at;
    let t = seqs[*at];
    *at += 1;
    let mut lits = (t >> 4) as usize;
    if lits == 15 {
        lits += read_varint(seqs, at);
    }
    let op = t & 15;
    let mut k = Tok { lits, op, len: 0, arg: 0, block: b, body: 0, lit: 0, out: 0 };
    match op {
        1 => {
            k.len = read_varint(seqs, at) + MIN_RUN;
            k.arg = seqs[*at] as usize;
            *at += 1;
        }
        2 => {
            k.len = read_varint(seqs, at) + MIN_MATCH;
            k.arg = read_varint(seqs, at);
        }
        4 => {
            let back = read_varint(seqs, at);
            k.len = read_varint(seqs, at) + 1;
            let (body, lit) = (read_varint(seqs, at), read_varint(seqs, at));
            k.out = read_varint(seqs, at);
            k.block = b - back;
            (k.body, k.lit) = if back == 0 { (here - body, li - lit) } else { (body, lit) };
        }
        _ => {}
    }
    k
}

/// Re-runs `n` sequences of block `b` from body offset `at`, literal offset `li`, passing each byte to `put`.
/// The stack is the depth bound: a span inside a span pushes one entry.
fn expand(parts: &[lp::Parts], dict: &[u8], b: usize, at: usize, li: usize, n: usize, put: &mut dyn FnMut(u8)) {
    let p = &parts[b];
    let (mut at, mut li) = (at, li);
    for _ in 0..n {
        let k = tok(p.seqs, &mut at, b, li);
        for &c in &p.lits[li..li + k.lits] {
            put(c);
        }
        match k.op {
            1 => (0..k.len).for_each(|_| put(k.arg as u8)),
            2 => (0..k.len).for_each(|j| put(pool_at(dict, p.lits, k.arg + j))),
            4 => expand(parts, dict, k.block, k.body, k.lit, k.len, put),
            _ => {}
        }
        li += k.lits;
    }
}

/// Block `b` of a file, decoded from the file's compressed blocks.
pub fn decompress(blocks: &[Vec<u8>], b: usize, dict: &Dict) -> Vec<u8> {
    let parts: Vec<lp::Parts> = blocks[..=b].iter().map(|c| lp::parts(c)).collect();
    let mut out = Vec::with_capacity(parts[b].len);
    expand(&parts, &dict.bytes, b, 0, 0, parts[b].nseq, &mut |c| out.push(c));
    out
}

/// `len` bytes at `offset` of block `b`: its checkpoint, then at most K sequences. A span that ends before
/// `offset` is skipped by its stored length; one that overlaps is expanded.
pub fn read_at(blocks: &[Vec<u8>], b: usize, dict: &Dict, offset: usize, len: usize, out: &mut Vec<u8>) {
    out.clear();
    let p = lp::parts(&blocks[b]);
    let ck = match p.checks.binary_search_by(|c| (c[0] as usize).cmp(&offset)) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    let [o, li, at] = p.checks[ck];
    let (mut o, mut li, mut at) = (o as usize, li as usize, at as usize);
    let end = offset + len;
    let mut i = ck * p.k;
    let mut parts: Option<Vec<lp::Parts>> = None;
    while o < end && i < p.nseq {
        let k = tok(p.seqs, &mut at, b, li);
        let q = std::cell::Cell::new(o);
        let mut put = |c: u8| {
            let x = q.get();
            if x >= offset && x < end {
                out.push(c);
            }
            q.set(x + 1);
        };
        for &c in &p.lits[li..li + k.lits] {
            put(c);
        }
        match k.op {
            1 => (0..k.len).for_each(|_| put(k.arg as u8)),
            2 => (0..k.len).for_each(|j| put(pool_at(&dict.bytes, p.lits, k.arg + j))),
            4 => {
                if q.get() + k.out <= offset {
                    q.set(q.get() + k.out);
                } else {
                    let ps = parts.get_or_insert_with(|| blocks[..=b].iter().map(|c| lp::parts(c)).collect());
                    expand(ps, &dict.bytes, k.block, k.body, k.lit, k.len, &mut put);
                }
            }
            _ => {}
        }
        o = q.get();
        li += k.lits;
        i += 1;
    }
}
