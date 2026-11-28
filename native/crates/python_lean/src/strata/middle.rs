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
use crate::lex::{Extra, Lexed, Refusal, T, Tok};

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
///
/// The lanes share their depths: a lane's depth reads only the brackets in its code, and the lanes' codes hold few
/// different sets of them (in the corpus at most 6 a block, at most 3 in 98% of blocks). So the block's sets are
/// chosen once each (`REPS` of them, by selections over the codes), each set's depth made once, and each lane's
/// read from its set's. Where a block has more sets (`more`), its map is `block_map_each`'s: the loop over blocks
/// takes it there.
#[inline(never)]
pub(super) fn block_map(prev: &B, blk: &B, b1: &B, out: &mut BlockMap) -> bool {
    let (exit, codes) = block_lanes(prev, blk, b1);
    let br = blk.ob | blk.cb;
    let mut sets = Sets { keys: [0; REPS], n: 0, of: [0; CODES], more: false };
    // A code is live where one of its two lanes is not at `WRONG` (a lane there reads no depth).
    let live = |c: usize| (usize::from(exit[(c / 4) * 8 + c % 4]) != WRONG) | (usize::from(exit[(c / 4) * 8 + c % 4 + 4]) != WRONG);
    // The codes' steps written out (a loop of them may stay a loop).
    each_of!(c in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71] {
        sets.step(c, live(c), codes[c] & br);
    });
    let k = sets.keys;
    let by_set = [
        Depth::of_masks(blk.ob & k[0], blk.cb & k[0]),
        Depth::of_masks(blk.ob & k[1], blk.cb & k[1]),
        Depth::of_masks(blk.ob & k[2], blk.cb & k[2]),
        Depth::of_masks(blk.ob & k[3], blk.cb & k[3]),
        Depth::of_masks(blk.ob & k[4], blk.cb & k[4]),
        Depth::of_masks(blk.ob & k[5], blk.cb & k[5]),
    ];
    const _: () = assert!(REPS == 6);
    // Each lane's depth its set's (the set's index clamped, so no bounds check), the lanes written out.
    macro_rules! by {
        ($l:expr) => {
            by_set[usize::from(sets.of[code_of($l)]).min(REPS - 1)]
        };
    }
    // Into `out` lane by lane: a map made and returned would be copied (2304 bytes).
    out.exit = exit;
    each_of!(l in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143] {
        out.depth[l] = by!(l);
    });
    sets.more
}

/// The most different sets of brackets in code a block's lanes share their depths among.
const REPS: usize = 6;

/// The sets of brackets in code a block's codes hold: each set's brackets (`keys`, `n` of them), each code's set,
/// and whether there are more than `REPS`.
struct Sets {
    keys: [u64; REPS],
    n: usize,
    of: [u8; CODES],
    more: bool,
}

impl Sets {
    /// Code `c`'s brackets in code `key`: its set where one holds them, else a new set (where `live`). No branch.
    #[inline(always)]
    fn step(&mut self, c: usize, live: bool, key: u64) {
        let mut found = false;
        let mut at = 0u8;
        each_of!(j in [5usize, 4, 3, 2, 1, 0] {
            let eq = (j < self.n) & (self.keys[j] == key);
            found |= eq;
            at = pick(eq, j as u8, at);
        });
        let add = live & !found;
        self.more |= add & (self.n == REPS);
        let slot = self.n.min(REPS - 1);
        self.keys[slot] = pick(add, key, self.keys[slot]);
        self.of[c] = pick(found, at, slot as u8);
        self.n = pick(add, (self.n + 1).min(REPS), self.n);
    }
}

/// `block_map` with a depth made for each code: a block whose codes hold more than `REPS` sets of brackets.
#[inline(never)]
fn block_map_each(prev: &B, blk: &B, b1: &B) -> BlockMap {
    let (exit, codes) = block_lanes(prev, blk, b1);
    let by_code: [Depth; CODES] = core::array::from_fn(|c| Depth::of_masks(blk.ob & codes[c], blk.cb & codes[c]));
    BlockMap { exit, depth: core::array::from_fn(|l| by_code[code_of(l)]) }
}

/// The lanes of a block's map that read different bytes as code: a lane's code is the same for both values of
/// `cext_prev` (a comment's end carried in decides only where the next line starts).
const CODES: usize = LANES / 2;

/// Lane `l`'s index among the `CODES` (its `cext_prev` dropped).
#[inline(always)]
const fn code_of(l: usize) -> usize {
    (l / 8) * 4 + l % 4
}

/// `block_map`'s layers: each lane's exit (`WRONG` where B's reading is wrong) and each code's bytes of code.
#[inline(always)]
fn block_lanes(prev: &B, blk: &B, b1: &B) -> ([u8; LANES], [u64; CODES]) {
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
    let mut codes = [0u64; CODES];
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
                    let code = !(comment | ((sb | sa) & !comment & !tr) | tr);
                    codes[usize::from(t0) * 24 + usize::from(s0) * 8 + ccarry as usize * 4 + tr_taken as usize] = code;
                    each_of!(cext_prev in [0u64, 1] {
                        let cext = comment | (blk.eol & !blk.lf & (comment << 1 | cext_prev));
                        let lend = cext << 1 | cext_prev;
                        let wrong = (t_open & (sb | comment)) | (blk.bare_lf & sb & !tr & !lend) | (blk.lf & blk.esc & sb & lend) | (blk.bt & !blk.esc & !tr & !sb & !comment);
                        let s_open = (qs | qd) & !tr & !sb & !comment;
                        let o = super::win(0, s_open | t_open);
                        let (_, over) = x.overflowing_add((o.reverse_bits() << 1) & x);
                        let l = usize::from(t0) * 48 + usize::from(s0) * 16 + ccarry as usize * 8 + cext_prev as usize * 4 + tr_taken as usize;
                        let exit = usize::from(t1) * 48 + usize::from(s1) * 16 + cc_out * 8 + (cext >> 63) as usize * 4 + tr_out;
                        let bad = u8::from(all_wrong | (wrong != 0) | over);
                        // A select by mask: `WRONG` where bad.
                        let keep = bad.wrapping_sub(1);
                        exit_of[l] = (exit as u8 & keep) | (WRONG as u8 & !keep);
                    });
                });
            });
        });
    });
    (exit_of, codes)
}

