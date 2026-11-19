//! UKL ("the Ugly Kernel Language", from MLDB): kernels written once, in C with macros, for every GPU backend
//! (decision 0400 §3, §4).
//!
//! A UKL kernel is C (a C++20 subset) whose GPU-specific parts are macros: address spaces (`ROBUFFER(T)`),
//! kernel signatures (`DEFINE_KERNEL2`'s rows of access, type, name and bounds), barriers, atomics and SIMD-group
//! operations. A backend's shim header turns the macros into that backend's language: MSL for Metal, CUDA C++
//! for NVRTC. The shims live at their upstream paths under `forest/src/mldb/` (MLDB's layout, as REUSE.toml
//! records) and are carried in the binary here, so that a kernel, written by hand or generated, is preprocessed
//! at run time with no C compiler on the machine.
//!
//! ```text
//! let src = Preprocess::new(Backend::Cuda { sm: 121 })
//!     .dir("forest/src")                 // where the kernel's own includes are found
//!     .run("forest/src/.../k.cu")?;      // CUDA C++ source, ready for NVRTC
//! ```
//!
//! The include search mirrors MLDB's build: the caller's directories, then the shims' root (`mldb/...`), the
//! backend's own shim (`<ukl_metal_shim.h>`, ...) and the common UKL headers (`<ukl>`, `<ukl_shim.h>`). UKL's
//! `ukl_shim.h` picks the backend from the predefines (`__METAL_VERSION__`, `__CUDA_ARCH__`; none for the CPU).
//!
//! **The library** (decision 0400 §5): Rerail's UKL headers, in `lib/ukl/`, carried here too and found as
//! `<ukl/NAME.h>` by every kernel, hand-written or generated. `ukl/backend.h` holds the intrinsics, one section per
//! backend; every other header is written once over them (`LIBRARY` lists them, lowest first). A kernel takes what
//! it uses by `#include`: the library is linked by inclusion, and the backend's compiler (the Metal front end,
//! NVRTC) sees one translation unit and inlines across it, as a device-code link would not.

use rerail_c_pp::files::{DirKind, Search};
use rerail_c_pp::sink::{At, Diag, Sink, TextSink};
use rerail_c_pp::{Config, Options, Pp};
use std::collections::HashMap;

/// The root the carried headers are found under; no file system path starts so.
const ROOT: &str = "/<ukl>";

/// The shims and UKL's common headers, at their paths under `forest/src/`.
const HEADERS: &[(&str, &str)] = &[
    ("mldb/block/ukl/ukl", include_str!("../../../../forest/src/mldb/block/ukl/ukl")),
    ("mldb/block/ukl/ukl_shim.h", include_str!("../../../../forest/src/mldb/block/ukl/ukl_shim.h")),
    ("mldb/block/ukl/ukl_macros.h", include_str!("../../../../forest/src/mldb/block/ukl/ukl_macros.h")),
    ("mldb/block/ukl/ukl_auto_macros.h", include_str!("../../../../forest/src/mldb/block/ukl/ukl_auto_macros.h")),
    ("mldb/builtin/metal/ukl/ukl_metal_shim.h", include_str!("../../../../forest/src/mldb/builtin/metal/ukl/ukl_metal_shim.h")),
    ("mldb/builtin/cuda/ukl/ukl_cuda_shim.h", include_str!("../../../../forest/src/mldb/builtin/cuda/ukl/ukl_cuda_shim.h")),
    ("mldb/builtin/cpu/ukl/ukl_cpu_shim.h", include_str!("../../../../forest/src/mldb/builtin/cpu/ukl/ukl_cpu_shim.h")),
];

