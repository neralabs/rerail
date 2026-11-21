//! The rolling cache (decision 0450, its layout `view::cache`) with its reads written directly in Rust, as the
//! numbering's are (`ids::Numbering`, decision 0517): a lookup in the live generations, head first; an entry taken
//! from the head's arena; a full head turned; a spare cleared once no writer is in it. And the cache's two users: a
//! text test's verdicts (decision 0478 step 2d) and a function's text (step 2e). The same in CLIF
//! (`reads::cache_lookup`, `verdict_reads`) stays the JIT's and the kernels'; the tests hold the two equal, step for
//! step, on one cache each (`the_rust_cache_is_the_jits`).
//!
//! Every u32 of the header, a generation and an entry's completion is read and written by any thread or the device:
//! sequentially consistent atomics, as CLIF's are, with its fences where it has them. An entry's key and payload are
//! plain words, written before a fence and read after one.

use crate::view::cache as c;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

/// The u32 at `p + off`.
///
/// # Safety
/// `p + off` is a 4-aligned u32 of a live cache.
unsafe fn word<'a>(p: *const u8, off: i32) -> &'a AtomicU32 {
    // SAFETY: the caller's.
    unsafe { AtomicU32::from_ptr(p.offset(off as isize).cast_mut().cast()) }
}

/// The u64 at `p + off` (an entry's key, a payload word): read and written plainly, the cache's fences ordering it.
///
/// # Safety
/// `p + off` is an 8-aligned u64 of a live cache.
unsafe fn plain<'a>(p: *const u8, off: i32) -> &'a AtomicU64 {
    // SAFETY: the caller's.
    unsafe { AtomicU64::from_ptr(p.offset(off as isize).cast_mut().cast()) }
}

/// Each u32 read by any thread: these four as CLIF's atomics are.
unsafe fn load(p: *const u8, off: i32) -> u32 {
    // SAFETY: the caller's.
    unsafe { word(p, off) }.load(Ordering::SeqCst)
}
unsafe fn store(p: *const u8, off: i32, v: u32) {
    // SAFETY: the caller's.
    unsafe { word(p, off) }.store(v, Ordering::SeqCst);
}
/// The value before.
unsafe fn add(p: *const u8, off: i32, v: u32) -> u32 {
    // SAFETY: the caller's.
    unsafe { word(p, off) }.fetch_add(v, Ordering::SeqCst)
}
/// The value found.
unsafe fn cas(p: *const u8, off: i32, old: u32, new: u32) -> u32 {
    // SAFETY: the caller's.
    match unsafe { word(p, off) }.compare_exchange(old, new, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(x) | Err(x) => x,
    }
}

fn barrier() {
    fence(Ordering::SeqCst);
}

/// A cache's shape and a key's place in it, read once a lookup.
struct Look {
    cache: *const u8,
    gens: u32,
    slots: u32,
    stride: u32,
    first: u32,
    entries: u32,
    /// The key, its slot (the high bits of a multiplicative hash) and its tag (16 bits of a second hash, never 0,
    /// at `TAG_SHIFT`, so a pending slot is not empty).
    key: u64,
    h: u32,
    tag: u32,
}

impl Look {
    /// # Safety
    /// `cache` is a cache `Spec::init` made, live.
    unsafe fn new(cache: *const u8, key: u64) -> Look {
        // SAFETY: the header's u32s.
        unsafe {
            let h = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as u32;
            let t = (key.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) >> 48) as u32 | 1;
            Look {
                cache,
                gens: load(cache, c::GENS),
                slots: load(cache, c::SLOTS),
                stride: load(cache, c::GEN_STRIDE),
                first: load(cache, c::FIRST_ENTRY),
                entries: load(cache, c::ENTRIES),
                key,
                h,
                tag: t << c::TAG_SHIFT,
            }
        }
    }

    /// The generation of epoch `e`.
    fn generation(&self, e: u32) -> *const u8 {
        let at = (e % self.gens).wrapping_mul(self.stride) as usize + c::HEAD;
        self.cache.wrapping_add(at)
    }

    /// Index slot `p` of the key's probes in `generation`: its u32's address.
    fn slot(&self, generation: *const u8, p: u32) -> *const u8 {
        let j = self.h.wrapping_add(p) & self.slots.wrapping_sub(1);
        generation.wrapping_add(j.wrapping_mul(4) as usize)
    }

    /// Where entry `n`'s offset lies in `generation` (after its index): its u32's address, less `INDEX`.
    fn offset_at(&self, generation: *const u8, n: u32) -> *const u8 {
        generation.wrapping_add(((n as usize) << 2) + ((self.slots as usize) << 2))
    }

    /// Entry `n` of `generation`.
    ///
    /// # Safety
    /// `n` is an entry `generation` holds.
    unsafe fn entry(&self, generation: *const u8, n: u32) -> *const u8 {
        // SAFETY: the caller's.
        let off = unsafe { load(self.offset_at(generation, n), c::INDEX) };
        generation.wrapping_add(off as usize + self.first as usize)
    }

    /// The statistics mode's count at `at`.
    unsafe fn stat(&self, at: i32) {
        // SAFETY: the header's u32s.
        unsafe {
            if load(self.cache, c::STATS) != 0 {
                add(self.cache, at, 1);
            }
        }
    }

    /// The dead entry, with `what` in its low bits where the lookup is open.
    unsafe fn dead(&self, open: bool, what: i64) -> u64 {
        // SAFETY: the header's u32.
        let dead = self.cache as u64 + u64::from(unsafe { load(self.cache, c::DEAD) });
        if open { dead | what as u64 } else { dead }
    }
}

