//! E12 (t39): the tree primitives rules ask for, answered on each candidate tape in place, in two forms:
//!
//! - **bulk**: the answer for every node of a file in one pass, as the engine evaluates a rule a set at a
//!   time (`nearest`, `inside`, `has` are "one tape pass per axis and set", core.rr);
//! - **point**: one node's answer, at random.
//!
//! Kind sets are named as rules name them (`kinds.tsv`, from `rerail-engine kind-names`), resolved per
//! language to a mask over the language's codes. Nothing is stored beside the tape: each answer is read
//! from the reader's own questions (`inplace::Reader`). Every answer is checked against the tree.

use crate::inplace::{Codes, Reader};
use crate::tree::{NONE, Tree};
use std::collections::HashMap;

/// `kinds.tsv`: `tag, id, name, named` for kinds, `tag, "field id", name` for fields.
pub struct Names {
    kinds: HashMap<(u8, String), Vec<u16>>,
    fields: HashMap<(u8, String), u8>,
}

impl Names {
    pub fn load() -> Names {
        let (mut kinds, mut fields) = (HashMap::<(u8, String), Vec<u16>>::new(), HashMap::new());
        for line in include_str!("../kinds.tsv").lines() {
            let c: Vec<&str> = line.split('\t').collect();
            let Ok(tag) = c[0].parse::<u8>() else { continue };
            if let Some(id) = c[1].strip_prefix("field ") {
                fields.insert((tag, c[2].to_string()), id.parse().unwrap());
            } else if c.get(3) == Some(&"1") {
                kinds.entry((tag, c[2].to_string())).or_default().push(c[1].parse().unwrap());
            }
        }
        Names { kinds, fields }
    }
    pub fn kind_ids(&self, tag: u8, names: &[&str]) -> Vec<u16> {
        names.iter().flat_map(|n| self.kinds.get(&(tag, n.to_string())).cloned().unwrap_or_default()).collect()
    }
    pub fn field(&self, tag: u8, name: &str) -> Option<u8> {
        self.fields.get(&(tag, name.to_string())).copied()
    }
    /// The language's codes whose kind is one of `names`.
    pub fn set(&self, tag: u8, codes: &Codes, names: &[&str]) -> Vec<bool> {
        let ids: Vec<u16> = names.iter().flat_map(|n| self.kinds.get(&(tag, n.to_string())).cloned().unwrap_or_default()).collect();
        codes.table.iter().map(|t| ids.contains(&t.0)).collect()
    }
}

/// The sets measured, as the rules write them (functions, classes, blocks, calls, strings; wrappers).
pub const SETS: [(&str, &[&str]); 5] = [
    ("fn", &["function_definition", "lambda", "function_declaration", "function_expression", "arrow_function", "method_definition", "generator_function_declaration", "function_item", "closure_expression"]),
    ("class", &["class_definition", "class_declaration", "class", "impl_item", "struct_item", "trait_item"]),
    ("block", &["block", "statement_block"]),
    ("call", &["call", "call_expression"]),
    ("string", &["string", "string_literal", "template_string"]),
];
pub const WRAP: &[&str] = &["parenthesized_expression"];

/// nearest(n, a, S) for every node: a is n's nearest strict ancestor in S (`NONE`: none), by preorder.
/// One sweep with a stack of the open S-nodes; only an S-node's subtree end is read.
pub fn nearest_all<R: Reader>(r: &R, set: &[bool], out: &mut Vec<u32>) {
    out.clear();
    let mut stack: Vec<(u32, usize)> = Vec::with_capacity(32);
    let mut id = r.id_of(0);
    let mut p = 0u32;
    loop {
        while stack.last().is_some_and(|&(_, end)| end <= id) {
            stack.pop();
        }
        out.push(stack.last().map_or(NONE, |&(a, _)| a));
        if set[r.code(id) as usize] {
            stack.push((p, r.subtree_end(id)));
        }
        p += 1;
        match r.next(id) {
            Some(n) => id = n,
            None => break,
        }
    }
}

