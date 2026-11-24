//! A thread's long-lived room (decision 0342, amended 2026-11-24): where a structure known to outlive the work it is
//! made in lies, beside the thread's stack (`scratch`), which holds the rest.
//!
//! - **Runs of one size class.** A region under `scratch::LARGE` takes a slot of the least class that holds it
//!   (`CLASSES`: four a doubling, so a region wastes an eighth on average), in a run of `RUN` bytes of that class's
//!   slots. Larger regions go to the thread's large room, as the stack's do.
//! - **A freed slot** goes on its run's list (its first 8 bytes the next's place), and the class's next region takes
//!   it. A run whose slots are all free gives its pages back and is any class's next run.
//! - **Any thread frees a region of it**: the room's one mutex, uncontended but for those.
//! - **One block a thread**, address space reserved once (pages touched as runs are taken).

use crate::block::Block;
use std::sync::{Arc, Mutex};

/// Address space a thread's long room reserves (nothing resident until written).
#[cfg(any(unix, windows))]
const RESERVE: usize = 1 << 36;
#[cfg(not(any(unix, windows)))]
const RESERVE: usize = 256 << 20;
/// A run's bytes: four pages, every slot of one class.
const RUN: usize = 64 << 10;
/// The size classes: 16 bytes apart to 128, then four a doubling to `scratch::LARGE`.
pub const CLASSES: [u32; 28] = [
    16, 32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 896, 1024, 1280, 1536, 1792, 2048,
    2560, 3072, 3584, 4096,
];
const _: () = assert!(CLASSES[CLASSES.len() - 1] as usize == crate::scratch::LARGE);
/// No class: a run free for any.
const NONE: u8 = u8::MAX;

/// The class of a region of `bytes` at a multiple of `align`: the least that holds it whose slots, `RUN`-aligned runs
/// apart, are all at a multiple of `align`. None past the last.
fn class_of(bytes: usize, align: usize) -> Option<usize> {
    let align = align.max(8);
    CLASSES.iter().position(|&c| c as usize >= bytes.max(8) && c as usize % align == 0)
}

#[derive(Clone, Copy)]
struct Run {
    class: u8,
    /// Whether it is on its class's list of runs with a free slot.
    listed: bool,
    /// Whether its pages went back (`mem::give_back`): marked in use again when it is taken.
    given: bool,
    live: u32,
    /// The slots ever handed out: the next is taken from here once the list is empty.
    bump: u32,
    /// The first free slot's number plus one (0: none); each free slot's first 8 bytes the next's.
    free: u32,
}

struct State {
    runs: Vec<Run>,
    /// By class, the runs that may have a free slot (a full one is dropped when it is reached).
    partial: Vec<Vec<u32>>,
    /// Runs no class holds.
    spare: Vec<u32>,
}

/// The room's block and its runs: what a region holds to be freed from any thread.
pub struct Long {
    block: Arc<Block>,
    base: *mut u8,
    /// The first run's offset in the block (past its header).
    floor: usize,
    state: Mutex<State>,
}

// SAFETY: `base` is the block's, which the room holds; its runs' bookkeeping is under the mutex.
unsafe impl Send for Long {}
unsafe impl Sync for Long {}

impl Long {
    fn new(reserved: usize) -> Long {
        let block = Arc::new(Block::anon(crate::block::kind::SCRATCH, reserved));
        let base = block.base_ptr();
        let floor = block.used().next_multiple_of(RUN);
        Long { block, base, floor, state: Mutex::new(State { runs: Vec::new(), partial: vec![Vec::new(); CLASSES.len()], spare: Vec::new() }) }
    }

    fn run_at(&self, run: u32) -> usize {
        self.floor + run as usize * RUN
    }

    /// A slot of `class`: its offset in the block.
    fn take(&self, class: usize) -> usize {
        let size = CLASSES[class] as usize;
        let slots = (RUN / size) as u32;
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let Some(&r) = s.partial[class].last() else { break };
            let run = &mut s.runs[r as usize];
            if run.free != 0 {
                let slot = run.free - 1;
                let at = self.run_at(r) + slot as usize * size;
                // SAFETY: a free slot of this run: its first 8 bytes the next free slot's number plus one.
                run.free = unsafe { self.base.add(at).cast::<u32>().read() };
                run.live += 1;
                return at;
            }
            if run.bump < slots {
                let at = self.run_at(r) + run.bump as usize * size;
                run.bump += 1;
                run.live += 1;
                return at;
            }
            run.listed = false;
            s.partial[class].pop();
        }
        // A new run: a spare one, else the next the block has.
        let r = match s.spare.pop() {
            Some(r) => r,
            None => {
                let r = s.runs.len() as u32;
                assert!(self.run_at(r) + RUN <= self.block.reserved(), "a thread's long room is full: {} runs", r);
                s.runs.push(Run { class: NONE, listed: false, given: false, live: 0, bump: 0, free: 0 });
                r
            }
        };
        let at = self.run_at(r);
        if s.runs[r as usize].given {
            // SAFETY: the run's own pages, given back when it was last emptied.
            unsafe { crate::mem::reuse_pages(self.base.add(at), RUN) };
        }
        s.runs[r as usize] = Run { class: class as u8, listed: true, given: false, live: 1, bump: 1, free: 0 };
        s.partial[class].push(r);
        // The device wraps a block's used bytes: they reach every region handed out.
        self.block.reach(at + RUN);
        at
    }

    /// Frees the slot at `at`.
    fn give(&self, at: usize) {
        let r = ((at - self.floor) / RUN) as u32;
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let run = s.runs[r as usize];
        let class = run.class as usize;
        let size = CLASSES[class] as usize;
        let slot = ((at - self.run_at(r)) / size) as u32;
        if run.live == 1 {
            // Its last slot: the run goes back whole, its pages to the OS while no GPU work binds the block.
            let given = self.block.ledger().quiet();
            if given {
                // SAFETY: the run's own pages: no region of it is live.
                unsafe { crate::mem::give_back(self.base.add(self.run_at(r)), RUN) };
            }
            if run.listed {
                s.partial[class].retain(|&x| x != r);
            }
            s.runs[r as usize] = Run { class: NONE, listed: false, given, live: 0, bump: 0, free: 0 };
            s.spare.push(r);
            return;
        }
        // SAFETY: the slot is free now: its first 8 bytes hold the list's next.
        unsafe { self.base.add(at).cast::<u32>().write(run.free) };
        let run = &mut s.runs[r as usize];
        run.free = slot + 1;
        run.live -= 1;
        if !run.listed {
            run.listed = true;
            s.partial[class].push(r);
        }
    }
}

