//! The reads of a 16z tape where it lies (decision 0353 step 4b), written directly: format 16 cut into blocks of
//! `F16Z_BLOCK` nodes, each stored compressed; a read finds its node's block decoded in the rolling cache its view
//! names (`rerail_clif::view::cache`), decoding it into the cache's head on a miss, and reads it there with format 16's
//! reads (`super::f16`) over the entry. The cache is the one the kernels and the JIT read and write
//! (`<ukl/reads/tape_f16z.h>`, the CLIF's `tape_z_block`): the same layout, the same protocol, the same steps, so
//! every reader of a batch shares its entries. Its words are read and written atomically, as the device's are.
//!
//! A read is made again until it stands (`read`, the kernels' `CLIF_Z_READ`): where threads share the cache, an entry
//! read may be taken for another block meanwhile, which the cache's epoch tells. A read given up (its tries spent) is
//! 0, and a failure is counted, so the run that shares the cache is refused (decision 0463).

use super::{InPlace, NONE, Src, f16, lz4};
use rerail_clif::view::cache as c;
use rerail_clif::view::{F16_K as K, F16Z_BLOCK as BLOCK, F16Z_PIECES as PIECES, TapeView, z};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::SeqCst, fence};

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
unsafe fn store(p: u64, x: u32) {
    unsafe { word(p).store(x, SeqCst) }
}

#[inline(always)]
unsafe fn add(p: u64, x: u32) -> u32 {
    unsafe { word(p).fetch_add(x, SeqCst) }
}

#[inline(always)]
unsafe fn cas(p: u64, e: u32, n: u32) -> u32 {
    unsafe { word(p).compare_exchange(e, n, SeqCst, SeqCst).unwrap_or_else(|was| was) }
}

