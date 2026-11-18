//! The forest's device, by target (Rerail, decision 0400 step 3): Metal on macOS (compute_kernel_metal.rs), CUDA on
//! Linux (compute_kernel_cuda.rs). Both give the grid runtime and the trainer the same interface (MLDB's
//! `ComputeRuntime` without its runtime choice: one backend a target, for now), under these names:
//!
//! - `Device`, `DeviceRef`: the device (`gpu::shared`'s); `QueueRoot`: what a `ComputeQueue` is made from;
//! - `Buffer`, `BufferRef`: memory the host reads and writes in place (`contents`) and kernels read;
//! - `ComputeQueue`, `ComputeEvent`: launches, copies and fills in order, and the event a flush gives;
//! - `ComputeFunctionLibrary`, `ComputeFunction`: compiled kernels, with their arguments;
//! - `SharedEvent`: a value the host signals and a queue waits for;
//! - the array helpers and `gpu_seconds_awaited`.

pub use crate::compute_kernel::*;

#[cfg(target_os = "macos")]
mod imp {
    pub use crate::compute_kernel_metal::{
        alloc_uninitialized_array_sync, gpu_seconds_awaited, transfer_to_device_immutable, transfer_to_host_sync, MetalBindContext as BindContext, MetalComputeEvent as ComputeEvent,
        MetalComputeFunction as ComputeFunction, MetalComputeFunctionLibrary as ComputeFunctionLibrary, MetalComputeQueue as ComputeQueue,
    };
    pub type Device = metal::Device;
    pub type DeviceRef = metal::DeviceRef;
    pub type Buffer = metal::Buffer;
    pub type BufferRef = metal::BufferRef;
    pub type QueueRoot = metal::CommandQueue;
    pub type SharedEvent = metal::SharedEvent;
    /// The backend's name, for messages.
    pub const BACKEND: &str = "Metal";
}

#[cfg(target_os = "linux")]
mod imp {
    pub use crate::compute_kernel_cuda::{
        alloc_uninitialized_array_sync, gpu_seconds_awaited, transfer_to_device_immutable, transfer_to_host_sync, CudaBindContext as BindContext, CudaBuffer as Buffer,
        CudaBufferRef as BufferRef, CudaComputeEvent as ComputeEvent, CudaComputeFunction as ComputeFunction, CudaComputeFunctionLibrary as ComputeFunctionLibrary,
        CudaComputeQueue as ComputeQueue, CudaDevice as Device, CudaDeviceRef as DeviceRef, CudaQueueRoot as QueueRoot, CudaSharedEvent as SharedEvent,
    };
    /// The backend's name, for messages.
    pub const BACKEND: &str = "CUDA";
}

pub use imp::*;
