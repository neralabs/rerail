use crate::Error;
#[allow(unused_imports)]
use crate::launch::{CheckedComputeEncoder, commit_checked, wait_checked};
use metal::{
    Buffer, CompileOptions, ComputePipelineState, Device, MTLResourceOptions,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::mem::size_of_val;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use crate::launch;

// The device pool: every Metal buffer alive at once, reserved before its mmap, copy or Metal
// allocation (page alignment and empty buffers included). Its size is the device's share of the
// process's 500 MB (02-shape §6: the resident tapes, 128-192 MB, and the plan-sized set, lattice
// and output pools, 32-64 MB), in place of a 2 GB guard (list step 13). A plan's buffers are
// sized from it (counted, not worst-case: `label_passes`), and a batch that does not fit is
// refused as `BackendCapacity` and split (`waves::matches::labelled_metal`). S7: peak 248 MB.
pub const METAL_BUFFER_LIMIT: usize = 256 << 20;
#[cfg(not(any(test, feature = "test-hooks")))]
static METAL_BUFFER_BYTES: AtomicUsize = AtomicUsize::new(0);
/// The most `METAL_BUFFER_BYTES` has held.
#[cfg(not(any(test, feature = "test-hooks")))]
static METAL_BUFFER_PEAK: AtomicUsize = AtomicUsize::new(0);

/// The pool's ledger: the bytes reserved now.
#[cfg(not(any(test, feature = "test-hooks")))]
fn pool() -> &'static AtomicUsize {
    &METAL_BUFFER_BYTES
}

/// The most the pool's ledger has held.
#[cfg(not(any(test, feature = "test-hooks")))]
fn pool_peak() -> &'static AtomicUsize {
    &METAL_BUFFER_PEAK
}

/// What the driver has allocated for the device, beside the pool's own reservations.
#[cfg(not(any(test, feature = "test-hooks")))]
fn driver_bytes(device: &metal::DeviceRef) -> u64 {
    device.current_allocated_size()
}

#[cfg(any(test, feature = "test-hooks"))]
use crate::test_pool;
#[cfg(any(test, feature = "test-hooks"))]
use test_pool::{driver_bytes, pool, pool_peak};

/// The Metal buffer budget, in bytes.
pub fn buffer_limit() -> usize {
    METAL_BUFFER_LIMIT
}

/// The bytes of the Metal buffer budget not reserved now.
/// The bytes the device's buffers hold now, of `METAL_BUFFER_LIMIT`: a user of them that can wait (the logic
/// engine's device joins) keeps within what is left.
pub fn reserved() -> usize {
    pool().load(Ordering::Acquire)
}

pub fn buffer_free() -> usize {
    METAL_BUFFER_LIMIT.saturating_sub(pool().load(Ordering::Acquire))
}

/// The bytes of Metal buffers alive now (their reservations), in MB.
pub fn reserved_mb() -> usize {
    pool().load(Ordering::Acquire) >> 20
}

/// The most bytes of Metal buffers alive at once, in MB.
pub fn peak_mb() -> usize {
    pool_peak().load(Ordering::Acquire) >> 20
}

/// What the Metal driver says it has allocated for the device, in MB.
pub fn driver_mb() -> usize {
    crate::METAL_DEVICE.as_ref().map_or(0, |device| (device.current_allocated_size() >> 20) as usize)
}
static METAL_PRIVATE_BYTES: AtomicUsize = AtomicUsize::new(0);
static METAL_ALLOCATION_LOCK: Mutex<()> = Mutex::new(());
static PRIVATE_RESERVATION_KEY: u8 = 0;
const METAL_UPLOAD_CHUNK_BYTES: usize = 8 * 1024 * 1024;

pub fn available_buffer_bytes(device: &Device) -> usize {
    let reserved = pool().load(Ordering::Acquire);
    let driver = usize::try_from(driver_bytes(device)).unwrap_or(usize::MAX);
    METAL_BUFFER_LIMIT.saturating_sub(reserved.max(driver))
}

