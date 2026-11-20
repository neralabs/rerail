use crate::Error;
use std::mem::size_of_val;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::Mutex;

use crate::launch;

// The device pool: every Metal buffer alive at once, reserved before its mmap, copy or Metal
// allocation (page alignment and empty buffers included). Its size is the device's share of the
// process's 500 MB (02-shape §6: the resident tapes, 128-192 MB, and the plan-sized set, lattice
// and output pools, 32-64 MB), in place of a 2 GB guard (list step 13). A plan's buffers are
// sized from it (counted, not worst-case: `label_passes`), and a batch that does not fit is
// refused as `BackendCapacity` and split (`waves::matches::labelled_metal`). S7: peak 248 MB.
// The size is the process's Metal budget (`rerail_accel::budget::metal`, `--metal-mb`; 512 MB by default, decision
// 0126), on top of the overall budget.
pub fn metal_buffer_limit() -> usize {
    rerail_accel::budget::metal()
}
/// The device's state on this machine (decision 0442: the machine's): the pool's ledger and the most it held, and
/// the allocations' lock. The compiled pipelines are the store of compiled kernels'
/// (`rerail_accel::gpu::kernels`, decision 0395).
/// A reservation holds the ledger it charged, so its drop on any thread goes back to it. A test build off macOS has
/// none of it: its ledgers are its threads' (`test_pool`), and only Metal's driver is weighed under the lock.
#[cfg(any(target_os = "macos", not(any(test, feature = "test-hooks"))))]
#[derive(Default)]
pub(crate) struct DeviceState {
    /// The pool's ledger and its most (a test build's are its thread's: `test_pool`).
    #[cfg(not(any(test, feature = "test-hooks")))]
    pool: Arc<AtomicUsize>,
    #[cfg(not(any(test, feature = "test-hooks")))]
    peak: Arc<AtomicUsize>,
    #[cfg(target_os = "macos")]
    allocation: Mutex<()>,
}

#[cfg(any(target_os = "macos", not(any(test, feature = "test-hooks"))))]
pub(crate) fn state() -> Arc<DeviceState> {
    rerail_context::machine::<DeviceState>()
}

/// The pool's ledger: the bytes reserved now.
#[cfg(not(any(test, feature = "test-hooks")))]
fn pool() -> Arc<AtomicUsize> {
    Arc::clone(&state().pool)
}

/// The most the pool's ledger has held.
#[cfg(not(any(test, feature = "test-hooks")))]
fn pool_peak() -> Arc<AtomicUsize> {
    Arc::clone(&state().peak)
}

/// Whether the driver refuses a buffer of `length` more: past the device's largest buffer, or past the OS's working
/// set for the device (`recommendedMaxWorkingSetSize`) with what the driver holds. Not past the Metal budget: the
/// driver's figure counts every buffer Metal knows of, the no-copy wraps of the run's host blocks (row blocks, tapes,
/// journals) with them, which are the host budget's pages, not device memory the budget bounds. Compared with the
/// budget, a 1-node wave's buffer was refused with 337 MB of a 2 GB budget free (the driver at 3.7 GB, the pool's own
/// at 1.7 GB; t62, Q203). The budget is the pool's ledger and leases, checked where they are reserved (`acquire`,
/// `make_room`).
#[cfg(target_os = "macos")]
fn driver_refuses(device: &metal::DeviceRef, length: usize) -> bool {
    let length = length as u64;
    let driver = driver_bytes(device);
    if rerail_prof::on() && driver.saturating_add(length) > metal_buffer_limit() as u64 {
        rerail_prof::count("capacity.driver_past_budget", 1);
    }
    length > device.max_buffer_length() || driver.saturating_add(length) > working_set(device)
}

/// The OS's working set for the device: what it can hold resident at once.
#[cfg(target_os = "macos")]
fn working_set(device: &metal::DeviceRef) -> u64 {
    #[cfg(any(test, feature = "test-hooks"))]
    if let Some(bytes) = test_pool::working_set() {
        return bytes;
    }
    device.recommended_max_working_set_size()
}

#[cfg(any(test, feature = "test-hooks"))]
use crate::test_pool;
/// What the Metal driver has allocated for the device (a test build's: `test_pool::driver_bytes`).
#[cfg(all(target_os = "macos", not(any(test, feature = "test-hooks"))))]
fn driver_bytes(device: &metal::DeviceRef) -> u64 {
    device.current_allocated_size()
}

#[cfg(all(target_os = "macos", any(test, feature = "test-hooks")))]
use test_pool::driver_bytes;
#[cfg(any(test, feature = "test-hooks"))]
use test_pool::{pool, pool_peak};