/// How many different codes a block's lanes read (`block_lanes`), over a text's blocks: by the bytes of code, and by
/// the brackets in code alone (what a lane's depth reads); a lane at `WRONG` from both its values of `cext_prev`
/// not counted. Each a histogram of blocks by that count.
pub fn distinct_codes(src: &[u8]) -> ([usize; CODES + 1], [usize; CODES + 1]) {
    let blocks = src.len().div_ceil(64);
    let mut b: Vec<B> = vec![B::default()];
    classes_in(src, 0, blocks, &mut s1_at(src, 0), &mut b, usize::MAX, &mut S1::default());
    b.push(B::default());
    b.push(B::default());
    let (mut by_code, mut by_brackets) = ([0; CODES + 1], [0; CODES + 1]);
    for w in b.windows(3).take(blocks) {
        let (exit, codes) = block_lanes(&w[0], &w[1], &w[2]);
        let live: Vec<usize> = (0..CODES).filter(|&c| (0..LANES).any(|l| code_of(l) == c && usize::from(exit[l]) != WRONG)).collect();
        let mut a: Vec<u64> = live.iter().map(|&c| codes[c]).collect();
        let mut k: Vec<u64> = live.iter().map(|&c| codes[c] & (w[1].ob | w[1].cb)).collect();
        a.sort_unstable();
        a.dedup();
        k.sort_unstable();
        k.dedup();
        by_code[a.len()] += 1;
        by_brackets[k.len()] += 1;
    }
    (by_code, by_brackets)
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
    // The block's map, written by each block over the last's.
    let mut m = BlockMap { exit: [0; LANES], depth: [Depth::default(); LANES] };
    for w in b[r0 + lead - c0 - 1..r1 + lead - c0 + 1].windows(3) {
        // The loop over blocks: a block whose lanes hold more sets of brackets than `block_map` shares among is
        // `block_map_each`'s.
        if block_map(&w[0], &w[1], &w[2], &mut m) {
            m = block_map_each(&w[0], &w[1], &w[2]);
        }
        then_block(&mut exit, &mut dep, &m);
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
/// its blocks' tops, none where it has none (the chunk after's is then its own). And the first content its blocks'
/// windows read in their upper halves (`Far::upper`: blocks `from + 1..=to`). None where the strata do not read the
/// chunk.
fn chunk_firsts(src: &[u8], from: usize, to: usize, rb: RB, depth: i8) -> Option<(Option<usize>, Far)> {
    let (c0, _) = firsts(from);
    let n = src.len().div_ceil(64);
    over_window(src, from, to, s1_at(src, c0), rb, depth, |m| {
        (from..to.min(n)).rev().fold((None, Far::NONE), |(after, far), bi| {
            let (t, at_end) = super::starts::tops(&m, bi);
            let top = if t != 0 {
                Some(64 * bi + t.trailing_zeros() as usize)
            } else if at_end {
                Some(src.len())
            } else {
                after
            };
            // A text under 64 bytes has no block above its first.
            let up = if n < 2 { Far::NONE } else { Far::upper(&m.win::<false>(bi)) };
            (top, up.first(far))
        })
    })
    .ok()
    .map(|(x, ..)| x)
}

/// What the walk's pass 1 by masks takes from the chunks after each: its place, the next chunk's, and the first
/// content past its blocks' windows (`Far`, in the blocks above its last's window).
pub struct WalkInputs {
    pub places: Vec<Place>,
    pub afters: Vec<Far>,
}

/// The walk's inputs from each chunk's firsts (`chunk_firsts`): a chunk's place is its first loop top, else the
/// chunk after's (past the last, the text's end read); the content after it is the first of the chunks' after it.
/// Err: the first chunk the strata do not read.
pub fn walk_inputs(src: &[u8], blocks: usize, strata: &[(RB, i8)], order: Order<'_>) -> Result<WalkInputs, usize> {
    let blocks = blocks.max(1);
    let k = strata.len();
    let own = each(k, order, |c| chunk_firsts(src, c * blocks, (c + 1) * blocks, strata[c].0, strata[c].1));
    if let Some(c) = own.iter().position(Option::is_none) {
        return Err(c);
    }
    let mut places = vec![Place(src.len(), true); k];
    let mut afters = vec![Far::NONE; k];
    for c in (0..k).rev() {
        let (top, _) = own[c].unwrap();
        places[c] = match top {
            _ if c == 0 => Place(0, false),
            Some(p) => Place(p, false),
            None => places.get(c + 1).copied().unwrap_or(Place(src.len(), true)),
        };
        afters[c] = match own.get(c + 1) {
            Some(Some((_, f))) => f.first(afters[c + 1]),
            _ => Far::NONE,
        };
    }
    Ok(WalkInputs { places, afters })
}

impl WalkInputs {
    /// Chunk `c`'s walk summary by masks.
    pub fn summary(&self, src: &[u8], blocks: usize, strata: &[(RB, i8)], c: usize) -> (WalkSummary, [u32; 3]) {
        let blocks = blocks.max(1);
        walk_summary_masks(src, c * blocks, (c + 1) * blocks, strata[c].0, strata[c].1, self.places[c], self.places.get(c + 1).copied(), self.afters[c])
    }
}

/// The strata's entries of a text's chunks of `blocks` blocks, by the scan of their summaries in order. None where
/// the strata do not read it.
fn strata_of(src: &[u8], blocks: usize) -> Option<Vec<(RB, i8)>> {
    let n = src.len().div_ceil(64);
    let blocks = blocks.max(1);
    let k = n.div_ceil(blocks).max(1);
    let sums: Vec<_> = (0..k).map(|c| summary_superposed(src, c * blocks, (c + 1) * blocks)).collect();
    entries_in(&sums, &|k| (0..k).collect()).into_iter().collect()
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

/// The most levels a layout lane opens, and the most reads of its entry it keeps: more is not known.
const OWN: usize = 8;
const CONDS: usize = 8;
/// Why a lane of the walk's pass 1 is not known by masks (bits; none: known). A lane's own: more reads of its entry
/// or levels than it holds, a dedent check in brackets that would refuse. A block's: its starts rare in code, in
/// strings or in extras; a `\r` before no `\n`; a continuation before a blank line; blanks to the window's top; a
/// next line's content past the window, or starting with a backslash; more than `EVENTS` events; the window not read.
pub const WHY_CONDS: u32 = 1 << 0;
pub const WHY_OWN: u32 = 1 << 1;
pub const WHY_BRACKETS: u32 = 1 << 2;
pub const WHY_CODE: u32 = 1 << 3;
pub const WHY_STRINGS: u32 = 1 << 4;
pub const WHY_EXTRAS: u32 = 1 << 5;
pub const WHY_CR: u32 = 1 << 6;
pub const WHY_CONT: u32 = 1 << 7;
pub const WHY_TOP: u32 = 1 << 8;
pub const WHY_FAR: u32 = 1 << 9;
pub const WHY_BS: u32 = 1 << 10;
pub const WHY_MANY: u32 = 1 << 11;
pub const WHY_WINDOW: u32 = 1 << 12;
/// The reasons' names, by bit.
pub const WHY_NAMES: [&str; 13] =
    ["reads", "levels", "brackets", "code", "strings", "extras", "cr", "continuation", "top", "far", "backslash", "events", "window"];

/// `why` where `c`, else none: no branch.
#[inline(always)]
fn why(c: bool, why: u32) -> u32 {
    u32::from(c) * why
}

/// The most layout events of a block masks read (6 in all but 0.024% of the corpus's blocks): more is not known.
const EVENTS: usize = 6;

use super::starts::{Win, byte, pick};

/// The kinds `closer_may_follow` takes after its last record, and those of the record before a `[` it takes it after
/// (as bits of `T`'s codes).
const CLOSER_AFTER: u128 = {
    let mut s = 0u128;
    let ks = [T::Name, T::Int, T::Float, T::True, T::False, T::None, T::Ellipsis, T::StrEnd, T::RParen, T::RBracket, T::RBrace, T::LParen, T::LBrace, T::Comma];
    let mut i = 0;
    while i < ks.len() {
        s |= 1 << (ks[i] as u8);
        i += 1;
    }
    s
};
const SUBSCRIPT_AFTER: u128 = (1 << T::Name as u8) | (1 << T::StrEnd as u8) | (1 << T::RParen as u8) | (1 << T::RBracket as u8) | (1 << T::RBrace as u8);
/// No record: a code no kind has.
const NO_KIND: u8 = 127;

/// A lane of the walk's layout by masks: what `Sym` keeps of the indents, in a fixed size, and the layout's tokens,
/// `open` and `header`. Each step is selections: no branch.
#[derive(Clone, Copy, Debug)]
struct LayoutLane {
    own: [u32; OWN],
    len: usize,
    t: i64,
    phantom: usize,
    conds: [(i64, i64); CONDS],
    nconds: usize,
    toks: usize,
    open: bool,
    header: bool,
    /// Layout tokens written after the last token read, and between the two last (the walk's `closer_may_follow`
    /// reads the kinds of its last two records).
    lay_since: bool,
    lay_between: bool,
    /// Why masks do not know it (`WHY_*`; none: known).
    bad: u32,
}

impl LayoutLane {
    fn new(open: bool, header: bool) -> LayoutLane {
        LayoutLane { own: [0; OWN], len: 0, t: i64::MAX, phantom: 0, conds: [(0, 0); CONDS], nconds: 0, toks: 0, open, header, lay_since: false, lay_between: false, bad: 0 }
    }

    /// A read of the entry where `on`: none of its levels in `[lo, hi]` (kept once).
    #[inline(always)]
    fn cond(&mut self, on: bool, lo: i64, hi: i64) {
        let dup = (0..CONDS).fold(false, |d, i| d | ((i < self.nconds) & (self.conds[i] == (lo, hi))));
        let add = on & !dup;
        self.bad |= why(add & (self.nconds == CONDS), WHY_CONDS);
        let i = self.nconds.min(CONDS - 1);
        self.conds[i] = pick(add, (lo, hi), self.conds[i]);
        self.nconds = pick(add, (self.nconds + 1).min(CONDS), self.nconds);
    }

    /// The top level it opened (where it opened one).
    #[inline(always)]
    fn top(&self) -> i64 {
        i64::from(self.own[self.len.wrapping_sub(1) & (OWN - 1)])
    }

    /// `k` tokens read, the last a header's `:` where `colon`.
    #[inline(always)]
    fn read(&mut self, k: u32, colon: bool) {
        let some = k > 0;
        self.lay_between = pick(some, (k == 1) & self.lay_since, self.lay_between);
        self.lay_since &= !some;
        self.open |= some;
        self.header = pick(some, colon, self.header);
    }

    /// Where `on`, a line's layout out of brackets (the walk's layout loop): the next line's width `w` and `m`, the
    /// larger of it and the first comment's.
    #[inline(always)]
    fn line(&mut self, on: bool, w: i64, m: i64) {
        let has = self.len > 0;
        let below = !has | (self.top() < w);
        let push = on & self.header & below;
        let pop = on & !push;
        self.cond(push & !has & (w <= self.t), w, self.t);
        self.bad |= why(push & (self.len == OWN), WHY_OWN);
        let at = self.len.min(OWN - 1);
        self.own[at] = pick(push, w as u32, self.own[at]);
        let keep = (0..OWN).fold(0, |k, i| k + usize::from((i < self.len) & (i64::from(self.own[i]) <= m)));
        let phantom = pop & (keep == 0) & (m < self.t);
        let added = pick(push, 1, pick(pop, usize::from(self.open) + (self.len - keep) + usize::from(phantom), 0));
        self.len = pick(push, (self.len + 1).min(OWN), pick(pop, keep, self.len));
        self.t = pick(phantom, m, self.t);
        self.phantom += usize::from(phantom);
        self.toks += added;
        self.lay_since |= added > 0;
        self.open &= !on;
        self.header &= !on;
    }

    /// Where `on`, a line in brackets the walk checks for a dedent: refused where the top is above `m` (not known
    /// here).
    #[inline(always)]
    fn in_brackets(&mut self, on: bool, m: i64) {
        let has = self.len > 0;
        self.bad |= why(on & has & (self.top() > m), WHY_BRACKETS);
        self.cond(on & !has & (m < self.t), m + 1, self.t);
    }

    /// The walk's `closer_may_follow` from the codes of the kinds of the last two tokens read.
    #[inline(always)]
    fn closer_may_follow(&self, k1: u8, k2: u8) -> bool {
        let bit = |set: u128, k: u8| ((set >> (k & 127)) & 1 == 1) & (k != NO_KIND);
        let bracket = (k1 == T::LBracket as u8) & (self.lay_between | !bit(SUBSCRIPT_AFTER, k2));
        !self.lay_since & (bit(CLOSER_AFTER, k1) | bracket)
    }

    fn sym(&self) -> Sym {
        Sym { own: self.own[..self.len].to_vec(), t: self.t, phantom: self.phantom, conds: self.conds[..self.nconds].to_vec() }
    }
}

/// The first content past a block's window, for its events whose next line starts there: its byte, its line's width,
/// whether it is a quote or a backslash, and whether the width is known (its line's start in the window it is read
/// from). None (`at` past every byte): the text's end, width 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Far {
    pub at: usize,
    pub w: i64,
    pub quote: bool,
    pub bs: bool,
    pub known: bool,
}

impl Far {
    pub const NONE: Far = Far { at: usize::MAX, w: 0, quote: false, bs: false, known: true };

    /// The first of two: no branch.
    #[inline(always)]
    pub fn first(self, b: Far) -> Far {
        pick(self.at <= b.at, self, b)
    }

    /// The first content in the upper half of block `r.bi`'s window (block `r.bi + 1`), its width read in the window:
    /// a block function of pass 1 (no branch).
    fn upper(r: &Win<'_, false>) -> Far {
        far_block(r)
    }
}

/// `Far::upper`'s block function, apart for the branch check.
#[inline(never)]
fn far_block(r: &Win<'_, false>) -> Far {
    {
        let em = EventMasks::of(r);
        let x = (em.content & !(u128::from(u64::MAX))).trailing_zeros();
        let some = x < 128;
        let bit = |m: u128| (m >> (x & 127)) & 1 == 1;
        // The line's start in the window: a line end or form feed before it, or the text's start.
        let started = (em.resets & ((1u128 << (x & 127)) - 1) != 0) | (r.bi == 0);
        let f = Far { at: 64 * r.bi + x as usize, w: em.width(x), quote: bit(em.quote), bs: bit(em.bs), known: started };
        pick(some, f, Far::NONE)
    }
}

/// A block's window of masks the layout events read: the block's and the next one's.
struct EventMasks {
    lim: u32,
    valid: u128,
    sf: u128,
    lf: u128,
    cr: u128,
    bs: u128,
    cstart: u128,
    content: u128,
    resets: u128,
    space: u128,
    tab: u128,
    quote: u128,
    /// At bit `j`: whether the bracket depth after byte `j - 1` is 0.
    depth0_before: u128,
    /// The block's words (and the next block's), code and depth-0 bits, for the kinds of its tokens.
    word: u128,
    code: u64,
    depth0: u64,
    /// The first content past the window (`Far`).
    far: Far,
}

impl EventMasks {
    /// Block `r.bi`'s window. The depth below it is the block below's last bit (at the text's start, 0).
    #[inline(always)]
    fn of<const S: bool>(r: &Win<'_, S>) -> EventMasks {
        let (pb, b, b1, bi) = (&r.pb, &r.b, &r.b1, r.bi);
        let w2 = |f: fn(&B) -> u64| u128::from(f(b)) | u128::from(f(b1)) << 64;
        let lim = r.src.len().saturating_sub(64 * bi).min(128) as u32;
        let valid = ((1u128 << (lim & 127)) - 1) | 0u128.wrapping_sub(u128::from(lim >> 7));
        let (sptab, lf, eol) = (w2(|b| b.sptab), w2(|b| b.lf), w2(|b| b.eol));
        let ff = w2(|b| b.skip) & !sptab & !lf;
        let sf = sptab | ff;
        let cstart = w2(|b| b.cstart);
        // A comment from below the window: the bytes before its first comment's end where no `#` is before it.
        let ce = w2(|b| b.cend).trailing_zeros();
        let from_below = pick(ce < cstart.trailing_zeros(), (1u128 << (ce & 127)).wrapping_sub(1), 0);
        let comment = super::starts::fill(cstart, !eol) | from_below;
        let below = u128::from(pb.depth0 >> 63);
        EventMasks {
            lim,
            valid,
            sf,
            lf,
            cr: eol & !lf,
            bs: w2(|b| b.bs),
            cstart,
            content: valid & !(sf | eol) & !comment,
            resets: eol | ff,
            space: w2(|b| b.space) & sptab,
            tab: sptab & !w2(|b| b.space),
            quote: w2(|b| b.sq | b.dq | b.bt),
            depth0_before: (w2(|b| b.depth0) << 1) | below,
            word: w2(|b| b.word),
            code: b.code,
            depth0: b.depth0,
            far: Far::NONE,
        }
    }

    /// The width of the line before byte `p`: spaces and 8 for each tab since the last line end, `\r` or form feed.
    #[inline(always)]
    fn width(&self, p: u32) -> i64 {
        let under = (1u128 << (p & 127)) - 1;
        let r = self.resets & under;
        // Both arms are computed (a selection): each wraps.
        let between = under & !pick(r == 0, 0, (2u128 << (127u32.wrapping_sub(r.leading_zeros()) & 127)).wrapping_sub(1));
        i64::from((self.space & between).count_ones() + 8 * (self.tab & between).count_ones())
    }

    /// The block's layout events among its loop tops `tops`, and whether one is not known by masks (a `\r` before
    /// no `\n`, a continuation before a blank line, blanks to the window's top).
    fn events(&self, t: u128) -> (u128, u32) {
        let ends = self.lf | (self.cr & (self.lf >> 1)) | !self.valid;
        let events = super::starts::reaching(ends, self.sf) & t;
        let reach = super::starts::reaching(ends, self.sf);
        let cont_blank = (self.bs & (self.lf >> 1) & (reach >> 2)) | (self.bs & (self.cr >> 1) & (self.lf >> 2) & (reach >> 3));
        let hit = |x: u128| super::starts::reaching(x, self.sf) & t != 0;
        let bad = why(hit(self.cr & !(self.lf >> 1)), WHY_CR) | why(hit(cont_blank), WHY_CONT) | why(hit((self.sf >> 127) << 127), WHY_TOP);
        (events, bad)
    }

    /// The event at bit `e`: whether it is in brackets, the next line's width `w` and `m` (the larger of it and the
    /// first comment's), whether the next line starts with a quote, and whether masks do not know it.
    #[inline(always)]
    fn event(&self, e: u32) -> (bool, i64, i64, bool, u32) {
        let above_eq = |x: u32| pick(x >= 128, 0, !0u128 << (x & 127));
        let k = (!self.sf & above_eq(e)).trailing_zeros();
        let eof = k >= self.lim;
        let eol = k + u32::from((self.cr >> (k & 127)) & 1 == 1);
        let after = above_eq(eol + 1);
        let found = (self.content & after).trailing_zeros();
        let none = found >= 128;
        let x = pick(none, self.lim, found);
        // Below `x` (all the window at 128: no content in it).
        let under = pick(x >= 128, !0, (1u128 << (x & 127)).wrapping_sub(1));
        let c = (self.cstart & after & under).trailing_zeros();
        let has_c = !eof & (c < x);
        // The next line's content past the window: the first there (`far`). Its first comment must be in the window
        // (one past it is not read), and its width known.
        let far = !eof & none & (self.lim >= 128);
        let bs = pick(far, self.far.bs, (x < self.lim) & ((self.bs >> (x & 127)) & 1 == 1));
        let bad = why(far & (!has_c | !self.far.known), WHY_FAR) | why(!eof & bs, WHY_BS);
        let fc = pick(has_c, self.width(c), -1);
        let w = pick(far, self.far.w, pick(eof | (x >= self.lim), 0, self.width(x)));
        let quote = pick(far, self.far.quote, !eof & (x < self.lim) & ((self.quote >> (x & 127)) & 1 == 1));
        let brackets = (self.depth0_before >> (e & 127)) & 1 == 0;
        (brackets, w, w.max(fc), quote, bad)
    }
}

/// The layout by masks's carry from a block to the next: the codes of the last two tokens' kinds (`NO_KIND` where
/// none), and whether the last is a header's `:`.
#[derive(Clone, Copy)]
struct LastToks {
    k: [u8; 2],
    colon: bool,
}

/// The operators' kinds that `closer_may_follow` names, by byte (else a `+`); `.` is apart.
const OP_KIND: [u8; 256] = {
    let mut t = [T::Plus as u8; 256];
    t[b')' as usize] = T::RParen as u8;
    t[b']' as usize] = T::RBracket as u8;
    t[b'}' as usize] = T::RBrace as u8;
    t[b'(' as usize] = T::LParen as u8;
    t[b'{' as usize] = T::LBrace as u8;
    t[b'[' as usize] = T::LBracket as u8;
    t[b',' as usize] = T::Comma as u8;
    t
};

/// The 8 bytes of the text from `i`, zeros past its end, by single-byte loads: no branch.
#[inline(always)]
fn bytes8(src: &[u8], i: usize) -> u64 {
    (0..8).fold(0u64, |w, k| w | u64::from(byte(src, i + k)) << (8 * k))
}

/// The kind's code of the token starting at bit `q` of block `bi` as the walk's `closer_may_follow` reads it (a
/// string's last token is its end; an operator it does not name is a `+`), and whether it is a header's `:`: from
/// the block's masks and a few bytes at it, no branch.
#[inline(always)]
fn kind_of(src: &[u8], em: &EventMasks, bi: usize, q: u32) -> (u8, bool) {
    let s = 64 * bi + q as usize;
    let bit = |x: u64| (x >> (q & 63)) & 1 == 1;
    let (c, c1, c2) = (byte(src, s), byte(src, s + 1), byte(src, s + 2));
    let len = (!(em.word >> (q & 63))).trailing_zeros() as usize;
    let word = super::keyword_of(bytes8(src, s), len.max(1)).0 as u8;
    let dot = pick(c1.is_ascii_digit(), T::Float as u8, pick((c1 == b'.') & (c2 == b'.'), T::Ellipsis as u8, T::Plus as u8));
    let op = pick(c == b'.', dot, OP_KIND[usize::from(c)]);
    let code_kind = pick(bit(em.word as u64 & em.code), pick(c.is_ascii_digit(), T::Int as u8, word), op);
    let colon = (c == b':') & (c1 != b'=') & bit(em.depth0 & em.code);
    (pick(bit(em.code), code_kind, T::StrEnd as u8), colon)
}

/// The tokens `bits` of block `bi` read (all after those before): `last` takes the last two, and their count.
#[inline(always)]
fn read_toks(src: &[u8], em: &EventMasks, bi: usize, bits: u64, last: &mut LastToks) -> u32 {
    let n = bits.count_ones();
    let h1 = 63 - bits.leading_zeros().min(63);
    let h2 = 63 - (bits & !(1u64 << h1)).leading_zeros().min(63);
    let ((k1, colon), (k2, _)) = (kind_of(src, em, bi, h1), kind_of(src, em, bi, h2));
    let k0 = last.k;
    last.k = [pick(n > 0, k1, k0[0]), pick(n > 1, k2, pick(n == 1, k0[0], k0[1]))];
    last.colon = pick(n > 0, colon, last.colon);
    n
}

/// A step's event: whether there is one, whether it is in brackets, the next line's width `w` and `m` (the larger
/// of it and the first comment's), and whether the next line starts with a quote.
#[derive(Clone, Copy)]
struct Ev {
    on: bool,
    brackets: bool,
    w: i64,
    m: i64,
    quote: bool,
}

/// A lane's step for an event, after the tokens `n` read before it.
#[inline(always)]
fn lane_step(l: &mut LayoutLane, last: &LastToks, n: u32, e: Ev) {
    l.read(n, last.colon);
    l.line(e.on & !e.brackets, e.w, e.m);
    let check = e.on & e.brackets & !e.quote & !l.closer_may_follow(last.k[0], last.k[1]);
    l.in_brackets(check, e.m);
}

/// The block's next event (the lowest of `rest`, where there is one): the tokens before it read, and each lane's step.
#[inline(always)]
fn event_step(src: &[u8], em: &EventMasks, bi: usize, rest: &mut u128, k: &mut u64, lanes: &mut [LayoutLane; 3], last: &mut LastToks) -> u32 {
    let on = *rest != 0;
    let e = rest.trailing_zeros();
    *rest &= rest.wrapping_sub(1);
    let before = *k & pick(e >= 64, !0, (1u64 << (e & 63)).wrapping_sub(1));
    *k &= !before;
    let n = read_toks(src, em, bi, before, last);
    let (brackets, w, m, quote, ebad) = em.event(e);
    let ev = Ev { on, brackets, w, m, quote };
    let [a, b, c] = lanes;
    lane_step(a, last, n, ev);
    lane_step(b, last, n, ev);
    lane_step(c, last, n, ev);
    why(on, ebad)
}

/// One block's layout by masks, for each lane: its window `em`, its tokens `tk` (starts in the chunk's range) and
/// loop tops `tops` (in it, and at bit 64 one at the text's end just past it), its events in `EVENTS` steps of
/// selections, written out. Why masks do not know it (`WHY_*`). No branch: pass 1 runs it for every lane at once.
#[inline(always)]
fn layout_block(src: &[u8], em: &EventMasks, bi: usize, tk: u64, tops: u128, lanes: &mut [LayoutLane; 3], last: &mut LastToks) -> u32 {
    let (events, mut bad) = em.events(tops);
    let (mut rest, mut k) = (events, tk);
    // `EVENTS` steps, written out (a loop of them may stay a loop).
    const _: () = assert!(EVENTS == 6);
    bad |= event_step(src, em, bi, &mut rest, &mut k, lanes, last);
    bad |= event_step(src, em, bi, &mut rest, &mut k, lanes, last);
    bad |= event_step(src, em, bi, &mut rest, &mut k, lanes, last);
    bad |= event_step(src, em, bi, &mut rest, &mut k, lanes, last);
    bad |= event_step(src, em, bi, &mut rest, &mut k, lanes, last);
    bad |= event_step(src, em, bi, &mut rest, &mut k, lanes, last);
    let n = read_toks(src, em, bi, k, last);
    let [a, b, c] = lanes;
    a.read(n, last.colon);
    b.read(n, last.colon);
    c.read(n, last.colon);
    bad | why(rest != 0, WHY_MANY)
}

/// What the walk's pass 1 by masks carries from a block to the next in a chunk.
struct WalkAcc {
    lanes: [LayoutLane; 3],
    last: LastToks,
    carry: super::starts::StrCarry,
    toks: usize,
    extras: usize,
    /// Why masks do not know the chunk (`WHY_*`).
    bad: u32,
}

/// One block of the walk's pass 1 by masks, from its reads `r`, in the chunk's range `lo..hi` (and its eof's `n`):
/// its tokens' and extras' starts, its loop tops and its layout, for every lane at once. No branch.
#[inline(never)]
fn walk_block<const S: bool>(r: &Win<'_, S>, lo: usize, hi: usize, far: Far, acc: &mut WalkAcc) {
    let (src, bi) = (r.src, r.bi);
    let n = src.len();
    let mk = |x: usize| pick(x >= 64, !0u64, (1u64 << (x & 63)).wrapping_sub(1));
    let range = mk(hi.saturating_sub(64 * bi)) & !mk(lo.saturating_sub(64 * bi));
    let (cs, r1) = super::starts::code_starts_w(r);
    let (ss, r2) = super::starts::string_starts_w(r, &mut acc.carry);
    let (xs, r3) = super::starts::extra_starts_w(r);
    let some = range != 0;
    acc.bad |= why((r1 != 0) & some, WHY_CODE) | why((r2 != 0) & some, WHY_STRINGS) | why(r3 & some, WHY_EXTRAS);
    let tk = (cs | ss) & range;
    acc.toks += tk.count_ones() as usize;
    acc.extras += (xs & range).count_ones() as usize;
    let (tp, at_end) = super::starts::tops_w(r);
    let own = u128::from((lo / 64 == bi) & (lo < hi)) << (lo % 64);
    // A loop top at the text's end just past the block (its last), in the range: bit 64.
    let end = u128::from(at_end & (n >= lo) & (n < hi)) << 64;
    let tops = u128::from(tp & range) | own | end;
    let em = EventMasks { far, ..EventMasks::of(r) };
    acc.bad |= layout_block(src, &em, bi, tk, tops, &mut acc.lanes, &mut acc.last);
}

/// Pass 1 of the walk by masks: chunk `from..to`'s summary from its place `p` to the chunk after's (`next`), over
/// the strata from `rb` and `depth`, for each of `COMBOS`: its tokens and extras the popcounts of their starts in
/// its range, its layout from its events (`layout_block`). A lane masks do not know is `Unknown`.
/// With it, why masks do not know each lane (`WHY_*`; none: known). `after` is the first content past the windows
/// of the chunk's blocks (`WalkInputs`).
#[allow(clippy::too_many_arguments)]
pub fn walk_summary_masks(src: &[u8], from: usize, to: usize, rb: RB, depth: i8, p: Place, next: Option<Place>, after: Far) -> (WalkSummary, [u32; 3]) {
    let unknown = || (core::array::from_fn(|_| WalkLane::Unknown), [WHY_WINDOW; 3]);
    let n = src.len();
    let nb = n.div_ceil(64);
    if p.1 {
        return (COMBOS.map(|(o, h)| WalkLane::Goes { exit: combo(o, h), toks: 0, extras: 0, sym: Sym::new() }), [0; 3]);
    }
    let (lo, eof) = (p.0, next.is_none_or(|x| x.1));
    let hi = if eof { n + 1 } else { next.map_or(n + 1, |x| x.0) };
    let last_block = (hi.min(n).saturating_sub(1) / 64).max(lo / 64);
    let (c0, _) = firsts(from);
    let r = over_window(src, from, to.max(last_block + 2).min(nb.max(1)), s1_at(src, c0), rb, depth, |m| {
        let mut acc = WalkAcc {
            lanes: COMBOS.map(|(o, h)| LayoutLane::new(o, h)),
            last: LastToks { k: [NO_KIND; 2], colon: false },
            carry: super::starts::StrCarry::default(),
            toks: usize::from(eof),
            extras: 0,
            bad: 0,
        };
        // Each block's first content past its window, from the right: the first its windows above read in their
        // upper halves (to the chunk's last and the walk's), then the chunks' after it (`after`).
        let (b0, b1) = (lo / 64, last_block.min(nb.saturating_sub(1)));
        let top = b1.max(to.min(nb).saturating_sub(1));
        let mut fars = vec![after; b1 + 1 - b0.min(b1)];
        let mut far = after;
        for k in (b0 + 1..=top).rev() {
            far = Far::upper(&m.win::<false>(k)).first(far);
            if k - 1 <= b1 {
                fars[k - 1 - b0] = far;
            }
        }
        // The loop over blocks: each block's reads from the window, then its block function (one instance for the
        // text: under 64 bytes or not).
        for bi in b0..=b1 {
            if n < 64 {
                walk_block(&m.win::<true>(bi), lo, hi, fars[bi - b0], &mut acc);
            } else {
                walk_block(&m.win::<false>(bi), lo, hi, fars[bi - b0], &mut acc);
            }
        }
        let WalkAcc { lanes, toks, extras, bad, .. } = acc;
        let whys = lanes.map(|l| bad | l.bad);
        let sum = core::array::from_fn(|i| {
            let l = &lanes[i];
            if whys[i] != 0 { WalkLane::Unknown } else { WalkLane::Goes { exit: combo(l.open, l.header), toks: toks + l.toks - l.phantom, extras, sym: l.sym() } }
        });
        (sum, whys)
    });
    match r {
        Ok((s, ..)) => s,
        _ => unknown(),
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

/// What a walk lane is: it goes on, it refuses, or masks do not know it.
const GOES: u8 = 0;
const REFUSED: u8 = 1;
const UNKNOWN: u8 = 2;

/// A walk lane in a fixed size, for pass 3's composition (`then_walk`): `Sym`'s levels and reads in arrays, at most
/// `OWN` and `CONDS` (more is not known: the chunk then waits for its entry), and the lane's kind, exit, refusal and
/// counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WLane {
    kind: u8,
    exit: u8,
    refusal: Refusal,
    len: usize,
    own: [u32; OWN],
    t: i64,
    nconds: usize,
    conds: [(i64, i64); CONDS],
    toks: usize,
    extras: usize,
}

/// A chunk's walk summary in fixed-size lanes.
pub type WLanes = [WLane; 3];

const W_UNKNOWN: WLane =
    WLane { kind: UNKNOWN, exit: 0, refusal: Refusal::TooLarge, len: 0, own: [0; OWN], t: i64::MAX, nconds: 0, conds: [(0, 0); CONDS], toks: 0, extras: 0 };

impl WLane {
    /// A lane of pass 1 in a fixed size: not known where its levels or reads do not fit.
    pub fn of(l: &WalkLane) -> WLane {
        let (kind, exit, refusal, sym, toks, extras) = match l {
            WalkLane::Goes { exit, sym, toks, extras } => (GOES, *exit, Refusal::TooLarge, sym, *toks, *extras),
            WalkLane::Refused(r, sym) => (REFUSED, 0, *r, sym, 0, 0),
            WalkLane::Unknown => return W_UNKNOWN,
        };
        if sym.own.len() > OWN || sym.conds.len() > CONDS {
            return W_UNKNOWN;
        }
        let mut w = WLane { kind, exit, refusal, len: sym.own.len(), t: sym.t, nconds: sym.conds.len(), toks, extras, ..W_UNKNOWN };
        w.own[..sym.own.len()].copy_from_slice(&sym.own);
        w.conds[..sym.conds.len()].copy_from_slice(&sym.conds);
        w
    }

    /// The lane's entry from the text's start (indents `[0]`): its `open`, `header`, indents and counts.
    fn entry(&self) -> WalkEntry {
        let held = (0..self.nconds).all(|i| !(self.conds[i].0 <= 0 && 0 <= self.conds[i].1));
        match self.kind {
            GOES if held => {
                let (open, header) = COMBOS[usize::from(self.exit)];
                let mut indents: Vec<u32> = if self.t >= 0 { vec![0] } else { Vec::new() };
                indents.extend(&self.own[..self.len]);
                WalkEntry::At { open, header, indents, toks: self.toks + usize::from(self.t < 0), extras: self.extras }
            }
            REFUSED if held => WalkEntry::Refused(self.refusal),
            _ => WalkEntry::Unknown,
        }
    }
}

/// Lane `a` then the summary `b` (`Sym::then` in a fixed size): no branch. `a` goes on into `b`'s lane of its exit;
/// that lane's reads of its entry must hold for every entry of `a` (none of `a`'s levels in one), and are `a`'s after
/// `a`'s top; `a`'s levels `b` closes go (a dedent each), and `b`'s own follow. More than the arrays hold is not known.
#[inline(always)]
fn then_lane(a: &WLane, b: &WLanes) -> WLane {
    let bl = *pick(a.exit == 2, &b[2], pick(a.exit == 1, &b[1], &b[0]));
    let level = |i: usize| i64::from(a.own[i]);
    let (mut conds, mut n) = (a.conds, a.nconds);
    // `CONDS` steps, written out (a loop of them stays a loop).
    const _: () = assert!(CONDS == 8);
    let mut bad = false;
    bad |= then_cond(a, &bl, 0, &mut conds, &mut n);
    bad |= then_cond(a, &bl, 1, &mut conds, &mut n);
    bad |= then_cond(a, &bl, 2, &mut conds, &mut n);
    bad |= then_cond(a, &bl, 3, &mut conds, &mut n);
    bad |= then_cond(a, &bl, 4, &mut conds, &mut n);
    bad |= then_cond(a, &bl, 5, &mut conds, &mut n);
    bad |= then_cond(a, &bl, 6, &mut conds, &mut n);
    bad |= then_cond(a, &bl, 7, &mut conds, &mut n);
    // `a`'s levels are increasing: those at most `b`'s `t` are a prefix.
    let keep = (0..OWN).fold(0, |k, i| k + usize::from((i < a.len) & (level(i) <= bl.t)));
    let mut own = [0u32; OWN];
    for (k, o) in own.iter_mut().enumerate() {
        let j = k.wrapping_sub(keep);
        *o = pick(k < keep, a.own[k], pick(j < bl.len, bl.own[j & (OWN - 1)], 0));
    }
    let len = keep + bl.len;
    bad |= len > OWN;
    let composed = WLane {
        kind: pick(bad, UNKNOWN, bl.kind),
        exit: bl.exit,
        refusal: bl.refusal,
        len: len.min(OWN),
        own,
        t: a.t.min(bl.t),
        nconds: n,
        conds,
        toks: a.toks + bl.toks + (a.len - keep),
        extras: a.extras + bl.extras,
    };
    // A lane that refuses, or that masks do not know, stays as it is.
    pick(a.kind == GOES, composed, *a)
}

/// `b`'s read `j` of its entry after lane `a` (where `b` has one): whether one of `a`'s levels is in it (the
/// composition is not known), else the read after `a`'s top, kept once in `conds` (more than it holds: not known).
#[inline(always)]
fn then_cond(a: &WLane, b: &WLane, j: usize, conds: &mut [(i64, i64); CONDS], n: &mut usize) -> bool {
    let on = j < b.nconds;
    let (lo, hi) = b.conds[j];
    let level = |i: usize| i64::from(a.own[i]);
    let hit = (0..OWN).fold(false, |h, i| h | ((i < a.len) & (lo <= level(i)) & (level(i) <= hi)));
    let c = (lo, hi.min(a.t));
    let dup = (0..CONDS).fold(false, |d, i| d | ((i < *n) & (conds[i] == c)));
    let add = on & (c.0 <= c.1) & !dup;
    let full = add & (*n == CONDS);
    let at = (*n).min(CONDS - 1);
    conds[at] = pick(add, c, conds[at]);
    *n = pick(add, (*n + 1).min(CONDS), *n);
    (on & hit) | full
}

/// Summary `a` then summary `b`, each lane: pass 3's composition, no branch.
#[inline(never)]
pub fn then_walk(a: &WLanes, b: &WLanes) -> WLanes {
    [then_lane(&a[0], b), then_lane(&a[1], b), then_lane(&a[2], b)]
}

/// A chunk's walk entry by pass 3: its `open`, `header` and indents, and the tokens and extras the chunks before it
/// wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalkEntry {
    At { open: bool, header: bool, indents: Vec<u32>, toks: usize, extras: usize },
    Refused(Refusal),
    Unknown,
}

/// Pass 3 of the walk: each chunk's walk entry, by Hillis and Steele's scan of the summaries' compositions in fixed
/// size (`then_walk`, no branch), each round's chunks in `order`'s order.
pub fn walk_entries_in(sums: &[WalkSummary], order: Order<'_>) -> Vec<WalkEntry> {
    let mut acc: Vec<WLanes> = sums.iter().map(|s| s.each_ref().map(WLane::of)).collect();
    let mut d = 1;
    while d < acc.len() {
        acc = each(acc.len(), order, |c| if c < d { acc[c] } else { then_walk(&acc[c - d], &acc[c]) });
        d *= 2;
    }
    each(acc.len(), order, |c| {
        if c == 0 {
            return WalkEntry::At { open: false, header: false, indents: vec![0], toks: 0, extras: 0 };
        }
        acc[c - 1][0].entry()
    })
}

/// Pass 3 of the walk by `Sym` (the oracle of `walk_entries_in`): the same scan by `compose_walk`.
pub fn walk_entries_by_sym(sums: &[WalkSummary], order: Order<'_>) -> Vec<WalkEntry> {
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
    /// Stopped after a pass (`lex_middle_to`): a sum of its results, so they are kept.
    Stopped(usize),
}

/// Where `lex_middle_to` stops: after the strata's summaries (pass 1), their scan (pass 3), the walk's inputs (each
/// chunk's firsts and their fills), the walk's summaries (pass 1), its scan and the waits (pass 3), or not (pass 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    StrataSummaries = 1,
    StrataScan,
    WalkInputs,
    WalkSummaries,
    WalkScan,
    Lexed,
}

/// What `lex_middle_to` counts: the chunks, the lanes masks do not know, the chunks the scan gives no entry (they
/// wait), and of those the ones entered by the chunk before's lane and the ones lexed again from its entry.
#[derive(Clone, Copy, Debug, Default)]
pub struct MiddleCounts {
    pub chunks: usize,
    pub unknown_lanes: usize,
    pub waits: usize,
    pub by_lane: usize,
    pub lexed_again: usize,
}

impl core::fmt::Debug for Middle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Middle::Lexed(l) => write!(f, "Lexed({} tokens, {} extras)", l.toks.len(), l.extras.len()),
            Middle::Refused(r) => write!(f, "Refused({r:?})"),
            Middle::NotB => write!(f, "NotB"),
            Middle::Unknown(c, why) => write!(f, "Unknown(chunk {c}: {why})"),
            Middle::Stopped(x) => write!(f, "Stopped({x})"),
        }
    }
}

