//! A stack a thread (decision 0342, amended 2026-11-24): the memory of a thread's short-lived structures and of its
//! scratch uses (a freeze's or a pack's temporaries), one anonymous block reserved once a thread.
//!
//! - **A region** is a 16-byte header (the previous header's offset, a state word) and its bytes, bumped from the
//!   top. Its going sets the state free, one atomic store, from any thread. The owning thread pops freed headers off
//!   the top (`settle`) at its next take, or at the going itself when it is the owner's, and moves the top back. No
//!   region or take is zeroed (`Room`): its holder writes each byte before it reads it.
//! - **A use** (`with`) takes slices tagged with the use; its end frees them, and it may nest. A slice is a region:
//!   a freeze's temporaries and the room's structures are one memory.
//! - **The room** (`StackRoom`) gives a structure a region of the calling thread's stack. Where a region lies is a
//!   block and an offset (`maps::place_of`), so a kernel reads a structure there in place.
//! - **Pages** past `KEEP` bytes above the top go back to the OS, in steps of `GIVE_STEP`, and a take that reaches
//!   them marks them in use again.
//! - **Freed regions under a live one** (a small array that outlives what was made after it): once `STRAND` bytes
//!   were freed and not popped, the owner's settle links each live region past the freed ones under it (one gap) and
//!   gives the gap's whole pages back (a hole). A pop past a hole leaves it above the top, as pages past `KEEP` are.
//!   In the census of a run, the stacks held 342 MB of freed regions under 7 MB of live ones at their most.
//! - **GPU work** (decision 0487): a kernel that binds the stack's bytes holds the block's ledger; while one is
//!   queued nothing is popped, so no take writes bytes a kernel reads. The pops wait for its fence.
//!
//! A region that outlives what was made after it holds the stack under it until it goes: what lives long belongs in
//! its owner's room, not here (the census of 0342's amendment finds such regions: `RERAIL_ANON_CENSUS`).

use crate::Pod;
use crate::block::Block;
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::atomic::{AtomicU64, AtomicUsize};

/// Address space a thread's stack reserves (nothing resident until written; on Windows nothing committed until
/// touched, `crate::windows`). Without a mapping (`crate::mem`) the reservation is allocated whole, so it is a working
/// size there. 4 GiB: a stack's bytes under its top stay resident, so a thread past that holds as much memory; and a
/// thread's 64 GiB reservation cost the kernel's map, with every thread making its own at once, about a fortieth of
/// an edit's CPU.
#[cfg(any(unix, windows))]
const ANON: usize = 1 << 32;
#[cfg(not(any(unix, windows)))]
const ANON: usize = 256 << 20;
/// The least bytes of a large region: it goes to the thread's free-listed blocks (`maps::FreeRoom::large`), not the stack.
/// A large array is freed out of the stack's order (a writer's parts, a tape) and would hold every region under it; in
/// the census of an edit, regions of a page and more are about 2% of them, and kept off the stack they leave its
/// footprint at about a third of what it is with them (decision 0342, amended 2026-11-24).
pub const LARGE: usize = 4096;
/// Bytes above the top kept resident; past them, freed pages go back to the OS.
const KEEP: usize = 1 << 20;
/// The fewest bytes past `KEEP` given back at once, so a top that swings round it does not advise at each pop.
const GIVE_STEP: usize = 1 << 20;
/// A region's header: the previous header's offset (0: none) with, above `PREV_BITS`, the padding between the top
/// before it and itself; and its state.
const HEAD: usize = 16;
const PREV_BITS: u32 = 32;
const _: () = assert!(ANON <= 1 << PREV_BITS, "a header's offsets fit its bits");
/// The bytes freed under a live region, and not popped, past which the owner's settle coalesces them (`coalesce`).
const STRAND: usize = 8 << 20;
/// The least gap whose pages go back (`coalesce`): smaller gaps are linked past, and keep theirs.
const HOLE_MIN: usize = 256 << 10;
/// A region's state: live, freed, or (even, from 2) a live slice of the use with that tag.
const LIVE: u64 = 0;
const FREE: u64 = 1;

