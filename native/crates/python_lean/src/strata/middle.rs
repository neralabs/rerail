//! The lexer of strata from the middle (decision 0698, on 0715's passes): a chunk of blocks lexed from its entry, the
//! state of every stratum where it starts (`Entry`), and the chunks lexed one after another, each from the one before's
//! exit (`lex_chunked`), which is the serial lexer's records.
//!
//! Stratum 2 is variant B's: its carries are a few bits, and an f-string is a plain string to it, its interpolations
//! read by the walk inside the string's token. So what crosses a chunk's start is stratum 1's three escape bits,
//! variant B's machines' states and carries, the bracket depth, and the walk's state between two items (where the next
//! starts, whether the line has a token, whether it is a block's header, the indents open). A text B reads wrongly is
//! read serially by variant A (`Chunked::NotB`), as `lex_with` reads it.

use super::{B, Masks, RB, S1, Stack, Text, Walk, classes_in};
use rerail_simd::prelude::*;
use super::{Variant, b_reads, lex_with};
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
    /// A walk read past the text's end: it was entered where the serial walk does not go.
    Past,
}

/// The strata over a chunk `from..to`'s window, from the carries into its first blocks (stratum 1's `s1`, B's `rb`,
/// the depth), and `f` of the walk's masks over it: the window from block `from - 2` to one past `to`, made again
/// twice as long while `f`'s reads go past it. `f`'s result, and the carries into the next chunk's first blocks; or why
/// the strata do not read the chunk.
fn over_window<R>(src: &[u8], from: usize, to: usize, s1: S1, rb: RB, depth: i8, mut f: impl FnMut(Masks<'_>) -> R) -> Result<(R, S1, RB, i8), Chunked> {
    let blocks = src.len().div_ceil(64);
    let to = to.min(blocks);
    let (c0, r0) = firsts(from);
    let (c0_next, r0_next) = firsts(to);
    let mut r1 = (to + 1).min(blocks).max(r0);
    loop {
        // Stratum 1 over `c0..r1 + 1` (stratum 2 reads the block after each of its own), its carries into the next
        // chunk's first kept.
        let mut b: Vec<B> = Vec::with_capacity(r1 + 2 - c0);
        let (mut s1, mut s1_next) = (s1, S1::default());
        classes_in(src, c0, (r1 + 1).min(blocks), &mut s1, &mut b, c0_next, &mut s1_next);
        if c0_next >= blocks {
            s1_next = s1;
        }
        if r1 == blocks {
            b.push(B::default());
        }
        let mut t = Text { src, b, first: c0 };
        // Strata 2 and 3 over `r0..r1`, their carries into the next chunk's first kept.
        let (mut rb, mut depth) = (rb, depth);
        let split = r0_next.clamp(r0, r1);
        if !t.regions_b_in(r0, split, &mut rb) {
            return Err(Chunked::NotB);
        }
        let rb_next = rb;
        if !t.regions_b_in(split, r1, &mut rb) {
            return Err(Chunked::NotB);
        }
        if r1 == blocks && (rb.t0 != 0 || rb.s0 != 0) {
            return Err(Chunked::NotB);
        }
        t.depth_in(r0, split, &mut depth).map_err(Chunked::Refused)?;
        let depth_next = depth;
        t.depth_in(split, r1, &mut depth).map_err(Chunked::Refused)?;
        // The walk's masks: the blocks strata 2 and 3 made (and the one past the text's end, where the window reaches
        // it).
        let short = core::cell::Cell::new(false);
        let upto = if r1 == blocks { r1 + 1 } else { r1 };
        let r = f(Masks { src, b: &t.b[r0 - c0..upto - c0], first: r0, short: &short });
        if short.get() {
            if r1 == blocks {
                // A read past the text's end: a walk from a place that is not the serial walk's.
                return Err(Chunked::Past);
            }
            r1 = (r0 + 2 * (r1 - r0).max(1)).min(blocks);
            continue;
        }
        return Ok((r, s1_next, rb_next, depth_next));
    }
}

/// What a walk over a chunk leaves: its records, where it stopped, its `open` and `header`, its indents, whether the
/// text's end was read, and whether B's reading was found wrong; or its refusal.
type Walked<S> = (Result<(bool, bool), Refusal>, Vec<Tok>, Vec<Extra>, usize, S, bool, bool);

/// The walk from `p` (`open`, `header`, the indents `indents`) over masks `t` to its first item from `stop` on.
fn walk<S: Stack>(t: Masks<'_>, p: usize, open: bool, header: bool, indents: S, done: bool, stop: usize) -> Walked<S> {
    let mut w = Walk { t, toks: Vec::new(), extras: Vec::new(), p, depth: 0, indents, inside_interpolated_string: false, outer: None, b: true, b_wrong: false, stop, done };
    let r = if done { Ok((open, header)) } else { w.module(open, header) };
    (r, w.toks, w.extras, w.p, w.indents, w.done, w.b_wrong)
}

/// The walk's stop for chunk `..to`: its end's byte, none for the text's last chunk.
fn stop_of(src: &[u8], to: usize) -> usize {
    if to >= src.len().div_ceil(64) { usize::MAX } else { 64 * to }
}

/// Blocks `from..to` of `src` lexed from `e`: their items' records after `toks` and `extras` (an item that starts in
/// them read to its end, past them where it runs past), and the entry of the chunk at `to`.
pub fn lex_chunk(src: &[u8], from: usize, to: usize, e: &Entry, toks: &mut Vec<Tok>, extras: &mut Vec<Extra>) -> Chunked {
    let stop = stop_of(src, to);
    match over_window(src, from, to, e.s1, e.rb, e.depth, |m| walk(m, e.p, e.open, e.header, e.indents.clone(), e.done, stop)) {
        Err(c) => c,
        Ok(((_, _, _, _, _, _, true), ..)) => Chunked::NotB,
        Ok(((Err(r), ..), ..)) => Chunked::Refused(r),
        Ok(((Ok((open, header)), mut t, mut x, p, indents, done, false), s1, rb, depth)) => {
            toks.append(&mut t);
            extras.append(&mut x);
            Chunked::Lexed(Entry { s1, rb, depth, p, open, header, indents, done })
        }
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
            Chunked::Past => panic!("chunk at block {from} read past the text's end from its entry"),
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
        let (out, ok) = self.apply_sel(d);
        ok.then_some(out)
    }

    /// `apply` with no branch: the depth out, and whether it stays within the bound.
    #[inline(always)]
    fn apply_sel(self, d: i8) -> (i8, bool) {
        let d = i32::from(d);
        ((d + self.net).max(self.net - self.min) as i8, (d + self.max).max(self.k) <= i32::from(super::MAX_DEPTH))
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
    RB { t0: (l / 48) as u8, s0: (l / 16 % 3) as u8, ccarry: (l / 8) & 1, cext_prev: (l / 4) & 1, tr_taken: l & 3 }
}

/// The lane of carries `c`, none where they are no lane's.
pub(crate) fn rb_lane(c: RB) -> Option<usize> {
    (c.t0 < 3 && c.s0 < 3 && c.ccarry < 2 && c.cext_prev < 2 && c.tr_taken < 4)
        .then(|| usize::from(c.t0) * 48 + usize::from(c.s0) * 16 + c.ccarry as usize * 8 + c.cext_prev as usize * 4 + c.tr_taken as usize)
}

/// A chunk's summary (pass 1): for each lane of the carries it can be entered with, the lane it leaves in and its
/// depth's run, none where B's reading from that entry is wrong.
pub type PerLane = [Option<(u8, Depth)>; LANES];

/// A chunk's summary (pass 1's, which pass 3 composes): its map over every lane, a block map's form.
pub type Summary = BlockMap;

impl BlockMap {
    /// The map of per-lane runs (`WRONG` for none).
    pub fn of_lanes(p: &PerLane) -> BlockMap {
        BlockMap { exit: core::array::from_fn(|l| p[l].map_or(WRONG as u8, |(e, _)| e)), depth: core::array::from_fn(|l| p[l].map_or(Depth::default(), |(_, d)| d)) }
    }

    /// Each lane's run, none at `WRONG`.
    pub fn lanes(&self) -> PerLane {
        core::array::from_fn(|l| (usize::from(self.exit[l]) != WRONG).then_some((self.exit[l], self.depth[l])))
    }
}

/// The range of blocks strata 2 and 3 cover for the chunk `from..to` (`blocks` the text's): from the block below its
/// first to the one below the next chunk's, the last chunk to the text's end.
fn region_range(from: usize, to: usize, blocks: usize) -> (usize, usize) {
    let (_, r0) = firsts(from);
    (r0, if to >= blocks { blocks } else { firsts(to).1 })
}

/// Pass 1: chunk `from..to`'s summary, from every lane at once, by its bytes alone (stratum 1's carries read from the
/// bytes below it).
pub fn summary(src: &[u8], from: usize, to: usize) -> PerLane {
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

/// The lane a carries' reading B finds wrong goes to: it stays there (pass 1 superposed).
const WRONG: usize = LANES;

/// A block's map over every lane of B's carries at once: the lane each leaves in (`WRONG` where B's reading from it
/// is wrong) and its depth's run. Every lane of it is written by the block's lanes, none cleared first.
#[derive(Clone, Copy)]
pub struct BlockMap {
    exit: [u8; LANES],
    depth: [Depth; LANES],
}

impl Depth {
    /// A block's brackets in code by masks: the prefix sums over its 64 bytes (`depth_lanes`'), their prefix
    /// minimum from 0, and the reductions, with no branch.
    #[inline(always)]
    fn of_masks(up: u64, down: u64) -> Depth {
        let (u, d) = (super::lanes(up), super::lanes(down));
        let (mut c, mut m) = (super::D::splat(0), super::D::splat(0));
        let (mut lo, mut hi, mut k) = (super::D::splat(0), super::D::splat(0), super::D::splat(0));
        for r in 0..4 {
            let s = super::prefix_sum(d[r] - u[r]) + c;
            c = super::D::splat(s[15]);
            let pm = super::prefix_min(s).simd_min(m);
            m = super::D::splat(pm[15]);
            lo = lo.simd_min(s);
            hi = hi.simd_max(s);
            k = k.simd_max(s - pm);
        }
        Depth { net: i32::from(c[0]), min: i32::from(lo.reduce_min()), max: i32::from(hi.reduce_max()), k: i32::from(k.reduce_max()) }
    }
}

/// `$body` once for each value of `$v` in the list, unrolled: the lanes' layers are each a constant, not a loop.
macro_rules! each_of {
    ($v:ident in [$($x:expr),*] $body:block) => {
        $( { let $v = $x; $body } )*
    };
}

/// A block's map over every lane (pass 1 superposed), from the block below it (`prev`, a zero block below the
/// text's start), the block and the one after it: B's layers for every value of their carries, each layer once for
/// each state of the layers it reads (the triples' machine once and from each of its 3 states, the strings' machine
/// for each of the triples' 12, the comments' addition for each of the strings' 36), B's checks as masks, and each
/// lane's depth by masks. No branch and no loop: the lanes are unrolled, and where B's reading is wrong the lane's
/// exit is selected as `WRONG`.
#[inline(never)]
pub(super) fn block_map(prev: &B, blk: &B, b1: &B) -> BlockMap {
    let (pqs, pqd) = (prev.sq & !prev.esc, prev.dq & !prev.esc);
    let (qs, qd) = (blk.sq & !blk.esc, blk.dq & !blk.esc);
    let runs = |q: u64, nq: u64, pq: u64| {
        let (q1, q2, q3) = (q >> 1 | nq << 63, q >> 2 | nq << 62, q >> 3 | nq << 61);
        (q & q1 & q2 & !(q << 1 | pq >> 63) & !q3, q & q1 & q2 & q3)
    };
    let (r3s, r4s) = runs(qs, b1.sq & !b1.esc, pqs);
    let (r3d, r4d) = runs(qd, b1.dq & !b1.esc, pqd);
    let all_wrong = (r4s | r4d) != 0;
    let r3 = r3s | r3d;
    let m3 = super::machine(r3s, r3d, 0);
    // The prefixes' window: a run of prefix letters below an opener that reaches the window's bottom is B's
    // refusal of the reading (`prefixes_b`'s none).
    let x = super::win(prev.pfx, blk.pfx).reverse_bits();
    let mut exit_of = [0u8; LANES];
    let mut depth: [Depth; LANES] = [Depth::default(); LANES];
    each_of!(t0 in [0u8, 1, 2] {
        let (tl, th, t1) = super::run_from(&m3, t0);
        let ta = tl | th;
        let tb = ta << 1 | u64::from(t0 != 0);
        let t_close = r3 & tb & !ta;
        let t_open = r3 & !tb;
        let tr_out = (t_close >> 62 | t_close >> 63) as usize;
        each_of!(tr_taken in [0u64, 1, 2, 3] {
            let tr = tb | ta | t_close << 1 | t_close << 2 | tr_taken;
            let ms = super::machine(qs & !tr, qd & !tr, blk.bare_lf);
            each_of!(s0 in [0u8, 1, 2] {
                let (sl, sh, s1) = super::run_from(&ms, s0);
                let sa = sl | sh;
                let sb = sa << 1 | u64::from(s0 != 0);
                let g = blk.hash & !tr & !sb;
                let a = !blk.eol | g;
                each_of!(ccarry in [0u64, 1] {
                    let (y, o1) = a.overflowing_add(g);
                    let (y, o2) = y.overflowing_add(ccarry);
                    let cc_out = usize::from(o1 | o2);
                    let comment = g | (a & (y ^ a ^ g));
                    each_of!(cext_prev in [0u64, 1] {
                        let cext = comment | (blk.eol & !blk.lf & (comment << 1 | cext_prev));
                        let lend = cext << 1 | cext_prev;
                        let wrong = (t_open & (sb | comment)) | (blk.bare_lf & sb & !tr & !lend) | (blk.lf & blk.esc & sb & lend) | (blk.bt & !blk.esc & !tr & !sb & !comment);
                        let s_open = (qs | qd) & !tr & !sb & !comment;
                        let o = super::win(0, s_open | t_open);
                        let (_, over) = x.overflowing_add((o.reverse_bits() << 1) & x);
                        let code = !(comment | ((sb | sa) & !comment & !tr) | tr);
                        let l = usize::from(t0) * 48 + usize::from(s0) * 16 + ccarry as usize * 8 + cext_prev as usize * 4 + tr_taken as usize;
                        let exit = usize::from(t1) * 48 + usize::from(s1) * 16 + cc_out * 8 + (cext >> 63) as usize * 4 + tr_out;
                        let bad = u8::from(all_wrong | (wrong != 0) | over);
                        // A select by mask: `WRONG` where bad.
                        let keep = bad.wrapping_sub(1);
                        exit_of[l] = (exit as u8 & keep) | (WRONG as u8 & !keep);
                        depth[l] = Depth::of_masks(blk.ob & code, blk.cb & code);
                    });
                });
            });
        });
    });
    BlockMap { exit: exit_of, depth }
}

/// The lanes' runs through the blocks so far (`exit`, `dep`, from each lane of the chunk's entry), on through block
/// map `m`: each lane on in the lane it left the block before in, by gathers, unrolled over the lanes. A lane at
/// `WRONG` stays there (its index clamped for the gather, its exit selected by mask).
#[inline(never)]
fn then_block(exit: &mut [u8; LANES], dep: &mut [Depth; LANES], m: &BlockMap) {
    each_of!(hi in [0usize, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] {
        each_of!(lo in [0usize, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] {
            let l = hi * 12 + lo;
            let e = exit[l];
            let wrong = u8::from(usize::from(e) == WRONG);
            let at = usize::from(e.min(LANES as u8 - 1));
            dep[l] = Depth::then(dep[l], m.depth[at]);
            let keep = wrong.wrapping_sub(1);
            exit[l] = (m.exit[at] & keep) | (WRONG as u8 & !keep);
        });
    });
}

/// Pass 1 superposed: chunk `from..to`'s summary from every lane at once, its blocks' maps composed in the outer
/// block loop by gathers (each lane on in the lane it left the block before in), with no branch.
pub fn summary_superposed(src: &[u8], from: usize, to: usize) -> Summary {
    let blocks = src.len().div_ceil(64);
    let to = to.min(blocks);
    let (c0, _) = firsts(from);
    let (r0, r1) = region_range(from, to, blocks);
    // A zero block below the text's start: the block below every block of the range is in `b`.
    let lead = usize::from(r0 == c0);
    let mut b: Vec<B> = Vec::with_capacity(r1 + 3 - c0);
    b.resize(lead, B::default());
    classes_in(src, c0, (r1 + 1).min(blocks), &mut s1_at(src, c0), &mut b, usize::MAX, &mut S1::default());
    if r1 == blocks {
        b.push(B::default());
    }
    let mut exit: [u8; LANES] = core::array::from_fn(|l| l as u8);
    let mut dep = [Depth::default(); LANES];
    for w in b[r0 + lead - c0 - 1..r1 + lead - c0 + 1].windows(3) {
        then_block(&mut exit, &mut dep, &block_map(&w[0], &w[1], &w[2]));
    }
    // At the text's end, a string open is B's reading wrong: selected by masks.
    let at_end = u8::from(r1 == blocks);
    for e in &mut exit {
        // A lane at `WRONG` (144) has `e / 16 % 3 == 0` and `e / 48 == 3`: open, and stays `WRONG`.
        let open = u8::from(*e / 48 != 0) | u8::from(*e / 16 % 3 != 0);
        let keep = (at_end & open).wrapping_sub(1);
        *e = (*e & keep) | (WRONG as u8 & !keep);
    }
    BlockMap { exit, depth: dep }
}

/// Pass 1 superposed against B run from each lane, chunk by chunk of `blocks` blocks: the same summary, or the first
/// lane that differs.
pub fn superposed_is_per_lane(src: &[u8], blocks: usize) -> Result<(), String> {
    let n = src.len().div_ceil(64);
    let mut from = 0;
    while from < n.max(1) {
        let to = from + blocks;
        let (a, b) = (summary_superposed(src, from, to).lanes(), summary(src, from, to));
        if let Some(l) = (0..LANES).find(|&l| a[l] != b[l]) {
            return Err(format!("chunk {from}..{to}, lane {l} ({:?}): superposed {:?}, per lane {:?}", lane_rb(l), a[l], b[l]));
        }
        from = to;
    }
    Ok(())
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
    let mut x = *a;
    then_block(&mut x.exit, &mut x.depth, b);
    x
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
        let (d, ok) = acc[c - 1].depth[start].apply_sel(0);
        let e = acc[c - 1].exit[start];
        let ok = ok & (usize::from(e) != WRONG);
        ok.then(|| (lane_rb(usize::from(e)), d))
    })
}

/// Where the walk enters a chunk: at its first loop top from the chunk's start on (`Entry::p`), and whether the text's
/// end was read there (no loop top is left).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place(pub usize, pub bool);

/// Chunk `from..to`'s first loop top by masks (`starts::tops`), over its strata from `rb` and `depth`: the lowest of
/// its blocks' tops, none where it has none (the chunk after's is then its own). None where the strata do not read
/// the chunk.
fn first_top(src: &[u8], from: usize, to: usize, rb: RB, depth: i8) -> Option<Option<usize>> {
    let (c0, _) = firsts(from);
    let n = src.len().div_ceil(64);
    over_window(src, from, to, s1_at(src, c0), rb, depth, |m| {
        (from..to.min(n)).rev().fold(None, |after, bi| {
            let (t, at_end) = super::starts::tops(&m, bi);
            if t != 0 {
                Some(64 * bi + t.trailing_zeros() as usize)
            } else if at_end {
                Some(src.len())
            } else {
                after
            }
        })
    })
    .ok()
    .map(|(x, ..)| x)
}


/// A chunk's indents as its walk keeps them over an entry it does not know: the levels it opened (`own`), above the
/// entry's levels at most `t` (each of the entry's above it closed by the chunk, a dedent each, counted where the
/// entry is known), where the entry has no level in any of `conds` (each a read of the entry's top the walk made).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sym {
    own: Vec<u32>,
    t: i64,
    /// The dedents the walk wrote for its entry's levels, one a filter (the entry's count is `|S > t|`).
    phantom: usize,
    conds: Vec<(i64, i64)>,
}

