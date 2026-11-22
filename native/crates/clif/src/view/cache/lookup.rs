//! The rolling cache's lookups in Rust, written directly (decision 0536, the user's standing rule: all generated Rust
//! is written directly, never translated from CLIF): the host's reads of a cache (`super`'s layout), and of the
//! verdicts and texts it keeps (decision 0478 steps 2d, 2e). The reads in CLIF (`reads::cache_lookup`,
//! `verdict_reads`) are the JIT's and the kernels'; these take the same steps over the same bytes, with the same
//! atomics and fences, so a host thread and a device's share one cache. Tests hold them to each other
//! (`verdict_reads::tests`).
//!
//! Every function takes the cache's first byte, `cache`, of a buffer `Spec::init` made. The JIT's atomics are
//! sequentially consistent and its fences full ones; these are too.

use super::*;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::SeqCst, fence};

/// The u32 at `p + at`, read and written by any thread.
///
/// # Safety
/// `p + at` is a 4-aligned u32 of a live cache.
#[inline(always)]
unsafe fn word<'a>(p: *const u8, at: i32) -> &'a AtomicU32 {
    // SAFETY: the caller's.
    unsafe { &*p.offset(at as isize).cast::<AtomicU32>() }
}

/// An entry's key, read and written by any thread.
///
/// # Safety
/// `e` is an entry of a live cache.
#[inline(always)]
unsafe fn key_of<'a>(e: *const u8) -> &'a AtomicU64 {
    // SAFETY: the caller's: an entry is `ALIGN`-aligned, its key its first u64.
    unsafe { &*e.add(KEY as usize).cast::<AtomicU64>() }
}

/// A cache's shape, read from its header, and a key's hashes: what every step of a lookup takes.
struct Look {
    cache: *mut u8,
    gens: u32,
    slots: u32,
    stride: u32,
    first: u32,
    entries: u32,
    max: u32,
    /// The key's slot, its tag in a slot.
    h: u32,
    tag: u32,
    key: u64,
}

impl Look {
    /// # Safety
    /// `cache` is a live cache.
    unsafe fn new(cache: *mut u8, key: u64) -> Look {
        // SAFETY: the caller's: the header's words.
        let w = |at: i32| unsafe { word(cache, at).load(SeqCst) };
        // The key's slot: its high bits of a multiplicative hash. Its tag (`TAG_SHIFT`): 16 bits of a second hash,
        // never 0, so a pending slot (no entry yet) is not empty.
        let h = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as u32;
        let tag = (((key.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) >> 48) as u32) | 1) << TAG_SHIFT;
        Look { cache, gens: w(GENS), slots: w(SLOTS), stride: w(GEN_STRIDE), first: w(FIRST_ENTRY), entries: w(ENTRIES), max: w(MAX), h, tag, key }
    }

    fn header(&self, at: i32) -> &AtomicU32 {
        // SAFETY: a header word of the live cache (`new`'s).
        unsafe { word(self.cache, at) }
    }

    fn count(&self, at: i32) {
        self.header(at).fetch_add(1, SeqCst);
    }

    /// The statistics mode's count at `at`.
    fn stat(&self, at: i32) {
        if self.header(STATS).load(SeqCst) != 0 {
            self.count(at);
        }
    }

    fn dead(&self) -> *mut u8 {
        self.cache.wrapping_add(self.header(DEAD).load(SeqCst) as usize)
    }

    /// The generation of epoch `e`.
    fn gen_of(&self, e: u32) -> *mut u8 {
        self.cache.wrapping_add(HEAD + ((e % self.gens) * self.stride) as usize)
    }

    /// Generation `g`'s u32 at `at` (its header's, or its index's from `INDEX`).
    fn gen_word(&self, g: *const u8, at: usize) -> &AtomicU32 {
        // SAFETY: a word of a generation of the live cache.
        unsafe { &*g.add(at).cast::<AtomicU32>() }
    }

    /// Index slot `j` of generation `g`.
    fn slot(&self, g: *const u8, j: u32) -> &AtomicU32 {
        self.gen_word(g, INDEX as usize + 4 * j as usize)
    }

    /// Entry `n` of generation `g`: its offset in the arena, from the generation's offsets (after its index).
    fn entry_of(&self, g: *mut u8, n: u32) -> *mut u8 {
        let off = self.gen_word(g, INDEX as usize + 4 * (n as usize + self.slots as usize)).load(SeqCst);
        g.wrapping_add(off as usize + self.first as usize)
    }

    fn slot_of(&self, p: u32) -> u32 {
        self.h.wrapping_add(p) & (self.slots - 1)
    }
}

