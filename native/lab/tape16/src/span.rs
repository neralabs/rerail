//! Span LZ: copies point at compressed input.
//!
//! A copy here names a run of earlier *compressed* sequences: in this block, or in any earlier block of the
//! file. Decoding it re-runs those sequences from their compressed bytes. That is the user's "LZ4 that
//! copies compressed input blocks to the output". Decoding needs only compressed input, which is all
//! resident, and never another thread's output. So every checkpoint segment of every block decodes in
//! parallel, and a block may still share whole runs with the blocks before it.
//!
//! A sequence is literal-pool LZ's (0156 §2), plus the span ops. How a span names its target is an axis
//! ([`Ptr`]):
//!
//! ```text
//! Full        op 4  back, n − 1, body, lit, out        (back 0: this block, body and lit as distances)
//! Split       op 4  n − 1, body dist, lit dist, out    this block
//!             op 5  back − 1, n − 1, body, lit, out    the block `back` before this one
//! SplitNoLen  Split without `out`
//! Seq         op 4  dist − 1, n − 1                    this block, the sequence `dist` back
//!             op 5  back − 1, seq, n − 1               that block's sequence `seq`
//! ```
//!
//! `out` is the span's output length, which lets a reader skip a span before the bytes it wants without
//! expanding it. Under `Seq` a decoder finds the target's body and literal offsets from the target's
//! checkpoint, walking at most K − 1 sequence headers.
//!
//! A span may name spans in turn, down to `depth` levels. The decoder therefore keeps a stack of at most
//! `depth` entries, never an unbounded one. Ops 1 and 2 in a re-run sequence read their own block's literal
//! pool (the dictionary, then that block's literals).
//!
//! The block layout is literal-pool LZ's (`lp::parts`): header, checkpoints every K sequences,
//! sequences, literals. The decoder reads blocks through [`Src`]: plain, or entropy-coded (`huff`) and
//! decoded one checkpoint segment at a time, as a read reaches it.

use std::borrow::Cow;

use crate::huff;
use crate::lp::{self, CHAIN, Dict, MIN_MATCH, MIN_RUN, NIL, hash4, pool_at, read_varint, varint};

/// How a span names its target (see the module's table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ptr {
    Full,
    Split,
    SplitNoLen,
    Seq,
}

impl Ptr {
    pub const ALL: [Ptr; 4] = [Ptr::Full, Ptr::Split, Ptr::SplitNoLen, Ptr::Seq];
    pub fn name(self) -> &'static str {
        match self {
            Ptr::Full => "",
            Ptr::Split => "-split",
            Ptr::SplitNoLen => "-nolen",
            Ptr::Seq => "-seq",
        }
    }
    /// The code the GPU kernel takes.
    pub fn code(self) -> u32 {
        self as u32
    }
}

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
pub fn compress_file(blocks: &[Vec<u8>], dict: &Dict, k: usize, depth: u8, cross: bool, ptr: Ptr) -> Vec<Vec<u8>> {
    let mut ix = Index { data: Vec::new(), tables: Vec::new(), head: vec![NIL; 1 << SPAN_HASH_BITS], prev: Vec::new(), at: Vec::new() };
    let mut out = Vec::with_capacity(blocks.len());
    for (b, data) in blocks.iter().enumerate() {
        if !cross {
            ix.head.fill(NIL);
        }
        ix.data.push(data);
        ix.tables.push(Table::default());
        out.push(compress_block(b, &mut ix, dict, k, depth, ptr));
    }
    out
}