/// The Metal buffer budget, in bytes.
pub fn buffer_limit() -> usize {
    metal_buffer_limit()
}

/// The bytes of the Metal buffer budget not reserved now.
/// The bytes the device's buffers hold now, of `metal_buffer_limit()`: a user of them that can wait (the logic
/// engine's device joins) keeps within what is left.
pub fn reserved() -> usize {
    pool().load(Ordering::Acquire)
}

/// The bytes of `metal_buffer_limit()` relational work (the logic engine's device joins) may still lease: past the
/// buffer pool's reservations and what relational work holds itself, leased and kept idle in its pool. Its leases
/// reserve nothing in the pool's ledger, so a check of `reserved` alone never saw them: a wave encoded strata until
/// it held 400 MB.
pub fn relational_bytes_left() -> usize {
    rerail_accel::gpu::shared::set_other_reserved(reserved);
    let (live, idle) = rerail_accel::gpu::shared::held_bytes();
    metal_buffer_limit().saturating_sub(pool().load(Ordering::Acquire) + live + idle)
}

pub fn buffer_free() -> usize {
    metal_buffer_limit().saturating_sub(pool().load(Ordering::Acquire))
}

/// The bytes of `metal_buffer_limit()` a reservation can have now (`BufferReservation::acquire`): past the pool's
/// ledger, the relational leases and reservations (`gpu::shared::bound_bytes`), less nothing for idle buffers, which
/// room is made of. Sized from `buffer_free` instead, a matcher wave's workspace was three quarters of the budget
/// while the chunk's row blocks held 275-384 MB of it: refused at every size, split down to one tape, then failed
/// (Q203). A test's ledger is its thread's: as `acquire`, the pool's leases are not counted there.
pub fn buffer_room() -> usize {
    let bound = if cfg!(any(test, feature = "test-hooks")) { 0 } else { rerail_accel::gpu::shared::bound_bytes() };
    metal_buffer_limit().saturating_sub(pool().load(Ordering::Acquire) + bound)
}

/// The bytes of Metal buffers alive now (their reservations), in MB.
pub fn reserved_mb() -> usize {
    pool().load(Ordering::Acquire) >> 20
}

/// The most bytes of Metal buffers alive at once, in MB.
pub fn peak_mb() -> usize {
    pool_peak().load(Ordering::Acquire) >> 20
}

/// What the Metal driver says it has allocated for the device, in MB: none where nothing opened the device (it is
/// not opened to read it), and none off macOS (Metal's count).
pub fn driver_mb() -> usize {
    #[cfg(target_os = "macos")]
    {
        rerail_accel::gpu::shared::opened_device().map_or(0, |device| (device.current_allocated_size() >> 20) as usize)
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}

#[cfg(any(test, feature = "test-hooks"))]
use crate::buffer_observation;
#[cfg(any(test, feature = "test-hooks"))]
pub use buffer_observation::BufferObservation;

struct BufferReservation {
    bytes: usize,
    ledger: Arc<AtomicUsize>,
    /// Admitted to the tests' weighted bound (`test_pool`).
    #[cfg(any(test, feature = "test-hooks"))]
    admitted: bool,
    #[cfg(any(test, feature = "test-hooks"))]
    _observations: Vec<buffer_observation::ReservationObservation>,
}

impl BufferReservation {
    fn acquire(bytes: usize, ledger: Arc<AtomicUsize>, limit: usize) -> Result<Self, Error> {
        // The device pool's ledger shares the one Metal budget with the pool's live leases (the residency's slabs,
        // the device blocks, relational work: 0266 §16), so a reservation counts them; room is made in the pool's one
        // order (idle buffers, then the residency's least recently used slabs).
        let pooled = Arc::ptr_eq(&ledger, &pool());
        // One account (Q203): the pool's leases and the admissions' reservations not yet leased (`bound_bytes`), as
        // `make_room` counts them; counting the leases alone, a reservation took room a stratum had been admitted to.
        // A test's ledger is its thread's, the pool's leases the process's: tests are bounded by `test_pool`.
        let bound = || if pooled && !cfg!(any(test, feature = "test-hooks")) { rerail_accel::gpu::shared::bound_bytes() } else { 0 };
        // Drawn from this thread's admission (a wave's workspace, admitted as a stratum's tables are): its room was
        // made when it was admitted.
        let drawn = pooled && rerail_accel::gpu::shared::draw_reserved(bytes);
        let take = || {
            let bound = if drawn { 0 } else { bound() };
            ledger.try_update(Ordering::AcqRel, Ordering::Acquire, |used| used.checked_add(bytes).filter(|next| drawn || *next + bound <= limit))
        };
        let previous = take()
            .or_else(|used| if pooled && rerail_accel::gpu::shared::make_room(bytes) { take() } else { Err(used) })
            .map_err(|used| {
                if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
                    eprintln!(
                        "TEMPO_METAL_RESERVATION_REJECT bytes={bytes} used={used} limit={limit}\n{}",
                        std::backtrace::Backtrace::force_capture()
                    );
                }
                rerail_prof::count("capacity.metal_pool_ledger", 1);
                Error::BackendCapacity
            })?;
        if pooled {
            // Idle buffers fit what the budget leaves now.
            rerail_accel::gpu::shared::trim_idle();
        }
        #[cfg(any(test, feature = "test-hooks"))]
        if pooled {
            test_pool::admit(bytes);
        }
        if pooled {
            pool_peak().fetch_max(previous + bytes, Ordering::AcqRel);
            if bytes >= 4 << 20 && std::env::var_os("RERAIL_BUFFERS").is_some() {
                let trace = std::backtrace::Backtrace::force_capture().to_string();
                let frames = trace
                    .lines()
                    .filter(|line| line.trim_start().starts_with("at ") && line.contains("code_rule_dsl"))
                    .take(3)
                    .map(|line| line.trim().trim_start_matches(|c: char| c.is_ascii_digit() || c == ':' || c == ' ').to_owned())
                    .collect::<Vec<_>>()
                    .join(" < ");
                eprintln!("buffer {} MB (alive {} MB): {frames}", bytes >> 20, (previous + bytes) >> 20);
            }
            launch::record_buffer_allocation(previous + bytes);
        }
        Ok(Self {
            bytes,
            ledger,
            #[cfg(any(test, feature = "test-hooks"))]
            admitted: pooled,
            #[cfg(any(test, feature = "test-hooks"))]
            _observations: if pooled {
                buffer_observation::reserve(bytes)
            } else {
                Vec::new()
            },
        })
    }

}

