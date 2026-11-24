//! The relation reads at `N` lanes: each a function of vectors, `N` rows or keys an operation, no call in it and none
//! per lane. They are `relation.h`'s reads (decision 0310's, written once in CLIF) in `std::simd`'s interface
//! (`rerail_simd` until it is stable), field for field; `tests` holds each to the compiled scalar read on every row
//! of a relation of many chunks, a frozen base, a delta's own rows and a chain of continuations. A relation (its descriptor `d`) and a column are
//! the same in every lane, as a join step reads one relation; rows, keys and hashes are each lane's.
//!
//! # Safety
//!
//! Each `unsafe` read takes `d`, a relation's descriptor (a filter's, for `bloom_may`; a column's directory, for
//! `col_get`) in a block mapped for the read, and rows or places within what it holds, as its compiled read does.
#![allow(unsafe_op_in_unsafe_fn, clippy::missing_safety_doc)]

use rerail_simd::prelude::*;
use rerail_simd::{LaneCount, SupportedLaneCount};

/// `N` rows, counts or 32-bit values.
pub type R<const N: usize> = Simd<u32, N>;
/// `N` 64-bit values or hashes.
pub type V<const N: usize> = Simd<u64, N>;
/// A mask of `N` lanes.
pub type M<const N: usize> = Mask<i32, N>;
/// `N` addresses.
pub type P<const N: usize> = Simd<*const u8, N>;

#[inline(always)]
fn s32<const N: usize>(x: u32) -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Simd::splat(x)
}
#[inline(always)]
fn s64<const N: usize>(x: u64) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Simd::splat(x)
}

/// The word at each lane's address.
#[inline(always)]
unsafe fn gather32<const N: usize>(a: P<N>) -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Simd::gather_ptr(a.cast::<u32>())
}
/// The double word at each lane's address.
#[inline(always)]
unsafe fn gather64<const N: usize>(a: P<N>) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Simd::gather_ptr(a.cast::<u64>())
}
/// The double word at `base + (ix << s)` for each lane: a scalar base and each lane's 32-bit index, each lane one
/// load whose address is its index extended and shifted (`ldr x, [base, w, uxtw #s]`), no vector of addresses.
#[inline(always)]
pub unsafe fn gather64_ix<const N: usize>(base: *const u8, ix: R<N>, s: u32) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let ix = ix.to_array();
    let mut v = [0u64; N];
    for (v, &i) in v.iter_mut().zip(&ix) {
        *v = base.wrapping_add((i as usize) << s).cast::<u64>().read_unaligned();
    }
    Simd::from_array(v)
}
/// The word at `base + (ix << s)` for each lane (`gather64_ix`).
#[inline(always)]
pub unsafe fn gather32_ix<const N: usize>(base: *const u8, ix: R<N>, s: u32) -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let ix = ix.to_array();
    let mut v = [0u32; N];
    for (v, &i) in v.iter_mut().zip(&ix) {
        *v = base.wrapping_add((i as usize) << s).cast::<u32>().read_unaligned();
    }
    Simd::from_array(v)
}
/// Each lane of `v` whose bit of `on` is set written at `base + (ix << s)`, in lane order; a lane off writes a place
/// of its own (its address chosen as a value, no branch a lane).
#[inline(always)]
pub unsafe fn scatter64_ix<const N: usize>(v: V<N>, base: *mut u8, ix: R<N>, s: u32, on: u64)
where
    LaneCount<N>: SupportedLaneCount,
{
    let (v, ix) = (v.to_array(), ix.to_array());
    let mut sink = 0u64;
    for i in 0..N {
        let p = core::hint::select_unpredictable(on >> i & 1 != 0, base.wrapping_add((ix[i] as usize) << s).cast::<u64>(), &raw mut sink);
        p.write_unaligned(v[i]);
    }
}

/// `p + u * k` as each lane's address.
#[inline(always)]
fn at<const N: usize>(p: *const u8, u: R<N>, k: u32) -> P<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Simd::splat(p).wrapping_add((u.cast::<u64>() * s64(k as u64)).cast::<usize>())
}

