//! E22 (t39 step 14): the field taken out of the code.
//!
//! A code is (kind, field, flags, supertypes); the field multiplies the rest (E20: 183–268 values without it,
//! 391–494 with). Here a node stores its base (kind, flags, supertypes) as E21 stores a code: numbered by
//! frequency, the byte itself below the cutoff 255. Its field is predicted from a context, and a node the
//! prediction gets wrong is listed in its block's side list (offset, field), as escapes are.
//!
//! The contexts, each with the node's own base: none; the previous byte (the node before in preorder, read
//! for free); the parent's base; the parent's base and the previous sibling's; the parent's base and the
//! child's index. The prediction is the most frequent field a context had on the even files, else the base's;
//! misses are counted on the odd ones. A context past the base alone needs a table of the contexts whose
//! field differs from the base's, and the reads that context takes (a parent, a sibling).

use crate::capture::File;
use crate::code8::{numbers_by, Rank8};
use crate::codec::{self, Codec, Dictionary};
use crate::disk::Lz4;
use crate::inplace::{BLOCK, Codes, Cols};
use crate::measure::cost;
use crate::tree::NONE;
use std::collections::HashMap;

/// What a node's field is predicted from, with its own base.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pred {
    Base,
    PrevByte,
    Parent,
    ParentPrevSibling,
    ParentIndex,
    /// Without the node's own base: a direct table [parent's base][index, at most 15], 4 KiB.
    ParentIndexOnly,
}

pub const PREDS: [Pred; 6] = [Pred::Base, Pred::PrevByte, Pred::Parent, Pred::ParentPrevSibling, Pred::ParentIndex, Pred::ParentIndexOnly];

pub fn pred_name(p: Pred) -> &'static str {
    match p {
        Pred::Base => "the base alone",
        Pred::PrevByte => "the previous byte (free)",
        Pred::Parent => "the parent's base",
        Pred::ParentPrevSibling => "the parent's and the previous sibling's bases",
        Pred::ParentIndex => "the parent's base and the child's index",
        Pred::ParentIndexOnly => "the parent's base and the child's index, not the node's own base",
    }
}

/// A file's nodes as E22 sees them.
struct Doc {
    base: Vec<u32>,
    field: Vec<u8>,
    parent: Vec<u32>,
    prev_sibling: Vec<u32>,
    index: Vec<u32>,
}

fn doc(c: &Cols, base_of: &[u32], field_of: &[u8]) -> Doc {
    let n = c.len();
    let (mut prev_sibling, mut index) = (vec![NONE; n], vec![0u32; n]);
    let (mut last, mut count) = (vec![NONE; n], vec![0u32; n]);
    for i in 0..n {
        let p = c.parent[i];
        if p != NONE {
            let p = p as usize;
            prev_sibling[i] = last[p];
            index[i] = count[p];
            count[p] += 1;
            last[p] = i as u32;
        }
    }
    Doc {
        base: c.code.iter().map(|&k| base_of[k as usize]).collect(),
        field: c.code.iter().map(|&k| field_of[k as usize]).collect(),
        parent: c.parent.clone(),
        prev_sibling,
        index,
    }
}

fn key(d: &Doc, i: usize, p: Pred) -> u64 {
    let of = |j: u32| if j == NONE { 0xffff } else { u64::from(d.base[j as usize]) };
    let b = u64::from(d.base[i]);
    let par = of(d.parent[i]);
    match p {
        Pred::Base => b,
        Pred::PrevByte => (if i == 0 { 0xffff } else { u64::from(d.base[i - 1]) }) << 16 | b,
        Pred::Parent => par << 16 | b,
        Pred::ParentPrevSibling => par << 32 | of(d.prev_sibling[i]) << 16 | b,
        Pred::ParentIndex => par << 32 | u64::from(d.index[i].min(255)) << 16 | b,
        Pred::ParentIndexOnly => par << 16 | u64::from(d.index[i].min(15)),
    }
}

/// The most frequent field of each key.
fn best(counts: HashMap<u64, HashMap<u8, u64>>) -> HashMap<u64, u8> {
    counts.into_iter().map(|(k, m)| (k, m.into_iter().max_by_key(|&(f, n)| (n, std::cmp::Reverse(f))).map_or(0, |x| x.0))).collect()
}

/// A predictor: its table, and the base's field to fall back on.
struct Predictor {
    p: Pred,
    table: HashMap<u64, u8>,
}

