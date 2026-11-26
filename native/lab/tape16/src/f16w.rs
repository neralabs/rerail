//! E28 (t39 step 20): format 16's writer, as 0156 §7 gives it: `open(code, start)` and `close(end)` as a parser
//! makes nodes, in preorder, and `finish` at the end. The language's numbering and widths are fixed before the
//! file starts, so every cell is written once at its node's index:
//!
//! - at `open`: the code byte (or the prefix, and a side-list entry), the gap, every K-th node's checkpoint,
//!   and the slot of the leaf before it, which describes this node (the distance back to the parent where it
//!   is at most `f16::FIT` and on every K-th sibling, else its previous sibling's size);
//! - at `close`: the length, and an inner node's slot (`size << 1`);
//! - at `finish`: the last node's slot, and the side lists, each block's entries sorted, laid in parts.
//!
//! Its output is checked byte for byte against the reader's tape (`F16::new` over the same nodes).

use crate::code8::{PART, side_parts};
use crate::f16::{F16Lang, K, ckpt_width, fit_limit};
use crate::inplace::{BLOCK, Cols, bits_for};

struct Open {
    i: u32,
    start: u64,
    children: u32,
    last: u32,
}

/// `W`: the slot, gap and length width in bytes when the language's three are equal (generated code knows its
/// widths), or 0 for any.
pub struct Writer<'a, const W: usize = 0> {
    lang: &'a F16Lang,
    /// Slot, gap and length widths in bytes, and their escapes (all ones).
    wb: [usize; 3],
    esc: [u64; 3],
    /// Slot values below this name the parent (`f16::fit_limit`).
    fit: u64,
    cut: u32,
    per: u32,
    low_w: usize,
    cwb: usize,
    pub mark: u8,
    codes: Vec<u8>,
    arrays: [Vec<u8>; 3],
    ckpt: Vec<u8>,
    /// Side-list entries: code low parts, then escaped slots, gaps, lengths, as (node, value).
    side: [Vec<(u32, u64)>; 4],
    stack: Vec<Open>,
    n: u32,
    prev_start: u64,
}

const NONE: u32 = u32::MAX;