impl Sym {
    fn new() -> Sym {
        Sym { own: Vec::new(), t: i64::MAX, phantom: 0, conds: Vec::new() }
    }

    /// `a` then `b`, none where `b`'s reads of its entry do not hold for every entry of `a`.
    fn then(a: &Sym, b: &Sym) -> Option<Sym> {
        let mut conds = a.conds.clone();
        for &(lo, hi) in &b.conds {
            if a.own.iter().any(|&o| lo <= i64::from(o) && i64::from(o) <= hi) {
                return None;
            }
            let c = (lo, hi.min(a.t));
            if c.0 <= c.1 && !conds.contains(&c) {
                conds.push(c);
            }
        }
        let own = a.own.iter().copied().filter(|&o| i64::from(o) <= b.t).chain(b.own.iter().copied()).collect();
        Some(Sym { own, t: a.t.min(b.t), phantom: 0, conds })
    }

    /// The indents out of the range from `s`, and the dedents it wrote for `s`'s levels; none where a read of it
    /// does not hold.
    fn apply(&self, s: &[u32]) -> Option<(Vec<u32>, usize)> {
        if self.conds.iter().any(|&(lo, hi)| s.iter().any(|&x| lo <= i64::from(x) && i64::from(x) <= hi)) {
            return None;
        }
        let dedents = s.iter().filter(|&&x| i64::from(x) > self.t).count();
        let mut out: Vec<u32> = s.iter().copied().filter(|&x| i64::from(x) <= self.t).collect();
        out.extend(&self.own);
        Some((out, dedents))
    }

