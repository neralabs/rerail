//! The one Metal manager for relational work (decision 0100 §2): what every relational submission on the GPU shares,
//! once per process.
//!
//! - **The device and one command queue.** The executor's GPU lane and the engine's sorts and probes
//!   (`metal_sort`, `metal_rows`) commit to the same queue, so their command buffers are ordered with each other.
//! - **Pipelines** by a key the caller chooses (a kernel's name and its variant): compiled once, found without
//!   hashing their source.
//! - **A pool of shared buffers** by size class: powers of two from 16 KiB, as decision 0039's blocks. A buffer a
//!   command has used is resident already, so a pooled one costs neither its creation nor its first mapping (about
//!   6 µs against 0.1 µs, measured in 0095). At most `IDLE_LIMIT` bytes are kept idle. A lease goes back only when
//!   it is dropped, and its holder drops it once every command buffer that reads it has completed: the host's next
//!   write into it is not ordered after a GPU read.
//! - **The guard.** A function the engine sets to its launch budget. Every submission checks it first, on the
//!   submitting thread, so relational work is refused by the same budget as the rest of the engine's GPU work.
//!
//! Everything here is safe to share between threads: Metal's device, queues and pipelines are, and the pool is
//! behind a lock.

use metal::{Buffer, BufferRef, CommandQueue, ComputePipelineState, Device, MTLResourceOptions};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The smallest size class.
const SMALLEST: usize = 16 << 10;
/// The most bytes the pool keeps idle (decision 0039).
const IDLE_LIMIT: usize = 32 << 20;

/// The device and its one queue.
pub struct Shared {
    pub device: Device,
    pub queue: CommandQueue,
    /// The host's page size (a column wrapped without a copy must be page-aligned).
    pub page: usize,
    pub max_buffer: u64,
}

/// The process's device and queue: `None` without a Metal device.
pub fn get() -> Option<&'static Shared> {
    static SHARED: OnceLock<Option<Shared>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            objc::rc::autoreleasepool(|| {
                let device = Device::system_default()?;
                let queue = device.new_command_queue();
                let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as usize;
                let max_buffer = device.max_buffer_length();
                Some(Shared { device, queue, page, max_buffer })
            })
        })
        .as_ref()
}

/// A pipeline's slot: made once. A thread that asks while another makes it waits for that one (the prewarm's
/// compile is not done twice by a job that needs it first).
type Slot = std::sync::Arc<Mutex<Option<ComputePipelineState>>>;

fn pipelines() -> &'static Mutex<HashMap<String, Slot>> {
    static PIPELINES: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    PIPELINES.get_or_init(Default::default)
}

/// The pipeline kept under `key`, made by `make` the first time. A caller that finds another thread making it
/// waits, scored as the phase `pipeline wait` (a cold job, like the compile itself).
pub fn pipeline<E>(key: &str, make: impl FnOnce() -> Result<ComputePipelineState, E>) -> Result<ComputePipelineState, E> {
    let slot = std::sync::Arc::clone(pipelines().lock().expect("the pipelines").entry(key.to_owned()).or_default());
    let mut s = match slot.try_lock() {
        Ok(s) => s,
        Err(std::sync::TryLockError::WouldBlock) => crate::score::phase("pipeline wait", || slot.lock().unwrap_or_else(|p| p.into_inner())),
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
    };
    if let Some(p) = &*s {
        return Ok(p.clone());
    }
    let p = crate::score::phase("pipeline compile", make)?;
    *s = Some(p.clone());
    Ok(p)
}

#[derive(Default)]
struct Pool {
    idle: HashMap<usize, Vec<Buffer>>,
    /// Bytes idle.
    held: usize,
    /// Leases made and served from the pool, for the profile.
    made: u64,
    reused: u64,
}

fn pool() -> &'static Mutex<Pool> {
    static POOL: OnceLock<Mutex<Pool>> = OnceLock::new();
    POOL.get_or_init(Default::default)
}

fn class(bytes: usize) -> usize {
    bytes.max(SMALLEST).next_power_of_two()
}

/// A shared buffer of the pool, returned to it when dropped.
pub struct Pooled {
    buffer: Option<Buffer>,
    /// Whether it goes back to the pool (a buffer past the largest class is made to size and released).
    keep: bool,
}

impl Pooled {
    pub fn buffer(&self) -> &Buffer {
        self.buffer.as_ref().expect("a leased buffer")
    }

