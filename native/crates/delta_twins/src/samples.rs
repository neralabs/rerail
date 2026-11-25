//! An operation of each shape the twins need first (Operations §13: map, emit, sort, merge, queue), small enough to
//! check by hand: the executor's tests run each at two tilings and with its tiles in shuffled order, and find the same
//! facts. Built for tests (`test-hooks`).
//!
//! Their blocks' bodies are plain: a values block is its body's length, a count `n` and `n` `i64`s; a graph is its
//! node count, edge count, `n + 1` offsets and the edges' targets.

use crate::sort::{CHUNK, chunks, merge_pairs, more, pairs, sort_chunks};
use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::gens::{BODY_AT, set_word, word, words, words_mut};

const N_AT: usize = BODY_AT + 8;
const VALS_AT: usize = BODY_AT + 16;

/// A values block's `n`.
pub fn count(b: &[u8]) -> usize {
    word::<u64>(b, N_AT) as usize
}

/// A values block's values.
pub fn vals(b: &[u8]) -> &[i64] {
    words(b, VALS_AT, count(b))
}

/// Lays a values block of `n` values out, and gives them to write.
pub fn vals_mut(b: &mut [u8], n: usize) -> &mut [i64] {
    set_word::<u64>(b, BODY_AT, (16 + 8 * n) as u64);
    set_word::<u64>(b, N_AT, n as u64);
    words_mut(b, VALS_AT, n)
}

/// The body bytes a values block of `n` values takes.
pub fn vals_bytes(n: usize) -> usize {
    16 + 8 * n
}

// ---- map: each value squared, by index -----------------------------------------------------------------------------

pub const SQUARE: &[TwinPhase] = &[TwinPhase { name: "map", items: |c| count(c.read(0)) as u32, run: square_tile, again: None }];

fn square_tile(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let ([input], [out]) = c.views([0], [1]);
    let src = vals(input);
    let dst = vals_mut(out, src.len());
    for i in t.items() {
        dst[i] = src[i] * src[i];
    }
}

// ---- emit: the even values, in order: count, scan, place ----------------------------------------------------------

pub const EVENS: &[TwinPhase] = &[
    TwinPhase { name: "count", items: |c| chunks(count(c.read(0))), run: evens_count, again: None },
    TwinPhase { name: "scan", items: |_| 1, run: evens_scan, again: None },
    TwinPhase { name: "place", items: |c| chunks(count(c.read(0))), run: evens_place, again: None },
];

/// A chunk's count, then its offset, in the work slot.
fn chunk_words(work: &mut [u8], n: usize) -> &mut [u64] {
    words_mut(work, BODY_AT + 8, chunks(n) as usize + 1)
}

fn evens_count(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let ([input], [work]) = c.views([0], [2]);
    let v = vals(input);
    let w = chunk_words(work, v.len());
    for k in t.items() {
        w[k] = v[k * CHUNK..((k + 1) * CHUNK).min(v.len())].iter().filter(|&&x| x % 2 == 0).count() as u64;
    }
}

fn evens_scan(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let ([input], [out, work]) = c.views([0], [1, 2]);
    let n = count(input);
    let w = chunk_words(work, n);
    let mut sum = 0;
    for x in w.iter_mut() {
        let k = *x;
        *x = sum;
        sum += k;
    }
    vals_mut(out, sum as usize);
}

fn evens_place(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let ([input], [out, work]) = c.views([0], [1, 2]);
    let v = vals(input);
    let w = chunk_words(work, v.len());
    let total = w[w.len() - 1] as usize;
    let at: Vec<u64> = t.items().map(|k| w[k]).collect(); // copy: not block data (a tile's offsets)
    let dst = vals_mut(out, total);
    for (k, mut o) in t.items().zip(at) {
        for &x in v[k * CHUNK..((k + 1) * CHUNK).min(v.len())].iter().filter(|&&x| x % 2 == 0) {
            dst[o as usize] = x;
            o += 1;
        }
    }
}