#[repr(C)]
struct Head {
    prev: AtomicU64,
    state: AtomicU64,
}

/// A thread's stack. Its top and chain are its owner's to move; any thread frees a region of it.
pub struct Stack {
    block: Arc<Block>,
    base: *mut u8,
    len: usize,
    /// The owning thread (`thread_id`); 0, a stack no thread owns (made while its thread went), never popped.
    owner: u64,
    /// The first byte past the block's header.
    floor: usize,
    top: AtomicUsize,
    /// The topmost header's offset; 0, none.
    last: AtomicUsize,
    /// The highest byte written since pages last went back.
    touched: AtomicUsize,
    /// Pages given back, from and to: marked in use again when a take reaches them.
    given: [AtomicUsize; 2],
    /// Bytes freed and not yet popped (any thread adds; the owner's pops and coalescing take off).
    stranded: AtomicUsize,
    /// Gaps under live regions whose pages went back (`coalesce`), from and to, lowest first; the bytes given; the
    /// owner's.
    holes: std::sync::Mutex<(Vec<(usize, usize)>, usize)>,
    /// Uses begun, for their tags.
    uses: AtomicU64,
}

// SAFETY: its top and chain are moved by its owner only (`owner`); other threads store a header's state, an atomic.
unsafe impl Send for Stack {}
unsafe impl Sync for Stack {}

fn thread_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    thread_local! { static ID: u64 = NEXT.fetch_add(1, Relaxed); }
    ID.try_with(|i| *i).unwrap_or(0)
}

impl Stack {
    fn new(reserved: usize, owner: u64) -> Stack {
        let block = Arc::new(Block::anon(crate::block::kind::SCRATCH, reserved));
        let (base, len, floor) = (block.base_ptr(), block.reserved(), block.used());
        Stack { block, base, len, owner, floor, top: AtomicUsize::new(floor), last: AtomicUsize::new(0), touched: AtomicUsize::new(floor), given: [AtomicUsize::new(0), AtomicUsize::new(0)], stranded: AtomicUsize::new(0), holes: std::sync::Mutex::new((Vec::new(), 0)), uses: AtomicU64::new(0) }
    }

    fn head(&self, at: usize) -> &Head {
        // SAFETY: a header this stack wrote at `at`, 16-byte aligned, inside its reservation.
        unsafe { &*self.base.add(at).cast::<Head>() }
    }

    /// The block it lies in.
    pub fn block(&self) -> &Arc<Block> {
        &self.block
    }

    /// The bytes from its first region to its top.
    pub fn used(&self) -> usize {
        self.top.load(Relaxed) - self.floor
    }

    /// A region of `bytes` at a multiple of `align`, in `state`: its header's offset and its first byte's. The owner's.
    fn push(&self, bytes: usize, align: usize, state: u64) -> (usize, usize) {
        self.settle();
        let top = self.top.load(Relaxed);
        let at = (top + HEAD).next_multiple_of(align.max(HEAD));
        let head = at - HEAD;
        let end = (at + bytes.max(1)).next_multiple_of(HEAD);
        assert!(end <= self.len, "a thread's stack is full: {end} of {} bytes", self.len);
        let [from, to] = [self.given[0].load(Relaxed), self.given[1].load(Relaxed)];
        if to > from && end > from {
            // SAFETY: the stack's own pages, given back by `settle`.
            unsafe { crate::mem::reuse_pages(self.base.add(from), to - from) };
            self.given[0].store(0, Relaxed);
            self.given[1].store(0, Relaxed);
        }
        let h = self.head(head);
        h.prev.store(self.last.load(Relaxed) as u64 | ((head - top) as u64) << PREV_BITS, Relaxed);
        h.state.store(state, Relaxed);
        self.last.store(head, Relaxed);
        USED.fetch_add(end - top, Relaxed);
        self.top.store(end, Relaxed);
        let touched = self.touched.load(Relaxed);
        if end > touched {
            self.touched.store(end, Relaxed);
            RESIDENT.fetch_add(end - touched, Relaxed);
        }
        // The device wraps a block's used bytes: they reach every region handed out.
        self.block.reach(end);
        (head, at)
    }