#[inline(always)]
unsafe fn r64(p: *const u8, o: u64) -> u64 {
    p.wrapping_add(o as usize).cast::<u64>().read_unaligned()
}
#[inline(always)]
unsafe fn r32(p: *const u8, o: u64) -> u32 {
    p.wrapping_add(o as usize).cast::<u32>().read_unaligned()
}
/// The address an offset field at `p + o` names (its own place plus the offset), as the reads' `p + o + *(p + o)`.
#[inline(always)]
unsafe fn follow(p: *const u8, o: u64) -> *const u8 {
    p.wrapping_add((o as usize).wrapping_add(r64(p, o) as usize))
}

/// `x * k` in each 64-bit lane, from 32-bit products (NEON multiplies 32 bits at most; a zero-extended product is
/// one `umull`): the low halves' whole product, then the cross products a word up.
#[inline(always)]
fn mul64<const N: usize>(x: V<N>, k: u64) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let (lo, hi) = (x.cast::<u32>(), (x >> s64(32)).cast::<u32>());
    let (kl, kh) = (k as u32, (k >> 32) as u32);
    let cross = lo * s32(kh) + hi * s32(kl);
    lo.cast::<u64>() * s64(kl as u64) + (cross.cast::<u64>() << s64(32))
}

/// `clif_rel_key_mix`: a key hash `h` mixed with value `v`.
#[inline(always)]
pub fn key_mix<const N: usize>(h: V<N>, v: V<N>) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let x = mul64(h ^ v, 0x9E37_79B9_7F4A_7C15);
    x ^ (x >> s64(29))
}

/// `clif_bloom_hash` of each lane's key, as its low and high halves (the probe's bits, then its word's choice).
#[inline(always)]
pub fn bloom_hash<const N: usize>(key: V<N>) -> (R<N>, R<N>)
where
    LaneCount<N>: SupportedLaneCount,
{
    let (lo, hi) = (key.cast::<u32>(), (key >> s64(32)).cast::<u32>());
    let v6 = lo ^ (hi * s32(18446744072069020081u64 as u32));
    let v8 = v6 ^ (v6 >> s32(16));
    let v9 = v8 * s32(18446744071661406827u64 as u32);
    let v11 = v9 ^ (v9 >> s32(13));
    let v12 = v11 * s32(18446744072681074229u64 as u32);
    let v14 = v12 ^ (v12 >> s32(16));
    let v15 = v14 * s32(668265263);
    (v14, v15 ^ (v15 >> s32(15)))
}

/// The hash `(lo, hi)` as one 64-bit lane each.
#[inline(always)]
pub fn joined<const N: usize>(lo: R<N>, hi: R<N>) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    lo.cast::<u64>() | (hi.cast::<u64>() << s64(32))
}

/// `clif_bloom_may` of filter `d` for hashes `(lo, hi)` (`bloom_hash`): the lanes whose key the filter may hold.
#[inline(always)]
pub unsafe fn bloom_may<const N: usize>(d: *const u8, lo: R<N>, hi: R<N>) -> M<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let n = r64(d, 8);
    if n == 0 {
        return Mask::splat(true);
    }
    let word = hi.cast::<u64>() & s64(n.wrapping_sub(1));
    let w = gather64(Simd::splat(follow(d, 0)).wrapping_add((word << s64(3)).cast::<usize>()));
    // Five bits of the word, each by six bits of the hash's low half.
    let mut m = s64(0);
    for k in 0..5 {
        m |= s64(1) << ((lo >> s32(6 * k)) & s32(63)).cast::<u64>();
    }
    (w & m).simd_eq(m).cast::<i32>()
}

