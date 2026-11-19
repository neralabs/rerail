//! The forest's device (Rerail, decision 0400): the target's GPU (Metal on macOS, compute_kernel_metal.rs; CUDA on
//! Linux, compute_kernel_cuda.rs) or the CPU (compute_kernel_cpu.rs: every threadgroup's threads coroutines, for
//! debugging kernels), chosen once a process: the CPU when `RERAIL_GPU=cpu`, else the GPU. Each type below is one or
//! the other, and gives the grid runtime and the trainer one interface (MLDB's `ComputeRuntime`):
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

#[cfg(target_os = "macos")]
mod native {
    pub use crate::compute_kernel_metal::{
        alloc_uninitialized_array_sync, gpu_seconds_awaited, transfer_to_device_immutable, transfer_to_host_sync, MetalComputeEvent as Event, MetalComputeFunction as Function,
        MetalComputeFunctionLibrary as Library, MetalComputeQueue as Queue,
    };
    pub type Device = metal::Device;
    pub type Buffer = metal::Buffer;
    pub type QueueRoot = metal::CommandQueue;
    pub type SharedEvent = metal::SharedEvent;
    pub const NAME: &str = "Metal";
}

#[cfg(target_os = "linux")]
mod native {
    pub use crate::compute_kernel_cuda::{
        alloc_uninitialized_array_sync, gpu_seconds_awaited, transfer_to_device_immutable, transfer_to_host_sync, CudaBuffer as Buffer, CudaComputeEvent as Event,
        CudaComputeFunction as Function, CudaComputeFunctionLibrary as Library, CudaComputeQueue as Queue, CudaDevice as Device, CudaQueueRoot as QueueRoot,
        CudaSharedEvent as SharedEvent,
    };
    pub const NAME: &str = "CUDA";
}

/// Whether this process runs its kernels on the CPU (`RERAIL_GPU=cpu`).
pub fn cpu_chosen() -> bool {
    static CPU: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CPU.get_or_init(|| std::env::var("RERAIL_GPU").is_ok_and(|v| v.eq_ignore_ascii_case("cpu")))
}

/// The lanes of the process's device's SIMD groups (`UKL_SIMD_WIDTH`).
pub fn simd_width() -> u32 {
    crate::shared::get().map_or(32, |s| s.device.simd_width())
}

/// The backend's name, for messages: the process's device's.
pub fn backend() -> &'static str {
    match crate::shared::get().map(|s| &s.device) {
        Some(Device::Cpu(_)) => "CPU",
        Some(Device::Native(_)) => native::NAME,
        None if cpu_chosen() => "CPU",
        None => native::NAME,
    }
}

fn other() -> ! {
    panic!("a value of one backend given to the other's (the GPU's and the CPU's)")
}

/// The device.
#[derive(Clone)]
pub enum Device {
    Native(native::Device),
    Cpu(cpu::CpuDevice),
}

pub type DeviceRef = Device;

impl Device {
    /// The Metal device, when the process's is Metal (the engine's own Metal kernels need it).
    #[cfg(target_os = "macos")]
    pub fn metal(&self) -> Option<&metal::Device> {
        match self {
            Device::Native(d) => Some(d),
            Device::Cpu(_) => None,
        }
    }

    /// The CUDA device, when the process's is CUDA.
    #[cfg(target_os = "linux")]
    pub fn cuda(&self) -> Option<&crate::compute_kernel_cuda::CudaDevice> {
        match self {
            Device::Native(d) => Some(d),
            Device::Cpu(_) => None,
        }
    }

    /// The lanes of the device's SIMD groups, as its kernels are compiled (`UKL_SIMD_WIDTH`).
    pub fn simd_width(&self) -> u32 {
        match self {
            Device::Native(_) => 32,
            Device::Cpu(_) => cpu::simd_width(),
        }
    }

    pub fn is_cpu(&self) -> bool {
        matches!(self, Device::Cpu(_))
    }

