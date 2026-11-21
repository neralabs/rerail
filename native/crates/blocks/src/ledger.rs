//! A block's ledger (decision 0487): the one place that knows which of its chunks are live, which GPU work is queued
//! on it, and when its chunks and its memory may go. Every hand-out, free, page given to the OS and release of the
//! block's memory goes through it, and what cannot work fails here, the chunk named.
//!
//! ```text
//! let f = Fence::new();            // one per command buffer
//! f.reads(&block);                 // before a kernel that reads it is encoded
//! block.free_chunk(at, n);         // retiring: its bits stay live, nothing is allocated over it
//! f.complete();                    // the command buffer's completion handler: the chunk goes on the free list
//! ```
//!
//! Bookkeeping on the CPU (the user, 2026-11-19: "Bookkeeping / scoreboards / schedules can be on the CPU"): a bit a
//! 64 bytes for what is live and one for where each live chunk starts, in a block that keeps a free list; the GPU's
//! uses by epoch. The chunks themselves stay in the block.

use crate::block::{Extent, PAGE};
use crate::free::MIN_LOG;
use std::sync::atomic::{AtomicBool, Ordering::*};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// How a GPU use reaches the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// How a freed chunk goes back.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Freeing {
    /// As the pieces of its length, the largest first (`try_alloc_chunk_fit`'s), else as one chunk of its class.
    pub pieces: bool,
    /// Its whole pages past each piece's link go to the OS (an anonymous block's, when the owner asks).
    pub pages: bool,
}

/// A chunk freed while a GPU use that began before it was queued: freed when every use of an epoch up to `epoch` is.
struct Retiring {
    at: usize,
    len: usize,
    how: Freeing,
    epoch: u64,
}

/// A GPU use of one chunk, which the CPU's own access is checked against (R4).
struct ChunkUse {
    id: u64,
    at: usize,
    len: usize,
    access: Access,
}

#[derive(Default)]
struct State {
    /// The free list's heads' offset (0: none, a bump block or a frozen one).
    heads: usize,
    /// Whether chunks are tracked: a bit a 64 bytes, live and start.
    tracked: bool,
    live: Vec<u64>,
    starts: Vec<u64>,
    /// The epoch new GPU uses begin in; a retiring free moves it on.
    epoch: u64,
    /// GPU uses queued, by the epoch they began in, the oldest first.
    open: Vec<(u64, u32)>,
    chunks: Vec<ChunkUse>,
    next_id: u64,
    retiring: Vec<Retiring>,
    /// The block's memory, handed over by its drop while a use was queued: let go when the last completes.
    gone: Option<Box<dyn FnOnce() + Send>>,
}

/// A block's ledger. The block holds it, and so does every fence with a use of the block, so its memory outlives the
/// GPU's work.
pub struct Ledger {
    state: Mutex<State>,
    /// GPU uses of chunks queued now (`State::chunks`' length): with none, a CPU access is checked without the lock
    /// (a block's arrays are read and written at every value, `cpu_read`, `cpu_write`).
    chunk_uses: std::sync::atomic::AtomicUsize,
    base: usize,
    /// Its pages may go to the OS: an anonymous block's (a slab's are the device buffer's, a file's are written back).
    pages: bool,
    ext: Arc<Extent>,
}

/// What is wrong, found under the lock and reported after it is let go (a poisoned ledger would fail every later use).
type Verdict = Result<(), String>;

fn fail(v: Verdict) {
    if let Err(e) = v {
        panic!("block ledger: {e}");
    }
}

const G: usize = 1 << MIN_LOG;

fn bits_range(bits: &[u64], from: usize, to: usize, mut each: impl FnMut(u64, u64) -> bool) -> bool {
    // Calls `each(word, mask)` over the words covering bits `from..to`; stops at the first `false`.
    let mut i = from;
    while i < to {
        let w = i / 64;
        let lo = i % 64;
        let hi = (to - w * 64).min(64);
        let mask = if hi - lo == 64 { !0 } else { ((1u64 << (hi - lo)) - 1) << lo };
        if !each(bits.get(w).copied().unwrap_or(0), mask) {
            return false;
        }
        i = w * 64 + hi;
    }
    true
}

fn any(bits: &[u64], from: usize, to: usize) -> bool {
    !bits_range(bits, from, to, |w, m| w & m == 0)
}

