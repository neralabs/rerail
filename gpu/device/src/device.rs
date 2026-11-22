//! The forest's device (Rerail, decision 0440): the target's GPU (Metal on macOS, compute_kernel_metal.rs; CUDA on
//! Linux and Windows, compute_kernel_cuda.rs), an OpenCL GPU (compute_kernel_opencl.rs: integrated GPUs with no other way in) or
//! the CPU (compute_kernel_cpu.rs: every threadgroup's threads coroutines, for debugging kernels), chosen once a
//! process: `RERAIL_GPU=cpu` or `=opencl`, else the target's GPU. Each type below is one of them, and gives the grid
//! runtime and the trainer one interface (MLDB's `ComputeRuntime`):
//!
//! - `Device`, `DeviceRef`: the device (`gpu::shared`'s); `QueueRoot`: what a `ComputeQueue` is made from;
//! - `Buffer`, `BufferRef`: memory the host reads and writes in place (`contents`) and kernels read;
//! - `ComputeQueue`, `ComputeEvent`: launches, copies and fills in order, and the event a flush gives;
//! - `ComputeFunctionLibrary`, `ComputeFunction`: compiled kernels, with their arguments;
//! - `SharedEvent`: a value the host signals and a queue waits for;
//! - the array helpers, `gpu_seconds_awaited` and `backend()`.
//!
//! A value of one backend given to the other's is a bug, and panics.

pub use crate::compute_kernel::*;
use crate::compute_kernel_cpu as cpu;
use crate::compute_kernel_opencl as cl;

#[cfg(target_os = "macos")]
mod native {
    pub use crate::compute_kernel_metal::{
        alloc_uninitialized_array_sync, gpu_seconds_awaited, transfer_to_device_immutable, transfer_to_host_sync, wrap_host_memory, MetalComputeEvent as Event,
        MetalComputeFunction as Function, MetalComputeFunctionLibrary as Library, MetalComputeQueue as Queue,
    };
    /// The kernels' address of a buffer's first byte.
    pub fn device_address(b: &Buffer) -> Option<u64> {
        Some(b.gpu_address())
    }
    pub type Device = metal::Device;
    pub type Buffer = metal::Buffer;
    pub type QueueRoot = metal::CommandQueue;
    pub type SharedEvent = metal::SharedEvent;
    pub const NAME: &str = "Metal";
    /// The device's own name (the GPU's model).
    pub fn device_name(d: &Device) -> String {
        d.name().to_owned()
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    pub use crate::compute_kernel_cuda::{
        alloc_uninitialized_array_sync, gpu_seconds_awaited, transfer_to_device_immutable, transfer_to_host_sync, wrap_host_memory, CudaBuffer as Buffer,
        CudaComputeEvent as Event, CudaComputeFunction as Function, CudaComputeFunctionLibrary as Library, CudaComputeQueue as Queue, CudaDevice as Device,
        CudaQueueRoot as QueueRoot, CudaSharedEvent as SharedEvent,
    };
    /// The kernels' address of a buffer's first byte.
    pub fn device_address(b: &Buffer) -> Option<u64> {
        Some(b.device_address())
    }
    pub const NAME: &str = "CUDA";
    /// The device's own name: the backend's until the driver's is read (step E).
    pub fn device_name(_d: &Device) -> String {
        NAME.to_owned()
    }
}

/// The backend `RERAIL_GPU` chooses, lower case: "cpu", "opencl", or "" for the target's GPU.
pub(crate) fn chosen() -> &'static str {
    static CHOSEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CHOSEN.get_or_init(|| std::env::var("RERAIL_GPU").map(|v| v.to_ascii_lowercase()).unwrap_or_default())
}

/// Whether this process runs its kernels on the CPU (`RERAIL_GPU=cpu`).
pub fn cpu_chosen() -> bool {
    chosen() == "cpu"
}

/// Whether this process runs its kernels on an OpenCL GPU (`RERAIL_GPU=opencl`).
pub fn opencl_chosen() -> bool {
    chosen() == "opencl"
}

/// The lanes of the process's device's SIMD groups (`UKL_SIMD_WIDTH`).
pub fn simd_width() -> u32 {
    crate::shared::get().map_or(32, |s| s.device.simd_width())
}

/// The most threads a threadgroup of the process's device can have.
pub fn max_threadgroup() -> u32 {
    crate::shared::get().map_or(1024, |s| s.device.max_threadgroup())
}