/// `clif_rel_dead` of relation `d`: the lanes whose row is dead.
#[inline(always)]
pub unsafe fn rel_dead<const N: usize>(d: *const u8, r: R<N>) -> M<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let n = r64(d, 128);
    let w = r >> s32(6);
    let inside = if n > u32::MAX as u64 { Mask::splat(true) } else { w.simd_lt(s32(n as u32)) };
    // A lane past the bits reads word 0's place (the descriptor's, when there are none), and is masked.
    let w = inside.select(w, s32(0));
    let base = if n == 0 { d } else { follow(d, 120) };
    let word = gather64(at(base, w, 8));
    let bit = (word >> (r & s32(63)).cast::<u64>()) & s64(1);
    bit.simd_ne(s64(0)).cast::<i32>() & inside
}

/// A relation's descriptor's indexes (`Relation::indexes`), an index's size, and its fields: its mask, buckets and
/// rows (`rerail_blocks::relation::Index`; `tests` holds them to it).
pub(crate) const REL_INDEXES: u64 = 136;
pub(crate) const INDEX_SIZE: usize = 168;
pub(crate) const INDEX_MASK: u64 = 24;
pub(crate) const INDEX_OFFS: u64 = 32;
pub(crate) const INDEX_ROWS: u64 = 48;

/// Index `index` of relation `d`: its descriptor.
#[inline(always)]
unsafe fn index_of(d: *const u8, index: u32) -> *const u8 {
    follow(d, REL_INDEXES).wrapping_add(index as usize * INDEX_SIZE)
}

/// `clif_rel_group` of relation `d`'s index `index` for key hashes `h`: each lane's group's first place in the index's
/// rows and its length (the read's `start << 32 | len`, as two vectors).
#[inline(always)]
pub unsafe fn rel_group<const N: usize>(d: *const u8, index: u32, h: V<N>) -> (R<N>, R<N>)
where
    LaneCount<N>: SupportedLaneCount,
{
    let ix = index_of(d, index);
    if r64(ix, INDEX_OFFS + 8) == 0 {
        return (s32(0), s32(0));
    }
    let slot = h & s64(r64(ix, INDEX_MASK));
    let a = Simd::splat(follow(ix, INDEX_OFFS)).wrapping_add((slot << s64(2)).cast::<usize>());
    let lo = gather32(a);
    let hi = gather32(a.wrapping_add(Simd::splat(4)));
    (lo, hi - lo)
}

/// `clif_rel_group_row` of relation `d`'s index `index`: the row at each lane's place `q` of its rows.
#[inline(always)]
pub unsafe fn rel_group_row<const N: usize>(d: *const u8, index: u32, q: R<N>) -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    gather32_ix(follow(index_of(d, index), INDEX_ROWS), q, 2)
}

/// `rel_group_row` of `N` consecutive places from `q` of relation `d`'s index `index` (all in its rows): one load.
#[inline(always)]
pub unsafe fn rel_group_rows<const N: usize>(d: *const u8, index: u32, q: u32) -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Simd::from_array((follow(index_of(d, index), INDEX_ROWS).wrapping_add(q as usize * 4) as *const [u32; N]).read_unaligned())
}

/// `clif_rel_len` of relation `d`.
#[inline(always)]
pub unsafe fn rel_len(d: *const u8) -> u32 {
    r32(d, 40)
}

/// Each lane's place in a column of chunks doubling from `1 << shift` rows: the chunk and the row in it, as
/// `clif_col_at` finds them.
#[inline(always)]
pub fn chunk<const N: usize>(i: R<N>, shift: R<N>) -> (R<N>, R<N>)
where
    LaneCount<N>: SupportedLaneCount,
{
    let one = s32(1);
    let s = s32(31) - ((i >> shift) + one).leading_zeros();
    let off = i - (((one << s) - one) << shift);
    // The chunk's half: `shift + s - 2` bits (none, below two: the whole offset is the row).
    let e = (shift + s).cast::<i32>() - Simd::splat(2);
    let none = e.simd_lt(Simd::splat(0));
    let e = none.select(Simd::splat(0), e).cast::<u32>();
    let seg = (s << s32(2)) + none.select(s32(0), off >> e);
    let within = none.select(off, off & ((one << e) - one));
    (seg, within)
}

