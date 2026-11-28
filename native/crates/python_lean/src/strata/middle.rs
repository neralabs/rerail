//! The lexer of strata from the middle (decision 0698, on 0715's passes): a chunk of blocks lexed from its entry, the
//! state of every stratum where it starts (`Entry`), and the chunks lexed one after another, each from the one before's
//! exit (`lex_chunked`), which is the serial lexer's records.
//!
//! Stratum 2 is variant B's: its carries are a few bits, and an f-string is a plain string to it, its interpolations
//! read by the walk inside the string's token. So what crosses a chunk's start is stratum 1's three escape bits,
//! variant B's machines' states and carries, the bracket depth, and the walk's state between two items (where the next
//! starts, whether the line has a token, whether it is a block's header, the indents open). A text B reads wrongly is
//! read serially by variant A (`Chunked::NotB`), as `lex_with` reads it.

use super::{B, Masks, RB, S1, Text, Walk, classes_in};
use super::{Variant, lex_with};
use crate::lex::{Extra, Lexed, Refusal, Tok};

/// The state of the strata where a chunk starts at block `from`: stratum 1's carries into block `from - 2`, variant
/// B's and the depth's into block `from - 1` (the walk reads the depth below its first item, and stratum 2 a string's
/// prefix in the block below its quote), and the walk's at its first item from byte `64 * from` on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub(crate) s1: S1,
    pub(crate) rb: RB,
    pub(crate) depth: i8,
    p: usize,
    open: bool,
    header: bool,
    indents: Vec<u32>,
    done: bool,
}

impl Entry {
    /// The text's start.
    pub fn start() -> Entry {
        Entry { s1: S1::default(), rb: RB::default(), depth: 0, p: 0, open: false, header: false, indents: vec![0], done: false }
    }
}

/// The block stratum 1 starts a chunk at `from` from, and the one strata 2 and 3 start it from.
fn firsts(from: usize) -> (usize, usize) {
    let r0 = from.saturating_sub(1);
    (r0.saturating_sub(1), r0)
}

/// A chunk lexed.
#[derive(Debug, PartialEq, Eq)]
pub enum Chunked {
    /// Its records written, and the entry of the chunk after it.
    Lexed(Entry),
    /// Variant B's reading of the text is wrong: it is read by variant A, from its start.
    NotB,
    /// The text is refused.
    Refused(Refusal),
}

/// Blocks `from..to` of `src` lexed from `e`: their items' records after `toks` and `extras` (an item that starts in
/// them read to its end, past them where it runs past), and the entry of the chunk at `to`. The strata are made over
/// a window of the blocks from `from - 2` to one past `to`, and over a longer one where the walk reads past it.
pub fn lex_chunk(src: &[u8], from: usize, to: usize, e: &Entry, toks: &mut Vec<Tok>, extras: &mut Vec<Extra>) -> Chunked {
    let blocks = src.len().div_ceil(64);
    let to = to.min(blocks);
    let (c0, r0) = firsts(from);
    let (c0_next, r0_next) = firsts(to);
    let mut r1 = (to + 1).min(blocks);
    loop {
        // Stratum 1 over `c0..r1 + 1` (stratum 2 reads the block after each of its own), its carries into the next
        // chunk's first kept.
        let mut b: Vec<B> = Vec::with_capacity(r1 + 2 - c0);
        let (mut s1, mut s1_next) = (e.s1, S1::default());
        classes_in(src, c0, (r1 + 1).min(blocks), &mut s1, &mut b, c0_next, &mut s1_next);
        if c0_next >= blocks {
            s1_next = s1;
        }
        if r1 == blocks {
            b.push(B::default());
        }
        let mut t = Text { src, b, first: c0 };
        // Strata 2 and 3 over `r0..r1`, their carries into the next chunk's first kept.
        let (mut rb, mut depth) = (e.rb, e.depth);
        let split = r0_next.clamp(r0, r1);
        if !t.regions_b_in(r0, split, &mut rb) {
            return Chunked::NotB;
        }
        let rb_next = rb;
        if !t.regions_b_in(split, r1, &mut rb) {
            return Chunked::NotB;
        }
        if r1 == blocks && (rb.t0 != 0 || rb.s0 != 0) {
            return Chunked::NotB;
        }
        if let Err(r) = t.depth_in(r0, split, &mut depth) {
            return Chunked::Refused(r);
        }
        let depth_next = depth;
        if let Err(r) = t.depth_in(split, r1, &mut depth) {
            return Chunked::Refused(r);
        }
        // The walk, over the blocks strata 2 and 3 made (and the one past the text's end, where the window reaches it).
        let short = core::cell::Cell::new(false);
        let upto = if r1 == blocks { r1 + 1 } else { r1 };
        let masks = Masks { src, b: &t.b[r0 - c0..upto - c0], first: r0, short: &short };
        let stop = if to >= blocks { usize::MAX } else { 64 * to };
        let mut w = Walk { t: masks, toks: Vec::new(), extras: Vec::new(), p: e.p, depth: 0, indents: e.indents.clone(), inside_interpolated_string: false, outer: None, b: true, b_wrong: false, stop, done: e.done };
        let r = if e.done { Ok((e.open, e.header)) } else { w.module(e.open, e.header) };
        if short.get() {
            assert!(r1 < blocks, "a read below the chunk's window: block {r0} on, from byte {}", e.p);
            r1 = (r0 + 2 * (r1 - r0)).min(blocks);
            continue;
        }
        if w.b_wrong {
            return Chunked::NotB;
        }
        let (open, header) = match r {
            Ok(x) => x,
            Err(r) => return Chunked::Refused(r),
        };
        toks.append(&mut w.toks);
        extras.append(&mut w.extras);
        return Chunked::Lexed(Entry { s1: s1_next, rb: rb_next, depth: depth_next, p: w.p, open, header, indents: w.indents, done: w.done });
    }
}