// ---- sort: the values ascending: chunks, then merge passes --------------------------------------------------------

pub const SORT: &[TwinPhase] = &[
    TwinPhase { name: "chunks", items: |c| chunks(count(c.read(0))), run: sort_first, again: None },
    TwinPhase { name: "passes", items: sort_pass_items, run: sort_pass, again: Some(sort_next) },
    TwinPhase { name: "settle", items: |c| count(c.read(0)) as u32, run: sort_settle, again: None },
];

/// The work slot's words: the pass's width, which array holds the runs, then the two arrays of positions.
struct SortWork<'a> {
    width: usize,
    which: usize,
    a: &'a mut [u32],
    b: &'a mut [u32],
}

fn sort_work(work: &mut [u8], n: usize) -> SortWork<'_> {
    let width = word::<u64>(work, BODY_AT + 8) as usize;
    let which = word::<u64>(work, BODY_AT + 16) as usize;
    let (a, b) = words_mut::<u32>(work, BODY_AT + 24, 2 * n).split_at_mut(n);
    SortWork { width, which, a, b }
}

fn sort_first(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let ([input], [work]) = c.views([0], [2]);
    let v = vals(input);
    set_word::<u64>(work, BODY_AT + 8, CHUNK as u64);
    set_word::<u64>(work, BODY_AT + 16, 0);
    let w = sort_work(work, v.len());
    for k in t.items() {
        for i in k * CHUNK..((k + 1) * CHUNK).min(v.len()) {
            w.a[i] = i as u32;
        }
    }
    sort_chunks(w.a, t, |x, y| v[x as usize].cmp(&v[y as usize]));
}

fn sort_pass_items(c: &Ctx<'_, '_, '_>) -> u32 {
    let n = count(c.read(0));
    let width = word::<u64>(c.read(2), BODY_AT + 8) as usize;
    if more(n, width) { pairs(n, width) } else { 0 }
}

fn sort_pass(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let ([input], [work]) = c.views([0], [2]);
    let v = vals(input);
    let w = sort_work(work, v.len());
    let cmp = |x: u32, y: u32| v[x as usize].cmp(&v[y as usize]);
    if w.which == 0 { merge_pairs(w.a, w.b, t, w.width, cmp) } else { merge_pairs(w.b, w.a, t, w.width, cmp) }
}

fn sort_next(c: &mut Ctx<'_, '_, '_>) -> bool {
    let ([input], [work]) = c.views([0], [2]);
    let n = count(input);
    let width = word::<u64>(work, BODY_AT + 8) as usize;
    if !more(n, width) {
        return false;
    }
    set_word::<u64>(work, BODY_AT + 8, 2 * width as u64);
    set_word::<u64>(work, BODY_AT + 16, 1 - word::<u64>(work, BODY_AT + 16));
    more(n, 2 * width)
}

fn sort_settle(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let ([input], [out, work]) = c.views([0], [1, 2]);
    let v = vals(input);
    let w = sort_work(work, v.len());
    let perm = if w.which == 0 { &*w.a } else { &*w.b };
    let dst = vals_mut(out, v.len());
    for i in t.items() {
        dst[i] = v[perm[i] as usize];
    }
}

// ---- merge: two sorted inputs merged, a chunk of the output an item (the merge path) ------------------------------

pub const MERGE: &[TwinPhase] = &[TwinPhase {
    name: "merge",
    items: |c| chunks(count(c.read(0)) + count(c.read(1))),
    run: merge_tile,
    again: None,
}];

/// The split of output position `k` between `a` and `b`: how many of `a` come before it (the co-rank).
fn co_rank(k: usize, a: &[i64], b: &[i64]) -> usize {
    let (mut lo, mut hi) = (k.saturating_sub(b.len()), k.min(a.len()));
    while lo < hi {
        let i = (lo + hi) / 2;
        let j = k - i;
        // `a[i]` belongs before position k when it is below `b[j - 1]` (a's ties first).
        if j > 0 && i < a.len() && a[i] <= b[j - 1] { lo = i + 1 } else { hi = i }
    }
    lo
}