pub fn buffer_usage() -> (usize, usize) {
    (
        pool().load(Ordering::Acquire),
        METAL_PRIVATE_BYTES.load(Ordering::Acquire),
    )
}

#[cfg(any(test, feature = "test-hooks"))]
use crate::buffer_observation;
#[cfg(any(test, feature = "test-hooks"))]
pub use buffer_observation::BufferObservation;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_setAssociatedObject(
        object: *mut std::ffi::c_void,
        key: *const std::ffi::c_void,
        value: *mut std::ffi::c_void,
        policy: usize,
    );
}

fn retain_buffer_owner<T: 'static>(buffer: &metal::BufferRef, owner: T) {
    let retention = block::ConcreteBlock::new(move || {
        let _ = &owner;
    })
    .copy();
    let retained_block: &block::Block<(), ()> = &retention;
    unsafe {
        objc_setAssociatedObject(
            std::ptr::from_ref(buffer).cast_mut().cast(),
            std::ptr::from_ref(&PRIVATE_RESERVATION_KEY).cast(),
            std::ptr::from_ref(retained_block).cast_mut().cast(),
            3,
        );
    }
}

struct BufferReservation {
    bytes: usize,
    ledger: &'static AtomicUsize,
    private: bool,
    /// Admitted to the tests' weighted bound (`test_pool`).
    #[cfg(any(test, feature = "test-hooks"))]
    admitted: bool,
    #[cfg(any(test, feature = "test-hooks"))]
    _observations: Vec<buffer_observation::ReservationObservation>,
}

impl BufferReservation {
    fn acquire(bytes: usize, ledger: &'static AtomicUsize, limit: usize) -> Result<Self, Error> {
        let previous = ledger
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= limit)
            })
            .map_err(|used| {
                if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
                    eprintln!(
                        "TEMPO_METAL_RESERVATION_REJECT bytes={bytes} used={used} limit={limit}\n{}",
                        std::backtrace::Backtrace::force_capture()
                    );
                }
                Error::BackendCapacity
            })?;
        let pooled = std::ptr::eq(ledger, pool());
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
            private: false,
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

    fn private(mut self) -> Self {
        METAL_PRIVATE_BYTES.fetch_add(self.bytes, Ordering::AcqRel);
        self.private = true;
        self
    }
}

impl Drop for BufferReservation {
    fn drop(&mut self) {
        self.ledger.fetch_sub(self.bytes, Ordering::AcqRel);
        #[cfg(any(test, feature = "test-hooks"))]
        if self.admitted {
            test_pool::release(self.bytes);
        }
        if self.private {
            METAL_PRIVATE_BYTES.fetch_sub(self.bytes, Ordering::AcqRel);
        }
    }
}

/// A page-backed host allocation retained for the complete lifetime of a no-copy Metal buffer.
/// Apple Silicon's shared storage can read this allocation directly; repeated dispatches do not
/// repack sources, tasks, programs, or anchor nodes.
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
            BufferReservation::acquire(length, pool(), METAL_BUFFER_LIMIT)?;
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

pub struct MappedBuffer {
    // The Metal deallocator retains the mapping and its budget reservation.
    pub buffer: Buffer,
}

// One bounded staging allocation serves the complete streamed corpus. Pending
// copies retain their destination until the checked command has completed.
pub struct PrivateInputUpload {
    queue: metal::CommandQueue,
    staging: MappedBuffer,
    used: usize,
    copies: Vec<(Buffer, u64, u64, u64)>,
}

impl PrivateInputUpload {
    pub fn new(device: &Device) -> Result<Self, Error> {
        Ok(Self {
            queue: device.new_command_queue(),
            staging: MappedBuffer::zeroed::<u8>(device, METAL_UPLOAD_CHUNK_BYTES)?,
            used: 0,
            copies: Vec::new(),
        })
    }

