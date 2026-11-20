//! The device layer (decision 0440): one interface over the backends, Metal on macOS and CUDA on Linux, an OpenCL
//! GPU when `RERAIL_GPU=opencl`, or the CPU when `RERAIL_GPU=cpu` (`device`); MLDB's compute-kernel runtime (the
//! argument types, the grid launcher that binds a kernel's parameters by name, and each backend's runtime, at their
//! upstream paths under `mldb/`); and the process's shared device (`shared`).
//!
//! It takes compiled or preprocessed kernels and nothing above them: the preprocessor, the kernels compiled from UKL
//! and the library's calls are `rerail-gpu`'s, which re-exports this crate. So the blocks (`rerail-blocks`), below
//! the preprocessor, bind their memory through it (decision 0440 step B).

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
#[path = "mldb/builtin/opencl/compute_kernel_opencl.rs"]
pub mod compute_kernel_opencl;
pub mod device;
mod rows;
pub mod store;

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
// Four words (a launch's parameters, a block program's segment).
unsafe impl Pod for [u32; 4] {}

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
    /// (`device::cpu_chosen`), an OpenCL GPU's when `RERAIL_GPU=opencl`, else the target's GPU's.
    pub fn get() -> Option<&'static Shared> {
        static SHARED: OnceLock<Option<Shared>> = OnceLock::new();
        SHARED
            .get_or_init(|| {
                if crate::device::cpu_chosen() {
                    open_cpu()
                } else if crate::device::opencl_chosen() {
                    open_opencl()
                } else {
                    open()
                }
            })
            .as_ref()
    }

    /// The OpenCL GPU, or none (why, on stderr: a process that asked for OpenCL says why it has no device).
    fn open_opencl() -> Option<Shared> {
        use crate::compute_kernel_opencl::{ClDevice, ClQueueRoot};
        let opened = ClDevice::open().and_then(|d| Ok((ClQueueRoot::new(&d)?, ClQueueRoot::new(&d)?, d)));
        match opened {
            Ok((queue, waiting, device)) => Some(Shared { device: Device::OpenCl(device), queue: QueueRoot::OpenCl(queue), waiting: QueueRoot::OpenCl(waiting) }),
            Err(e) => {
                eprintln!("RERAIL_GPU=opencl: {e}");
                None
            }
        }
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
        // The waiting root first: its stream is the only one an armed wait holds (`CudaQueueRoot`).
        let waiting = QueueRoot::Native(CudaQueueRoot::new(device));
        let queue = QueueRoot::Native(CudaQueueRoot::new(device));
        Some(Shared { device: Device::Native(device), queue, waiting })
    }
}