/// A 4-byte column's document high words (at each lane's `tab`, `n` of them): each lane's row's, by a search of all
/// lanes in step, each lane's bounds its own.
#[inline(always)]
unsafe fn doc_high<const N: usize>(tab: P<N>, n: R<N>, i: R<N>) -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let mut lo = s32(0);
    let mut hi = n;
    loop {
        let go = lo.simd_lt(hi);
        if !go.any() {
            break;
        }
        let mid = lo + ((hi - lo) >> s32(1));
        let key = gather32(tab.wrapping_add((mid.cast::<u64>() * s64(8) + s64(8)).cast::<usize>()));
        let le = go & key.simd_le(i);
        let gt = go & !le;
        lo = le.select(mid + s32(1), lo);
        hi = gt.select(mid, hi);
    }
    gather32(tab.wrapping_add((lo.cast::<u64>() * s64(8) + s64(4)).cast::<usize>()))
}

/// The compiled `col_sealed` read (`rerail_clif::reads::READS`' "col_sealed", decision 0622): value `i` of the frozen
/// column of the sealed chunk at `ic`. The one reader of a sealed chunk: a lane whose chunk is sealed reads through
/// it, as `clif_col_get` does.
pub type Sealed = unsafe extern "C" fn(u64, u32) -> u64;

/// A chunk's directory word's low bit: the chunk sealed (`rerail_blocks::rows::SEALED`), the rest its frozen
/// column's offset from the directory.
const SEALED: u64 = 1;

/// `clif_col_get` of column `c` (the same in every lane) of chunks from `1 << shift` rows: row `i`'s value.
#[inline(always)]
pub unsafe fn col_get<const N: usize>(c: *const u8, shift: u32, i: R<N>, sealed: Sealed) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let (seg, within) = chunk(i, s32(shift));
    col_get_in(c, seg, within, i, seg.simd_eq(s32(seg[0])).all(), sealed)
}

/// Each lane's value where its chunk (`chunks`, its directory word) is sealed, through the compiled read (`ic` its
/// directory's address), the rest `raw`'s: `raw` is given the lanes not sealed, a sealed lane taking the first
/// unsealed one's chunk and place (none: no raw read).
#[inline(always)]
unsafe fn with_sealed<const N: usize>(dir: P<N>, chunks: V<N>, within: R<N>, sealed: Sealed, raw: impl FnOnce(P<N>, V<N>, R<N>) -> V<N>) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let tag = (chunks & s64(SEALED)).simd_ne(s64(0));
    let mut v = match (!tag).first_set() {
        Some(f) => {
            let ok = !tag;
            raw(ok.cast::<isize>().select(dir, Simd::splat(dir[f])), ok.select(chunks, s64(chunks[f])), ok.cast::<i32>().select(within, s32(within[f]))).to_array()
        }
        None => [0; N],
    };
    for (l, x) in v.iter_mut().enumerate() {
        if tag.test(l) {
            *x = sealed((dir[l] as u64).wrapping_add(chunks[l] & !SEALED), within[l]);
        }
    }
    Simd::from_array(v)
}

/// `within * width` as each lane's byte offset: a shift where the width is a power of two (NEON has no 64-bit
/// multiply).
#[inline(always)]
fn scaled<const N: usize>(within: R<N>, width: u32) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    if width.is_power_of_two() { within.cast::<u64>() << s64(u64::from(width.trailing_zeros())) } else { within.cast::<u64>() * s64(width as u64) }
}