impl Drop for BufferReservation {
    fn drop(&mut self) {
        self.ledger.fetch_sub(self.bytes, Ordering::AcqRel);
        #[cfg(any(test, feature = "test-hooks"))]
        if self.admitted {
            test_pool::release(self.bytes);
        }
    }
}

/// A page-backed host allocation, the memory of a [`HostMapped`] buffer, which the device reads in place: repeated
/// dispatches do not repack sources, tasks, programs, or anchor nodes.
struct MappedRegion {
    pointer: NonNull<u8>,
    length: usize,
    _reservation: BufferReservation,
}

unsafe impl Send for MappedRegion {}
unsafe impl Sync for MappedRegion {}

impl MappedRegion {
    fn zeroed(byte_length: usize) -> Result<Self, Error> {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page <= 0 {
            return Err(Error::BackendFailure);
        }
        let page = page as usize;
        let wanted = byte_length.max(1);
        let length = wanted
            .checked_add(page - 1)
            .map(|value| value / page * page)
            .ok_or(Error::BackendCapacity)?;
        let reservation =
            BufferReservation::acquire(length, pool(), metal_buffer_limit())?;
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(Error::BackendFailure);
        }
        let pointer = NonNull::new(raw.cast::<u8>()).ok_or(Error::BackendFailure)?;
        Ok(Self {
            pointer,
            length,
            _reservation: reservation,
        })
    }

    fn new(bytes: &[u8]) -> Result<Self, Error> {
        let region = Self::zeroed(bytes.len())?;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), region.pointer.as_ptr(), bytes.len());
        }
        Ok(region)
    }
}

/// A buffer of the device layer's over an anonymous mapping of the host's (decision 0440 step C), on every
/// backend. Its whole length is reserved against the buffer budget, and
/// untouched pages stay unmapped, so it costs what is written to it. The kernels read and write it in place (the
/// device layer's `wrap_host_memory`, no copy). Its holder waits for every command that uses it before dropping it:
/// a command holds the buffer, not the mapping.
pub struct HostMapped {
    region: Arc<MappedRegion>,
    buffer: rerail_device::device::Buffer,
}

impl HostMapped {
    /// `count` values of `T`, zero (pages unwritten until used).
    #[track_caller]
    pub fn zeroed<T>(count: usize) -> Result<Self, Error> {
        let bytes = count.checked_mul(std::mem::size_of::<T>()).ok_or(Error::BackendCapacity)?;
        Self::over(MappedRegion::zeroed(bytes)?)
    }

