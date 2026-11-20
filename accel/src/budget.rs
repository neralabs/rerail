//! The process's memory budgets (decision 0126): the one place every bounded part of the engine reads its size from.
//!
//! - **Overall** (`--memory-mb`, `RERAIL_MEMORY_MB`; 1 GB by default): what the process keeps resident on the host.
//!   The parts sized from it (the write queue, the working set, what a session keeps of its cross-file part) take
//!   a fixed share of it: `share(part_at_default)` is the part's size at the default budget, scaled.
//! - **Metal** (`--metal-mb`, `RERAIL_METAL_MB`; half the overall budget by default): the device's buffers, on top of
//!   the overall budget. A run with `--metal` may use the two together: 1.5 GB by default.
//!
//! A command line sets them (`set_memory_mb`, `set_metal_mb`) before the run starts; read before, they are the
//! environment's or the defaults. They are read on every use, so a test may set them.

use std::sync::atomic::{AtomicUsize, Ordering};

/// The overall budget by default: 1 GB.
pub const MEMORY_DEFAULT: usize = 1 << 30;

/// The budgets set (`set_memory_mb`, `set_metal_mb`; 0: from the environment, or the defaults): the machine's
/// (decision 0442), which every caller on it shares.
#[derive(Default)]
pub(crate) struct Budgets {
    memory: AtomicUsize,
    metal: AtomicUsize,
}

/// The budgets of the caller's machine: what a lease keeps to go back to its own (`gpu::shared::Pooled`).
pub(crate) fn budgets() -> std::sync::Arc<Budgets> {
    rerail_context::machine::<Budgets>()
}

impl Budgets {
    /// This machine's Metal budget, in bytes (`metal`).
    pub(crate) fn metal(&self) -> usize {
        #[cfg(test)]
        if let bytes @ 1.. = METAL_TEST.with(std::cell::Cell::get) {
            return bytes;
        }
        match self.metal.load(Ordering::Relaxed) {
            0 => env_mb("RERAIL_METAL_MB").unwrap_or_else(|| self.memory() / 2),
            bytes => bytes,
        }
    }

    /// This machine's overall budget, in bytes (`memory`).
    pub(crate) fn memory(&self) -> usize {
        match self.memory.load(Ordering::Relaxed) {
            0 => {
                let bytes = env_mb("RERAIL_MEMORY_MB").unwrap_or(MEMORY_DEFAULT);
                self.memory.store(bytes, Ordering::Relaxed);
                bytes
            }
            bytes => bytes,
        }
    }
}

fn env_mb(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|&mb| mb > 0).map(|mb| mb << 20)
}

/// The overall budget, in bytes.
pub fn memory() -> usize {
    budgets().memory()
}

#[cfg(test)]
thread_local! {
    /// A test's Metal budget, on its thread alone (tests run side by side; the budgets are the process's).
    pub(crate) static METAL_TEST: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The Metal budget, in bytes: half the overall budget unless set.
pub fn metal() -> usize {
    budgets().metal()
}

/// Sets the overall budget (a command line's `--memory-mb`); 0 is refused.
pub fn set_memory_mb(mb: usize) -> Result<(), String> {
    if mb == 0 {
        return Err("the memory budget must be at least 1 MB".into());
    }
    budgets().memory.store(mb << 20, Ordering::Relaxed);
    Ok(())
}

/// Sets the Metal budget (a command line's `--metal-mb`); 0 is refused.
pub fn set_metal_mb(mb: usize) -> Result<(), String> {
    if mb == 0 {
        return Err("the Metal budget must be at least 1 MB".into());
    }
    budgets().metal.store(mb << 20, Ordering::Relaxed);
    Ok(())
}

/// A part of the overall budget: `at_default` bytes at the default budget, in proportion to the budget set.
pub fn share(at_default: usize) -> usize {
    (at_default as u128 * memory() as u128 / MEMORY_DEFAULT as u128) as usize
}

/// A part of the Metal budget: `at_default` bytes of its default (half the default overall budget), in proportion.
pub fn metal_share(at_default: usize) -> usize {
    (at_default as u128 * metal() as u128 / (MEMORY_DEFAULT / 2) as u128) as usize
}

/// The machine's count (decision 0442: the machine's): the reader of the heap's live bytes, from the process's
/// allocator (`set_heap_reader`: the engine's jemalloc, 0 until one is set), and the most the count has been when read.
#[derive(Default)]
struct Count {
    heap: std::sync::OnceLock<fn() -> usize>,
    /// The most the count has been when read (`written`), for the profile.
    most: AtomicUsize,
    /// The most since the last `take_most`, beside the run's most (`most`).
    latest: AtomicUsize,
}

fn count() -> std::sync::Arc<Count> {
    rerail_context::machine::<Count>()
}

/// Sets the machine's reader of the heap's live bytes (once; a second call is ignored), and starts the sampler that
/// keeps the count's most (`take_most`): every `SAMPLE` while the process lives.
pub fn set_heap_reader(f: fn() -> usize) {
    let c = count();
    if c.heap.set(f).is_ok() {
        // The machine's own thread: it reads its caller's shared and machine values (the pool, the host blocks).
        let shared = rerail_context::shared_here();
        let _ = std::thread::Builder::new().name("rerail-budget-sampler".into()).spawn(move || {
            let _within = rerail_context::within(shared);
            loop {
                let _ = c.written();
                std::thread::sleep(SAMPLE);
            }
        });
    }
}

/// How often the sampler reads the count: a chunk's peak between two reads is missed by at most what 10 ms writes.
const SAMPLE: std::time::Duration = std::time::Duration::from_millis(10);

/// What the run has written, by where it lies (decision 0435 §3): the one count its memory is weighed by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Written {
    /// The heap's live bytes.
    pub heap: usize,
    /// Host blocks' used bytes (`crate::blocks::host_held`): resident only where written.
    pub host_blocks: usize,
    /// The GPU pool's leases, and its idle buffers.
    pub leased: usize,
    pub idle: usize,
    /// `rerail-metal`'s ledger (its reservations).
    pub ledger: usize,
}

