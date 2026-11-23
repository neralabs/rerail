//! A target (decision 0645): what a kernel's text is prepared for, named, so that a build prepares for a target the
//! same text a device of that target prepares for itself.
//!
//! A shippable target's defines are a function of its name alone: `Target::named` is what a build takes, and a
//! device's own target (`rerail_device::Device::target`) only chooses a name. Two devices of one target then
//! prepare the same text, and a kernel compiled for the target ahead of time is the one the device would compile.
//! Kernels test the target's features (`UKL_HAS_*`), never a device's name.
//!
//! The names:
//!
//! - **Metal**, by feature tier (`METAL`): `apple7`, the M1-M4 (`simdgroup_matrix`; bf16 from Metal 3.1); `apple10`,
//!   the M5 (Metal 4's `matmul2d`, `UKL_HAS_MATMUL2D`). A tier is added only where its text differs: the back end
//!   still compiles for the exact GPU.
//! - **CUDA**, `sm_<N>`: the compiler's architecture (`__CUDA_ARCH__`, which the kernels' features test).
//! - **HIP**, the `gfx` target (`gfx1035`, `gfx11-generic`): its wave (64 on gfx9's, the CDNA parts; 32 on RDNA's)
//!   and, on the exact RDNA2 targets (`gfx103*`), its dot-product instructions (`UKL_HIP_AMD_DOT`).
//!
//! OpenCL's, the CPU's and the CPU's vector code's (`Vec`, decision 0567) text is prepared from what the device
//! reports (its sub-group width, its largest group; the CPU's knobs): their drivers and compilers are on every
//! machine, so nothing of theirs ships, and their targets are local (`Target::local`).

use crate::{Backend, Preprocess};

/// Metal's tiers: name, the Metal version the text is prepared for (`__METAL_VERSION__`), and its features.
pub const METAL: &[(&str, u32, &[&str])] = &[("apple7", 310, &[]), ("apple10", 400, &["UKL_HAS_MATMUL2D"])];

/// The targets a release ships (decision 0645 §3): Metal's tiers; CUDA's SASS from the V100 (sm_70) on, and PTX for
/// GPUs newer than the table (`compute_<N>`, which the driver compiles); AMD's exact targets, RDNA2 to RDNA4 and the
/// CDNA parts. Not AMD's generic targets: the HIP SDK's hiprtc (7.2, Windows) has no device library for them
/// ("cannot find ROCm device library for gfx10-3-generic"). A generic one shipped would still be taken
/// (`Target::runs`).
pub const SHIPPED: &[&str] = &[
    "apple7", "apple10", //
    "sm_70", "sm_75", "sm_80", "sm_86", "sm_89", "sm_90", "sm_100", "sm_120", "sm_121", "compute_70", "compute_80", //
    "gfx1030", "gfx1031", "gfx1032", "gfx1033", "gfx1034", "gfx1035", "gfx1036", //
    "gfx1100", "gfx1101", "gfx1102", "gfx1103", "gfx1150", "gfx1151", "gfx1200", "gfx1201", //
    "gfx90a", "gfx942", "gfx950",
];

/// AMD's generic targets (LLVM's `gfx*-generic`) and the exact targets each one's code runs on.
pub const AMD_GENERIC: &[(&str, &[&str])] = &[
    ("gfx9-generic", &["gfx900", "gfx902", "gfx904", "gfx906", "gfx909", "gfx90c"]),
    ("gfx9-4-generic", &["gfx940", "gfx941", "gfx942", "gfx950"]),
    ("gfx10-1-generic", &["gfx1010", "gfx1011", "gfx1012", "gfx1013"]),
    ("gfx10-3-generic", &["gfx1030", "gfx1031", "gfx1032", "gfx1033", "gfx1034", "gfx1035", "gfx1036"]),
    ("gfx11-generic", &["gfx1100", "gfx1101", "gfx1102", "gfx1103", "gfx1150", "gfx1151", "gfx1152", "gfx1153"]),
    ("gfx12-generic", &["gfx1200", "gfx1201"]),
];

/// What a kernel's text is prepared for: a name, its backend and the defines the name gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// A shippable target's name (`apple7`, `sm_80`, `gfx11-generic`), or the backend's (`opencl`, `cpu`) for a
    /// local one.
    pub name: String,
    pub backend: Backend,
    /// The defines, in order.
    pub defines: Vec<(String, String)>,
    shippable: bool,
}