/// `nearest_all` (or, with `outer`, `outermost_all`) from the S-nodes alone: the codes are matched 64 at a
/// time, and only an S-node's subtree end is read. Between two events (an S-node, the end of an open one) the
/// answer is constant, so it is written as a run.
pub fn axis_groups<R: Reader>(r: &R, set: &[bool], outer: bool, out: &mut Vec<u32>) {
    let n = r.len();
    out.clear();
    let mut stack: Vec<(u32, usize)> = Vec::with_capacity(32);
    // Writes every node's answer up to rank t.
    let fill = |out: &mut Vec<u32>, stack: &mut Vec<(u32, usize)>, t: usize| {
        while out.len() < t {
            while stack.last().is_some_and(|&(_, end)| end <= out.len()) {
                stack.pop();
            }
            let (v, lim) = match stack.last() {
                Some(&(a, end)) => (if outer { stack[0].0 } else { a }, end.min(t)),
                None => (NONE, t),
            };
            out.resize(lim, v);
        }
    };
    let mut on = |s: usize| {
        fill(out, &mut stack, s + 1);
        let id = r.id_of(s);
        let end = r.subtree_end(id);
        stack.push((s as u32, if R::PREORDER_IDS { end } else { r.rank(end) }));
    };
    match r.packed_codes() {
        Some(p) => p.match_groups(n, set, |base, mut m| {
            while m != 0 {
                on(base + m.trailing_zeros() as usize);
                m &= m - 1;
            }
        }),
        None => (0..n).filter(|&k| set[r.code_by_rank(k) as usize]).for_each(&mut on),
    }
    fill(out, &mut stack, n);
}

/// The outermost strict ancestor in S, for every node: the bottom of the same stack.
pub fn outermost_all<R: Reader>(r: &R, set: &[bool], out: &mut Vec<u32>) {
    out.clear();
    let mut stack: Vec<(u32, usize)> = Vec::with_capacity(32);
    let mut id = r.id_of(0);
    let mut p = 0u32;
    loop {
        while stack.last().is_some_and(|&(_, end)| end <= id) {
            stack.pop();
        }
        out.push(stack.first().map_or(NONE, |&(a, _)| a));
        if set[r.code(id) as usize] {
            stack.push((p, r.subtree_end(id)));
        }
        p += 1;
        match r.next(id) {
            Some(n) => id = n,
            None => break,
        }
    }
}

/// has(a, _, S) for every node: some strict descendant of a is in S. One sweep keeping, for each open node,
/// the preorder number of the last S-node seen: a node has one if an S-node came after it before its end.
/// (Forward, so B's ids need no reverse step: the stack holds the open nodes and their first number.)
pub fn has_all<R: Reader>(r: &R, set: &[bool], out: &mut Vec<bool>) {
    out.clear();
    let mut stack: Vec<(u32, usize)> = Vec::with_capacity(64);
    let mut id = r.id_of(0);
    let mut p = 0u32;
    // The preorder number of the last S-node seen so far (u32::MAX: none).
    let mut last = u32::MAX;
    loop {
        while stack.last().is_some_and(|&(_, end)| end <= id) {
            let (a, _) = stack.pop().unwrap();
            out[a as usize] = last != u32::MAX && last > a;
        }
        out.push(false);
        if set[r.code(id) as usize] {
            last = p;
        }
        stack.push((p, r.subtree_end(id)));
        p += 1;
        match r.next(id) {
            Some(n) => id = n,
            None => break,
        }
    }
    while let Some((a, _)) = stack.pop() {
        out[a as usize] = last != u32::MAX && last > a;
    }
}

/// nearest for one node: climb parents until one is in S.
#[inline(always)]
pub fn nearest_one<R: Reader>(r: &R, set: &[bool], id: usize) -> Option<usize> {
    let mut q = r.parent(id);
    while let Some(a) = q {
        if set[r.code(a) as usize] {
            return Some(a);
        }
        q = r.parent(a);
    }
    None
}

