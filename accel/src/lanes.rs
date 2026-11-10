//! The compute lanes, and which of them this machine has.

/// A compute lane: a unit the executor submits work to, which runs beside the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lane {
    /// The CPU's cores (NEON), on the rayon pool.
    Cpu,
    /// The CPU's matrix unit (SME2): one per cluster, shared by the cluster's cores.
    Sme,
    /// The GPU (Metal compute).
    Gpu,
    /// The Neural Engine (Core ML).
    Ane,
}

impl Lane {
    pub const ALL: [Lane; 4] = [Lane::Cpu, Lane::Sme, Lane::Gpu, Lane::Ane];

    pub fn name(self) -> &'static str {
        match self {
            Lane::Cpu => "cpu",
            Lane::Sme => "sme",
            Lane::Gpu => "gpu",
            Lane::Ane => "ane",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Which lanes this machine has: the CPU always; SME when the CPU reports SME2; the GPU when Metal has a device; the
/// Neural Engine on Apple silicon running macOS (Core ML decides placement per model, and the `ane` lane checks it).
pub fn present() -> [bool; 4] {
    [true, sme2(), gpu(), ane()]
}

/// `hw.optional.arm.FEAT_SME2`.
pub fn sme2() -> bool {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        sysctl_flag(c"hw.optional.arm.FEAT_SME2")
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        false
    }
}

fn gpu() -> bool {
    #[cfg(target_os = "macos")]
    {
        metal::Device::system_default().is_some()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

fn ane() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

/// The number of CPU clusters with an SME unit: `hw.nperflevels` levels, one SME unit per cluster of each level's
/// `hw.perflevel<k>.cpusperl2` cores (an L2 is shared by a cluster).
pub fn sme_units() -> usize {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        if !sme2() {
            return 0;
        }
        let levels = sysctl_u32(c"hw.nperflevels").unwrap_or(1);
        let mut units = 0;
        for k in 0..levels {
            let name = |s: &str| std::ffi::CString::new(format!("hw.perflevel{k}.{s}")).unwrap();
            let cores = sysctl_u32(&name("logicalcpu")).unwrap_or(0);
            let per = sysctl_u32(&name("cpusperl2")).unwrap_or(cores.max(1)).max(1);
            units += cores.div_ceil(per) as usize;
        }
        units.max(1)
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        0
    }
}

#[cfg(target_os = "macos")]
fn sysctl_u32(name: &std::ffi::CStr) -> Option<u32> {
    let mut v: u32 = 0;
    let mut len = std::mem::size_of::<u32>();
    let rc = unsafe { libc::sysctlbyname(name.as_ptr(), (&mut v as *mut u32).cast(), &mut len, std::ptr::null_mut(), 0) };
    (rc == 0).then_some(v)
}

#[cfg(target_os = "macos")]
fn sysctl_flag(name: &std::ffi::CStr) -> bool {
    sysctl_u32(name).is_some_and(|v| v != 0)
}
