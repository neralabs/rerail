//! The reads of a 16z tape where it lies (decision 0353 step 4b), written directly: format 16 cut into blocks of
//! `F16Z_BLOCK` nodes, each stored compressed; a read finds its node's block decoded in the rolling cache its view
//! names (`crate::view::cache`), decoding it into the cache on a miss, and reads it there with format 16's reads
//! (`super::f16`) over the entry. The cache is the one the kernels and the JIT read and write: its lookup is the
//! host's (`cache::lookup`, held to theirs by tests), the decode the kernels' (`clif_tz_decode`), so every reader of a
//! batch shares its entries.
//!
//! A read is made again until it stands (`read`, the kernels' `CLIF_Z_READ`): where threads share the cache, an entry
//! read may be taken for another block meanwhile, which the cache's epoch tells. A read given up (its tries spent) is
//! 0, and a failure is counted, so the run that shares the cache is refused (decision 0463).

use super::{InPlace, NONE, Src, f16, lz4};
use crate::view::cache as c;
use crate::view::{F16_K as K, F16Z_BLOCK as BLOCK, F16Z_PIECES as PIECES, TapeView, z};
use std::sync::atomic::{AtomicU32, Ordering::SeqCst, fence};

/// The bytes at `p`, any address: the tape's, the cache's.
#[inline(always)]
unsafe fn at<T>(p: u64) -> *mut T {
    p as usize as *mut T
}

#[inline(always)]
unsafe fn word(p: u64) -> &'static AtomicU32 {
    // SAFETY: the caller's: `p` is a u32 of the cache, aligned, which every reader reads and writes atomically.
    unsafe { &*at::<AtomicU32>(p) }
}

#[inline(always)]
unsafe fn load(p: u64) -> u32 {
    unsafe { word(p).load(SeqCst) }
}

#[inline(always)]
unsafe fn add(p: u64, x: u32) -> u32 {
    unsafe { word(p).fetch_add(x, SeqCst) }
}

/// The tape's base's address, and the cache's (its offset from the base, `side[0..2]`), and a block's key (the
/// tape's id, `side[2..4]`, above the block).
#[inline(always)]
fn cache_of(t: InPlace, v: &TapeView) -> u64 {
    (t.base() as u64).wrapping_add(u64::from(v.side[0]) | u64::from(v.side[1]) << 32)
}

#[inline(always)]
fn key_of(v: &TapeView, block: u32) -> u64 {
    (u64::from(v.side[2]) | u64::from(v.side[3]) << 32) << 22 | u64::from(block)
}

/// `n` bytes from `s` to `d`.
unsafe fn copy(s: u64, d: u64, n: u64) {
    unsafe { std::ptr::copy_nonoverlapping(at::<u8>(s), at::<u8>(d), n as usize) }
}

