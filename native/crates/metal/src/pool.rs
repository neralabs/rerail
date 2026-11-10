//! Pre-pinned blocks (decision 0039): the device's scratch buffers, made once and reused.
//!
//! A device solve needs a few buffers for its inputs, its working arrays and its flags. Making
//! them for each call costs a mapping, the first write of every page, a no-copy Metal buffer over
//! it, and the device making the new pages resident before the kernel starts. On a 172K-node
//! piece that is 0.4–0.6 ms a call against 1–2.5 ms of GPU work (`nearest_costs_by_solver`).
//!
//! A block is the same kind of buffer as a residency slab (decision 0032): an anonymous mapping
//! with one retained no-copy shared `MTLBuffer` over it. Here every page is written once when the
//! block is made, so it is backed and the device has seen it, and the block goes back to a pool
//! when its lease drops, for the next call of any size up to its own.
//!
//! - **Sizes** are powers of two from `SMALLEST` bytes, so a lease wastes at most half a block
//!   and a run of growing chunks makes few blocks.
//! - **What is kept** is bounded by `SHARE` bytes of the Metal buffer budget. A block returned
//!   past it is released.
//! - **Everything is dropped** with the residency slabs, two minutes after the last execution
//!   (the engine's `residency::slabs::drop_all`).
//! - **One command queue** is kept for the solves that lease blocks (`queue`).
//!
//! A lease's holder waits for every command that reads the block before dropping the lease.
use crate::Error;
use crate::runtime::MappedBuffer;
use metal::Device;
use std::mem::size_of;


/// The smallest block: one 16 KiB page.
const SMALLEST: usize = 16 << 10;
/// Bytes of idle blocks kept: 32 MiB of the 256 MB Metal buffer budget.
const SHARE: usize = 32 << 20;

struct Pool {
    /// Idle blocks per size class (`SMALLEST << class`).
    idle: Vec<Vec<MappedBuffer>>,
    /// Bytes in `idle`.
    held: usize,
    queue: Option<metal::CommandQueue>,
}

type Shared = std::sync::Mutex<Pool>;

static POOL: Shared = Shared::new(Pool { idle: Vec::new(), held: 0, queue: None });

fn class(bytes: usize) -> usize {
    (bytes.max(SMALLEST).next_power_of_two() / SMALLEST).trailing_zeros() as usize
}

/// A leased block: at least the bytes asked for. `made` says whether it was made for this lease
/// (else it was reused).
pub struct Block {
    buffer: Option<MappedBuffer>,
    pool: &'static Shared,
    class: usize,
    pub made: bool,
}

impl Block {
    pub fn buffer(&self) -> &metal::BufferRef {
        &self.buffer.as_ref().expect("a leased block").buffer
    }

    pub fn host(&self) -> *mut u8 {
        self.buffer.as_ref().expect("a leased block").host()
    }

    pub fn byte_length(&self) -> usize {
        SMALLEST << self.class
    }

    /// The first `count` values of type `T`.
    pub fn read<T: Copy>(&self, count: usize) -> Vec<T> {
        assert!(count * size_of::<T>() <= self.byte_length());
        // SAFETY: in bounds; the device is done with the block (the holder waited).
        unsafe { std::slice::from_raw_parts(self.host().cast::<T>(), count).to_vec() }
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        let Some(buffer) = self.buffer.take() else { return };
        let bytes = self.byte_length();
        let Ok(mut pool) = self.pool.lock() else { return };
        if pool.held + bytes > SHARE {
            return;
        }
        if pool.idle.len() <= self.class {
            pool.idle.resize_with(self.class + 1, Vec::new);
        }
        pool.idle[self.class].push(buffer);
        pool.held += bytes;
    }
}

/// A block of at least `bytes`, its contents whatever its last lease left.
pub fn lease(bytes: usize) -> Result<Block, Error> {
    lease_in(&POOL, bytes)
}

fn lease_in(shared: &'static Shared, bytes: usize) -> Result<Block, Error> {
    let class = class(bytes);
    {
        let mut pool = shared.lock().map_err(|_| Error::BackendFailure)?;
        if let Some(buffer) = pool.idle.get_mut(class).and_then(Vec::pop) {
            pool.held -= SMALLEST << class;
            return Ok(Block { buffer: Some(buffer), pool: shared, class, made: false });
        }
    }
    let length = SMALLEST.checked_shl(class as u32).ok_or(Error::BackendCapacity)?;
    let buffer = MappedBuffer::slab(length)?;
    // Every page written once: backed now, not on the device's first touch.
    // SAFETY: `length` bytes of a fresh mapping.
    unsafe { std::ptr::write_bytes(buffer.host(), 0, length) };
    Ok(Block { buffer: Some(buffer), pool: shared, class, made: true })
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

fn clear(block: Block, bytes: usize) -> Result<Block, Error> {
    // SAFETY: as `upload`.
    unsafe { std::ptr::write_bytes(block.host(), 0, bytes) };
    Ok(block)
}

/// The command queue for solves over blocks: the process's one (`rerail_accel::gpu::shared`, decision 0100 §2), so
/// the engine's programs and the executor's lanes submit in one order; a queue of its own only without it.
pub fn queue(device: &Device) -> Result<metal::CommandQueue, Error> {
    if let Some(shared) = rerail_accel::gpu::shared::get() {
        return Ok(shared.queue.clone());
    }
    let mut pool = POOL.lock().map_err(|_| Error::BackendFailure)?;
    Ok(pool.queue.get_or_insert_with(|| device.new_command_queue()).clone())
}

/// The bytes of idle blocks kept.
pub fn held() -> usize {
    POOL.lock().map(|p| p.held).unwrap_or(0)
}

/// Releases every idle block and the queue.
pub fn drop_all() {
    if let Ok(mut pool) = POOL.lock() {
        pool.idle.clear();
        pool.held = 0;
        pool.queue = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::METAL_DEVICE;

    /// A returned block serves the next lease of its size class, written over in place; past the share, a
    /// returned block is released.
    #[test]
    fn blocks_are_reused_by_size_class() {
        if METAL_DEVICE.is_none() {
            return;
        }
        static OWN: Shared = Shared::new(Pool { idle: Vec::new(), held: 0, queue: None });
        let held = || OWN.lock().unwrap().held;
        let a = fill(lease_in(&OWN, 12).unwrap(), &[1_u32, 2, 3]).unwrap();
        assert!(a.made);
        assert_eq!(a.byte_length(), SMALLEST);
        assert_eq!(a.read::<u32>(3), [1, 2, 3]);
        let first = a.buffer().gpu_address();
        drop(a);
        assert_eq!(held(), SMALLEST);
        let b = clear(lease_in(&OWN, 400).unwrap(), 400).unwrap();
        assert!(!b.made);
        assert_eq!(b.buffer().gpu_address(), first);
        assert_eq!(b.read::<u32>(3), [0, 0, 0]);
        // A larger lease is another class.
        let c = lease_in(&OWN, SMALLEST + 1).unwrap();
        assert!(c.made && c.byte_length() == 2 * SMALLEST);
        drop((b, c));
        assert_eq!(held(), 3 * SMALLEST);
        // A block as large as the share is not kept.
        drop(lease_in(&OWN, SHARE).unwrap());
        assert_eq!(held(), 3 * SMALLEST);
    }
}