/// has for one node: the subtree's codes, by rank over its interval, until one is in S (no structure read
/// inside the loop; on BE no close is searched for).
#[inline(always)]
pub fn has_one<R: Reader>(r: &R, set: &[bool], id: usize) -> bool {
    let lo = r.rank(id) + 1;
    let end = r.subtree_end(id);
    let hi = if R::PREORDER_IDS { end } else { r.rank(end) };
    (lo..hi).any(|k| set[r.code_by_rank(k) as usize])
}

/// The same by following the reader's preorder steps (`next`), as first measured.
#[inline(always)]
pub fn has_one_steps<R: Reader>(r: &R, set: &[bool], id: usize) -> bool {
    let end = r.subtree_end(id);
    let mut q = r.next(id);
    while let Some(c) = q {
        if c >= end {
            return false;
        }
        if set[r.code(c) as usize] {
            return true;
        }
        q = r.next(c);
    }
    false
}

/// field(p, f, c): the first child of `id` whose field is `f` (`field_of` maps a code to its field).
#[inline(always)]
pub fn field_one<R: Reader>(r: &R, field_of: &[u8], f: u8, id: usize) -> Option<usize> {
    let pe = r.subtree_end(id);
    let mut c = r.first_child(id);
    while let Some(x) = c {
        if field_of[r.code(x) as usize] == f {
            return Some(x);
        }
        c = r.next_sibling(x, pe);
    }
    None
}

/// unparen: down through wrapper kinds by first child.
#[inline(always)]
pub fn unparen_one<R: Reader>(r: &R, wrap: &[bool], mut id: usize) -> usize {
    while wrap[r.code(id) as usize] {
        match r.first_child(id) {
            Some(c) => id = c,
            None => break,
        }
    }
    id
}

/// The tree's answers, by preorder number, for the checks.
pub mod truth {
    use super::*;
    pub fn nearest(t: &Tree, inset: &[bool]) -> Vec<u32> {
        (0..t.len()).map(|i| {
            let mut q = t.parent[i];
            while q != NONE && !inset[q as usize] {
                q = t.parent[q as usize];
            }
            q
        }).collect()
    }
    pub fn outermost(t: &Tree, inset: &[bool]) -> Vec<u32> {
        (0..t.len()).map(|i| {
            let (mut q, mut best) = (t.parent[i], NONE);
            while q != NONE {
                if inset[q as usize] {
                    best = q;
                }
                q = t.parent[q as usize];
            }
            best
        }).collect()
    }
    pub fn has(t: &Tree, inset: &[bool]) -> Vec<bool> {
        (0..t.len()).map(|i| (i + 1..i + t.size[i] as usize).any(|j| inset[j])).collect()
    }
}

/// has for every node, read backwards: the next S-node after a, against a's subtree end. No stack.
pub fn has_all_back<R: Reader>(r: &R, set: &[bool], out: &mut Vec<bool>) {
    let n = r.len();
    out.clear();
    out.resize(n, false);
    // Ids compare in preorder in every candidate (BE's are open positions), so "after a, before its end"
    // is an id comparison.
    let mut next_s = usize::MAX;
    let mut id = r.id_of(n - 1);
    let mut p = n;
    loop {
        p -= 1;
        out[p] = next_s < r.subtree_end(id);
        if set[r.code(id) as usize] {
            next_s = id;
        }
        match r.prev(id) {
            Some(q) => id = q,
            None => break,
        }
    }
}