/// An entry's payload written by a lookup that takes it (`get`): whether it fits.
pub type Fill<'a> = &'a mut dyn FnMut(*mut u8) -> bool;

/// The entry of `key` in the rolling cache at `cache`: looked up in its live generations, head first, a hit in the
/// oldest copied into the head, an entry another thread writes waited for (`SPIN` times at most); on a miss, an entry
/// of `size` bytes (its header and payload) taken from the head's arena, indexed, `fill`ed (its payload: `fill` gives
/// whether it fits) and completed. A full head turns the spare into the head. After `TRIES` tries without room, or
/// for a `size` past the cache's most, a failure counted and the dead entry given.
///
/// # Safety
/// `cache` is a cache `Spec::init` made, live; `fill` writes the entry's payload, its `size` less `PAYLOAD` bytes,
/// alone.
pub unsafe fn get(cache: *const u8, key: u64, size: u32, fill: Fill<'_>) -> *mut u8 {
    // SAFETY: the caller's.
    unsafe { lookup(cache, key, size, Some(fill)) as *mut u8 }
}

/// `get`, or with no `fill` an open lookup: it waits for no writer, and on a miss takes an entry of `size` bytes
/// (none for 0) that its caller writes and completes (`done`); its result an entry and what it found (`FOUND`,
/// `TAKEN`, `NOT_KEPT`, `PENDING`) in the low bits.
///
/// # Safety
/// As `get`'s; an entry taken open is its caller's to write and complete.
pub unsafe fn lookup(cache: *const u8, key: u64, size: u32, mut fill: Option<Fill<'_>>) -> u64 {
    let open = fill.is_none();
    // SAFETY: the cache's own u32s, entries and words, each where its layout puts it (`view::cache`).
    unsafe {
        let l = Look::new(cache, key);
        let max = load(cache, c::MAX);
        let live = l.gens.wrapping_sub(1);
        let mut tries = 0u32;
        'try_: loop {
            let epoch = load(cache, c::EPOCH);
            // The entry found, its generation's place back from the head, and its stamp.
            let mut found: Option<(*const u8, u32, u32)> = None;
            'generations: for k in 0..live {
                let want = epoch.wrapping_sub(k);
                let generation = l.generation(want);
                let stamp = load(generation, c::STAMP);
                if stamp != want {
                    continue;
                }
                let mut p = 0;
                'probes: while p < c::PROBES {
                    let at = l.slot(generation, p);
                    p += 1;
                    let mut x = load(at, c::INDEX);
                    loop {
                        if x == 0 {
                            continue 'generations;
                        }
                        if x & !c::NUMBER != l.tag {
                            continue 'probes;
                        }
                        let number = x & c::NUMBER;
                        if number != 0 {
                            break;
                        }
                        // Ours, pending (its writer has not taken an entry yet): waited for.
                        if open {
                            return l.dead(open, c::PENDING);
                        }
                        let mut spin = 0;
                        loop {
                            if spin >= c::SPIN {
                                l.stat(c::DUPLICATES);
                                break 'generations;
                            }
                            barrier();
                            spin += 1;
                            if load(at, c::INDEX) != x {
                                // Moved: the slot read again.
                                x = load(at, c::INDEX);
                                break;
                            }
                        }
                    }
                    // The fence pairing with the writer's before it put the number: the entry's offset, key and
                    // completion as written.
                    barrier();
                    let e = l.entry(generation, (x & c::NUMBER) - 1);
                    let same = plain(e, c::KEY).load(Ordering::Relaxed) == key;
                    let ready = load(e, c::READY);
                    if same && ready == stamp {
                        l.stat(c::HITS);
                        found = Some((e, k, stamp));
                        break 'generations;
                    }
                    // Its block being written by another thread: waited for, a bounded number of times, else made
                    // here too (a duplicate).
                    let writing = stamp | c::WRITING;
                    if !(same && ready == writing) {
                        continue;
                    }
                    if open {
                        return l.dead(open, c::PENDING);
                    }
                    let mut spin = 0;
                    loop {
                        if spin >= c::SPIN {
                            l.stat(c::DUPLICATES);
                            break 'generations;
                        }
                        barrier();
                        let now = load(e, c::READY);
                        spin += 1;
                        if now == stamp {
                            l.stat(c::WAITED);
                            found = Some((e, k, stamp));
                            break 'generations;
                        }
                        if now != writing {
                            continue 'probes;
                        }
                    }
                }
            }
            if let Some((e, k, stamp)) = found {
                // A hit: from the oldest live generation (of two or more), copied into the head.
                if !(k == live.wrapping_sub(1) && live > 1) {
                    return e as u64;
                }
                let len = load(e, c::LEN);
                let mut copy = |to: *mut u8| {
                    // The payload, a word at a time, and the source's completion read again after it.
                    let mut at = c::PAYLOAD as u32;
                    while at < len {
                        let w = plain(e, at as i32).load(Ordering::Relaxed);
                        plain(to, at as i32).store(w, Ordering::Relaxed);
                        at += 8;
                    }
                    barrier();
                    load(e, c::READY) == stamp
                };
                let copied = take(&l, epoch, len, Some(&mut copy));
                if copied == 0 {
                    return e as u64;
                }
                add(cache, c::PROMOTED, 1);
                return copied;
            }
            // A miss: an entry of the head taken and filled, else (the head full) the spare turned and tried again.
            // An entry past the most is never taken: no generation turns for it. A find alone (a size of 0) takes
            // none and fails none.
            if size == 0 {
                return l.dead(open, c::NOT_KEPT);
            }
            if size <= max {
                let taken = take(&l, epoch, size, fill.as_mut().map(|f| &mut **f as &mut dyn FnMut(*mut u8) -> bool));
                if taken != 0 {
                    add(cache, c::MISSES, 1);
                    return if open { taken | c::TAKEN as u64 } else { taken };
                }
                tries += 1;
                if tries < c::TRIES {
                    continue 'try_;
                }
            }
            add(cache, c::FAILED, 1);
            return l.dead(open, c::NOT_KEPT);
        }
    }
}

