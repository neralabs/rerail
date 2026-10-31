use super::{EMIT, Error, KIND, Program, TEXT, TextPredicate, split};
use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::transducer::packed::metal_backend::launch::{
    CheckedComputeEncoder, commit_checked, wait_checked,
};
use metal::{
    Buffer, CommandQueue, CompileOptions, ComputePipelineState, Device, MTLResourceOptions, MTLSize,
};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::mem::size_of_val;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

#[path = "metal_launch.rs"]
pub(crate) mod launch;

// This is a process-wide buffer budget, shared by anchor selection, execution, and caches.
// Reserve before mmap/copy/Metal allocation, including page alignment and empty buffers.
pub(super) const METAL_BUFFER_LIMIT: usize = 2_000_000_000;
static METAL_BUFFER_BYTES: AtomicUsize = AtomicUsize::new(0);

/// The bytes of Metal buffers alive now (their reservations), in MB.
pub(in crate::code_rule_dsl) fn reserved_mb() -> usize {
    METAL_BUFFER_BYTES.load(Ordering::Acquire) >> 20
}

/// What the Metal driver says it has allocated for the device, in MB.
pub(in crate::code_rule_dsl) fn driver_mb() -> usize {
    super::metal_program_backend::METAL_DEVICE.as_ref().map_or(0, |device| (device.current_allocated_size() >> 20) as usize)
}
static METAL_PRIVATE_BYTES: AtomicUsize = AtomicUsize::new(0);
static METAL_ALLOCATION_LOCK: Mutex<()> = Mutex::new(());
static PRIVATE_RESERVATION_KEY: u8 = 0;
const METAL_UPLOAD_CHUNK_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn available_buffer_bytes(device: &Device) -> usize {
    let reserved = METAL_BUFFER_BYTES.load(Ordering::Acquire);
    let driver = usize::try_from(device.current_allocated_size()).unwrap_or(usize::MAX);
    METAL_BUFFER_LIMIT.saturating_sub(reserved.max(driver))
}

pub(super) fn buffer_usage() -> (usize, usize) {
    (
        METAL_BUFFER_BYTES.load(Ordering::Acquire),
        METAL_PRIVATE_BYTES.load(Ordering::Acquire),
    )
}