    /// The dedents the range wrote of its own levels, of `a`'s own levels `b` closes (in `a` then `b`).
    fn closed_of(a: &Sym, b: &Sym) -> usize {
        a.own.iter().filter(|&&o| i64::from(o) > b.t).count()
    }
}

impl Stack for Sym {
    fn below(&mut self, w: u32) -> bool {
        match self.own.last() {
            Some(&c) => c < w,
            None => {
                // The entry's top below `w`: none of its levels in `[w, t]`.
                if i64::from(w) <= self.t {
                    self.conds.push((i64::from(w), self.t));
                }
                true
            }
        }
    }

    fn pops_above(&mut self, m: i64) -> bool {
        match self.own.last() {
            Some(&c) => i64::from(c) > m,
            None if m < self.t => {
                // Each of the entry's levels above `m` closed: one filter, the dedents counted where the entry is known.
                self.t = m;
                self.phantom += 1;
                true
            }
            None => false,
        }
    }

    fn above(&mut self, m: i64) -> bool {
        match self.own.last() {
            Some(&c) => i64::from(c) > m,
            None => {
                // The entry's top at most `m`: none of its levels in `(m, t]`.
                if m < self.t {
                    self.conds.push((m + 1, self.t));
                }
                false
            }
        }
    }

    fn push(&mut self, w: u32) {
        self.own.push(w);
    }