/// An entry of `size` bytes of the head at `epoch` taken for the key from the head's arena, `fill`ed (it gives
/// whether the payload fits), completed and indexed: its address, or 0 where there was none (the head full, its index
/// or its arena: the spare turned into the head if it was cleared, by the thread whose compare-and-swap of the epoch
/// won, which clears the next spare once its writers are done; or the payload too large).
///
/// A writer counts itself in its generation (`WRITERS`) from before it takes an entry to its completion, and checks
/// the generation is still its epoch's after: a generation is cleared only with no writer in it, so no write of an
/// older epoch's lands in an entry of a newer one.
unsafe fn take(l: &Look, epoch: u32, size: u32, fill: Option<&mut dyn FnMut(*mut u8) -> bool>) -> u64 {
    // SAFETY: the head's own u32s, slots and entries, each where its layout puts it.
    unsafe {
        let head = l.generation(epoch);
        if load(head, c::STAMP) != epoch {
            return 0;
        }
        // Its slot in the head's index claimed first, pending (its tag, no entry yet), so a thread that misses the
        // same key at once finds it and waits rather than taking an entry too. A slot of the same key already: none
        // (the lookup, tried again, finds it). No slot free in its probes: the head full.
        let mut slot = None;
        for p in 0..c::PROBES {
            let at = l.slot(head, p);
            let was = cas(at, c::INDEX, 0, l.tag);
            if was == 0 {
                slot = Some(at);
                break;
            }
            if was & !c::NUMBER == l.tag {
                let number = was & c::NUMBER;
                if number == 0 {
                    return 0;
                }
                barrier();
                if plain(l.entry(head, number - 1), c::KEY).load(Ordering::Relaxed) == l.key {
                    return 0;
                }
            }
        }
        let Some(slot) = slot else {
            full(l, epoch);
            return 0;
        };
        // The slot this thread's: the thread counted a writer of the head, the head checked still its epoch's, an
        // entry number and its bytes taken, else (the head turned, or full) the slot given back.
        add(head, c::WRITERS, 1);
        barrier();
        if load(head, c::STAMP) != epoch {
            store(slot, c::INDEX, 0);
            add(head, c::WRITERS, u32::MAX);
            return 0;
        }
        let n = add(head, c::USED, 1);
        let mut at = 0;
        let room = n < l.entries && {
            // Its bytes, rounded to the arena's alignment: past the arena's end, the head full.
            let rounded = size.wrapping_add(c::ALIGN as u32 - 1) & !(c::ALIGN as u32 - 1);
            at = add(head, c::BUMP, rounded);
            let arena = load(l.cache, c::GEN_STRIDE).wrapping_sub(load(l.cache, c::FIRST_ENTRY));
            u64::from(at) + u64::from(rounded) <= u64::from(arena)
        };
        if !room {
            store(slot, c::INDEX, 0);
            add(head, c::WRITERS, u32::MAX);
            full(l, epoch);
            return 0;
        }
        // Its offset under its number, its completion marked as being written and its bytes, its key (a fence
        // after), then (a fence after the key) its number in the slot, so a waiter finds it being written; then its
        // payload and (a fence after it) its completion.
        store(l.offset_at(head, n), c::INDEX, at);
        let e = l.entry(head, n);
        store(e, c::READY, epoch | c::WRITING);
        store(e, c::LEN, size);
        barrier();
        plain(e, c::KEY).store(l.key, Ordering::Relaxed);
        barrier();
        store(slot, c::INDEX, l.tag | (n + 1));
        // Open (no fill): the entry is its caller's to write and complete (`done`), its writer still counted.
        let Some(fill) = fill else { return e as u64 };
        if !fill(e.cast_mut()) {
            // Too large for an entry: the entry given back.
            store(e, c::READY, 0);
            barrier();
            add(head, c::WRITERS, u32::MAX);
            return 0;
        }
        barrier();
        store(e, c::READY, epoch);
        barrier();
        add(head, c::WRITERS, u32::MAX);
        e as u64
    }
}