/// An entry's filler: given the entry taken, it writes the payload and gives whether it fits.
type Fill<'f> = &'f dyn Fn(*mut u8) -> bool;

/// The entry of `key` (`reads::cache_lookup`): looked up in the live generations, head first, a hit in the oldest
/// copied into the head; with `fill`, an entry another thread writes waited for (`SPIN` times at most), and on a miss
/// an entry of `size` bytes taken, `fill`ed and completed; with none (an open lookup) no writer waited for, and on a
/// miss an entry of `size` bytes (none for 0) taken for its caller to write and complete (`done`), what was found in
/// the result's low bits (`FOUND`, `TAKEN`, `NOT_KEPT`, `PENDING`).
///
/// # Safety
/// `cache` is a live cache (`Spec::init`), and `fill` writes only its entry's `size` bytes.
unsafe fn lookup(cache: *mut u8, key: u64, size: u32, fill: Option<Fill<'_>>) -> u64 {
    // SAFETY: the caller's.
    let l = unsafe { Look::new(cache, key) };
    let open = fill.is_none();
    let marked = |e: *mut u8, k: i64| if open { e as u64 | k as u64 } else { e as u64 };
    let live = l.gens - 1;
    for _ in 0..TRIES {
        let epoch = l.header(EPOCH).load(SeqCst);
        // Generation `k` back from the head.
        let mut found = None;
        'gens: for k in 0..live {
            let want = epoch.wrapping_sub(k);
            let g = l.gen_of(want);
            let stamp = l.gen_word(g, STAMP as usize).load(SeqCst);
            if stamp != want {
                continue;
            }
            let mut p = 0;
            while p < PROBES {
                let at = l.slot(g, l.slot_of(p));
                let x = at.load(SeqCst);
                if x == 0 {
                    continue 'gens;
                }
                if x & !NUMBER != l.tag {
                    p += 1;
                    continue;
                }
                let number = x & NUMBER;
                if number == 0 {
                    // Its writer has not taken an entry yet: waited for, else (an open lookup) pending.
                    if open {
                        return marked(l.dead(), PENDING);
                    }
                    let mut moved = false;
                    for _ in 0..SPIN {
                        fence(SeqCst);
                        if at.load(SeqCst) != x {
                            moved = true;
                            break;
                        }
                    }
                    if moved {
                        continue;
                    }
                    l.stat(DUPLICATES);
                    break 'gens;
                }
                // The fence pairing with the writer's before it put the number.
                fence(SeqCst);
                let e = l.entry_of(g, number - 1);
                // SAFETY: an entry of the live cache.
                let (same, ready) = unsafe { (key_of(e).load(SeqCst) == key, word(e, READY)) };
                let now = ready.load(SeqCst);
                if same && now == stamp {
                    l.stat(HITS);
                    found = Some((e, k, stamp));
                    break 'gens;
                }
                let writing = stamp | WRITING;
                if same && now == writing {
                    if open {
                        return marked(l.dead(), PENDING);
                    }
                    let mut spin = 0;
                    loop {
                        if spin >= SPIN {
                            l.stat(DUPLICATES);
                            break 'gens;
                        }
                        fence(SeqCst);
                        let now = ready.load(SeqCst);
                        spin += 1;
                        if now == stamp {
                            l.stat(WAITED);
                            found = Some((e, k, stamp));
                            break 'gens;
                        }
                        if now != writing {
                            break;
                        }
                    }
                }
                p += 1;
            }
        }
        if let Some((e, k, stamp)) = found {
            // A hit from the oldest live generation (of two or more): copied into the head.
            if k == live - 1 && live > 1 {
                // SAFETY: an entry of the live cache, complete.
                let len = unsafe { word(e, LEN).load(SeqCst) };
                let copy = |to: *mut u8| {
                    let mut at = PAYLOAD as usize;
                    while at < len as usize {
                        // SAFETY: both entries' bytes, `len` of them each (an entry's room is a multiple of 8).
                        unsafe {
                            let w = (*e.add(at).cast::<AtomicU64>()).load(std::sync::atomic::Ordering::Relaxed);
                            (*to.add(at).cast::<AtomicU64>()).store(w, std::sync::atomic::Ordering::Relaxed);
                        }
                        at += 8;
                    }
                    fence(SeqCst);
                    // SAFETY: as above.
                    unsafe { word(e, READY).load(SeqCst) == stamp }
                };
                // SAFETY: the caller's.
                let copied = unsafe { take(&l, epoch, len, Some(&copy)) };
                if !copied.is_null() {
                    l.count(PROMOTED);
                    return copied as u64;
                }
            }
            return e as u64;
        }
        // A miss: an entry of the head taken and filled, else (the head full) the spare turned and tried again. An
        // entry past the most is never taken: no generation turns for it.
        if size == 0 {
            return marked(l.dead(), NOT_KEPT);
        }
        if size > l.max {
            break;
        }
        // SAFETY: the caller's.
        let taken = unsafe { take(&l, epoch, size, fill) };
        if !taken.is_null() {
            l.count(MISSES);
            return marked(taken, TAKEN);
        }
    }
    l.count(FAILED);
    marked(l.dead(), NOT_KEPT)
}