    /// What `rerail_ukl` preprocesses this device's kernels for.
    pub fn ukl_backend(&self) -> rerail_ukl::Backend {
        match self {
            Device::Cpu(_) => rerail_ukl::Backend::Cpu,
            #[cfg(target_os = "macos")]
            Device::Native(_) => rerail_ukl::Backend::Metal { version: 310 },
            #[cfg(target_os = "linux")]
            Device::Native(d) => rerail_ukl::Backend::Cuda { sm: d.sm },
        }
    }

    /// A preprocessing of UKL for this device: its backend, and the defines its compile takes (the CPU's knobs,
    /// `compute_kernel_cpu::defines`).
    pub fn preprocess(&self) -> rerail_ukl::Preprocess {
        let mut p = rerail_ukl::Preprocess::new(self.ukl_backend());
        if let Device::Cpu(_) = self {
            for (name, value) in cpu::defines() {
                p = p.define(name, Some(&value));
            }
        }
        p
    }

    pub fn new_shared_event(&self) -> SharedEvent {
        match self {
            Device::Native(d) => SharedEvent::Native(d.new_shared_event()),
            Device::Cpu(d) => SharedEvent::Cpu(d.new_shared_event()),
        }
    }
}

/// Memory the host and the kernels read in place.
#[derive(Clone)]
pub enum Buffer {
    Native(native::Buffer),
    Cpu(cpu::CpuBuffer),
}

pub type BufferRef = Buffer;

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Buffer::Native(_) => write!(f, "Buffer({} bytes)", self.length()),
            Buffer::Cpu(b) => b.fmt(f),
        }
    }
}

impl Buffer {
    pub fn contents(&self) -> *mut std::ffi::c_void {
        match self {
            Buffer::Native(b) => b.contents(),
            Buffer::Cpu(b) => b.contents(),
        }
    }

    pub fn length(&self) -> u64 {
        match self {
            Buffer::Native(b) => b.length(),
            Buffer::Cpu(b) => b.length(),
        }
    }

    /// The Metal buffer, when the process's device is Metal.
    #[cfg(target_os = "macos")]
    pub fn metal(&self) -> Option<&metal::Buffer> {
        match self {
            Buffer::Native(b) => Some(b),
            Buffer::Cpu(_) => None,
        }
    }
}

/// A value the host signals and a queue waits for.
#[derive(Clone)]
pub enum SharedEvent {
    Native(native::SharedEvent),
    Cpu(cpu::CpuSharedEvent),
}

impl SharedEvent {
    pub fn set_signaled_value(&self, v: u64) {
        match self {
            SharedEvent::Native(e) => e.set_signaled_value(v),
            SharedEvent::Cpu(e) => e.set_signaled_value(v),
        }
    }

    pub fn signaled_value(&self) -> u64 {
        match self {
            SharedEvent::Native(e) => e.signaled_value(),
            SharedEvent::Cpu(e) => e.signaled_value(),
        }
    }
}

/// What a `ComputeQueue` is made from.
pub enum QueueRoot {
    Native(native::QueueRoot),
    Cpu(cpu::CpuQueueRoot),
}

impl QueueRoot {
    /// The Metal command queue, when the process's device is Metal.
    #[cfg(target_os = "macos")]
    pub fn metal(&self) -> Option<&metal::CommandQueue> {
        match self {
            QueueRoot::Native(q) => Some(q),
            QueueRoot::Cpu(_) => None,
        }
    }
}

/// Launches, copies and fills, in order.
pub enum ComputeQueue {
    Native(native::Queue),
    Cpu(cpu::CpuComputeQueue),
}

impl ComputeQueue {
    pub fn new(root: &QueueRoot) -> Self {
        match root {
            QueueRoot::Native(r) => ComputeQueue::Native(native::Queue::new(r)),
            QueueRoot::Cpu(r) => ComputeQueue::Cpu(cpu::CpuComputeQueue::new(r)),
        }
    }

    pub fn new_bind_context<'a>(&'a self, function: &'a ComputeFunction) -> BindContext<'a> {
        match (self, function) {
            (ComputeQueue::Native(q), ComputeFunction::Native(f)) => BindContext::Native(q.new_bind_context(f)),
            (ComputeQueue::Cpu(q), ComputeFunction::Cpu(f)) => BindContext::Cpu(q.new_bind_context(f)),
            _ => other(),
        }
    }