/// Block `block` of the tape decoded into the cache entry `e` (`size` bytes) as a format 16 tape of one block
/// (decision 0434 §3's stored form): each piece decoded where its view places it, its checkpoints copied, each side
/// list's one part's starts after them, and its view. Whether it fits the entry, else nothing written.
unsafe fn decode(t: InPlace, v: &TapeView, block: u32, e: u64, size: u64) -> bool {
    let tb = t.base() as u64;
    let ld = |base: u32, off: u32, w: usize| -> u32 {
        let p = tb + u64::from(base) + u64::from(off);
        // SAFETY: the tape's piece index, side parts and checkpoints, which its view places.
        unsafe {
            match w {
                1 => u32::from(*at::<u8>(p)),
                _ => at::<u32>(p).read_unaligned(),
            }
        }
    };
    let nodes = v.nodes;
    let nb = nodes.wrapping_sub(block << 10).min(BLOCK);
    let blocks = (nodes + (BLOCK - 1)) >> 10;
    // Each side list's part of the block: from its start to the next's, never past it.
    let parts = v.columns[1];
    let mut side = [0u32; 4];
    for (q, s) in side.iter_mut().enumerate() {
        let a = ((blocks + 1) * q as u32 + block) * 4;
        let from = ld(parts, a, 4);
        *s = ld(parts, a + 4, 4).max(from) - from;
    }
    // The columns' places: column `q` holds `nb * w[q]` bytes, its side part after it, then 16 bytes of room.
    let widths = v.widths;
    let cw = (widths >> 12) & 15;
    let (mut col, mut ends) = ([0u32; 5], [0u32; 4]);
    for q in 0..4 {
        let w = if q == 0 { 1 } else { (widths >> (4 * (q as u32 - 1))) & 15 };
        ends[q] = nb * col[q] + nb * w;
        col[q + 1] = (ends[q] + side[q] + 16).div_ceil(nb);
    }
    let ckpt_at = nb * col[4];
    let ckpt_len = ((nb + K - 1) >> 3) * cw;
    let parts_at = (ckpt_at + ckpt_len + 16 + 7) & !7;
    if u64::from(parts_at + 40) + z::SLOT_BYTES as u64 + z::SLACK as u64 > size {
        return false;
    }
    let dst = e + z::SLOT_BYTES as u64;
    // The block's view: its nodes, format 16's, its columns' places, its side parts, the tape's widths and choices.
    let view = e + z::SLOT_VIEW as u64;
    let put = |field: usize, x: u32| {
        // SAFETY: the entry's view, within its payload.
        unsafe { at::<u32>(view + field as u64).write(x) }
    };
    use std::mem::offset_of;
    put(offset_of!(TapeView, nodes), nb);
    put(offset_of!(TapeView, format), 16);
    put(offset_of!(TapeView, widths), widths);
    put(offset_of!(TapeView, cut), v.cut);
    put(offset_of!(TapeView, per), v.per);
    put(offset_of!(TapeView, source_len), v.source_len);
    put(offset_of!(TapeView, stride), 0);
    for (k, &x) in col.iter().enumerate() {
        put(offset_of!(TapeView, columns) + 4 * k, x);
    }
    // Each side list's one part: its start (0) and its end.
    let parts_end = dst + u64::from(parts_at);
    for q in 0..4 {
        put(offset_of!(TapeView, side) + 4 * q, ends[q]);
        put(offset_of!(TapeView, parts) + 4 * q, parts_at + 8 * q as u32);
        // SAFETY: the parts' starts, within the entry (`parts_at + 40` fits it).
        unsafe {
            at::<u32>(parts_end + 8 * q as u64).write(0);
            at::<u32>(parts_end + 8 * q as u64 + 4).write(side[q]);
        }
    }
    // Its checkpoints, raw where the tape stores them (`BLOCK / K` a block, `cw` bytes each).
    let checkpoints = u64::from(v.columns[2]) + u64::from(block * (BLOCK / K) * cw);
    unsafe { copy(tb + checkpoints, dst + u64::from(ckpt_at), u64::from(ckpt_len)) };
    // Its pieces, one a turn: the piece index's start, length and kind (a repeat's, the piece it names).
    let pieces = blocks * PIECES;
    let starts = v.columns[0];
    let lens = starts + pieces * 4;
    let kinds = lens + pieces * 4;
    let payload = v.stride;
    for q in 0..PIECES as usize {
        let j = block * PIECES + q as u32;
        let named = ld(payload, ld(starts, j * 4, 4), 4);
        let k = if ld(kinds, j, 1) == u32::from(z::REPEAT) && named < pieces { named } else { j };
        let kind = ld(kinds, k, 1);
        let start = ld(starts, k * 4, 4);
        let len = u64::from(ld(lens, k * 4, 4));
        let src = tb + u64::from(payload) + u64::from(start);
        // Where it decodes: its column to its side part's end.
        let to = dst + u64::from(nb * col[q]);
        let to_end = dst + u64::from(ends[q] + side[q]);
        let room = (to_end - to) as usize;
        if kind == u32::from(z::LZ4) {
            // SAFETY: the piece's bytes in the payload, and its place in the entry.
            let (s, d) = unsafe { (std::slice::from_raw_parts(at::<u8>(src), len as usize), std::slice::from_raw_parts_mut(at::<u8>(to), room)) };
            lz4(s, 0, s.len(), d, room);
        } else {
            unsafe { copy(src, to, len.min(room as u64)) };
        }
    }
    true
}