fn compress_block(b: usize, ix: &mut Index, dict: &Dict, k: usize, max_depth: u8, ptr: Ptr) -> Vec<u8> {
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
        // Op 5 is a span into an earlier block, under the split and sequence forms.
        let op = if s.op == 4 && s.block != b && ptr != Ptr::Full { 5 } else { s.op };
        body.push((nib << 4) as u8 | op);
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
                let same = s.block == b;
                let (body_ref, lit_ref) =
                    if same { (here - t.body_at[s.first], li - t.lit_at[s.first]) } else { (t.body_at[s.first], t.lit_at[s.first]) };
                match ptr {
                    Ptr::Full => {
                        for v in [b - s.block, s.len - 1, body_ref, lit_ref, s.out] {
                            varint(&mut body, v);
                        }
                    }
                    Ptr::Split | Ptr::SplitNoLen => {
                        if !same {
                            varint(&mut body, b - s.block - 1);
                        }
                        for v in [s.len - 1, body_ref, lit_ref] {
                            varint(&mut body, v);
                        }
                        if ptr == Ptr::Split {
                            varint(&mut body, s.out);
                        }
                    }
                    Ptr::Seq => {
                        if same {
                            varint(&mut body, i - s.first - 1);
                        } else {
                            varint(&mut body, b - s.block - 1);
                            varint(&mut body, s.first);
                        }
                        varint(&mut body, s.len - 1);
                    }
                }
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

/// A block as the decoder reads it: plain, or entropy-coded and decoded one checkpoint segment at a time,
/// as reads reach it.
pub struct Src<'a> {
    pub len: usize,
    pub nseq: usize,
    pub k: usize,
    /// Output offset, literal offset, sequence byte offset.
    checks: Vec<[u32; 3]>,
    seqs: Cow<'a, [u8]>,
    lits: Cow<'a, [u8]>,
    lazy: Option<Lazy<'a>>,
    /// Each sequence's body and literal offsets, recorded as a whole-block decode passes them, so a
    /// sequence-index span finds its target without walking from the checkpoint.
    index: Vec<[u32; 2]>,
}

struct Lazy<'a> {
    coded: huff::Coded<'a>,
    tables: &'a (huff::Table, huff::Table),
    seq_done: Vec<bool>,
    lit_done: Vec<bool>,
}

impl<'a> Src<'a> {
    pub fn plain(c: &'a [u8]) -> Self {
        let p = lp::parts(c);
        Src { len: p.len, nseq: p.nseq, k: p.k, checks: p.checks, seqs: Cow::Borrowed(p.seqs), lits: Cow::Borrowed(p.lits), lazy: None, index: Vec::new() }
    }

    /// An entropy-coded block (`huff::encode`), nothing decoded yet.
    pub fn coded(c: &'a [u8], tables: &'a (huff::Table, huff::Table)) -> Self {
        let coded = huff::coded(c);
        let n = coded.checks.len();
        Src {
            len: coded.len,
            nseq: coded.nseq,
            k: coded.k,
            checks: coded.checks.iter().map(|c| [c[0], c[1], c[2]]).collect(),
            seqs: Cow::Owned(vec![0; coded.nseq_bytes]),
            lits: Cow::Owned(vec![0; coded.nlit]),
            lazy: Some(Lazy { coded, tables, seq_done: vec![false; n], lit_done: vec![false; n] }),
            index: Vec::new(),
        }
    }

    /// Checkpoint segments decoded so far (sequence and literal halves), for a reader's cost.
    pub fn decoded_segments(&self) -> usize {
        self.lazy.as_ref().map_or(0, |l| l.seq_done.iter().chain(&l.lit_done).filter(|&&d| d).count())
    }

    fn ensure_seq(&mut self, at: usize) {
        let Some(l) = &mut self.lazy else { return };
        let i = self.checks.partition_point(|c| c[2] as usize <= at).saturating_sub(1);
        if l.seq_done[i] {
            return;
        }
        l.seq_done[i] = true;
        let lo = self.checks[i][2] as usize;
        let hi = self.checks.get(i + 1).map_or(l.coded.nseq_bytes, |c| c[2] as usize);
        let mut buf = Vec::with_capacity(hi - lo);
        huff::decode(l.coded.seq_bits, u64::from(l.coded.checks[i][3]), hi - lo, &l.tables.0, &mut buf);
        self.seqs.to_mut()[lo..hi].copy_from_slice(&buf);
    }

    fn ensure_lits(&mut self, lo: usize, hi: usize) {
        let Some(l) = &mut self.lazy else { return };
        if lo >= hi {
            return;
        }
        let mut i = self.checks.partition_point(|c| c[1] as usize <= lo).saturating_sub(1);
        while i < self.checks.len() && (self.checks[i][1] as usize) < hi {
            if !l.lit_done[i] {
                l.lit_done[i] = true;
                let a = self.checks[i][1] as usize;
                let z = self.checks.get(i + 1).map_or(l.coded.nlit, |c| c[1] as usize);
                let mut buf = Vec::with_capacity(z - a);
                huff::decode(l.coded.lit_bits, u64::from(l.coded.checks[i][4]), z - a, &l.tables.1, &mut buf);
                self.lits.to_mut()[a..z].copy_from_slice(&buf);
            }
            i += 1;
        }
    }
}

/// Plain sources for a file's compressed blocks.
pub fn sources(blocks: &[Vec<u8>]) -> Vec<Src<'_>> {
    blocks.iter().map(|c| Src::plain(c)).collect()
}