/// The library's headers, by the name a kernel includes them by (`#include <ukl/memory.h>`), each after the ones it
/// includes. Their tests are `rerail-gpu`'s (`library_tests`), on each device.
pub const LIBRARY: &[(&str, &str)] = &[
    ("ukl/backend.h", include_str!("../lib/ukl/backend.h")),
    ("ukl/bits.h", include_str!("../lib/ukl/bits.h")),
    ("ukl/memory.h", include_str!("../lib/ukl/memory.h")),
    ("ukl/simd.h", include_str!("../lib/ukl/simd.h")),
    ("ukl/group.h", include_str!("../lib/ukl/group.h")),
    // A body over a type (decision 0400 §3.5): its header includes it once per type, with its parameters defined.
    ("ukl/body/lz4_decode.h", include_str!("../lib/ukl/body/lz4_decode.h")),
    ("ukl/body/histogram_part.h", include_str!("../lib/ukl/body/histogram_part.h")),
    ("ukl/body/radix_tile.h", include_str!("../lib/ukl/body/radix_tile.h")),
    ("ukl/lz4.h", include_str!("../lib/ukl/lz4.h")),
    ("ukl/scan.h", include_str!("../lib/ukl/scan.h")),
    ("ukl/compact.h", include_str!("../lib/ukl/compact.h")),
    ("ukl/histogram.h", include_str!("../lib/ukl/histogram.h")),
    ("ukl/radix.h", include_str!("../lib/ukl/radix.h")),
    ("ukl/hash.h", include_str!("../lib/ukl/hash.h")),
    ("ukl/workqueue.h", include_str!("../lib/ukl/workqueue.h")),
    ("ukl/clif.h", include_str!("../lib/ukl/clif.h")),
    // The reads written once in CLIF (decision 0310), as `rerail_clif::ukl` writes them: generated, held here so
    // that every kernel finds them as it finds the rest of the library (`rerail-clif`'s tests write them again).
    ("ukl/reads/tape.h", include_str!("../lib/ukl/reads/tape.h")),
    ("ukl/reads/tape_z.h", include_str!("../lib/ukl/reads/tape_z.h")),
    ("ukl/reads/relation.h", include_str!("../lib/ukl/reads/relation.h")),
    ("ukl/reads/ids.h", include_str!("../lib/ukl/reads/ids.h")),
    ("ukl/reads/view.h", include_str!("../lib/ukl/reads/view.h")),
];

/// The Metal front end's own headers, which it includes in every source itself: empty when a source that names them
/// (a hand-written MSL kernel that takes the library's headers) is preprocessed ahead of it.
const METAL_OWN: &[&str] = &["metal_stdlib", "metal_atomic"];