    /// A buffer holding `values` (at least a page long).
    #[track_caller]
    pub fn from_slice<T: Copy>(values: &[T]) -> Result<Self, Error> {
        // SAFETY: the values' bytes, all initialized (`Copy` records of the batch's ABI).
        let bytes = unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), size_of_val(values)) };
        Self::over(MappedRegion::new(bytes)?)
    }

    fn over(region: MappedRegion) -> Result<Self, Error> {
        let device = crate::lane::device()?;
        // Metal's driver refuses past the OS's working set for the device; the budget is weighed where the
        // reservation was taken, never against the driver's count (Q203, `driver_refuses`).
        #[cfg(target_os = "macos")]
        if let Some(metal) = rerail_accel::gpu::shared::opened_device() {
            let st = state();
            let _allocation = st.allocation.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if driver_refuses(metal, region.length) {
                if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
                    eprintln!("TEMPO_HOST_MAPPED_REJECT bytes={} driver={}", region.length, driver_bytes(metal));
                }
                return Err(Error::BackendCapacity);
            }
        }
        // SAFETY: the mapping is held with the buffer, and its holder waits for the commands that use it.
        let buffer = unsafe { rerail_device::device::wrap_host_memory(device, region.pointer.as_ptr(), region.length) }.map_err(|e| {
            if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
                eprintln!("TEMPO_HOST_MAPPED_REJECT bytes={} error={e}", region.length);
            }
            Error::BackendCapacity
        })?;
        Ok(Self { region: Arc::new(region), buffer })
    }

    /// The buffer, as kernels bind it.
    pub fn buffer(&self) -> &rerail_device::device::Buffer {
        &self.buffer
    }

    /// Its bytes as the host writes them.
    pub fn host(&self) -> *mut u8 {
        self.region.pointer.as_ptr()
    }

    /// Its length in bytes (page-rounded).
    pub fn byte_length(&self) -> usize {
        self.region.length
    }
}

impl Drop for MappedRegion {
    fn drop(&mut self) {
        unsafe {
            libc::madvise(
                self.pointer.as_ptr().cast(),
                self.length,
                libc::MADV_DONTNEED,
            );
            libc::munmap(self.pointer.as_ptr().cast(), self.length);
        }
    }
}

/// An evaluation: its launch budget on this thread ([`launch::LaunchScope`]), which nested runs share. It is no lock:
/// executions run side by side, one encoding while another's command buffers run, each on device memory it holds
/// alone (a lease: the label passes' kept buffers, the program backend's workspaces, a run's own buffers), and what
/// they share (pipelines, libraries) is behind its own mutex. (Until 2026-11-17 it was also a process-wide lock,
/// copied from Tempo and held for a whole run, so no run could encode while another waited on the GPU.)
pub fn evaluation_scope() -> launch::LaunchScope {
    launch::LaunchScope::enter()
}

/// Releases the compiled pipelines and libraries (the residency's drop): the machine's; the store keeps them
/// (`rerail_accel::gpu::kernels`, decision 0395).
pub fn drop_pipelines() {
    rerail_accel::gpu::kernels::release();
    if let Ok(mut kernels) = crate::block_program::serial::kernels().0.lock() {
        kernels.clear();
    }
}

#[cfg(any(test, feature = "test-hooks"))]
pub fn pipeline_cache_len() -> usize {
    rerail_accel::gpu::kernels::held()
}

/// The kernels this thread has made, not found in the process's memory (on any backend: `rerail_accel`'s
/// `made_here`): a test's own, whatever other tests make.
#[cfg(any(test, feature = "test-hooks"))]
pub fn pipelines_compiled_here() -> usize {
    rerail_accel::gpu::kernels::made_here() as usize
}

#[cfg(test)]
mod memory_budget_tests {
    use super::*;

