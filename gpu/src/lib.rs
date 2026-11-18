//! The GPU device layer (decision 0400): one interface over the backends, Metal on macOS and CUDA on Linux (`device`),
//! MLDB's compute-kernel runtime (the argument types, the grid launcher that binds a kernel's parameters by name, and
//! each backend's runtime, at their upstream paths under `mldb/`), the process's shared device, and kernels compiled
//! from UKL source with the UKL libraries (`kernels`). The forest's trainer and the UKL libraries' tests run on it;
//! the engine's GPU lane moves onto it (0400 step 3).

#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "mldb/block/compute_kernel.rs"]
pub mod compute_kernel;
#[path = "mldb/block/compute_kernel_grid.rs"]
pub mod compute_kernel_grid;
#[cfg(target_os = "macos")]
#[path = "mldb/builtin/metal/compute_kernel_metal.rs"]
pub mod compute_kernel_metal;
#[cfg(target_os = "linux")]
#[path = "mldb/builtin/cuda/compute_kernel_cuda.rs"]
pub mod compute_kernel_cuda;
pub mod device;

/// A plain value a buffer can be viewed as: any bit pattern is a value, no padding read as data, no pointers.
///
/// # Safety
/// Implementors must be `Copy`, without padding that is read, and valid for every bit pattern.
pub unsafe trait Pod: Copy + 'static {}
macro_rules! pod {
    ($($t:ty),*) => {$( unsafe impl Pod for $t {} )*};
}
pod!(u8, i8, u16, i16, u32, i32, u64, i64, f32, f64);

// Pairs of floats (a row's weights to false and true, in the forest).
unsafe impl Pod for [f32; 2] {}

/// The process's one device and command queue. They are made here, in the lowest crate that uses the device, so
/// that the forest's training and prediction and the engine's every submission (`rerail_accel::gpu::shared`, which
/// takes them from here on Metal) share one queue: one order of command buffers, no queue apart (decision 0100 §2).
/// On CUDA `queue` and `waiting` are sets of streams made here, one hardware queue each (`CudaQueueRoot`).
pub mod shared {
    use crate::device::{Device, QueueRoot};
    use std::sync::OnceLock;

    pub struct Shared {
        pub device: Device,
        pub queue: QueueRoot,
        /// The queue of work committed ahead that waits on an event (the forest's armed prediction): a command
        /// buffer waiting on an event holds back the ones its queue commits after it, so it is not `queue`.
        pub waiting: QueueRoot,
    }

    /// The process's device and queue: `None` without a device.
    pub fn get() -> Option<&'static Shared> {
        static SHARED: OnceLock<Option<Shared>> = OnceLock::new();
        SHARED.get_or_init(open).as_ref()
    }

    #[cfg(target_os = "macos")]
    fn open() -> Option<Shared> {
        objc::rc::autoreleasepool(|| {
            let device = metal::Device::system_default()?;
            let queue = device.new_command_queue();
            let waiting = device.new_command_queue();
            Some(Shared { device, queue, waiting })
        })
    }

    #[cfg(target_os = "linux")]
    fn open() -> Option<Shared> {
        use crate::compute_kernel_cuda::{CudaDevice, CudaQueueRoot};
        let device = CudaDevice::system_default()?;
        // The waiting root first: its one stream is the only one an armed wait holds (`CudaQueueRoot`).
        let waiting = CudaQueueRoot::new(device, 1);
        let queue = CudaQueueRoot::new(device, 16);
        Some(Shared { device, queue, waiting })
    }
}