/// `src` lexed in chunks of `blocks` blocks, each from the entry the one before leaves: the serial lexer's records
/// (variant B's; none where B's reading is wrong), and the chunks' entries.
pub fn lex_chunked(src: &[u8], blocks: usize) -> Option<(Result<Lexed, Refusal>, Vec<Entry>)> {
    let n = src.len().div_ceil(64);
    let (mut toks, mut extras) = (Vec::new(), Vec::new());
    let mut e = Entry::start();
    let mut entries = Vec::new();
    let mut from = 0;
    loop {
        entries.push(e.clone());
        let to = from + blocks.max(1);
        match lex_chunk(src, from, to, &e, &mut toks, &mut extras) {
            Chunked::Lexed(x) => e = x,
            Chunked::NotB => return None,
            Chunked::Refused(r) => return Some((Err(r), entries)),
        }
        if to >= n {
            return Some((Ok(Lexed { toks, extras }), entries));
        }
        from = to;
    }
}

/// The chunked lexer's records against the serial one's (variant B's), at each chunk size: none where B's reading
/// is wrong (then the serial lexer reads it by A). Whether the chunks gave the serial records, as text where not.
pub fn chunked_is_serial(src: &[u8], blocks: usize) -> Result<bool, String> {
    let Some((got, _)) = lex_chunked(src, blocks) else { return Ok(false) };
    let want = lex_with(src, Variant::B);
    let same = match (&got, &want) {
        (Ok(a), Ok(b)) => a.toks.iter().map(|t| (t.kind, t.soft, t.start, t.end)).eq(b.toks.iter().map(|t| (t.kind, t.soft, t.start, t.end))) && a.extras.iter().map(|e| (e.start, e.end, e.comment)).eq(b.extras.iter().map(|e| (e.start, e.end, e.comment))),
        // Where each refuses is the first refusal of its own order of reading; either way the text goes to tree-sitter.
        (Err(_), Err(_)) => true,
        _ => false,
    };
    if same {
        return Ok(true);
    }
    let show = |r: &Result<Lexed, Refusal>| match r {
        Ok(l) => l.text(src),
        Err(r) => format!("refused: {r:?}"),
    };
    let (a, b) = (show(&got), show(&want));
    let k = a.lines().zip(b.lines()).position(|(x, y)| x != y).unwrap_or(0);
    let around = |t: &str| t.lines().skip(k.saturating_sub(2)).take(5).collect::<Vec<_>>().join("\n");
    Err(format!("{blocks} blocks a chunk, {:?}:\n  chunked:\n{}\n  serial:\n{}", String::from_utf8_lossy(src), around(&a), around(&b)))
}