    /// Q203: the run's host pages wrapped for the device (row blocks, tapes: no-copy buffers) are the host budget's,
    /// not device memory the Metal budget bounds. With the driver holding them past the budget, a buffer within the
    /// budget is still given; the driver's figure refuses only past the OS's working set for the device.
    #[cfg(target_os = "macos")]
    #[test]
    fn host_pages_wrapped_past_the_budget_leave_the_budgets_buffers() {
        use metal::MTLResourceOptions;
        let Some(device) = crate::METAL_DEVICE.as_ref() else { return };
        test_pool::measure_driver();
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
        let len = (metal_buffer_limit() + (64 << 20)).div_ceil(page) * page;
        // An anonymous mapping, never touched: the host's pages, wrapped as a run's row blocks are.
        let host = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0) };
        assert_ne!(host, libc::MAP_FAILED);
        let wrap = device.new_buffer_with_bytes_no_copy(host.cast_const(), len as u64, MTLResourceOptions::StorageModeShared, None);
        assert!(driver_bytes(device) > metal_buffer_limit() as u64, "the driver counts the wrap: {} MB", driver_bytes(device) >> 20);
        let given = HostMapped::zeroed::<u8>(1 << 20);
        assert!(given.is_ok(), "a 1 MB buffer within the budget, the driver at {} MB of a {} MB budget", driver_bytes(device) >> 20, metal_buffer_limit() >> 20);
        drop(given);
        // Past the working set the driver's figure refuses.
        test_pool::set_working_set(Some(driver_bytes(device) + (1 << 19)));
        assert_eq!(HostMapped::zeroed::<u8>(1 << 20).err(), Some(Error::BackendCapacity), "past the working set");
        test_pool::set_working_set(None);
        drop(wrap);
        unsafe { libc::munmap(host, len) };
    }

    /// What relational work may still lease counts its own leases (held and idle in the pool), not only the
    /// buffer pool's reservations: a wave of device joins that checks it stops once its leases fill it.
    #[test]
    fn the_budget_left_counts_relational_leases() {
        if rerail_accel::gpu::join::device().is_err() {
            return;
        }
        let before = relational_bytes_left();
        let held = rerail_accel::gpu::shared::lease(32 << 20).unwrap();
        let during = relational_bytes_left();
        assert!(before - during >= 32 << 20 || during == 0, "a 32 MB lease leaves {} MB of {} MB", during >> 20, before >> 20);
        drop(held);
    }

    /// An observation counts the buffers made after it began, not one made before and dropped during it.
    #[test]
    fn operation_allocation_observation_excludes_unrelated_retirement() {
        if crate::lane::device().is_err() {
            return;
        }
        let _scope = evaluation_scope();
        let previous = HostMapped::zeroed::<u8>(65_536).unwrap();
        let observation = BufferObservation::enter();
        let output = HostMapped::zeroed::<u8>(16_384).unwrap();
        let bytes = output.byte_length();
        drop(previous);
        assert_eq!(observation.live_bytes(), bytes);
        drop(output);
        assert_eq!(observation.live_bytes(), 0);
    }

    /// Nested observations: the inner counts its own buffers, the outer both, and each falls as they drop.
    #[test]
    fn operation_allocation_observations_nest() {
        if crate::lane::device().is_err() {
            return;
        }
        let _scope = evaluation_scope();
        let outer = BufferObservation::enter();
        let first = HostMapped::zeroed::<u8>(16_384).unwrap();
        let first_bytes = first.byte_length();
        let inner = BufferObservation::enter();
        let second = HostMapped::zeroed::<u8>(32_768).unwrap();
        let second_bytes = second.byte_length();
        assert_eq!(inner.live_bytes(), second_bytes);
        assert_eq!(outer.live_bytes(), first_bytes + second_bytes);
        drop(second);
        assert_eq!(inner.live_bytes(), 0);
        assert_eq!(outer.live_bytes(), first_bytes);
        drop(first);
        assert_eq!(outer.live_bytes(), 0);
    }

    #[test]
    fn reservations_enforce_aggregate_limit_and_release_on_drop() {
        let ledger = Arc::new(AtomicUsize::new(0));
        let first = BufferReservation::acquire(60, Arc::clone(&ledger), 100).unwrap();
        assert!(matches!(
            BufferReservation::acquire(41, Arc::clone(&ledger), 100),
            Err(Error::BackendCapacity)
        ));
        let second = BufferReservation::acquire(40, Arc::clone(&ledger), 100).unwrap();
        assert!(matches!(
            BufferReservation::acquire(usize::MAX, Arc::clone(&ledger), 100),
            Err(Error::BackendCapacity)
        ));
        assert_eq!(ledger.load(Ordering::Acquire), 100);
        drop(first);
        let third = BufferReservation::acquire(60, Arc::clone(&ledger), 100).unwrap();
        drop((second, third));
        assert_eq!(ledger.load(Ordering::Acquire), 0);
    }

    #[test]
    fn oversized_mapping_is_rejected_before_allocation() {
        assert!(matches!(
            MappedRegion::zeroed(metal_buffer_limit() + 1),
            Err(Error::BackendCapacity)
        ));
        assert!(matches!(
            MappedRegion::zeroed(usize::MAX),
            Err(Error::BackendCapacity)
        ));
        assert!(matches!(HostMapped::zeroed::<u8>(metal_buffer_limit() + 1), Err(Error::BackendCapacity)));
        assert!(matches!(HostMapped::zeroed::<u32>(usize::MAX), Err(Error::BackendCapacity)));
    }
}