/// has for every node from the S-nodes alone. The nodes from one S-node up to the next, s, all have s as their
/// next S-node, so each has one exactly when it is an ancestor of s: a climb from s's parent while the parent
/// is at or after the S-node before. (An ancestor of s before that S-node contains it too, so it was marked
/// already.) Each node is climbed over at most once; the codes are matched 64 at a time.
pub fn has_groups<R: Reader>(r: &R, set: &[bool], out: &mut Vec<bool>) {
    let n = r.len();
    out.clear();
    out.resize(n, false);
    let mut prev = 0usize;
    let mut on = |s: usize| {
        let mut id = r.id_of(s);
        while let Some(q) = r.parent(id) {
            let k = if R::PREORDER_IDS { q } else { r.rank(q) };
            if k < prev {
                break;
            }
            out[k] = true;
            id = q;
        }
        prev = s;
    };
    match r.packed_codes() {
        Some(p) => p.match_groups(n, set, |base, mut m| {
            while m != 0 {
                on(base + m.trailing_zeros() as usize);
                m &= m - 1;
            }
        }),
        None => (0..n).filter(|&k| set[r.code_by_rank(k) as usize]).for_each(&mut on),
    }
}

/// The span (start, end) of every node of the set, in preorder: what `text` and `match` read for a set.
pub fn spans_groups<R: Reader>(r: &R, set: &[bool], out: &mut Vec<(u64, u64)>) {
    out.clear();
    let mut on = |k: usize| {
        let id = r.id_of(k);
        out.push((r.start(id), r.end(id)));
    };
    match r.packed_codes() {
        Some(p) => p.match_groups(r.len(), set, |base, mut m| {
            while m != 0 {
                on(base + m.trailing_zeros() as usize);
                m &= m - 1;
            }
        }),
        None => (0..r.len()).filter(|&k| set[r.code_by_rank(k) as usize]).for_each(&mut on),
    }
}

/// The span of every node, in preorder.
pub fn spans_all<R: Reader>(r: &R, out: &mut Vec<(u64, u64)>) {
    out.clear();
    let mut id = r.id_of(0);
    loop {
        out.push((r.start(id), r.end(id)));
        match r.next(id) {
            Some(n) => id = n,
            None => break,
        }
    }
}

/// The span of every node, in preorder, for forms that keep the end as the gap to the start just past the
/// subtree: the open nodes wait on a stack, and each is closed when the sweep reaches that start. No start
/// is read twice, and none at random. Other forms read the end as `spans_all` does.
pub fn spans_stream<R: Reader>(r: &R, out: &mut Vec<(u64, u64)>) {
    let n = r.len();
    let id0 = r.id_of(0);
    if r.end_from_next_start(id0, 0).is_none() {
        return spans_all(r, out);
    }
    out.clear();
    out.resize(n, (0, 0));
    let mut stack: Vec<(u32, u32, usize)> = Vec::with_capacity(64);
    let mut id = id0;
    let mut k = 0usize;
    loop {
        let s = r.start(id);
        while let Some(&(q, nx, qid)) = stack.last() {
            if nx as usize != k {
                break;
            }
            out[q as usize].1 = r.end_from_next_start(qid, s).unwrap();
            stack.pop();
        }
        out[k].0 = s;
        let e = r.subtree_end(id);
        stack.push((k as u32, if R::PREORDER_IDS { e } else { r.rank(e) } as u32, id));
        k += 1;
        match r.next(id) {
            Some(nid) => id = nid,
            None => break,
        }
    }
    // The nodes that end with the file.
    for &(q, _, qid) in &stack {
        out[q as usize].1 = r.end(qid);
    }
}

/// The span of every node, in preorder, a block at a time where the candidate can (each block's headers read
/// once), else as `spans_stream`.
pub fn spans_bulk<R: Reader>(r: &R, out: &mut Vec<(u64, u64)>) {
    if !r.spans_bulk(out) {
        spans_stream(r, out);
    }
}

/// Which codes occur in each block of `SUM_BLOCK` nodes (by preorder rank): a bit a code, `words` words a
/// block. Part of the tape when used: its bytes are counted. (A 64-bit filter, each code hashed to one bit,
/// was measured and dropped: a block holds 40–60 distinct codes, so the filter is mostly ones.)
pub const SUM_BLOCK: usize = 256;

pub struct Summary {
    pub words: usize,
    pub exact: bool,
    pub bits: Vec<u64>,
}