    /// The region of `bytes` whose header lies at `head` goes: from any thread; popped now when this is its owner.
    fn free(&self, head: usize, bytes: usize) {
        self.stranded.fetch_add(bytes, Relaxed);
        self.head(head).state.store(FREE, Release);
        if self.owner != 0 && self.owner == thread_id() {
            self.settle();
        }
    }

    /// Pops the freed regions off the top, and gives back the pages past `KEEP` above it. The owner's; nothing while
    /// GPU work that binds the block is queued.
    fn settle(&self) {
        let mut last = self.last.load(Relaxed);
        if last == 0 || !(self.head(last).state.load(Acquire) == FREE || self.stranded.load(Relaxed) >= STRAND) || !self.block.ledger().quiet() {
            return;
        }
        if self.head(last).state.load(Acquire) != FREE {
            self.coalesce();
            return;
        }
        let mut top = self.top.load(Relaxed);
        while last != 0 {
            let h = self.head(last);
            if h.state.load(Acquire) != FREE {
                break;
            }
            let prev = h.prev.load(Relaxed);
            top = last - (prev >> PREV_BITS) as usize;
            last = (prev & ((1 << PREV_BITS) - 1)) as usize;
        }
        self.last.store(last, Relaxed);
        let popped = self.top.load(Relaxed) - top;
        USED.fetch_sub(popped, Relaxed);
        self.top.store(top, Relaxed);
        let stranded = self.stranded.load(Relaxed);
        self.stranded.store(if last == 0 { 0 } else { stranded.saturating_sub(popped) }, Relaxed);
        self.uncover(top);
        let keep = (top + KEEP).next_multiple_of(crate::block::PAGE);
        let touched = self.touched.load(Relaxed);
        if touched >= keep + GIVE_STEP {
            // SAFETY: the stack's own pages, above its top: no region reaches them.
            unsafe { crate::mem::free_pages(self.base.add(keep), touched - keep) };
            let [from, to] = [self.given[0].load(Relaxed), self.given[1].load(Relaxed)];
            let (from, to) = if to > from { (from.min(keep), to.max(touched)) } else { (keep, touched) };
            self.given[0].store(from, Relaxed);
            self.given[1].store(to, Relaxed);
            self.touched.store(keep, Relaxed);
            RESIDENT.fetch_sub(touched - keep, Relaxed);
        }
        if last != 0 && self.stranded.load(Relaxed) >= STRAND {
            self.coalesce();
        }
    }

    /// Links each live region past the freed ones under it, into one gap the pop jumps (its header's padding), and
    /// gives back the whole pages of gaps of `HOLE_MIN` and more. The owner's, with no GPU work queued: a freed
    /// region's bytes may be bound by a kernel still to run.
    fn coalesce(&self) {
        const MASK: u64 = (1 << PREV_BITS) - 1;
        let mut holes = self.holes.lock().unwrap_or_else(|e| e.into_inner());
        let mut above = self.last.load(Relaxed);
        let mut found = Vec::new();
        while above != 0 {
            let h = self.head(above);
            let prev = h.prev.load(Relaxed);
            let mut below = (prev & MASK) as usize;
            if below == 0 || self.head(below).state.load(Acquire) != FREE {
                above = below;
                continue;
            }
            // The freed run down to the next live region (or the floor): the gap from its end to `above`'s header.
            let mut low = above - (prev >> PREV_BITS) as usize;
            while below != 0 && self.head(below).state.load(Acquire) == FREE {
                let p = self.head(below).prev.load(Relaxed);
                low = below - (p >> PREV_BITS) as usize;
                below = (p & MASK) as usize;
            }
            h.prev.store(below as u64 | ((above - low) as u64) << PREV_BITS, Relaxed);
            if above - low >= HOLE_MIN {
                found.push((low, above));
            }
            above = below;
        }
        let page = crate::block::PAGE;
        let mut given = 0;
        for &(from, to) in found.iter().rev() {
            let (a, b) = (from.next_multiple_of(page), to / page * page);
            if b > a {
                // SAFETY: the stack's own pages, every region on them freed and unlinked: no region reaches them.
                unsafe { crate::mem::free_pages(self.base.add(a), b - a) };
                // A gap reaching down past a live region freed since takes in the holes under that one.
                let mut inside = 0;
                holes.0.retain(|&(x, y)| {
                    let within = a <= x && y <= b;
                    inside += if within { y - x } else { 0 };
                    !within
                });
                holes.0.push((a, b));
                given += b - a - inside;
            }
        }
        holes.0.sort_unstable();
        holes.1 += given;
        RESIDENT.fetch_sub(given, Relaxed);
        self.stranded.store(0, Relaxed);
    }