/// `col_get` of row `i`, its chunk and its place in it found (`chunk`): one chunk for every column of a relation.
/// Where every lane's row is in one chunk (`one`), its address is one read. A sealed chunk's lanes read through the
/// compiled read (`with_sealed`).
#[inline(always)]
pub unsafe fn col_get_in<const N: usize>(c: *const u8, seg: R<N>, within: R<N>, i: R<N>, one: bool, sealed: Sealed) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    // Each lane's chunk: its word of the directory, by its index from one base (a step's rows' chunks span more than
    // 8 words of it, so one load of 8 would not hold them).
    let chunks = if one { s64(r64(c, 8 + 8 * seg[0] as u64)) } else { gather64_ix(c.wrapping_add(8), seg, 3) };
    if (chunks & s64(SEALED)).simd_eq(s64(0)).all() {
        return col_raw_in(c, chunks, within, i, one);
    }
    with_sealed(Simd::splat(c), chunks, within, sealed, |_, chunks, within| col_raw_in(c, chunks, within, i, false))
}

/// `col_get_in` of raw chunks (none sealed).
#[inline(always)]
unsafe fn col_raw_in<const N: usize>(c: *const u8, chunks: V<N>, within: R<N>, i: R<N>, one: bool) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let width = r32(c, 0);
    let low = if one && (width == 4 || width == 8) {
        // Every lane's row in one chunk: its base and each lane's place in it.
        let base = c.wrapping_add(chunks[0] as usize);
        if width == 8 {
            return gather64_ix(base, within, 3);
        }
        gather32_ix(base, within, 2).cast::<u64>()
    } else {
        let a = Simd::splat(c).wrapping_add((scaled(within, width) + chunks).cast::<usize>());
        if width != 4 {
            return gather64(a);
        }
        gather32(a).cast::<u64>()
    };
    let dh = r32(c, 4);
    if dh == 0 {
        return low;
    }
    let tab = c.wrapping_add((dh as i32 as i64 as usize).wrapping_mul(8));
    low | (doc_high(Simd::splat(tab), s32(r32(tab, 0)), i).cast::<u64>() << s64(32))
}

/// `clif_col_get` of each lane's own column `c`: row `i`, its chunk and its place in it found (`chunk`).
#[inline(always)]
unsafe fn col_get_each_in<const N: usize>(c: P<N>, seg: R<N>, within: R<N>, i: R<N>, sealed: Sealed) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let chunks = gather64(c.wrapping_add((seg.cast::<u64>() * s64(8) + s64(8)).cast::<usize>()));
    if (chunks & s64(SEALED)).simd_eq(s64(0)).all() {
        return col_raw_each_in(c, chunks, within, i);
    }
    with_sealed(c, chunks, within, sealed, |c, chunks, within| col_raw_each_in(c, chunks, within, i))
}

/// `col_get_each_in` of raw chunks (none sealed).
#[inline(always)]
unsafe fn col_raw_each_in<const N: usize>(c: P<N>, chunks: V<N>, within: R<N>, i: R<N>) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let width = gather32(c);
    // Each lane's width a power of two: its offset a shift.
    let off = if (width & (width - s32(1))).simd_eq(s32(0)).all() {
        within.cast::<u64>() << width.trailing_zeros().cast::<u64>()
    } else {
        within.cast::<u64>() * width.cast::<u64>()
    };
    let a = c.wrapping_add((off + chunks).cast::<usize>());
    let four = width.simd_eq(s32(4));
    let low = gather32(a);
    // The high words: an 8-byte lane's next word, a 4-byte lane's document's (none: zero).
    let high8 = gather32(four.cast::<isize>().select(a, a.wrapping_add(Simd::splat(4))));
    let dh = gather32(c.wrapping_add(Simd::splat(4)));
    let searched = four & dh.simd_ne(s32(0));
    let high4 = if !searched.any() {
        s32(0)
    } else {
        let off = searched.select(dh, s32(0)).cast::<i32>().cast::<isize>() * Simd::splat(8);
        let tab = c.wrapping_offset(off);
        let n = searched.select(gather32(tab), s32(0));
        searched.select(doc_high(tab, n, i), s32(0))
    };
    low.cast::<u64>() | (four.select(high4, high8).cast::<u64>() << s64(32))
}

/// A relation's rows (`rows`, at `d + 40` of its descriptor): column `c`'s directory.
#[inline(always)]
pub unsafe fn rows_col(rows: *const u8, c: u32) -> *const u8 {
    follow(rows, 24).wrapping_add(c as usize * 776)
}

