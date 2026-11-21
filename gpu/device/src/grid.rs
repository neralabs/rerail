//! A persistent grid's groups (decision 0568): how many groups of a kernel the device runs at once, so a grid of
//! them meets its barriers (`ukl/grid.h`).
//!
//! - Where the backend answers it, its answer: CUDA's occupancy of the function (`cuOccupancyMaxActiveBlocksPerMultiprocessor`,
//!   its registers and shared memory, times the multiprocessors), the CPU's own rule (`compute_kernel_cpu::resident_groups`).
//! - Metal and OpenCL have no such call. There it is measured: a grid run at a count, and the count it met its
//!   barriers at is kept for the machine, in the kernel store (`grid/<key>.txt`), keyed by the device, the kernel and
//!   the group's threads. A failed barrier halves it (`halved`), and the half is kept. Until a measure is kept, the
//!   device's bound (`Device::resident_groups`: its cores times the groups a core holds at its least) is the first
//!   count tried.
//!
//! The module keeps nothing in memory (decision 0442): its caller holds the machine's answer.

use crate::device::{ComputeFunction, Device};

/// What a kernel's resident groups are known from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Known {
    /// The backend's own answer.
    Api,
    /// A measure kept for this machine.
    Measured,
    /// No answer and no measure: the device's bound, an upper bound, to be measured.
    Bound,
}

/// A kernel's resident groups, and what they are known from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Residency {
    pub groups: u64,
    pub known: Known,
}

/// The store's key of a measure: the device, the kernel (`kernel_key`, its source and name as its caller keys it),
/// and the group's threads.
fn key(device: &Device, kernel_key: &str, threads: u64) -> String {
    crate::store::key(&[b"grid", device.name().as_bytes(), device.gpu_cores().unwrap_or(0).to_string().as_bytes(), kernel_key.as_bytes(), threads.to_string().as_bytes()])
}

/// The backend's answer for `kernel`'s groups of `threads` threads, where it has one.
pub fn api(device: &Device, kernel: &ComputeFunction, threads: u64) -> Option<u64> {
    match (device, kernel) {
        (Device::Cpu(_), _) => Some(crate::compute_kernel_cpu::resident_groups(threads as usize) as u64),
        #[cfg(not(target_os = "macos"))]
        (Device::Native(d), ComputeFunction::Native(f)) => f.resident_groups(d, threads as u32, 0).ok().filter(|&g| g > 0),
        _ => None,
    }
}

/// The measure kept for `kernel_key`'s groups of `threads` threads on this machine.
pub fn kept(device: &Device, kernel_key: &str, threads: u64) -> Option<u64> {
    let bytes = crate::store::load("grid", &key(device, kernel_key, threads), "txt")?;
    std::str::from_utf8(&bytes).ok()?.trim().parse().ok().filter(|&g| g > 0)
}

/// Keeps `groups` as the measure for `kernel_key`'s groups of `threads` threads on this machine.
pub fn keep(device: &Device, kernel_key: &str, threads: u64, groups: u64) {
    crate::store::save("grid", &key(device, kernel_key, threads), "txt", groups.max(1).to_string().as_bytes());
}

/// `kernel`'s resident groups of `threads` threads, each using `local` bytes of threadgroup memory: the backend's
/// answer, else the measure kept, else the device's bound (8 groups where it has none), to be measured.
pub fn residency(device: &Device, kernel: &ComputeFunction, kernel_key: &str, threads: u64, local: u64) -> Residency {
    if let Some(groups) = api(device, kernel, threads) {
        return Residency { groups, known: Known::Api };
    }
    if let Some(groups) = kept(device, kernel_key, threads) {
        return Residency { groups, known: Known::Measured };
    }
    Residency { groups: device.resident_groups(threads as u32, local).unwrap_or(8).max(1), known: Known::Bound }
}

/// After a grid of `groups` failed a barrier: half of it, kept as the measure (never below 1). Only a measure or the
/// bound is halved: a caller does not call it where the backend answered (`Known::Api`), whose count is resident.
pub fn halved(device: &Device, kernel_key: &str, threads: u64, groups: u64) -> u64 {
    let half = (groups / 2).max(1);
    keep(device, kernel_key, threads, half);
    half
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A measure kept is read back, and a halving keeps the half.
    #[test]
    fn a_measure_is_kept_and_a_halving_keeps_the_half() {
        // The store is the user's (`RERAIL_GPU_KERNELS`): a key of this run's own.
        let Some(device) = crate::compute_kernel_cpu::CpuDevice::system_default().map(Device::Cpu) else { return };
        let k = format!("grid test {} {}", std::process::id(), crate::store::unique());
        keep(&device, &k, 256, 48);
        if crate::store::dir().is_none() {
            return;
        }
        assert_eq!(kept(&device, &k, 256), Some(48));
        assert_eq!(halved(&device, &k, 256, 48), 24);
        assert_eq!(kept(&device, &k, 256), Some(24));
        assert_eq!(halved(&device, &k, 256, 1), 1, "never below one group");
    }
}