    /// The holes a pop to `top` left above it join the pages given back above the top (`given`), marked in use again
    /// when a take reaches them; their bytes count as touched again, as the pages past `KEEP` the next settle gives.
    fn uncover(&self, top: usize) {
        let mut holes = self.holes.lock().unwrap_or_else(|e| e.into_inner());
        let Some(at) = holes.0.iter().position(|&(_, to)| to > top) else { return };
        let (from, to) = (holes.0[at].0, holes.0.last().map_or(0, |h| h.1));
        let back: usize = holes.0.drain(at..).map(|(a, b)| b - a).sum();
        holes.1 -= back;
        RESIDENT.fetch_add(back, Relaxed);
        let [f, t] = [self.given[0].load(Relaxed), self.given[1].load(Relaxed)];
        let (f, t) = if t > f { (f.min(from), t.max(to)) } else { (from, to) };
        self.given[0].store(f, Relaxed);
        self.given[1].store(t, Relaxed);
    }
}

/// The bytes every thread's stack has touched above its floor and not given back: what the stacks hold in memory,
/// for the profile's account of a footprint (their blocks are no allocator's, so no other count holds them).
static RESIDENT: AtomicUsize = AtomicUsize::new(0);

/// What the threads' stacks hold in memory now (`RESIDENT`).
pub fn resident() -> usize {
    RESIDENT.load(Relaxed)
}

/// The bytes every thread's stack holds under its top: its live regions, and the freed ones under a live one (what
/// a stack holds beside them is pages above its top, `KEEP` and those a settle has not given back).
static USED: AtomicUsize = AtomicUsize::new(0);

/// What the threads' stacks hold under their tops now (`USED`).
pub fn used() -> usize {
    USED.load(Relaxed)
}

impl Drop for Stack {
    fn drop(&mut self) {
        let holes = self.holes.get_mut().unwrap_or_else(|e| e.into_inner()).1;
        RESIDENT.fetch_sub(self.touched.load(Relaxed) - self.floor - holes, Relaxed);
        USED.fetch_sub(self.top.load(Relaxed) - self.floor, Relaxed);
    }
}

thread_local! {
    static THREAD: std::cell::OnceCell<Arc<Stack>> = const { std::cell::OnceCell::new() };
}

/// This thread's stack, made at its first use; while the thread goes, a stack of `bytes` that no thread owns.
fn mine(bytes: usize) -> Arc<Stack> {
    THREAD
        .try_with(|t| t.get_or_init(|| Arc::new(Stack::new(ANON, thread_id()))).clone())
        .unwrap_or_else(|_| Arc::new(Stack::new(bytes + crate::block::PAGE * 2, 0)))
}

/// What holds a region of a stack (`Region`'s owner): its header popped once freed.
pub struct OnStack {
    stack: Arc<Stack>,
    head: usize,
    at: usize,
    bytes: usize,
    _census: crate::maps::Keep,
}