/// The backend's name, for messages: the process's device's.
pub fn backend() -> &'static str {
    match crate::shared::get().map(|s| &s.device) {
        Some(Device::Cpu(_)) => "CPU",
        Some(Device::OpenCl(_)) => "OpenCL",
        Some(Device::Native(_)) => native::NAME,
        None if cpu_chosen() => "CPU",
        None if opencl_chosen() => "OpenCL",
        None => native::NAME,
    }
}

fn other() -> ! {
    panic!("a value of one backend given to another's (the GPU's, OpenCL's and the CPU's)")
}

/// The device.
#[derive(Clone)]
pub enum Device {
    Native(native::Device),
    Cpu(cpu::CpuDevice),
    OpenCl(cl::ClDevice),
}

pub type DeviceRef = Device;

impl Device {
    /// The device's name, as a machine's samples are keyed: the GPU's model on the target's GPU, its backend and
    /// its own name on OpenCL, "CPU" on the CPU's.
    pub fn name(&self) -> String {
        match self {
            Device::Native(d) => native::device_name(d),
            Device::Cpu(_) => "CPU".to_owned(),
            Device::OpenCl(d) => format!("OpenCL {}", d.name()),
        }
    }

    /// The Metal device, when the process's is Metal (the engine's own Metal kernels need it).
    #[cfg(target_os = "macos")]
    pub fn metal(&self) -> Option<&metal::Device> {
        match self {
            Device::Native(d) => Some(d),
            _ => None,
        }
    }

    /// The CUDA device, when the process's is CUDA.
    #[cfg(not(target_os = "macos"))]
    pub fn cuda(&self) -> Option<&crate::compute_kernel_cuda::CudaDevice> {
        match self {
            Device::Native(d) => Some(d),
            _ => None,
        }
    }

    /// The lanes of the device's SIMD groups, as its kernels are compiled (`UKL_SIMD_WIDTH`).
    pub fn simd_width(&self) -> u32 {
        match self {
            Device::Native(_) => 32,
            Device::Cpu(_) => cpu::simd_width(),
            Device::OpenCl(d) => d.simd_width(),
        }
    }

    /// Whether a source written for `w` lanes (`UKL_SIMD_WIDTH_NEEDED`) runs here: its own width, or (OpenCL) one of
    /// its sub-groups' sizes.
    pub fn offers_simd_width(&self, w: u32) -> bool {
        match self {
            Device::OpenCl(d) => d.offers_simd_width(w),
            _ => self.simd_width() == w,
        }
    }

    /// The memory domain its kernels read (decisions 0440 §7.2, 0491 step 4): 0 for the host's memory, read in place
    /// (Apple's unified memory, the CPU, an OpenCL device sharing the host's memory, a CUDA device reading the host's
    /// pages, as a GB10 does); 1 for a discrete card's own memory, whose copy of a region a mirror holds
    /// (`rerail_blocks::region::Region::mirror_in`).
    pub fn domain(&self) -> u32 {
        match self {
            #[cfg(not(target_os = "macos"))]
            Device::Native(d) => u32::from(!d.pageable),
            #[cfg(target_os = "macos")]
            Device::Native(_) => 0,
            Device::Cpu(_) | Device::OpenCl(_) => 0,
        }
    }

    /// Whether every buffer has a kernels' address (`Buffer::device_address`), memory wrapped in place too: every
    /// backend's but OpenCL's.
    pub fn has_addresses(&self) -> bool {
        !matches!(self, Device::OpenCl(_))
    }

    /// Whether the buffers it allocates have a kernels' address (not memory wrapped in place): every backend's, and
    /// OpenCL's where it has Intel's unified shared memory (`ClDevice::allocations_have_addresses`).
    pub fn allocations_have_addresses(&self) -> bool {
        match self {
            Device::OpenCl(d) => d.allocations_have_addresses(),
            _ => true,
        }
    }

    /// The most threads a threadgroup can have: 1,024 on Metal, CUDA and the CPU; an OpenCL device's own (256 on
    /// Apple's).
    pub fn max_threadgroup(&self) -> u32 {
        match self {
            Device::OpenCl(d) => d.max_group() as u32,
            _ => 1024,
        }
    }

    /// The largest buffer the device makes: Metal's own; OpenCL's largest allocation; on CUDA and the CPU, the
    /// host's memory bounds it, not the device.
    pub fn max_buffer_length(&self) -> u64 {
        match self {
            #[cfg(target_os = "macos")]
            Device::Native(d) => d.max_buffer_length(),
            Device::OpenCl(d) => d.max_alloc(),
            _ => 1 << 40,
        }
    }