    fn pop(&mut self) {
        // A level of the entry's closes by the filter (`pops_above`).
        self.own.pop();
    }
}

/// The walk's `open` and `header` a lane of its summary enters with (a header is a line with a token).
const COMBOS: [(bool, bool); 3] = [(false, false), (true, false), (true, true)];

fn combo(open: bool, header: bool) -> u8 {
    if header { 2 } else { u8::from(open) }
}

/// A lane of a chunk's walk: what it does from one of `COMBOS`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalkLane {
    /// It leaves the walk at `COMBOS[exit]`, its indents `sym`, having written `toks` tokens (and the dedents of its
    /// entry's levels) and `extras` extras.
    Goes { exit: u8, sym: Sym, toks: usize, extras: usize },
    /// It refuses the text, where its reads of the entry (`sym`'s) hold.
    Refused(Refusal, Sym),
    /// Not known: B's reading is wrong, or a read of the entry does not hold for every entry.
    Unknown,
}

/// A chunk's walk's summary (its pass 1 once each chunk's strata entries are known): a lane for each of `COMBOS`.
pub type WalkSummary = [WalkLane; 3];

/// Pass 1 of the walk: chunk `from..to`'s walk from `p` (and `done`) over the strata from `rb` and `depth`, from each
/// of `COMBOS` with indents it does not know.
pub fn walk_summary(src: &[u8], from: usize, to: usize, rb: RB, depth: i8, p: usize, done: bool) -> WalkSummary {
    let (c0, _) = firsts(from);
    let stop = stop_of(src, to);
    match over_window(src, from, to, s1_at(src, c0), rb, depth, |m| {
        COMBOS.map(|(open, header)| match walk(m, p, open, header, Sym::new(), done, stop) {
            (_, _, _, _, _, _, true) => WalkLane::Unknown,
            (Err(r), _, _, _, sym, _, _) => WalkLane::Refused(r, sym),
            (Ok((o, h)), toks, extras, _, sym, _, _) => WalkLane::Goes { exit: combo(o, h), toks: toks.len() - sym.phantom, extras: extras.len(), sym },
        })
    }) {
        Ok((s, ..)) => s,
        Err(_) => core::array::from_fn(|_| WalkLane::Unknown),
    }
}

