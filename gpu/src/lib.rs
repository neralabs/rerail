//! The GPU through UKL (decision 0440): the device layer (`rerail-device`, re-exported here: the backends, MLDB's
//! compute-kernel runtime and the process's shared device), each device's preprocessing (`Preprocessing`), kernels
//! compiled from UKL source at run time with the UKL library on their include path (`kernels`), and the library's
//! device-wide calls (`library`). The library's tests (`library_tests`) run each primitive on the device against its reference.
//! The forest's trainer runs on it; the engine's GPU lane moves onto it (0440 step 3).

#![cfg(any(target_os = "macos", target_os = "linux", windows))]

// The device layer, below the preprocessor (`rerail-device`): its paths are this crate's too.
pub use rerail_device::{compute_kernel, compute_kernel_cpu, compute_kernel_grid, compute_kernel_opencl, device, os, shared, store, Pod};
#[cfg(not(target_os = "macos"))]
pub use rerail_device::compute_kernel_cuda;
#[cfg(target_os = "macos")]
pub use rerail_device::compute_kernel_metal;
pub mod kernels;
pub mod library;
#[cfg(test)]
mod device_tests;
#[cfg(test)]
mod library_tests;

/// A device's preprocessing of UKL (`rerail_ukl`), which the device layer, below the preprocessor, does not do.
pub trait Preprocessing {
    /// What `rerail_ukl` preprocesses this device's kernels for.
    fn ukl_backend(&self) -> rerail_ukl::Backend;
    /// A preprocessing of UKL for this device: its backend, and the defines its compile takes (the CPU's knobs,
    /// `compute_kernel_cpu::defines`; OpenCL's width and largest threadgroup).
    fn preprocess(&self) -> rerail_ukl::Preprocess;
}

impl Preprocessing for device::Device {
    fn ukl_backend(&self) -> rerail_ukl::Backend {
        match self {
            device::Device::Cpu(_) => rerail_ukl::Backend::Cpu,
            device::Device::OpenCl(_) => rerail_ukl::Backend::OpenCl,
            #[cfg(target_os = "macos")]
            device::Device::Native(_) => rerail_ukl::Backend::Metal { version: 310 },
            #[cfg(not(target_os = "macos"))]
            device::Device::Native(d) => match d.vendor {
                compute_kernel_cuda::Vendor::Cuda => rerail_ukl::Backend::Cuda { sm: d.sm },
                compute_kernel_cuda::Vendor::Hip => rerail_ukl::Backend::Hip { wave: d.simd_width },
            },
        }
    }

    fn preprocess(&self) -> rerail_ukl::Preprocess {
        let mut p = rerail_ukl::Preprocess::new(self.ukl_backend());
        for (name, value) in self.compile_defines() {
            p = p.define(name, Some(&value));
        }
        p
    }
}
