//! Pre-pinned blocks (decision 0039): the device's scratch buffers, made once and reused.
//!
//! A device solve needs a few buffers for its inputs, its working arrays and its flags. Making
//! them for each call costs a mapping, the first write of every page, a buffer over it, and the
//! device making the new pages resident before the kernel starts. On a 172K-node piece that is
//! 0.4–0.6 ms a call against 1–2.5 ms of GPU work (`nearest_costs_by_solver`).
//!
//! A block is a lease of the one GPU buffer pool (`rerail_accel::gpu::shared`, 0266 §16): its size
//! classes, its one budget with every other GPU buffer, and its one eviction order (idle buffers,
//! the least recently returned first, then the residency's least recently used slabs). A block
//! made for its lease has every page written once, so it is backed and the device has seen it; it
//! goes back to the pool when its lease drops, for the next lease of its class, of any user.
//!
//! - **Refused** (`Error::BackendCapacity`) when the budget cannot make room for it: the caller
//!   splits its batch.
//! - **Everything idle is dropped** with the residency slabs, two minutes after the last execution
//!   (the engine's `residency::slabs::drop_all`).
//! - **One command queue** for the solves that lease blocks: the process's (`lane`).
//!
//! A lease's holder waits for every command that reads the block before dropping the lease.
use crate::Error;
use rerail_accel::gpu::shared::{self, Pooled};
use std::mem::size_of;

/// A leased block: at least the bytes asked for. `made` says whether it was made for this lease
/// (else it was reused).
pub struct Block {
    lease: Pooled,
    pub made: bool,
}

impl Block {
    /// The block's buffer in the device layer (decision 0440 step C), on any backend.
    pub fn device_buffer(&self) -> &rerail_device::device::Buffer {
        self.lease.device_buffer()
    }

    pub fn host(&self) -> *mut u8 {
        self.lease.host()
    }

    pub fn byte_length(&self) -> usize {
        self.lease.length() as usize
    }

    /// The first `count` values of type `T`.
    pub fn read<T: Copy>(&self, count: usize) -> Vec<T> {
        assert!(count * size_of::<T>() <= self.byte_length());
        // SAFETY: in bounds; the device is done with the block (the holder waited).
        unsafe { std::slice::from_raw_parts(self.host().cast::<T>(), count).to_vec() }
    }
}

static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static MADE_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The blocks made so far, and their bytes: a warm run makes none (each is a new buffer, every
/// page faulted in by the host).
pub fn made() -> (usize, usize) {
    use std::sync::atomic::Ordering::Relaxed;
    (MADE.load(Relaxed), MADE_BYTES.load(Relaxed))
}

/// A block of at least `bytes`, its contents whatever its last lease left.
pub fn lease(bytes: usize) -> Result<Block, Error> {
    let lease = shared::lease_within(bytes.max(1)).map_err(|refused| {
        if std::env::var("RERAIL_POOL_DEBUG").is_ok() {
            eprintln!("pool: {refused}");
        }
        rerail_prof::count("capacity.metal_pool_ledger", 1);
        Error::BackendCapacity
    })?;
    let made = lease.fresh();
    if made {
        let length = lease.length() as usize;
        MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        MADE_BYTES.fetch_add(length, std::sync::atomic::Ordering::Relaxed);
        // Every page written once: backed now, not on the device's first touch.
        // SAFETY: `length` bytes of a new buffer, this lease's alone.
        unsafe { std::ptr::write_bytes(lease.host(), 0, length) };
    }
    Ok(Block { lease, made })
}

/// A block holding `values` (at least one byte long).
pub fn upload<T: Copy>(values: &[T]) -> Result<Block, Error> {
    fill(lease(size_of_val(values).max(1))?, values)
}

fn fill<T: Copy>(block: Block, values: &[T]) -> Result<Block, Error> {
    // SAFETY: the block holds at least these bytes; no command reads it yet.
    unsafe { std::ptr::copy_nonoverlapping(values.as_ptr().cast::<u8>(), block.host(), size_of_val(values)) };
    Ok(block)
}

/// A block whose first `count` values of `T` are zero.
pub fn zeroed<T>(count: usize) -> Result<Block, Error> {
    let bytes = count.checked_mul(size_of::<T>()).ok_or(Error::BackendCapacity)?;
    clear(lease(bytes.max(1))?, bytes)
}

/// A block whose first `count` values of `T` are zero, zeroed by the device (a fill on the
/// process's device, decision 0440 step C), not by the host: for the large ones, such as a run's
/// slots.
pub fn zeroed_on_device<T>(count: usize) -> Result<Block, Error> {
    let bytes = count.checked_mul(size_of::<T>()).ok_or(Error::BackendCapacity)?;
    let block = lease(bytes.max(1))?;
    if bytes == 0 {
        return Ok(block);
    }
    debug_assert!(bytes.div_ceil(4) * 4 <= block.byte_length(), "a lease of whole words");
    // A launch scope of its own (the evaluation's, when one is running): a submission outside one is refused.
    let _scope = crate::launch::LaunchScope::enter();
    let mut command = crate::lane::Command::new("Rerail zeroed block");
    command.fill(block.device_buffer(), 0, bytes.div_ceil(4) as u64, 0);
    command.run()?;
    Ok(block)
}

fn clear(block: Block, bytes: usize) -> Result<Block, Error> {
    // SAFETY: as `upload`.
    unsafe { std::ptr::write_bytes(block.host(), 0, bytes) };
    Ok(block)
}

/// Releases every idle buffer of the pool (the engine's drop: its blocks are the pool's).
pub fn drop_all() {
    shared::drop_free();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A returned block serves the next lease of its size class, written over in place.
    #[test]
    fn blocks_are_reused_by_size_class() {
        if crate::lane::device().is_err() {
            return;
        }
        // 3 MiB + 1: a class (3.5 MiB) no other test leases.
        let bytes: usize = (3 << 20) + 1;
        let a = upload(&vec![7_u32; bytes.div_ceil(4)]).unwrap();
        assert!(a.byte_length() >= bytes);
        assert_eq!(a.read::<u32>(3), [7, 7, 7]);
        let first = a.device_buffer().device_address();
        drop(a);
        let b = zeroed::<u8>(bytes).unwrap();
        assert!(!b.made, "the returned block served the lease");
        assert_eq!(b.device_buffer().device_address(), first);
        assert_eq!(b.read::<u32>(3), [0, 0, 0]);
    }
}