/// The address of the cache entry that holds the tape's block `block` decoded; the dead entry where the cache had no
/// room, or for a block past the tape's last (a failure counted).
#[inline(never)]
fn block_entry(t: InPlace, v: &TapeView, block: u32) -> u64 {
    let cache = cache_of(t, v);
    let blocks = (v.nodes + (BLOCK - 1)) >> 10;
    let size = v.columns[3];
    // SAFETY: `InPlace::new`'s caller's: the view's cache is the tape's, laid out as `cache::Spec` made it; a fill
    // writes its entry's `size` bytes (`decode` checks the block fits them).
    unsafe {
        if block >= blocks {
            add(cache + c::FAILED as u64, 1);
            return cache + u64::from(load(cache + c::DEAD as u64));
        }
        let fill = |e: *mut u8| decode(t, v, block, e as u64, u64::from(size));
        c::lookup(cache as usize as *mut u8, key_of(v, block), size, Some(&fill))
    }
}

/// A decoded block's bytes in a cache entry: every offset held to the entry (its size less one, `m`), so a read of an
/// entry another thread writes again reads nothing outside the cache (the kernels' `clif_f16b_ld`).
#[derive(Clone, Copy)]
struct Entry {
    bytes: u64,
    m: u64,
}

impl Src for Entry {
    #[inline(always)]
    fn load<const N: usize>(self, from: usize) -> [u8; N] {
        // SAFETY: within the entry and the room past it (`cache::SLACK`).
        unsafe { at::<[u8; N]>(self.bytes + (from as u64 & self.m)).read_unaligned() }
    }
}

/// Node `i`'s block's entry: its bytes, its view, and the node's number in it.
#[inline(always)]
fn entry_of(t: InPlace, v: &TapeView, i: u32) -> (Entry, TapeView, u32) {
    let e = block_entry(t, v, i >> 10);
    // SAFETY: the entry's view, which its decode wrote (the dead entry's: its zeros).
    let view = unsafe { at::<TapeView>(e + z::SLOT_VIEW as u64).read_unaligned() };
    (Entry { bytes: e + z::SLOT_BYTES as u64, m: u64::from(v.columns[3]).wrapping_sub(1) }, view, i & (BLOCK - 1))
}

/// Whether threads share the cache (the view's `columns[4]`): else every read stands.
#[inline(always)]
fn shared(v: &TapeView) -> bool {
    v.columns[4] != 0
}

/// `x`, a read of the tape, made until it stands (the kernels' `CLIF_Z_READ`, `clif_z_kept`): a read stands when the
/// cache's epoch after it is the one before it. After `TRIES` reads that did not, a failure is counted and the read
/// given up: 0.
#[inline(always)]
fn read<T: Default>(t: InPlace, v: &TapeView, x: impl Fn() -> T) -> T {
    if !shared(v) {
        return x();
    }
    let epoch_at = cache_of(t, v) + c::EPOCH as u64;
    // SAFETY: the cache's epoch, a u32 of its header.
    let mut epoch = unsafe { load(epoch_at) };
    fence(SeqCst);
    for k in 0.. {
        let value = x();
        fence(SeqCst);
        let now = unsafe { load(epoch_at) };
        if now == epoch {
            return value;
        }
        epoch = now;
        if k + 1 >= c::TRIES {
            unsafe { add(cache_of(t, v) + c::FAILED as u64, 1) };
            return T::default();
        }
    }
    unreachable!()
}