#[inline(always)]
unsafe fn key_at(p: u64) -> u64 {
    // SAFETY: an entry's key, a u64 aligned at its first byte.
    unsafe { (*at::<AtomicU64>(p)).load(SeqCst) }
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

/// The cache's dead entry: what a read with no entry of its own reads (its zeros).
unsafe fn dead(cache: u64) -> u64 {
    unsafe { cache + u64::from(load(cache + c::DEAD as u64)) }
}

/// The statistics mode's count at `a`, where the cache keeps them.
unsafe fn stat(cache: u64, a: i32) {
    unsafe {
        if load(cache + c::STATS as u64) != 0 {
            add(cache + a as u64, 1);
        }
    }
}

/// A cache's shape, from its header.
#[derive(Clone, Copy)]
struct Cache {
    at: u64,
    gens: u32,
    max: u32,
    slots: u32,
    stride: u32,
    first: u32,
    entries: u32,
}

impl Cache {
    unsafe fn open(cache: u64) -> Cache {
        unsafe {
            Cache {
                at: cache,
                gens: load(cache + c::GENS as u64),
                max: load(cache + c::MAX as u64),
                slots: load(cache + c::SLOTS as u64),
                stride: load(cache + c::GEN_STRIDE as u64),
                first: load(cache + c::FIRST_ENTRY as u64),
                entries: load(cache + c::ENTRIES as u64),
            }
        }
    }

    /// The generation of epoch `e`.
    fn gen_of(self, e: u32) -> u64 {
        self.at + c::HEAD as u64 + u64::from((e % self.gens).wrapping_mul(self.stride))
    }

    /// Where entry `n` of a generation's index's offsets is written, and the entry.
    fn offset_at(self, gen_: u64, n: u32) -> u64 {
        gen_ + (u64::from(n) << 2) + (u64::from(self.slots) << 2) + c::INDEX as u64
    }

    unsafe fn entry(self, gen_: u64, n: u32) -> u64 {
        unsafe { gen_ + u64::from(load(self.offset_at(gen_, n))) + u64::from(self.first) }
    }

    /// Slot `p` of a key's probes in a generation's index (its hash `h`), masked by the most slots too
    /// (`SLOTS_MAX`), as the kernels' is.
    fn slot(self, gen_: u64, h: u32, p: u32) -> u64 {
        gen_ + u64::from((h.wrapping_add(p) & (self.slots.wrapping_sub(1)) & (c::SLOTS_MAX - 1)) * 4) + c::INDEX as u64
    }

    /// Generation `gen_` (unstamped) cleared and stamped `stamp` as a spare, when no writer is in it.
    unsafe fn clear(self, gen_: u64, stamp: u32) {
        unsafe {
            if load(gen_ + c::WRITERS as u64) != 0 {
                return;
            }
            if cas(gen_ + c::STAMP as u64, 0, c::CLEARING) != 0 {
                return;
            }
            for j in 0..self.slots {
                store(gen_ + u64::from(j * 4) + c::INDEX as u64, 0);
            }
            store(gen_ + c::USED as u64, 0);
            store(gen_ + c::BUMP as u64, 0);
            fence(SeqCst);
            store(gen_ + c::STAMP as u64, stamp);
        }
    }

    /// The head full at `epoch`: the spare turned into the head by the thread whose compare-and-swap of the epoch
    /// wins, which then clears the next spare; a spare left uncleared cleared now, if its writers are done.
    unsafe fn full(self, epoch: u32) {
        unsafe {
            let next = epoch.wrapping_add(1);
            let spare = self.gen_of(next);
            let ready = load(spare + c::STAMP as u64);
            if ready != next {
                if ready == 0 {
                    self.clear(spare, next);
                }
                return;
            }
            if cas(self.at + c::EPOCH as u64, epoch, next) != epoch {
                return;
            }
            add(self.at + c::ROTATED as u64, 1);
            let gen_ = self.gen_of(epoch.wrapping_add(2));
            store(gen_ + c::STAMP as u64, 0);
            fence(SeqCst);
            self.clear(gen_, epoch.wrapping_add(2));
        }
    }
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
        col[q + 1] = (ends[q] + side[q] + 16 + nb - 1) / nb;
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

/// The payload of entry `from` (`len` bytes) copied into entry `e`, and whether `from` was still complete at `stamp`.
unsafe fn copy_entry(from: u64, stamp: u32, e: u64, len: u32) -> bool {
    unsafe {
        let p = c::PAYLOAD as u64;
        if u64::from(len) > p {
            copy(from + p, e + p, (u64::from(len) - p).next_multiple_of(8));
        }
        fence(SeqCst);
        load(from + c::READY as u64) == stamp
    }
}

/// An entry of `size` bytes of the head at `epoch` taken for `key`, filled, completed and indexed: its address, or 0
/// where there was none. Filled with block `block` decoded, or with entry `from`'s payload (a promotion). The kernels'
/// `clif_tc_take`, step for step.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
unsafe fn take(t: InPlace, v: &TapeView, cc: Cache, epoch: u32, key: u64, h: u32, tag: u32, size: u32, block: u32, from: u64, from_stamp: u32, own: bool) -> u64 {
    unsafe {
        let head = cc.gen_of(epoch);
        let (mut slot, mut e, mut won, mut full) = (0u64, 0u64, false, false);
        if load(head + c::STAMP as u64) == epoch {
            let mut looking = true;
            let mut p = 0;
            while looking && p < c::PROBES {
                slot = cc.slot(head, h, p);
                let was = cas(slot, 0, tag);
                if was == 0 {
                    won = true;
                    looking = false;
                } else if !own && (was & !c::NUMBER) == tag {
                    let number = was & c::NUMBER;
                    if number != 0 {
                        fence(SeqCst);
                    }
                    if number == 0 || key_at(cc.entry(head, number - 1) + c::KEY as u64) == key {
                        looking = false;
                    }
                }
                p += 1;
            }
            full = looking;
        }
        if won {
            add(head + c::WRITERS as u64, 1);
            fence(SeqCst);
            if load(head + c::STAMP as u64) != epoch {
                store(slot, 0);
                add(head + c::WRITERS as u64, !0);
                won = false;
            }
        }
        let (mut n, mut off) = (0u32, 0u32);
        if won {
            n = add(head + c::USED as u64, 1);
            let mut room = false;
            if n < cc.entries {
                let rounded = (size + (c::ALIGN as u32 - 1)) & !(c::ALIGN as u32 - 1);
                off = add(head + c::BUMP as u64, rounded);
                room = u64::from(off) + u64::from(rounded) <= u64::from(cc.stride - cc.first);
            }
            if !room {
                store(slot, 0);
                add(head + c::WRITERS as u64, !0);
                won = false;
                full = true;
            }
        }
        if won {
            store(cc.offset_at(head, n), off);
            let entry = cc.entry(head, n);
            store(entry + c::READY as u64, epoch | c::WRITING);
            store(entry + c::LEN as u64, size);
            fence(SeqCst);
            (*at::<AtomicU64>(entry + c::KEY as u64)).store(key, SeqCst);
            fence(SeqCst);
            store(slot, tag | (n + 1));
            let fits = if from != 0 { copy_entry(from, from_stamp, entry, size) } else { decode(t, v, block, entry, u64::from(size)) };
            fence(SeqCst);
            if fits {
                store(entry + c::READY as u64, epoch);
                fence(SeqCst);
                e = entry;
            } else {
                store(entry + c::READY as u64, 0);
                fence(SeqCst);
                store(slot, n + 1);
                fence(SeqCst);
            }
            add(head + c::WRITERS as u64, !0);
        }
        if full {
            cc.full(epoch);
        }
        e
    }
}

/// What a probe of a generation found.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Found {
    NotHere,
    Pending,
    Found(u64),
}

/// The key's entry in generation `gen_` (stamped `stamp`), by its probes (the kernels' `clif_tc_probe`).
unsafe fn probe(cc: Cache, gen_: u64, stamp: u32, key: u64, h: u32, tag: u32) -> Found {
    unsafe {
        let mut found = Found::NotHere;
        for p in 0..c::PROBES {
            let x = load(cc.slot(gen_, h, p));
            if x == 0 {
                break;
            }
            if (x & !c::NUMBER) != tag {
                continue;
            }
            if (x & c::NUMBER) == 0 {
                found = Found::Pending;
                continue;
            }
            fence(SeqCst);
            let entry = cc.entry(gen_, (x & c::NUMBER) - 1);
            if key_at(entry + c::KEY as u64) == key {
                let ready = load(entry + c::READY as u64);
                if ready == stamp {
                    stat(cc.at, c::HITS);
                    return Found::Found(entry);
                } else if ready == (stamp | c::WRITING) {
                    found = Found::Pending;
                }
            }
        }
        found
    }
}

/// The entry of block `block`'s `key` in the rolling cache at `cache`: looked up in its live generations, head first,
/// a hit in the oldest copied into the head; on a miss, an entry of `size` bytes taken and the block decoded into it.
/// The kernels' `clif_tc_get`, step for step: a pending entry waited for `SPIN` turns at most, then decoded into an
/// entry of its own; after `TRIES` tries without room, or for a size past the most, a failure counted and the dead
/// entry given.
unsafe fn get(t: InPlace, v: &TapeView, cache: u64, key: u64, size: u32, block: u32) -> u64 {
    unsafe {
        let cc = Cache::open(cache);
        let h = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as u32;
        let tag = (((key.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) >> 48) as u32) | 1) << c::TAG_SHIFT;
        let live = cc.gens - 1;
        let (mut tries, mut waits, mut waited) = (0u32, 0u32, false);
        let mut got = 0u64;
        loop {
            let epoch = load(cache + c::EPOCH as u64);
            let (mut found, mut k, mut stamp) = (Found::NotHere, 0u32, 0u32);
            let mut j = 0;
            while found == Found::NotHere && j < live {
                let want = epoch.wrapping_sub(j);
                let gen_ = cc.gen_of(want);
                let s = load(gen_ + c::STAMP as u64);
                if s == want {
                    found = probe(cc, gen_, s, key, h, tag);
                    k = j;
                    stamp = s;
                }
                j += 1;
            }
            let mut own = false;
            if found == Found::Pending && load(cache + c::FAILED as u64) != 0 {
                break;
            } else if found == Found::Pending {
                waited = true;
                waits += 1;
                if waits >= c::SPIN {
                    stat(cc.at, c::DUPLICATES);
                    waits = 0;
                    own = true;
                    found = Found::NotHere;
                } else {
                    fence(SeqCst);
                    std::hint::spin_loop();
                }
            }
            match found {
                Found::Found(e) => {
                    if waited {
                        stat(cc.at, c::WAITED);
                    }
                    got = e;
                    if k == live - 1 && live > 1 {
                        let copied = take(t, v, cc, epoch, key, h, tag, load(e + c::LEN as u64), block, e, stamp, false);
                        if copied != 0 {
                            add(cache + c::PROMOTED as u64, 1);
                            got = copied;
                        }
                    }
                    break;
                }
                Found::NotHere => {
                    if size == 0 {
                        got = dead(cache);
                        break;
                    } else if size > cc.max {
                        break;
                    }
                    got = take(t, v, cc, epoch, key, h, tag, size, block, 0, 0, own);
                    if got != 0 {
                        add(cache + c::MISSES as u64, 1);
                        break;
                    }
                    tries += 1;
                    if tries >= c::TRIES {
                        break;
                    }
                }
                Found::Pending => {}
            }
        }
        if got == 0 {
            add(cache + c::FAILED as u64, 1);
            got = dead(cache);
        }
        got
    }
}

/// The address of the cache entry that holds the tape's block `block` decoded; the dead entry where the cache had no
/// room, or for a block past the tape's last (a failure counted).
#[inline(never)]
fn block_entry(t: InPlace, v: &TapeView, block: u32) -> u64 {
    let cache = cache_of(t, v);
    let blocks = (v.nodes + (BLOCK - 1)) >> 10;
    // SAFETY: `InPlace::new`'s caller's: the view's cache is the tape's, laid out as `cache::Spec` made it.
    unsafe {
        if block >= blocks {
            add(cache + c::FAILED as u64, 1);
            return dead(cache);
        }
        get(t, v, cache, key_of(v, block), v.columns[3], block)
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
