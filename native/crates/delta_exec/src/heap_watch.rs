//! The twins' heap watch (*0686: Operations* §2: an operation is a function of its blocks' bytes, run on a CPU or a
//! GPU over the same blocks): in the crate's tests, every heap allocation made while a twin's phase runs (an item, a
//! count, an `again`), its own code's or any blocks code it calls, fails the run, with where it was made. A device has
//! no heap: what a phase holds is in its slots or in fixed working memory. Host versions (`Impl::Host`: the file
//! layer, the parser) are not watched.
//!
//! `RERAIL_HEAP_AUDIT=1` writes each allocation's size and place to stderr instead of failing (a line each, distinct
//! ones once a run, `HEAP_AUDIT` first; run the tests with `--nocapture`): a list of every place, from one run.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};

// No twin takes the heap: every one that allocated is gone from the list that named them (*Flat Views and
// Lifecycles* step 5: an out block keeps no free list, so its claims take no host bookkeeping, and its writers
// claim exactly, writing their own state). An allocation in any twin's phase fails the run.

struct Watch;

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static SEEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Records the allocation's size and place when the watch is on (off while it records, as recording allocates).
fn note(size: usize) {
    if ON.try_with(|on| on.replace(false)).unwrap_or(false) {
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let mut frames = vec![format!("{size} bytes")];
        let mut lines = bt.lines().peekable();
        while let Some(l) = lines.next() {
            let sym = l.trim_start().split_once(": ").map_or("", |(_, s)| s);
            if sym.contains("rerail_") && !sym.contains("heap_watch") {
                let at = lines.peek().map_or("", |n| n.trim()).to_string();
                frames.push(format!("{sym} {at}"));
                if frames.len() == 7 {
                    break;
                }
            }
        }
        let place = frames.join("\n  ");
        let _ = SEEN.try_with(|s| {
            let mut s = s.borrow_mut();
            if !s.contains(&place) {
                s.push(place);
            }
        });
        let _ = ON.try_with(|on| on.set(true));
    }
}

unsafe impl GlobalAlloc for Watch {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        note(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        note(l.size());
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        note(n);
        unsafe { System.realloc(p, l, n) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static WATCH: Watch = Watch;

/// `f`, watched when `on`: an allocation in it fails the run (or, under `RERAIL_HEAP_AUDIT`, is written down).
pub fn during<R>(on: bool, what: &str, f: impl FnOnce() -> R) -> R {
    if !on {
        return f();
    }
    let was = ON.with(|o| o.replace(true));
    let r = f();
    ON.with(|o| o.set(was));
    if was {
        return r;
    }
    let seen = SEEN.with(|s| std::mem::take(&mut *s.borrow_mut()));
    if !seen.is_empty() {
        match std::env::var_os("RERAIL_HEAP_AUDIT") {
            Some(_) => {
                use std::io::Write;
                // One write a run, so parallel tests' records do not interleave.
                let test = std::thread::current().name().unwrap_or("?").to_string();
                let text: String = seen.iter().map(|s| format!("HEAP_AUDIT {what}: {} (in {test})\n", s.replace('\n', " | "))).collect();
                std::io::stderr().lock().write_all(text.as_bytes()).expect("stderr");
            }
            None => panic!("{what}: a twin's phase allocated on the heap ({} places):\n{}", seen.len(), seen.join("\n---\n")),
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use rerail_blocks::flat::{Arena, Bucket, Chain, Grow, Map, Pool, Region, Stack, Stamped};

    /// *Flat Views and Lifecycles* step 3: the working forms, built, grown, read and emptied in an arena over a
    /// buffer made before the watch, take nothing from the heap.
    #[test]
    fn the_working_forms_take_no_heap() {
        let mut bytes = vec![0u64; 1 << 17];
        let bytes = unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr().cast::<u8>(), 1 << 20) };
        let total = super::during(true, "flat working forms", || {
            let mut a = Arena::init(Region::of(bytes), Bucket::Launch).unwrap();
            let keep = a.mark();
            let mut sum = 0u64;
            {
                let g = Grow::<u64>::make(&a, 1, 0).unwrap();
                for i in 0..5000 {
                    g.push(&a, i).unwrap();
                }
                sum += g.as_slice().iter().sum::<u64>();
                let p = Pool::<u32>::make(&a, 2, 100).unwrap();
                let (i, room) = p.take().unwrap();
                room.write(9);
                sum += *p.get(i) as u64;
                p.give(i);
                let c = Chain::<u32>::make(&a, 3, 100).unwrap();
                let l = c.cons(0, 1).unwrap();
                let l = c.cons(l, 2).unwrap();
                sum += c.read().walk(l).map(|&v| v as u64).sum::<u64>();
                let m = Map::<u32, u32>::make(&a, 4, 1000, u32::MAX, u32::MAX).unwrap();
                for k in 0..1000 {
                    m.claim_min(k % 300, k).unwrap();
                }
                m.remove(7);
                sum += m.iter().map(|(_, v)| v as u64).sum::<u64>();
                let t = Stamped::<u32, u32>::make(&a, 5, 100).unwrap();
                let c = t.claim(&a, 3).unwrap();
                t.set_at(c.slot, 4);
                sum += t.get(&a, 3).unwrap() as u64;
                let spill = Grow::<u32>::make(&a, 6, 0).unwrap();
                let mut s: Stack<'_, '_, u32, 8> = Stack::new(spill, &a);
                for i in 0..100 {
                    s.push(i).unwrap();
                }
                while let Some(v) = s.pop() {
                    sum += v as u64;
                }
            }
            a.next_round(keep);
            sum
        });
        assert_eq!(total, (0..5000).sum::<u64>() + 9 + 3 + ((0..300).sum::<u64>() - 7) + 4 + (0..100).sum::<u64>());
    }
}