/// An entry of `size` bytes of the head at `epoch` taken for the key (`reads::cache_take`): its slot claimed,
/// pending, then its writer counted, its number and bytes taken, its key and number written; `fill`ed and completed,
/// or (none) left to its caller. Null where there was none (the head turned or full: `full`'s turn of the spare; or
/// the payload too large).
///
/// # Safety
/// As `lookup`'s.
unsafe fn take(l: &Look, epoch: u32, size: u32, fill: Option<Fill<'_>>) -> *mut u8 {
    let none = std::ptr::null_mut();
    let head = l.gen_of(epoch);
    let stamp = l.gen_word(head, STAMP as usize);
    if stamp.load(SeqCst) != epoch {
        return none;
    }
    // Its slot in the head's index claimed first, pending (its tag, no entry yet). A slot of the same key already:
    // none (the lookup, tried again, finds it). No slot free in its probes: the head full.
    let mut slot = None;
    for p in 0..PROBES {
        let s = l.slot(head, l.slot_of(p));
        match s.compare_exchange(0, l.tag, SeqCst, SeqCst) {
            Ok(_) => {
                slot = Some(s);
                break;
            }
            Err(was) if was & !NUMBER == l.tag => {
                let number = was & NUMBER;
                if number == 0 {
                    return none;
                }
                fence(SeqCst);
                let other = l.entry_of(head, number - 1);
                // SAFETY: an entry of the live cache.
                if unsafe { key_of(other).load(SeqCst) } == l.key {
                    return none;
                }
            }
            Err(_) => {}
        }
    }
    let Some(slot) = slot else {
        full(l, epoch);
        return none;
    };
    // The slot this thread's: the thread counted a writer of the head, the head checked still its epoch's.
    let writers = l.gen_word(head, WRITERS as usize);
    writers.fetch_add(1, SeqCst);
    fence(SeqCst);
    if stamp.load(SeqCst) != epoch {
        slot.store(0, SeqCst);
        writers.fetch_sub(1, SeqCst);
        return none;
    }
    let n = l.gen_word(head, USED as usize).fetch_add(1, SeqCst);
    let rounded = (size + (ALIGN as u32 - 1)) & !(ALIGN as u32 - 1);
    let fits = n < l.entries && {
        let at = l.gen_word(head, BUMP as usize).fetch_add(rounded, SeqCst);
        let arena = l.stride - l.first;
        if u64::from(at) + u64::from(rounded) <= u64::from(arena) {
            l.gen_word(head, INDEX as usize + 4 * (n as usize + l.slots as usize)).store(at, SeqCst);
            true
        } else {
            false
        }
    };
    if !fits {
        slot.store(0, SeqCst);
        writers.fetch_sub(1, SeqCst);
        full(l, epoch);
        return none;
    }
    // Its completion marked as being written and its bytes, its key (a fence after), then its number in the slot.
    let e = l.entry_of(head, n);
    // SAFETY: the entry just taken, this thread's.
    unsafe {
        word(e, READY).store(epoch | WRITING, SeqCst);
        word(e, LEN).store(size, SeqCst);
        fence(SeqCst);
        key_of(e).store(l.key, SeqCst);
    }
    fence(SeqCst);
    slot.store(l.tag | (n + 1), SeqCst);
    // Open (no fill): the entry is its caller's to write and complete (`done`), its writer still counted.
    let Some(fill) = fill else { return e };
    // SAFETY: as above.
    let ready = unsafe { word(e, READY) };
    if !fill(e) {
        ready.store(0, SeqCst);
        fence(SeqCst);
        writers.fetch_sub(1, SeqCst);
        return none;
    }
    fence(SeqCst);
    ready.store(epoch, SeqCst);
    fence(SeqCst);
    writers.fetch_sub(1, SeqCst);
    e
}