/// What holds a region of a long room (`Region`'s owner): its slot freed once it goes.
pub struct InLong {
    long: Arc<Long>,
    at: usize,
    _census: crate::maps::Keep,
}

impl InLong {
    /// Its block and its first byte's offset there.
    pub fn place(&self) -> (&Arc<Block>, usize) {
        (&self.long.block, self.at)
    }
}

impl Drop for InLong {
    fn drop(&mut self) {
        self.long.give(self.at);
    }
}

/// A thread's long-lived room (`Room`): its regions in its runs, large ones in the thread's large room.
pub struct LongRoom(Arc<Long>);

impl LongRoom {
    pub fn new() -> LongRoom {
        LongRoom(Arc::new(Long::new(RESERVE)))
    }
}

impl Default for LongRoom {
    fn default() -> Self {
        LongRoom::new()
    }
}

impl crate::maps::Room for LongRoom {
    fn region(&self, bytes: usize, align: usize, name: &'static str) -> crate::maps::Region {
        let Some(class) = class_of(bytes, align) else {
            return crate::scratch::large_region(bytes, align, name);
        };
        let at = self.0.take(class);
        // SAFETY: the slot's bytes, this region's alone until it goes.
        let ptr = unsafe { self.0.base.add(at) };
        // Zeroed, as every room's region reads (a freed slot holds its list's next).
        // SAFETY: the region's own bytes, within its slot.
        unsafe { std::ptr::write_bytes(ptr, 0, bytes.max(8)) };
        let census = crate::census::counted(crate::census::LONG, name, bytes);
        // SAFETY: `bytes` writable bytes at `ptr`, aligned as asked (the class is a multiple of `align`, a run at a
        // multiple of `RUN`), held by `InLong` while the region lives.
        unsafe { crate::maps::Region::new(ptr, bytes, Box::new(InLong { long: Arc::clone(&self.0), at, _census: census })) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maps::Room;

    fn room() -> LongRoom {
        LongRoom(Arc::new(Long::new(64 << 20)))
    }

    /// A region lies in its room's block, reads zero, and a freed slot is taken again by its class.
    #[test]
    fn a_freed_slot_is_taken_again_by_its_class_and_reads_zero() {
        let r = room();
        let mut a = r.region(40, 8, "a");
        let (block, at) = crate::maps::place_of(&a).expect("in a block");
        assert!(Arc::ptr_eq(block, &r.0.block));
        // SAFETY: the region's own 40 bytes.
        unsafe { std::ptr::write_bytes(a.as_mut_ptr(), 7, 40) };
        drop(a);
        let b = r.region(48, 16, "b");
        assert_eq!(crate::maps::place_of(&b).unwrap().1, at, "the class's freed slot is taken first");
        // SAFETY: the region's own 48 bytes.
        assert!(unsafe { std::slice::from_raw_parts(b.as_ptr(), 48) }.iter().all(|&x| x == 0), "a region reads zero");
    }

    /// A run whose slots are all freed goes back, and another class takes it.
    #[test]
    fn an_emptied_run_is_taken_by_another_class() {
        let r = room();
        let n = RUN / 64;
        let all: Vec<_> = (0..n).map(|_| r.region(64, 8, "x")).collect();
        let first = crate::maps::place_of(&all[0]).unwrap().1;
        drop(all);
        let y = r.region(3000, 8, "y");
        assert_eq!(crate::maps::place_of(&y).unwrap().1, first, "the emptied run is the next class's");
        assert!(r.0.state.lock().unwrap().runs.len() == 1, "no second run was taken");
    }

    /// A region freed on another thread goes back to its room, and the room takes it again.
    #[test]
    fn a_region_freed_on_another_thread_goes_back_to_its_room() {
        let r = room();
        let a = r.region(100, 8, "a");
        let at = crate::maps::place_of(&a).unwrap().1;
        std::thread::spawn(move || drop(a)).join().unwrap();
        let b = r.region(100, 8, "b");
        assert_eq!(crate::maps::place_of(&b).unwrap().1, at);
    }

    /// A region of `scratch::LARGE` and more is the thread's large room's, not a slot.
    #[test]
    fn a_large_region_goes_to_the_large_room() {
        let r = room();
        let a = r.region(crate::scratch::LARGE + 1, 8, "a");
        assert!(!Arc::ptr_eq(crate::maps::place_of(&a).unwrap().0, &r.0.block));
    }

    /// Each class's slots lie at a multiple of the alignment asked.
    #[test]
    fn slots_are_aligned_as_asked() {
        for align in [8, 16, 32, 64, 128] {
            let c = class_of(40, align).unwrap();
            assert_eq!(CLASSES[c] as usize % align, 0);
        }
    }
}
