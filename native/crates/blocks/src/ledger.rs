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
    /// The pages it leaves with no live byte go to the OS (an anonymous block's, when the owner asks).
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
    /// It keeps free lists (false: a bump block, or a frozen one).
    listed: bool,
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
    /// The free chunks by class (decision 0342, amended by 0494 step 6): here, not in the chunks, so a free chunk's
    /// pages hold nothing and go to the OS once no live chunk shares them. Each chunk is aligned to its size, and
    /// merged with its buddy when both are free; the lowest first is taken.
    lists: Vec<std::collections::BTreeSet<usize>>,
    /// A bit a page: given to the OS, and marked in use again when a chunk handed out reaches it.
    given: Vec<u64>,
}

/// A block's ledger. The block holds it, and so does every fence with a use of the block, so its memory outlives the
/// GPU's work.
pub struct Ledger {
    state: Mutex<State>,
    /// GPU uses of chunks queued now (`State::chunks`' length): with none, a CPU access is checked without the lock
    /// (a block's arrays are read and written at every value, `cpu_read`, `cpu_write`).
    chunk_uses: std::sync::atomic::AtomicUsize,
    /// The block's own region while a handle holds it (`region::whole`, decision 0491 step 3): what a kernel's bind
    /// of the block, found by address, pins (`region::bind_address`).
    pub(crate) root: Mutex<std::sync::Weak<crate::region::Region>>,
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
        let l = Arc::new(Ledger { state: Mutex::default(), chunk_uses: std::sync::atomic::AtomicUsize::new(0), root: Mutex::new(std::sync::Weak::new()), base: base as usize, pages, ext });
        let mut live = blocks();
        // After any at the same address: memory an outer ledger holds (a pooled lease) made a block's (`Block::over`)
        // or `Tracked`, the inner ledger is the one its address finds (`of_address`).
        let i = live.partition_point(|e| e.0 <= l.base);
        live.insert(i, (l.base, l.base + len, Arc::downgrade(&l)));
        l
    }

    /// Its block keeps only its first `len` bytes now (a freeze gave the rest of its reservation back).
    pub(crate) fn resized(&self, len: usize) {
        if let Some(e) = blocks().iter_mut().find(|e| std::ptr::eq(e.2.as_ptr(), self)) {
            e.1 = self.base + len;
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts tracking its chunks: the block keeps free lists now, and `0..top` is its own (its header).
    pub(crate) fn track(&self, top: usize) {
        let mut s = self.lock();
        s.listed = true;
        s.tracked = true;
        set(&mut s.live, 0, top.div_ceil(G), true);
        set(&mut s.starts, 0, 1, true);
    }

    /// The free lists are gone (a freeze): the pages their chunks leave with no live byte go to the OS, and chunks
    /// still retiring finish with theirs given back, onto no list.
    pub(crate) fn untrack_list(&self) {
        let mut s = self.lock();
        s.listed = false;
        let lists = std::mem::take(&mut s.lists);
        if self.pages {
            for (class, l) in lists.iter().enumerate() {
                for &at in l {
                    self.give_free_pages(&mut s, at, 1 << (MIN_LOG as usize + class));
                }
            }
        }
    }

    /// `len` bytes at `at` from the block's front are handed out: past its top, so none can be live, and of any
    /// alignment (two may share 64 bytes), so not checked.
    pub(crate) fn taken_front(&self, at: usize, len: usize) {
        self.take(at, len, false);
    }

    fn take(&self, at: usize, len: usize, check: bool) {
        let v = {
            let mut s = self.lock();
            self.take_in(&mut s, at, len, check)
        };
        fail(v);
    }

    fn take_in(&self, s: &mut State, at: usize, len: usize, check: bool) -> Verdict {
        if !s.tracked || len == 0 {
            return Ok(());
        }
        let (from, to) = (at / G, (at + len).div_ceil(G));
        if check && any(&s.live, from, to) {
            return Err(format!("{len} bytes at {at} handed out over a live chunk"));
        }
        set(&mut s.live, from, to, true);
        set(&mut s.starts, from, from + 1, true);
        self.reuse_in(s, at, len);
        Ok(())
    }

    /// A chunk of `class` from the lists, handed out (`taken`): its class's lowest, else the lowest of the smallest
    /// larger class that has one, split down (each upper half onto its list). Its offset.
    pub(crate) fn pop(&self, class: usize) -> Option<usize> {
        let size = 1usize << (MIN_LOG as usize + class);
        let (at, v) = {
            let mut s = self.lock();
            let from = (class..s.lists.len()).find(|&c| !s.lists[c].is_empty())?;
            let at = s.lists[from].pop_first()?;
            for c in (class..from).rev() {
                s.lists[c].insert(at + (1usize << (MIN_LOG as usize + c)));
            }
            self.ext.free.fetch_sub(size, Relaxed);
            (at, self.take_in(&mut s, at, size, true))
        };
        fail(v);
        Some(at)
    }

    /// A chunk of `size` bytes (a class's, aligned to it), never live (the rest of a fitted chunk, or the bytes
    /// skipped to align one), onto its list; the pages it leaves with no live byte go to the OS.
    pub(crate) fn push_never_live(&self, at: usize, size: usize) {
        let mut s = self.lock();
        if !s.listed {
            return;
        }
        let (class, size) = crate::free::class_of(size);
        Self::put(&mut s, at, class);
        self.ext.free.fetch_add(size, Relaxed);
        if self.pages {
            self.give_free_pages(&mut s, at, size);
        }
    }

    /// A free chunk of `class` at `at` (aligned to its size, or to `ALIGN_MAX`) onto its list, merged with its buddy
    /// while the buddy is free.
    fn put(s: &mut State, mut at: usize, mut class: usize) {
        loop {
            if s.lists.len() <= class + 1 {
                s.lists.resize_with(class + 2, Default::default);
            }
            let size = 1usize << (MIN_LOG as usize + class);
            assert!(at.is_multiple_of(size.min(crate::free::ALIGN_MAX)), "block ledger: a free chunk of {size} bytes at {at}, misaligned");
            // One not aligned to its size (a large one, at `ALIGN_MAX`) has no buddy.
            if at.is_multiple_of(size) && class + 1 < crate::free::CLASSES && s.lists[class].remove(&(at ^ size)) {
                at &= !size;
                class += 1;
                continue;
            }
            s.lists[class].insert(at);
            return;
        }
    }

    /// The pages `len` bytes at `at` reach that were given to the OS, marked in use again.
    fn reuse_in(&self, s: &mut State, at: usize, len: usize) {
        if !self.pages || s.given.is_empty() {
            return;
        }
        let (p0, p1) = (at / PAGE, (at + len).div_ceil(PAGE));
        let mut p = p0;
        while p < p1 {
            if !bit(&s.given, p) {
                p += 1;
                continue;
            }
            let q0 = p;
            while p < p1 && bit(&s.given, p) {
                p += 1;
            }
            set(&mut s.given, q0, p, false);
            // SAFETY: pages of the block given back earlier; their chunk is the taker's now.
            unsafe { crate::mem::reuse_pages((self.base + q0 * PAGE) as *mut u8, (p - q0) * PAGE) };
        }
    }

    /// The pages `len` bytes at `at` reach that hold no live byte now and were not given yet, to the OS (R6: whole
    /// pages, none live). The bytes given.
    fn give_free_pages(&self, s: &mut State, at: usize, len: usize) -> usize {
        const PER: usize = PAGE / G;
        let (p0, p1) = (at / PAGE, (at + len).div_ceil(PAGE));
        let mut given = 0;
        let mut p = p0;
        while p < p1 {
            let free = |s: &State, p: usize| !bit(&s.given, p) && !any(&s.live, p * PER, (p + 1) * PER);
            if !free(s, p) {
                p += 1;
                continue;
            }
            let q0 = p;
            while p < p1 && free(s, p) {
                p += 1;
            }
            set(&mut s.given, q0, p, true);
            // SAFETY: whole pages of the block, none live (above).
            given += unsafe { crate::mem::give_back((self.base + q0 * PAGE) as *mut u8, (p - q0) * PAGE) };
        }
        given
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
        let mut given = 0;
        let mut piece_at = at;
        let mut left = len;
        while left > 0 {
            // A chunk of its class whole, a fitted one in its pieces (`free::piece`).
            let piece = crate::free::piece(piece_at, left);
            if s.listed {
                let (class, size) = crate::free::class_of(piece);
                Self::put(s, piece_at, class);
                self.ext.free.fetch_add(size, Relaxed);
            }
            piece_at += piece;
            left = left.saturating_sub(piece);
        }
        if how.pages && self.pages {
            // A free chunk holds no link: every page it reaches with no live byte left goes.
            given += self.give_free_pages(s, at, len);
        }
        given
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
        let me = std::ptr::from_ref(self) as usize;
        let release: Box<dyn FnOnce() + Send> = Box::new(move || {
            // No longer found by its address before another mapping can take it; an outer ledger of the same memory
            // (a lease's) stays.
            blocks().retain(|e| e.2.as_ptr() as usize != me);
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

/// A value that owns memory a kernel may bind and that is no block's (a device pool's lease): held as a block's memory
/// is, by a ledger found at its address. A command buffer that binds its bytes holds it (`Fence::uses_address`), and
/// when dropped it goes (`T`'s drop: back to its pool) only once that work completes (R5): the next holder's host
/// writes are never ordered before a queued kernel's reads.
pub struct Tracked<T: Send + 'static> {
    value: Option<T>,
    ledger: Arc<Ledger>,
}

impl<T: Send + 'static> Tracked<T> {
    /// `value`, whose memory is the `len` bytes at `base`: found by any address in them while it lives.
    pub fn new(value: T, base: *mut u8, len: usize) -> Tracked<T> {
        let ext = Arc::new(crate::block::Extent { top: std::sync::atomic::AtomicUsize::new(0), free: std::sync::atomic::AtomicUsize::new(0) });
        Tracked { value: Some(value), ledger: Ledger::new(base, len.max(1), false, ext) }
    }

    /// Its ledger: what a fence's use of its bytes holds.
    pub fn ledger(&self) -> &Arc<Ledger> {
        &self.ledger
    }
}

impl<T: Send + 'static> std::ops::Deref for Tracked<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value.as_ref().expect("a tracked value lives until dropped")
    }
}

impl<T: Send + 'static> std::ops::DerefMut for Tracked<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("a tracked value lives until dropped")
    }
}