/// A fingerprint of everything a kernel can include from here (the shims and the library): a kernel's text names its
/// headers, not their contents, so a store of compiled kernels keys a kernel by its text and this.
pub fn fingerprint() -> u64 {
    // FNV-1a over each carried file's name and text.
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for (name, text) in HEADERS.iter().chain(LIBRARY) {
        for b in name.bytes().chain([0]).chain(text.bytes()).chain([0]) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Whether `text` takes UKL's library (`#include <ukl/...>` or `<ukl>`), so it must be preprocessed before its
/// backend's compiler sees it.
pub fn takes_library(text: &str) -> bool {
    text.contains("#include <ukl")
}

/// A Metal source that takes the library (`takes_library`), preprocessed for the Metal front end: the library's
/// headers in place, the source's own text and macros as written. A hand-written MSL kernel includes the generated
/// reads so (decision 0400 §3.3) until it is UKL itself.
pub fn metal_source(name: &str, text: &str) -> Result<String, Error> {
    let path = format!("/<kernels>/{name}");
    Preprocess::new(Backend::Metal { version: 310 }).file(&path, text).run(&path)
}

/// A GPU backend a kernel is preprocessed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Metal; `version` is the MSL version the shim sees (310: 3.1). The output is MSL for the Metal front end.
    Metal { version: u32 },
    /// CUDA; `sm` is the compute capability as two or three digits (121: 12.1). The output is CUDA C++ for NVRTC.
    Cuda { sm: u32 },
    /// The CPU: C++ for the host's compiler, each threadgroup's threads coroutines (`ukl_cpu_shim.h`), for this
    /// process's architecture and system.
    Cpu,
}

impl Backend {
    /// The shim directory's name under `mldb/builtin/`.
    fn dir(self) -> &'static str {
        match self {
            Backend::Metal { .. } => "metal",
            Backend::Cuda { .. } => "cuda",
            Backend::Cpu => "cpu",
        }
    }

    /// What the backend's compiler would predefine that the shims test, and the marker that tells a shim it is
    /// being preprocessed ahead of its compiler (so it leaves the compiler's own headers to the compiler).
    pub fn predefines(self) -> Vec<(String, Option<String>)> {
        let d = |n: &str, v: String| (n.to_string(), Some(v));
        match self {
            Backend::Metal { version } => vec![
                d("__METAL_VERSION__", version.to_string()),
                d("__METAL_MACOS__", "1".into()),
                d("__UKL_METAL_PREPROCESSING", "1".into()),
            ],
            Backend::Cuda { sm } => vec![
                d("__CUDA_ARCH__", (sm * 10).to_string()),
                d("__CUDACC__", "1".into()),
                d("__CUDACC_RTC__", "1".into()),
                d("__UKL_CUDA_PREPROCESSING", "1".into()),
            ],
            // The host compiler's own predefines that the shim tests (its context switch is per architecture and
            // system), and the backend's choice, which UKL's `ukl_shim.h` takes when no GPU compiler's is set.
            Backend::Cpu => {
                let mut v = vec![d("UKL_CPU_BACKEND", "1".into()), d("__UKL_CPU_PREPROCESSING", "1".into())];
                v.push(d(if cfg!(target_arch = "aarch64") { "__aarch64__" } else { "__x86_64__" }, "1".into()));
                v.push(d(if cfg!(target_os = "macos") { "__APPLE__" } else { "__linux__" }, "1".into()));
                v
            }
        }
    }
}

/// A preprocessing failure: the preprocessor's errors, as `path:line: message`.
#[derive(Debug)]
pub struct Error(pub Vec<String>);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UKL preprocessing failed: {}", self.0.join("; "))
    }
}

impl std::error::Error for Error {}

/// One preprocessing: the backend, the caller's include directories and in-memory files, and its defines.
pub struct Preprocess {
    backend: Backend,
    dirs: Vec<String>,
    files: HashMap<String, Vec<u8>>,
    defines: Vec<(String, Option<String>)>,
}

/// The output text, and the paths of the files diagnostics name (by file id).
struct Out {
    text: TextSink,
    paths: Vec<String>,
    errors: Vec<(u32, u32, String)>,
}

impl Sink for Out {
    fn wants_lines(&self) -> bool {
        true
    }
    fn token(&mut self, t: &rerail_c_pp::macros::Tk, text: &[u8], at: Option<At>, off: u32) {
        self.text.token(t, text, at, off);
    }
    fn enter(&mut self, file: u32, path: &str, from: Option<(u32, u32)>, depth: u32) {
        if self.paths.len() <= file as usize {
            self.paths.resize(file as usize + 1, String::new());
        }
        self.paths[file as usize] = path.to_string();
        self.text.enter(file, path, from, depth);
    }
    fn pragma(&mut self, file: u32, at: u32, text: &[u8]) {
        self.text.pragma(file, at, text);
    }
    fn diag(&mut self, file: u32, at: u32, d: Diag, text: &[u8]) {
        // Every diagnostic but a `#warning` fails the kernel: a header not found, a bad call or paste, an
        // unterminated conditional would each make another program than the one written.
        if !matches!(d, Diag::Warning) {
            self.errors.push((file, at, format!("{d:?}: {}", String::from_utf8_lossy(text))));
        }
    }
}

impl Preprocess {
    pub fn new(backend: Backend) -> Self {
        Preprocess { backend, dirs: Vec::new(), files: HashMap::new(), defines: Vec::new() }
    }

