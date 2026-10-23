//! The daemon's memory as its two allocators see it: the Rust heap (jemalloc, which tree-sitter
//! shares) and the C heap (the system malloc zones, where the bundled SQLite allocates its page
//! caches, statements and schema). RSS above both is allocator retention, code and stacks.
use parking_lot::Mutex;
use serde_json::{Value as Json, json};
use std::sync::{Arc, mpsc};
use std::time::Instant;

/// Live bytes: (Rust heap, C heap).
pub fn heaps() -> (u64, u64) {
    (crate::repo::heap_in_use(), system_in_use())
}

/// Bytes in use in every system malloc zone (`malloc_zone_statistics(NULL)`).
fn system_in_use() -> u64 {
    let mut s = libc::malloc_statistics_t { blocks_in_use: 0, size_in_use: 0, max_size_in_use: 0, size_allocated: 0 };
    // SAFETY: a null zone asks for every zone's statistics, written to `s`.
    unsafe { libc::malloc_zone_statistics(std::ptr::null_mut(), &mut s) };
    s.size_in_use as u64
}

/// Returns every jemalloc arena's dirty and muzzy pages to the system (`arena.<all>.purge`): the
/// pages a request's workers freed would otherwise stay resident until jemalloc's decay.
fn purge() {
    // SAFETY: `arena.4096.purge` (4096: MALLCTL_ARENAS_ALL) takes and returns nothing.
    unsafe { tikv_jemalloc_sys::mallctl(c"arena.4096.purge".as_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0) };
}

/// A thread that purges (`purge`) when asked, off the engine thread so that the next request does
/// not wait for it; asks made while one is queued fold into it.
pub struct Purger {
    tx: mpsc::SyncSender<()>,
    /// Purges done, their total and largest milliseconds.
    done: Arc<Mutex<(u64, f64, f64)>>,
}

impl Purger {
    pub fn spawn() -> Purger {
        let (tx, rx) = mpsc::sync_channel::<()>(1);
        let done: Arc<Mutex<(u64, f64, f64)>> = Arc::new(Mutex::new((0, 0.0, 0.0)));
        let d = done.clone();
        std::thread::Builder::new()
            .name("facts-purge".into())
            .spawn(move || {
                while rx.recv().is_ok() {
                    let t = Instant::now();
                    purge();
                    let ms = t.elapsed().as_secs_f64() * 1e3;
                    let mut d = d.lock();
                    *d = (d.0 + 1, d.1 + ms, d.2.max(ms));
                }
            })
            .expect("purge thread");
        Purger { tx, done }
    }

    /// Asks for a purge (none is queued twice).
    pub fn ask(&self) {
        let _ = self.tx.try_send(());
    }

    pub fn json(&self) -> Json {
        let d = *self.done.lock();
        json!({"count": d.0, "ms": d.1, "max_ms": d.2})
    }
}

/// Both allocators' counters: jemalloc's allocated (live), active (pages of live extents),
/// resident, mapped, retained and metadata; the system zones' in use and allocated.
pub fn allocators() -> Json {
    use tikv_jemalloc_ctl::stats;
    let _ = tikv_jemalloc_ctl::epoch::advance();
    let mut s = libc::malloc_statistics_t { blocks_in_use: 0, size_in_use: 0, max_size_in_use: 0, size_allocated: 0 };
    // SAFETY: as in `system_in_use`.
    unsafe { libc::malloc_zone_statistics(std::ptr::null_mut(), &mut s) };
    json!({
        "jemalloc": {
            "allocated": stats::allocated::read().unwrap_or(0), "active": stats::active::read().unwrap_or(0),
            "resident": stats::resident::read().unwrap_or(0), "mapped": stats::mapped::read().unwrap_or(0),
            "retained": stats::retained::read().unwrap_or(0), "metadata": stats::metadata::read().unwrap_or(0),
        },
        "system": {"in_use": s.size_in_use, "allocated": s.size_allocated, "max_in_use": s.max_size_in_use},
        "rss": crate::repo::cur_rss(),
    })
}