fn all(bits: &[u64], from: usize, to: usize) -> bool {
    bits_range(bits, from, to, |w, m| w & m == m)
}

fn set(bits: &mut Vec<u64>, from: usize, to: usize, on: bool) {
    if to.div_ceil(64) > bits.len() {
        bits.resize(to.div_ceil(64), 0);
    }
    let mut i = from;
    while i < to {
        let w = i / 64;
        let lo = i % 64;
        let hi = (to - w * 64).min(64);
        let mask = if hi - lo == 64 { !0 } else { ((1u64 << (hi - lo)) - 1) << lo };
        if on {
            bits[w] |= mask;
        } else {
            bits[w] &= !mask;
        }
        i = w * 64 + hi;
    }
}

fn bit(bits: &[u64], i: usize) -> bool {
    bits.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1)
}

impl Ledger {
    pub(crate) fn new(base: *mut u8, len: usize, pages: bool, ext: Arc<Extent>) -> Arc<Ledger> {
        let l = Arc::new(Ledger { state: Mutex::default(), chunk_uses: std::sync::atomic::AtomicUsize::new(0), base: base as usize, pages, ext });
        let mut live = blocks();
        let i = live.partition_point(|e| e.0 < l.base);
        live.insert(i, (l.base, l.base + len, Arc::downgrade(&l)));
        l
    }