/// The head full at `epoch` (`reads::cache_full`): the spare (stamped `epoch + 1`) turned into the head by the thread
/// whose compare-and-swap of the epoch wins, which unstamps the next spare and clears it; a spare left uncleared
/// (stamped 0) cleared now.
fn full(l: &Look, epoch: u32) {
    let next = epoch.wrapping_add(1);
    let spare = l.gen_of(next);
    let ready = l.gen_word(spare, STAMP as usize).load(SeqCst);
    if ready != next {
        if ready == 0 {
            clear(l, spare, next);
        }
        return;
    }
    if l.header(EPOCH).compare_exchange(epoch, next, SeqCst, SeqCst).is_err() {
        return;
    }
    l.count(ROTATED);
    let spare_epoch = epoch.wrapping_add(2);
    let g = l.gen_of(spare_epoch);
    l.gen_word(g, STAMP as usize).store(0, SeqCst);
    fence(SeqCst);
    clear(l, g, spare_epoch);
}

/// Generation `g` (unstamped) cleared and stamped `stamp` as a spare when no writer is in it (`reads::cache_clear`):
/// by the one thread whose compare-and-swap of its stamp from 0 to `CLEARING` wins.
fn clear(l: &Look, g: *mut u8, stamp: u32) {
    if l.gen_word(g, WRITERS as usize).load(SeqCst) != 0 {
        return;
    }
    let s = l.gen_word(g, STAMP as usize);
    if s.compare_exchange(0, CLEARING, SeqCst, SeqCst).is_err() {
        return;
    }
    for j in 0..l.slots {
        l.slot(g, j).store(0, SeqCst);
    }
    l.gen_word(g, USED as usize).store(0, SeqCst);
    l.gen_word(g, BUMP as usize).store(0, SeqCst);
    fence(SeqCst);
    s.store(stamp, SeqCst);
}

/// The entry of `key` (`reads::cache_get`, as a 16z block's read takes it: `z_block`): found in the live
/// generations, or on a miss an entry of `size` bytes taken and `fill`ed (it gives whether its payload fit) and
/// completed; an entry another thread writes waited for. The dead entry where the cache had no room (`FAILED`
/// counted): its caller reads it as no entry.
///
/// # Safety
/// `cache` is a live cache (`Spec::init`), and `fill` writes only its entry's `size` bytes.
pub unsafe fn get(cache: *mut u8, key: u64, size: u32, fill: &dyn Fn(*mut u8) -> bool) -> *mut u8 {
    // SAFETY: the caller's.
    unsafe { lookup(cache, key, size, Some(fill)) as *mut u8 }
}

/// Whether `e` is `cache`'s dead entry (`get` found no room).
///
/// # Safety
/// `cache` is a live cache.
pub unsafe fn is_dead(cache: *const u8, e: *const u8) -> bool {
    // SAFETY: the caller's.
    unsafe { e == cache.add(word(cache, DEAD).load(SeqCst) as usize) }
}

/// Entry `r` an open lookup took (`TAKEN`'s bits cleared or not), written: completed, and its writer gone from its
/// generation's count (`reads::cache_done`).
///
/// # Safety
/// `cache` is a live cache, and `r` an entry of it this thread took and has written.
pub unsafe fn done(cache: *const u8, r: u64) {
    let e = (r & !3) as *mut u8;
    fence(SeqCst);
    // SAFETY: the caller's.
    unsafe {
        let ready = word(e, READY);
        ready.store(ready.load(SeqCst) & !WRITING, SeqCst);
        fence(SeqCst);
        let stride = word(cache, GEN_STRIDE).load(SeqCst) as usize;
        let g = (e as usize - cache as usize - HEAD) / stride * stride;
        word(cache, (HEAD + g + WRITERS as usize) as i32).fetch_sub(1, SeqCst);
    }
}