/// Walk summary `a` then `b`.
pub fn compose_walk(a: &WalkSummary, b: &WalkSummary) -> WalkSummary {
    core::array::from_fn(|c| match &a[c] {
        WalkLane::Goes { exit, sym, toks, extras } => match &b[usize::from(*exit)] {
            WalkLane::Goes { exit: e2, sym: s2, toks: t2, extras: x2 } => match Sym::then(sym, s2) {
                Some(s) => WalkLane::Goes { exit: *e2, toks: toks + t2 + Sym::closed_of(sym, s2), extras: extras + x2, sym: s },
                None => WalkLane::Unknown,
            },
            WalkLane::Refused(r, s2) => Sym::then(sym, s2).map_or(WalkLane::Unknown, |s| WalkLane::Refused(*r, s)),
            WalkLane::Unknown => WalkLane::Unknown,
        },
        x => x.clone(),
    })
}

/// A chunk's walk entry by pass 3: its `open`, `header` and indents, and the tokens and extras the chunks before it
/// wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalkEntry {
    At { open: bool, header: bool, indents: Vec<u32>, toks: usize, extras: usize },
    Refused(Refusal),
    Unknown,
}

/// Pass 3 of the walk: each chunk's walk entry, by Hillis and Steele's scan of the summaries' compositions, each
/// round's chunks in `order`'s order.
pub fn walk_entries_in(sums: &[WalkSummary], order: Order<'_>) -> Vec<WalkEntry> {
    let mut acc = sums.to_vec();
    let mut d = 1;
    while d < acc.len() {
        acc = each(acc.len(), order, |c| if c < d { acc[c].clone() } else { compose_walk(&acc[c - d], &acc[c]) });
        d *= 2;
    }
    each(acc.len(), order, |c| {
        if c == 0 {
            return WalkEntry::At { open: false, header: false, indents: vec![0], toks: 0, extras: 0 };
        }
        match &acc[c - 1][0] {
            WalkLane::Goes { exit, sym, toks, extras } => match sym.apply(&[0]) {
                Some((indents, dedents)) => {
                    let (open, header) = COMBOS[usize::from(*exit)];
                    WalkEntry::At { open, header, indents, toks: toks + dedents, extras: *extras }
                }
                None => WalkEntry::Unknown,
            },
            WalkLane::Refused(r, sym) => sym.apply(&[0]).map_or(WalkEntry::Unknown, |_| WalkEntry::Refused(*r)),
            WalkLane::Unknown => WalkEntry::Unknown,
        }
    })
}

