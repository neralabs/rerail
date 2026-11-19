//! The GPU device layer (decision 0400): one interface over the backends, Metal on macOS and CUDA on Linux, or the CPU
//! when `RERAIL_GPU=cpu` (`device`),
//! MLDB's compute-kernel runtime (the argument types, the grid launcher that binds a kernel's parameters by name, and
//! each backend's runtime, at their upstream paths under `mldb/`), the process's shared device, kernels compiled
//! from UKL source at run time with the UKL library on their include path (`kernels`), and the library's device-wide
//! calls (`library`). The library's tests (`library_tests`) run each primitive on the device against its reference.
//! The forest's trainer runs on it; the engine's GPU lane moves onto it (0400 step 3).

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
#[path = "mldb/builtin/cpu/compute_kernel_cpu.rs"]
pub mod compute_kernel_cpu;
pub mod device;
pub mod kernels;
pub mod library;
mod rows;
#[cfg(test)]
mod library_tests;

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
/// On CUDA `queue` and `waiting` are sets of streams made here, one hardware queue each (`CudaQueueRoot`); on the CPU
/// a thread each (`CpuQueueRoot`).
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

    /// The process's device and queue: `None` without a device. The CPU's when `RERAIL_GPU=cpu`
    /// (`device::cpu_chosen`), else the GPU's.
    pub fn get() -> Option<&'static Shared> {
        static SHARED: OnceLock<Option<Shared>> = OnceLock::new();
        SHARED.get_or_init(|| if crate::device::cpu_chosen() { open_cpu() } else { open() }).as_ref()
    }

    fn open_cpu() -> Option<Shared> {
        use crate::compute_kernel_cpu::{CpuDevice, CpuQueueRoot};
        let device = CpuDevice::system_default()?;
        let (queue, waiting) = (QueueRoot::Cpu(CpuQueueRoot::new(device)), QueueRoot::Cpu(CpuQueueRoot::new(device)));
        Some(Shared { device: Device::Cpu(device), queue, waiting })
    }

    #[cfg(target_os = "macos")]
    fn open() -> Option<Shared> {
        objc::rc::autoreleasepool(|| {
            let device = metal::Device::system_default()?;
            let queue = QueueRoot::Native(device.new_command_queue());
            let waiting = QueueRoot::Native(device.new_command_queue());
            Some(Shared { device: Device::Native(device), queue, waiting })
        })
    }

    #[cfg(target_os = "linux")]
    fn open() -> Option<Shared> {
        use crate::compute_kernel_cuda::{CudaDevice, CudaQueueRoot};
        let device = CudaDevice::system_default()?;
        // The waiting root first: its one stream is the only one an armed wait holds (`CudaQueueRoot`).
        let waiting = QueueRoot::Native(CudaQueueRoot::new(device, 1));
        let queue = QueueRoot::Native(CudaQueueRoot::new(device, 16));
        Some(Shared { device: Device::Native(device), queue, waiting })
    }
}