    pub fn enqueue_copy_from_host(&self, dst: &Buffer, dst_offset: u64, bytes: &[u8]) {
        match (self, dst) {
            (ComputeQueue::Native(q), Buffer::Native(d)) => q.enqueue_copy_from_host(d, dst_offset, bytes),
            (ComputeQueue::Cpu(q), Buffer::Cpu(d)) => q.enqueue_copy_from_host(d, dst_offset, bytes),
            _ => other(),
        }
    }

    pub fn enqueue_fill_array<T: crate::Pod>(&self, dst: &Buffer, value: &T, offset: usize, count: Option<usize>) {
        match (self, dst) {
            (ComputeQueue::Native(q), Buffer::Native(d)) => q.enqueue_fill_array(d, value, offset, count),
            (ComputeQueue::Cpu(q), Buffer::Cpu(d)) => q.enqueue_fill_array(d, value, offset, count),
            _ => other(),
        }
    }

    pub fn enqueue_copy_between_device_regions(&self, src: &Buffer, dst: &Buffer, src_offset: u64, dst_offset: u64, length: u64) {
        match (self, src, dst) {
            (ComputeQueue::Native(q), Buffer::Native(s), Buffer::Native(d)) => q.enqueue_copy_between_device_regions(s, d, src_offset, dst_offset, length),
            (ComputeQueue::Cpu(q), Buffer::Cpu(s), Buffer::Cpu(d)) => q.enqueue_copy_between_device_regions(s, d, src_offset, dst_offset, length),
            _ => other(),
        }
    }

    pub fn wait_for_event(&self, event: &SharedEvent, value: u64) {
        match (self, event) {
            (ComputeQueue::Native(q), SharedEvent::Native(e)) => q.wait_for_event(e, value),
            (ComputeQueue::Cpu(q), SharedEvent::Cpu(e)) => q.wait_for_event(e, value),
            _ => other(),
        }
    }

    pub fn flush(&mut self) -> ComputeEvent {
        match self {
            ComputeQueue::Native(q) => ComputeEvent::Native(q.flush()),
            ComputeQueue::Cpu(q) => ComputeEvent::Cpu(q.flush()),
        }
    }

    pub fn finish(self) -> Result<(), String> {
        match self {
            ComputeQueue::Native(q) => q.finish(),
            ComputeQueue::Cpu(q) => q.finish(),
        }
    }
}

/// The work a queue flushed.
pub enum ComputeEvent {
    Native(native::Event),
    Cpu(cpu::CpuComputeEvent),
}

impl ComputeEvent {
    pub fn await_(&self) -> Result<(), String> {
        match self {
            ComputeEvent::Native(e) => e.await_(),
            ComputeEvent::Cpu(e) => e.await_(),
        }
    }

    pub fn is_complete(&self) -> bool {
        match self {
            ComputeEvent::Native(e) => e.is_complete(),
            ComputeEvent::Cpu(e) => e.is_complete(),
        }
    }

    pub fn gpu_seconds(&self) -> f64 {
        match self {
            ComputeEvent::Native(e) => e.gpu_seconds(),
            ComputeEvent::Cpu(e) => e.gpu_seconds(),
        }
    }
}

/// Compiled kernels.
pub enum ComputeFunctionLibrary {
    Native(native::Library),
    Cpu(cpu::CpuComputeFunctionLibrary),
}

impl ComputeFunctionLibrary {
    /// `preprocessed_source`, UKL preprocessed for `device` (`Device::ukl_backend`), compiled.
    pub fn compile_from_source(device: &Device, preprocessed_source: &str, file_name_to_appear_in_error_messages: &str) -> Result<Self, String> {
        Ok(match device {
            Device::Native(d) => ComputeFunctionLibrary::Native(native::Library::compile_from_source(d, preprocessed_source, file_name_to_appear_in_error_messages)?),
            Device::Cpu(d) => ComputeFunctionLibrary::Cpu(cpu::CpuComputeFunctionLibrary::compile_from_source(d, preprocessed_source, file_name_to_appear_in_error_messages)?),
        })
    }