    pub fn write<T: Copy>(
        &mut self,
        target: &MappedBuffer,
        offset: usize,
        values: &[T],
    ) -> Result<(), Error> {
        let offset = offset
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(Error::BackendCapacity)?;
        let bytes = unsafe {
            std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), size_of_val(values))
        };
        if offset % 4 != 0
            || bytes.len() % 4 != 0
            || offset
                .checked_add(bytes.len())
                .is_none_or(|end| end as u64 > target.buffer.length())
        {
            return Err(Error::BackendCapacity);
        }
        let mut copied = 0;
        while copied < bytes.len() {
            if self.used == METAL_UPLOAD_CHUNK_BYTES {
                self.finish()?;
            }
            let count = (bytes.len() - copied).min(METAL_UPLOAD_CHUNK_BYTES - self.used);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr().add(copied),
                    self.staging.buffer.contents().cast::<u8>().add(self.used),
                    count,
                );
            }
            self.copies.push((
                target.buffer.to_owned(),
                self.used as u64,
                (offset + copied) as u64,
                count as u64,
            ));
            self.used += count;
            copied += count;
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), Error> {
        if self.copies.is_empty() {
            return Ok(());
        }
        metal::objc::rc::autoreleasepool(|| {
            let command = self.queue.new_command_buffer();
            command.set_label("Tempo bounded resident input upload");
            let blit = command.new_blit_command_encoder();
            for (target, source_offset, target_offset, count) in &self.copies {
                blit.copy_from_buffer(
                    &self.staging.buffer,
                    *source_offset,
                    target,
                    *target_offset,
                    *count,
                );
            }
            blit.end_encoding();
            command.commit();
            command.wait_until_completed();
            if command.status() != metal::MTLCommandBufferStatus::Completed {
                return Err(Error::BackendFailure);
            }
            Ok(())
        })?;
        self.copies.clear();
        self.used = 0;
        Ok(())
    }
}

impl MappedBuffer {
    /// A residency slab (PIAT §20.2, §20.4): `length` bytes, page-rounded, of an anonymous mapping
    /// with one retained no-copy shared buffer over it. Untouched pages stay unmapped, so a slab
    /// costs what is written to it; its whole length is reserved against the buffer budget.
    #[track_caller]
    pub fn slab(length: usize) -> Result<Self, Error> {
        let device = crate::METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
        Self::from_mapping(device, Arc::new(MappedRegion::zeroed(length)?))
    }

    /// The buffer's bytes as the host writes them.
    pub fn host(&self) -> *mut u8 {
        self.buffer.contents().cast()
    }

    /// The buffer's length in bytes.
    pub fn byte_length(&self) -> usize {
        self.buffer.length() as usize
    }

    /// Another owner of `buffer` (Metal retains it), as a kept buffer is handed between runs.
    pub fn wrap(buffer: Buffer) -> Self {
        Self { buffer }
    }

    #[track_caller]
    pub fn new(device: &Device, bytes: &[u8]) -> Result<Self, Error> {
        let mapping = Arc::new(MappedRegion::new(bytes)?);
        Self::from_mapping(device, mapping)
    }