    /// The alignment, in bytes, of an offset a buffer is bound at: OpenCL's sub-buffer origin
    /// (`CL_DEVICE_MEM_BASE_ADDR_ALIGN`: 512 on NVIDIA's); 1 elsewhere, where any offset its type aligns binds.
    pub fn bind_align(&self) -> u64 {
        match self {
            Device::OpenCl(d) => d.base_align(),
            _ => 1,
        }
    }

    pub fn is_cpu(&self) -> bool {
        matches!(self, Device::Cpu(_))
    }

    /// The device's GPU cores, when it says: on a Mac, IORegistry's `gpu-core-count` of the device's entry
    /// (the `MTLDevice`'s registry ID). A persistent grid's resident groups are these times the groups a core
    /// holds (decision 0468 §4).
    pub fn gpu_cores(&self) -> Option<u32> {
        match self {
            #[cfg(target_os = "macos")]
            Device::Native(d) => ioreg::gpu_core_count(d.registry_id()),
            _ => None,
        }
    }

    /// The most threadgroup memory a group may have: Metal's `maxThreadgroupMemoryLength`; 32 KB elsewhere.
    pub fn max_threadgroup_memory(&self) -> u64 {
        match self {
            #[cfg(target_os = "macos")]
            Device::Native(d) => d.max_threadgroup_memory_length(),
            _ => 32 << 10,
        }
    }

    /// The groups of `threads` threads, each using `local` bytes of threadgroup memory, that the device holds at
    /// once, when it gives its cores (`gpu_cores`): a persistent grid's G (decision 0468 §4). Each core is taken at
    /// its least: one group of the device's largest (`max_threadgroup`) and the most threadgroup memory a group may
    /// have (`max_threadgroup_memory`). The CPU's is exact: the groups its launches run at once
    /// (`compute_kernel_cpu::resident_groups`, the launch's `resident`), its memory a host's.
    pub fn resident_groups(&self, threads: u32, local: u64) -> Option<u64> {
        if let Device::Cpu(_) = self {
            return Some(cpu::resident_groups(threads as usize) as u64);
        }
        let cores = self.gpu_cores()? as u64;
        let by_threads = (self.max_threadgroup() / threads.max(1)).max(1) as u64;
        let by_memory = self.max_threadgroup_memory().checked_div(local).map_or(by_threads, |n| n.max(1));
        Some(cores * by_threads.min(by_memory))
    }

    /// The defines a kernel's preprocessing for this device takes (the CPU's knobs, `compute_kernel_cpu::defines`;
    /// OpenCL's width and largest threadgroup): `rerail_gpu::Preprocessing`.
    pub fn compile_defines(&self) -> Vec<(&'static str, String)> {
        match self {
            Device::Cpu(_) => cpu::defines(),
            Device::OpenCl(d) => d.defines(),
            Device::Native(_) => Vec::new(),
        }
    }

    pub fn new_shared_event(&self) -> SharedEvent {
        match self {
            Device::Native(d) => SharedEvent::Native(d.new_shared_event()),
            Device::Cpu(d) => SharedEvent::Cpu(d.new_shared_event()),
            Device::OpenCl(d) => SharedEvent::OpenCl(d.new_shared_event()),
        }
    }
}

/// Memory the host and the kernels read in place.
#[derive(Clone)]
pub enum Buffer {
    Native(native::Buffer),
    Cpu(cpu::CpuBuffer),
    OpenCl(cl::ClBuffer),
}

pub type BufferRef = Buffer;

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Buffer::Native(_) => write!(f, "Buffer({} bytes)", self.length()),
            Buffer::Cpu(b) => b.fmt(f),
            Buffer::OpenCl(b) => b.fmt(f),
        }
    }
}

impl Buffer {
    pub fn contents(&self) -> *mut std::ffi::c_void {
        match self {
            Buffer::Native(b) => b.contents(),
            Buffer::Cpu(b) => b.contents(),
            Buffer::OpenCl(b) => b.contents(),
        }
    }

    pub fn length(&self) -> u64 {
        match self {
            Buffer::Native(b) => b.length(),
            Buffer::Cpu(b) => b.length(),
            Buffer::OpenCl(b) => b.length(),
        }
    }

    /// The kernels' address of the buffer's first byte, for a pointer that a buffer holds (a directory of tapes, a
    /// view's rows): Metal's GPU address, CUDA's device pointer, the host's on the CPU, an OpenCL host allocation's
    /// (Intel's unified shared memory). `None` for other OpenCL buffers: memory wrapped in place, or a device without
    /// it, whose kernels follow no pointer a buffer holds.
    pub fn device_address(&self) -> Option<u64> {
        match self {
            Buffer::Native(b) => native::device_address(b),
            Buffer::Cpu(b) => Some(b.device_address()),
            Buffer::OpenCl(b) => b.address(),
        }
    }

