//! The forest's device (Rerail, decision 0440): the target's GPU (Metal on macOS, compute_kernel_metal.rs; CUDA on
//! Linux, compute_kernel_cuda.rs), an OpenCL GPU (compute_kernel_opencl.rs: integrated GPUs with no other way in) or
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

#[cfg(target_os = "linux")]
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
fn chosen() -> &'static str {
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
    #[cfg(target_os = "linux")]
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
            Device::OpenCl(_) => cl::simd_width(),
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

    pub fn is_cpu(&self) -> bool {
        matches!(self, Device::Cpu(_))
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

/// Launches, copies and fills, in order.
pub enum ComputeQueue {
    Native(native::Queue),
    Cpu(cpu::CpuComputeQueue),
    OpenCl(cl::ClComputeQueue),
}

impl ComputeQueue {
    pub fn new(root: &QueueRoot) -> Self {
        match root {
            QueueRoot::Native(r) => ComputeQueue::Native(native::Queue::new(r)),
            QueueRoot::Cpu(r) => ComputeQueue::Cpu(cpu::CpuComputeQueue::new(r)),
            QueueRoot::OpenCl(r) => ComputeQueue::OpenCl(cl::ClComputeQueue::new(r)),
        }
    }

    pub fn new_bind_context<'a>(&'a self, function: &'a ComputeFunction) -> BindContext<'a> {
        match (self, function) {
            (ComputeQueue::Native(q), ComputeFunction::Native(f)) => BindContext::Native(q.new_bind_context(f)),
            (ComputeQueue::Cpu(q), ComputeFunction::Cpu(f)) => BindContext::Cpu(q.new_bind_context(f)),
            (ComputeQueue::OpenCl(q), ComputeFunction::OpenCl(f)) => BindContext::OpenCl(q.new_bind_context(f)),
            _ => other(),
        }
    }

    pub fn enqueue_copy_from_host(&self, dst: &Buffer, dst_offset: u64, bytes: &[u8]) {
        match (self, dst) {
            (ComputeQueue::Native(q), Buffer::Native(d)) => q.enqueue_copy_from_host(d, dst_offset, bytes),
            (ComputeQueue::Cpu(q), Buffer::Cpu(d)) => q.enqueue_copy_from_host(d, dst_offset, bytes),
            (ComputeQueue::OpenCl(q), Buffer::OpenCl(d)) => q.enqueue_copy_from_host(d, dst_offset, bytes),
            _ => other(),
        }
    }

    pub fn enqueue_fill_array<T: crate::Pod>(&self, dst: &Buffer, value: &T, offset: usize, count: Option<usize>) {
        match (self, dst) {
            (ComputeQueue::Native(q), Buffer::Native(d)) => q.enqueue_fill_array(d, value, offset, count),
            (ComputeQueue::Cpu(q), Buffer::Cpu(d)) => q.enqueue_fill_array(d, value, offset, count),
            (ComputeQueue::OpenCl(q), Buffer::OpenCl(d)) => q.enqueue_fill_array(d, value, offset, count),
            _ => other(),
        }
    }

    pub fn enqueue_copy_between_device_regions(&self, src: &Buffer, dst: &Buffer, src_offset: u64, dst_offset: u64, length: u64) {
        match (self, src, dst) {
            (ComputeQueue::Native(q), Buffer::Native(s), Buffer::Native(d)) => q.enqueue_copy_between_device_regions(s, d, src_offset, dst_offset, length),
            (ComputeQueue::Cpu(q), Buffer::Cpu(s), Buffer::Cpu(d)) => q.enqueue_copy_between_device_regions(s, d, src_offset, dst_offset, length),
            (ComputeQueue::OpenCl(q), Buffer::OpenCl(s), Buffer::OpenCl(d)) => q.enqueue_copy_between_device_regions(s, d, src_offset, dst_offset, length),
            _ => other(),
        }
    }

    pub fn wait_for_event(&self, event: &SharedEvent, value: u64) {
        match (self, event) {
            (ComputeQueue::Native(q), SharedEvent::Native(e)) => q.wait_for_event(e, value),
            (ComputeQueue::Cpu(q), SharedEvent::Cpu(e)) => q.wait_for_event(e, value),
            (ComputeQueue::OpenCl(q), SharedEvent::OpenCl(e)) => q.wait_for_event(e, value),
            _ => other(),
        }
    }

    pub fn flush(&mut self) -> ComputeEvent {
        match self {
            ComputeQueue::Native(q) => ComputeEvent::Native(q.flush()),
            ComputeQueue::Cpu(q) => ComputeEvent::Cpu(q.flush()),
            ComputeQueue::OpenCl(q) => ComputeEvent::OpenCl(q.flush()),
        }
    }

    pub fn finish(self) -> Result<(), String> {
        match self {
            ComputeQueue::Native(q) => q.finish(),
            ComputeQueue::Cpu(q) => q.finish(),
            ComputeQueue::OpenCl(q) => q.finish(),
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

/// One launch's arguments.
pub enum BindContext<'a> {
    #[cfg(target_os = "macos")]
    Native(crate::compute_kernel_metal::MetalBindContext),
    #[cfg(target_os = "linux")]
    Native(crate::compute_kernel_cuda::CudaBindContext<'a>),
    Cpu(cpu::CpuBindContext<'a>),
    OpenCl(cl::ClBindContext<'a>),
}

impl crate::compute_kernel_grid::GridBindContext for BindContext<'_> {
    fn set_primitive(&mut self, arg_num: u64, bytes: &[u8]) {
        match self {
            BindContext::Native(c) => c.set_primitive(arg_num, bytes),
            BindContext::Cpu(c) => c.set_primitive(arg_num, bytes),
            BindContext::OpenCl(c) => c.set_primitive(arg_num, bytes),
        }
    }

    fn set_buffer(&mut self, arg_num: u64, buffer: &BufferRef) {
        match (self, buffer) {
            (BindContext::Native(c), Buffer::Native(b)) => c.set_buffer(arg_num, b),
            (BindContext::Cpu(c), Buffer::Cpu(b)) => c.set_buffer(arg_num, b),
            (BindContext::OpenCl(c), Buffer::OpenCl(b)) => c.set_buffer(arg_num, b),
            _ => other(),
        }
    }

    fn set_buffer_at(&mut self, arg_num: u64, buffer: &BufferRef, offset: u64) -> Result<(), String> {
        if offset > buffer.length() {
            return Err(format!("argument {arg_num} bound at byte {offset} of a {}-byte buffer", buffer.length()));
        }
        match (self, buffer) {
            (BindContext::Native(c), Buffer::Native(b)) => c.set_buffer_at(arg_num, b, offset),
            (BindContext::Cpu(c), Buffer::Cpu(b)) => c.set_buffer_at(arg_num, b, offset),
            (BindContext::OpenCl(c), Buffer::OpenCl(b)) => c.set_buffer_at(arg_num, b, offset),
            _ => other(),
        }
    }

    fn set_thread_group_memory(&mut self, arg_num: u64, n_bytes: u64) {
        match self {
            BindContext::Native(c) => c.set_thread_group_memory(arg_num, n_bytes),
            BindContext::Cpu(c) => c.set_thread_group_memory(arg_num, n_bytes),
            BindContext::OpenCl(c) => c.set_thread_group_memory(arg_num, n_bytes),
        }
    }

    fn launch(&mut self, grid: &[u64], block: &[u64]) -> Result<(), String> {
        match self {
            BindContext::Native(c) => c.launch(grid, block),
            BindContext::Cpu(c) => c.launch(grid, block),
            BindContext::OpenCl(c) => c.launch(grid, block),
        }
    }

    fn use_buffer(&mut self, buffer: &BufferRef) {
        match (self, buffer) {
            (BindContext::Native(c), Buffer::Native(b)) => c.use_buffer(b),
            (BindContext::Cpu(c), Buffer::Cpu(b)) => c.use_buffer(b),
            (BindContext::OpenCl(c), Buffer::OpenCl(b)) => c.use_buffer(b),
            _ => other(),
        }
    }

    fn launch_indirect(&mut self, groups: &BufferRef, offset: u64, max_groups: &[u64], block: &[u64]) -> Result<(), String> {
        if offset % 4 != 0 || offset + 12 > groups.length() {
            return Err(format!("an indirect launch's counts at byte {offset} of a {}-byte buffer", groups.length()));
        }
        match (self, groups) {
            (BindContext::Native(c), Buffer::Native(b)) => c.launch_indirect(b, offset, max_groups, block),
            (BindContext::Cpu(c), Buffer::Cpu(b)) => c.launch_indirect(b, offset, max_groups, block),
            (BindContext::OpenCl(c), Buffer::OpenCl(b)) => c.launch_indirect(b, offset, max_groups, block),
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