#[inline(always)]
fn hash_bit(c: u32) -> u32 {
    (c.wrapping_mul(0x9e37_79b9) >> 26) & 63
}

impl Summary {
    pub fn new<R: Reader>(r: &R, codes: usize, exact: bool) -> Summary {
        let words = if exact { codes.div_ceil(64) } else { 1 };
        let n = r.len();
        let mut bits = vec![0u64; n.div_ceil(SUM_BLOCK) * words];
        for k in 0..n {
            let c = r.code_by_rank(k);
            let (w, b) = if exact { (c as usize / 64, c % 64) } else { (0, hash_bit(c)) };
            bits[(k / SUM_BLOCK) * words + w] |= 1 << b;
        }
        Summary { words, exact, bits }
    }
    pub fn bytes(&self) -> usize {
        self.bits.len() * 8
    }
    /// A set as the summary's mask.
    pub fn mask(&self, set: &[bool]) -> Vec<u64> {
        let mut m = vec![0u64; self.words];
        for (c, _) in set.iter().enumerate().filter(|e| *e.1) {
            if !self.exact {
                m[0] |= 1 << hash_bit(c as u32);
            } else {
                m[c / 64] |= 1 << (c % 64);
            }
        }
        m
    }
    /// Whether block `b` may hold a node of the set.
    #[inline(always)]
    pub fn may(&self, b: usize, mask: &[u64]) -> bool {
        let w = &self.bits[b * self.words..(b + 1) * self.words];
        w.iter().zip(mask).any(|(x, m)| x & m != 0)
    }
}

/// has for one node, skipping the blocks the summary rules out: the subtree's codes by rank.
#[inline(always)]
pub fn has_one_skip<R: Reader>(r: &R, sum: &Summary, mask: &[u64], set: &[bool], id: usize) -> bool {
    let lo = r.rank(id) + 1;
    let end = r.subtree_end(id);
    let hi = if R::PREORDER_IDS { end } else { r.rank(end) };
    let mut k = lo;
    while k < hi {
        if k % SUM_BLOCK == 0 && !sum.may(k / SUM_BLOCK, mask) {
            k += SUM_BLOCK;
            continue;
        }
        if set[r.code_by_rank(k) as usize] {
            return true;
        }
        k += 1;
    }
    false
}

// Measured and dropped (E12, t39): nearest for one node by scanning backwards with the block summary,
// to the last S-node covering the node. Common sets (calls, strings) occur in every block, so it read
// thousands of nodes a call (0.5–10 µs, against 45–150 ns climbing parents).

/// A sorted list, Elias–Fano, with rank (the values below x) and successor (the first value at or above x):
/// every 64th zero of the high bits is sampled for the select a rank starts from. Used for the source's
/// newline offsets (`Lines`: a node's line is the newlines before its start) and for the per-kind position
/// lists. Part of the tape when used: its bytes are counted.
pub type Lines = EfRank;

pub struct EfRank {
    m: usize,
    l: u32,
    low: crate::inplace::Packed,
    high: Vec<u64>,
    nbits: usize,
    zeros: Vec<u32>,
}