/// How a text lexed from the middle went: its records, or its refusal; or the chunks it could not enter (`Unknown`)
/// and where B's reading is wrong (`NotB`).
pub enum Middle {
    Lexed(Lexed),
    Refused(Refusal),
    NotB,
    /// The first chunk an entry was not known for, and why.
    Unknown(usize, &'static str),
}

impl core::fmt::Debug for Middle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Middle::Lexed(l) => write!(f, "Lexed({} tokens, {} extras)", l.toks.len(), l.extras.len()),
            Middle::Refused(r) => write!(f, "Refused({r:?})"),
            Middle::NotB => write!(f, "NotB"),
            Middle::Unknown(c, why) => write!(f, "Unknown(chunk {c}: {why})"),
        }
    }
}

/// `src` lexed from the middle in chunks of `blocks` blocks: every pass's chunks each on their own, in `order`'s order.
/// 1. Strata 2 and 3: each chunk's summary from every lane, superposed (`summary_superposed`), and each chunk's entry by the scan
///    (`entries_in`).
/// 2. The walk's entry place in each chunk: its first loop top by masks, else the chunk after's (`first_top`).
/// 3. The walk: each chunk's summary from every `open` and `header` with indents it does not know (`walk_summary`),
///    and each chunk's entry by the scan (`walk_entries_in`), with the records the chunks before it wrote.
/// 4. Each chunk's records from its entry (`lex_chunk`), placed at the counts before it.
///
/// Where the chunks lexed from their entries leave an entry that is not the next chunk's, that is an error (`Err`).
pub fn lex_middle(src: &[u8], blocks: usize, order: Order<'_>) -> Result<Middle, String> {
    let n = src.len().div_ceil(64);
    let blocks = blocks.max(1);
    let k = n.div_ceil(blocks).max(1);
    let range = |c: usize| (c * blocks, (c + 1) * blocks);
    let sums = each(k, order, |c| summary_superposed(src, range(c).0, range(c).1));
    let strata = entries_in(&sums, order);
    let Some(strata) = strata.into_iter().collect::<Option<Vec<_>>>() else { return Ok(Middle::NotB) };
    // A chunk's place: its first loop top, else the chunk after's; past the last, the text's end read.
    let own = each(k, order, |c| first_top(src, range(c).0, range(c).1, strata[c].0, strata[c].1));
    if let Some(c) = own.iter().position(Option::is_none) {
        return Ok(Middle::Unknown(c, "a chunk's strata"));
    }
    let mut places = vec![Place(src.len(), true); k];
    for c in (0..k).rev() {
        places[c] = match own[c] {
            _ if c == 0 => Place(0, false),
            Some(Some(p)) => Place(p, false),
            _ => places.get(c + 1).copied().unwrap_or(Place(src.len(), true)),
        };
    }
    let wsums = each(k, order, |c| {
        let Place(p, done) = places[c];
        walk_summary(src, range(c).0, range(c).1, strata[c].0, strata[c].1, p, done)
    });
    let walks = walk_entries_in(&wsums, order);
    for (c, w) in walks.iter().enumerate() {
        match w {
            WalkEntry::Unknown => return Ok(Middle::Unknown(c, "a chunk's walk")),
            WalkEntry::Refused(r) => return Ok(Middle::Refused(*r)),
            WalkEntry::At { .. } => {}
        }
    }
    let entry = |c: usize| {
        let WalkEntry::At { open, header, indents, .. } = &walks[c] else { unreachable!() };
        let Place(p, done) = places[c];
        Entry { s1: s1_at(src, firsts(range(c).0).0), rb: strata[c].0, depth: strata[c].1, p, open: *open, header: *header, indents: indents.clone(), done }
    };
    let lexed = each(k, order, |c| {
        let (mut toks, mut extras) = (Vec::new(), Vec::new());
        (lex_chunk(src, range(c).0, range(c).1, &entry(c), &mut toks, &mut extras), toks, extras)
    });
    let (mut toks, mut extras) = (Vec::new(), Vec::new());
    for (c, (r, t, x)) in lexed.into_iter().enumerate() {
        match r {
            Chunked::Lexed(next) => {
                if c + 1 < k && next != entry(c + 1) {
                    return Err(format!("chunk {c}'s exit {next:?}, chunk {}'s entry {:?}", c + 1, entry(c + 1)));
                }
            }
            Chunked::NotB => return Ok(Middle::NotB),
            Chunked::Refused(r) => return Ok(Middle::Refused(r)),
            Chunked::Past => return Err(format!("chunk {c} read past the text's end from its entry")),
        }
        let WalkEntry::At { toks: tc, extras: xc, .. } = &walks[c] else { unreachable!() };
        if (toks.len(), extras.len()) != (*tc, *xc) {
            return Err(format!("chunk {c}'s records at {:?}, the scan's counts {:?}", (toks.len(), extras.len()), (tc, xc)));
        }
        toks.extend(t);
        extras.extend(x);
    }
    Ok(Middle::Lexed(Lexed { toks, extras }))
}