impl Predictor {
    fn field(&self, d: &Doc, i: usize, base_best: &HashMap<u64, u8>) -> u8 {
        if let Some(&f) = self.table.get(&key(d, i, self.p)) {
            return f;
        }
        if self.p == Pred::ParentIndexOnly {
            return 0;
        }
        base_best.get(&u64::from(d.base[i])).copied().unwrap_or(0)
    }
}

/// The nodes a prediction misses, a block at a time: (start a block, offsets in it, fields).
pub struct Misses {
    start: Vec<u32>,
    off: Vec<u8>,
    field: Vec<u8>,
}

impl Misses {
    fn new(n: usize, miss: &[(usize, u8)]) -> Misses {
        let blocks = n.div_ceil(BLOCK);
        let mut start = vec![0u32; blocks + 1];
        let (mut off, mut field) = (Vec::new(), Vec::new());
        for &(i, f) in miss {
            start[i / BLOCK + 1] += 1;
            off.push((i % BLOCK) as u8);
            field.push(f);
        }
        for b in 0..blocks {
            start[b + 1] += start[b];
        }
        Misses { start, off, field }
    }
    fn bytes(&self) -> usize {
        self.start.len() * 4 + self.off.len() + self.field.len()
    }
    /// A missed node's field, if listed.
    #[inline(always)]
    fn get(&self, i: usize) -> Option<u8> {
        let b = i / BLOCK;
        // SAFETY: b + 1 < start.len() for i < len.
        let (s, e) = unsafe { (*self.start.get_unchecked(b) as usize, *self.start.get_unchecked(b + 1) as usize) };
        if s == e {
            return None;
        }
        self.search(s, e, i)
    }
    #[inline(never)]
    fn search(&self, s: usize, e: usize, i: usize) -> Option<u8> {
        self.off[s..e].binary_search(&((i % BLOCK) as u8)).ok().map(|k| self.field[s + k])
    }
}

/// The (parent's base, child's index, base) -> field entries whose field differs from the base's, open
/// addressing over base numbers as stored: a key `par << 21 | index << 13 | base` (u32) and a field byte a
/// slot, at most half full.
pub struct FieldTab {
    keys: Vec<u32>,
    fields: Vec<u8>,
    mask: u32,
    shift: u32,
    /// The base's own field, by base number.
    base: Vec<u8>,
}

const EMPTY: u32 = u32::MAX;

#[inline(always)]
fn tab_key(par: u32, index: u32, base: u32) -> u32 {
    par.min(0x7ff) << 21 | index.min(255) << 13 | base
}

impl FieldTab {
    fn new(entries: &[(u32, u8)], base: Vec<u8>) -> FieldTab {
        let slots = (entries.len() * 2).next_power_of_two().max(16);
        let mask = slots as u32 - 1;
        // The product's top bits: every key bit reaches them.
        let shift = 32 - slots.trailing_zeros();
        let (mut keys, mut fields) = (vec![EMPTY; slots], vec![0u8; slots]);
        for &(k, f) in entries {
            let mut h = k.wrapping_mul(0x9e37_79b1) >> shift;
            while keys[h as usize] != EMPTY {
                h = (h + 1) & mask;
            }
            keys[h as usize] = k;
            fields[h as usize] = f;
        }
        FieldTab { keys, fields, mask, shift, base }
    }
    pub fn bytes(&self) -> usize {
        self.keys.len() * 5 + self.base.len()
    }
    /// The first slot decides without a branch on the data (a select between its field and the base's);
    /// only a collision probes on.
    #[inline(always)]
    fn get(&self, par: u32, index: u32, base: u32) -> u8 {
        let k = tab_key(par, index, base);
        let h = (k.wrapping_mul(0x9e37_79b1) >> self.shift) as usize;
        // SAFETY: h < keys.len(), base < base.len().
        let (x, f, d) = unsafe { (*self.keys.get_unchecked(h), *self.fields.get_unchecked(h), *self.base.get_unchecked(base as usize)) };
        if x != k && x != EMPTY {
            return self.probe(h, k, base);
        }
        if x == k { f } else { d }
    }
    #[cold]
    #[inline(never)]
    fn probe(&self, mut h: usize, k: u32, base: u32) -> u8 {
        loop {
            h = (h + 1) & self.mask as usize;
            let x = self.keys[h];
            if x == k {
                return self.fields[h];
            }
            if x == EMPTY {
                return self.base[base as usize];
            }
        }
    }
}