impl<'a, const W: usize> Writer<'a, W> {
    /// A writer for one file of `source_len` bytes, with room for `nodes_hint` nodes.
    pub fn new(lang: &'a F16Lang, source_len: u64, nodes_hint: usize) -> Writer<'a, W> {
        let wb = [lang.widths[0] as usize / 8, lang.widths[1] as usize / 8, lang.widths[2] as usize / 8];
        let ncodes = lang.table.max(lang.num.len()) as u32;
        let cut = lang.cut;
        let per = if cut >= 256 { 1 } else { (ncodes - cut).div_ceil(256 - cut) };
        let mk = |b: usize| if b >= 8 { u64::MAX } else { (1u64 << (8 * b)) - 1 };
        Writer {
            lang,
            wb,
            esc: [mk(wb[0]), mk(wb[1]), mk(wb[2])],
            fit: fit_limit(lang.widths[0]),
            cut,
            per,
            low_w: if per <= 256 { 1 } else { 2 },
            cwb: ckpt_width(source_len) as usize / 8,
            mark: lang.version,
            codes: vec![0; nodes_hint],
            arrays: [vec![0; nodes_hint * wb[0]], vec![0; nodes_hint * wb[1]], vec![0; nodes_hint * wb[2]]],
            ckpt: Vec::with_capacity(nodes_hint.div_ceil(K) * 4),
            side: Default::default(),
            stack: Vec::with_capacity(64),
            n: 0,
            prev_start: 0,
        }
    }

    /// Writes value `v` of array `a` (0 slots, 1 gaps, 2 lengths) at node i's cell, or its escape.
    #[inline(always)]
    fn put(&mut self, a: usize, i: u32, v: u64) {
        let w = if W != 0 { W } else { self.wb[a] };
        let v = if v >= self.esc[a] {
            self.side[a + 1].push((i, v));
            self.esc[a]
        } else {
            v
        };
        let at = (i as usize).wrapping_mul(w);
        let dst = &mut self.arrays[a];
        // One store at the width, as code generated for the language's widths would write it.
        match w {
            // SAFETY: `room` made node i's cell in every array before any write to it.
            1 => unsafe { *dst.get_unchecked_mut(at) = v as u8 },
            2 => dst[at..at + 2].copy_from_slice(&(v as u16).to_le_bytes()),
            4 => dst[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes()),
            _ => dst[at..at + w].copy_from_slice(&v.to_le_bytes()[..w]),
        }
    }
    /// Room for node i in every array: grown by doubling, cut to length at `finish`.
    #[inline(always)]
    fn room(&mut self, i: usize) {
        if i >= self.codes.len() {
            let n = (2 * self.codes.len()).max(i + 1).max(1024);
            self.codes.resize(n, 0);
            for a in 0..3 {
                let w = if W != 0 { W } else { self.wb[a] };
                self.arrays[a].resize(n * w, 0);
            }
        }
    }

    /// A node opens: its code's number in the language's table, and its start.
    #[inline]
    pub fn open(&mut self, num: u32, start: u64) {
        // Wrapping arithmetic and unchecked cells throughout: the lab's profile checks overflow and bounds,
        // generated code would not.
        let i = self.n;
        self.n = i.wrapping_add(1);
        self.room(i as usize);
        if num < self.cut {
            // SAFETY: `room` made node i's cell.
            unsafe { *self.codes.get_unchecked_mut(i as usize) = num as u8 };
        } else {
            let t = num - self.cut;
            self.codes[i as usize] = (self.cut + t / self.per) as u8;
            self.side[0].push((i, u64::from(t % self.per)));
        }
        let gap = if i == 0 { 0 } else { start.wrapping_sub(self.prev_start) };
        self.put(1, i, gap);
        self.prev_start = start;
        if i as usize % K == 0 {
            let b = start.to_le_bytes();
            match self.cwb {
                2 => self.ckpt.extend_from_slice(&b[..2]),
                3 => self.ckpt.extend_from_slice(&b[..3]),
                4 => self.ckpt.extend_from_slice(&b[..4]),
                w => self.ckpt.extend_from_slice(&b[..w]),
            }
        }
        if let Some(top) = self.stack.last_mut() {
            let (idx, prev, p) = (top.children, top.last, top.i);
            top.children = idx.wrapping_add(1);
            top.last = i;
            // The node before is a leaf unless it is the parent: its slot describes this node.
            let before = i.wrapping_sub(1);
            if before != p {
                // The parent distance on every K-th sibling, and wherever it is at most FIT (t39 step 23); else
                // the previous sibling's.
                let pd = (u64::from(i.wrapping_sub(p)) << 2) | 3;
                let v = if idx as usize % K == 0 || pd < self.fit { pd } else { (u64::from(i.wrapping_sub(prev)) << 2) | 1 };
                self.put(0, before, v);
            }
        }
        self.stack.push(Open { i, start, children: 0, last: NONE });
    }

    /// The node opened last and not yet closed closes at `end`.
    #[inline]
    pub fn close(&mut self, end: u64) {
        let o = self.stack.pop().expect("a node to close");
        self.put(2, o.i, end.wrapping_sub(o.start));
        let size = self.n.wrapping_sub(o.i);
        if size > 1 {
            self.put(0, o.i, u64::from(size) << 1);
        }
    }

    /// The arrays as stored: each array's fixed-width bytes and its side list in parts (`F16::streams`).
    pub fn finish(mut self) -> [(Vec<u8>, Vec<u8>); 5] {
        assert!(self.stack.is_empty(), "every node closed");
        let n = self.n as usize;
        if n > 0 {
            // The last node in preorder is a leaf with nothing after it.
            self.put(0, self.n - 1, 1);
        }
        let mut parts: Vec<Vec<u8>> = Vec::with_capacity(4);
        for k in 0..4 {
            let e = &mut self.side[k];
            // Codes and gaps arrive in order; slots and lengths of inner nodes at their close.
            e.sort_unstable_by_key(|x| x.0);
            let vw = if k == 0 { self.low_w } else { (bits_for(e.iter().map(|x| x.1).max().unwrap_or(0)) as usize).div_ceil(8).max(1) };
            let blocks = n.div_ceil(BLOCK);
            let mut start = vec![0u32; blocks + 1];
            for x in e.iter() {
                start[x.0 as usize / BLOCK + 1] += 1;
            }
            for b in 0..blocks {
                start[b + 1] += start[b];
            }
            let off: Vec<u8> = e.iter().map(|x| (x.0 as usize % BLOCK) as u8).collect();
            let ps = side_parts(&start, &off, |j, out| out.extend_from_slice(&e[j].1.to_le_bytes()[..vw]), n, PART);
            parts.push(ps.concat());
        }
        let _ = self.lang;
        self.codes.truncate(n);
        for a in 0..3 {
            self.arrays[a].truncate(n * self.wb[a]);
        }
        let [s, g, l] = self.arrays;
        let mut p = parts.into_iter();
        [(self.codes, p.next().unwrap()), (s, p.next().unwrap()), (g, p.next().unwrap()), (l, p.next().unwrap()), (self.ckpt, Vec::new())]
    }
}

/// Drives a writer from a file's nodes as a parser would: each node opened in preorder, each closed after
/// its subtree (the parser's own stack).
pub fn write(lang: &F16Lang, c: &Cols) -> [(Vec<u8>, Vec<u8>); 5] {
    if lang.widths == [8, 8, 8] { drive::<1>(lang, c).finish() } else { drive::<0>(lang, c).finish() }
}

/// The opens and closes alone, the writer left before `finish`.
pub fn drive<'a, const W: usize>(lang: &'a F16Lang, c: &Cols) -> Writer<'a, W> {
    let n = c.len();
    let mut w = Writer::<W>::new(lang, c.source_len, n);
    let mut ends: Vec<(usize, u64)> = Vec::with_capacity(64);
    for i in 0..n {
        while ends.last().is_some_and(|&(e, _)| e <= i) {
            w.close(ends.pop().unwrap().1);
        }
        w.open(lang.num[c.code[i] as usize], c.start[i]);
        ends.push((i + c.size[i] as usize, c.end[i]));
    }
    while let Some((_, end)) = ends.pop() {
        w.close(end);
    }
    w
}

