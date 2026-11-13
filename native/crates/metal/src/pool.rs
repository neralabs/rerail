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
//!   past it is released. Idle blocks give way to any buffer the budget would refuse
//!   (`give_way`), the largest first.
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
/// Bytes of idle blocks kept: up to the Metal buffer budget. A tier 1 run's slots take most of
/// it (848 bytes a block of `.rr`), and a block made again at each run cost the host most of
/// its time: a fresh mapping, every page faulted in and written. Idle blocks give way to a
/// lease the budget would refuse (`lease_in`).
const SHARE: usize = crate::runtime::METAL_BUFFER_LIMIT;

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
            if std::env::var("RERAIL_POOL_DEBUG").is_ok() {
                eprintln!("pool: a block of {} MiB released ({} MiB idle)", bytes >> 20, pool.held >> 20);
            }
            return;
        }
        if pool.idle.len() <= self.class {
            pool.idle.resize_with(self.class + 1, Vec::new);
        }
        pool.idle[self.class].push(buffer);
        pool.held += bytes;
    }
}

static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static MADE_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The blocks made so far, and their bytes: a warm run makes none (each is a fresh mapping, every
/// page faulted in by the host).
pub fn made() -> (usize, usize) {
    use std::sync::atomic::Ordering::Relaxed;
    (MADE.load(Relaxed), MADE_BYTES.load(Relaxed))
}

/// Releases the device pool's idle blocks, the largest first, until `bytes` more fit the budget
/// of `limit` bytes; whether any was released. None is when even all would not make room.
pub(crate) fn give_way(bytes: usize, limit: usize) -> bool {
    let Ok(mut pool) = POOL.lock() else { return false };
    let used = || crate::runtime::reserved();
    if used().saturating_sub(pool.held) + bytes > limit {
        return false;
    }
    let mut released = false;
    while used() + bytes > limit {
        let Some(c) = (0..pool.idle.len()).rev().find(|&c| !pool.idle[c].is_empty()) else { break };
        if std::env::var("RERAIL_POOL_DEBUG").is_ok() {
            eprintln!("pool: an idle block of {} MiB gives way to {} MiB", (SMALLEST << c) >> 20, bytes >> 20);
        }
        // Dropping the buffer frees its reservation.
        pool.idle[c].pop();
        pool.held -= SMALLEST << c;
        released = true;
    }
    released
}

/// A block of at least `bytes`, its contents whatever its last lease left.
pub fn lease(bytes: usize) -> Result<Block, Error> {
    lease_in(&POOL, bytes)
}

fn lease_in(shared: &'static Shared, bytes: usize) -> Result<Block, Error> {
    let class = class(bytes);
    {
        let mut pool = shared.lock().map_err(|_| Error::BackendFailure)?;
        // An idle block of the class, or of the one above: a batch a little smaller than the
        // last takes the last's block, and the budget holds one large block, not two.
        for c in [class, class + 1] {
            if let Some(buffer) = pool.idle.get_mut(c).and_then(Vec::pop) {
                pool.held -= SMALLEST << c;
                return Ok(Block { buffer: Some(buffer), pool: shared, class: c, made: false });
            }
        }
    }
    let length = SMALLEST.checked_shl(class as u32).ok_or(Error::BackendCapacity)?;
    let buffer = MappedBuffer::slab(length)?;
    MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    MADE_BYTES.fetch_add(length, std::sync::atomic::Ordering::Relaxed);
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

/// A block whose first `count` values of `T` are zero, zeroed by the device (a blit's fill),
/// not by the host: for the large ones, such as a run's slots.
pub fn zeroed_on_device<T>(count: usize) -> Result<Block, Error> {
    let bytes = count.checked_mul(size_of::<T>()).ok_or(Error::BackendCapacity)?;
    let block = lease(bytes.max(1))?;
    if bytes == 0 {
        return Ok(block);
    }
    let device = crate::METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let queue = queue(device)?;
    let _scope = crate::launch::LaunchScope::enter();
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        command.set_label("Rerail zeroed block");
        let blit = command.new_blit_command_encoder();
        blit.fill_buffer(block.buffer(), metal::NSRange::new(0, bytes as u64), 0);
        blit.end_encoding();
        if let Err(refused) = crate::launch::commit_checked(command) {
            crate::launch::release_refused(command);
            return Err(refused);
        }
        crate::launch::wait_checked(command)
    })?;
    Ok(block)
}

fn clear(block: Block, bytes: usize) -> Result<Block, Error> {
    // SAFETY: as `upload`.
    unsafe { std::ptr::write_bytes(block.host(), 0, bytes) };
    Ok(block)
}

/// The command queue for solves over blocks: the process's one (`rerail_accel::gpu::shared`, decision 0100 §2), so
/// the engine's programs and the executor's lanes submit in one order.
pub fn queue(_device: &Device) -> Result<metal::CommandQueue, Error> {
    crate::queue()
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
    }

    #[test]
    fn idle_blocks_give_way_to_the_budget() {
        if METAL_DEVICE.is_none() {
            return;
        }
        // Idle, a block of most of the budget: a buffer as large is still made, and the idle
        // block is what gave way.
        let most = crate::runtime::METAL_BUFFER_LIMIT / 2 + SMALLEST;
        drop(lease(most).unwrap());
        assert!(held() >= most);
        let other = MappedBuffer::slab(most).unwrap();
        assert!(held() < most);
        drop(other);
    }
}