pub struct Report {
    pub files: usize,
    pub measured_nodes: u64,
    pub train_nodes: u64,
    pub codes: usize,
    pub bases: usize,
    pub fields: usize,
    /// Measured nodes with a field; bases seen with two fields or more (on the even files), and the measured
    /// nodes on them.
    pub with_field: u64,
    pub ambiguous_bases: usize,
    pub ambiguous_nodes: u64,
    /// Nodes past the cutoff 255: codes (E21), bases.
    pub tail: [u64; 2],
    /// Per predictor (PREDS): measured nodes it misses; on the files it was chosen on; table entries whose
    /// field differs from the base's.
    pub miss: [u64; 6],
    pub miss_train: [u64; 6],
    pub entries: [usize; 6],
    /// Memory, B a node: E21 (code byte + side list); bases + misses per predictor.
    pub mem_e21: f64,
    pub mem: [f64; 6],
    /// Disk, zstd 3 + dict / LZ4 HC + dict, B a node: E21; bases + misses of the base alone; of the parent's
    /// and previous sibling's bases.
    pub disk: [[f64; 2]; 3],
    /// ns at random / in order: E21's code; E21's field (code, then a 512-entry table); E22's base; E22's field
    /// by the base alone (base, a table, the block's misses).
    /// and the field of a child reached from its parent (parent id and index known): E21's code then a table;
    /// E22's parent byte, own byte, the (parent, index, base) table and the block's misses.
    pub ns: [[f64; 2]; 9],
    /// Bytes of E22's (parent, index, base) table.
    pub tab_bytes: usize,
    pub wrong: u64,
}