#[cfg(test)]
#[path = "metal_buffer_observation.rs"]
mod buffer_observation;
#[cfg(test)]
pub(super) use buffer_observation::BufferObservation;

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
    #[cfg(test)]
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
        if std::ptr::eq(ledger, &METAL_BUFFER_BYTES) {
            launch::record_buffer_allocation(previous + bytes);
        }
        Ok(Self {
            bytes,
            ledger,
            private: false,
            #[cfg(test)]
            _observations: if std::ptr::eq(ledger, &METAL_BUFFER_BYTES) {
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
        if self.private {
            METAL_PRIVATE_BYTES.fetch_sub(self.bytes, Ordering::AcqRel);
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AnchorHit {
    node: u32,
    group: u32,
}

#[derive(Default)]
struct LiteralTrie {
    group: Option<u32>,
    children: BTreeMap<u8, LiteralTrie>,
}

impl LiteralTrie {
    fn insert(&mut self, text: &[u8], group: u32) {
        let mut node = self;
        for &byte in text {
            node = node.children.entry(byte).or_default();
        }
        debug_assert!(node.group.is_none_or(|existing| existing == group));
        node.group = Some(group);
    }

    fn emit(&self, depth: usize, source: &mut String) {
        if let Some(group) = self.group {
            source.push_str(&format!("group={group}u;"));
        }
        if self.children.is_empty() {
            return;
        }
        source.push_str(&format!("switch(source[span.x+{depth}u]){{"));
        for (byte, child) in &self.children {
            source.push_str(&format!("case {byte}u:"));
            child.emit(depth + 1, source);
            source.push_str("break;");
        }
        source.push_str("default:break;}");
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
            BufferReservation::acquire(length, &METAL_BUFFER_BYTES, METAL_BUFFER_LIMIT)?;
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

pub(in crate::code_rule_dsl) struct MappedBuffer {
    // The Metal deallocator retains the mapping and its budget reservation.
    pub(super) buffer: Buffer,
}

// One bounded staging allocation serves the complete streamed corpus. Pending
// copies retain their destination until the checked command has completed.
pub(super) struct PrivateInputUpload {
    queue: metal::CommandQueue,
    staging: MappedBuffer,
    used: usize,
    copies: Vec<(Buffer, u64, u64, u64)>,
}

impl PrivateInputUpload {
    pub(super) fn new(device: &Device) -> Result<Self, Error> {
        Ok(Self {
            queue: device.new_command_queue(),
            staging: MappedBuffer::zeroed::<u8>(device, METAL_UPLOAD_CHUNK_BYTES)?,
            used: 0,
            copies: Vec::new(),
        })
    }

    pub(super) fn write<T: Copy>(
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

    pub(super) fn finish(&mut self) -> Result<(), Error> {
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
    /// Another owner of `buffer` (Metal retains it), as a kept buffer is handed between runs.
    pub(super) fn wrap(buffer: Buffer) -> Self {
        Self { buffer }
    }

    #[track_caller]
    fn new(device: &Device, bytes: &[u8]) -> Result<Self, Error> {
        let mapping = Arc::new(MappedRegion::new(bytes)?);
        Self::from_mapping(device, mapping)
    }

    #[track_caller]
    fn from_mapping(device: &Device, mapping: Arc<MappedRegion>) -> Result<Self, Error> {
        if crate::code_rule_dsl::prof::on() && mapping.length >= 64 << 10 {
            crate::code_rule_dsl::prof::count_named(format!("alloc_kb.{}", std::panic::Location::caller()), mapping.length >> 10);
        }
        let _allocation = METAL_ALLOCATION_LOCK
            .lock()
            .map_err(|_| Error::BackendFailure)?;
        if mapping.length as u64 > device.max_buffer_length()
            || device
                .current_allocated_size()
                .saturating_add(mapping.length as u64)
                > METAL_BUFFER_LIMIT as u64
        {
            if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
                eprintln!(
                    "TEMPO_METAL_DRIVER_REJECT bytes={} reserved={} driver={} maximumBuffer={}",
                    mapping.length,
                    METAL_BUFFER_BYTES.load(Ordering::Acquire),
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
    pub(super) fn from_slice<T: Copy>(device: &Device, values: &[T]) -> Result<Self, Error> {
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
        let _allocation = METAL_ALLOCATION_LOCK
            .lock()
            .map_err(|_| Error::BackendFailure)?;
        if length as u64 > device.max_buffer_length()
            || device
                .current_allocated_size()
                .saturating_add(length as u64)
                > METAL_BUFFER_LIMIT as u64
        {
            if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
                eprintln!(
                    "TEMPO_METAL_DRIVER_REJECT bytes={length} reserved={} driver={} maximumBuffer={} caller={}",
                    METAL_BUFFER_BYTES.load(Ordering::Acquire),
                    device.current_allocated_size(),
                    device.max_buffer_length(),
                    std::panic::Location::caller()
                );
            }
            return Err(Error::BackendCapacity);
        }
        if crate::code_rule_dsl::prof::on() && length >= 64 << 10 {
            crate::code_rule_dsl::prof::count_named(format!("alloc_kb.{}", std::panic::Location::caller()), length >> 10);
        }
        let buffer = device.new_buffer(length as u64, storage);
        // Private buffers have no bytes-no-copy deallocator. An associated copied Objective-C
        // block holds the reservation until the Metal object loses its final reference.
        retain_buffer_owner(&buffer, reservation);
        Ok(Self { buffer })
    }

    pub(super) fn private_from_slice<T: Copy>(
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
            BufferReservation::acquire(length, &METAL_BUFFER_BYTES, METAL_BUFFER_LIMIT)?.private(),
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

    pub(super) fn private_capacity<T: Copy>(device: &Device, count: usize) -> Result<Self, Error> {
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
            BufferReservation::acquire(length, &METAL_BUFFER_BYTES, METAL_BUFFER_LIMIT)?.private(),
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
    pub(super) fn untouched<T>(device: &Device, count: usize) -> Result<Self, Error> {
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
            &METAL_BUFFER_BYTES,
            METAL_BUFFER_LIMIT,
        )?);
        Self::owned_allocation(device, length, reservation, MTLResourceOptions::StorageModeShared)
    }

    #[track_caller]
    pub(super) fn zeroed<T>(device: &Device, count: usize) -> Result<Self, Error> {
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
            &METAL_BUFFER_BYTES,
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

fn anchor_literal(program: &Program) -> Result<(u16, &[u8], u16, u16), Error> {
    if program.words.len() != 4
        || !program.directives.is_empty()
        || !matches!(split(program.words[0]), (KIND, 0, _))
        || !matches!(split(program.words[1]), (TEXT, 0, _))
        || !matches!(split(program.words[2]), (super::CAPTURE, 0, _))
        || !matches!(split(program.words[3]), (EMIT, 0, _))
    {
        return Err(Error::UnsupportedBackend);
    }
    let predicate = split(program.words[1]).2;
    let TextPredicate::Equal(string) = program.predicates[usize::from(predicate)] else {
        return Err(Error::UnsupportedBackend);
    };
    Ok((
        split(program.words[0]).2,
        &program.strings[usize::from(string)],
        split(program.words[2]).2,
        split(program.words[3]).2,
    ))
}

static METAL_EXECUTION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub(super) struct MetalExecutionGuard {
    _lock: MutexGuard<'static, ()>,
    _launch_scope: launch::LaunchScope,
}

pub(super) fn metal_execution_guard() -> Result<MetalExecutionGuard, Error> {
    let lock = METAL_EXECUTION_LOCK
        .lock()
        .map_err(|_| Error::BackendFailure)?;
    Ok(MetalExecutionGuard {
        _lock: lock,
        _launch_scope: launch::LaunchScope::enter(),
    })
}

thread_local! {
    static PIPELINES: RefCell<HashMap<[u8; 32], ComputePipelineState>> = RefCell::new(HashMap::new());
    static LIBRARIES: RefCell<HashMap<[u8;32],metal::Library>>=RefCell::new(HashMap::new());
}

pub(super) fn pipeline_named(source: &str, name: &str) -> Result<ComputePipelineState, Error> {
    pipeline_named_constant(source, name, None)
}

pub(super) fn pipeline_named_constant(
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
    if let Some(cached) = PIPELINES.with_borrow(|pipelines| pipelines.get(&key).cloned()) {
        return Ok(cached);
    }
    let mut library_digest = Sha256::new();
    library_digest.update(device.registry_id().to_le_bytes());
    library_digest.update(source.as_bytes());
    let library_key: [u8; 32] = library_digest.finalize().into();
    let library = if let Some(library) =
        LIBRARIES.with_borrow(|libraries| libraries.get(&library_key).cloned())
    {
        library
    } else {
        let library = device
            .new_library_with_source(source, &CompileOptions::new())
            .map_err(|error| {
                eprintln!("Metal library compilation failed: {error}");
                Error::BackendFailure
            })?;
        LIBRARIES.with_borrow_mut(|libraries| {
            if libraries.len() >= 16
                && let Some(oldest) = libraries.keys().next().copied()
            {
                libraries.remove(&oldest);
            }
            libraries.insert(library_key, library.clone());
        });
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
    PIPELINES.with_borrow_mut(|pipelines| {
        if pipelines.len() >= 512
            && let Some(oldest) = pipelines.keys().next().copied()
        {
            pipelines.remove(&oldest);
        }
        pipelines.insert(key, pipeline.clone());
    });
    Ok(pipeline)
}

#[cfg(test)]
pub(super) fn pipeline_cache_len() -> usize {
    PIPELINES.with_borrow(HashMap::len)
}

pub(super) struct PreparedAnchorScan {
    pipeline: ComputePipelineState,
    queue: CommandQueue,
    kinds: MappedBuffer,
    spans: MappedBuffer,
    node_files: MappedBuffer,
    group_enabled: MappedBuffer,
    source: MappedBuffer,
    hits: MappedBuffer,
    hit_count: MappedBuffer,
    locations: Vec<(usize, u32)>,
    group_programs: Vec<Vec<usize>>,
    file_program_masks: Vec<u32>,
    program_words: usize,
    file_count: usize,
    program_count: usize,
}

/// Prepare one specialized GPU dispatch for exact kind/value anchor selection. The generated
/// kernel groups literals by kind and length, then uses byte tries. It appends only positive hits.
pub(super) fn prepare_anchor_scan(
    programs: &[Program],
    files: &[TapeDoc],
) -> Result<PreparedAnchorScan, Error> {
    prepare_anchor_scan_with_eligibility(programs, files, None)
}

/// `file_program_masks` carries file/check eligibility from SQL into the GPU scan. Each file owns
/// `ceil(programs / 32)` words. This representation does not split large program sets into more
/// dispatches.
pub(super) fn prepare_anchor_scan_with_eligibility(
    programs: &[Program],
    files: &[TapeDoc],
    file_program_masks: Option<&[Vec<u32>]>,
) -> Result<PreparedAnchorScan, Error> {
    if file_program_masks.is_some_and(|masks| masks.len() != files.len()) {
        return Err(Error::InvalidAnchor);
    }
    let program_words = programs.len().div_ceil(32);
    if file_program_masks.is_some_and(|masks| masks.iter().any(|mask| mask.len() != program_words))
    {
        return Err(Error::InvalidAnchor);
    }
    if let (Some(masks), Some(last)) = (file_program_masks, programs.len().checked_rem(32))
        && last != 0
        && masks
            .iter()
            .any(|mask| mask.last().is_some_and(|word| word >> last != 0))
    {
        return Err(Error::InvalidAnchor);
    }
    let mut flat_program_masks = Vec::with_capacity(files.len().saturating_mul(program_words));
    for file in 0..files.len() {
        if let Some(masks) = file_program_masks {
            flat_program_masks.extend_from_slice(&masks[file]);
        } else {
            flat_program_masks.extend(std::iter::repeat_n(u32::MAX, program_words));
            if !programs.len().is_multiple_of(32)
                && let Some(last) = flat_program_masks.last_mut()
            {
                *last = (1_u32 << (programs.len() % 32)) - 1;
            }
        }
    }

    let mut literal_groups = BTreeMap::<(u16, Vec<u8>), usize>::new();
    let mut group_programs = Vec::<Vec<usize>>::new();
    for (program_index, program) in programs.iter().enumerate() {
        let (kind, text, _, _) = anchor_literal(program)?;
        let next = literal_groups.len();
        let group = *literal_groups.entry((kind, text.to_vec())).or_insert(next);
        if group == group_programs.len() {
            group_programs.push(Vec::new());
        }
        group_programs[group].push(program_index);
    }
    let group_words = group_programs.len().div_ceil(32);
    let mut group_enabled = vec![0_u32; files.len().saturating_mul(group_words)];
    for file in 0..files.len() {
        for (group, members) in group_programs.iter().enumerate() {
            let allowed = members.iter().any(|program| {
                flat_program_masks[file * program_words + program / 32] & (1_u32 << (program % 32))
                    != 0
            });
            if allowed {
                group_enabled[file * group_words + group / 32] |= 1_u32 << (group % 32);
            }
        }
    }

    let mut tries = BTreeMap::<(u16, usize), LiteralTrie>::new();
    for ((kind, text), group) in literal_groups {
        tries
            .entry((kind, text.len()))
            .or_default()
            .insert(&text, group as u32);
    }
    let mut tests = String::new();
    for ((kind, length), trie) in &tries {
        tests.push_str(&format!("if(kind=={kind}u && span.y-span.x=={length}u){{"));
        trie.emit(0, &mut tests);
        tests.push('}');
    }
    let metal_source = format!(
        r#"#include <metal_stdlib>
using namespace metal;
struct AnchorHit {{ uint node; uint group; }};
kernel void anchors(device const ushort* kinds [[buffer(0)]], device const uint2* spans [[buffer(1)]],
 device const uint* nodeFiles [[buffer(2)]], device const uint* groupEnabled [[buffer(3)]],
 device const uchar* source [[buffer(4)]], device AnchorHit* hits [[buffer(5)]],
 device atomic_uint* hitCount [[buffer(6)]], constant uint& count [[buffer(7)]],
 uint id [[thread_position_in_grid]]) {{
 if(id>=count)return; uint kind=kinds[id]; uint2 span=spans[id]; uint group=0xffffffffu; {tests}
 if(group!=0xffffffffu){{uint file=nodeFiles[id]; uint word=groupEnabled[file*{group_words}u+(group>>5u)];
  if((word&(1u<<(group&31u)))!=0u){{uint slot=atomic_fetch_add_explicit(hitCount,1u,memory_order_relaxed);hits[slot]={{id,group}};}}}}
}}"#
    );
    let pipeline = pipeline_named(&metal_source, "anchors")?;
    let device = Device::system_default().ok_or(Error::UnsupportedBackend)?;
    let queue = device.new_command_queue();
    let mut source = Vec::new();
    let mut kinds = Vec::new();
    let mut spans = Vec::new();
    let mut node_files = Vec::new();
    let mut locations = Vec::new();
    for (fi, file) in files.iter().enumerate() {
        let base = u32::try_from(source.len()).map_err(|_| Error::BackendFailure)?;
        source.extend_from_slice(file.transducer_source());
        for ordinal in 0..file.transducer_len() {
            let ordinal = u32::try_from(ordinal).map_err(|_| Error::BackendFailure)?;
            let span = file.transducer_span(ordinal);
            let start = u32::try_from(span.start)
                .ok()
                .and_then(|start| base.checked_add(start))
                .ok_or(Error::BackendFailure)?;
            let end = u32::try_from(span.end)
                .ok()
                .and_then(|end| base.checked_add(end))
                .ok_or(Error::BackendFailure)?;
            kinds.push(file.transducer_kind(ordinal));
            spans.push([start, end]);
            node_files.push(u32::try_from(fi).map_err(|_| Error::BackendFailure)?);
            locations.push((fi, ordinal));
        }
    }
    let kinds = MappedBuffer::from_slice(&device, &kinds)?;
    let spans = MappedBuffer::from_slice(&device, &spans)?;
    let node_files = MappedBuffer::from_slice(&device, &node_files)?;
    let group_enabled = MappedBuffer::from_slice(&device, &group_enabled)?;
    let source = MappedBuffer::new(&device, &source)?;
    let hits = MappedBuffer::zeroed::<AnchorHit>(&device, locations.len())?;
    let hit_count = MappedBuffer::zeroed::<u32>(&device, 1)?;
    Ok(PreparedAnchorScan {
        pipeline,
        queue,
        kinds,
        spans,
        node_files,
        group_enabled,
        source,
        hits,
        hit_count,
        locations,
        group_programs,
        file_program_masks: flat_program_masks,
        program_words,
        file_count: files.len(),
        program_count: programs.len(),
    })
}

type AnchorLists = Vec<Vec<Vec<u32>>>;

pub(super) struct AnchorScanProfile {
    pub dispatch_and_wait: std::time::Duration,
    pub reconstruction: std::time::Duration,
    pub hit_count: usize,
}

impl PreparedAnchorScan {
    pub(super) fn run(&self) -> Result<AnchorLists, Error> {
        self.run_profiled().map(|(anchors, _)| anchors)
    }

    pub(super) fn run_profiled(&self) -> Result<(AnchorLists, AnchorScanProfile), Error> {
        let mut anchors = (0..self.file_count)
            .map(|_| (0..self.program_count).map(|_| Vec::new()).collect())
            .collect::<Vec<Vec<Vec<u32>>>>();
        if self.locations.is_empty() {
            return Ok((
                anchors,
                AnchorScanProfile {
                    dispatch_and_wait: std::time::Duration::ZERO,
                    reconstruction: std::time::Duration::ZERO,
                    hit_count: 0,
                },
            ));
        }
        let _execution_guard = metal_execution_guard()?;
        unsafe {
            self.hit_count.buffer.contents().cast::<u32>().write(0);
        }
        let dispatch_started = std::time::Instant::now();
        let command = self.queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        encoder.set_label("anchor_selection");
        encoder.set_compute_pipeline_state(&self.pipeline);
        encoder.set_buffer(0, Some(&self.kinds.buffer), 0);
        encoder.set_buffer(1, Some(&self.spans.buffer), 0);
        encoder.set_buffer(2, Some(&self.node_files.buffer), 0);
        encoder.set_buffer(3, Some(&self.group_enabled.buffer), 0);
        encoder.set_buffer(4, Some(&self.source.buffer), 0);
        encoder.set_buffer(5, Some(&self.hits.buffer), 0);
        encoder.set_buffer(6, Some(&self.hit_count.buffer), 0);
        let count = u32::try_from(self.locations.len()).map_err(|_| Error::BackendFailure)?;
        encoder.set_bytes(7, 4, (&count as *const u32).cast());
        let width = self.pipeline.thread_execution_width();
        // One dispatch is the complete anchor-selection phase.
        encoder.checked_dispatch_threads(
            command,
            MTLSize {
                width: count as u64,
                height: 1,
                depth: 1,
            },
            MTLSize {
                width,
                height: 1,
                depth: 1,
            },
        );
        encoder.end_encoding();
        commit_checked(command)?;
        wait_checked(command)?;
        let dispatch_and_wait = dispatch_started.elapsed();
        let reconstruction_started = std::time::Instant::now();
        let hit_count = unsafe { self.hit_count.buffer.contents().cast::<u32>().read() as usize };
        if hit_count > self.locations.len() {
            return Err(Error::BackendFailure);
        }
        let mut hits = unsafe {
            std::slice::from_raw_parts(self.hits.buffer.contents().cast::<AnchorHit>(), hit_count)
                .to_vec()
        };
        // Atomic append order is undefined. Global node order is file order plus preorder order.
        hits.sort_unstable_by_key(|hit| hit.node);
        for hit in hits {
            let (file, ordinal) = self.locations[hit.node as usize];
            let programs = self
                .group_programs
                .get(hit.group as usize)
                .ok_or(Error::BackendFailure)?;
            for &program in programs {
                let enabled = self.file_program_masks[file * self.program_words + program / 32]
                    & (1_u32 << (program % 32))
                    != 0;
                if enabled {
                    anchors[file][program].push(ordinal);
                }
            }
        }
        let reconstruction = reconstruction_started.elapsed();
        Ok((
            anchors,
            AnchorScanProfile {
                dispatch_and_wait,
                reconstruction,
                hit_count,
            },
        ))
    }
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