    #[track_caller]
    fn from_mapping(device: &Device, mapping: Arc<MappedRegion>) -> Result<Self, Error> {
        if rerail_prof::on() && mapping.length >= 64 << 10 {
            rerail_prof::count_named(format!("alloc_kb.{}", std::panic::Location::caller()), mapping.length >> 10);
        }
        // A lock over no data: a panic elsewhere while it was held leaves nothing to distrust.
        let _allocation = METAL_ALLOCATION_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if mapping.length as u64 > device.max_buffer_length()
            || driver_bytes(device)
                .saturating_add(mapping.length as u64)
                > METAL_BUFFER_LIMIT as u64
        {
            if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
                eprintln!(
                    "TEMPO_METAL_DRIVER_REJECT bytes={} reserved={} driver={} maximumBuffer={}",
                    mapping.length,
                    pool().load(Ordering::Acquire),
                    device.current_allocated_size(),
                    device.max_buffer_length()
                );
            }
            return Err(Error::BackendCapacity);
        }
        // The command buffer can retain this resource after the Rust wrapper drops. The Metal
        // deallocator owns both the mapping and its reservation until the final reference dies.
        let retained = Arc::clone(&mapping);
        let deallocator = block::ConcreteBlock::new(move |_: *const std::ffi::c_void, _: u64| {
            let _ = &retained;
        })
        .copy();
        let buffer = device.new_buffer_with_bytes_no_copy(
            mapping.pointer.as_ptr().cast(),
            mapping.length as u64,
            MTLResourceOptions::StorageModeShared,
            Some(&deallocator),
        );
        // Validation wrappers can release the callback before their Metal buffer.
        // The object itself must also retain the backing mapping and reservation.
        retain_buffer_owner(&buffer, Arc::clone(&mapping));
        Ok(Self { buffer })
    }

    #[track_caller]
    pub fn from_slice<T: Copy>(device: &Device, values: &[T]) -> Result<Self, Error> {
        let bytes = unsafe {
            std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), size_of_val(values))
        };
        Self::new(device, bytes)
    }

    #[track_caller]
    fn owned_allocation(
        device: &Device,
        length: usize,
        reservation: Arc<BufferReservation>,
        storage: MTLResourceOptions,
    ) -> Result<Self, Error> {
        // A lock over no data: a panic elsewhere while it was held leaves nothing to distrust.
        let _allocation = METAL_ALLOCATION_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if length as u64 > device.max_buffer_length()
            || driver_bytes(device)
                .saturating_add(length as u64)
                > METAL_BUFFER_LIMIT as u64
        {
            if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
                eprintln!(
                    "TEMPO_METAL_DRIVER_REJECT bytes={length} reserved={} driver={} maximumBuffer={} caller={}",
                    pool().load(Ordering::Acquire),
                    device.current_allocated_size(),
                    device.max_buffer_length(),
                    std::panic::Location::caller()
                );
            }
            return Err(Error::BackendCapacity);
        }
        if rerail_prof::on() && length >= 64 << 10 {
            rerail_prof::count_named(format!("alloc_kb.{}", std::panic::Location::caller()), length >> 10);
        }
        let buffer = device.new_buffer(length as u64, storage);
        // Private buffers have no bytes-no-copy deallocator. An associated copied Objective-C
        // block holds the reservation until the Metal object loses its final reference.
        retain_buffer_owner(&buffer, reservation);
        Ok(Self { buffer })
    }

    pub fn private_from_slice<T: Copy>(
        device: &Device,
        values: &[T],
    ) -> Result<Self, Error> {
        let bytes = unsafe {
            std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), size_of_val(values))
        };
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page <= 0 {
            return Err(Error::BackendFailure);
        }
        let page = page as usize;
        let length = bytes
            .len()
            .max(1)
            .checked_add(page - 1)
            .map(|length| length / page * page)
            .ok_or(Error::BackendCapacity)?;
        let reservation = Arc::new(
            BufferReservation::acquire(length, pool(), METAL_BUFFER_LIMIT)?.private(),
        );
        let target = Self::owned_allocation(
            device,
            length,
            reservation,
            MTLResourceOptions::StorageModePrivate,
        )?;
        let queue = device.new_command_queue();
        for (chunk, input) in bytes.chunks(METAL_UPLOAD_CHUNK_BYTES).enumerate() {
            metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
                let staging = Self::new(device, input)?;
                let command = queue.new_command_buffer();
                let blit = command.new_blit_command_encoder();
                let copied = input
                    .len()
                    .checked_add(3)
                    .map(|length| length / 4 * 4)
                    .ok_or(Error::BackendCapacity)?;
                blit.copy_from_buffer(
                    &staging.buffer,
                    0,
                    &target.buffer,
                    (chunk * METAL_UPLOAD_CHUNK_BYTES) as u64,
                    copied as u64,
                );
                blit.end_encoding();
                command.commit();
                command.wait_until_completed();
                if command.status() != metal::MTLCommandBufferStatus::Completed {
                    return Err(Error::BackendFailure);
                }
                Ok(())
            })?;
        }
        Ok(target)
    }

    pub fn private_capacity<T: Copy>(device: &Device, count: usize) -> Result<Self, Error> {
        let bytes = count
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(Error::BackendCapacity)?;
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page <= 0 {
            return Err(Error::BackendFailure);
        }
        let page = page as usize;
        let length = bytes
            .max(1)
            .checked_add(page - 1)
            .map(|length| length / page * page)
            .ok_or(Error::BackendCapacity)?;
        let reservation = Arc::new(
            BufferReservation::acquire(length, pool(), METAL_BUFFER_LIMIT)?.private(),
        );
        Self::owned_allocation(
            device,
            length,
            reservation,
            MTLResourceOptions::StorageModePrivate,
        )
    }

    /// A shared buffer of `count` records that the host never writes (a workspace's arenas,
    /// which the GPU writes before it reads them): Metal zero-fills a new buffer, and its pages
    /// are committed only as the GPU touches them, where `zeroed` commits every page (O28).
    #[track_caller]
    pub fn untouched<T>(device: &Device, count: usize) -> Result<Self, Error> {
        let byte_length = count
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(Error::BackendCapacity)?;
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page <= 0 {
            return Err(Error::BackendFailure);
        }
        let page = page as usize;
        let length = byte_length
            .max(1)
            .checked_add(page - 1)
            .map(|length| length / page * page)
            .ok_or(Error::BackendCapacity)?;
        let reservation = Arc::new(BufferReservation::acquire(
            length,
            pool(),
            METAL_BUFFER_LIMIT,
        )?);
        Self::owned_allocation(device, length, reservation, MTLResourceOptions::StorageModeShared)
    }

    #[track_caller]
    pub fn zeroed<T>(device: &Device, count: usize) -> Result<Self, Error> {
        let byte_length = count
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(Error::BackendCapacity)?;
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page <= 0 {
            return Err(Error::BackendFailure);
        }
        let page = page as usize;
        let length = byte_length
            .max(1)
            .checked_add(page - 1)
            .map(|length| length / page * page)
            .ok_or(Error::BackendCapacity)?;
        let reservation = Arc::new(BufferReservation::acquire(
            length,
            pool(),
            METAL_BUFFER_LIMIT,
        )?);
        let buffer = Self::owned_allocation(
            device,
            length,
            reservation,
            MTLResourceOptions::StorageModeShared,
        )?;
        unsafe {
            std::ptr::write_bytes(buffer.buffer.contents().cast::<u8>(), 0, length);
        }
        Ok(buffer)
    }
}