impl<T: Send + 'static> Drop for Tracked<T> {
    fn drop(&mut self) {
        let value = self.value.take();
        self.ledger.let_go(Box::new(move || drop(value)));
    }
}

/// The live blocks' address ranges, the lowest first, each with its ledger: how a buffer bound for a kernel is found
/// to be a block's (`Fence::uses_address`), whoever made the buffer.
// state: bookkeeping (the blocks in the process, by address: what is mapped, not what a relation has)
static BLOCKS: Mutex<Vec<(usize, usize, std::sync::Weak<Ledger>)>> = Mutex::new(Vec::new());

fn blocks() -> MutexGuard<'static, Vec<(usize, usize, std::sync::Weak<Ledger>)>> {
    BLOCKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The ledger of the live block whose memory holds `addr`: the innermost, where one ledger's memory lies in
/// another's (a block over a pooled lease: the block's, registered after the lease's).
pub fn of_address(addr: usize) -> Option<Arc<Ledger>> {
    let live = blocks();
    let i = live.partition_point(|e| e.0 <= addr);
    live[..i].iter().rev().filter(|(start, end, _)| *start <= addr && addr < *end).find_map(|(_, _, l)| l.upgrade())
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
    /// The regions its binds by address pinned (`region::bind_address`), by address and access: each once.
    pinned: Mutex<Vec<(usize, bool)>>,
    /// The addresses its binds by address held already (`region::bind_address`), each with the ledger it found, or
    /// none (a buffer of no block's): a program's stack binds the same buffers at each of its thousands of
    /// dispatches (decision 0641).
    bound: Mutex<std::collections::HashMap<usize, Option<Arc<Ledger>>>>,
    signal: Condvar,
}