    /// An include directory on the file system (`-I`), searched before UKL's own.
    pub fn dir(mut self, d: impl Into<String>) -> Self {
        self.dirs.push(d.into());
        self
    }

    /// A file held in memory at `path` (a generated kernel, or a header that is not on disk).
    pub fn file(mut self, path: impl Into<String>, text: impl Into<Vec<u8>>) -> Self {
        self.files.insert(path.into(), text.into());
        self
    }

    /// `-D name=value` (`None`: `-D name`, defined as 1).
    pub fn define(mut self, name: &str, value: Option<&str>) -> Self {
        self.defines.push((name.to_string(), Some(value.unwrap_or("1").to_string())));
        self
    }

    /// The include search: the caller's directories, then the carried headers' root, the backend's shim directory
    /// and UKL's own directory.
    fn search(&self) -> Vec<(String, DirKind)> {
        let mut d: Vec<(String, DirKind)> = self.dirs.iter().map(|d| (d.clone(), DirKind::Angle)).collect();
        d.push((ROOT.to_string(), DirKind::Angle));
        // The backend's shim first: upstream's CPU shim (`block/ukl/ukl_cpu_shim.h`, over MLDB's C++ runtime) has the
        // name of Rerail's (`builtin/cpu/ukl/`), and is not carried.
        d.push((format!("{ROOT}/mldb/builtin/{}/ukl", self.backend.dir()), DirKind::Angle));
        d.push((format!("{ROOT}/mldb/block/ukl"), DirKind::Angle));
        d.push((format!("{ROOT}/lib"), DirKind::Angle));
        if matches!(self.backend, Backend::Metal { .. }) {
            d.push((format!("{ROOT}/metal"), DirKind::Angle));
        }
        d
    }

    /// `main` preprocessed: the backend's source text, with line markers to the original files.
    pub fn run(&self, main: &str) -> Result<String, Error> {
        let mut cfg = Config::new(Options::cpp(20, true));
        cfg.predefines = vec![("__cplusplus".into(), Some("202002L".into()))];
        cfg.predefines.extend(self.backend.predefines());
        cfg.predefines.extend(self.defines.iter().cloned());
        let carried: HashMap<String, &'static str> = HEADERS
            .iter()
            .map(|(p, t)| (format!("{ROOT}/{p}"), *t))
            .chain(LIBRARY.iter().map(|(p, t)| (format!("{ROOT}/lib/{p}"), *t)))
            .collect();
        let files = &self.files;
        let metal = matches!(self.backend, Backend::Metal { .. });
        let read = |p: &str| -> Option<Vec<u8>> {
            if let Some(t) = files.get(p) {
                return Some(t.clone());
            }
            if let Some(t) = carried.get(p) {
                return Some(t.as_bytes().to_vec());
            }
            if metal && p.strip_prefix(ROOT).and_then(|n| n.strip_prefix("/metal/")).is_some_and(|n| METAL_OWN.contains(&n)) {
                return Some(Vec::new());
            }
            if p.starts_with(ROOT) {
                return None;
            }
            std::fs::read(p).ok().filter(|_| std::path::Path::new(p).is_file())
        };
        let mut io = Search::new(self.search(), read);
        let mut out = Out { text: TextSink::default(), paths: Vec::new(), errors: Vec::new() };
        // A `__has_*` test the configuration does not answer reads as 0, as on a compiler without the feature.
        let ok = Pp::new(cfg, &mut io, &mut out).run(main);
        if !ok || !out.errors.is_empty() {
            let mut msgs: Vec<String> = out
                .errors
                .iter()
                .map(|(f, at, m)| format!("{}@{at}: {m}", out.paths.get(*f as usize).map_or("?", |s| s.as_str())))
                .collect();
            if msgs.is_empty() {
                msgs.push(format!("{main}: the preprocessor did not finish"));
            }
            return Err(Error(msgs));
        }
        Ok(String::from_utf8_lossy(&out.text.out).into_owned())
    }
}

#[cfg(test)]
mod tests;