    pub fn get_function(&self, device: &Device, function_name: &str) -> Result<ComputeFunction, String> {
        Ok(match (self, device) {
            (ComputeFunctionLibrary::Native(l), Device::Native(d)) => ComputeFunction::Native(l.get_function(d, function_name)?),
            (ComputeFunctionLibrary::Cpu(l), Device::Cpu(d)) => ComputeFunction::Cpu(l.get_function(d, function_name)?),
            _ => other(),
        })
    }
}

/// A kernel, with its arguments.
pub enum ComputeFunction {
    Native(native::Function),
    Cpu(cpu::CpuComputeFunction),
}

impl ComputeFunction {
    pub fn get_argument_info(&self) -> &[GridComputeFunctionArgument] {
        match self {
            ComputeFunction::Native(f) => f.get_argument_info(),
            ComputeFunction::Cpu(f) => f.get_argument_info(),
        }
    }

    /// The Metal function, when the process's device is Metal.
    #[cfg(target_os = "macos")]
    pub fn metal(&self) -> Option<&crate::compute_kernel_metal::MetalComputeFunction> {
        match self {
            ComputeFunction::Native(f) => Some(f),
            ComputeFunction::Cpu(_) => None,
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
}

impl crate::compute_kernel_grid::GridBindContext for BindContext<'_> {
    fn set_primitive(&mut self, arg_num: u64, bytes: &[u8]) {
        match self {
            BindContext::Native(c) => c.set_primitive(arg_num, bytes),
            BindContext::Cpu(c) => c.set_primitive(arg_num, bytes),
        }
    }

    fn set_buffer(&mut self, arg_num: u64, buffer: &BufferRef) {
        match (self, buffer) {
            (BindContext::Native(c), Buffer::Native(b)) => c.set_buffer(arg_num, b),
            (BindContext::Cpu(c), Buffer::Cpu(b)) => c.set_buffer(arg_num, b),
            _ => other(),
        }
    }

    fn set_thread_group_memory(&mut self, arg_num: u64, n_bytes: u64) {
        match self {
            BindContext::Native(c) => c.set_thread_group_memory(arg_num, n_bytes),
            BindContext::Cpu(c) => c.set_thread_group_memory(arg_num, n_bytes),
        }
    }

    fn launch(&mut self, grid: &[u64], block: &[u64]) -> Result<(), String> {
        match self {
            BindContext::Native(c) => c.launch(grid, block),
            BindContext::Cpu(c) => c.launch(grid, block),
        }
    }
}

/// `allocUninitializedArraySync`: memory for `length` elements (at least 16 bytes).
pub fn alloc_uninitialized_array_sync<T>(device: &Device, length: usize) -> Buffer {
    match device {
        Device::Native(d) => Buffer::Native(native::alloc_uninitialized_array_sync::<T>(d, length)),
        Device::Cpu(d) => Buffer::Cpu(cpu::alloc_uninitialized_array_sync::<T>(d, length)),
    }
}

/// `transferToDeviceImmutable`: memory holding `data`.
pub fn transfer_to_device_immutable<T: crate::Pod>(device: &Device, data: &[T]) -> Buffer {
    match device {
        Device::Native(d) => Buffer::Native(native::transfer_to_device_immutable(d, data)),
        Device::Cpu(d) => Buffer::Cpu(cpu::transfer_to_device_immutable(d, data)),
    }
}

/// `transferToHostSync`: the first `length` elements (the work writing them must have completed).
pub fn transfer_to_host_sync<T: crate::Pod + Clone>(buffer: &Buffer, length: usize) -> Vec<T> {
    match buffer {
        Buffer::Native(b) => native::transfer_to_host_sync(b, length),
        Buffer::Cpu(b) => cpu::transfer_to_host_sync(b, length),
    }
}

/// The device seconds of every event awaited so far in this process.
pub fn gpu_seconds_awaited() -> f64 {
    native::gpu_seconds_awaited() + cpu::gpu_seconds_awaited()
}
