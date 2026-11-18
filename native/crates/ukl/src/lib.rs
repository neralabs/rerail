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
//! common UKL headers (`<ukl>`, `<ukl_shim.h>`) and the backend's own (`<ukl_metal_shim.h>`, ...). UKL's
//! `ukl_shim.h` picks the backend from the predefines (`__METAL_VERSION__`, `__CUDA_ARCH__`).

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
];

/// A GPU backend a kernel is preprocessed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Metal; `version` is the MSL version the shim sees (310: 3.1). The output is MSL for the Metal front end.
    Metal { version: u32 },
    /// CUDA; `sm` is the compute capability as two or three digits (121: 12.1). The output is CUDA C++ for NVRTC.
    Cuda { sm: u32 },
}

impl Backend {
    /// The shim directory's name under `mldb/builtin/`.
    fn dir(self) -> &'static str {
        match self {
            Backend::Metal { .. } => "metal",
            Backend::Cuda { .. } => "cuda",
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

    /// The include search: the caller's directories, then the carried headers' root, UKL's own directory and the
    /// backend's shim directory.
    fn search(&self) -> Vec<(String, DirKind)> {
        let mut d: Vec<(String, DirKind)> = self.dirs.iter().map(|d| (d.clone(), DirKind::Angle)).collect();
        d.push((ROOT.to_string(), DirKind::Angle));
        d.push((format!("{ROOT}/mldb/block/ukl"), DirKind::Angle));
        d.push((format!("{ROOT}/mldb/builtin/{}/ukl", self.backend.dir()), DirKind::Angle));
        d
    }

    /// `main` preprocessed: the backend's source text, with line markers to the original files.
    pub fn run(&self, main: &str) -> Result<String, Error> {
        let mut cfg = Config::new(Options::cpp(20, true));
        cfg.predefines = vec![("__cplusplus".into(), Some("202002L".into()))];
        cfg.predefines.extend(self.backend.predefines());
        cfg.predefines.extend(self.defines.iter().cloned());
        let carried: HashMap<String, &'static str> =
            HEADERS.iter().map(|(p, t)| (format!("{ROOT}/{p}"), *t)).collect();
        let files = &self.files;
        let read = |p: &str| -> Option<Vec<u8>> {
            if let Some(t) = files.get(p) {
                return Some(t.clone());
            }
            if let Some(t) = carried.get(p) {
                return Some(t.as_bytes().to_vec());
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
