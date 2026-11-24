//! UKL ("the Ugly Kernel Language", from MLDB): kernels written once, in C with macros, for every GPU backend
//! (decision 0440 §3, §4).
//!
//! A UKL kernel is C (a C++20 subset) whose GPU-specific parts are macros: address spaces (`ROBUFFER(T)`),
//! kernel signatures (`DEFINE_KERNEL2`'s rows of access, type, name and bounds), barriers, atomics and SIMD-group
//! operations. A backend's shim header turns the macros into that backend's language: MSL for Metal, CUDA C++
//! for NVRTC. The shims live at their upstream paths under this crate's `mldb/` (MLDB's layout, as REUSE.toml
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
//! **The library** (decision 0440 §5): Rerail's UKL headers, in `lib/ukl/`, carried here too and found as
//! `<ukl/NAME.h>` by every kernel, hand-written or generated. `ukl/backend.h` holds the intrinsics, one section per
//! backend; every other header is written once over them (`LIBRARY` lists them, lowest first). A kernel takes what
//! it uses by `#include`: the library is linked by inclusion, and the backend's compiler (the Metal front end,
//! NVRTC) sees one translation unit and inlines across it, as a device-code link would not.

use rerail_c_pp::files::{DirKind, Search};
use rerail_c_pp::sink::{At, Diag, Sink, TextSink};
use rerail_c_pp::{Config, Options, Pp};
/// A frontend's input (`Preprocess::expanded`).
pub use rerail_c_pp::tokens::Expanded;
use std::collections::HashMap;

/// The root the carried headers are found under; no file system path starts so.
const ROOT: &str = "/<ukl>";

include!("carried.rs");

/// The Metal front end's own headers, which it includes in every source itself: empty when a source that names them
/// (a hand-written MSL kernel that takes the library's headers) is preprocessed ahead of it.
const METAL_OWN: &[&str] = &["metal_stdlib", "metal_atomic"];

/// A fingerprint of everything a kernel can include from here (the shims and the library): a kernel's text names its
/// headers, not their contents, so a store of compiled kernels keys a kernel by its text and this.
pub fn fingerprint() -> u64 {
    FINGERPRINT
}

// `FINGERPRINT`: `fingerprint`, computed by the build script over `carried.rs`'s files. Hashing their 1.2 MB at each
// kernel's lookup took 17 ms in a debug build: every Metal launch of the block tests paid it once or twice.
include!(concat!(env!("OUT_DIR"), "/fingerprint.rs"));

/// Whether `text` takes UKL's library (`#include <ukl/...>` or `<ukl>`), so it must be preprocessed before its
/// backend's compiler sees it.
pub fn takes_library(text: &str) -> bool {
    text.contains("#include <ukl")
}

/// A Metal source that takes the library (`takes_library`), preprocessed for the Metal front end: the library's
/// headers in place, the source's own text and macros as written. A hand-written MSL kernel includes the generated
/// reads so (decision 0440 §3.3) until it is UKL itself.
pub fn metal_source(name: &str, text: &str) -> Result<String, Error> {
    metal_source_for(name, text, false)
}

/// `metal_source` for a device: `g13`, an Apple G13 GPU (the M1s), whose back end numbers a function's loop exits by
/// their low byte, takes its large bodies as calls (`UKL_METAL_G13`, `ukl/backend.h`; decision 0672).
pub fn metal_source_for(name: &str, text: &str, g13: bool) -> Result<String, Error> {
    let path = format!("/<kernels>/{name}");
    let p = Preprocess::new(Backend::Metal { version: 310 }).file(&path, text);
    if g13 { p.define("UKL_METAL_G13", None) } else { p }.run(&path)
}

/// A GPU backend a kernel is preprocessed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Metal; `version` is the MSL version the shim sees (310: 3.1). The output is MSL for the Metal front end.
    Metal { version: u32 },
    /// CUDA; `sm` is the compute capability as two or three digits (121: 12.1). The output is CUDA C++ for NVRTC.
    Cuda { sm: u32 },
    /// HIP (AMD's GPUs); `wave` is the wavefront's lanes (32 on RDNA, 64 on CDNA and GCN). The output is HIP C++ for
    /// hiprtc, through the CUDA shim (HIP's language is CUDA's) with HIP's SIMD-group operations
    /// (`ukl_cuda_shim.h`, `ukl/backend.h`'s `UKL_HIP`).
    Hip { wave: u32 },
    /// The CPU: C++ for the host's compiler, each threadgroup's threads coroutines (`ukl_cpu_shim.h`), for this
    /// process's architecture and system.
    Cpu,
    /// OpenCL: OpenCL C 1.2 for the device's driver (`ukl_opencl_shim.h`). The device's SIMD width is a define
    /// (`UKL_SIMD_WIDTH`, 32 by default: emulated through threadgroup memory on a device without subgroups).
    OpenCl,
}

impl Backend {
    /// Whether the backend's compiler has a 32-bit `long`: the host compiler on Windows (LLP64), and NVRTC and hiprtc
    /// there, whose device types match the host's.
    fn long_is_32(self) -> bool {
        cfg!(windows) && matches!(self, Backend::Cpu | Backend::Cuda { .. } | Backend::Hip { .. })
    }

