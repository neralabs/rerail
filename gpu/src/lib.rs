//! The GPU through UKL (decision 0440): the device layer (`rerail-device`, re-exported here: the backends, MLDB's
//! compute-kernel runtime and the process's shared device), each device's preprocessing (`Preprocessing`), kernels
//! compiled from UKL source at run time with the UKL library on their include path (`kernels`), and the library's
//! device-wide calls (`library`). The library's tests (`library_tests`) run each primitive on the device against its reference.
//! The forest's trainer runs on it; the engine's GPU lane moves onto it (0440 step 3).

#![cfg(any(target_os = "macos", target_os = "linux", windows))]

// The device layer, below the preprocessor (`rerail-device`): its paths are this crate's too.
pub use rerail_device::{compute_kernel, compute_kernel_cpu, compute_kernel_grid, compute_kernel_opencl, device, os, shared, store, Pod};
/// The preprocessor and its targets (decision 0645), for a caller that names one.
pub use rerail_ukl as ukl;
#[cfg(not(target_os = "macos"))]
pub use rerail_device::compute_kernel_cuda;
#[cfg(target_os = "macos")]
pub use rerail_device::compute_kernel_metal;
pub mod kernels;
pub mod library;
pub mod vec;
#[cfg(test)]
mod device_tests;
#[cfg(test)]
mod library_tests;
#[cfg(test)]
mod matrix_tests;

/// A device's preprocessing of UKL (`rerail_ukl`), which the device layer, below the preprocessor, does not do.
pub trait Preprocessing {
    /// What this device's kernels' text is prepared for (decision 0645): its target's name's
    /// (`device::Device::target_name`), else, on OpenCL and the CPU, what the device reports.
    fn target(&self) -> rerail_ukl::Target;
    /// What `rerail_ukl` preprocesses this device's kernels for.
    fn ukl_backend(&self) -> rerail_ukl::Backend;
    /// A preprocessing of UKL for this device: its target's (decision 0645, `device::Device::target`).
    fn preprocess(&self) -> rerail_ukl::Preprocess;
}

impl Preprocessing for device::Device {
    fn target(&self) -> rerail_ukl::Target {
        let local = |backend| rerail_ukl::Target::local(backend, self.local_defines().into_iter().map(|(n, v)| (n.to_string(), v)).collect());
        match self {
            device::Device::Cpu(_) => local(rerail_ukl::Backend::Cpu),
            device::Device::OpenCl(_) => local(rerail_ukl::Backend::OpenCl),
            device::Device::Native(d) => {
                let name = self.target_name().expect("a native device's target");
                let named = rerail_ukl::Target::named(&name).unwrap_or_else(|| panic!("{name}: no target of that name"));
                // HIP's wave is the name's (`Target::named`); a device that runs another takes a local target, its
                // text no shipped one's.
                #[cfg(not(target_os = "macos"))]
                if d.vendor == compute_kernel_cuda::Vendor::Hip && named.backend != (rerail_ukl::Backend::Hip { wave: d.simd_width }) {
                    return local(rerail_ukl::Backend::Hip { wave: d.simd_width });
                }
                #[cfg(target_os = "macos")]
                let _ = d;
                named
            }
        }
    }

    fn ukl_backend(&self) -> rerail_ukl::Backend {
        self.target().backend
    }

    fn preprocess(&self) -> rerail_ukl::Preprocess {
        self.target().preprocess()
    }
}