impl Target {
    /// The shippable target `name`: None where the name is no target's.
    pub fn named(name: &str) -> Option<Target> {
        let t = |backend, defines: Vec<(String, String)>| Some(Target { name: name.to_string(), backend, defines, shippable: true });
        if let Some(&(_, version, features)) = METAL.iter().find(|(n, _, _)| *n == name) {
            return t(Backend::Metal { version }, features.iter().map(|f| (f.to_string(), "1".to_string())).collect());
        }
        // `sm_<N>`: SASS for the architecture; `compute_<N>`: its PTX, which the driver compiles for any GPU from N on.
        // Both prepare the text `__CUDA_ARCH__` N's.
        if let Some(sm) = name.strip_prefix("sm_").or_else(|| name.strip_prefix("compute_")).and_then(|n| n.parse::<u32>().ok()).filter(|&sm| sm >= 50) {
            return t(Backend::Cuda { sm }, Vec::new());
        }
        if name.strip_prefix("gfx").is_some_and(|n| n.starts_with(|c: char| c.is_ascii_digit())) {
            let wave = if name.starts_with("gfx9") { 64 } else { 32 };
            let mut defines = Vec::new();
            if name.starts_with("gfx103") {
                defines.push(("UKL_HIP_AMD_DOT".to_string(), "1".to_string()));
            }
            return t(Backend::Hip { wave }, defines);
        }
        None
    }

    /// A local target: the backend's text prepared from what the device reports (`defines`), nothing of it shipped.
    pub fn local(backend: Backend, defines: Vec<(String, String)>) -> Target {
        let name = match backend {
            Backend::OpenCl => "opencl",
            Backend::Cpu => "cpu",
            Backend::Vec => "vec",
            Backend::Metal { .. } => "metal-local",
            Backend::Cuda { .. } => "cuda-local",
            Backend::Hip { .. } => "hip-local",
        };
        Target { name: name.to_string(), backend, defines, shippable: false }
    }

    /// Whether a kernel compiled for this target ahead of time may be shipped: its text is the name's alone.
    pub fn shippable(&self) -> bool {
        self.shippable
    }

    /// The form a kernel compiled for this target ahead of time takes, as its file's extension: a Metal library (AIR),
    /// CUDA's SASS (`cubin`) or PTX, an AMD code object (`hsaco`). None for a local target.
    pub fn form(&self) -> Option<&'static str> {
        if !self.shippable {
            return None;
        }
        Some(match self.backend {
            Backend::Metal { .. } => "metallib",
            Backend::Cuda { .. } if self.name.starts_with("compute_") => "ptx",
            Backend::Cuda { .. } => "cubin",
            Backend::Hip { .. } => "hsaco",
            Backend::OpenCl | Backend::Cpu | Backend::Vec => return None,
        })
    }

    /// The shippable targets whose compiled code a device of this target runs, best first: itself; Metal's lower
    /// tiers (`apple7` under `apple10`); CUDA's SASS of the same major architecture and a lower minor one, then PTX of
    /// any architecture up to it (`SHIPPED`'s, highest first); an AMD target's generic family (`AMD_GENERIC`).
    ///
    /// A kernel compiled for another target is taken only where its text is this target's text (the shipped key
    /// holds the text): no feature this target's text takes is lost.
    pub fn runs(&self) -> Vec<String> {
        if !self.shippable {
            return Vec::new();
        }
        let mut v = vec![self.name.clone()];
        match self.backend {
            Backend::Metal { version } => {
                v.extend(METAL.iter().rev().filter(|&&(n, ver, _)| ver < version && n != self.name).map(|(n, _, _)| n.to_string()));
            }
            Backend::Cuda { sm } if self.name.starts_with("sm_") => {
                let shipped = |prefix: &str| -> Vec<u32> {
                    let mut a: Vec<u32> = SHIPPED.iter().filter_map(|n| n.strip_prefix(prefix)?.parse().ok()).collect();
                    a.sort_unstable_by(|x, y| y.cmp(x));
                    a
                };
                v.extend(shipped("sm_").into_iter().filter(|&m| m < sm && m / 10 == sm / 10).map(|m| format!("sm_{m}")));
                v.extend(shipped("compute_").into_iter().filter(|&m| m <= sm).map(|m| format!("compute_{m}")));
            }
            Backend::Hip { .. } => {
                v.extend(AMD_GENERIC.iter().filter(|(_, exact)| exact.contains(&self.name.as_str())).map(|(g, _)| g.to_string()));
            }
            _ => {}
        }
        v
    }

    /// A preprocessing for this target: its backend and its defines.
    pub fn preprocess(&self) -> Preprocess {
        self.defines.iter().fold(Preprocess::new(self.backend), |p, (n, v)| p.define(n, Some(v)))
    }
}