/// `lex_middle` against the serial lexer (variant B's): whether its records are the serial ones (none where B's reading
/// is wrong), the first chunk not entered and why (`Unknown`), or what differs.
pub fn middle_is_serial(src: &[u8], blocks: usize, order: Order<'_>) -> Result<Option<Middle>, String> {
    let got = lex_middle(src, blocks, order)?;
    let want = lex_with(src, Variant::B);
    let tok = |t: &Tok| (t.kind, t.soft, t.start, t.end);
    let ex = |e: &Extra| (e.start, e.end, e.comment);
    match (&got, &want) {
        (Middle::Lexed(a), Ok(b)) if a.toks.iter().map(tok).eq(b.toks.iter().map(tok)) && a.extras.iter().map(ex).eq(b.extras.iter().map(ex)) => Ok(None),
        (Middle::Refused(_), Err(_)) => Ok(None),
        (Middle::NotB, _) if !b_reads(src) || lex_chunked(src, blocks).is_none() => Ok(None),
        (Middle::Unknown(..), _) => Ok(Some(got)),
        (Middle::Lexed(a), Ok(b)) => {
            let (x, y) = (a.text(src), b.text(src));
            let k = x.lines().zip(y.lines()).position(|(p, q)| p != q).unwrap_or(0);
            let around = |t: &str| t.lines().skip(k.saturating_sub(2)).take(5).collect::<Vec<_>>().join("\n");
            Err(format!("{blocks} blocks a chunk:\n  from the middle:\n{}\n  serial:\n{}", around(&x), around(&y)))
        }
        (g, w) => Err(format!("{blocks} blocks a chunk: from the middle {g:?}, serial {:?}", w.as_ref().map(|l| l.toks.len()))),
    }
}