/// `src` lexed from the middle in chunks of `blocks` blocks: every pass's chunks each on their own, in `order`'s order.
/// 1. Strata 2 and 3: each chunk's summary from every lane, superposed (`summary_superposed`), and each chunk's entry by the scan
///    (`entries_in`).
/// 2. The walk's entry place in each chunk: its first loop top by masks, else the chunk after's (`first_top`).
/// 3. The walk: each chunk's summary from every `open` and `header` with indents it does not know, by masks
///    (`walk_summary_masks`), and each chunk's entry by the scan (`walk_entries_in`), with the records the chunks
///    before it wrote. A chunk the scan gives none waits for the one before's: by its lane, or by lexing it.
/// 4. Each chunk's records from its entry (`lex_chunk`), placed at the counts before it.
///
/// Where the chunks lexed from their entries leave an entry that is not the next chunk's, that is an error (`Err`).
pub fn lex_middle(src: &[u8], blocks: usize, order: Order<'_>) -> Result<Middle, String> {
    lex_middle_to(src, blocks, order, Stage::Lexed, &mut MiddleCounts::default())
}

/// `lex_middle` up to `stop`, with its counts.
pub fn lex_middle_to(src: &[u8], blocks: usize, order: Order<'_>, stop: Stage, counts: &mut MiddleCounts) -> Result<Middle, String> {
    let n = src.len().div_ceil(64);
    let blocks = blocks.max(1);
    let k = n.div_ceil(blocks).max(1);
    counts.chunks += k;
    let range = |c: usize| (c * blocks, (c + 1) * blocks);
    let sums = each(k, order, |c| summary_superposed(src, range(c).0, range(c).1));
    if stop == Stage::StrataSummaries {
        return Ok(Middle::Stopped(sums.len()));
    }
    let strata = entries_in(&sums, order);
    if stop == Stage::StrataScan {
        return Ok(Middle::Stopped(strata.iter().flatten().count()));
    }
    let Some(strata) = strata.into_iter().collect::<Option<Vec<_>>>() else { return Ok(Middle::NotB) };
    // Each chunk's place and the first content after it, from the chunks after it.
    let inputs = match walk_inputs(src, blocks, &strata, order) {
        Ok(x) => x,
        Err(c) => return Ok(Middle::Unknown(c, "a chunk's strata")),
    };
    let places = &inputs.places;
    if stop == Stage::WalkInputs {
        return Ok(Middle::Stopped(places.iter().map(|p| p.0).sum::<usize>() ^ inputs.afters.iter().map(|f| f.at).fold(0, usize::wrapping_add)));
    }
    let wsums = each(k, order, |c| inputs.summary(src, blocks, &strata, c));
    counts.unknown_lanes += wsums.iter().flat_map(|(_, w)| w).filter(|&&w| w != 0).count();
    let wsums: Vec<_> = wsums.into_iter().map(|(s, _)| s).collect();
    if stop == Stage::WalkSummaries {
        return Ok(Middle::Stopped(wsums.iter().flatten().filter(|l| matches!(l, WalkLane::Goes { .. })).count()));
    }
    let mut walks = walk_entries_in(&wsums, order);
    // A chunk the scan gives no entry waits for the one before's: by that chunk's lane where masks know it, else by
    // lexing that chunk from its entry.
    for c in 1..k {
        if !matches!(walks[c], WalkEntry::Unknown) {
            continue;
        }
        counts.waits += 1;
        let WalkEntry::At { open, header, indents, toks, extras } = walks[c - 1].clone() else { continue };
        let by_lane = match &wsums[c - 1][usize::from(combo(open, header))] {
            WalkLane::Goes { exit, sym, toks: t, extras: x } => sym.apply(&indents).map(|(indents, dedents)| {
                let (open, header) = COMBOS[usize::from(*exit)];
                WalkEntry::At { open, header, indents, toks: toks + t + dedents, extras: extras + x }
            }),
            WalkLane::Refused(r, sym) => sym.apply(&indents).map(|_| WalkEntry::Refused(*r)),
            WalkLane::Unknown => None,
        };
        walks[c] = match by_lane {
            Some(w) => {
                counts.by_lane += 1;
                w
            }
            None => {
                counts.lexed_again += 1;
                let Place(p, done) = places[c - 1];
                let e = Entry { s1: s1_at(src, firsts(range(c - 1).0).0), rb: strata[c - 1].0, depth: strata[c - 1].1, p, open, header, indents, done };
                let (mut t, mut x) = (Vec::new(), Vec::new());
                match lex_chunk(src, range(c - 1).0, range(c - 1).1, &e, &mut t, &mut x) {
                    Chunked::Lexed(next) => WalkEntry::At { open: next.open, header: next.header, indents: next.indents, toks: toks + t.len(), extras: extras + x.len() },
                    Chunked::Refused(r) => WalkEntry::Refused(r),
                    Chunked::NotB => return Ok(Middle::NotB),
                    Chunked::Past => return Err(format!("chunk {} read past the text's end from its entry", c - 1)),
                }
            }
        };
    }
    if stop == Stage::WalkScan {
        return Ok(Middle::Stopped(walks.len()));
    }
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

/// The oracle of `walk_summary_masks`: each chunk's summary by masks against the walk's (`walk_summary`), in chunks
/// of `blocks` blocks. The chunks whose lanes masks know, those they do not, and the first that differs.
pub fn layout_is_the_walks(src: &[u8], blocks: usize) -> Option<(usize, usize, Option<String>)> {
    let blocks = blocks.max(1);
    let range = |c: usize| (c * blocks, (c + 1) * blocks);
    let strata = strata_of(src, blocks)?;
    let k = strata.len();
    let inputs = walk_inputs(src, blocks, &strata, &in_order).ok()?;
    let places = &inputs.places;
    let norm = |l: &WalkLane| match l {
        WalkLane::Goes { exit, sym, toks, extras } => {
            let mut s = sym.clone();
            s.conds.sort_unstable();
            s.conds.dedup();
            WalkLane::Goes { exit: *exit, sym: s, toks: *toks, extras: *extras }
        }
        x => x.clone(),
    };
    let (mut same, mut unknown) = (0, 0);
    for c in 0..k {
        let Place(p, done) = places[c];
        let walk = walk_summary(src, range(c).0, range(c).1, strata[c].0, strata[c].1, p, done);
        let masks = inputs.summary(src, blocks, &strata, c).0;
        for l in 0..3 {
            match (&masks[l], &walk[l]) {
                (WalkLane::Unknown, _) => unknown += 1,
                (a, b) if norm(a) == norm(b) => same += 1,
                (a, b) => {
                    let at = 64 * range(c).0;
                    let text = String::from_utf8_lossy(&src[at.min(src.len())..(at + 64 * blocks + 32).min(src.len())]).into_owned();
                    return Some((same, unknown, Some(format!("chunk {c} lane {l} (place {:?}, next {:?}): masks {a:?}, walk {b:?} (the chunk's text: {text:?})", places[c], places.get(c + 1)))));
                }
            }
        }
    }
    Some((same, unknown, None))
}

/// The oracle of pass 3's fixed-size scan (`walk_entries_in`): for a text in chunks of `blocks` blocks, with the
/// summaries by masks (as `lex_middle` makes them), each chunk's entry against the scan by `Sym`
/// (`walk_entries_by_sym`). The entries the same, those the fixed sizes do not know where `Sym` does (the chunk waits),
/// and the first that differs. None where the strata do not read the text.
pub fn walk_scan_is_syms(src: &[u8], blocks: usize, order: Order<'_>) -> Option<(usize, usize, Option<String>)> {
    let blocks = blocks.max(1);
    let strata = strata_of(src, blocks)?;
    let k = strata.len();
    let inputs = walk_inputs(src, blocks, &strata, &in_order).ok()?;
    let wsums: Vec<_> = (0..k).map(|c| inputs.summary(src, blocks, &strata, c).0).collect();
    let (fixed, by_sym) = (walk_entries_in(&wsums, order), walk_entries_by_sym(&wsums, order));
    let (mut same, mut waits) = (0, 0);
    for (c, (f, y)) in fixed.iter().zip(&by_sym).enumerate() {
        match (f, y) {
            (WalkEntry::Unknown, WalkEntry::Unknown) => {}
            (WalkEntry::Unknown, _) => waits += 1,
            (a, b) if a == b => same += 1,
            (a, b) => return Some((same, waits, Some(format!("chunk {c} of {k}, {blocks} blocks a chunk: fixed {a:?}, by Sym {b:?}")))),
        }
    }
    Some((same, waits, None))
}

/// `walk_why_counts`: the lanes, per reason the lanes among its reasons and those it alone marks, and per reason the
/// first chunk's first block it marks.
pub type WhyCounts = (usize, [usize; 13], [usize; 13], [Option<usize>; 13]);

/// Why pass 1 by masks does not know a text's lanes, in chunks of `blocks` blocks: the lanes, and for each reason
/// (`WHY_NAMES`) the lanes it is among the reasons of, and those it is the only reason of. None where the strata do
/// not read the text.
pub fn walk_why_counts(src: &[u8], blocks: usize) -> Option<WhyCounts> {
    let blocks = blocks.max(1);
    let range = |c: usize| (c * blocks, (c + 1) * blocks);
    let strata = strata_of(src, blocks)?;
    let k = strata.len();
    let inputs = walk_inputs(src, blocks, &strata, &in_order).ok()?;
    let (mut lanes, mut any, mut alone, mut first) = (0, [0; 13], [0; 13], [None; 13]);
    for c in 0..k {
        let (_, whys) = inputs.summary(src, blocks, &strata, c);
        for w in whys {
            lanes += 1;
            for (b, (a, o)) in any.iter_mut().zip(alone.iter_mut()).enumerate() {
                if w >> b & 1 == 1 {
                    *a += 1;
                    first[b] = first[b].or(Some(range(c).0));
                    if w == 1 << b {
                        *o += 1;
                    }
                }
            }
        }
    }
    Some((lanes, any, alone, first))
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
            "if a:\n    b\n    # one\n    # ------------------------------------------------------------\n# two ----------------------------------------------------------\n  # three -------------------------------------------------------\n\n# four -----------------------------------------------------------\nc\n",
            "f'a{b!r:>{w}}c'\n",
            "x = f\"{d['k']} and {x.get('a')} {', '.join(a)} {''}\"\n",
            "y = f'{\"x\"}' + f\"{d['a:b']}\" + f\"{r'a'}\" + f\"{a['x'] + b['y']}\"\n",
            "z = f\"\"\"{a['x'] + b[\"y\"]}\"\"\"\n",
            "w = f\"{d['a long key that crosses the edge of a block, well past it, to the next one']}\"\n",
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

    #[test]
    fn walk_scan_in_fixed_size_is_the_scan_by_sym() {
        let mut same = 0;
        for (i, t) in texts().iter().enumerate() {
            for blocks in [1, 2, 3, 16] {
                if let Some((s, _, first)) = super::walk_scan_is_syms(t.as_bytes(), blocks, &super::in_order) {
                    same += s;
                    if let Some(e) = first {
                        panic!("text {i}: {e}");
                    }
                }
            }
        }
        assert!(same > 30, "{same}");
        // Random texts of lines, each scan's rounds in seeded random orders.
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = |k: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 8) % k
        };
        let pieces: [&str; 14] = ["if a:", "x = 1", "y(", "z)", "# c", "", "  ", "f(a,", "b)", "else:", "'s'", "[", "]", "def f():"];
        for k in 0..600 {
            let mut t = String::new();
            for _ in 0..(5 + rnd(80)) {
                t.push_str(["", " ", "  ", "    ", "\t", "        ", "\t "][rnd(7) as usize]);
                t.push_str(pieces[rnd(14) as usize]);
                let pad = rnd(16) as usize;
                if pad > 2 {
                    t.push_str("  #");
                    t.push_str(&"-".repeat(pad));
                }
                t.push_str(["\n", "\n", "\n", "\r\n", "\n\n"][rnd(5) as usize]);
            }
            for blocks in [1, 2, 3] {
                if let Some((s, _, first)) = super::walk_scan_is_syms(t.as_bytes(), blocks, &shuffled(k * 7 + blocks as u64)) {
                    same += s;
                    if let Some(e) = first {
                        panic!("{e}: {t:?}");
                    }
                }
            }
        }
        assert!(same > 3000, "{same}");
    }

    /// The shared depths (`block_map`) against a depth for each code (`block_map_each`): each lane's exit, and the
    /// depth of each lane not at `WRONG`, for every block of `src`. The blocks that had more sets than `REPS`.
    fn shared_is_each(src: &[u8]) -> usize {
        let blocks = src.len().div_ceil(64);
        let mut b: Vec<B> = vec![B::default()];
        classes_in(src, 0, blocks, &mut s1_at(src, 0), &mut b, usize::MAX, &mut S1::default());
        b.push(B::default());
        b.push(B::default());
        let mut more = 0;
        for (bi, w) in b.windows(3).take(blocks).enumerate() {
            let mut m = BlockMap { exit: [0; LANES], depth: [Depth::default(); LANES] };
            if block_map(&w[0], &w[1], &w[2], &mut m) {
                more += 1;
                continue;
            }
            let e = block_map_each(&w[0], &w[1], &w[2]);
            for l in 0..LANES {
                assert_eq!(m.exit[l], e.exit[l], "block {bi}, lane {l}: {src:?}");
                if usize::from(e.exit[l]) != WRONG {
                    assert_eq!(m.depth[l], e.depth[l], "block {bi}, lane {l}: {src:?}");
                }
            }
        }
        more
    }

    #[test]
    fn shared_depths_are_each_codes() {
        for t in texts() {
            shared_is_each(t.as_bytes());
        }
        // Random texts of brackets, quotes of every kind, comments and line ends: many sets of brackets a block.
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = |k: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 8) % k
        };
        let pieces: [&str; 16] = ["(", ")", "[", "]", "{", "}", "'", "\"", "'''", "\"\"\"", "#", "\n", "x", " ", "\\", "f'"];
        let mut more = 0;
        for _ in 0..3000 {
            let t: String = (0..(10 + rnd(200))).map(|_| pieces[rnd(16) as usize]).collect();
            more += shared_is_each(t.as_bytes());
        }
        assert!(more > 0, "no block had more sets than REPS");
    }

    #[test]
    fn next_lines_past_the_window_are_known() {
        // Each line end before the run of comments has its next line's content blocks away.
        let mut t = String::from("import a\n\nif a:\n    b = 1\n");
        for i in 0..8 {
            t.push_str(&format!("{}# {i} {}\n", ["", "    "][i % 2], "-".repeat(50)));
        }
        t.push_str("c = 2\n");
        for blocks in [1, 2, 3] {
            let (lanes, any, _, first) = super::walk_why_counts(t.as_bytes(), blocks).unwrap();
            assert!(any.iter().all(|&a| a == 0), "{blocks} blocks a chunk: of {lanes} lanes, not known: {any:?} (first at {first:?})");
            let (same, unknown, e) = super::layout_is_the_walks(t.as_bytes(), blocks).unwrap();
            assert!(e.is_none() && unknown == 0 && same > 0, "{same} {unknown} {e:?}");
        }
    }

    #[test]
    fn layout_by_masks_is_the_walks() {
        let mut lanes = 0;
        for (i, t) in texts().iter().enumerate() {
            for blocks in [1, 2, 3, 16] {
                if let Some((same, _, first)) = super::layout_is_the_walks(t.as_bytes(), blocks) {
                    lanes += same;
                    if let Some(e) = first {
                        panic!("text {i}, {blocks} blocks a chunk: {e}");
                    }
                }
            }
        }
        assert!(lanes > 100, "{lanes}");
        // Random texts of lines: indents of spaces and tabs, headers, brackets across lines, comments, blank lines.
        let mut x = 0x3c6e_f372_fe94_f82bu64;
        let mut rnd = |k: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 8) % k
        };
        let pieces: [&str; 14] = ["if a:", "x = 1", "y(", "z)", "# c", "", "  ", "f(a,", "b)", "else:", "'s'", "[", "]", "def f():"];
        for _ in 0..1500 {
            let mut t = String::new();
            for _ in 0..(5 + rnd(60)) {
                let indent = ["", " ", "  ", "    ", "\t", "        ", "\t "][rnd(7) as usize];
                t.push_str(indent);
                t.push_str(pieces[rnd(14) as usize]);
                // A trailing comment, so lines are near real lengths (masks read at most `EVENTS` line ends a block).
                let pad = rnd(16) as usize;
                if pad > 2 {
                    t.push_str("  #");
                    t.push_str(&"-".repeat(pad));
                }
                t.push_str(["\n", "\n", "\n", "\r\n", "\n\n"][rnd(5) as usize]);
                // Now and then a run of comment lines and blank ones past a block's window (the next line's content
                // read past it).
                if rnd(8) == 0 {
                    for _ in 0..(2 + rnd(10)) {
                        t.push_str(["", " ", "    ", "\t", "        "][rnd(5) as usize]);
                        if rnd(5) > 0 {
                            t.push('#');
                            t.push_str(&"=".repeat(10 + rnd(50) as usize));
                        }
                        t.push_str(["\n", "\n", "\r\n"][rnd(3) as usize]);
                    }
                }
            }
            for blocks in [1, 2, 3] {
                if let Some((same, _, first)) = super::layout_is_the_walks(t.as_bytes(), blocks) {
                    lanes += same;
                    if let Some(e) = first {
                        panic!("{blocks} blocks a chunk: {e}: {t:?}");
                    }
                }
            }
        }
        assert!(lanes > 10000, "{lanes}");
    }
}