impl Written {
    pub fn total(&self) -> usize {
        self.heap + self.host_blocks + self.leased + self.idle + self.ledger
    }
}

/// The run's count now (decision 0435 §3). Not the footprint, which misses pages the GPU writes into a no-copy wrap,
/// and not the driver's count, which counts a wrap of host memory again.
pub fn written() -> Written {
    count().written()
}

impl Count {
    fn written(&self) -> Written {
        let heap = self.heap.get().map_or(0, |f| f());
        let host_blocks = crate::blocks::host_held().0;
        #[cfg(all(feature = "gpu", target_os = "macos"))]
        let ((leased, idle), ledger) = (crate::gpu::shared::held_bytes(), crate::gpu::shared::ledger_bytes());
        #[cfg(not(all(feature = "gpu", target_os = "macos")))]
        let ((leased, idle), ledger) = ((0, 0), 0);
        let w = Written { heap, host_blocks, leased, idle, ledger };
        self.most.fetch_max(w.total(), Ordering::Relaxed);
        self.latest.fetch_max(w.total(), Ordering::Relaxed);
        w
    }
}

/// The most the count has been when read.
pub fn written_most() -> usize {
    count().most.load(Ordering::Relaxed)
}

/// The most the count has been since the last call (read now too), and starts the next span from the count now: a
/// chunk's peak, when it takes this at its start and at its end (decision 0435 §4).
pub fn take_most() -> usize {
    let c = count();
    let now = c.written().total();
    c.latest.swap(now, Ordering::Relaxed).max(now)
}

/// What the count is weighed against: the overall budget and Metal's together (decision 0435 §3; 0126's two numbers,
/// one memory on a machine whose device shares it).
pub fn limit() -> usize {
    memory() + metal()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The count rises by what a host block has written, not by its reservation (decision 0435 R3). The count is
    /// the process's, which the tests running beside this one move both ways (a block of theirs freed made a
    /// difference of two counts negative), so the block's own term is checked: its used bytes (`host_held` sums
    /// them), within the count.
    #[test]
    fn the_count_is_what_a_block_wrote_not_what_it_reserved() {
        use rerail_blocks::alloc::{Device, Need, Writes};
        let b = crate::blocks::policy(&Need { kind: rerail_blocks::block::kind::FACTS, bytes: 64 << 20, writes: Writes::Churn, device: Device::May, ..Need::SLAB }).unwrap();
        assert!(b.reserved() >= 64 << 20);
        let untouched = b.used();
        assert!(untouched < 1 << 20, "a 64 MB reservation counts {untouched} bytes untouched");
        b.try_alloc_chunk(8 << 20, 1 << 14).unwrap();
        let after = b.used();
        assert!(after >= 8 << 20 && after < 9 << 20, "8 MB written counts {after} bytes");
        assert!(written().host_blocks >= after, "the block is in the count");
        assert!(limit() == memory() + metal());
        drop(b);
    }

    /// One test owns the process's budgets (they are process-wide).
    #[test]
    fn budgets_default_set_and_share() {
        if std::env::var_os("RERAIL_MEMORY_MB").is_none() && std::env::var_os("RERAIL_METAL_MB").is_none() {
            assert_eq!(memory(), 1 << 30);
            assert_eq!(metal(), 512 << 20, "Metal defaults to half the overall budget");
            assert_eq!(share(192 << 20), 192 << 20);
        }
        set_memory_mb(2048).unwrap();
        assert_eq!(share(192 << 20), 384 << 20, "a part scales with the overall budget");
        assert_eq!(metal(), 1 << 30, "unset, Metal follows the overall budget");
        METAL_TEST.with(|m| m.set(256 << 20));
        assert_eq!(metal(), 256 << 20);
        assert_eq!(metal_share(64 << 20), 32 << 20);
        METAL_TEST.with(|m| m.set(0));
        assert!(set_memory_mb(0).is_err());
    }
}