/// Stratum 1's carries into block `k`, from the bytes below it alone (no lane: they are the bytes'): an odd run of
/// backslashes, a `'\r'` an odd run escapes, an odd run of `{`, each ending at the block's last byte below it.
pub(crate) fn s1_at(src: &[u8], k: usize) -> S1 {
    let at = 64 * k;
    // The length of the run of `c` ending just below `i`, odd.
    let odd = |i: usize, c: u8| (src[..i.min(src.len())].iter().rev().take_while(|&&x| x == c).count() & 1) as u64;
    if k == 0 {
        return S1::default();
    }
    let cr = u64::from(src.get(at - 1) == Some(&b'\r')) & odd(at - 1, b'\\');
    S1 { carry: odd(at, b'\\'), cr, lb: odd(at, b'{') }
}

/// A bracket depth's run over a range of code, as a function of the depth it enters with (the lean lexer's counter,
/// which a closer clamps at 0): out goes `max(d + net, net - min)`, and the deepest it goes is `max(d + max, k)`.
/// `min` and `max` are the lowest and highest of the range's running sums (each with the empty one, 0), `k` the most
/// a sum rises above the lowest before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Depth {
    net: i32,
    min: i32,
    max: i32,
    k: i32,
}

impl Depth {
    /// The range `a` then `b`.
    pub fn then(a: Depth, b: Depth) -> Depth {
        Depth { net: a.net + b.net, min: a.min.min(a.net + b.min), max: a.max.max(a.net + b.max), k: a.k.max(b.k).max(a.net - a.min + b.max) }
    }

    /// The depth out of the range from `d`, none where it goes past the bound (refused).
    pub fn apply(self, d: i8) -> Option<i8> {
        let d = i32::from(d);
        if (d + self.max).max(self.k) > i32::from(super::MAX_DEPTH) {
            return None;
        }
        Some((d + self.net).max(self.net - self.min) as i8)
    }

    /// A block's brackets in code, opener `up`, closer `down`.
    fn of_block(up: u64, down: u64) -> Depth {
        let (mut x, mut s, mut lo) = (Depth::default(), 0i32, 0i32);
        let mut m = up | down;
        while m != 0 {
            let i = m.trailing_zeros();
            m &= m - 1;
            s += if up >> i & 1 == 1 { 1 } else { -1 };
            lo = lo.min(s);
            x.min = x.min.min(s);
            x.max = x.max.max(s);
            x.k = x.k.max(s - lo);
        }
        x.net = s;
        x
    }
}

/// The values of variant B's carries a chunk can be entered with, a lane each: the triples' machine's state (3), the
/// one-line strings' (3), the comment's carry, the line-end bit, the triple closer's taken bits (4). B's layers each
/// read as if the ones above were not there, so the machines' states do not exclude each other (a quote in a comment
/// moves the strings' machine).
pub const LANES: usize = 144;

/// Lane `l`'s carries.
pub(crate) fn lane_rb(l: usize) -> RB {
    let l = l as u64;
    RB { t0: (l / 48) as u8, s0: (l / 16 % 3) as u8, ccarry: l / 8 & 1, cext_prev: l / 4 & 1, tr_taken: l & 3 }
}

/// The lane of carries `c`, none where they are no lane's.
pub(crate) fn rb_lane(c: RB) -> Option<usize> {
    (c.t0 < 3 && c.s0 < 3 && c.ccarry < 2 && c.cext_prev < 2 && c.tr_taken < 4)
        .then(|| usize::from(c.t0) * 48 + usize::from(c.s0) * 16 + c.ccarry as usize * 8 + c.cext_prev as usize * 4 + c.tr_taken as usize)
}

/// A chunk's summary (pass 1): for each lane of the carries it can be entered with, the lane it leaves in and its
/// depth's run, none where B's reading from that entry is wrong.
pub type Summary = [Option<(u8, Depth)>; LANES];

/// The range of blocks strata 2 and 3 cover for the chunk `from..to` (`blocks` the text's): from the block below its
/// first to the one below the next chunk's, the last chunk to the text's end.
fn region_range(from: usize, to: usize, blocks: usize) -> (usize, usize) {
    let (_, r0) = firsts(from);
    (r0, if to >= blocks { blocks } else { firsts(to).1 })
}