    /// The shim directory's name under `mldb/builtin/`.
    fn dir(self) -> &'static str {
        match self {
            Backend::Metal { .. } => "metal",
            Backend::Cuda { .. } | Backend::Hip { .. } => "cuda",
            Backend::Cpu => "cpu",
            Backend::OpenCl => "opencl",
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
            // hip-clang's device compile's, which the shims test: no `__CUDA_ARCH__` (HIP has none).
            Backend::Hip { wave } => vec![
                d("__HIP__", "1".into()),
                d("__HIPCC_RTC__", "1".into()),
                d("__HIP_DEVICE_COMPILE__", "1".into()),
                d("__AMDGCN__", "1".into()),
                d("__AMDGCN_WAVEFRONT_SIZE", wave.to_string()),
                d("__UKL_HIP_PREPROCESSING", "1".into()),
            ],
            // The host compiler's own predefines that the shim tests (its context switch is per architecture and
            // system), and the backend's choice, which UKL's `ukl_shim.h` takes when no GPU compiler's is set.
            Backend::Cpu => {
                let mut v = vec![d("UKL_CPU_BACKEND", "1".into()), d("__UKL_CPU_PREPROCESSING", "1".into())];
                v.push(d(if cfg!(target_arch = "aarch64") { "__aarch64__" } else { "__x86_64__" }, "1".into()));
                v.push(d(if cfg!(target_os = "macos") { "__APPLE__" } else if cfg!(windows) { "_WIN32" } else { "__linux__" }, "1".into()));
                v
            }
            Backend::OpenCl => vec![
                d("__OPENCL_VERSION__", "120".into()),
                d("__OPENCL_C_VERSION__", "120".into()),
                d("__UKL_OPENCL_PREPROCESSING", "1".into()),
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

/// The output (text, or the frontends' tokens), and the paths of the files diagnostics name (by file id).
struct Out<S> {
    text: S,
    paths: Vec<String>,
    errors: Vec<(u32, u32, String)>,
    /// The compiler's `long` is 32 bits (`long_is_32`): an integer literal's `l` suffix is written `ll`.
    long_32: bool,
}

/// Integer literal `text` with its `long` suffix (`l`, `ul`, `lu`, any case) written `long long` (`ll`, `ull`), or
/// `None` when it has none. UKL is written for 64-bit `long`, as Metal, OpenCL and every other target have it:
/// `(1ul << 39) - 1`, `~0ul`. Under Windows' LLP64 (the CPU's host compiler, NVRTC beside it) `long` is 32 bits, so
/// such a shift is out of range and `~0ul` is `0xffffffff`; the device executor's arena texts read at offset 0 (an
/// `@printf` gave "." for "002.").
fn long_long(text: &[u8]) -> Option<Vec<u8>> {
    if !text.first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let digits = text.iter().rposition(|b| !matches!(b, b'u' | b'U' | b'l' | b'L')).map_or(0, |i| i + 1);
    let (number, suffix) = text.split_at(digits);
    let hex = number.len() > 1 && matches!(number[1], b'x' | b'X');
    let float = number.contains(&b'.') || number.iter().any(|b| if hex { matches!(b, b'p' | b'P') } else { matches!(b, b'e' | b'E') });
    let longs = suffix.iter().filter(|b| matches!(b, b'l' | b'L')).count();
    if float || longs != 1 {
        return None;
    }
    let unsigned = suffix.iter().any(|b| matches!(b, b'u' | b'U'));
    Some([number, if unsigned { b"ull" } else { b"ll" }].concat())
}

impl<S: Sink> Sink for Out<S> {
    fn wants_lines(&self) -> bool {
        true
    }
    fn token(&mut self, t: &rerail_c_pp::macros::Tk, text: &[u8], at: Option<At>, off: u32) {
        match self.long_32.then(|| long_long(text)).flatten() {
            Some(wide) => self.text.token(t, &wide, at, off),
            None => self.text.token(t, text, at, off),
        }
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
        let (text, _) = self.pass(main, TextSink::default())?;
        Ok(String::from_utf8_lossy(&text.out).into_owned())
    }

    /// `main` preprocessed for a frontend (decision 0327, mode (b)): the expanded text, the C parser's tokens over
    /// it and where each part came from (no line markers), and the paths of the files those name, by file id.
    pub fn expanded(&self, main: &str) -> Result<(rerail_c_pp::tokens::Expanded, Vec<String>), Error> {
        let (tokens, paths) = self.pass(main, rerail_c_pp::tokens::TokenSink::new(self.backend != Backend::OpenCl))?;
        Ok((tokens.finish(), paths))
    }

    /// The text of file `path` as a pass reads it, a given file's or a carried header's (a path the files of an
    /// `expanded` name): for telling a place's line.
    pub fn text_of(&self, path: &str) -> Option<Vec<u8>> {
        if let Some(t) = self.files.get(path) {
            return Some(t.clone());
        }
        let carried = HEADERS.iter().find(|(p, _)| format!("{ROOT}/{p}") == path).or_else(|| LIBRARY.iter().find(|(p, _)| format!("{ROOT}/lib/{p}") == path));
        if let Some((_, t)) = carried {
            return Some(t.as_bytes().to_vec());
        }
        if path.starts_with(ROOT) { None } else { std::fs::read(path).ok() }
    }

    /// One pass of the preprocessor over `main` into `sink`: the sink, and the paths of the files, by file id.
    fn pass<S: Sink>(&self, main: &str, sink: S) -> Result<(S, Vec<String>), Error> {
        // OpenCL C is C (C99 with clang's extensions); every other backend's language is C++.
        let mut cfg = if self.backend == Backend::OpenCl {
            Config::new(Options::c(99, true))
        } else {
            let mut c = Config::new(Options::cpp(20, true));
            c.predefines = vec![("__cplusplus".into(), Some("202002L".into()))];
            c
        };
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
        let long_32 = self.backend.long_is_32();
        let mut out = Out { text: sink, paths: Vec::new(), errors: Vec::new(), long_32 };
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
        Ok((out.text, out.paths))
    }
}

pub mod split;

#[cfg(test)]
mod tests;