/// Passes 1 and 3 against the chunks lexed one after another: whether every chunk's entry by the scan of the
/// summaries (each pass in `order`) is the one the chunk before left, and stratum 1's carries read from the bytes the
/// ones it carried. None where B's reading is wrong (the text is read by A) or the text is refused.
pub fn entries_are_the_folds(src: &[u8], blocks: usize, order: Order<'_>) -> Option<Result<(), String>> {
    let (got, fold) = lex_chunked(src, blocks)?;
    got.ok()?;
    let n = src.len().div_ceil(64);
    let k = n.div_ceil(blocks.max(1)).max(1);
    let sums = each(k, order, |c| BlockMap::of_lanes(&summary(src, c * blocks.max(1), (c + 1) * blocks.max(1))));
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

/// The oracle of `starts::tops`: for each block, the walk's first loop top at or after its start (its chunk's
/// entry's `p`) against the first by masks.
pub fn tops_are_the_walks(src: &[u8]) -> Option<(usize, Option<String>)> {
    let (Ok(_), entries) = lex_chunked(src, 1)? else { return None };
    let mut t = super::Text { src, b: super::classes(src), first: 0 };
    if !t.regions_b() || t.depth().is_err() {
        return None;
    }
    let short = core::cell::Cell::new(false);
    let m = t.m(&short);
    let n = src.len().div_ceil(64);
    let tops: Vec<(u64, bool)> = (0..n).map(|bi| super::starts::tops(&m, bi)).collect();
    let mut same = 0;
    for (bi, e) in entries.iter().enumerate().skip(1) {
        let got = (bi..n).find(|&k| tops[k] != (0, false)).map_or((src.len(), true), |k| (if tops[k].0 != 0 { 64 * k + tops[k].0.trailing_zeros() as usize } else { src.len() }, false));
        if got == (e.p, e.done) {
            same += 1;
        } else {
            let at = 64 * bi;
            let text = String::from_utf8_lossy(&src[at.saturating_sub(16)..(at + 80).min(src.len())]).into_owned();
            return Some((same, Some(format!("block {bi}: masks {got:?}, walk {:?} (the text from 16 before the block: {text:?})", (e.p, e.done)))));
        }
    }
    Some((same, None))
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

    #[test]
    fn texts_lexed_from_the_middle_in_any_order_are_the_serial_lexer() {
        let mut lexed = 0;
        for (i, t) in texts().iter().enumerate() {
            for blocks in [1, 2, 3, 16] {
                match middle_is_serial(t.as_bytes(), blocks, &shuffled(i as u64 * 7 + blocks as u64)) {
                    Ok(None) => lexed += 1,
                    Ok(Some(m)) => panic!("{blocks} blocks, text {i}: {m:?}"),
                    Err(e) => panic!("text {i}: {e}"),
                }
            }
        }
        assert_eq!(lexed, 4 * texts().len());
        // Random texts: quotes, f-strings, comments, escapes, brackets and indents across blocks' edges. A chunk not
        // entered is lexed after the one before it; a wrong entry or record is never given.
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let (mut same, mut unknown) = (0, 0);
        for len in [65usize, 130, 400, 1000] {
            for k in 0..200u64 {
                let src: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        b"''\\\"\"#\n\n    ab(f{}x)\n\r[]:if "[(x >> 8) as usize % 27]
                    })
                    .collect();
                for blocks in [1, 2, 3] {
                    match middle_is_serial(&src, blocks, &shuffled(k * 3 + len as u64 + blocks as u64)) {
                        Ok(None) => same += 1,
                        Ok(Some(_)) => unknown += 1,
                        Err(e) => panic!("{e}: {:?}", String::from_utf8_lossy(&src)),
                    }
                }
            }
        }
        assert!(same > unknown, "{same} the serial lexer's, {unknown} not entered");
    }

    fn superposed_is_per_lane(src: &[u8], blocks: usize) -> Result<(), String> {
        super::superposed_is_per_lane(src, blocks)
    }

    #[test]
    fn pass_1_superposed_is_b_from_every_lane() {
        for (i, t) in texts().iter().enumerate() {
            for blocks in [1, 2, 3, 16] {
                if let Err(e) = superposed_is_per_lane(t.as_bytes(), blocks) {
                    panic!("text {i}: {e}");
                }
            }
        }
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for len in [65usize, 130, 400] {
            for _ in 0..100 {
                let src: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        b"''\\\"\"#\n\n  ab(rf{}x)\n\r[]`"[(x >> 8) as usize % 24]
                    })
                    .collect();
                for blocks in [1, 2] {
                    if let Err(e) = superposed_is_per_lane(&src, blocks) {
                        panic!("{e}: {:?}", String::from_utf8_lossy(&src));
                    }
                }
            }
        }
    }

    #[test]
    fn token_starts_by_masks_are_the_walks() {
        let mut blocks = 0;
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut random: Vec<Vec<u8>> = Vec::new();
        for len in [65usize, 130, 400, 1000] {
            for _ in 0..1000 {
                random.push(
                    (0..len)
                        .map(|_| {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                            b"ab1_0.5e-+*=/<>.:x \n  (f)[]0x"[(x >> 8) as usize % 29]
                        })
                        .collect(),
                );
            }
        }
        for t in texts().iter().map(|t| t.as_bytes()).chain(random.iter().map(Vec::as_slice)) {
            if let Some((same, _, first)) = super::super::starts::starts_are_the_walks(t) {
                blocks += same;
                if let Some(e) = first {
                    panic!("{e}");
                }
            }
        }
        assert!(blocks > 100, "{blocks}");
    }

    #[test]
    fn string_starts_by_masks_are_the_walks() {
        let mut blocks = 0;
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut random: Vec<Vec<u8>> = Vec::new();
        for len in [65usize, 130, 400, 1000] {
            for _ in 0..1000 {
                random.push(
                    (0..len)
                        .map(|_| {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                            b"ab'\"\\\\nx41{}{}rbf u N \n#=(1)U\r0'\\!:[]{f'"[(x >> 8) as usize % 40]
                        })
                        .collect(),
                );
            }
        }
        for t in texts().iter().map(|t| t.as_bytes()).chain(random.iter().map(Vec::as_slice)) {
            if let Some((same, _, first)) = super::super::starts::string_starts_are_the_walks(t) {
                blocks += same;
                if let Some(e) = first {
                    panic!("{e}: {:?}", String::from_utf8_lossy(t));
                }
            }
        }
        assert!(blocks > 100, "{blocks}");
    }

    #[test]
    fn extra_starts_by_masks_are_the_walks() {
        let mut blocks = 0;
        let mut x = 0x6a09_e667_f3bc_c908u64;
        let mut random: Vec<Vec<u8>> = Vec::new();
        for len in [65usize, 130, 400, 1000] {
            for _ in 0..1000 {
                random.push(
                    (0..len)
                        .map(|_| {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                            b"ab#\\\n\n\n  rf'\"(x)=1\r\x0cbu"[(x >> 8) as usize % 22]
                        })
                        .collect(),
                );
            }
        }
        for t in texts().iter().map(|t| t.as_bytes()).chain(random.iter().map(Vec::as_slice)) {
            if let Some((same, _, first)) = super::super::starts::extras_are_the_walks(t) {
                blocks += same;
                if let Some(e) = first {
                    panic!("{e}: {:?}", String::from_utf8_lossy(t));
                }
            }
        }
        assert!(blocks > 100, "{blocks}");
    }

    #[test]
    fn loop_tops_by_masks_are_the_walks() {
        let mut blocks = 0;
        let mut x = 0xbb67_ae85_84ca_a73bu64;
        let mut random: Vec<Vec<u8>> = Vec::new();
        for len in [65usize, 130, 400, 1000] {
            for _ in 0..1000 {
                random.push(
                    (0..len)
                        .map(|_| {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                            b"ab #\\\n\n\n  rf'\"(x)=1\x0c:  if\t"[(x >> 8) as usize % 26]
                        })
                        .collect(),
                );
            }
        }
        for t in texts().iter().map(|t| t.as_bytes()).chain(random.iter().map(Vec::as_slice)) {
            if let Some((same, first)) = super::tops_are_the_walks(t) {
                blocks += same;
                if let Some(e) = first {
                    panic!("{e}: {:?}", String::from_utf8_lossy(t));
                }
            }
        }
        assert!(blocks > 100, "{blocks}");
    }
}