/// A verdict's masks' offsets in an entry (symbols known to pass, then to fail), and the verdicts: none yet, the
/// symbol fails, it passes (`verdict_reads`).
pub const YES: i32 = PAYLOAD;
pub const NO: i32 = PAYLOAD + 8;
pub const UNKNOWN: u32 = 0;
pub const FAILS: u32 = 1;
pub const PASSES: u32 = 2;

/// The entry of `test`'s 64 symbols about `sym` (`verdict_entry`), both masks empty when it was made; and the offset
/// of `sym`'s word in a mask from the entry, and its bit. `None` for the dead entry.
///
/// # Safety
/// `cache` is a live cache.
unsafe fn verdict_entry(cache: *const u8, test: u32, sym: u32) -> Option<(*mut u8, u32)> {
    let key = ((u64::from(test) + 1) << 32) | u64::from(sym >> 6);
    let zero = |e: *mut u8| {
        // SAFETY: the entry taken, this thread's: its two masks.
        unsafe {
            (*e.add(YES as usize).cast::<AtomicU64>()).store(0, std::sync::atomic::Ordering::Relaxed);
            (*e.add(NO as usize).cast::<AtomicU64>()).store(0, std::sync::atomic::Ordering::Relaxed);
        }
        true
    };
    // SAFETY: the caller's.
    let e = unsafe { lookup(cache.cast_mut(), key, PAYLOAD as u32 + 16, Some(&zero)) } as *mut u8;
    // SAFETY: as above: the dead entry's offset in the header.
    let dead = cache.cast_mut().wrapping_add(unsafe { word(cache, DEAD) }.load(SeqCst) as usize);
    (e != dead).then(|| (e.wrapping_add(4 * ((sym >> 5) & 1) as usize), 1 << (sym & 31)))
}

/// `verdict_get(cache, test, sym)`: `PASSES`, `FAILS`, or `UNKNOWN` (none kept).
///
/// # Safety
/// `cache` is a live cache.
pub unsafe fn verdict_get(cache: *const u8, test: u32, sym: u32) -> u32 {
    // SAFETY: the caller's.
    let Some((at, bit)) = (unsafe { verdict_entry(cache, test, sym) }) else { return UNKNOWN };
    // SAFETY: the entry's masks.
    let (yes, no) = unsafe { (word(at, YES).load(SeqCst), word(at, NO).load(SeqCst)) };
    if yes & bit != 0 {
        PASSES
    } else if no & bit != 0 {
        FAILS
    } else {
        UNKNOWN
    }
}

/// `verdict_put(cache, test, sym, v)`: keeps verdict `v` (`PASSES` or `FAILS`); whether it was (none with no room).
///
/// # Safety
/// `cache` is a live cache.
pub unsafe fn verdict_put(cache: *const u8, test: u32, sym: u32, v: u32) -> bool {
    // SAFETY: the caller's.
    let Some((at, bit)) = (unsafe { verdict_entry(cache, test, sym) }) else { return false };
    // SAFETY: the entry's mask.
    unsafe { word(at, if v == PASSES { YES } else { NO }) }.fetch_or(bit, SeqCst);
    true
}

/// A text's key: bit 63 set (no verdict's key has it), the functions' id plus one, the symbol.
fn text_key(fns: u32, sym: u32) -> u64 {
    ((u64::from(fns) + 1) << 32) | (1 << 63) | u64::from(sym)
}

/// The text of functions `fns` of symbol `sym` kept in the cache: its entry (its bytes `LEN` less `PAYLOAD`, at
/// `PAYLOAD`), with what was found in the low bits (`FOUND`, `NOT_KEPT`, `PENDING`).
///
/// # Safety
/// `cache` is a live cache.
pub unsafe fn text_find(cache: *const u8, fns: u32, sym: u32) -> u64 {
    // SAFETY: the caller's.
    unsafe { lookup(cache.cast_mut(), text_key(fns, sym), 0, None) }
}

/// `text_find`, or on a miss an entry taken for a text of `len` bytes (`TAKEN`): its caller writes the text at
/// `PAYLOAD`, then `done`. A text past the cache's most is not kept (`NOT_KEPT`).
///
/// # Safety
/// `cache` is a live cache.
pub unsafe fn text_open(cache: *const u8, fns: u32, sym: u32, len: u32) -> u64 {
    // SAFETY: the caller's.
    unsafe { lookup(cache.cast_mut(), text_key(fns, sym), len + PAYLOAD as u32, None) }
}