static METAL_EXECUTION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub struct MetalExecutionGuard {
    _lock: MutexGuard<'static, ()>,
    _launch_scope: launch::LaunchScope,
}

pub fn metal_execution_guard() -> Result<MetalExecutionGuard, Error> {
    // A lock over no data (it makes Metal executions serial): a caller that panicked while holding it, a failed
    // test's assertion among them, leaves nothing to distrust, so the next execution goes ahead.
    let lock = METAL_EXECUTION_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(MetalExecutionGuard {
        _lock: lock,
        _launch_scope: launch::LaunchScope::enter(),
    })
}

// Process-wide, so the residency's drop (`residency::slabs`, two minutes after a full execution)
// releases every compiled pipeline with the slabs.
static PIPELINES: LazyLock<Mutex<HashMap<[u8; 32], ComputePipelineState>>> = LazyLock::new(Default::default);
static LIBRARIES: LazyLock<Mutex<HashMap<[u8; 32], metal::Library>>> = LazyLock::new(Default::default);

/// Releases the compiled pipelines and libraries (the residency's drop).
pub fn drop_pipelines() {
    if let Ok(mut pipelines) = PIPELINES.lock() {
        pipelines.clear();
    }
    if let Ok(mut libraries) = LIBRARIES.lock() {
        libraries.clear();
    }
}