/// Pass 1: chunk `from..to`'s summary, from every lane at once, by its bytes alone (stratum 1's carries read from the
/// bytes below it).
pub fn summary(src: &[u8], from: usize, to: usize) -> Summary {
    let blocks = src.len().div_ceil(64);
    let to = to.min(blocks);
    let (c0, _) = firsts(from);
    let (r0, r1) = region_range(from, to, blocks);
    let mut b: Vec<B> = Vec::with_capacity(r1 + 2 - c0);
    classes_in(src, c0, (r1 + 1).min(blocks), &mut s1_at(src, c0), &mut b, usize::MAX, &mut S1::default());
    if r1 == blocks {
        b.push(B::default());
    }
    core::array::from_fn(|l| {
        let mut t = Text { src, b: b.clone(), first: c0 };
        let mut c = lane_rb(l);
        if !t.regions_b_in(r0, r1, &mut c) || (r1 == blocks && (c.t0 != 0 || c.s0 != 0)) {
            return None;
        }
        let d = t.b[r0 - c0..r1 - c0].iter().fold(Depth::default(), |d, b| Depth::then(d, Depth::of_block(b.ob & b.code, b.cb & b.code)));
        Some((rb_lane(c)? as u8, d))
    })
}

/// The order a pass runs its chunks in: a permutation of `0..n`, asked each time a pass runs them (0726's `Order`).
/// Each chunk is on its own, so any order gives the same answer.
pub type Order<'a> = &'a dyn Fn(usize) -> Vec<usize>;

/// The chunks in order.
pub fn in_order(n: usize) -> Vec<usize> {
    (0..n).collect()
}

/// `f` of each of `n` chunks, run in `order`'s order (0726's `each`).
fn each<T>(n: usize, order: Order<'_>, mut f: impl FnMut(usize) -> T) -> Vec<T> {
    let mut out: Vec<Option<T>> = (0..n).map(|_| None).collect();
    for c in order(n) {
        assert!(out[c].is_none(), "a chunk run twice");
        out[c] = Some(f(c));
    }
    out.into_iter().map(|x| x.expect("every chunk run")).collect()
}

/// Summary `a` then `b`: each lane of `a` on in the lane of `b` it leaves in.
pub fn compose(a: &Summary, b: &Summary) -> Summary {
    core::array::from_fn(|l| a[l].and_then(|(e, d)| b[usize::from(e)].map(|(e2, d2)| (e2, Depth::then(d, d2)))))
}

/// Pass 3: each chunk's carries into its first block of strata 2 and 3 and its depth there, from the chunks' summaries,
/// by Hillis and Steele's scan of their compositions (JSON's `entries_in`), each round's chunks in `order`'s order.
/// None from the first chunk whose entry B reads wrongly or whose depth goes past the bound.
pub fn entries_in(sums: &[Summary], order: Order<'_>) -> Vec<Option<(RB, i8)>> {
    let mut acc = sums.to_vec();
    let mut d = 1;
    while d < acc.len() {
        acc = each(acc.len(), order, |c| if c < d { acc[c] } else { compose(&acc[c - d], &acc[c]) });
        d *= 2;
    }
    let start = rb_lane(RB::default()).expect("the start's lane");
    each(acc.len(), order, |c| {
        if c == 0 {
            return Some((RB::default(), 0));
        }
        let (e, dep) = acc[c - 1][start]?;
        Some((lane_rb(usize::from(e)), dep.apply(0)?))
    })
}

