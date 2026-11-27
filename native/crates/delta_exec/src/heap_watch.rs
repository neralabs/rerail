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

/// The twins that still take the heap, and through what: this list only shrinks. A twin not on it that allocates
/// fails the run. A phase that takes room in its out block takes it through the block's allocator, whose bookkeeping
/// (`ledger::Ledger`: free lists, live bits, under a lock) and bump top (`block::Extent`) are host memory, not the
/// block's bytes; a write pin's access is checked there too. A layer's statistics are built in a heap `Sketch`.
const NOT_YET: &[(&str, &str)] = &[
    ("consolidate", "the block allocator's host bookkeeping; the layer's sketch"),
    ("merge", "the block allocator's host bookkeeping; the layer's sketch"),
    ("square", "a write pin's access, checked in the block's ledger"),
    ("append", "the block allocator's host bookkeeping"),
];

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
                let text: String = seen.iter().map(|s| format!("HEAP_AUDIT {what}: {}\n", s.replace('\n', " | "))).collect();
                std::io::stderr().lock().write_all(text.as_bytes()).expect("stderr");
            }
            None if NOT_YET.iter().any(|(op, _)| what.split(' ').next() == Some(op)) => {}
            None => panic!("{what}: a twin's phase allocated on the heap ({} places):\n{}", seen.len(), seen.join("\n---\n")),
        }
    }
    r
}
