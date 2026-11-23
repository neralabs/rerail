//! The store of compiled kernels every process of the user shares (decision 0395, on the device layer: 0440 step
//! B): a kernel's compiled form, kept on disk under a key of everything its compile reads, so a later process loads
//! it in place of compiling it again.
//!
//! ```text
//! RERAIL_GPU_KERNELS=<dir>|off    the store: ~/Library/Caches/rerail/gpu-kernels (macOS),
//!                                 $XDG_CACHE_HOME/rerail/gpu-kernels or ~/.cache/rerail/gpu-kernels (Linux)
//!                                 %LOCALAPPDATA%\rerail\gpu-kernels (Windows)
//! ```
//!
//! Each backend keeps its own form, in a directory of its own: CUDA the cubin NVRTC wrote (`cuda/<key>.cubin`),
//! keyed by the source, the architecture, the options and the NVRTC and driver versions; OpenCL the program's
//! binary (`opencl/<key>.bin`), keyed by the source, the options and the device's name, version and driver; the
//! CPU the shared object the compiler made (`cpu/<key>.so`), keyed by the source, the options and the compiler's
//! version. Metal's pipelines are in the engine's archive store (`rerail_accel::gpu::kernels`) until the engine's
//! kernels are on the device layer (0440 step C).
//!
//! A form that does not load (damaged, or of another build the key missed) is compiled again and replaced. A store
//! that cannot be written is no error: the kernel is compiled, and the next process compiles it too.
//!
//! The store keeps within `RERAIL_GPU_KERNELS_MB` (2 GB by default): an entry's use touches it, and `prune` removes
//! the least recently used past the bound (an older driver's or compiler's are never used again, so they go first).
//! The engine's upkeep prunes it beside the Metal archives; a process prunes it once itself, at its first save.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The store's directory, or none (`RERAIL_GPU_KERNELS=off`, or no home).
pub fn dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        match std::env::var_os("RERAIL_GPU_KERNELS") {
            Some(v) if v == "off" => return None,
            Some(v) if !v.is_empty() => return Some(PathBuf::from(v)),
            _ => {}
        }
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let cache = if cfg!(windows) {
            // Windows sets no HOME: its per-user cache is under %LOCALAPPDATA%.
            std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()).map(PathBuf::from)?
        } else if cfg!(target_os = "macos") {
            home?.join("Library/Caches")
        } else {
            std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| home.map(|h| h.join(".cache")))?
        };
        Some(cache.join("rerail/gpu-kernels"))
    })
    .as_deref()
}

/// A key of `parts`: 128 bits, as hex (two FNV-1a hashes of differing seeds; each part ended by a zero byte, so
/// parts do not run into each other).
pub fn key(parts: &[&[u8]]) -> String {
    let mut h = [0xcbf2_9ce4_8422_2325u64, 0x8422_2325_cbf2_9ce4u64];
    for part in parts {
        for &b in part.iter().chain(&[0u8]) {
            h[0] = (h[0] ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
            h[1] = (h[1] ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(5);
        }
    }
    format!("{:016x}{:016x}", h[0], h[1])
}

/// The kernels shipped with the engine (`rerail-engine kernels ship`): read-only. Each kernel compiled ahead of time
/// for a target under `<dir>/gpu/<target>/` (decision 0645, `rerail_gpu::kernels::shipped_path`); Metal's AIR and
/// archives (`rerail_accel::gpu::kernels`); and decision 0395's forms by the store's own keys under
/// `<dir>/<backend>/`, used only where every part of the key is this machine's. `RERAIL_SHIPPED_KERNELS=<dir>|off`,
/// else `rerail-kernels/` beside the program, or beside the library the engine was loaded from (the Node addon in
/// the package's `native/`: `os::own_image`).
pub fn shipped_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| match std::env::var_os("RERAIL_SHIPPED_KERNELS") {
        Some(v) if v == "off" => None,
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => {
            let beside = |p: PathBuf| Some(p.parent()?.join("rerail-kernels")).filter(|d| d.is_dir());
            std::env::current_exe().ok().and_then(beside).or_else(|| crate::os::own_image().and_then(beside))
        }
    })
    .as_deref()
}

/// The shipped form of `backend`'s `key`, when there is one.
pub fn shipped(backend: &str, key: &str, ext: &str) -> Option<PathBuf> {
    let p = shipped_dir()?.join(backend).join(format!("{key}.{ext}"));
    if !p.is_file() {
        return None;
    }
    touch(&p);
    Some(p)
}

/// Where `backend`'s form of `key` is kept (`ext` its extension), its directory made: none without a store.
pub fn path(backend: &str, key: &str, ext: &str) -> Option<PathBuf> {
    let d = dir()?.join(backend);
    std::fs::create_dir_all(&d).ok()?;
    let p = d.join(format!("{key}.{ext}"));
    touch(&p);
    Some(p)
}