#[derive(Default)]
pub struct Report {
    pub files: usize,
    pub nodes: u64,
    pub source: u64,
    /// Instructions and ns: format 16's writer driven as a parser drives it; and, with the `lean` feature on
    /// `.rerail`, the lean parse with its format 16 write.
    pub f16: crate::measure::Cost,
    /// Of `f16`: `finish` alone (the side lists laid in parts).
    pub finish: crate::measure::Cost,
    pub lean: crate::measure::Cost,
    pub lean_files: usize,
    pub wrong: usize,
}

pub fn run(files: &[&crate::capture::File]) -> Report {
    use crate::f16::{F16, grammar_codes};
    use crate::inplace::Codes;
    use crate::measure::cost;
    let read = |f: &crate::capture::File| crate::captured::read(&f.image).unwrap_or_else(|e| panic!("{}: {e}", f.path));
    let mut codes = Codes::default();
    for f in files {
        codes.learn(&read(f));
    }
    let split = files.len() >= 16;
    let cols: Vec<Cols> = files.iter().map(|f| Cols::of_tree(&read(f), &codes)).collect();
    let train: Vec<usize> = (0..files.len()).filter(|i| !split || i % 2 == 0).collect();
    let test: Vec<usize> = (0..files.len()).filter(|i| !split || i % 2 == 1).collect();
    let lang = F16Lang::learn(&codes, grammar_codes(files[0].tag), train.iter().map(|&i| &cols[i])).marked(files[0].tag);
    let mut r = Report { files: test.len(), ..Report::default() };
    for &i in &test {
        let f = files[i];
        let c = &cols[i];
        r.nodes += c.len() as u64;
        r.source += f.source.len() as u64;
        // The best of three, each.
        let best = |r: &mut crate::measure::Cost, x: crate::measure::Cost| {
            if r.nanos == 0 || x.nanos < r.nanos {
                *r = x;
            }
        };
        let (mut b16, mut bf) = (crate::measure::Cost::default(), crate::measure::Cost::default());
        let mut out = None;
        for _ in 0..3 {
            let ((s, z), y) = if lang.widths == [8, 8, 8] {
                let (w, y) = cost(|| drive::<1>(&lang, c));
                (cost(|| w.finish()), y)
            } else {
                let (w, y) = cost(|| drive::<0>(&lang, c));
                (cost(|| w.finish()), y)
            };
            let mut t = y;
            t += z;
            best(&mut b16, t);
            best(&mut bf, z);
            out = Some(s);
        }
        r.f16 += b16;
        r.finish += bf;
        r.wrong += usize::from(out.unwrap() != F16::new(c, &lang).streams());
        #[cfg(feature = "lean")]
        if f.tag == 13 {
            let src = std::str::from_utf8(&f.source).unwrap_or("");
            let mut bl = crate::measure::Cost::default();
            for _ in 0..3 {
                let (img, x) = cost(|| rerail_rr_syntax::lean::tape(src));
                std::hint::black_box(img.expect("the lean parser writes format 16"));
                best(&mut bl, x);
            }
            r.lean += bl;
            r.lean_files += 1;
        }
    }
    r
}

pub fn print_header() {
    println!("| language | files | nodes | format 16's writer, instructions / ns a node (of which finish) | lean parse + format 16 write | bytes right |");
    println!("|---|---|---|---|---|---|");
}

pub fn print(lang: &str, r: &Report) {
    let nn = r.nodes.max(1) as f64;
    let per = |c: &crate::measure::Cost| format!("{:.0} / {:.1}", c.instructions as f64 / nn, c.nanos as f64 / nn);
    let a = if r.lean_files > 0 { per(&r.lean) } else { String::new() };
    println!("| {lang} | {} | {} | {} ({}) | {a} | {} |", r.files, r.nodes, per(&r.f16), per(&r.finish), if r.wrong == 0 { "yes".into() } else { format!("NO ({})", r.wrong) });
}