fn merge_tile(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let ([a, b], [out]) = c.views([0, 1], [2]);
    let (a, b) = (vals(a), vals(b));
    let n = a.len() + b.len();
    let dst = vals_mut(out, n);
    for k in t.items() {
        let (lo, hi) = (k * CHUNK, ((k + 1) * CHUNK).min(n));
        let (mut i, mut j) = (co_rank(lo, a, b), lo - co_rank(lo, a, b));
        for d in &mut dst[lo..hi] {
            if j >= b.len() || (i < a.len() && a[i] <= b[j]) {
                *d = a[i];
                i += 1;
            } else {
                *d = b[j];
                j += 1;
            }
        }
    }
}

// ---- queue: the nodes reachable from node 0, a frontier's items a turn, pushed by claims --------------------------

pub const REACH: &[TwinPhase] = &[
    TwinPhase { name: "seed", items: |_| 1, run: reach_seed, again: None },
    TwinPhase { name: "turn", items: |c| word::<u64>(c.read(2), BODY_AT + 8) as u32, run: reach_turn, again: Some(reach_next) },
];

/// A graph's nodes, edges, offsets and targets.
pub fn graph(b: &[u8]) -> (usize, &[u64], &[u64]) {
    let n = word::<u64>(b, BODY_AT + 8) as usize;
    let m = word::<u64>(b, BODY_AT + 16) as usize;
    (n, words(b, BODY_AT + 24, n + 1), words(b, BODY_AT + 24 + 8 * (n + 1), m))
}

/// The body bytes a graph of `n` nodes and `m` edges takes.
pub fn graph_bytes(n: usize, m: usize) -> usize {
    24 + 8 * (n + 1) + 8 * m
}

/// The work slot: the current queue's length, the next's, which is current, and the two queues.
fn queues(work: &mut [u8], n: usize) -> (&mut [u64], &mut [u32], &mut [u32]) {
    let (head, rest) = work[BODY_AT + 8..].split_at_mut(24);
    let lens = words_mut::<u64>(head, 0, 3);
    let (qa, qb) = words_mut::<u32>(rest, 0, 2 * n).split_at_mut(n);
    (lens, qa, qb)
}

fn reach_seed(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let ([g], [out, work]) = c.views([0], [1, 2]);
    let (n, _, _) = graph(g);
    let visited = vals_mut(out, n);
    visited[0] = 1;
    let (lens, qa, _) = queues(work, n);
    qa[0] = 0;
    lens[0] = 1;
    lens[1] = 0;
    lens[2] = 0;
}

fn reach_turn(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let ([g], [out, work]) = c.views([0], [1, 2]);
    let (n, offsets, targets) = graph(g);
    let visited = vals_mut(out, n);
    let (lens, qa, qb) = queues(work, n);
    let (cur, next) = if lens[2] == 0 { (qa, qb) } else { (qb, qa) };
    for i in t.items() {
        let u = cur[i] as usize;
        for &v in &targets[offsets[u] as usize..offsets[u + 1] as usize] {
            // In place: marked once (a CAS on a GPU), and pushed by its claim of the next queue's place.
            if visited[v as usize] == 0 {
                visited[v as usize] = 1;
                next[lens[1] as usize] = v as u32;
                lens[1] += 1;
            }
        }
    }
}

fn reach_next(c: &mut Ctx<'_, '_, '_>) -> bool {
    let ([g], [work]) = c.views([0], [2]);
    let (n, _, _) = graph(g);
    let (lens, _, _) = queues(work, n);
    lens[0] = lens[1];
    lens[1] = 0;
    lens[2] ^= 1;
    lens[0] > 0
}