impl EfRank {
    /// The newlines of a source (`Lines`). Loading needs no source.
    pub fn new(source: &[u8]) -> EfRank {
        let nl: Vec<u64> = source.iter().enumerate().filter(|e| *e.1 == b'\n').map(|e| e.0 as u64).collect();
        EfRank::of(&nl, source.len() as u64 + 1)
    }
    /// Sorted values below `u`.
    pub fn of(nl: &[u64], u: u64) -> EfRank {
        let m = nl.len();
        let l = if u > m.max(1) as u64 { 63 - (u / m.max(1) as u64).leading_zeros() } else { 0 };
        let low: Vec<u64> = nl.iter().map(|&v| v & ((1u64 << l) - 1)).collect();
        let nbits = (u >> l) as usize + m + 2;
        let mut high = vec![0u64; nbits.div_ceil(64) + 2];
        for (i, &v) in nl.iter().enumerate() {
            let p = (v >> l) as usize + i;
            high[p >> 6] |= 1 << (p & 63);
        }
        let mut zeros = Vec::new();
        let mut z = 0usize;
        for p in 0..nbits {
            if high[p >> 6] >> (p & 63) & 1 == 0 {
                if z % 64 == 0 {
                    zeros.push(p as u32);
                }
                z += 1;
            }
        }
        EfRank { m, l, low: crate::inplace::Packed::new_width(&low, l), high, nbits, zeros }
    }
    pub fn bytes(&self) -> usize {
        self.low.bytes() + self.high.len() * 8 + self.zeros.len() * 4 + 8
    }
    /// The position of the k-th zero (from 0) of the high bits.
    #[inline(always)]
    fn select0(&self, k: usize) -> Option<usize> {
        let mut p = *self.zeros.get(k / 64)? as usize;
        let mut r = k % 64;
        // p is a zero; count r more zeros after it.
        if r == 0 {
            return Some(p);
        }
        p += 1;
        loop {
            if p >= self.nbits {
                return None;
            }
            let w = !self.high[p >> 6] >> (p & 63);
            let avail = (64 - (p & 63)) as u32;
            let w = if avail == 64 { w } else { w & ((1u64 << avail) - 1) };
            let c = w.count_ones() as usize;
            if r <= c && c > 0 {
                // The r-th zero (1-based) in w.
                let mut w = w;
                for _ in 1..r {
                    w &= w - 1;
                }
                return Some(p + w.trailing_zeros() as usize);
            }
            r -= c;
            p += avail as usize;
        }
    }
    pub fn len(&self) -> usize {
        self.m
    }
    /// Every value in order: one pass over the high bits.
    #[inline(always)]
    pub fn for_each(&self, mut f: impl FnMut(u64)) {
        let mut i = 0usize;
        for w in 0..self.high.len() {
            let mut word = self.high[w];
            while word != 0 && i < self.m {
                let p = w * 64 + word.trailing_zeros() as usize;
                f((((p - i) as u64) << self.l) | self.low.get(i));
                i += 1;
                word &= word - 1;
            }
        }
    }
    pub fn is_empty(&self) -> bool {
        self.m == 0
    }
    /// The first value at or above x, if any.
    #[inline(always)]
    pub fn succ(&self, x: u64) -> Option<u64> {
        let (i, mut p) = self.rank_at(x);
        if i >= self.m {
            return None;
        }
        // Past the bucket's zeros to the i-th one.
        while self.high[p >> 6] >> (p & 63) & 1 == 0 {
            p += 1;
        }
        Some((((p - i) as u64) << self.l) | self.low.get(i))
    }
    #[inline(always)]
    pub fn line(&self, x: u64) -> usize {
        self.rank_at(x).0
    }
    /// The values below x, and the high-bit position the scan stopped at.
    #[inline(always)]
    fn rank_at(&self, x: u64) -> (usize, usize) {
        let h = (x >> self.l) as usize;
        let mut p = if h == 0 {
            0
        } else {
            match self.select0(h - 1) {
                Some(z) => z + 1,
                None => return (self.m, self.nbits),
            }
        };
        let mut cnt = p - h;
        let lo = x & ((1u64 << self.l) - 1);
        while p < self.nbits && self.high[p >> 6] >> (p & 63) & 1 == 1 && self.low.get(cnt) < lo {
            cnt += 1;
            p += 1;
        }
        (cnt, p)
    }
}

/// Every node's line, by preorder: starts never decrease in preorder, so one merge with the newlines.
pub fn lines_all<R: Reader>(r: &R, lines: &Lines, out: &mut Vec<u32>) {
    out.clear();
    // The newlines in order, from the high bits' ones and the low bits.
    let (mut p, mut i) = (0usize, 0usize);
    let next = |p: &mut usize, i: usize| -> u64 {
        while lines.high[*p >> 6] >> (*p & 63) & 1 == 0 {
            *p += 1;
        }
        (((*p - i) as u64) << lines.l) | lines.low.get(i)
    };
    let mut nl = if lines.m > 0 { next(&mut p, 0) } else { u64::MAX };
    let mut id = r.id_of(0);
    loop {
        let s = r.start(id);
        while nl < s {
            i += 1;
            p += 1;
            nl = if i < lines.m { next(&mut p, i) } else { u64::MAX };
        }
        out.push(i as u32);
        match r.next(id) {
            Some(n) => id = n,
            None => break,
        }
    }
}