pub fn run(files: &[&File]) -> Report {
    let read = |f: &File| crate::captured::read(&f.image).unwrap_or_else(|e| panic!("{}: {e}", f.path));
    let trees: Vec<_> = files.iter().map(|f| read(f)).collect();
    let mut codes = Codes::default();
    for t in &trees {
        codes.learn(t);
    }
    let cols: Vec<Cols> = trees.iter().map(|t| Cols::of_tree(t, &codes)).collect();
    drop(trees);
    // Bases: (kind, flags, supertypes).
    let mut base_ids: HashMap<(u16, u8, Vec<u16>), u32> = HashMap::new();
    let mut base_of = Vec::with_capacity(codes.table.len());
    let mut field_of = Vec::with_capacity(codes.table.len());
    let mut fields = std::collections::HashSet::new();
    for (kind, field, flags, sup) in &codes.table {
        let n = base_ids.len() as u32;
        base_of.push(*base_ids.entry((*kind, *flags, sup.clone())).or_insert(n));
        field_of.push(*field);
        fields.insert(*field);
    }
    let nb = base_ids.len();
    let split = files.len() >= 4;
    let (train, test): (Vec<&Cols>, Vec<&Cols>) = if split {
        (cols.iter().step_by(2).collect(), cols.iter().skip(1).step_by(2).collect())
    } else {
        (cols.iter().collect(), cols.iter().collect())
    };
    let train_docs: Vec<Doc> = train.iter().map(|c| doc(c, &base_of, &field_of)).collect();
    let test_docs: Vec<Doc> = test.iter().map(|c| doc(c, &base_of, &field_of)).collect();
    // Numberings by frequency on the even files: codes (E21), bases.
    let mut code_freq = vec![0u64; codes.table.len()];
    let mut base_freq = vec![0u64; nb];
    for c in &train {
        for &k in &c.code {
            code_freq[k as usize] += 1;
            base_freq[base_of[k as usize] as usize] += 1;
        }
    }
    let code_num = numbers_by(&code_freq);
    let base_num = numbers_by(&base_freq);
    // Predictors.
    let preds: Vec<Predictor> = PREDS
        .iter()
        .map(|&p| {
            let mut counts: HashMap<u64, HashMap<u8, u64>> = HashMap::new();
            for d in &train_docs {
                for i in 0..d.base.len() {
                    *counts.entry(key(d, i, p)).or_default().entry(d.field[i]).or_insert(0) += 1;
                }
            }
            Predictor { p, table: best(counts) }
        })
        .collect();
    let base_best = preds[0].table.clone();
    let mut r = Report {
        files: files.len(),
        measured_nodes: test.iter().map(|c| c.len() as u64).sum(),
        train_nodes: train.iter().map(|c| c.len() as u64).sum(),
        codes: codes.table.len(),
        bases: nb,
        fields: fields.len(),
        with_field: test_docs.iter().map(|d| d.field.iter().filter(|&&f| f != 0).count() as u64).sum(),
        ambiguous_bases: 0,
        ambiguous_nodes: 0,
        tail: [0; 2],
        miss: [0; 6],
        miss_train: [0; 6],
        entries: [0; 6],
        mem_e21: 0.0,
        mem: [0.0; 6],
        disk: [[0.0; 2]; 3],
        ns: [[0.0; 2]; 9],
        tab_bytes: 0,
        wrong: 0,
    };
    // Bases seen with two fields or more.
    let mut seen: HashMap<u32, std::collections::HashSet<u8>> = HashMap::new();
    for d in &train_docs {
        for i in 0..d.base.len() {
            seen.entry(d.base[i]).or_default().insert(d.field[i]);
        }
    }
    r.ambiguous_bases = seen.values().filter(|s| s.len() > 1).count();
    r.ambiguous_nodes = test_docs.iter().map(|d| d.base.iter().filter(|b| seen.get(b).is_some_and(|s| s.len() > 1)).count() as u64).sum();
    for (k, pr) in preds.iter().enumerate() {
        r.entries[k] = if pr.p == Pred::ParentIndexOnly {
            pr.table.len()
        } else if pr.p == Pred::Base {
            pr.table.values().filter(|&&f| f != 0).count()
        } else {
            pr.table.iter().filter(|(key, f)| base_best.get(&(*key & 0xffff)) != Some(f)).count()
        };
        for d in &train_docs {
            r.miss_train[k] += (0..d.base.len()).filter(|&i| pr.field(d, i, &base_best) != d.field[i]).count() as u64;
        }
    }
    let (mut side21, mut side_base) = (0u64, 0u64);
    let mut miss_bytes = [0u64; 6];
    for (c, d) in test.iter().zip(&test_docs) {
        let n = c.len();
        let r21 = Rank8::new(c, &code_num, 255);
        let r22 = Rank8::of_ids(&d.base, &base_num, 255);
        r.tail[0] += r21.tail_nodes() as u64;
        r.tail[1] += r22.tail_nodes() as u64;
        side21 += r21.side_bytes() as u64;
        side_base += r22.side_bytes() as u64;
        for (k, pr) in preds.iter().enumerate() {
            let miss: Vec<(usize, u8)> = (0..n).filter_map(|i| (pr.field(d, i, &base_best) != d.field[i]).then_some((i, d.field[i]))).collect();
            r.miss[k] += miss.len() as u64;
            let m = Misses::new(n, &miss);
            miss_bytes[k] += m.bytes() as u64;
            // The read gives the field back.
            r.wrong += (0..n).filter(|&i| m.get(i).unwrap_or_else(|| pr.field(d, i, &base_best)) != d.field[i]).count() as u64;
        }
    }
    let mn = r.measured_nodes.max(1) as f64;
    r.mem_e21 = (mn + side21 as f64) / mn;
    for k in 0..6 {
        r.mem[k] = (mn + (side_base + miss_bytes[k]) as f64) / mn;
    }
    // The base read back and its field, by the base-alone predictor, numbered as stored.
    let mut by_num = vec![0u32; nb];
    for (b, &n) in base_num.iter().enumerate() {
        by_num[n as usize] = b as u32;
    }
    let f_by_base_num: Vec<u8> = (0..nb).map(|n| base_best.get(&u64::from(by_num[n])).copied().unwrap_or(0)).collect();
    let f_by_code_num: Vec<u8> = {
        let mut v = vec![0u8; codes.table.len()];
        for (k, &n) in code_num.iter().enumerate() {
            v[n as usize] = field_of[k];
        }
        v
    };
    let misses_of = |d: &Doc, p: &Predictor| {
        let miss: Vec<(usize, u8)> = (0..d.base.len()).filter_map(|i| (p.field(d, i, &base_best) != d.field[i]).then_some((i, d.field[i]))).collect();
        Misses::new(d.base.len(), &miss)
    };
    // Disk: each form a file, a dictionary trained on the even files' forms.
    if split {
        let form = |c: &Cols, d: &Doc, k: usize| -> Vec<u8> {
            if k == 0 {
                return Rank8::new(c, &code_num, 255).disk_bytes();
            }
            let mut b = Rank8::of_ids(&d.base, &base_num, 255).disk_bytes();
            let m = misses_of(d, &preds[if k == 1 { 0 } else { 3 }]);
            for (&o, &f) in m.off.iter().zip(&m.field) {
                b.push(o);
                b.push(f);
            }
            b
        };
        for k in 0..3 {
            let samples: Vec<Vec<u8>> = train.iter().zip(&train_docs).map(|(c, d)| form(c, d, k)).collect();
            let dict = Dictionary::train(&samples, 64 << 10);
            let mut lz = Lz4::new(true, &dict.bytes);
            let (mut z, mut l) = (0u64, 0u64);
            for (c, d) in test.iter().zip(&test_docs) {
                let x = form(c, d, k);
                z += codec::compress(Codec::Zstd, &x, &dict, 0).len() as u64;
                l += lz.compress(&x).len() as u64;
            }
            r.disk[k] = [z as f64 / mn, l as f64 / mn];
        }
    }
    // Reads on the largest measured file.
    let big = (0..test.len()).max_by_key(|&f| test[f].len()).unwrap_or(0);
    let (c, d) = (test[big], &test_docs[big]);
    let n = c.len();
    let r21 = Rank8::new(c, &code_num, 255);
    let r22 = Rank8::of_ids(&d.base, &base_num, 255);
    let m0 = misses_of(d, &preds[0]);
    let mut order: Vec<usize> = (0..n).collect();
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    for i in (1..order.len()).rev() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        order.swap(i, (rng % (i as u64 + 1)) as usize);
    }
    let best_of = |f: &dyn Fn() -> u64| {
        let mut b = u64::MAX;
        for _ in 0..5 {
            let (s, t) = cost(f);
            std::hint::black_box(s);
            b = b.min(t.nanos);
        }
        b as f64 / n.max(1) as f64
    };
    fn both(order: &[usize], n: usize, best: &dyn Fn(&dyn Fn() -> u64) -> f64, f: impl Fn(usize) -> u64) -> [f64; 2] {
        [
            best(&|| order.iter().fold(0u64, |s, &i| s.wrapping_add(f(i)))),
            best(&|| (0..n).fold(0u64, |s, i| s.wrapping_add(f(i)))),
        ]
    }
    let f21 = |i: usize| u64::from(f_by_code_num[r21.get(i) as usize]);
    let f22 = |i: usize| u64::from(m0.get(i).unwrap_or_else(|| f_by_base_num[r22.get(i) as usize]));
    // A child reached from its parent: the parent's id and the child's index are in hand.
    let mut tab_entries: Vec<(u32, u8)> = Vec::new();
    for (&k, &f) in &preds[4].table {
        let (par, index, b) = ((k >> 32) as u32, (k >> 16 & 0xffff) as u32, (k & 0xffff) as u32);
        if base_best.get(&u64::from(b)).copied().unwrap_or(0) != f {
            let pn = if par == 0xffff { 0x7ff } else { base_num[par as usize] };
            tab_entries.push((tab_key(pn, index, base_num[b as usize]), f));
        }
    }
    let tab = FieldTab::new(&tab_entries, f_by_base_num.clone());
    r.tab_bytes = tab.bytes();
    let m4 = misses_of(d, &preds[4]);
    // An enumeration of children holds the parent's base and the child's index: the reads get them as a
    // stream (node, parent's base number, index), at random and in order.
    let trip = |i: usize| {
        let p = d.parent[i];
        (i as u32, if p == NONE { 0x7ff } else { r22.get(p as usize) }, d.index[i])
    };
    let trips = [order.iter().map(|&i| trip(i)).collect::<Vec<_>>(), (0..n).map(trip).collect::<Vec<_>>()];
    // The direct table: row a parent's base number (the root's row last), 16 indexes a row.
    let rows = nb + 1;
    let mut direct = vec![0u8; rows * 16];
    for (&k, &f) in &preds[5].table {
        let (par, x) = ((k >> 16) as u32, (k & 0xffff) as usize);
        let row = if par == 0xffff { nb } else { base_num[par as usize] as usize };
        direct[row * 16 + x] = f;
    }
    let m5 = misses_of(d, &preds[5]);
    let f22d = |(i, pn, x): (u32, u32, u32)| {
        let row = if pn == 0x7ff { nb } else { pn as usize };
        // SAFETY: row < rows, x.min(15) < 16.
        u64::from(m5.get(i as usize).unwrap_or(unsafe { *direct.get_unchecked(row * 16 + x.min(15) as usize) }))
    };
    let f22p = |(i, pn, x): (u32, u32, u32)| u64::from(m4.get(i as usize).unwrap_or_else(|| tab.get(pn, x, r22.get(i as usize))));
    let f21p = |(i, _, _): (u32, u32, u32)| f21(i as usize);
    let stream = |f: &dyn Fn((u32, u32, u32)) -> u64| -> [f64; 2] { [0, 1].map(|k| best_of(&|| trips[k].iter().fold(0u64, |s, &t| s.wrapping_add(f(t))))) };
    r.ns = [
        both(&order, n, &best_of, |i| u64::from(r21.get(i))),
        both(&order, n, &best_of, f21),
        both(&order, n, &best_of, |i| u64::from(r22.get(i))),
        both(&order, n, &best_of, f22),
        stream(&f21p),
        stream(&f22p),
        stream(&|(i, pn, x)| u64::from(tab.get(pn, x, r22.get(i as usize)))),
        stream(&|(i, _, _)| u64::from(m4.get(i as usize).unwrap_or(0))),
        stream(&f22d),
    ];
    r.wrong += (0..n).filter(|&i| f22d(trip(i)) != u64::from(d.field[i])).count() as u64;
    r.wrong += (0..n).filter(|&i| f22p(trip(i)) != u64::from(d.field[i])).count() as u64;
    r.wrong += (0..n).filter(|&i| f21(i) != u64::from(d.field[i]) || f22(i) != u64::from(d.field[i]) || by_num[r22.get(i) as usize] != d.base[i]).count() as u64;
    r
}

