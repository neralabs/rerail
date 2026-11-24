//! E19 (t39 step 11): LZ4 with random access.
//!
//! Each in-place array of a file (E18's mixed and byte-aligned forms) is cut into fixed blocks of N nodes, so
//! a node's block is `i / N` and its bytes are at a fixed offset in it. Every block is stored one of three
//! ways, named by a 4-byte entry in the file's block index:
//!
//! - **repeat**: a pointer to an identical block stored earlier, in this file (4 bytes) or in a pool shared
//!   by the store (8 bytes: a pool id), stored once;
//! - **raw**: the bytes as they are, read in place;
//! - **lz4**: one LZ4 block (HC level 9, the reference library's block format) whose matches reach only the
//!   language's dictionary (64 KiB, trained per array on the other half of the files) and the block itself.
//!
//! So a read decodes at most one block, and a block whose stored copy is raw is read in place, never decoded.
//! A second variant lets an LZ4 block also match into one earlier block of its file that has no reference
//! itself (raw, or LZ4 over the dictionary alone): a read then decodes at most two blocks.
//! The escapes' side lists stay whole: one LZ4 block a file and array. Against it: the same arrays compressed
//! whole, file by file, with the same dictionary (no random access).

use crate::capture::File;
use crate::codec::Dictionary;
use crate::disk::{Lz4, POS_K, SIB_K, lang_widths, lz4_decode};
use crate::inplace::{Cols, Packed, bits_for};
use crate::measure::cost;
use crate::relative::{gaps, sib_slots};
use crate::repeat::{Esc, le_bytes};
use std::collections::HashMap;

pub const N: [usize; 3] = [256, 1024, 4096];
pub const FORMS: [&str; 2] = ["mixed", "byte-aligned"];
pub const STREAMS: [&str; 5] = ["codes", "slots", "gaps", "lengths", "checkpoints"];
const DICT: usize = 64 << 10;
/// An LZ4 block is kept only if it saves at least this share of the raw bytes.
const KEEP: f64 = 0.9;

/// One array of one file: its fixed-width bytes, its width, its escapes' lists, and how many nodes an entry
/// covers (1, or `POS_K` for the checkpoints).
struct Arr {
    main: Vec<u8>,
    w: u32,
    side: Vec<u8>,
    per: usize,
}

impl Arr {
    /// The bytes of block b for blocks of `n` nodes.
    fn block(&self, n: usize, b: usize) -> &[u8] {
        let bytes = (n / self.per) * self.w as usize / 8;
        let s = (b * bytes).min(self.main.len());
        &self.main[s..(s + bytes).min(self.main.len())]
    }
    fn blocks(&self, n: usize) -> usize {
        let bytes = (n / self.per) * self.w as usize / 8;
        self.main.len().div_ceil(bytes.max(1))
    }
}

fn arrays(c: &Cols, widths: [u32; 4], round: bool) -> Vec<Arr> {
    let n = c.len();
    let (gap, len) = gaps(c);
    let sib = sib_slots(c, SIB_K);
    let ckpt: Vec<u64> = c.start.iter().step_by(POS_K).copied().collect();
    let esc = |vals: &[u64], w: u32| {
        let e = Esc::new(vals, w);
        let (main, side) = e.split(vals.len());
        Arr { main, w, side, per: 1 }
    };
    // Both forms keep slots, gaps and lengths at byte widths; mixed keeps codes bit-packed at the language's
    // width, byte-aligned rounds them up too.
    let wc = if round { widths[0].next_multiple_of(8) } else { widths[0] };
    let code = Packed::new_width(&c.code, wc);
    let ck_w = ckpt.iter().map(|&v| bits_for(v)).max().unwrap_or(0).max(1).next_multiple_of(8);
    let ck = Packed::new_width(&ckpt, ck_w);
    vec![
        Arr { main: le_bytes(&code.words, (n * wc as usize).div_ceil(8)), w: wc, side: Vec::new(), per: 1 },
        esc(&sib, widths[1].next_multiple_of(8)),
        esc(&gap, widths[2].next_multiple_of(8)),
        esc(&len, widths[3].next_multiple_of(8)),
        Arr { main: le_bytes(&ck.words, (ckpt.len() * ck_w as usize).div_ceil(8)), w: ck_w, side: Vec::new(), per: POS_K },
    ]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    RepeatFile,
    RepeatPool,
    Raw,
    Lz4,
}