/// `p` among the entries this thread's compiles looked up, while it records them (`recording`).
fn touch(p: &Path) {
    TOUCHED.with(|t| {
        if let Some(t) = t.borrow_mut().as_mut() {
            t.push(p.to_path_buf());
        }
    });
}

/// The entries `f`'s compiles looked up on this thread, shipped and in the store: what a kernel's compile ahead used,
/// which `kernels ship` copies.
pub fn recording<T>(f: impl FnOnce() -> T) -> (T, Vec<PathBuf>) {
    let outer = TOUCHED.with(|t| t.borrow_mut().replace(Vec::new()));
    let r = f();
    let got = TOUCHED.with(|t| std::mem::replace(&mut *t.borrow_mut(), outer)).unwrap_or_default();
    // A recording around this one has them too.
    TOUCHED.with(|t| {
        if let Some(o) = t.borrow_mut().as_mut() {
            o.extend(got.iter().cloned());
        }
    });
    (r, got)
}

/// The bytes kept for `backend`'s `key`, if any: the shipped form, else the store's (a use, so the entry is touched:
/// `prune` keeps the recently used).
pub fn load(backend: &str, key: &str, ext: &str) -> Option<Vec<u8>> {
    if let Some(bytes) = shipped(backend, key, ext).and_then(|p| std::fs::read(p).ok()).filter(|b| !b.is_empty()) {
        return Some(bytes);
    }
    let p = path(backend, key, ext)?;
    let bytes = std::fs::read(&p).ok().filter(|b| !b.is_empty())?;
    used(&p);
    Some(bytes)
}

/// Marks the entry at `p` used now.
pub fn used(p: &Path) {
    let _ = std::fs::File::options().append(true).open(p).and_then(|f| f.set_modified(std::time::SystemTime::now()));
}

/// The store's bound: `RERAIL_GPU_KERNELS_MB`, else 2 GB.
pub fn max_bytes() -> u64 {
    std::env::var("RERAIL_GPU_KERNELS_MB").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(2048) << 20
}

/// Keeps the store within `max_bytes`: the least recently used entries go first. How many were removed.
pub fn prune(max_bytes: u64) -> u64 {
    let Some(store) = dir() else { return 0 };
    let mut entries = Vec::new();
    let mut stack = vec![store.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                // A library's debug information (macOS's `.dSYM`, beside it) goes with the library.
                if e.path().extension().is_none_or(|x| x != "dSYM") {
                    stack.push(e.path());
                }
            } else {
                entries.push((m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()));
            }
        }
    }
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    entries.sort();
    let mut removed = 0;
    for (_, len, path) in entries {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            let mut sym = path.into_os_string();
            sym.push(".dSYM");
            let _ = std::fs::remove_dir_all(sym);
            total -= len;
            removed += 1;
        }
    }
    removed
}

/// Prunes the store once a process, at its first save (`save`, or a backend that writes its entry itself).
pub fn prune_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| _ = prune(max_bytes()));
}

/// Keeps `bytes` as `backend`'s form of `key`: written under a name of its own, then renamed into place, so a
/// reader never sees part of it (another process may write the same form meanwhile: either is the same).
pub fn save(backend: &str, key: &str, ext: &str, bytes: &[u8]) {
    prune_once();
    let Some(p) = path(backend, key, ext) else { return };
    let tmp = p.with_extension(format!("{ext}.{}.{}", std::process::id(), unique()));
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, &p).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// `RERAIL_GPU_DUMP=<dir>`, a measurement aid: each source a backend's compiler is given, as `<label>.<ext>` in
/// `<dir>` with its options in a first comment line, to compile again outside the engine (decision 0395 §6).
pub fn dump(label: &str, ext: &str, options: &str, source: &str) {
    let Some(dir) = std::env::var_os("RERAIL_GPU_DUMP") else { return };
    let dir = Path::new(&dir);
    let _ = std::fs::create_dir_all(dir);
    let name: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { '_' }).collect();
    let _ = std::fs::write(dir.join(format!("{name}.{ext}")), format!("// options: {options}\n{source}"));
}

/// A number no other call in this process gives: a temporary file's own name.
pub fn unique() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

thread_local! {
    static HERE: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
    static TOUCHED: std::cell::RefCell<Option<Vec<PathBuf>>> = const { std::cell::RefCell::new(None) };
}

/// Counted on this thread: a library loaded from the store (`true`) or compiled.
pub fn count(from_store: bool) {
    HERE.with(|h| {
        let (s, c) = h.get();
        h.set(if from_store { (s + 1, c) } else { (s, c + 1) });
    });
}

/// The libraries this thread loaded from the store, and compiled: a test's own counts, which the tests beside it
/// on other threads do not move.
pub fn here() -> (u64, u64) {
    HERE.with(Cell::get)
}