impl OnStack {
    /// Its block and its first byte's offset there.
    pub fn place(&self) -> (&Arc<Block>, usize) {
        (&self.stack.block, self.at)
    }
}

impl Drop for OnStack {
    fn drop(&mut self) {
        self.stack.free(self.head, self.bytes);
    }
}

/// A large region (`LARGE` and more): the calling thread's free-listed blocks' (`maps::FreeRoom::large`), as the
/// stack's and the long room's large regions are.
pub(crate) fn large_region(bytes: usize, align: usize, name: &'static str) -> crate::maps::Region {
    use crate::maps::Room;
    thread_local! { static LARGE_ROOM: crate::maps::FreeRoom = crate::maps::FreeRoom::large(); }
    LARGE_ROOM.try_with(|r| r.region(bytes, align, name)).unwrap_or_else(|_| crate::maps::FreeRoom::large().region(bytes, align, name))
}

/// The room of a thread's stack: a region of the calling thread's (`Room`; the run's default, decision 0342 amended).
#[derive(Default)]
pub struct StackRoom;

impl crate::maps::Room for StackRoom {
    fn region(&self, bytes: usize, align: usize, name: &'static str) -> crate::maps::Region {
        if bytes >= LARGE {
            return large_region(bytes, align, name);
        }
        let stack = mine(bytes + align);
        let (head, at) = stack.push(bytes, align, LIVE);
        // SAFETY: the stack's bytes past the header just pushed, this region's alone until it goes. Not zeroed: its
        // holder writes each byte before it reads it (`Room`).
        let ptr = unsafe { stack.base.add(at) };
        let census = crate::census::counted(crate::census::STACK, name, bytes);
        // SAFETY: `bytes` writable bytes at `ptr`, aligned as asked, held by `OnStack` while the region lives.
        unsafe { crate::maps::Region::new(ptr, bytes, Box::new(OnStack { stack, head, at, bytes, _census: census })) }
    }
}

/// A use of a thread's stack (`with`): slices taken there, freed at its end.
pub struct Scratch {
    stack: Arc<Stack>,
    tag: u64,
    taken: Cell<usize>,
    bytes: Cell<usize>,
}

impl Scratch {
    /// `n` values of `T`, their contents whatever was there.
    #[allow(clippy::mut_from_ref)]
    pub fn take<T: Pod + Copy>(&self, n: usize) -> &mut [T] {
        let (_, at) = self.stack.push(n * size_of::<T>(), align_of::<T>().max(64), self.tag);
        self.taken.set(self.taken.get() + 1);
        self.bytes.set(self.bytes.get() + n * size_of::<T>());
        // SAFETY: the stack's bytes past the header just pushed, this use's alone until it ends (which outlives the
        // borrow of `self`); every bit pattern is a value of a `Pod` type.
        unsafe { std::slice::from_raw_parts_mut(self.stack.base.add(at).cast::<T>(), n) }
    }

    /// `n` zero values of `T`.
    #[allow(clippy::mut_from_ref)]
    pub fn zeroed<T: Pod + Copy>(&self, n: usize) -> &mut [T] {
        let s = self.take::<T>(n);
        // SAFETY: the take's own bytes; zero is a value of every `Pod` type here.
        unsafe { std::ptr::write_bytes(s.as_mut_ptr(), 0, n) };
        s
    }

    /// The stack it takes from.
    pub fn stack(&self) -> &Arc<Stack> {
        &self.stack
    }
}

impl Drop for Scratch {
    /// Its slices go: each found down the chain (they are all on it, live), freed, then popped.
    fn drop(&mut self) {
        let mut left = self.taken.get();
        self.stack.stranded.fetch_add(self.bytes.get(), Relaxed);
        let mut at = self.stack.last.load(Relaxed);
        while left > 0 && at != 0 {
            let h = self.stack.head(at);
            if h.state.load(Relaxed) == self.tag {
                h.state.store(FREE, Release);
                left -= 1;
            }
            at = (h.prev.load(Relaxed) & ((1 << PREV_BITS) - 1)) as usize;
        }
        if self.stack.owner != 0 && self.stack.owner == thread_id() {
            self.stack.settle();
        }
    }
}