#[derive(Default, Clone)]
pub struct Cell {
    /// Bytes: blocks stored, the index, the side lists.
    pub stored: u64,
    pub index: u64,
    pub side: u64,
    /// Nodes by how their block is stored: repeat in the file, repeat from the pool, raw, LZ4.
    pub nodes: [u64; 4],
    /// Nodes whose block's stored copy is raw (read in place, never decoded).
    pub in_place: u64,
    /// Nodes whose block is LZ4 over a reference block (a read decodes two).
    pub with_ref: u64,
    pub whole: u64,
    pub decode_block_ns: f64,
    pub decode_all_ns: u64,
    pub wrong: u64,
}

pub struct Report {
    pub files: usize,
    pub measured: usize,
    pub nodes: u64,
    /// `cells[form][n]`, summed over the arrays; `by_array[form][n][array]` the stored bytes of each.
    /// `[form][n][refs]`.
    pub cells: Vec<Vec<[Cell; 2]>>,
    pub by_array: Vec<Vec<[[u64; 5]; 2]>>,
}

pub fn run(files: &[&File]) -> Report {
    let (codes, widths) = lang_widths(files);
    let read = |f: &File| crate::captured::read(&f.image).unwrap_or_else(|e| panic!("{}: {e}", f.path));
    let dicts = files.len() >= 16;
    let (mut train, mut test): (Vec<[Vec<Arr>; 2]>, Vec<[Vec<Arr>; 2]>) = (Vec::new(), Vec::new());
    let mut nodes = 0u64;
    let mut node_counts = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let c = Cols::of_tree(&read(f), &codes);
        let a = [arrays(&c, widths, false), arrays(&c, widths, true)];
        if dicts && i % 2 == 0 {
            train.push(a);
        } else {
            nodes += c.len() as u64;
            node_counts.push(c.len());
            test.push(a);
        }
    }
    // A dictionary per form and array, trained on 1,024-node blocks of the other files (at most 8 MiB).
    let dict: Vec<Vec<Dictionary>> = (0..2).map(|form| (0..5).map(|q| {
        if !dicts {
            return Dictionary::none();
        }
        let mut samples = Vec::new();
        let mut total = 0;
        'files: for a in &train {
            let arr = &a[form][q];
            for b in 0..arr.blocks(1024) {
                if total > 8 << 20 {
                    break 'files;
                }
                let blk = arr.block(1024, b).to_vec();
                total += blk.len();
                samples.push(blk);
            }
        }
        Dictionary::train(&samples, DICT)
    }).collect()).collect();
    drop(train);
    let mut cells = vec![vec![[Cell::default(), Cell::default()]; N.len()]; 2];
    let mut by_array = vec![vec![[[0u64; 5]; 2]; N.len()]; 2];
    for form in 0..2 {
        for (ni, &n) in N.iter().enumerate() {
            for refs in [false, true] {
                let (cell, arr_bytes) = (&mut cells[form][ni][refs as usize], &mut by_array[form][ni][refs as usize]);
                let mut lz_total = 0usize;
                for q in 0..5 {
                    one_array(&test, &node_counts, form, q, n, refs, &dict[form][q].bytes, cell, &mut arr_bytes[q], &mut lz_total);
                }
                cell.decode_block_ns /= lz_total.max(1) as f64;
            }
        }
    }
    Report { files: files.len(), measured: test.len(), nodes, cells, by_array }
}

/// How a block's copy is stored.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Copy {
    Raw,
    /// LZ4 over the dictionary alone.
    Root,
    /// LZ4 over the dictionary and one earlier block of the file (a root or raw), named by its index.
    Ref(usize),
}

struct Stored {
    c: Vec<u8>,
    raw: Vec<u8>,
    copy: Copy,
}

/// The dictionary tail a block over a reference sees before the reference, for blocks of `bytes`: the
/// language dictionary's last 64 KiB − `bytes` (LZ4 reads at most 64 KiB back).
fn tail(dict: &[u8], bytes: usize) -> &[u8] {
    &dict[dict.len() - (64usize << 10).saturating_sub(bytes).min(dict.len())..]
}

/// The reference dictionary of a block over reference `r` (a full block): the tail, then `r`.
fn ref_dict(dict: &[u8], r: &[u8]) -> Vec<u8> {
    let mut d = tail(dict, r.len()).to_vec();
    d.extend_from_slice(r);
    d
}

/// A reader's buffer for blocks over a reference: `[tail][reference][output]`, the tail written once, so a
/// read writes the reference (decoded or copied) and decodes right after it, with no dictionary copied.
struct Reader {
    buf: Vec<u8>,
    tail: usize,
    bytes: usize,
}