pub fn pipeline_named(source: &str, name: &str) -> Result<ComputePipelineState, Error> {
    pipeline_named_constant(source, name, None)
}

pub fn pipeline_named_constant(
    source: &str,
    name: &str,
    constant: Option<u32>,
) -> Result<ComputePipelineState, Error> {
    let device = Device::system_default().ok_or(Error::UnsupportedBackend)?;
    let mut digest = Sha256::new();
    digest.update(device.registry_id().to_le_bytes());
    digest.update(name.as_bytes());
    digest.update([0]);
    if let Some(value) = constant {
        digest.update([1]);
        digest.update(value.to_le_bytes());
    } else {
        digest.update([0]);
    }
    digest.update(source.as_bytes());
    let key: [u8; 32] = digest.finalize().into();
    if let Some(cached) = PIPELINES.lock().map_err(|_| Error::BackendFailure)?.get(&key).cloned() {
        return Ok(cached);
    }
    let mut library_digest = Sha256::new();
    library_digest.update(device.registry_id().to_le_bytes());
    library_digest.update(source.as_bytes());
    let library_key: [u8; 32] = library_digest.finalize().into();
    let cached = LIBRARIES.lock().map_err(|_| Error::BackendFailure)?.get(&library_key).cloned();
    let library = if let Some(library) = cached {
        library
    } else {
        let library = device
            .new_library_with_source(source, &CompileOptions::new())
            .map_err(|error| {
                eprintln!("Metal library compilation failed: {error}");
                Error::BackendFailure
            })?;
        if let Ok(mut libraries) = LIBRARIES.lock() {
            if libraries.len() >= 16
                && let Some(oldest) = libraries.keys().next().copied()
            {
                libraries.remove(&oldest);
            }
            libraries.insert(library_key, library.clone());
        }
        library
    };
    let constants = constant.map(|value| {
        let constants = metal::FunctionConstantValues::new();
        constants.set_constant_value_at_index(
            (&value as *const u32).cast(),
            metal::MTLDataType::UInt,
            0,
        );
        constants
    });
    let function = library
        .get_function(name, constants)
        .map_err(|_| Error::BackendFailure)?;
    let descriptor = metal::ComputePipelineDescriptor::new();
    descriptor.set_label(name);
    descriptor.set_compute_function(Some(&function));
    let pipeline = device
        .new_compute_pipeline_state(&descriptor)
        .map_err(|_| Error::BackendFailure)?;
    if let Ok(mut pipelines) = PIPELINES.lock() {
        if pipelines.len() >= 512
            && let Some(oldest) = pipelines.keys().next().copied()
        {
            pipelines.remove(&oldest);
        }
        pipelines.insert(key, pipeline.clone());
    }
    #[cfg(any(test, feature = "test-hooks"))]
    COMPILED_HERE.set(COMPILED_HERE.get() + 1);
    Ok(pipeline)
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Pipelines this thread compiled (the process's cache is shared with every other test's).
    static COMPILED_HERE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-hooks"))]
pub fn pipeline_cache_len() -> usize {
    PIPELINES.lock().map_or(0, |pipelines| pipelines.len())
}

/// The pipelines this thread has compiled, not found in the cache: a test's own, whatever other tests compile.
#[cfg(any(test, feature = "test-hooks"))]
pub fn pipelines_compiled_here() -> usize {
    COMPILED_HERE.get()
}

#[cfg(test)]
mod execution_lock_tests {
    use super::*;

    #[test]
    fn metal_execution_is_process_serial() {
        let guard = metal_execution_guard().unwrap();
        assert!(matches!(
            METAL_EXECUTION_LOCK.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ));
        drop(guard);
        assert!(METAL_EXECUTION_LOCK.try_lock().is_ok());
    }
}

#[cfg(test)]
mod memory_budget_tests {
    use super::*;

    #[test]
    fn operation_allocation_observation_excludes_unrelated_retirement() {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let previous = MappedBuffer::zeroed::<u8>(&device, 65_536).unwrap();
        let observation = BufferObservation::enter();
        let output = MappedBuffer::zeroed::<u8>(&device, 16_384).unwrap();
        let bytes = output.buffer.length() as usize;
        let retained = output.buffer.clone();
        drop(previous);
        assert_eq!(observation.live_bytes(), bytes);
        drop(output);
        assert_eq!(observation.live_bytes(), bytes);
        drop(retained);
        assert_eq!(observation.live_bytes(), 0);
    }

    #[test]
    fn operation_allocation_observation_tracks_retained_owners_on_other_threads() {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let outer = BufferObservation::enter();
        let first = MappedBuffer::zeroed::<u8>(&device, 16_384).unwrap();
        let first_bytes = first.buffer.length() as usize;
        let inner = BufferObservation::enter();
        let second = MappedBuffer::zeroed::<u8>(&device, 32_768).unwrap();
        let second_bytes = second.buffer.length() as usize;
        assert_eq!(inner.live_bytes(), second_bytes);
        assert_eq!(outer.live_bytes(), first_bytes + second_bytes);
        let retained = second.buffer.clone();
        drop(second);
        let (release, gate) = std::sync::mpsc::channel();
        let owner = std::thread::spawn(move || {
            gate.recv().unwrap();
            drop(retained);
        });
        assert_eq!(inner.live_bytes(), second_bytes);
        release.send(()).unwrap();
        assert_eq!(inner.wait_for_live_bytes(0), 0);
        owner.join().unwrap();
        assert_eq!(outer.live_bytes(), first_bytes);
        drop(first);
        assert_eq!(outer.live_bytes(), 0);
    }

    #[test]
    fn reservations_enforce_aggregate_limit_and_release_on_drop() {
        static LEDGER: AtomicUsize = AtomicUsize::new(0);
        let first = BufferReservation::acquire(60, &LEDGER, 100).unwrap();
        assert!(matches!(
            BufferReservation::acquire(41, &LEDGER, 100),
            Err(Error::BackendCapacity)
        ));
        let second = BufferReservation::acquire(40, &LEDGER, 100).unwrap();
        assert!(matches!(
            BufferReservation::acquire(usize::MAX, &LEDGER, 100),
            Err(Error::BackendCapacity)
        ));
        assert_eq!(LEDGER.load(Ordering::Acquire), 100);
        drop(first);
        let third = BufferReservation::acquire(60, &LEDGER, 100).unwrap();
        drop((second, third));
        assert_eq!(LEDGER.load(Ordering::Acquire), 0);
    }

    #[test]
    fn oversized_mapping_is_rejected_before_allocation() {
        assert!(matches!(
            MappedRegion::zeroed(METAL_BUFFER_LIMIT + 1),
            Err(Error::BackendCapacity)
        ));
        assert!(matches!(
            MappedRegion::zeroed(usize::MAX),
            Err(Error::BackendCapacity)
        ));
        let device = Device::system_default().unwrap();
        assert!(matches!(
            MappedBuffer::zeroed::<u8>(&device, METAL_BUFFER_LIMIT + 1),
            Err(Error::BackendCapacity)
        ));
        assert!(matches!(
            MappedBuffer::zeroed::<u32>(&device, usize::MAX),
            Err(Error::BackendCapacity)
        ));
    }

    #[test]
    fn private_upload_copies_multiple_chunks_and_odd_tail_exactly() {
        let device = Device::system_default().unwrap();
        let bytes = (0..METAL_UPLOAD_CHUNK_BYTES + 37)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>();
        let uploaded = MappedBuffer::private_from_slice(&device, &bytes).unwrap();
        assert_eq!(
            uploaded.buffer.storage_mode(),
            metal::MTLStorageMode::Private
        );
        let readback = MappedBuffer::zeroed::<u8>(&device, bytes.len()).unwrap();
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let blit = command.new_blit_command_encoder();
        blit.copy_from_buffer(
            &uploaded.buffer,
            0,
            &readback.buffer,
            0,
            (bytes.len().div_ceil(4) * 4) as u64,
        );
        blit.end_encoding();
        command.commit();
        command.wait_until_completed();
        assert_eq!(command.status(), metal::MTLCommandBufferStatus::Completed);
        assert_eq!(
            unsafe {
                std::slice::from_raw_parts(readback.buffer.contents().cast::<u8>(), bytes.len())
            },
            bytes
        );
    }

    #[test]
    fn batched_private_upload_preserves_offsets_across_staging_reuse() {
        let device = Device::system_default().unwrap();
        let values = (0..(METAL_UPLOAD_CHUNK_BYTES / 4 + 37))
            .map(|index| (index as u32).wrapping_mul(7919))
            .collect::<Vec<_>>();
        let target = MappedBuffer::private_capacity::<u32>(&device, values.len()).unwrap();
        let other = MappedBuffer::private_capacity::<u32>(&device, 3).unwrap();
        let mut upload = PrivateInputUpload::new(&device).unwrap();
        upload.write(&target, 0, &values[..11]).unwrap();
        upload.write(&other, 0, &[31u32, 41, 59]).unwrap();
        upload.write(&target, 11, &values[11..]).unwrap();
        upload.finish().unwrap();
        assert_eq!(upload.used, 0);
        assert!(upload.copies.is_empty());
        // The allocation is page aligned; reject an offset beyond that allocation.
        assert!(matches!(
            upload.write(&other, other.buffer.length() as usize / 4, &[1u32]),
            Err(Error::BackendCapacity)
        ));
        for (source, expected) in [(&target, values.as_slice()), (&other, &[31u32, 41, 59])] {
            let readback = MappedBuffer::zeroed::<u32>(&device, expected.len()).unwrap();
            metal::objc::rc::autoreleasepool(|| {
                let queue = device.new_command_queue();
                let command = queue.new_command_buffer();
                let blit = command.new_blit_command_encoder();
                blit.copy_from_buffer(
                    &source.buffer,
                    0,
                    &readback.buffer,
                    0,
                    std::mem::size_of_val(expected) as u64,
                );
                blit.end_encoding();
                command.commit();
                command.wait_until_completed();
                assert_eq!(command.status(), metal::MTLCommandBufferStatus::Completed);
            });
            assert_eq!(
                unsafe {
                    std::slice::from_raw_parts(
                        readback.buffer.contents().cast::<u32>(),
                        expected.len(),
                    )
                },
                expected
            );
        }
    }

    #[test]
    fn owned_metal_reference_keeps_its_budget_reservation() {
        static LEDGER: AtomicUsize = AtomicUsize::new(0);
        let device = Device::system_default().unwrap();
        for storage in [
            MTLResourceOptions::StorageModePrivate,
            MTLResourceOptions::StorageModeShared,
        ] {
            let reservation =
                Arc::new(BufferReservation::acquire(16_384, &LEDGER, 16_384).unwrap());
            let weak = Arc::downgrade(&reservation);
            let buffer =
                MappedBuffer::owned_allocation(&device, 16_384, reservation, storage).unwrap();
            let retained = buffer.buffer.clone();
            drop(buffer);
            assert!(weak.upgrade().is_some());
            assert_eq!(LEDGER.load(Ordering::Acquire), 16_384);
            drop(retained);
            assert!(weak.upgrade().is_none());
            assert_eq!(LEDGER.load(Ordering::Acquire), 0);
        }
    }

    #[test]
    fn retained_metal_reference_keeps_its_mapping_and_reservation() {
        let device = Device::system_default().unwrap();
        let mapping = Arc::new(MappedRegion::zeroed(4).unwrap());
        let weak = Arc::downgrade(&mapping);
        let buffer = MappedBuffer::from_mapping(&device, mapping).unwrap();
        let retained = buffer.buffer.clone();
        drop(buffer);
        assert!(weak.upgrade().is_some());
        unsafe {
            retained.contents().cast::<u32>().write(42);
        }
        drop(retained);
        assert!(weak.upgrade().is_none());
    }
}