    /// Its block keeps only its first `len` bytes now (a freeze gave the rest of its reservation back).
    pub(crate) fn resized(&self, len: usize) {
        if let Some(e) = blocks().iter_mut().find(|e| e.0 == self.base) {
            e.1 = self.base + len;
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts tracking its chunks: the block keeps a free list at `heads` now, and `0..top` is its own (the header,
    /// the heads).
    pub(crate) fn track(&self, heads: usize, top: usize) {
        let mut s = self.lock();
        s.heads = heads;
        s.tracked = true;
        set(&mut s.live, 0, top.div_ceil(G), true);
        set(&mut s.starts, 0, 1, true);
    }

    /// The free list is gone (a freeze): chunks still retiring finish with their pages given back, onto no list.
    pub(crate) fn untrack_list(&self) {
        self.lock().heads = 0;
    }

    /// `len` bytes at `at`, a chunk from the free list, are handed out (R1: none of them live).
    pub(crate) fn taken(&self, at: usize, len: usize) {
        self.take(at, len, true);
    }

    /// `len` bytes at `at` from the block's front are handed out: past its top, so none can be live, and of any
    /// alignment (two may share 64 bytes), so not checked.
    pub(crate) fn taken_front(&self, at: usize, len: usize) {
        self.take(at, len, false);
    }

    fn take(&self, at: usize, len: usize, check: bool) {
        let v = {
            let mut s = self.lock();
            if !s.tracked || len == 0 {
                return;
            }
            let (from, to) = (at / G, (at + len).div_ceil(G));
            if check && any(&s.live, from, to) {
                Err(format!("{len} bytes at {at} handed out over a live chunk"))
            } else {
                set(&mut s.live, from, to, true);
                set(&mut s.starts, from, from + 1, true);
                Ok(())
            }
        };
        fail(v);
    }

    /// The tail `at + keep..at + size` of a chunk just handed out is not the holder's (never written).
    pub(crate) fn untaken_tail(&self, at: usize, size: usize, keep: usize) {
        let mut s = self.lock();
        if s.tracked && size > keep {
            set(&mut s.live, (at + keep).div_ceil(G), (at + size).div_ceil(G), false);
        }
    }

    /// The chunk of `len` bytes at `at` is freed (R2: it is a live chunk, of that length). Freed now when no GPU use
    /// of the block is queued, else retiring until those queued now complete (R3). The bytes given to the OS now.
    pub(crate) fn free(&self, at: usize, len: usize, how: Freeing) -> usize {
        let v = {
            let mut s = self.lock();
            if !s.tracked {
                return 0;
            }
            let (from, to) = (at / G, (at + len).div_ceil(G));
            if !bit(&s.starts, from) {
                Err(format!("a free of {len} bytes at {at}, where no live chunk starts (freed twice, or never handed out)"))
            } else if !all(&s.live, from, to) || any(&s.starts, from + 1, to) || (bit(&s.live, to) && !bit(&s.starts, to)) {
                Err(format!("a free of {len} bytes at {at}, not the live chunk's length"))
            } else if s.retiring.iter().any(|r| r.at == at) {
                Err(format!("a second free of the chunk at {at}, retiring"))
            } else if !s.open.is_empty() {
                let epoch = s.epoch;
                s.retiring.push(Retiring { at, len, how, epoch });
                s.epoch += 1;
                return 0;
            } else {
                return self.finish(&mut s, at, len, how);
            }
        };
        fail(v);
        0
    }

    /// A free taking effect: its bits cleared, onto the free list, its pages given back.
    fn finish(&self, s: &mut State, at: usize, len: usize, how: Freeing) -> usize {
        set(&mut s.live, at / G, (at + len).div_ceil(G), false);
        set(&mut s.starts, at / G, at / G + 1, false);
        let base = self.base as *mut u8;
        let mut given = 0;
        let mut piece_at = at;
        let mut left = len;
        while left > 0 {
            let piece = if how.pieces { 1usize << (usize::BITS - 1 - left.leading_zeros()) } else { crate::free::class_of(left).1 };
            if how.pages && self.pages {
                // A piece on the list keeps its link, its first word; with no list, all of it goes.
                let skip = if s.heads != 0 { 8 } else { 0 };
                given += self.give_back_in(s, piece_at + skip, piece - skip);
            }
            if s.heads != 0 {
                let (class, size) = crate::free::class_of(piece);
                // SAFETY: the block's own heads; the piece is free now, nothing reads it, of its class's size.
                unsafe { crate::free::push(base, base.add(s.heads) as *const std::sync::atomic::AtomicU64, class, piece_at) };
                self.ext.free.fetch_add(size, Relaxed);
            }
            piece_at += piece;
            left = left.saturating_sub(piece);
        }
        given
    }

    /// The whole pages inside `len` bytes at `at` to the OS (R6: none holds a live byte). The bytes given.
    pub(crate) fn give_back(&self, at: usize, len: usize) -> usize {
        if !self.pages {
            return 0;
        }
        let mut s = self.lock();
        self.give_back_in(&mut s, at, len)
    }

    fn give_back_in(&self, s: &mut State, at: usize, len: usize) -> usize {
        let p = self.base + at;
        let (start, end) = (p.next_multiple_of(PAGE), (p + len) / PAGE * PAGE);
        if end <= start {
            return 0;
        }
        let (from, to) = ((start - self.base) / G, (end - self.base) / G);
        if s.tracked && any(&s.live, from, to) {
            let off = start - self.base;
            let msg = format!("pages {off}..{} given to the OS hold a live chunk's bytes", end - self.base);
            // The guard poisons as this unwinds; every lock takes a poisoned one as it is (`lock`).
            panic!("block ledger: {msg}");
        }
        // SAFETY: whole pages of the block, none live (above).
        unsafe { crate::mem::give_back(start as *mut u8, end - start) }
    }

    /// The CPU is about to write `len` bytes at `at` (R4: no GPU use of them is queued).
    #[inline]
    pub fn cpu_write(&self, at: usize, len: usize) {
        self.cpu(at, len, true);
    }

    /// The CPU is about to read `len` bytes at `at` (R4: no GPU write of them is queued).
    #[inline]
    pub fn cpu_read(&self, at: usize, len: usize) {
        self.cpu(at, len, false);
    }

    #[inline]
    fn cpu(&self, at: usize, len: usize, write: bool) {
        // Relaxed: a use begins on the thread that encodes it, before its command buffer is committed; one begun by
        // another thread meanwhile is a race the check cannot order.
        if self.chunk_uses.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            return;
        }
        self.cpu_locked(at, len, write);
    }

    #[cold]
    fn cpu_locked(&self, at: usize, len: usize, write: bool) {
        let v = {
            let s = self.lock();
            match s.chunks.iter().find(|u| u.at < at + len && at < u.at + u.len && (write || u.access == Access::Write)) {
                Some(u) => Err(format!(
                    "a CPU {} of {len} bytes at {at}, with a GPU {:?} of {} bytes at {} queued (wait for its fence first)",
                    if write { "write" } else { "read" },
                    u.access,
                    u.len,
                    u.at
                )),
                None => Ok(()),
            }
        };
        fail(v);
    }

    /// Whether no GPU use of the block is queued.
    pub fn quiet(&self) -> bool {
        self.lock().open.is_empty()
    }

    /// Chunks freed but held by queued GPU work.
    pub fn retiring(&self) -> usize {
        self.lock().retiring.len()
    }

    /// A GPU use begins: of the whole block, or of the chunk `len` bytes at `at`. Its id, to end it.
    fn begin(&self, access: Access, chunk: Option<(usize, usize)>) -> (u64, u64) {
        let mut s = self.lock();
        let epoch = s.epoch;
        match s.open.last_mut() {
            Some((e, n)) if *e == epoch => *n += 1,
            _ => s.open.push((epoch, 1)),
        }
        let id = s.next_id;
        s.next_id += 1;
        if let Some((at, len)) = chunk {
            s.chunks.push(ChunkUse { id, at, len, access });
            self.chunk_uses.store(s.chunks.len(), std::sync::atomic::Ordering::Release);
        }
        (id, epoch)
    }

    /// A GPU use ends: the frees it held finish, and the block's memory goes when it was dropped and none is left.
    fn end(&self, id: u64, epoch: u64) {
        let gone = {
            let mut s = self.lock();
            s.chunks.retain(|u| u.id != id);
            self.chunk_uses.store(s.chunks.len(), std::sync::atomic::Ordering::Release);
            if let Some(i) = s.open.iter().position(|(e, _)| *e == epoch) {
                s.open[i].1 -= 1;
                if s.open[i].1 == 0 {
                    s.open.remove(i);
                }
            }
            let oldest = s.open.first().map(|(e, _)| *e);
            if s.gone.is_some() {
                if oldest.is_some() {
                    return;
                }
                // The block is gone: its chunks with it.
                s.retiring.clear();
                s.gone.take()
            } else {
                let ready: Vec<Retiring> = {
                    let (done, wait) = std::mem::take(&mut s.retiring).into_iter().partition(|r| oldest.is_none_or(|o| o > r.epoch));
                    s.retiring = wait;
                    done
                };
                for r in ready {
                    self.finish(&mut s, r.at, r.len, r.how);
                }
                None
            }
        };
        if let Some(release) = gone {
            release();
        }
    }

    /// The block is dropped: `release` lets its memory go, now when no GPU use is queued, else when the last one
    /// completes (R5).
    pub(crate) fn let_go(&self, release: Box<dyn FnOnce() + Send>) {
        let base = self.base;
        let release: Box<dyn FnOnce() + Send> = Box::new(move || {
            // No longer found by its address before another mapping can take it.
            blocks().retain(|e| e.0 != base);
            release()
        });
        let now = {
            let mut s = self.lock();
            if s.open.is_empty() {
                Some(release)
            } else {
                s.gone = Some(release);
                None
            }
        };
        if let Some(r) = now {
            r();
        }
    }
}

/// The live blocks' address ranges, the lowest first, each with its ledger: how a buffer bound for a kernel is found
/// to be a block's (`Fence::uses_address`), whoever made the buffer.
// state: bookkeeping (the blocks in the process, by address: what is mapped, not what a relation has)
static BLOCKS: Mutex<Vec<(usize, usize, std::sync::Weak<Ledger>)>> = Mutex::new(Vec::new());

fn blocks() -> MutexGuard<'static, Vec<(usize, usize, std::sync::Weak<Ledger>)>> {
    BLOCKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The ledger of the live block whose memory holds `addr`.
pub fn of_address(addr: usize) -> Option<Arc<Ledger>> {
    let live = blocks();
    let i = live.partition_point(|e| e.0 <= addr).checked_sub(1)?;
    let (start, end, l) = &live[i];
    (*start <= addr && addr < *end).then(|| l.upgrade()).flatten()
}

/// A command buffer's uses of blocks: registered before its work is encoded, completed by its completion handler
/// (or when dropped: work never committed ran nothing).
pub struct Fence {
    uses: Mutex<Vec<(Arc<Ledger>, u64, u64)>>,
    /// What completing it lets go of, before its blocks' uses end: the pins its work holds (`region`).
    after: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    done: AtomicBool,
    /// Its command buffer is committed: a wait for it ends (`mark_committed`).
    committed: AtomicBool,
    signal: Condvar,
}

impl Fence {
    pub fn new() -> Arc<Fence> {
        Arc::new(Fence { uses: Mutex::new(Vec::new()), after: Mutex::new(Vec::new()), done: AtomicBool::new(false), committed: AtomicBool::new(false), signal: Condvar::new() })
    }

    fn add(&self, ledger: &Arc<Ledger>, access: Access, chunk: Option<(usize, usize)>) {
        let mut uses = self.uses.lock().unwrap_or_else(|e| e.into_inner());
        if self.done.load(Acquire) {
            drop(uses);
            panic!("block ledger: a use registered on a fence already complete");
        }
        let (id, epoch) = ledger.begin(access, chunk);
        uses.push((ledger.clone(), id, epoch));
    }

    /// Its work reads or writes the block whose memory holds `addr` (a buffer bound for a kernel, by its contents),
    /// once however often it is bound. Whether a block holds it: a buffer of no block's is not the ledger's.
    pub fn uses_address(&self, addr: usize) -> bool {
        let Some(l) = of_address(addr) else { return false };
        let known = self.uses.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|u| Arc::ptr_eq(&u.0, &l));
        if !known {
            self.add(&l, Access::Write, None);
        }
        true
    }

    /// Runs `f` when its work completes (a pin its work holds, let go). Fails on a fence already complete.
    pub fn on_complete(&self, f: Box<dyn FnOnce() + Send>) {
        let _uses = self.uses.lock().unwrap_or_else(|e| e.into_inner());
        if self.done.load(Acquire) {
            drop(_uses);
            panic!("block ledger: a pin taken on a fence already complete");
        }
        self.after.lock().unwrap_or_else(|e| e.into_inner()).push(f);
    }

    /// Its work reads `block`, anywhere in it.
    pub fn reads(&self, block: &crate::Block) {
        self.add(block.ledger(), Access::Read, None);
    }

    /// Its work writes `block`, anywhere in it (rows the CPU reserved for it).
    pub fn writes(&self, block: &crate::Block) {
        self.add(block.ledger(), Access::Write, None);
    }

    /// Its work reads the chunk `len` bytes at `at` of `block`: the CPU may not write it until the fence completes.
    pub fn reads_chunk(&self, block: &crate::Block, at: usize, len: usize) {
        self.add(block.ledger(), Access::Read, Some((at, len)));
    }

    /// Its work writes the chunk `len` bytes at `at` of `block`: the CPU may not read or write it until the fence
    /// completes.
    pub fn writes_chunk(&self, block: &crate::Block, at: usize, len: usize) {
        self.add(block.ledger(), Access::Write, Some((at, len)));
    }

    /// Its work completed: every use it holds ends. Idempotent.
    pub fn complete(&self) {
        let uses = {
            let mut uses = self.uses.lock().unwrap_or_else(|e| e.into_inner());
            if self.done.swap(true, AcqRel) {
                return;
            }
            std::mem::take(&mut *uses)
        };
        // Pins first: a region its work held may go now, its chunk freed while the block's use still holds it.
        let after = std::mem::take(&mut *self.after.lock().unwrap_or_else(|e| e.into_inner()));
        for f in after {
            f();
        }
        for (ledger, id, epoch) in uses {
            ledger.end(id, epoch);
        }
        let _g = self.uses.lock().unwrap_or_else(|e| e.into_inner());
        self.signal.notify_all();
    }

    /// Its command buffer was committed: its work will complete, so a wait for it ends.
    pub fn mark_committed(&self) {
        self.committed.store(true, Release);
    }

    /// Whether a wait for it ends: committed, or already complete.
    pub fn is_committed(&self) -> bool {
        self.committed.load(Acquire) || self.is_complete()
    }

    pub fn is_complete(&self) -> bool {
        self.done.load(Acquire)
    }

    /// Waits until its work completes.
    pub fn wait(&self) {
        let mut g = self.uses.lock().unwrap_or_else(|e| e.into_inner());
        while !self.done.load(Acquire) {
            g = self.signal.wait(g).unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Drop for Fence {
    fn drop(&mut self) {
        self.complete();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Backing, Block, kind};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn listed() -> Block {
        Block::anon(kind::FACTS, 8 << 20).with_free_list()
    }

    fn fails(f: impl FnOnce()) -> bool {
        catch_unwind(AssertUnwindSafe(f)).is_err()
    }

    /// R1: nothing is handed out over a live chunk.
    #[test]
    fn allocating_over_a_live_chunk_fails() {
        let b = listed();
        let at = b.try_alloc_chunk(4096, 64).unwrap();
        assert!(fails(|| b.ledger().taken(at + 64, 64)), "inside a live chunk");
        assert!(fails(|| b.ledger().taken(at - 64, 128)), "across a live chunk's start");
        // The ledger is not poisoned by a refusal; small allocations from the front may share 64 bytes.
        assert!(b.try_alloc_chunk(64, 64).is_some());
        let (x, y) = (b.alloc(8, 8), b.alloc(8, 8));
        assert_eq!(y, x + 8);
    }

    /// R2: a free of no live chunk, or twice, fails.
    #[test]
    fn a_second_free_of_a_chunk_fails() {
        let b = listed();
        let at = b.try_alloc_chunk(4096, 64).unwrap();
        b.free_chunk(at, 4096);
        assert!(fails(|| { b.free_chunk(at, 4096); }), "freed twice");
        let at = b.try_alloc_chunk(4096, 64).unwrap();
        assert!(fails(|| { b.free_chunk(at + 64, 64); }), "no chunk starts there");
        // Retiring, a second free fails too.
        let f = Fence::new();
        f.reads(&b);
        b.free_chunk(at, 4096);
        assert!(fails(|| { b.free_chunk(at, 4096); }), "freed twice while retiring");
    }

    /// R2: a free of the wrong length fails.
    #[test]
    fn a_free_of_the_wrong_size_fails() {
        let b = listed();
        let at = b.try_alloc_chunk(4096, 64).unwrap();
        assert!(fails(|| { b.free_chunk(at, 64); }), "shorter than the chunk");
        assert!(fails(|| { b.free_chunk(at, 8192); }), "longer than the chunk");
        b.free_chunk(at, 4096);
        // A fitted chunk: freed by what it kept.
        let (at, kept) = b.try_alloc_chunk_fit(3000, 64).unwrap();
        assert!(fails(|| b.free_chunk_fit(at, kept + 512)));
        b.free_chunk_fit(at, kept);
    }

    /// R3: a chunk freed while a GPU read of its block is queued is not handed out again until the read completes; a
    /// read that began after the free does not hold it.
    #[test]
    fn a_chunk_freed_under_a_queued_gpu_read_waits_for_it() {
        let b = listed();
        let at = b.try_alloc_chunk(4096, 64).unwrap();
        let before = Fence::new();
        before.reads(&b);
        b.free_chunk(at, 4096);
        assert_eq!(b.ledger().retiring(), 1);
        let other = b.try_alloc_chunk(4096, 64).unwrap();
        assert_ne!(other, at, "a retiring chunk is not handed out");
        let after = Fence::new();
        after.reads(&b);
        before.complete();
        assert_eq!(b.ledger().retiring(), 0, "the read after the free does not hold it");
        assert_eq!(b.try_alloc_chunk(4096, 64), Some(at), "free once the read before it completed");
        drop(after);
        assert!(b.ledger().quiet());
    }

    /// R4: the CPU may not write a chunk a queued GPU use reads or writes, nor read one it writes; once the fence
    /// completes it may.
    #[test]
    fn a_cpu_write_under_a_queued_gpu_use_fails() {
        let b = listed();
        let at = b.try_alloc_chunk(4096, 64).unwrap();
        let next = b.try_alloc_chunk(4096, 64).unwrap();
        let r = Fence::new();
        r.reads_chunk(&b, at, 4096);
        assert!(fails(|| b.ledger().cpu_write(at + 100, 8)));
        b.ledger().cpu_read(at, 4096);
        b.ledger().cpu_write(next, 4096);
        let w = Fence::new();
        w.writes_chunk(&b, next, 4096);
        assert!(fails(|| b.ledger().cpu_read(next, 8)));
        r.complete();
        w.complete();
        b.ledger().cpu_write(at, 4096);
        b.ledger().cpu_read(next, 4096);
        // A use on a complete fence fails.
        assert!(fails(|| r.reads(&b)));
    }

    struct Watched {
        base: *mut u8,
        len: usize,
        gone: Arc<AtomicBool>,
    }
    unsafe impl Send for Watched {}
    unsafe impl Sync for Watched {}
    impl Backing for Watched {
        fn base(&self) -> *mut u8 {
            self.base
        }
        fn len(&self) -> usize {
            self.len
        }
    }
    impl Drop for Watched {
        fn drop(&mut self) {
            unsafe { crate::mem::release(self.base, self.len) };
            self.gone.store(true, SeqCst);
        }
    }
    fn watched() -> (Box<Watched>, Arc<AtomicBool>) {
        let gone = Arc::new(AtomicBool::new(false));
        let len = 4 * PAGE;
        let base = crate::mem::reserve(len);
        // SAFETY: the reservation just made: a slab is memory throughout (committed on Windows).
        unsafe { crate::mem::commit(base, len) };
        (Box::new(Watched { base, len, gone: gone.clone() }), gone)
    }

    /// R5: a block dropped while a GPU use of it is queued keeps its memory until the use completes.
    #[test]
    fn a_block_dropped_under_a_queued_gpu_use_keeps_its_memory() {
        let (slab, gone) = watched();
        let b = Block::over(kind::FACTS, slab);
        let f = Fence::new();
        f.reads(&b);
        drop(b);
        assert!(!gone.load(SeqCst), "the slab is held while the GPU may read it");
        f.complete();
        assert!(gone.load(SeqCst), "and let go once it completes");
        // Quiet, a block's memory goes with it.
        let (slab, gone) = watched();
        drop(Block::over(kind::FACTS, slab));
        assert!(gone.load(SeqCst));
    }

    /// R6: no page that holds a live byte goes to the OS.
    #[test]
    fn a_page_with_a_live_byte_is_never_given_back() {
        let b = listed();
        let small = b.try_alloc_chunk(64, 64).unwrap();
        let page = small / PAGE * PAGE;
        assert!(fails(|| { b.ledger().give_back(page, 2 * PAGE); }), "the page holds a live chunk");
        // A retiring chunk's pages wait too, and its neighbours' pages are not given back when it finishes.
        let big = b.try_alloc_chunk(2 * PAGE, 64).unwrap();
        let after = b.try_alloc_chunk(64, 64).unwrap();
        unsafe { *b.ptr::<u64>(after) = 9 };
        let f = Fence::new();
        f.reads(&b);
        b.free_chunk(big, 2 * PAGE);
        assert!(fails(|| { b.ledger().give_back(big, 2 * PAGE); }), "a retiring chunk is live");
        f.complete();
        assert_eq!(unsafe { *b.ptr::<u64>(after) }, 9);
        // macOS's `mincore` tells a page given back by its modified bit; elsewhere the kept value says it.
        #[cfg(target_vendor = "apple")]
        assert!(crate::mem::tests::modified(b.ptr::<u8>(after)), "its neighbour's page kept");
    }

    /// A buffer bound for a kernel is found to be its block's by its address, and holds the block's memory as any
    /// use does; once the block's memory is gone, its address is no block's.
    #[test]
    fn a_bound_buffers_block_is_found_by_its_address() {
        let (slab, gone) = watched();
        let b = Block::over(kind::FACTS, slab);
        let addr = b.base_ptr() as usize + 100;
        let f = Fence::new();
        assert!(f.uses_address(addr) && f.uses_address(addr + 64), "inside the block");
        assert!(!f.uses_address(addr + 8 * PAGE), "past it");
        drop(b);
        assert!(!gone.load(SeqCst));
        drop(f);
        assert!(gone.load(SeqCst));
        assert!(of_address(addr).is_none(), "gone, its address is no block's");
    }

    /// R7: a block is not moved while a GPU use of it is queued.
    #[test]
    fn moving_a_block_under_a_queued_gpu_use_fails() {
        let b = Block::anon(kind::FACTS, 1 << 20);
        let f = Fence::new();
        f.reads(&b);
        let (slab, _) = watched();
        assert!(fails(move || drop(b.relocate(slab))));
        f.complete();
        let b = Block::anon(kind::FACTS, 1 << 20);
        let (slab, _) = watched();
        drop(b.relocate(slab));
    }
}