    /// The Metal buffer, when the process's device is Metal.
    #[cfg(target_os = "macos")]
    pub fn metal(&self) -> Option<&metal::Buffer> {
        match self {
            Buffer::Native(b) => Some(b),
            _ => None,
        }
    }
}

/// A value the host signals and a queue waits for.
#[derive(Clone)]
pub enum SharedEvent {
    Native(native::SharedEvent),
    Cpu(cpu::CpuSharedEvent),
    OpenCl(cl::ClSharedEvent),
}

impl SharedEvent {
    pub fn set_signaled_value(&self, v: u64) {
        match self {
            SharedEvent::Native(e) => e.set_signaled_value(v),
            SharedEvent::Cpu(e) => e.set_signaled_value(v),
            SharedEvent::OpenCl(e) => e.set_signaled_value(v),
        }
    }

    pub fn signaled_value(&self) -> u64 {
        match self {
            SharedEvent::Native(e) => e.signaled_value(),
            SharedEvent::Cpu(e) => e.signaled_value(),
            SharedEvent::OpenCl(e) => e.signaled_value(),
        }
    }
}

/// What a `ComputeQueue` is made from.
pub enum QueueRoot {
    Native(native::QueueRoot),
    Cpu(cpu::CpuQueueRoot),
    OpenCl(cl::ClQueueRoot),
}

impl QueueRoot {
    /// The Metal command queue, when the process's device is Metal.
    #[cfg(target_os = "macos")]
    pub fn metal(&self) -> Option<&metal::CommandQueue> {
        match self {
            QueueRoot::Native(q) => Some(q),
            _ => None,
        }
    }
}

/// What sees every buffer a queue's work reaches, as the work is enqueued, before it can run (decision 0487 step 2):
/// a kernel's bound and resident buffers, a copy's source and destination, a fill's. The device layer knows nothing
/// of what memory is; the engine's watch (`rerail_accel::gpu::shared::Queue`) holds the block or pooled lease each
/// lies in until the work completes.
pub trait Watch: Send + Sync {
    /// Work enqueued now reads (`write` false) or writes `buffer`'s memory, `what` the kernel or operation.
    fn reaches(&self, buffer: &Buffer, write: bool, what: &'static str);
}

/// Launches, copies and fills, in order, each buffer they reach seen by the queue's watch (`ComputeQueue::watched`).
pub struct ComputeQueue {
    kind: QueueKind,
    watch: Option<std::sync::Arc<dyn Watch>>,
}

enum QueueKind {
    Native(native::Queue),
    Cpu(cpu::CpuComputeQueue),
    OpenCl(cl::ClComputeQueue),
}

impl ComputeQueue {
    /// A queue no watch sees: a test's or a tool's, over its own device and buffers. The engine's work on the
    /// process's device goes through its watched queue (`rerail_accel::gpu::shared::Queue`).
    pub fn new(root: &QueueRoot) -> Self {
        let kind = match root {
            QueueRoot::Native(r) => QueueKind::Native(native::Queue::new(r)),
            QueueRoot::Cpu(r) => QueueKind::Cpu(cpu::CpuComputeQueue::new(r)),
            QueueRoot::OpenCl(r) => QueueKind::OpenCl(cl::ClComputeQueue::new(r)),
        };
        ComputeQueue { kind, watch: None }
    }

    /// A queue whose every reach of a buffer `watch` sees first.
    pub fn watched(root: &QueueRoot, watch: std::sync::Arc<dyn Watch>) -> Self {
        ComputeQueue { watch: Some(watch), ..ComputeQueue::new(root) }
    }

    fn sees(&self, buffer: &Buffer, write: bool, what: &'static str) {
        if let Some(w) = &self.watch {
            w.reaches(buffer, write, what);
        }
    }

    pub fn new_bind_context<'a>(&'a self, function: &'a ComputeFunction) -> BindContext<'a> {
        let kind = match (&self.kind, function) {
            (QueueKind::Native(q), ComputeFunction::Native(f)) => BindKind::Native(q.new_bind_context(f)),
            (QueueKind::Cpu(q), ComputeFunction::Cpu(f)) => BindKind::Cpu(q.new_bind_context(f)),
            (QueueKind::OpenCl(q), ComputeFunction::OpenCl(f)) => BindKind::OpenCl(q.new_bind_context(f)),
            _ => other(),
        };
        BindContext { kind, watch: self.watch.as_deref(), args: function.get_argument_info() }
    }