/// Runs `f` with a use of this thread's stack: its takes go when it ends. Uses nest.
pub fn with<R>(f: impl FnOnce(&Scratch) -> R) -> R {
    let stack = mine(0);
    let tag = (stack.uses.fetch_add(1, Relaxed) + 1) << 1;
    let s = Scratch { stack, tag, taken: Cell::new(0), bytes: Cell::new(0) };
    f(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::Fence;
    use crate::maps::Room;

    fn stack() -> Arc<Stack> {
        mine(0)
    }

    /// A use's takes go back at its end, and a nested use's at its own.
    #[test]
    fn a_use_takes_and_its_end_gives_back() {
        let s = stack();
        let before = s.used();
        with(|a| {
            let x = a.zeroed::<u64>(1000);
            assert!(x.iter().all(|v| *v == 0));
            x[999] = 7;
            let mid = s.used();
            with(|b| {
                b.take::<u32>(5000)[0] = 1;
                assert!(s.used() > mid);
            });
            assert_eq!(s.used(), mid, "the nested use's slice is popped at its end");
            assert_eq!(x[999], 7, "and the outer use's is untouched");
        });
        assert_eq!(s.used(), before, "every take went back");
    }

    /// A room's region freed under a live one waits; once the live one goes, both are popped.
    #[test]
    fn a_region_outliving_those_after_it_holds_the_stack_under_it() {
        let s = stack();
        let before = s.used();
        let a = StackRoom.region(1024, 8, "a");
        let b = StackRoom.region(64, 8, "b");
        let after_b = s.used();
        drop(a);
        assert_eq!(s.used(), after_b, "a is freed but b above it is live: nothing is popped");
        let c = StackRoom.region(64, 8, "c");
        assert!(s.used() > after_b, "a take goes above b");
        drop(c);
        assert_eq!(s.used(), after_b);
        drop(b);
        assert_eq!(s.used(), before, "b gone: b and a are popped");
    }

    /// A region's place is its stack's block and its offset there.
    #[test]
    fn a_region_lies_in_its_stacks_block() {
        let mut r = StackRoom.region(256, 64, "placed");
        let p = r.as_mut_ptr() as usize;
        assert_eq!(p % 64, 0, "aligned as asked");
        let (block, at) = crate::maps::place_of(&r).expect("a stack's region lies in a block");
        assert_eq!(block.base_ptr() as usize + at, p);
    }

    /// A region of `LARGE` bytes or more goes to the thread's free-listed blocks, not the stack.
    #[test]
    fn a_large_region_is_not_on_the_stack() {
        let s = stack();
        let before = s.used();
        let r = StackRoom.region(LARGE, 8, "large");
        assert_eq!(s.used(), before, "nothing taken on the stack");
        let (block, _) = crate::maps::place_of(&r).expect("a large region lies in a block");
        assert!(!Arc::ptr_eq(block, s.block()), "in a block of its own room");
        assert!(block.has_free_list());
        let small = StackRoom.region(LARGE - 1, 8, "small");
        assert!(s.used() > before, "one byte less is the stack's");
        drop((r, small));
        assert_eq!(s.used(), before);
    }

    /// The block's used bytes reach every region of its stack: the device wraps them (`Block::device_buffer`), so a
    /// region past them would be read outside the wrap.
    #[test]
    fn the_blocks_used_bytes_reach_every_region() {
        let s = stack();
        with(|u| {
            u.take::<u8>(3 << 20);
            let r = StackRoom.region(1024, 8, "high");
            let (block, at) = crate::maps::place_of(&r).unwrap();
            assert!(block.used() >= at + 1024, "the region lies within the used bytes");
            assert!(Arc::ptr_eq(block, s.block()));
        });
    }

    /// A region freed on another thread is popped by its owner at its next take.
    #[test]
    fn a_free_from_another_thread_is_settled_by_the_owner() {
        let s = stack();
        let before = s.used();
        let r = StackRoom.region(2048, 8, "sent");
        assert!(s.used() > before);
        std::thread::spawn(move || drop(r)).join().unwrap();
        assert!(s.used() > before, "its owner has not taken since");
        let t = StackRoom.region(8, 8, "next");
        drop(t);
        assert_eq!(s.used(), before, "the owner popped it at its next take");
    }

    /// Pages more than `KEEP` above the top go back once popped, and are taken (and marked in use) again.
    #[test]
    fn pages_past_keep_go_back_and_are_taken_again() {
        let s = stack();
        let big = KEEP + 4 * GIVE_STEP;
        with(|u| {
            let x = u.take::<u8>(big);
            x[big - 1] = 1;
        });
        let [from, to] = [s.given[0].load(Relaxed), s.given[1].load(Relaxed)];
        assert!(to > from, "pages past KEEP went back");
        with(|u| {
            let x = u.zeroed::<u8>(big);
            x[big - 1] = 2;
        });
    }

    /// Regions freed under a live one: past `STRAND` bytes they are linked past as one gap and its pages go back (a
    /// hole); once the live one goes, all are popped, and the hole's pages are taken (and marked in use) again.
    #[test]
    fn freed_regions_under_a_live_one_give_their_pages_back() {
        let s = stack();
        let before = s.used();
        let (n, bytes) = (STRAND / 2000 + 64, 2000);
        let mut under: Vec<_> = (0..n).map(|_| StackRoom.region(bytes, 8, "under")).collect();
        // SAFETY (each fill): the region's own bytes.
        let fill = |r: &mut crate::maps::Region, v: u8| unsafe { std::ptr::write_bytes(r.as_mut_ptr(), v, bytes) };
        for r in &mut under {
            fill(r, 1);
        }
        let live = StackRoom.region(64, 8, "live");
        let used = s.used();
        let resident = crate::mem::resident(s.block().base_ptr() as usize, s.block().reserved());
        drop(under);
        assert_eq!(s.used(), used, "the live region holds the stack under it");
        let holes = s.holes.lock().unwrap().1;
        assert!(holes >= n * bytes - 2 * HOLE_MIN, "the freed regions' pages went back: {holes} of {} bytes", n * bytes);
        // On macOS reusable pages leave memory at once; elsewhere `MADV_FREE` leaves them until the OS wants them.
        let after = crate::mem::resident(s.block().base_ptr() as usize, s.block().reserved());
        #[cfg(target_vendor = "apple")]
        assert!(after + holes <= resident + crate::block::PAGE, "in memory {after}, was {resident}");
        let _ = (after, resident);
        let next = StackRoom.region(256, 8, "next");
        drop((next, live));
        assert_eq!(s.used(), before, "the live one gone: every region popped, the gap jumped");
        assert!(s.holes.lock().unwrap().0.is_empty(), "the hole is above the top");
        let mut again: Vec<_> = (0..n).map(|_| StackRoom.region(bytes, 8, "again")).collect();
        for r in &mut again {
            fill(r, 3);
        }
        drop(again);
        assert_eq!(s.used(), before);
    }

    /// Decision 0487 at a stack: while a kernel's bind of its bytes is queued, nothing is popped (the next take would
    /// write what the kernel reads); once the work completes, the next pop catches up.
    #[test]
    fn nothing_is_popped_under_queued_gpu_work() {
        let s = stack();
        let before = s.used();
        let f = Fence::new();
        with(|u| {
            let at = u.take::<u64>(512).as_ptr() as usize;
            assert!(f.uses_address(at), "its bytes are found by address");
        });
        assert!(s.used() > before, "the use's slice stays while the work is queued");
        f.complete();
        drop(StackRoom.region(8, 8, "after"));
        assert_eq!(s.used(), before, "and is popped once it completes");
    }
}