/// The head full at `epoch`: the spare (cleared ahead, stamped `epoch + 1`) turned into the head; a spare left
/// uncleared (stamped 0: its writers were not done when its turn came) cleared now, if they are.
unsafe fn full(l: &Look, epoch: u32) {
    // SAFETY: the header's and the generations' u32s.
    unsafe {
        let next = epoch.wrapping_add(1);
        let spare = l.generation(next);
        let ready = load(spare, c::STAMP);
        if ready != next {
            if ready == 0 {
                clear(l, spare, next);
            }
            return;
        }
        if cas(l.cache, c::EPOCH, epoch, next) != epoch {
            return;
        }
        // The next spare: the oldest live generation of `epoch`, unstamped (no lookup or writer takes it from now),
        // then cleared and stamped once no writer is in it.
        add(l.cache, c::ROTATED, 1);
        let spare_epoch = epoch.wrapping_add(2);
        let generation = l.generation(spare_epoch);
        store(generation, c::STAMP, 0);
        barrier();
        clear(l, generation, spare_epoch);
    }
}

/// `generation` (unstamped) cleared and stamped `stamp` as a spare, when no writer is in it: one thread clears it
/// (its stamp's compare-and-swap from 0 to `CLEARING`), its index, entries and arena given back. With a writer still
/// in it, it stays unstamped, and a later full head clears it.
unsafe fn clear(l: &Look, generation: *const u8, stamp: u32) {
    // SAFETY: the generation's u32s and index.
    unsafe {
        if load(generation, c::WRITERS) != 0 || cas(generation, c::STAMP, 0, c::CLEARING) != 0 {
            return;
        }
        for j in 0..l.slots {
            store(generation.wrapping_add(j.wrapping_mul(4) as usize), c::INDEX, 0);
        }
        store(generation, c::USED, 0);
        store(generation, c::BUMP, 0);
        barrier();
        store(generation, c::STAMP, stamp);
    }
}

/// Entry `e` of the cache at `cache`, taken by an open lookup (`TAKEN`'s bit cleared or not) and written: completed
/// (its epoch, the writing bit cleared, after a fence) and its writer gone from its generation's count.
///
/// # Safety
/// `e` is an entry this thread took open from the live cache at `cache`, not yet completed.
pub unsafe fn done(cache: *const u8, e: u64) {
    let e = (e & !3) as *const u8;
    // SAFETY: the caller's: the entry's and its generation's u32s.
    unsafe {
        barrier();
        let ready = load(e, c::READY);
        store(e, c::READY, ready & !c::WRITING);
        barrier();
        let stride = load(cache, c::GEN_STRIDE) as usize;
        let at = e as usize - cache as usize - c::HEAD;
        let generation = cache.wrapping_add(at / stride * stride + c::HEAD);
        add(generation, c::WRITERS, u32::MAX);
    }
}