    pub fn enqueue_copy_from_host(&self, dst: &Buffer, dst_offset: u64, bytes: &[u8]) {
        self.sees(dst, true, "a copy from the host");
        match (&self.kind, dst) {
            (QueueKind::Native(q), Buffer::Native(d)) => q.enqueue_copy_from_host(d, dst_offset, bytes),
            (QueueKind::Cpu(q), Buffer::Cpu(d)) => q.enqueue_copy_from_host(d, dst_offset, bytes),
            (QueueKind::OpenCl(q), Buffer::OpenCl(d)) => q.enqueue_copy_from_host(d, dst_offset, bytes),
            _ => other(),
        }
    }

    pub fn enqueue_fill_array<T: crate::Pod>(&self, dst: &Buffer, value: &T, offset: usize, count: Option<usize>) {
        self.sees(dst, true, "a fill");
        match (&self.kind, dst) {
            (QueueKind::Native(q), Buffer::Native(d)) => q.enqueue_fill_array(d, value, offset, count),
            (QueueKind::Cpu(q), Buffer::Cpu(d)) => q.enqueue_fill_array(d, value, offset, count),
            (QueueKind::OpenCl(q), Buffer::OpenCl(d)) => q.enqueue_fill_array(d, value, offset, count),
            _ => other(),
        }
    }

    pub fn enqueue_copy_between_device_regions(&self, src: &Buffer, dst: &Buffer, src_offset: u64, dst_offset: u64, length: u64) {
        self.sees(src, false, "a copy's source");
        self.sees(dst, true, "a copy's destination");
        match (&self.kind, src, dst) {
            (QueueKind::Native(q), Buffer::Native(s), Buffer::Native(d)) => q.enqueue_copy_between_device_regions(s, d, src_offset, dst_offset, length),
            (QueueKind::Cpu(q), Buffer::Cpu(s), Buffer::Cpu(d)) => q.enqueue_copy_between_device_regions(s, d, src_offset, dst_offset, length),
            (QueueKind::OpenCl(q), Buffer::OpenCl(s), Buffer::OpenCl(d)) => q.enqueue_copy_between_device_regions(s, d, src_offset, dst_offset, length),
            _ => other(),
        }
    }

    pub fn wait_for_event(&self, event: &SharedEvent, value: u64) {
        match (&self.kind, event) {
            (QueueKind::Native(q), SharedEvent::Native(e)) => q.wait_for_event(e, value),
            (QueueKind::Cpu(q), SharedEvent::Cpu(e)) => q.wait_for_event(e, value),
            (QueueKind::OpenCl(q), SharedEvent::OpenCl(e)) => q.wait_for_event(e, value),
            _ => other(),
        }
    }

    pub fn flush(&mut self) -> ComputeEvent {
        match &mut self.kind {
            QueueKind::Native(q) => ComputeEvent::Native(q.flush()),
            QueueKind::Cpu(q) => ComputeEvent::Cpu(q.flush()),
            QueueKind::OpenCl(q) => ComputeEvent::OpenCl(q.flush()),
        }
    }

    pub fn finish(self) -> Result<(), String> {
        match self.kind {
            QueueKind::Native(q) => q.finish(),
            QueueKind::Cpu(q) => q.finish(),
            QueueKind::OpenCl(q) => q.finish(),
        }
    }
}

/// The work a queue flushed.
pub enum ComputeEvent {
    Native(native::Event),
    Cpu(cpu::CpuComputeEvent),
    OpenCl(cl::ClComputeEvent),
}

impl ComputeEvent {
    pub fn await_(&self) -> Result<(), String> {
        match self {
            ComputeEvent::Native(e) => e.await_(),
            ComputeEvent::Cpu(e) => e.await_(),
            ComputeEvent::OpenCl(e) => e.await_(),
        }
    }

    pub fn is_complete(&self) -> bool {
        match self {
            ComputeEvent::Native(e) => e.is_complete(),
            ComputeEvent::Cpu(e) => e.is_complete(),
            ComputeEvent::OpenCl(e) => e.is_complete(),
        }
    }

    /// The Metal command buffer, when the process's device is Metal (its host-clock times, its label).
    #[cfg(target_os = "macos")]
    pub fn metal(&self) -> Option<&metal::CommandBufferRef> {
        match self {
            ComputeEvent::Native(e) => Some(&e.command_buffer),
            _ => None,
        }
    }