/// Per-kind position lists: for each kind of the language, the preorder ranks of its nodes (Elias–Fano).
/// The option the "no stored answers" decision prices (t39 §4.2): every node of a kind without a sweep, and
/// `has` by one successor search a kind of the set.
pub struct Postings {
    by_kind: HashMap<u16, EfRank>,
}

impl Postings {
    pub fn new<R: Reader>(r: &R, kind_of_code: &[u16]) -> Postings {
        let mut lists: HashMap<u16, Vec<u64>> = HashMap::new();
        for k in 0..r.len() {
            lists.entry(kind_of_code[r.code_by_rank(k) as usize]).or_default().push(k as u64);
        }
        let n = r.len() as u64;
        Postings { by_kind: lists.into_iter().map(|(k, v)| (k, EfRank::of(&v, n))).collect() }
    }
    pub fn bytes(&self) -> usize {
        self.by_kind.values().map(|e| e.bytes() + 4).sum()
    }
    /// The lists of a set's kinds.
    pub fn of_set(&self, kinds: &[u16]) -> Vec<&EfRank> {
        kinds.iter().filter_map(|k| self.by_kind.get(k)).collect()
    }
}

/// has for one node by the position lists: some kind of the set has a node in the subtree's rank interval.
#[inline(always)]
pub fn has_one_post<R: Reader>(r: &R, lists: &[&EfRank], id: usize) -> bool {
    let lo = r.rank(id) + 1;
    let end = r.subtree_end(id);
    let hi = if R::PREORDER_IDS { end } else { r.rank(end) };
    lists.iter().any(|e| e.succ(lo as u64).is_some_and(|v| (v as usize) < hi))
}

/// Every node of the set, by preorder rank: a sweep of the codes.
pub fn every_sweep<R: Reader>(r: &R, set: &[bool], out: &mut Vec<u32>) {
    out.clear();
    for k in 0..r.len() {
        if set[r.code_by_rank(k) as usize] {
            out.push(k as u32);
        }
    }
}

/// Every node of the set, 64 codes at a time: a match mask a group, its bits taken out by trailing zeros.
/// A candidate without one packed code array falls back to the plain sweep.
pub fn every_groups<R: Reader>(r: &R, set: &[bool], out: &mut Vec<u32>) {
    let Some(p) = r.packed_codes() else {
        return every_sweep(r, set, out);
    };
    out.clear();
    p.match_groups(r.len(), set, |base, mut m| {
        while m != 0 {
            out.push((base + m.trailing_zeros() as usize) as u32);
            m &= m - 1;
        }
    });
}

/// The same from the position lists: each list read in order, merged.
pub fn every_post(lists: &[&EfRank], out: &mut Vec<u32>) {
    out.clear();
    for e in lists {
        e.for_each(|v| out.push(v as u32));
    }
    if lists.len() > 1 {
        out.sort_unstable();
    }
}

/// Every node of the set by a code sweep that skips the blocks the summary rules out.
pub fn every_sweep_skip<R: Reader>(r: &R, sum: &Summary, mask: &[u64], set: &[bool], out: &mut Vec<u32>) {
    out.clear();
    let n = r.len();
    let mut k = 0;
    while k < n {
        if k % SUM_BLOCK == 0 && !sum.may(k / SUM_BLOCK, mask) {
            k += SUM_BLOCK;
            continue;
        }
        if set[r.code_by_rank(k) as usize] {
            out.push(k as u32);
        }
        k += 1;
    }
}