/// Passes 1 and 3 against the chunks lexed one after another: whether every chunk's entry by the scan of the
/// summaries (each pass in `order`) is the one the chunk before left, and stratum 1's carries read from the bytes the
/// ones it carried. None where B's reading is wrong (the text is read by A) or the text is refused.
pub fn entries_are_the_folds(src: &[u8], blocks: usize, order: Order<'_>) -> Option<Result<(), String>> {
    let (got, fold) = lex_chunked(src, blocks)?;
    got.ok()?;
    let n = src.len().div_ceil(64);
    let k = n.div_ceil(blocks.max(1)).max(1);
    let sums = each(k, order, |c| summary(src, c * blocks.max(1), (c + 1) * blocks.max(1)));
    let es = entries_in(&sums, order);
    for (c, (e, f)) in es.iter().zip(&fold).enumerate() {
        let from = c * blocks.max(1);
        if *e != Some((f.rb, f.depth)) {
            return Some(Err(format!("chunk {c} (block {from}) of {k}, {blocks} blocks a chunk: the scan's {e:?}, the fold's {:?}", (f.rb, f.depth))));
        }
        let s1 = s1_at(src, firsts(from).0);
        if s1 != f.s1 {
            return Some(Err(format!("chunk {c} (block {from}): stratum 1's carries from the bytes {s1:?}, carried {:?}", f.s1)));
        }
    }
    Some(Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The texts the tests lex from the middle: each region, bracket, indent and long token crossing a block's edge
    /// somewhere.
    fn texts() -> Vec<String> {
        let mut texts: Vec<String> = [
            "if a:\n    b\nc\n",
            "x = (1,\n 2)\n",
            "if a:\n    b\n    # c\nd\n",
            "f'a{b!r:>{w}}c'\n",
            "x = 'a # b' # c ( \ny = [1, # ]\n 2]\n",
            "t = '''a\n'' b ( '''\nu = \"\"\"a\\\"\"\" b\"\"\"\n",
            "def f():\n    return [\n        1,\n    ]\n\nclass C:\n    pass\n",
            "x = 1 \\\n  + 2\n",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // A text across many blocks: each region, bracket, indent and long token crossing a block's edge somewhere.
        let mut long = String::new();
        for k in 0..60 {
            long.push_str(&format!("x{k} = f(a, '{}', \"\"\"q\n{}\"\"\", # c ( {k}\n  [{}])  # e\n", "s".repeat(k), "t".repeat(k * 3), "1, ".repeat(k)));
            long.push_str(&format!("if x{k}:\n{}y = rf'{}{{x{k}}}'\n{}name_{} = 0\n", " ".repeat(k % 9 + 1), "\\\\".repeat(k), " ".repeat(k % 9 + 1), "n".repeat(k * 2)));
        }
        texts.push(long);
        texts
    }

    #[test]
    fn chunks_lexed_from_their_entries_are_the_serial_lexer() {
        let (mut same, mut not_b) = (0, 0);
        for t in &texts() {
            for blocks in [1, 2, 3, 16] {
                match chunked_is_serial(t.as_bytes(), blocks) {
                    Ok(true) => same += 1,
                    Ok(false) => not_b += 1,
                    Err(e) => panic!("{e}"),
                }
            }
        }
        assert_eq!(not_b, 0, "{same} the same");
    }

    #[test]
    fn random_texts_chunked_are_the_serial_lexer() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut checked = 0;
        for len in [63usize, 64, 65, 130, 400, 1000] {
            for _ in 0..200 {
                let src: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        b"''''\\\"\"\"\"#\n\n  ab(rf{}x)\n    :"[(x >> 8) as usize % 29]
                    })
                    .collect();
                for blocks in [1, 2, 3] {
                    checked += usize::from(chunked_is_serial(&src, blocks).unwrap_or_else(|e| panic!("{e}")));
                }
            }
        }
        assert!(checked > 0);
    }

    /// A seeded random order of `n` chunks, another each time it is asked.
    fn shuffled(seed: u64) -> impl Fn(usize) -> Vec<usize> {
        let x = core::cell::Cell::new(seed | 1);
        move |n| {
            let mut v: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                let mut y = x.get();
                y ^= y << 13;
                y ^= y >> 7;
                y ^= y << 17;
                x.set(y);
                v.swap(i, (y % (i as u64 + 1)) as usize);
            }
            v
        }
    }

    #[test]
    fn the_scan_of_summaries_gives_each_chunk_its_entry() {
        let mut checked = 0;
        for (i, t) in texts().iter().enumerate() {
            for blocks in [1, 2, 3, 16] {
                let order = shuffled(i as u64 * 31 + blocks as u64);
                match entries_are_the_folds(t.as_bytes(), blocks, &order) {
                    Some(Ok(())) => checked += 1,
                    Some(Err(e)) => panic!("{e}"),
                    None => {}
                }
            }
        }
        assert!(checked > 20, "{checked}");
        // Random texts: quotes, comments, escapes and brackets across blocks' edges.
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for len in [65usize, 130, 400, 1000] {
            for k in 0..200 {
                let src: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        b"''\\\"\"#\n\n  ab(rf{}x)\n\r[]"[(x >> 8) as usize % 23]
                    })
                    .collect();
                for blocks in [1, 2, 3] {
                    if let Some(Err(e)) = entries_are_the_folds(&src, blocks, &shuffled(k + len as u64)) {
                        panic!("{e}: {:?}", String::from_utf8_lossy(&src));
                    }
                }
            }
        }
    }
}