    pub fn gpu_seconds(&self) -> f64 {
        match self {
            ComputeEvent::Native(e) => e.gpu_seconds(),
            ComputeEvent::Cpu(e) => e.gpu_seconds(),
            ComputeEvent::OpenCl(e) => e.gpu_seconds(),
        }
    }
}

/// Compiled kernels.
pub enum ComputeFunctionLibrary {
    Native(native::Library),
    Cpu(cpu::CpuComputeFunctionLibrary),
    OpenCl(cl::ClComputeFunctionLibrary),
}

impl ComputeFunctionLibrary {
    /// `preprocessed_source`, UKL preprocessed for `device` (`Device::ukl_backend`), compiled.
    pub fn compile_from_source(device: &Device, preprocessed_source: &str, file_name_to_appear_in_error_messages: &str) -> Result<Self, String> {
        Ok(match device {
            Device::Native(d) => ComputeFunctionLibrary::Native(native::Library::compile_from_source(d, preprocessed_source, file_name_to_appear_in_error_messages)?),
            Device::Cpu(d) => ComputeFunctionLibrary::Cpu(cpu::CpuComputeFunctionLibrary::compile_from_source(d, preprocessed_source, file_name_to_appear_in_error_messages)?),
            Device::OpenCl(d) => ComputeFunctionLibrary::OpenCl(cl::ClComputeFunctionLibrary::compile_from_source(d, preprocessed_source, file_name_to_appear_in_error_messages)?),
        })
    }

    pub fn get_function(&self, device: &Device, function_name: &str) -> Result<ComputeFunction, String> {
        Ok(match (self, device) {
            (ComputeFunctionLibrary::Native(l), Device::Native(d)) => ComputeFunction::Native(l.get_function(d, function_name)?),
            (ComputeFunctionLibrary::Cpu(l), Device::Cpu(d)) => ComputeFunction::Cpu(l.get_function(d, function_name)?),
            (ComputeFunctionLibrary::OpenCl(l), Device::OpenCl(d)) => ComputeFunction::OpenCl(l.get_function(d, function_name)?),
            _ => other(),
        })
    }
}

/// A kernel, with its arguments.
pub enum ComputeFunction {
    Native(native::Function),
    Cpu(cpu::CpuComputeFunction),
    OpenCl(cl::ClComputeFunction),
}

impl ComputeFunction {
    /// The threadgroup memory the kernel declares, as its pipeline says (Metal's); 0 where the backend does not
    /// say.
    pub fn threadgroup_bytes(&self) -> u64 {
        match self {
            #[cfg(target_os = "macos")]
            ComputeFunction::Native(f) => f.compute_pipeline_state.static_threadgroup_memory_length(),
            _ => 0,
        }
    }

    /// The most threads a threadgroup of the kernel may have, as its pipeline says (Metal's); `None` where the
    /// backend does not say.
    pub fn max_threads_per_group(&self) -> Option<u64> {
        match self {
            #[cfg(target_os = "macos")]
            ComputeFunction::Native(f) => Some(f.compute_pipeline_state.max_total_threads_per_threadgroup()),
            _ => None,
        }
    }

    pub fn get_argument_info(&self) -> &[GridComputeFunctionArgument] {
        match self {
            ComputeFunction::Native(f) => f.get_argument_info(),
            ComputeFunction::Cpu(f) => f.get_argument_info(),
            ComputeFunction::OpenCl(f) => f.get_argument_info(),
        }
    }

    /// The Metal function, when the process's device is Metal.
    #[cfg(target_os = "macos")]
    pub fn metal(&self) -> Option<&crate::compute_kernel_metal::MetalComputeFunction> {
        match self {
            ComputeFunction::Native(f) => Some(f),
            _ => None,
        }
    }
}

/// One launch's arguments, each buffer seen by its queue's watch as it is bound: read or written as the kernel
/// declares the row (`MemoryRegionAccess`).
pub struct BindContext<'a> {
    kind: BindKind<'a>,
    watch: Option<&'a dyn Watch>,
    args: &'a [GridComputeFunctionArgument],
}

impl BindContext<'_> {
    fn sees(&self, arg_num: u64, buffer: &Buffer) {
        if let Some(w) = self.watch {
            let row = self.args.iter().find(|a| a.compute_function_arg_index == arg_num);
            let write = row.is_none_or(|a| a.ty.access != MemoryRegionAccess::Read);
            w.reaches(buffer, write, if row.is_some() { "a kernel's buffer row" } else { "a kernel's row it does not declare" });
        }
    }
}