impl Reader {
    fn new(dict: &[u8], bytes: usize) -> Reader {
        let t = tail(dict, bytes);
        let mut buf = t.to_vec();
        buf.resize(t.len() + 2 * bytes, 0);
        Reader { buf, tail: t.len(), bytes }
    }
}

/// Decodes stored block `i` (its reference first) as a random read would; the bytes land in `out`, or for a
/// block over a reference in the reader's output slot. Returns the decoded length.
fn decode(stored: &[Stored], i: usize, dict: &[u8], rd: &mut Reader, out: &mut [u8]) -> usize {
    let s = &stored[i];
    match s.copy {
        Copy::Raw => {
            out[..s.raw.len()].copy_from_slice(&s.raw);
            s.raw.len()
        }
        Copy::Root => lz4_decode(&s.c, &mut out[..s.raw.len()], dict),
        Copy::Ref(r) => {
            let t = &stored[r];
            // Blocks of one array have one length within a file (the checkpoints' width is the file's).
            if rd.bytes != t.raw.len() {
                *rd = Reader::new(dict, t.raw.len());
            }
            let (head, slot_out) = rd.buf.split_at_mut(rd.tail + rd.bytes);
            let slot = &mut head[rd.tail..];
            match t.copy {
                Copy::Raw => slot.copy_from_slice(&t.raw),
                _ => {
                    lz4_decode(&t.c, slot, dict);
                }
            }
            lz4_decode(&s.c, &mut slot_out[..s.raw.len()], head)
        }
    }
}

/// The bytes of a decode, wherever `decode` left them.
fn decoded<'a>(stored: &[Stored], i: usize, rd: &'a Reader, out: &'a [u8], k: usize) -> &'a [u8] {
    match stored[i].copy {
        Copy::Ref(_) => &rd.buf[rd.tail + rd.bytes..rd.tail + rd.bytes + k],
        _ => &out[..k],
    }
}

#[allow(clippy::too_many_arguments)]
fn one_array(test: &[[Vec<Arr>; 2]], node_counts: &[usize], form: usize, q: usize, n: usize, refs: bool, d: &[u8], cell: &mut Cell, arr_bytes: &mut u64, lz_total: &mut usize) {
    // One compressor over the language's dictionary, one for blocks over a reference (re-indexed per block).
    let mut lz = Lz4::new(true, d);
    let mut lzr = Lz4::new(true, &[]);
    // The store's pool: every block stored so far, by content, and whether its copy is read in place.
    let mut pool: HashMap<[u8; 32], bool> = HashMap::new();
    let mut stored: Vec<Stored> = Vec::new();
    let mut lz_blocks: Vec<usize> = Vec::new();
    for (t, a) in test.iter().enumerate() {
        let arr = &a[form][q];
        let full = (n / arr.per) * arr.w as usize / 8;
        let mut own: HashMap<[u8; 32], bool> = HashMap::new();
        // The array whole, for comparison (no random access).
        cell.whole += lz.compress(&arr.main).len() as u64;
        // The file's latest block stored without a reference (a reference must not need another).
        let mut last_root: Option<usize> = None;
        for b in 0..arr.blocks(n) {
            let blk = arr.block(n, b);
            let covers = ((b + 1) * n).min(node_counts[t]).saturating_sub(b * n) as u64;
            let h = *blake3::hash(blk).as_bytes();
            let kind = if let Some(&raw) = own.get(&h) {
                cell.in_place += if raw { covers } else { 0 };
                Kind::RepeatFile
            } else if let Some(&raw) = pool.get(&h) {
                cell.in_place += if raw { covers } else { 0 };
                own.insert(h, raw);
                Kind::RepeatPool
            } else {
                let root = lz.compress(blk);
                let mut best = (root, Copy::Root);
                // Only a full block takes a reference, and only a full one is named (the reader's layout).
                if refs && let Some(r) = last_root && blk.len() == full && stored[r].raw.len() == full {
                    lzr.set_dict(&ref_dict(d, &stored[r].raw));
                    let c = lzr.compress(blk);
                    if (c.len() as f64) < 0.97 * best.0.len() as f64 {
                        best = (c, Copy::Ref(r));
                    }
                }
                let (c, copy) = if (best.0.len() as f64) < KEEP * blk.len() as f64 { best } else { (Vec::new(), Copy::Raw) };
                let bytes = if copy == Copy::Raw { blk.len() } else { c.len() };
                cell.stored += bytes as u64;
                *arr_bytes += bytes as u64;
                if copy == Copy::Raw {
                    cell.in_place += covers;
                } else {
                    lz_blocks.push(stored.len());
                }
                if matches!(copy, Copy::Ref(_)) {
                    // The reference: a block number in the file, two bytes in its index entry.
                    cell.index += 2;
                    cell.with_ref += covers;
                } else {
                    last_root = Some(stored.len());
                }
                stored.push(Stored { c, raw: blk.to_vec(), copy });
                pool.insert(h, copy == Copy::Raw);
                own.insert(h, copy == Copy::Raw);
                if copy == Copy::Raw { Kind::Raw } else { Kind::Lz4 }
            };
            cell.index += if kind == Kind::RepeatPool { 8 } else { 4 };
            cell.nodes[kind as usize] += covers;
        }
        if !arr.side.is_empty() {
            cell.side += lz.compress(&arr.side).len() as u64;
        }
    }
    // Every stored block decoded (timed) and checked; then blocks at random, one read each, timed.
    let mut out = vec![0u8; n * 8];
    let mut rd = Reader::new(d, 0);
    let ((), t) = cost(|| {
        for i in 0..stored.len() {
            std::hint::black_box(decode(&stored, i, d, &mut rd, &mut out));
        }
    });
    cell.decode_all_ns += t.nanos;
    for i in 0..stored.len() {
        let k = decode(&stored, i, d, &mut rd, &mut out);
        cell.wrong += u64::from(decoded(&stored, i, &rd, &out, k) != &stored[i].raw[..]);
    }
    let mut rng = 0x9e37_79b9_7f4a_7c15u64 ^ (q as u64) << 8 ^ n as u64;
    let picks: Vec<usize> = (0..lz_blocks.len().min(4000)).map(|_| {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        lz_blocks[(rng % lz_blocks.len() as u64) as usize]
    }).collect();
    if !picks.is_empty() {
        let mut best = u64::MAX;
        for _ in 0..3 {
            let ((), t) = cost(|| {
                for &p in &picks {
                    std::hint::black_box(decode(&stored, p, d, &mut rd, &mut out));
                }
            });
            best = best.min(t.nanos);
        }
        // Each array's mean, weighted by its LZ4 blocks (the share of random reads that decode).
        cell.decode_block_ns += best as f64 / picks.len() as f64 * lz_blocks.len() as f64;
        *lz_total += lz_blocks.len();
    }
}