/// Lazy sources for a file's entropy-coded blocks.
pub fn coded_sources<'a>(blocks: &'a [Vec<u8>], tables: &'a (huff::Table, huff::Table)) -> Vec<Src<'a>> {
    blocks.iter().map(|c| Src::coded(c, tables)).collect()
}

/// One decoded sequence header, with where a span's target is.
struct Tok {
    lits: usize,
    op: u8,
    len: usize,
    arg: usize,
    /// A span: its target block, and its first sequence either by offsets or (`Seq`) by index.
    block: usize,
    body: usize,
    lit: usize,
    seq: Option<usize>,
    out: Option<usize>,
}

/// The header at `at` of block `b`, whose literals start at `li` and whose index is `si`.
fn tok(seqs: &[u8], at: &mut usize, b: usize, li: usize, si: usize, ptr: Ptr) -> Tok {
    let here = *at;
    let t = seqs[*at];
    *at += 1;
    let mut lits = (t >> 4) as usize;
    if lits == 15 {
        lits += read_varint(seqs, at);
    }
    let op = t & 15;
    let mut k = Tok { lits, op, len: 0, arg: 0, block: b, body: 0, lit: 0, seq: None, out: None };
    let mut v = || read_varint(seqs, at);
    match (op, ptr) {
        (1, _) => {
            k.len = v() + MIN_RUN;
            k.arg = seqs[*at] as usize;
            *at += 1;
        }
        (2, _) => {
            k.len = v() + MIN_MATCH;
            k.arg = v();
        }
        (4, Ptr::Full) => {
            let back = v();
            k.len = v() + 1;
            let (body, lit) = (v(), v());
            k.out = Some(v());
            k.block = b - back;
            (k.body, k.lit) = if back == 0 { (here - body, li - lit) } else { (body, lit) };
        }
        (4 | 5, Ptr::Split | Ptr::SplitNoLen) => {
            let back = if op == 5 { v() + 1 } else { 0 };
            k.len = v() + 1;
            let (body, lit) = (v(), v());
            if ptr == Ptr::Split {
                k.out = Some(v());
            }
            k.block = b - back;
            (k.body, k.lit) = if back == 0 { (here - body, li - lit) } else { (body, lit) };
        }
        (4, Ptr::Seq) => {
            k.seq = Some(si - (v() + 1));
            k.len = v() + 1;
        }
        (5, Ptr::Seq) => {
            k.block = b - (v() + 1);
            k.seq = Some(v());
            k.len = v() + 1;
        }
        _ => {}
    }
    k
}

/// The body and literal offsets of sequence `s` of block `b`: from its checkpoint, at most K − 1 headers on.
fn locate(srcs: &mut [Src], b: usize, s: usize, ptr: Ptr) -> (usize, usize) {
    if let Some(&[at, li]) = srcs[b].index.get(s) {
        return (at as usize, li as usize);
    }
    let ck = s / srcs[b].k;
    let [_, li, at] = srcs[b].checks[ck];
    let (mut li, mut at) = (li as usize, at as usize);
    for i in ck * srcs[b].k..s {
        srcs[b].ensure_seq(at);
        let k = tok(&srcs[b].seqs, &mut at, b, li, i, ptr);
        li += k.lits;
    }
    (at, li)
}

/// Re-runs `n` sequences of block `b` from body offset `at`, literal offset `li` and sequence index `si`,
/// appending their bytes to `out`. The recursion is the depth bound: a span inside a span is one level.
#[allow(clippy::too_many_arguments)]
fn expand(srcs: &mut [Src], dict: &[u8], b: usize, at: usize, li: usize, si: usize, n: usize, ptr: Ptr, out: &mut Vec<u8>) {
    let (mut at, mut li) = (at, li);
    for si in si..si + n {
        srcs[b].ensure_seq(at);
        let k = tok(&srcs[b].seqs, &mut at, b, li, si, ptr);
        step(srcs, dict, b, li, &k, ptr, out);
        li += k.lits;
    }
}