enum BindKind<'a> {
    #[cfg(target_os = "macos")]
    Native(crate::compute_kernel_metal::MetalBindContext),
    #[cfg(not(target_os = "macos"))]
    Native(crate::compute_kernel_cuda::CudaBindContext<'a>),
    Cpu(cpu::CpuBindContext<'a>),
    OpenCl(cl::ClBindContext<'a>),
}

impl crate::compute_kernel_grid::GridBindContext for BindContext<'_> {
    fn set_primitive(&mut self, arg_num: u64, bytes: &[u8]) {
        match &mut self.kind {
            BindKind::Native(c) => c.set_primitive(arg_num, bytes),
            BindKind::Cpu(c) => c.set_primitive(arg_num, bytes),
            BindKind::OpenCl(c) => c.set_primitive(arg_num, bytes),
        }
    }

    fn set_buffer(&mut self, arg_num: u64, buffer: &BufferRef) {
        self.sees(arg_num, buffer);
        match (&mut self.kind, buffer) {
            (BindKind::Native(c), Buffer::Native(b)) => c.set_buffer(arg_num, b),
            (BindKind::Cpu(c), Buffer::Cpu(b)) => c.set_buffer(arg_num, b),
            (BindKind::OpenCl(c), Buffer::OpenCl(b)) => c.set_buffer(arg_num, b),
            _ => other(),
        }
    }

    fn set_buffer_at(&mut self, arg_num: u64, buffer: &BufferRef, offset: u64) -> Result<(), String> {
        if offset > buffer.length() {
            return Err(format!("argument {arg_num} bound at byte {offset} of a {}-byte buffer", buffer.length()));
        }
        self.sees(arg_num, buffer);
        match (&mut self.kind, buffer) {
            (BindKind::Native(c), Buffer::Native(b)) => c.set_buffer_at(arg_num, b, offset),
            (BindKind::Cpu(c), Buffer::Cpu(b)) => c.set_buffer_at(arg_num, b, offset),
            (BindKind::OpenCl(c), Buffer::OpenCl(b)) => c.set_buffer_at(arg_num, b, offset),
            _ => other(),
        }
    }

    fn set_thread_group_memory(&mut self, arg_num: u64, n_bytes: u64) {
        match &mut self.kind {
            BindKind::Native(c) => c.set_thread_group_memory(arg_num, n_bytes),
            BindKind::Cpu(c) => c.set_thread_group_memory(arg_num, n_bytes),
            BindKind::OpenCl(c) => c.set_thread_group_memory(arg_num, n_bytes),
        }
    }

    fn launch(&mut self, grid: &[u64], block: &[u64]) -> Result<(), String> {
        match &mut self.kind {
            BindKind::Native(c) => c.launch(grid, block),
            BindKind::Cpu(c) => c.launch(grid, block),
            BindKind::OpenCl(c) => c.launch(grid, block),
        }
    }

    fn use_buffer_read(&mut self, buffer: &BufferRef) {
        if let Some(w) = self.watch {
            w.reaches(buffer, false, "a resident buffer read");
        }
        let watch = self.watch.take();
        self.use_buffer(buffer);
        self.watch = watch;
    }

    fn use_buffer(&mut self, buffer: &BufferRef) {
        if let Some(w) = self.watch {
            w.reaches(buffer, true, "a resident buffer");
        }
        match (&mut self.kind, buffer) {
            (BindKind::Native(c), Buffer::Native(b)) => c.use_buffer(b),
            (BindKind::Cpu(c), Buffer::Cpu(b)) => c.use_buffer(b),
            (BindKind::OpenCl(c), Buffer::OpenCl(b)) => c.use_buffer(b),
            _ => other(),
        }
    }

    fn launch_indirect(&mut self, groups: &BufferRef, offset: u64, max_groups: &[u64], block: &[u64]) -> Result<(), String> {
        if !offset.is_multiple_of(4) || offset + 12 > groups.length() {
            return Err(format!("an indirect launch's counts at byte {offset} of a {}-byte buffer", groups.length()));
        }
        if let Some(w) = self.watch {
            w.reaches(groups, false, "an indirect launch's counts");
        }
        match (&mut self.kind, groups) {
            (BindKind::Native(c), Buffer::Native(b)) => c.launch_indirect(b, offset, max_groups, block),
            (BindKind::Cpu(c), Buffer::Cpu(b)) => c.launch_indirect(b, offset, max_groups, block),
            (BindKind::OpenCl(c), Buffer::OpenCl(b)) => c.launch_indirect(b, offset, max_groups, block),
            _ => other(),
        }
    }
}

/// `allocUninitializedArraySync`: memory for `length` elements (at least 16 bytes).
pub fn alloc_uninitialized_array_sync<T>(device: &Device, length: usize) -> Buffer {
    match device {
        Device::Native(d) => Buffer::Native(native::alloc_uninitialized_array_sync::<T>(d, length)),
        Device::Cpu(d) => Buffer::Cpu(cpu::alloc_uninitialized_array_sync::<T>(d, length)),
        Device::OpenCl(d) => Buffer::OpenCl(cl::alloc_uninitialized_array_sync::<T>(d, length)),
    }
}

/// A buffer over `len` bytes of the host's memory at `ptr` that the kernels read and write in place, no copy made
/// (decision 0353): a block's pages, a tape where it lies. Each backend states what memory it takes (Metal and
/// OpenCL: page-aligned); other memory is an error, never a copy.
///
/// # Safety
/// The memory outlives the buffer and its clones, and every use of them by queued work.
pub unsafe fn wrap_host_memory(device: &Device, ptr: *mut u8, len: usize) -> Result<Buffer, String> {
    unsafe {
        Ok(match device {
            Device::Native(d) => Buffer::Native(native::wrap_host_memory(d, ptr, len)?),
            Device::Cpu(d) => Buffer::Cpu(cpu::wrap_host_memory(d, ptr, len)?),
            Device::OpenCl(d) => Buffer::OpenCl(cl::wrap_host_memory(d, ptr, len)?),
        })
    }
}

/// `transferToDeviceImmutable`: memory holding `data`.
pub fn transfer_to_device_immutable<T: crate::Pod>(device: &Device, data: &[T]) -> Buffer {
    match device {
        Device::Native(d) => Buffer::Native(native::transfer_to_device_immutable(d, data)),
        Device::Cpu(d) => Buffer::Cpu(cpu::transfer_to_device_immutable(d, data)),
        Device::OpenCl(d) => Buffer::OpenCl(cl::transfer_to_device_immutable(d, data)),
    }
}

/// `transferToHostSync`: the first `length` elements (the work writing them must have completed).
pub fn transfer_to_host_sync<T: crate::Pod + Clone>(buffer: &Buffer, length: usize) -> Vec<T> {
    match buffer {
        Buffer::Native(b) => native::transfer_to_host_sync(b, length),
        Buffer::Cpu(b) => cpu::transfer_to_host_sync(b, length),
        Buffer::OpenCl(b) => cl::transfer_to_host_sync(b, length),
    }
}

/// The device seconds of every event awaited so far in this process.
pub fn gpu_seconds_awaited() -> f64 {
    native::gpu_seconds_awaited() + cpu::gpu_seconds_awaited() + cl::gpu_seconds_awaited()
}

/// IORegistry's description of a Mac's GPU: its `gpu-core-count`.
#[cfg(target_os = "macos")]
mod ioreg {
    use std::ffi::c_void;

    type CfRef = *const c_void;
    const UTF8: u32 = 0x0800_0100;
    const SINT32: isize = 3;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IORegistryEntryIDMatching(id: u64) -> CfRef;
        fn IOServiceGetMatchingService(main_port: u32, matching: CfRef) -> u32;
        fn IORegistryEntryCreateCFProperty(entry: u32, key: CfRef, allocator: CfRef, options: u32) -> CfRef;
        fn IOObjectRelease(object: u32) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(allocator: CfRef, s: *const u8, encoding: u32) -> CfRef;
        fn CFGetTypeID(v: CfRef) -> usize;
        fn CFNumberGetTypeID() -> usize;
        fn CFNumberGetValue(n: CfRef, kind: isize, out: *mut c_void) -> bool;
        fn CFRelease(v: CfRef);
    }

    /// `gpu-core-count` of the IORegistry entry `id`, when it has one.
    pub fn gpu_core_count(id: u64) -> Option<u32> {
        // SAFETY: each object made here is released here; the matching dictionary is consumed by the lookup.
        unsafe {
            let entry = IOServiceGetMatchingService(0, IORegistryEntryIDMatching(id));
            if entry == 0 {
                return None;
            }
            let key = CFStringCreateWithCString(std::ptr::null(), c"gpu-core-count".as_ptr().cast(), UTF8);
            let v = IORegistryEntryCreateCFProperty(entry, key, std::ptr::null(), 0);
            CFRelease(key);
            IOObjectRelease(entry);
            if v.is_null() {
                return None;
            }
            let mut n = 0i32;
            let ok = CFGetTypeID(v) == CFNumberGetTypeID() && CFNumberGetValue(v, SINT32, (&raw mut n).cast());
            CFRelease(v);
            (ok && n > 0).then_some(n as u32)
        }
    }
}