pub fn print(title: &str, r: &Report) {
    let pct = |a: u64, b: u64| format!("{:.2}%", 100.0 * a as f64 / b.max(1) as f64);
    println!("### {title}\n");
    println!(
        "{} files; {} measured nodes{}. {} codes, {} bases (kind, flags, supertypes), {} fields. Decoded wrong: {}.\n",
        r.files, r.measured_nodes, if r.files >= 4 { ", predictors chosen on the others" } else { "" }, r.codes, r.bases, r.fields, r.wrong
    );
    println!("| | value |");
    println!("|---|---|");
    println!("| nodes with a field | {} |", pct(r.with_field, r.measured_nodes));
    println!("| bases seen with two fields or more; measured nodes on them | {}; {} |", r.ambiguous_bases, pct(r.ambiguous_nodes, r.measured_nodes));
    println!("| nodes past the cutoff 255: codes (E21) / bases | {} / {} |", pct(r.tail[0], r.measured_nodes), pct(r.tail[1], r.measured_nodes));
    for (k, &p) in PREDS.iter().enumerate() {
        println!(
            "| field from {}: nodes missed (measured; chosen on); table entries; memory B a node | {} ({}); {}; {:.3} |",
            pred_name(p), pct(r.miss[k], r.measured_nodes), pct(r.miss_train[k], r.train_nodes), r.entries[k], r.mem[k]
        );
    }
    println!("| memory, B a node: E21 (field in the code) | {:.3} |", r.mem_e21);
    println!("| E22's (parent, index, base) table, bytes | {} |", r.tab_bytes);
    println!("| on disk, zstd 3 + dict / LZ4 HC + dict: E21 | {:.3} / {:.3} |", r.disk[0][0], r.disk[0][1]);
    println!("| the same: bases + misses of the base alone | {:.3} / {:.3} |", r.disk[1][0], r.disk[1][1]);
    println!("| the same: bases + misses of the parent's and previous sibling's bases | {:.3} / {:.3} |", r.disk[2][0], r.disk[2][1]);
    let names = [
        "E21's code",
        "E21's field (code, then a table)",
        "E22's base",
        "E22's field by the base alone (base, a table, the block's misses)",
        "a child's field from its parent: E21 (code, then a table)",
        "a child's field from its parent: E22 (parent byte, own byte, table, misses)",
        "the same, the table alone (own byte, table)",
        "the same, the block's misses alone",
        "a child's field from its parent: E22, direct [parent][index] table (4 KiB) and misses",
    ];
    for (k, name) in names.iter().enumerate() {
        println!("| read at random / in order, ns: {name} | {:.2} / {:.2} |", r.ns[k][0], r.ns[k][1]);
    }
    println!();
}
