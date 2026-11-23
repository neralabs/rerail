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
//! OpenCL's and the CPU's text is prepared from what the device reports (its sub-group width, its largest group;
//! the CPU's knobs): their drivers and compilers are on every machine, so nothing of theirs ships, and their
//! targets are local (`Target::local`).

use crate::{Backend, Preprocess};

/// Metal's tiers: name, the Metal version the text is prepared for (`__METAL_VERSION__`), and its features.
pub const METAL: &[(&str, u32, &[&str])] = &[("apple7", 310, &[]), ("apple10", 400, &["UKL_HAS_MATMUL2D"])];

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
        if let Some(sm) = name.strip_prefix("sm_").and_then(|n| n.parse::<u32>().ok()).filter(|&sm| sm >= 50) {
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

    /// A preprocessing for this target: its backend and its defines.
    pub fn preprocess(&self) -> Preprocess {
        self.defines.iter().fold(Preprocess::new(self.backend), |p, (n, v)| p.define(n, Some(v)))
    }
}