/// `clif_rel_get` of relation `d`: column `c` of each lane's row `r` (a delta's, its base's under it, by the chain of
/// blocks each lane walks to its row's; a base in another block, `FAR_BASE`, in `far`, that block's start).
#[inline(always)]
pub unsafe fn rel_get<const N: usize>(d: *const u8, r: R<N>, c: u32, far: *const u8, sealed: Sealed) -> V<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    rel_at(d, r, far, sealed).get(c)
}

/// Each lane's row of a relation, found once for all its columns (`rel_at`): its block's columns and the row's place
/// in their chunks (every column of a block has the same chunks).
pub struct RowAt<const N: usize>
where
    LaneCount<N>: SupportedLaneCount,
{
    /// Every lane's row in one block: its first column's directory.
    one: *const u8,
    /// Else each lane's block's.
    each: P<N>,
    seg: R<N>,
    within: R<N>,
    /// The row in its block.
    i: R<N>,
    /// Every lane's row in one chunk of one block.
    uniform: bool,
    /// The compiled read of a sealed chunk.
    sealed: Sealed,
}

impl<const N: usize> RowAt<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    /// Column `c` of each lane's row: `clif_rel_get`'s.
    #[inline(always)]
    pub unsafe fn get(&self, c: u32) -> V<N> {
        let o = c as usize * 776;
        if !self.one.is_null() {
            return col_get_in(self.one.wrapping_add(o), self.seg, self.within, self.i, self.uniform, self.sealed);
        }
        col_get_each_in(self.each.wrapping_add(Simd::splat(o)), self.seg, self.within, self.i, self.sealed)
    }
}

/// Each lane's row `r` of relation `d` found (a delta's, its base's under it, by the chain of blocks each lane walks
/// to its row's; a base in another block, `FAR_BASE`, `base_off` bytes into `far`, as `clif_rel_get` reads it), for
/// `RowAt::get` of its columns.
#[inline(always)]
pub unsafe fn rel_at<const N: usize>(d: *const u8, r: R<N>, far: *const u8, sealed: Sealed) -> RowAt<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    if r64(d, 192) == 0 {
        let rows = d.wrapping_add(40);
        let i = r - s32(r32(d, 8));
        let (seg, within) = chunk(i, s32(r32(rows, 8)));
        let uniform = seg.simd_eq(s32(seg[0])).all();
        return RowAt { one: rows_col(rows, 0), each: Simd::splat(core::ptr::null()), seg, within, i, uniform, sealed };
    }
    // Each lane walks from the newest block to the one its row is in: down while its row is below the block's first.
    let mut b: P<N> = Simd::splat(d);
    let mut first;
    loop {
        first = gather32(b.wrapping_add(Simd::splat(8)));
        let next = gather64(b.wrapping_add(Simd::splat(192)));
        let go = r.simd_lt(first) & next.simd_ne(s64(0)).cast::<i32>();
        if !go.any() {
            break;
        }
        // A base in its own block lies `next` bytes from the descriptor; a far one (`FAR_BASE`, flags at 4) that far
        // into `far`.
        let is_far = (gather32(b.wrapping_add(Simd::splat(4))) & s32(64)).simd_ne(s32(0));
        let from = is_far.cast::<isize>().select(Simd::splat(far), b);
        b = go.cast::<isize>().select(from.wrapping_add(next.cast::<usize>()), b);
    }
    let rows = b.wrapping_add(Simd::splat(40));
    let shift = gather32(rows.wrapping_add(Simd::splat(8)));
    let cols = rows.wrapping_add((gather64(rows.wrapping_add(Simd::splat(24))) + s64(24)).cast::<usize>());
    let i = r - first;
    let (seg, within) = chunk(i, shift);
    RowAt { one: core::ptr::null(), each: cols, seg, within, i, uniform: false, sealed }
}

#[cfg(test)]
mod tests;