/// The masks' offsets in a verdicts' entry: symbols known to pass, then to fail.
pub const YES: i32 = c::PAYLOAD;
pub const NO: i32 = c::PAYLOAD + 8;
/// A verdict: none yet, the symbol fails, it passes.
pub const UNKNOWN: u32 = 0;
pub const FAILS: u32 = 1;
pub const PASSES: u32 = 2;

/// The entry of `test`'s 64 symbols about `sym` (made, both masks empty, when none is), as the word of `sym` in a
/// mask from the mask, and its bit; `None` for the dead entry. An entry's key is `(test + 1) << 32 | sym >> 6`.
unsafe fn verdicts(cache: *const u8, test: u32, sym: u32) -> Option<(*const u8, u32)> {
    let key = (u64::from(test) + 1) << 32 | u64::from(sym >> 6);
    // SAFETY: the caller's; the fill writes the new entry's two masks, its own.
    unsafe {
        let e = get(cache, key, c::PAYLOAD as u32 + 16, &mut |e| {
            plain(e, YES).store(0, Ordering::Relaxed);
            plain(e, NO).store(0, Ordering::Relaxed);
            true
        });
        if e.cast_const() == cache.wrapping_add(load(cache, c::DEAD) as usize) {
            return None;
        }
        Some((e.cast_const().wrapping_add((((sym >> 5) & 1) << 2) as usize), 1 << (sym & 31)))
    }
}

/// `test`'s verdict of `sym`: `PASSES`, `FAILS`, or `UNKNOWN` (none kept: made by `verdict_put`).
///
/// # Safety
/// `cache` is a cache `Spec::init` made, live.
pub unsafe fn verdict_get(cache: *const u8, test: u32, sym: u32) -> u32 {
    // SAFETY: the caller's; the entry's masks.
    unsafe {
        let Some((at, bit)) = verdicts(cache, test, sym) else { return UNKNOWN };
        if load(at, YES) & bit != 0 {
            PASSES
        } else if load(at, NO) & bit != 0 {
            FAILS
        } else {
            UNKNOWN
        }
    }
}

/// Verdict `v` (`PASSES` or `FAILS`) of `test` for `sym` kept: 1, else 0 (the cache had no room: the dead entry). A
/// verdict is one atomic or into a mask, so a copy of the entry may miss one but never gives a wrong one.
///
/// # Safety
/// As `verdict_get`'s.
pub unsafe fn verdict_put(cache: *const u8, test: u32, sym: u32, v: u32) -> u32 {
    // SAFETY: the caller's; the entry's masks.
    unsafe {
        let Some((at, bit)) = verdicts(cache, test, sym) else { return 0 };
        word(at, if v == PASSES { YES } else { NO }).fetch_or(bit, Ordering::SeqCst);
        1
    }
}

/// A text's key: bit 63 set (no verdict's key has it), the functions' id plus one, the symbol.
fn text_key(fns: u32, sym: u32) -> u64 {
    (u64::from(fns) + 1) << 32 | 1 << 63 | u64::from(sym)
}

/// The text of functions `fns` of symbol `sym` kept in the cache: its entry (its bytes `LEN` less `PAYLOAD`, at
/// `PAYLOAD`), with what was found in the low bits (`FOUND`, `NOT_KEPT`, `PENDING`).
///
/// # Safety
/// As `verdict_get`'s.
pub unsafe fn text_find(cache: *const u8, fns: u32, sym: u32) -> u64 {
    // SAFETY: the caller's.
    unsafe { lookup(cache, text_key(fns, sym), 0, None) }
}

/// `text_find`, or on a miss an entry taken for a text of `len` bytes (`TAKEN`): its caller writes the text at
/// `PAYLOAD`, then `done`. A text past the cache's most is not kept (`NOT_KEPT`).
///
/// # Safety
/// As `verdict_get`'s; an entry taken is its caller's to write and complete.
pub unsafe fn text_open(cache: *const u8, fns: u32, sym: u32, len: u32) -> u64 {
    // SAFETY: the caller's.
    unsafe { lookup(cache, text_key(fns, sym), len.wrapping_add(c::PAYLOAD as u32), None) }
}