impl Fence {
    pub fn new() -> Arc<Fence> {
        Arc::new(Fence { uses: Mutex::new(Vec::new()), after: Mutex::new(Vec::new()), done: AtomicBool::new(false), committed: AtomicBool::new(false), pinned: Mutex::new(Vec::new()), bound: Mutex::new(std::collections::HashMap::new()), signal: Condvar::new() })
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
        self.held_at(addr).is_some()
    }

    /// The ledger `addr` lies in, its block's use held by it (once however often it is bound). Found once a fence
    /// and address: it holds the ledger until it completes, so nothing frees what lies there and puts another block
    /// there before. An address no ledger held at its first bind is none of a ledger's at the next: the buffer
    /// bound there, held by its submission, keeps the memory from any block for the fence's life.
    pub(crate) fn held_at(&self, addr: usize) -> Option<Arc<Ledger>> {
        if let Some(l) = self.bound.lock().unwrap_or_else(|e| e.into_inner()).get(&addr) {
            return l.clone();
        }
        let Some(l) = of_address(addr) else {
            self.bound.lock().unwrap_or_else(|e| e.into_inner()).insert(addr, None);
            return None;
        };
        let known = self.uses.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|u| Arc::ptr_eq(&u.0, &l));
        if !known {
            self.add(&l, Access::Write, None);
        }
        self.bound.lock().unwrap_or_else(|e| e.into_inner()).insert(addr, Some(Arc::clone(&l)));
        Some(l)
    }

    /// Whether `key` (a region, with its access) is pinned on it for the first time now.
    pub(crate) fn first_pin(&self, key: usize, write: bool) -> bool {
        let mut p = self.pinned.lock().unwrap_or_else(|e| e.into_inner());
        if p.contains(&(key, write)) {
            return false;
        }
        p.push((key, write));
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
        self.bound.lock().unwrap_or_else(|e| e.into_inner()).clear();
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

/// A block's ledger as a region's guard (`rerail_vector::Region::guarded`): a vector in the block has each CPU read and
/// write checked against the device's queued uses (R4).
impl rerail_vector::Guard for Ledger {
    #[inline]
    fn cpu_read(&self, at: usize, len: usize) {
        Ledger::cpu_read(self, at, len)
    }
    #[inline]
    fn cpu_write(&self, at: usize, len: usize) {
        Ledger::cpu_write(self, at, len)
    }
    fn uses(&self) -> Option<&std::sync::atomic::AtomicUsize> {
        Some(&self.chunk_uses)
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
        assert!(fails(|| b.ledger().take(at + 64, 64, true)), "inside a live chunk");
        assert!(fails(|| b.ledger().take(at - 64, 128, true)), "across a live chunk's start");
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

    /// R4 through a vector in the block (`Region::guarded`): read freely while nothing is queued on the block
    /// (`Region::quiet` skips the guard), refused while a device write of its chunk is queued, read again once that
    /// fence completes.
    #[test]
    fn a_vector_in_a_block_is_checked_while_a_use_of_it_is_queued() {
        struct InBlock(Arc<Block>, std::sync::atomic::AtomicUsize);
        impl crate::maps::Room for InBlock {
            fn region(&self, bytes: usize, align: usize, name: &'static str) -> crate::maps::Region {
                let (at, kept) = self.0.try_alloc_chunk_fit(bytes, align).expect("room in the block");
                self.1.store(at, Relaxed);
                // SAFETY: the chunk was taken for this region, `kept` bytes at `at`.
                let m = unsafe { crate::region::Mutable::<u8>::adopt(&self.0, at, bytes, kept, name) };
                crate::maps::region_of(m, None).guarded(self.0.ledger().clone(), at)
            }
        }
        let b = Arc::new(listed());
        let room = Arc::new(InBlock(b.clone(), std::sync::atomic::AtomicUsize::new(0)));
        let mut v = crate::vector::Vector::<u64>::new_in(room.clone());
        v.extend_from_slice(&[1, 2, 3]);
        assert_eq!(v.iter().sum::<u64>(), 6, "nothing queued: read");
        let other = b.try_alloc_chunk(4096, 64).unwrap();
        let w = Fence::new();
        w.writes_chunk(&b, other, 4096);
        assert_eq!(v.iter().sum::<u64>(), 6, "a use of another chunk queued: read");
        let mine = Fence::new();
        mine.writes_chunk(&b, room.1.load(Relaxed), 64);
        assert!(fails(|| drop(v.iter().sum::<u64>())), "its own chunk written by the device: refused");
        mine.complete();
        w.complete();
        v.push(4);
        assert_eq!(v.iter().sum::<u64>(), 10, "the fences complete: read and written");
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
        (Box::new(Watched { base: crate::mem::reserve(len), len, gone: gone.clone() }), gone)
    }

    /// R5 for memory of no block's (a pool's lease, `Tracked`): a fence that binds its bytes, found by address, holds it;
    /// dropped meanwhile, it goes only when the fence completes. Unbound, it goes at once.
    #[test]
    fn a_tracked_lease_bound_by_address_goes_after_its_fence() {
        struct Lease(Arc<AtomicBool>);
        impl Drop for Lease {
            fn drop(&mut self) {
                self.0.store(true, SeqCst);
            }
        }
        let mem = vec![0u8; 4096];
        let base = mem.as_ptr().cast_mut();
        let gone = Arc::new(AtomicBool::new(false));
        let t = Tracked::new(Lease(gone.clone()), base, 4096);
        let f = Fence::new();
        assert!(f.uses_address(base as usize + 100), "found by an address inside it");
        drop(t);
        assert!(!gone.load(SeqCst), "held by the queued work");
        f.complete();
        assert!(gone.load(SeqCst), "gone once its work completes");
        assert!(!Fence::new().uses_address(base as usize + 100), "no longer found once gone");
        let gone = Arc::new(AtomicBool::new(false));
        drop(Tracked::new(Lease(gone.clone()), base, 4096));
        assert!(gone.load(SeqCst), "unbound: at once");
        drop(mem);
    }

    /// R9: memory one ledger holds made another's (a pooled lease made a block's, `Block::over`): its address finds the
    /// inner ledger while it lives, a bind holds the inner, and the inner gone, the outer is found again. Each goes
    /// only once the work that bound it completes, the outer after the inner.
    #[test]
    fn a_ledger_inside_another_is_the_one_its_address_finds() {
        struct Lease(Arc<AtomicBool>);
        impl Drop for Lease {
            fn drop(&mut self) {
                self.0.store(true, SeqCst);
            }
        }
        let mem = vec![0u8; 8192];
        let base = mem.as_ptr().cast_mut();
        let (outer_gone, inner_gone) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let outer = Tracked::new(Lease(outer_gone.clone()), base, 8192);
        let inner = Tracked::new(Lease(inner_gone.clone()), base, 8192);
        assert!(Arc::ptr_eq(&of_address(base as usize + 4100).expect("found"), inner.ledger()), "the inner ledger is found");
        let f = Fence::new();
        assert!(f.uses_address(base as usize + 4100));
        assert!(!inner.ledger().quiet() && outer.ledger().quiet(), "the bind holds the inner");
        drop(inner);
        assert!(!inner_gone.load(SeqCst), "held by the queued work");
        f.complete();
        assert!(inner_gone.load(SeqCst));
        assert!(Arc::ptr_eq(&of_address(base as usize + 4100).expect("found"), outer.ledger()), "the outer stays, found again");
        drop(outer);
        assert!(outer_gone.load(SeqCst) && of_address(base as usize).is_none());
        drop(mem);
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

    /// R6: no page that holds a live byte goes to the OS: a chunk freed beside a live one in its page leaves the page,
    /// and a retiring chunk's pages wait for its GPU work.
    #[test]
    fn a_page_with_a_live_byte_is_never_given_back() {
        let b = listed();
        let small = b.try_alloc_chunk(64, 64).unwrap();
        let beside = b.try_alloc_chunk(4096, 64).unwrap();
        assert_eq!(small / PAGE, beside / PAGE, "one page");
        unsafe { *b.ptr::<u64>(small) = 7 };
        b.free_chunk(beside, 4096);
        assert_eq!(unsafe { *b.ptr::<u64>(small) }, 7);
        #[cfg(target_vendor = "apple")]
        assert!(crate::mem::tests::modified(b.ptr::<u8>(small)), "the live chunk's page kept");
        // A retiring chunk's pages wait too, and its neighbours' pages are not given back when it finishes.
        let big = b.try_alloc_chunk(2 * PAGE, 64).unwrap();
        let after = b.try_alloc_chunk(64, 64).unwrap();
        unsafe { *b.ptr::<u64>(after) = 9 };
        unsafe { std::ptr::write_bytes(b.ptr::<u8>(big), 0xab, 2 * PAGE) };
        let f = Fence::new();
        f.reads(&b);
        b.free_chunk(big, 2 * PAGE);
        #[cfg(target_vendor = "apple")]
        assert!(crate::mem::tests::modified(b.ptr::<u8>(big)), "a retiring chunk's pages wait");
        f.complete();
        #[cfg(target_vendor = "apple")]
        assert!(!crate::mem::tests::modified(b.ptr::<u8>(big)), "and go once its GPU work completes");
        assert_eq!(unsafe { *b.ptr::<u64>(after) }, 9);
        // The page's modified bit is macOS's `mincore` alone; elsewhere the byte read back says it.
        #[cfg(target_vendor = "apple")]
        assert!(crate::mem::tests::modified(b.ptr::<u8>(after)), "its neighbour's page kept");
    }

    /// A buffer bound for a kernel is found to be its block's by its address, and holds the block's memory as any
    /// use does; once the block's memory is gone, its address is no block's.
    #[test]
    fn a_bound_buffers_block_is_found_by_its_address() {
        // The slab is the first 4 pages of 16 reserved here: the pages past it are no other test's block.
        let gone = Arc::new(AtomicBool::new(false));
        let range = crate::mem::reserve(16 * PAGE);
        let slab = Box::new(Watched { base: range, len: 4 * PAGE, gone: gone.clone() });
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
        // SAFETY: the reservation's pages past the slab's, which the slab's drop left.
        unsafe { crate::mem::release(range.add(4 * PAGE), 12 * PAGE) };
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