macro_rules! block_read {
    ($(#[$doc:meta])* $name:ident -> $r:ty) => {
        $(#[$doc])*
        pub fn $name(t: InPlace, v: &TapeView, i: u32) -> $r {
            read(t, v, || {
                let (e, view, local) = entry_of(t, v, i);
                f16::$name(e, &view, local)
            })
        }
    };
}

block_read!(slot -> u64);
block_read!(size -> u32);
block_read!(number -> u32);
block_read!(start_stated -> u32);
block_read!(start -> u32);
block_read!(end -> u32);
block_read!(length -> u32);

/// The node's start, its previous node's `prev` (`f16::start_next`).
pub fn start_next(t: InPlace, v: &TapeView, i: u32, prev: u32) -> u32 {
    read(t, v, || {
        let (e, view, local) = entry_of(t, v, i);
        f16::start_next(e, &view, local, prev)
    })
}

/// The node's parent, or `NONE` at the root: format 16's walk back over the slots, each read in its block, the
/// block's entry found again only where the walk crosses into another (the kernels' `clif_f16z_parent`).
pub fn parent(t: InPlace, v: &TapeView, i: u32) -> u32 {
    read(t, v, || {
        let mut held = NONE;
        let mut e = (Entry { bytes: 0, m: 0 }, TapeView::default());
        let mut j = i;
        while j != 0 {
            let before = j - 1;
            if before >> 10 != held {
                held = before >> 10;
                let (entry, view, _) = entry_of(t, v, before);
                e = (entry, view);
            }
            let s = f16::slot(e.0, &e.1, before & (BLOCK - 1));
            if s & 1 == 0 {
                return before;
            }
            let step = (s >> 2) as u32;
            if step.wrapping_sub(1) >= j {
                return NONE;
            }
            j -= step;
            if s & 2 != 0 {
                return j;
            }
        }
        NONE
    })
}

/// The newline index: held raw, in place, as format 16 holds it.
pub fn line(t: InPlace, v: &TapeView, x: u32) -> u32 {
    f16::line(t, v, x)
}

pub fn line_start(t: InPlace, v: &TapeView, k: u32) -> u32 {
    f16::line_start(t, v, k)
}

/// The reads of a 16z tape through the rolling cache its reads share (decision 0353 step 4b): a node's block found
/// in the cache, or decoded into it as a format 16 tape of one block (`tape_z_block`, `z_decode`), then read by format
/// 16's reads in the entry (the CLIF's `in_block`). Written directly, the CLIF's steps over the same bytes, so a host
/// thread and a device share one cache; held to the JIT's by `f16::tests::the_rust_16z_reads_are_the_jits`.
///
/// The cache is named apart from the view (`Z::cache`): the view's own (`side[0..2]`, the cache less the tape) is
/// the device's where the device reads it, and the host's reads find the cache at its host address.
///
/// A slice-based reader beside the in-place one above, the host executor's (`rerail_tape_format::view_reads`); the
/// two to be one with the join program's reads (`rerail_ukl_rust::join_run`).
pub mod slice {
    use super::super::{NONE, TapeView, f16, lz4, u8_at, u32_at, width};
    use crate::view::{F16_K as K, F16Z_BLOCK as Z_BLOCK, F16Z_PIECES as PIECES, cache, z};

    /// A 16z tape as its reads take it: the bytes its view's offsets count from, its view, and the cache its blocks
    /// are decoded into (a live `cache::Spec` of at least the view's entry size, `columns[3]`).
    #[derive(Clone, Copy)]
    pub struct Z<'a> {
        pub t: &'a [u8],
        pub v: &'a TapeView,
        pub cache: *const u8,
    }

    /// The block's key in the cache: the tape's id (`side[2..4]`) above the block (`tape_z_block`'s).
    #[inline(always)]
    fn key(v: &TapeView, block: u32) -> u64 {
        (u64::from(v.side[3]) << 32 | u64::from(v.side[2])) << 22 | u64::from(block)
    }

    impl Z<'_> {
        /// Block `block`'s entry in the cache, decoded into it on a miss; the dead entry (a failure counted) for a
        /// block past the tape's last, or where the cache keeps none (`tape_z_block`).
        pub fn block(&self, block: u32) -> *const u8 {
            let v = self.v;
            let blocks = v.nodes.wrapping_add(Z_BLOCK - 1) >> Z_BLOCK.trailing_zeros();
            if block >= blocks {
                // SAFETY: the reader's cache (`Z`'s).
                return unsafe { cache::failed(self.cache) };
            }
            let size = v.columns[3];
            // SAFETY: the reader's cache; the fill writes the entry's `size` bytes alone.
            unsafe { cache::get(self.cache, key(v, block), size, &|e| decode(self.t, v, block, e, size)) }
        }

        /// `body` over node `i`'s block's format 16 tape and view in the cache, and the node's place there: where
        /// threads share the cache (`columns[4]`), the entry checked complete and the block's before the read and
        /// unchanged after it, read again a bounded number of times, else the dead entry read (a failure counted).
        #[inline]
        pub fn read<R>(&self, i: u32, body: impl Fn(&[u8], &TapeView, u32) -> R) -> R {
            let block = i >> Z_BLOCK.trailing_zeros();
            let local = i & (Z_BLOCK - 1);
            let size = self.v.columns[3] as usize;
            let shared = self.v.columns[4] != 0;
            let of = |e: *const u8| {
                // SAFETY: an entry of the cache (or its dead entry), `size` bytes and the cache's slack: its tape after
                // its header and view, its view at `SLOT_VIEW`.
                unsafe { (std::slice::from_raw_parts(e.add(z::SLOT_BYTES as usize), size - z::SLOT_BYTES as usize), &*e.add(z::SLOT_VIEW as usize).cast::<TapeView>()) }
            };
            let mut tries = 0;
            loop {
                let e = self.block(block);
                // SAFETY: an entry of the reader's cache.
                let before = unsafe { cache::ready(e) };
                let (e, recheck) = if !shared {
                    (e, false)
                } else {
                    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
                    // SAFETY: as `before`.
                    let ours = unsafe { cache::key(e) } == key(self.v, block);
                    if ours && before & cache::WRITING == 0 && before != 0 {
                        (e, true)
                    } else if tries >= cache::TRIES {
                        // SAFETY: the reader's cache.
                        (unsafe { cache::failed(self.cache) }.cast_const(), false)
                    } else {
                        tries += 1;
                        continue;
                    }
                };
                let (t, v) = of(e);
                let value = body(t, v, local);
                if !recheck {
                    return value;
                }
                std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
                // SAFETY: as `before`.
                if unsafe { cache::ready(e) } == before {
                    return value;
                }
                if tries >= cache::TRIES {
                    // SAFETY: the reader's cache.
                    let (t, v) = of(unsafe { cache::failed(self.cache) });
                    return body(t, v, local);
                }
                tries += 1;
            }
        }

        pub fn slot(&self, i: u32) -> u64 {
            self.read(i, |t, v, j| f16::slot(t, v, j))
        }
        pub fn size(&self, i: u32) -> u32 {
            self.read(i, |t, v, j| f16::size(t, v, j))
        }
        pub fn number(&self, i: u32) -> u32 {
            self.read(i, |t, v, j| f16::number(t, v, j))
        }
        pub fn start(&self, i: u32) -> u32 {
            self.read(i, |t, v, j| f16::start(t, v, j))
        }
        pub fn end(&self, i: u32) -> u32 {
            self.read(i, |t, v, j| f16::end(t, v, j))
        }
        /// As format 16's (`parent16` with `tape_slot`), each slot read in its block: the walk steps back over
        /// blocks' starts.
        pub fn parent(&self, i: u32) -> u32 {
            let mut j = i;
            while j != 0 {
                let s = self.slot(j - 1);
                if s & 1 == 0 {
                    return j - 1;
                }
                let step = (s >> 2) as u32;
                if step.wrapping_sub(1) >= j {
                    return NONE;
                }
                let back = j - step;
                if s & 2 != 0 {
                    return back;
                }
                j = back;
            }
            NONE
        }
    }

    /// Block `block` of the 16z tape `t` (its view `v`) decoded into the cache entry `e` of `size` bytes, as a format
    /// 16 tape of one block (`z_decode`): each piece decoded where the block's view places it (its column, then its
    /// side list's part), its checkpoints copied, each side list's one part's starts after them, and its view.
    /// Whether it fits the entry, else nothing written.
    fn decode(t: &[u8], v: &TapeView, block: u32, e: *mut u8, size: u32) -> bool {
        let nodes = v.nodes;
        let nb = nodes.wrapping_sub(block << Z_BLOCK.trailing_zeros()).min(Z_BLOCK);
        let blocks = nodes.wrapping_add(Z_BLOCK - 1) >> Z_BLOCK.trailing_zeros();
        // Each side list's part of the block: from its start to the next's (list `q`'s starts at `4 * q * (blocks +
        // 1)`), held to its list's order.
        let parts = v.columns[1];
        let count = blocks.wrapping_add(1);
        let side: [u32; 4] = std::array::from_fn(|q| {
            let at = (count.wrapping_mul(q as u32).wrapping_add(block)).wrapping_mul(4);
            let from = u32_at(t, parts, at);
            u32_at(t, parts, at.wrapping_add(4)).max(from).wrapping_sub(from)
        });
        // The columns' places: column `q` holds `nb * w[q]` bytes, its side part after it, then 16 bytes of room.
        let w = [1, width(v, 0), width(v, 1), width(v, 2)];
        let cw = width(v, 3);
        let mut c = [0u32; 5];
        let mut ends = [0u32; 4];
        for q in 0..4 {
            let end = nb.wrapping_mul(c[q]).wrapping_add(nb.wrapping_mul(w[q]));
            ends[q] = end;
            let end = end.wrapping_add(side[q]).wrapping_add(16);
            c[q + 1] = end.wrapping_add(nb - 1) / nb;
        }
        let ckpt_at = nb.wrapping_mul(c[4]);
        let ckpt_len = (nb.wrapping_add(K - 1) >> K.trailing_zeros()).wrapping_mul(cw);
        let parts_at = ckpt_at.wrapping_add(ckpt_len).wrapping_add(16 + 7) & !7;
        let end = parts_at.wrapping_add(32 + 8);
        if u64::from(end) + z::SLOT_BYTES as u64 + z::SLACK as u64 > u64::from(size) {
            return false;
        }
        // SAFETY: the entry taken, this fill's alone: `size` bytes.
        let entry = unsafe { std::slice::from_raw_parts_mut(e, size as usize) };
        let (head, dst) = entry.split_at_mut(z::SLOT_BYTES as usize);
        // The block's view: its nodes, format 16's, its columns' places, its side parts, the tape's widths and choices.
        let view = &mut head[z::SLOT_VIEW as usize..];
        let put = |view: &mut [u8], at: usize, value: u32| view[at..at + 4].copy_from_slice(&value.to_le_bytes());
        use std::mem::offset_of;
        for (value, at) in [
            (nb, offset_of!(TapeView, nodes)),
            (16, offset_of!(TapeView, format)),
            (v.widths, offset_of!(TapeView, widths)),
            (v.cut, offset_of!(TapeView, cut)),
            (v.per, offset_of!(TapeView, per)),
            (v.source_len, offset_of!(TapeView, source_len)),
            (0, offset_of!(TapeView, stride)),
        ] {
            put(view, at, value);
        }
        for (k, &value) in c.iter().enumerate() {
            put(view, offset_of!(TapeView, columns) + 4 * k, value);
        }
        for q in 0..4 {
            put(view, offset_of!(TapeView, side) + 4 * q, ends[q]);
            put(view, offset_of!(TapeView, parts) + 4 * q, parts_at.wrapping_add(8 * q as u32));
            let p = parts_at as usize + 8 * q;
            dst[p..p + 4].copy_from_slice(&0u32.to_le_bytes());
            dst[p + 4..p + 8].copy_from_slice(&side[q].to_le_bytes());
        }
        // Its checkpoints, raw where the tape stores them (`Z / K` a block, `cw` bytes each).
        let from = v.columns[2] as usize + (block.wrapping_mul(Z_BLOCK / K).wrapping_mul(cw)) as usize;
        let (at, len) = (ckpt_at as usize, ckpt_len as usize);
        dst[at..at + len].copy_from_slice(&t[from..from + len]);
        // Its pieces, one a turn: the piece index's start, length and kind (a repeat's, the piece it names).
        let pieces = blocks.wrapping_mul(PIECES);
        let starts = v.columns[0];
        let lens = starts.wrapping_add(pieces.wrapping_mul(4));
        let kinds = lens.wrapping_add(pieces.wrapping_mul(4));
        let payload = v.stride;
        for q in 0..PIECES as usize {
            let j = block.wrapping_mul(PIECES).wrapping_add(q as u32);
            let kind = u8_at(t, kinds, j) as u8;
            // A repeat names a piece of the tape that is none (checked where the tape is read in): else itself.
            let k = if kind == z::REPEAT {
                let named = u32_at(t, payload, u32_at(t, starts, j.wrapping_mul(4)));
                if named < pieces { named } else { j }
            } else {
                j
            };
            let kind = u8_at(t, kinds, k) as u8;
            let start = u32_at(t, starts, k.wrapping_mul(4));
            let len = u32_at(t, lens, k.wrapping_mul(4));
            let src = payload.wrapping_add(start) as usize;
            // Where it decodes: its column (`nb * C[q]`) to its side part's end.
            let (to, to_end) = (nb.wrapping_mul(c[q]) as usize, ends[q].wrapping_add(side[q]) as usize);
            let want = to_end.saturating_sub(to);
            if kind == z::LZ4 {
                lz4(t, src, src + len as usize, &mut dst[to..], want);
            } else {
                let n = (len as usize).min(want);
                dst[to..to + n].copy_from_slice(&t[src..src + n]);
            }
        }
        true
    }
}