/// One sequence's literals and op.
fn step(srcs: &mut [Src], dict: &[u8], b: usize, li: usize, k: &Tok, ptr: Ptr, out: &mut Vec<u8>) {
    srcs[b].ensure_lits(li, li + k.lits);
    out.extend_from_slice(&srcs[b].lits[li..li + k.lits]);
    match k.op {
        1 => out.resize(out.len() + k.len, k.arg as u8),
        2 => {
            let (a, z) = (k.arg, k.arg + k.len);
            let d = dict.len();
            if a < d {
                out.extend_from_slice(&dict[a..z.min(d)]);
            }
            if z > d {
                let (la, lz) = (a.max(d) - d, z - d);
                srcs[b].ensure_lits(la, lz);
                out.extend_from_slice(&srcs[b].lits[la..lz]);
            }
        }
        4 | 5 => {
            let (body, lit, seq) = match k.seq {
                Some(s) => {
                    let (at, li) = locate(srcs, k.block, s, ptr);
                    (at, li, s)
                }
                // The offset forms need no index: a nested `Seq` span never occurs under them.
                None => (k.body, k.lit, 0),
            };
            expand(srcs, dict, k.block, body, lit, seq, k.len, ptr, out);
        }
        _ => {}
    }
}

/// Block `b` of a file, decoded from the file's sources. Under `Seq` it records each sequence's offsets as
/// it passes them (`Src::index`), for later spans into this block.
pub fn decode(srcs: &mut [Src], b: usize, dict: &Dict, ptr: Ptr) -> Vec<u8> {
    let mut out = Vec::with_capacity(srcs[b].len);
    let n = srcs[b].nseq;
    if ptr != Ptr::Seq {
        expand(srcs, &dict.bytes, b, 0, 0, 0, n, ptr, &mut out);
        return out;
    }
    let record = srcs[b].index.is_empty();
    let (mut at, mut li) = (0usize, 0usize);
    for si in 0..n {
        if record {
            srcs[b].index.push([at as u32, li as u32]);
        }
        srcs[b].ensure_seq(at);
        let k = tok(&srcs[b].seqs, &mut at, b, li, si, ptr);
        step(srcs, &dict.bytes, b, li, &k, ptr, &mut out);
        li += k.lits;
    }
    out
}

/// Every block of a file in order, sharing the sources (and so the recorded offsets).
pub fn decode_file(srcs: &mut [Src], dict: &Dict, ptr: Ptr) -> Vec<Vec<u8>> {
    (0..srcs.len()).map(|b| decode(srcs, b, dict, ptr)).collect()
}

/// Block `b` of a file, decoded from the file's compressed blocks.
pub fn decompress(blocks: &[Vec<u8>], b: usize, dict: &Dict, ptr: Ptr) -> Vec<u8> {
    decode(&mut sources(&blocks[..=b]), b, dict, ptr)
}

/// `len` bytes at `offset` of block `b`: its checkpoint, then at most K sequences. A span that ends before
/// `offset` is skipped by its stored length where the form keeps one, else expanded.
#[allow(clippy::too_many_arguments)]
pub fn read(srcs: &mut [Src], b: usize, dict: &Dict, ptr: Ptr, offset: usize, len: usize, out: &mut Vec<u8>) {
    out.clear();
    let ck = srcs[b].checks.partition_point(|c| c[0] as usize <= offset) - 1;
    let [o0, li, at] = srcs[b].checks[ck];
    let (o0, mut li, mut at) = (o0 as usize, li as usize, at as usize);
    let end = offset + len;
    let (mut skipped, mut si) = (0usize, ck * srcs[b].k);
    let mut scratch = Vec::new();
    while o0 + skipped + scratch.len() < end && si < srcs[b].nseq {
        srcs[b].ensure_seq(at);
        let k = tok(&srcs[b].seqs, &mut at, b, li, si, ptr);
        match k.out {
            Some(n) if matches!(k.op, 4 | 5) && o0 + skipped + scratch.len() + k.lits + n <= offset => {
                srcs[b].ensure_lits(li, li + k.lits);
                scratch.extend_from_slice(&srcs[b].lits[li..li + k.lits]);
                skipped += n;
            }
            _ => step(srcs, &dict.bytes, b, li, &k, ptr, &mut scratch),
        }
        li += k.lits;
        si += 1;
    }
    let from = offset - o0 - skipped;
    out.extend_from_slice(&scratch[from..from + len]);
}

/// `len` bytes at `offset` of block `b` of a file's compressed blocks.
#[allow(clippy::too_many_arguments)]
pub fn read_at(blocks: &[Vec<u8>], b: usize, dict: &Dict, ptr: Ptr, offset: usize, len: usize, out: &mut Vec<u8>) {
    read(&mut sources(&blocks[..=b]), b, dict, ptr, offset, len, out);
}