pub fn print(title: &str, r: &Report) {
    let per = |b: u64| b as f64 / r.nodes.max(1) as f64;
    let pct = |a: u64| format!("{:.0}%", 100.0 * a as f64 / (5 * r.nodes).max(1) as f64);
    println!("### {title}\n");
    println!(
        "{} files, {} measured ({} nodes){}. Shares are of the five arrays' nodes.\n",
        r.files,
        r.measured,
        r.nodes,
        if r.measured < r.files { ", dictionaries trained on the others" } else { ", too few files for a dictionary" }
    );
    println!("| form, N nodes a block, references | B a node | of which index | side lists | whole arrays, LZ4 HC (no random access) | repeated in the file | from the pool | raw | LZ4 | of it over a reference | read in place | one LZ4 block read at random, ns | all decoded, ns a node | wrong |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for form in 0..2 {
        for (ni, &n) in N.iter().enumerate() {
            for refs in 0..2 {
                let c = &r.cells[form][ni][refs];
                println!(
                    "| {}, {n}, {} | {:.3} | {:.3} | {:.3} | {:.3} | {} | {} | {} | {} | {} | {} | {:.0} | {:.2} | {} |",
                    FORMS[form],
                    if refs == 1 { "one" } else { "none" },
                    per(c.stored + c.index + c.side),
                    per(c.index),
                    per(c.side),
                    per(c.whole + c.side),
                    pct(c.nodes[0]),
                    pct(c.nodes[1]),
                    pct(c.nodes[2]),
                    pct(c.nodes[3]),
                    pct(c.with_ref),
                    pct(c.in_place),
                    c.decode_block_ns,
                    c.decode_all_ns as f64 / r.nodes.max(1) as f64,
                    c.wrong
                );
            }
        }
    }
    println!();
    println!("| stored blocks, B a node | {} |", STREAMS.join(" | "));
    println!("|---|{}", "---|".repeat(STREAMS.len()));
    for form in 0..2 {
        for (ni, &n) in N.iter().enumerate() {
            for refs in 0..2 {
                println!("| {}, {n}, {} | {} |", FORMS[form], if refs == 1 { "one" } else { "none" }, r.by_array[form][ni][refs].iter().map(|&b| format!("{:.3}", per(b))).collect::<Vec<_>>().join(" | "));
            }
        }
    }
    println!();
}