    /// Releases the buffer to Metal instead of the pool: a command buffer still encoding or running holds it until
    /// it completes, and no later lease can be handed it meanwhile.
    pub fn release(mut self) {
        self.keep = false;
    }

    /// The buffer's host address.
    pub fn host(&self) -> *mut u8 {
        self.buffer().contents().cast()
    }
}

impl std::ops::Deref for Pooled {
    type Target = BufferRef;
    fn deref(&self) -> &BufferRef {
        self.buffer()
    }
}

impl Drop for Pooled {
    fn drop(&mut self) {
        let Some(b) = self.buffer.take() else { return };
        if !self.keep {
            return;
        }
        let class = b.length() as usize;
        let mut p = pool().lock().expect("the pool");
        if p.held + class <= IDLE_LIMIT {
            p.held += class;
            p.idle.entry(class).or_default().push(b);
        }
    }
}

/// A buffer of at least `bytes`. Its contents are a previous lease's: every reader writes it first.
pub fn lease(bytes: usize) -> Result<Pooled, String> {
    let shared = get().ok_or("no Metal device")?;
    let class = class(bytes);
    if class as u64 > shared.max_buffer {
        if bytes as u64 > shared.max_buffer {
            return Err(format!("a buffer of {bytes} bytes is past the device's largest ({})", shared.max_buffer));
        }
        let b = objc::rc::autoreleasepool(|| shared.device.new_buffer(bytes as u64, MTLResourceOptions::StorageModeShared));
        return Ok(Pooled { buffer: Some(b), keep: false });
    }
    let mut p = pool().lock().expect("the pool");
    if let Some(b) = p.idle.get_mut(&class).and_then(Vec::pop) {
        p.held -= class;
        p.reused += 1;
        return Ok(Pooled { buffer: Some(b), keep: true });
    }
    p.made += 1;
    drop(p);
    let b = crate::score::phase("new buffer", || objc::rc::autoreleasepool(|| shared.device.new_buffer(class as u64, MTLResourceOptions::StorageModeShared)));
    Ok(Pooled { buffer: Some(b), keep: true })
}

/// A buffer of at least `bytes`, its first `bytes` zero (a new buffer is zero already).
pub fn lease_zeroed(bytes: usize) -> Result<Pooled, String> {
    let b = lease(bytes)?;
    // SAFETY: the lease is this caller's alone, its buffer at least `bytes` long and not in use by any command.
    unsafe { std::ptr::write_bytes(b.host(), 0, bytes) };
    Ok(b)
}

/// The pool's counts: bytes idle, leases made, leases served from the pool.
pub fn pool_counts() -> (usize, u64, u64) {
    let p = pool().lock().expect("the pool");
    (p.held, p.made, p.reused)
}

/// Releases every idle buffer and every pipeline (the engine's drop after its last execution, decision 0032).
pub fn drop_idle() {
    let mut p = pool().lock().expect("the pool");
    p.idle.clear();
    p.held = 0;
    drop(p);
    pipelines().lock().expect("the pipelines").clear();
}

static GUARD: OnceLock<fn() -> Result<(), String>> = OnceLock::new();

/// Sets the check every submission makes first (once per process; a second call is ignored).
pub fn set_guard(f: fn() -> Result<(), String>) {
    let _ = GUARD.set(f);
}

/// The submission check: the engine's launch budget when it set one.
pub fn guard() -> Result<(), String> {
    GUARD.get().map_or(Ok(()), |f| f())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leases_return_to_the_pool_by_size_class() {
        if get().is_none() {
            return;
        }
        // Other tests lease from the same pool at the same time, so only the sizes and the zeroing are this test's.
        let a = lease(20_000).unwrap();
        assert_eq!(a.length(), 32 << 10);
        // SAFETY: the lease is this test's, 32 KiB long.
        unsafe { std::ptr::write_bytes(a.host(), 7, 32 << 10) };
        drop(a);
        let b = lease_zeroed(30_000).unwrap();
        assert_eq!(b.length(), 32 << 10);
        // SAFETY: the lease is this test's, 32 KiB long.
        assert!(unsafe { std::slice::from_raw_parts(b.host(), 30_000) }.iter().all(|&x| x == 0));
        let p = pipeline::<String>("test no-op", || Err("not made".into()));
        assert!(p.is_err());
    }
}
