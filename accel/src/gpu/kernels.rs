//! Metal kernels: the one path from MSL source to a compute pipeline, and the store of compiled kernels every
//! process of the user shares (decision 0395).
//!
//! ```text
//! RERAIL_METAL_KERNELS=<dir>|off      the store: ~/Library/Caches/rerail/metal-kernels by default
//! RERAIL_SHIPPED_KERNELS=<dir>|off    the kernels shipped with the engine: rerail-kernels/ beside it
//! RERAIL_METAL_COMPILE=wait|background|defer
//!                                     what a caller with another form of the work (`ready`) gets while a
//!                                     kernel is not compiled: `wait` compiles it on the caller's thread (the
//!                                     library's default: tests); `background` runs the other form and compiles
//!                                     it on a thread of this process; `defer` runs the other form and leaves the
//!                                     compile to maintenance (the engine's runs: upkeep, the compile after a
//!                                     run, `rerail-engine kernels compile`)
//! ```
//!
//! A pipeline costs two compiles: the MSL front end (source to AIR, per source) and the back end (AIR to the GPU's
//! code, per kernel and device). The store keeps both forms a later process can use:
//!
//! - **Sources**, `src/<library>.metal`: what was wanted and not compiled, for maintenance to compile.
//! - **Binary archives**, `<device>/<pipeline>.archive`: one pipeline's GPU code, keyed by its source, kernel name
//!   and constant, under the device that made it (`device_tag`). A pipeline made from its archive skips the back
//!   end.
//! - **Wanted lists**, `<device>/wanted/<run>`: the pipelines a run went without (`ready` in `defer` or
//!   `background`), one line each, so `compile_wanted` compiles exactly the last run's.
//!
//! Shipped kernels (`rerail-engine kernels ship`) are the interpreters' fixed sources compiled ahead of time:
//! `<library>.metallib` (AIR, any Apple GPU: skips the front end) and `<device>/<pipeline>.archive` for the
//! devices they were built on. They are the middle tier between a kernel generated for the work (compiled later)
//! and the CPU.
//!
//! A kernel that does not compile is a bug, and fatal: the run stops and says which kernel and why. A kernel not
//! compiled yet is not an error: `ready` says `None`, and its caller runs another form of the work.

use metal::{ComputePipelineState, Device, Library};
/// A kernel as the device layer launches it (decision 0440 step C): on Metal a pipeline with its arguments'
/// reflection, made through this store; elsewhere the device layer's compile, kept in its store.
pub type Function = rerail_device::device::ComputeFunction;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// What `ready` does about a kernel that is not compiled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Compile it on the caller's thread, and return it.
    Wait,
    /// Return `None`, and compile it on a thread of this process; a later `ready` finds it.
    Background,
    /// Return `None`, and record it for maintenance.
    Defer,
}

/// The mode its caller set on a machine (`set_mode`; decision 0442: the machine's, as its pipelines are).
#[derive(Default)]
struct ModeSet(Mutex<Option<Mode>>);

/// The mode: `RERAIL_METAL_COMPILE`, else what `set_mode` set on the run's machine, else `Wait`.
pub fn mode() -> Mode {
    match std::env::var("RERAIL_METAL_COMPILE").as_deref() {
        Ok("wait") => return Mode::Wait,
        Ok("background") => return Mode::Background,
        Ok("defer") => return Mode::Defer,
        _ => {}
    }
    rerail_context::machine::<ModeSet>().0.lock().unwrap_or_else(|p| p.into_inner()).unwrap_or(Mode::Wait)
}

/// Sets the mode for the runs on `machine` (the engine's command: `Defer`); `RERAIL_METAL_COMPILE` still wins.
pub fn set_mode(machine: &rerail_context::Machine, mode: Mode) {
    *machine.get::<ModeSet>().0.lock().unwrap_or_else(|p| p.into_inner()) = Some(mode);
}

/// A kernel's identity: its library (the source's digest) and its pipeline (the library, the kernel's name and
/// its constant).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Key {
    pub library: [u8; 32],
    pub pipeline: [u8; 32],
}

impl Key {
    pub fn of(source: &str, name: &str, constant: Option<u32>) -> Key {
        // A source that takes UKL's library names its headers, not their text: the library's fingerprint keys it too.
        let library: [u8; 32] = if rerail_ukl::takes_library(source) {
            let mut d = Sha256::new();
            d.update(source.as_bytes());
            d.update(rerail_ukl::fingerprint().to_le_bytes());
            d.finalize().into()
        } else {
            Sha256::digest(source.as_bytes()).into()
        };
        let mut d = Sha256::new();
        d.update(library);
        d.update(name.as_bytes());
        d.update([0]);
        match constant {
            Some(c) => {
                d.update([1]);
                d.update(c.to_le_bytes());
            }
            None => d.update([0]),
        }
        Key { library, pipeline: d.finalize().into() }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Where the store and the shipped kernels are, and this device's tag in them.
struct Places {
    store: Option<PathBuf>,
    shipped: Option<PathBuf>,
    tag: String,
    /// This process's wanted list.
    run: String,
}

/// The machine's places, found the first time they are asked for (decision 0442: the machine's).
#[derive(Default)]
struct PlacesFound(OnceLock<Arc<Places>>);

fn places() -> Arc<Places> {
    let found = rerail_context::machine::<PlacesFound>();
    let places = found.0.get_or_init(|| {
        let store = match std::env::var("RERAIL_METAL_KERNELS") {
            Ok(v) if v == "off" => None,
            Ok(v) if !v.is_empty() => Some(PathBuf::from(v)),
            _ => std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches/rerail/metal-kernels")),
        };
        let shipped = match std::env::var("RERAIL_SHIPPED_KERNELS") {
            Ok(v) if v == "off" => None,
            Ok(v) if !v.is_empty() => Some(PathBuf::from(v)),
            _ => std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("rerail-kernels"))).filter(|p| p.is_dir()),
        };
        let tag = super::shared::get().map(|s| device_tag(&s.device)).unwrap_or_else(|| "no-device".to_owned());
        let started = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        Arc::new(Places { store, shipped, tag, run: format!("{started:024}-{}", std::process::id()) })
    });
    Arc::clone(places)
}

/// The store's directory, when it is on.
pub fn store_dir() -> Option<PathBuf> {
    places().store.clone()
}

/// The shipped kernels' directory, when there is one.
pub fn shipped_dir() -> Option<PathBuf> {
    places().shipped.clone()
}

/// A device's tag: its name. A binary archive is the GPU code of one device and driver; Metal refuses an archive of
/// another driver (`FailOnBinaryArchiveMiss`: a miss, not an error), and the store's is then compiled again and
/// replaced, so an update of the OS costs each kernel one compile, and shipped archives of an older OS are
/// replaced only when Metal refuses them.
pub fn device_tag(device: &Device) -> String {
    device.name().chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect()
}

/// A file URL of `path`, owned. Not `metal::URL::new_with_string`: it wraps `URLWithString:`'s autoreleased URL as
/// owned, so dropping it releases the URL once too often (a crash in the next autorelease drain).
#[allow(unexpected_cfgs)]
fn url_of(path: &Path) -> metal::URL {
    use metal::foreign_types::ForeignType;
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("a path without a NUL");
    // SAFETY: an NSString of the path (autoreleased: the caller's pool), and a URL made by alloc/init (+1, owned).
    unsafe {
        let s: *mut Object = msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()];
        let u: *mut Object = msg_send![class!(NSURL), alloc];
        let u: *mut Object = msg_send![u, initFileURLWithPath: s];
        metal::URL::from_ptr(u.cast())
    }
}

/// Writes `bytes` to `path` whole: a temporary file renamed over it, so a reader in another process sees the old
/// file or the new one.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

type Slot<T> = Arc<Mutex<Option<T>>>;

/// The process's compiled libraries and pipelines, by key (one manager, decision 0100 §2). A thread that asks
/// while another compiles one waits for that compile. Released by `release` (the residency's drop).
#[derive(Default)]
struct Made {
    libraries: HashMap<[u8; 32], Slot<Library>>,
    pipelines: HashMap<[u8; 32], Slot<Arc<Function>>>,
    /// Pipelines a background compile is making (`Mode::Background`).
    compiling: std::collections::HashSet<[u8; 32]>,
    /// Pipelines this process recorded as wanted (once each).
    wanted: std::collections::HashSet<[u8; 32]>,
    /// Fixed pipelines this process recorded in the manifest (once each).
    fixed: std::collections::HashSet<[u8; 32]>,
    /// The kernels on a device other than Metal (`elsewhere`), by pipeline key and constant.
    elsewhere: HashMap<([u8; 32], Option<u32>), Arc<Function>>,
}

/// The machine's compiled kernels (decision 0442: the device's pipelines are the machine's).
#[derive(Default)]
struct MadeHeld(Mutex<Made>);

fn made() -> Arc<MadeHeld> {
    rerail_context::machine::<MadeHeld>()
}

/// Releases every compiled library and pipeline this process holds (the store keeps them).
pub fn release() {
    let made = made();
    let mut m = made.0.lock().unwrap_or_else(|p| p.into_inner());
    m.libraries.clear();
    m.pipelines.clear();
}

/// What a run went without and maintenance compiled, for the profile and `rerail-engine kernels`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub from_memory: u64,
    pub from_archive: u64,
    pub compiled: u64,
    pub front_end_s: f64,
    pub back_end_s: f64,
    pub wanted: u64,
}

/// The machine's counts (decision 0442: the machine's, as the kernels they count).
#[derive(Default)]
struct CountsHeld(Mutex<Counts>);

thread_local! {
    /// Pipelines this thread made (compiled, or from an archive), not found in the process's memory.
    static MADE_HERE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Of them, those made from an archive and those compiled (`here`).
    static HERE: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
}

/// The pipelines this thread made from an archive, and compiled: a test's own counts, which the process's
/// (`counts`) are not while other tests make pipelines beside it.
pub fn here() -> (u64, u64) {
    HERE.get()
}

/// The pipelines this thread made, not found in the process's memory (a test's own, whatever other tests make).
pub fn made_here() -> u64 {
    MADE_HERE.get()
}

/// The pipelines the process holds.
pub fn held() -> usize {
    made().0.lock().unwrap_or_else(|p| p.into_inner()).pipelines.values().filter(|s| s.try_lock().is_ok_and(|p| p.is_some())).count()
}

pub fn counts() -> Counts {
    *rerail_context::machine::<CountsHeld>().0.lock().unwrap_or_else(|p| p.into_inner())
}

fn count(f: impl FnOnce(&mut Counts)) {
    f(&mut rerail_context::machine::<CountsHeld>().0.lock().unwrap_or_else(|p| p.into_inner()));
}

/// The kernel `name` (with `constant` at function constant 0) of `source` (UKL), as the device layer launches it, on
/// the process's device. On Metal: from this process, else from an archive (shipped or in the store), else compiled
/// on this thread and archived in the store, with its arguments' reflection. For a caller with no other form of the
/// work. A kernel that does not compile is fatal. Elsewhere `source`
/// (UKL) compiled by the device layer, `constant` given as the define `UKL_CONSTANT_0` (`UKL_CONSTANT_U32`).
pub fn function(source: &str, name: &str, constant: Option<u32>) -> Arc<Function> {
    if super::shared::get().is_none() {
        return elsewhere(source, name, constant).unwrap_or_else(|e| panic!("the kernel {name}: {e}"));
    }
    let key = Key::of(source, name, constant);
    let slot = Arc::clone(made().0.lock().unwrap_or_else(|p| p.into_inner()).pipelines.entry(key.pipeline).or_default());
    let mut s = match slot.try_lock() {
        Ok(s) => s,
        Err(std::sync::TryLockError::WouldBlock) => crate::score::phase("pipeline wait", || slot.lock().unwrap_or_else(|p| p.into_inner())),
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
    };
    if let Some(p) = &*s {
        count(|c| c.from_memory += 1);
        return Arc::clone(p);
    }
    let p = make(key, source, name, constant, true).expect("a pipeline made with compiling allowed");
    *s = Some(Arc::clone(&p));
    p
}

/// A kernel on a device other than Metal: compiled by the device layer (`rerail_gpu::kernels`, its store keeping
/// the backend's compiled form).
fn elsewhere(source: &str, name: &str, constant: Option<u32>) -> Result<Arc<Function>, String> {
    let key = (Key::of(source, name, constant).pipeline, constant);
    let held = made();
    if let Some(f) = held.0.lock().unwrap_or_else(|p| p.into_inner()).elsewhere.get(&key) {
        return Ok(Arc::clone(f));
    }
    let value = constant.map(|c| format!("{c}u"));
    let defines: Vec<(&str, &str)> = value.as_deref().map(|v| ("UKL_CONSTANT_0", v)).into_iter().collect();
    let program = rerail_gpu::kernels::program_with(&format!("{name}.uglykl"), source, &defines)?;
    let shared = rerail_gpu::shared::get().ok_or("no device")?;
    let f = Arc::new(program.library().get_function(&shared.device, name)?);
    // Made here, not found in this process's memory: as Metal's kernels are counted (`made_here`).
    MADE_HERE.set(MADE_HERE.get() + 1);
    held.0.lock().unwrap_or_else(|p| p.into_inner()).elsewhere.insert(key, Arc::clone(&f));
    Ok(f)
}

/// [`function`] of a fixed source: an interpreter's or a relational kernel's, the same for every rule set. Recorded
/// in the store's manifest of fixed kernels (`fixed.list`), which `ship` compiles ahead of time for the engine's
/// bundle: the tier between a kernel generated for the work and the CPU.
pub fn function_fixed(source: &str, name: &str, constant: Option<u32>) -> Arc<Function> {
    let f = function(source, name, constant);
    if super::shared::get().is_some() {
        record_fixed(source, name, constant);
    }
    f
}

fn record_fixed(source: &str, name: &str, constant: Option<u32>) {
    let key = Key::of(source, name, constant);
    if !made().0.lock().unwrap_or_else(|p| p.into_inner()).fixed.insert(key.pipeline) {
        return;
    }
    let p = places();
    let Some(store) = p.store.as_ref() else { return };
    let src = store.join("src").join(format!("{}.metal", hex(&key.library)));
    if !src.is_file() {
        let _ = write_whole(&src, source.as_bytes());
    }
    let list = store.join("fixed.list");
    let line = format!("{} {} {}\n", hex(&key.library), name, constant.map_or("-".to_owned(), |c| c.to_string()));
    if std::fs::read_to_string(&list).is_ok_and(|t| t.lines().any(|l| format!("{l}\n") == line)) {
        return;
    }
    use std::io::Write as _;
    let _ = std::fs::File::options().create(true).append(true).open(&list).and_then(|mut f| f.write_all(line.as_bytes()));
}

/// The kernel ([`function`]) when there is a form of it this process can have without the back end's compile (made
/// here before, or an archive); otherwise `None`, and what the mode says (`Mode`): the caller runs another form of the
/// work meanwhile. In `Wait` it is compiled here and returned. Elsewhere than Metal, compiled on the caller's
/// thread: the device layer's store is its tier.
pub fn ready_function(source: &str, name: &str, constant: Option<u32>) -> Option<Arc<Function>> {
    if super::shared::get().is_none() {
        return Some(function(source, name, constant));
    }
    ready_in(mode(), source, name, constant)
}

/// [`ready_function`] that never waits: in `Wait`, the compile runs behind the caller as in `Background` (decision
/// 0012's callers, which swap to the kernel at a later batch). Elsewhere than Metal, compiled on the caller's thread.
pub fn ready_behind_function(source: &str, name: &str, constant: Option<u32>) -> Option<Arc<Function>> {
    if super::shared::get().is_none() {
        return Some(function(source, name, constant));
    }
    let mode = match mode() {
        Mode::Wait => Mode::Background,
        m => m,
    };
    ready_in(mode, source, name, constant)
}

fn ready_in(mode: Mode, source: &str, name: &str, constant: Option<u32>) -> Option<Arc<Function>> {
    if mode == Mode::Wait {
        return Some(function(source, name, constant));
    }
    let key = Key::of(source, name, constant);
    let slot = Arc::clone(made().0.lock().unwrap_or_else(|p| p.into_inner()).pipelines.entry(key.pipeline).or_default());
    // A slot held is a compile in progress (or a load): the other form, now.
    let mut s = slot.try_lock().ok()?;
    if let Some(p) = &*s {
        count(|c| c.from_memory += 1);
        return Some(Arc::clone(p));
    }
    if let Some(p) = make(key, source, name, constant, false) {
        *s = Some(Arc::clone(&p));
        return Some(p);
    }
    drop(s);
    want(key, source, name, constant);
    crate::forms::interpreted("msl-pipeline", &hex(&key.pipeline), name, 1);
    if mode == Mode::Background {
        let first = made().0.lock().unwrap_or_else(|p| p.into_inner()).compiling.insert(key.pipeline);
        if first {
            let (source, name) = (source.to_owned(), name.to_owned());
            let spawned = crate::exec::service("rerail-msl-compile", None, move || {
                crate::upkeep::lower_this_thread();
                let started = std::time::Instant::now();
                let _ = function(&source, &name, constant);
                crate::forms::compiled("msl-pipeline", &hex(&key.pipeline), &name, started.elapsed().as_secs_f64(), true);
                made().0.lock().unwrap_or_else(|p| p.into_inner()).compiling.remove(&key.pipeline);
            });
            if spawned.is_err() {
                made().0.lock().unwrap_or_else(|p| p.into_inner()).compiling.remove(&key.pipeline);
            }
        }
    }
    None
}

/// Makes the pipeline: its library (this process's, a shipped AIR library, or the front end's compile), then the
/// pipeline from an archive, or (when `compile`) the back end's compile into a new archive, kept in the store.
fn make(key: Key, source: &str, name: &str, constant: Option<u32>, compile: bool) -> Option<Arc<Function>> {
    let made = make_uncounted(key, source, name, constant, compile);
    if made.is_some() {
        MADE_HERE.set(MADE_HERE.get() + 1);
    }
    made
}

fn make_uncounted(key: Key, source: &str, name: &str, constant: Option<u32>, compile: bool) -> Option<Arc<Function>> {
    let device = &super::shared::get().unwrap_or_else(|| panic!("the Metal kernel {name}: no Metal device")).device;
    let archive = archive_path(&key);
    if !compile && archive.is_none() {
        return None;
    }
    let library = library(key.library, source, name);
    let function = objc::rc::autoreleasepool(|| {
        let constants = constant.map(|value| {
            let c = metal::FunctionConstantValues::new();
            c.set_constant_value_at_index((&value as *const u32).cast(), metal::MTLDataType::UInt, 0);
            c
        });
        library.get_function(name, constants).unwrap_or_else(|e| panic!("the Metal kernel {name} (library {}): {e}", hex(&key.library[..8])))
    });
    let descriptor = metal::ComputePipelineDescriptor::new();
    descriptor.set_label(name);
    descriptor.set_compute_function(Some(&function));
    // The pipeline with its arguments' reflection: the device layer binds a launch's arguments by their names.
    let kernel = |(state, reflection): (ComputePipelineState, metal::ComputePipelineReflection)| {
        let f = rerail_device::compute_kernel_metal::MetalComputeFunction::from_reflection(function.clone(), state, &reflection)
            .unwrap_or_else(|e| panic!("the Metal kernel {name}'s arguments: {e}"));
        Arc::new(Function::Native(f))
    };
    if let Some(path) = archive {
        let loaded = crate::score::phase("pipeline from archive", || from_archive(device, &descriptor, &path));
        match loaded {
            Some(p) => {
                count(|c| c.from_archive += 1);
                HERE.set((HERE.get().0 + 1, HERE.get().1));
                // Its use, for the store's pruning (the oldest unused first).
                let _ = std::fs::File::options().append(true).open(&path).and_then(|f| f.set_modified(std::time::SystemTime::now()));
                return Some(kernel(p));
            }
            // An archive of another driver's build, or damaged: compiled again (when allowed) and replaced.
            None if !compile => return None,
            None => {}
        }
    }
    let started = std::time::Instant::now();
    let p = crate::score::phase("pipeline back end", || objc::rc::autoreleasepool(|| compile_archived(device, &descriptor, &key)))
        .unwrap_or_else(|e| panic!("the Metal kernel {name} (library {}) does not compile: {e}", hex(&key.library[..8])));
    let seconds = started.elapsed().as_secs_f64();
    count(|c| {
        c.compiled += 1;
        c.back_end_s += seconds;
    });
    HERE.set((HERE.get().0, HERE.get().1 + 1));
    Some(kernel(p))
}

/// The archive that holds `key`'s pipeline for this device: the shipped one, else the store's.
fn archive_path(key: &Key) -> Option<PathBuf> {
    let p = places();
    let file = format!("{}.archive", hex(&key.pipeline));
    [p.shipped.as_ref(), p.store.as_ref()].into_iter().flatten().map(|d| d.join(&p.tag).join(&file)).find(|f| f.is_file())
}

/// A pipeline with its reflection.
type Reflected = (ComputePipelineState, metal::ComputePipelineReflection);

/// What a pipeline is made with: its arguments' reflection.
fn reflected() -> metal::MTLPipelineOption {
    metal::MTLPipelineOption::ArgumentInfo | metal::MTLPipelineOption::BufferTypeInfo
}

fn from_archive(device: &Device, descriptor: &metal::ComputePipelineDescriptorRef, path: &Path) -> Option<Reflected> {
    objc::rc::autoreleasepool(|| {
        let d = metal::BinaryArchiveDescriptor::new();
        d.set_url(&url_of(path));
        let archive = device.new_binary_archive_with_descriptor(&d).ok()?;
        descriptor.set_binary_archives(&[&archive]);
        let p = device.new_compute_pipeline_state_with_reflection(descriptor, metal::MTLPipelineOption::FailOnBinaryArchiveMiss | reflected());
        descriptor.set_binary_archives(&[]);
        p.ok()
    })
}

/// The back end's compile, into an archive the store keeps: adding the pipeline to a new archive compiles it, and
/// the pipeline is then made from that archive (no second compile). Without a store, a plain compile.
fn compile_archived(device: &Device, descriptor: &metal::ComputePipelineDescriptorRef, key: &Key) -> Result<Reflected, String> {
    let p = places();
    let Some(store) = p.store.as_ref() else {
        return device.new_compute_pipeline_state_with_reflection(descriptor, reflected());
    };
    let archive = device.new_binary_archive_with_descriptor(&metal::BinaryArchiveDescriptor::new())?;
    archive.add_compute_pipeline_functions_with_descriptor(descriptor)?;
    let path = store.join(&p.tag).join(format!("{}.archive", hex(&key.pipeline)));
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // A store that cannot be written (full, read-only) costs the next process this compile, nothing else.
    if archive.serialize_to_url(&url_of(&tmp)).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
    descriptor.set_binary_archives(&[&archive]);
    let p = device
        .new_compute_pipeline_state_with_reflection(descriptor, metal::MTLPipelineOption::FailOnBinaryArchiveMiss | reflected())
        .or_else(|_| device.new_compute_pipeline_state_with_reflection(descriptor, reflected()));
    descriptor.set_binary_archives(&[]);
    p
}

/// `source` as the Metal front end takes it: a source that takes UKL's library (`#include <ukl/...>`: the reads
/// written once in CLIF, decision 0440 §3.3) preprocessed with it in place, any other as it is.
pub fn msl<'a>(source: &'a str, name: &str) -> Result<std::borrow::Cow<'a, str>, String> {
    if !rerail_ukl::takes_library(source) {
        return Ok(std::borrow::Cow::Borrowed(source));
    }
    rerail_ukl::metal_source(name, source).map(std::borrow::Cow::Owned).map_err(|e| format!("{name}: {e}"))
}

/// The library of `source` (its digest `library`): this process's, else the shipped AIR library, else the front
/// end's compile.
fn library(library: [u8; 32], source: &str, name: &str) -> Library {
    let slot = Arc::clone(made().0.lock().unwrap_or_else(|p| p.into_inner()).libraries.entry(library).or_default());
    let mut s = slot.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(l) = &*s {
        return l.clone();
    }
    let device = &super::shared::get().expect("a Metal device").device;
    // A measurement aid: `RERAIL_DEVICE_TRACE` prints where each library is loaded.
    if std::env::var_os("RERAIL_DEVICE_TRACE").is_some() {
        eprintln!("RERAIL_DEVICE_TRACE a library loaded, kernel {name} (driver {} MB):\n{}", device.current_allocated_size() >> 20, std::backtrace::Backtrace::force_capture());
    }
    let shipped = places().shipped.as_ref().map(|d| d.join(format!("{}.metallib", hex(&library)))).filter(|f| f.is_file());
    let started = std::time::Instant::now();
    let l = match shipped {
        Some(file) => crate::score::phase("msl from shipped", || objc::rc::autoreleasepool(|| device.new_library_with_file(&file)))
            .unwrap_or_else(|e| panic!("the shipped Metal library {} (kernel {name}) does not load: {e}", file.display())),
        None => {
            let source = &*msl(source, name).unwrap_or_else(|e| panic!("the Metal library of kernel {name} does not preprocess: {e}"));
            let nonce;
            // `RERAIL_DIAG_MSL_NONCE`: every source made new to the system's shader cache, to measure first compiles.
            let text = match std::env::var("RERAIL_DIAG_MSL_NONCE") {
                Ok(n) => {
                    nonce = format!("{source}\n// {n}\n");
                    &nonce
                }
                Err(_) => source,
            };
            // A measurement's sources (`RERAIL_METAL_DUMP=<dir>`): each library's source by its digest, and the kernels
            // made of it, to compile apart and cold (`scripts/dev/metal/compile-time.swift`).
            if let Some(dir) = std::env::var_os("RERAIL_METAL_DUMP") {
                let stem = Path::new(&dir).join(hex(&library[..6]));
                if !stem.with_extension("metal").exists() {
                    let _ = std::fs::write(stem.with_extension("metal"), text);
                }
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(stem.with_extension("pipelines")) {
                    use std::io::Write as _;
                    let _ = writeln!(f, "{name}");
                }
            }
            let l = crate::score::phase("msl front end", || objc::rc::autoreleasepool(|| device.new_library_with_source(text, &metal::CompileOptions::new())))
                .unwrap_or_else(|e| panic!("the Metal library of kernel {name} ({}) does not compile: {e}", hex(&library[..8])));
            let seconds = started.elapsed().as_secs_f64();
            count(|c| c.front_end_s += seconds);
            l
        }
    };
    *s = Some(l.clone());
    l
}

/// Records `key` as wanted: its source in the store (once), and a line in this run's wanted list.
fn want(key: Key, source: &str, name: &str, constant: Option<u32>) {
    if !made().0.lock().unwrap_or_else(|p| p.into_inner()).wanted.insert(key.pipeline) {
        return;
    }
    count(|c| c.wanted += 1);
    let p = places();
    let Some(store) = p.store.as_ref() else { return };
    let src = store.join("src").join(format!("{}.metal", hex(&key.library)));
    if !src.is_file() {
        let _ = write_whole(&src, source.as_bytes());
    }
    let list = store.join(&p.tag).join("wanted").join(&p.run);
    let line = format!("{} {} {} {}\n", hex(&key.pipeline), hex(&key.library), name, constant.map_or("-".to_owned(), |c| c.to_string()));
    let _ = std::fs::create_dir_all(list.parent().expect("a wanted list's directory"));
    use std::io::Write as _;
    let _ = std::fs::File::options().create(true).append(true).open(&list).and_then(|mut f| f.write_all(line.as_bytes()));
}

/// The pipelines this process recorded as wanted.
pub fn wanted_here() -> u64 {
    counts().wanted
}

/// One wanted pipeline, read from a wanted list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub pipeline: String,
    pub library: String,
    pub name: String,
    pub constant: Option<u32>,
}

/// Which wanted lists `compile_wanted` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    /// The newest run's (of another process than this one).
    Last,
    /// Every run's.
    All,
}

fn wanted_lists(which: Which) -> Vec<PathBuf> {
    let p = places();
    let Some(store) = p.store.as_ref() else { return Vec::new() };
    let mut lists: Vec<PathBuf> = std::fs::read_dir(store.join(&p.tag).join("wanted"))
        .map(|d| d.flatten().map(|e| e.path()).filter(|f| f.file_name().is_some_and(|n| *n != *p.run && !n.to_string_lossy().contains(".tmp"))).collect())
        .unwrap_or_default();
    lists.sort();
    if which == Which::Last {
        lists = lists.pop().into_iter().collect();
    }
    lists
}

fn read_list(list: &Path) -> Vec<Wanted> {
    std::fs::read_to_string(list)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut w = l.split(' ');
            Some(Wanted { pipeline: w.next()?.to_owned(), library: w.next()?.to_owned(), name: w.next()?.to_owned(), constant: w.next()?.parse().ok() })
        })
        .collect()
}

/// The wanted pipelines not in the store yet, once each, from `which` lists.
pub fn pending(which: Which) -> Vec<Wanted> {
    let p = places();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for list in wanted_lists(which) {
        for w in read_list(&list) {
            let done = p.store.as_ref().is_some_and(|s| s.join(&p.tag).join(format!("{}.archive", w.pipeline)).is_file());
            if !done && seen.insert(w.pipeline.clone()) {
                out.push(w);
            }
        }
    }
    out
}

/// One compile `compile_wanted` did.
#[derive(Clone, Debug)]
pub struct Compiled {
    pub name: String,
    pub pipeline: String,
    pub seconds: f64,
    /// Why it was not compiled (its source gone from the store).
    pub skipped: Option<String>,
}

/// Compiles the wanted pipelines of `which` lists not in the store yet, on this thread, each under a lock that
/// other processes compiling the store respect (a pipeline another holds is skipped). `limit` bounds how many (an
/// upkeep step's one). Lists whose every pipeline is in the store are removed.
pub fn compile_wanted(which: Which, limit: usize) -> Vec<Compiled> {
    let p = places();
    let Some(store) = p.store.as_ref() else { return Vec::new() };
    let mut out = Vec::new();
    for w in pending(which).into_iter().take(limit) {
        let src = store.join("src").join(format!("{}.metal", w.library));
        let Ok(source) = std::fs::read_to_string(&src) else {
            out.push(Compiled { name: w.name, pipeline: w.pipeline, seconds: 0.0, skipped: Some("its source is not in the store".to_owned()) });
            continue;
        };
        let Some(_lock) = Lock::take(&store.join(&p.tag).join(format!("{}.lock", w.pipeline))) else {
            out.push(Compiled { name: w.name, pipeline: w.pipeline, seconds: 0.0, skipped: Some("another process compiles it".to_owned()) });
            continue;
        };
        let key = Key::of(&source, &w.name, w.constant);
        if hex(&key.pipeline) != w.pipeline {
            out.push(Compiled { name: w.name, pipeline: w.pipeline, seconds: 0.0, skipped: Some("its stored source is another".to_owned()) });
            continue;
        }
        let started = std::time::Instant::now();
        let _ = make(key, &source, &w.name, w.constant, true);
        let seconds = started.elapsed().as_secs_f64();
        crate::forms::compiled("msl-pipeline", &w.pipeline, &w.name, seconds, true);
        out.push(Compiled { name: w.name, pipeline: w.pipeline, seconds, skipped: None });
    }
    for list in wanted_lists(Which::All) {
        let all_done = read_list(&list).iter().all(|w| store.join(&p.tag).join(format!("{}.archive", w.pipeline)).is_file());
        if all_done {
            let _ = std::fs::remove_file(&list);
        }
    }
    out
}

/// An exclusive lock on a file, released when dropped or when the process ends (`flock`).
struct Lock(#[allow(dead_code, reason = "held for its flock")] std::fs::File);

impl Lock {
    fn take(path: &Path) -> Option<Lock> {
        use std::os::fd::AsRawFd;
        let _ = std::fs::create_dir_all(path.parent()?);
        let f = std::fs::File::options().create(true).truncate(false).write(true).open(path).ok()?;
        // SAFETY: an open file's descriptor.
        (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(Lock(f))
    }
}

/// The store's files, and their bytes: archives (this device's and others'), sources, wanted lists.
#[derive(Clone, Copy, Debug, Default)]
pub struct Size {
    pub archives: u64,
    pub archive_bytes: u64,
    pub sources: u64,
    pub source_bytes: u64,
    pub wanted_lists: u64,
}

pub fn size() -> Size {
    let mut s = Size::default();
    let Some(store) = store_dir() else { return s };
    for e in walk(&store) {
        let len = e.1;
        match e.0.extension().and_then(|x| x.to_str()) {
            Some("archive") => {
                s.archives += 1;
                s.archive_bytes += len;
            }
            Some("metal") => {
                s.sources += 1;
                s.source_bytes += len;
            }
            _ if e.0.parent().is_some_and(|d| d.ends_with("wanted")) => s.wanted_lists += 1,
            _ => {}
        }
    }
    s
}

fn walk(dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else {
                out.push((e.path(), m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH)));
            }
        }
    }
    out
}

/// Keeps the store's archives within `max_bytes`: the least recently used go first (another device's or OS
/// build's are never used again here, so they go with them), and sources no wanted list names and no archive
/// was made from.
pub fn prune(max_bytes: u64) -> u64 {
    let Some(store) = store_dir() else { return 0 };
    let mut archives: Vec<_> = walk(&store).into_iter().filter(|e| e.0.extension().is_some_and(|x| x == "archive")).collect();
    let mut total: u64 = archives.iter().map(|e| e.1).sum();
    archives.sort_by_key(|e| e.2);
    let mut removed = 0;
    for (path, len, _) in archives {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
            removed += 1;
        }
    }
    removed
}

/// The store's bound: `RERAIL_METAL_KERNELS_MB`, else 2 GB.
pub fn max_bytes() -> u64 {
    std::env::var("RERAIL_METAL_KERNELS_MB").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(2048) << 20
}

/// Upkeep's job (decision 0190): when the machine is idle, compiles the wanted pipelines one a step, newest run's
/// first, and keeps the store within its bound.
pub struct Maintenance {
    pruned: bool,
}

impl Maintenance {
    pub fn new() -> Maintenance {
        Maintenance { pruned: false }
    }
}

impl Default for Maintenance {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::upkeep::Job for Maintenance {
    fn name(&self) -> String {
        "metal-kernels compile and prune".to_owned()
    }

    fn due(&mut self) -> crate::upkeep::Due {
        if !pending(Which::All).is_empty() || !self.pruned {
            crate::upkeep::Due::Idle { starve: std::time::Duration::from_secs(24 * 3600) }
        } else {
            crate::upkeep::Due::No
        }
    }

    fn step(&mut self, _slice: &crate::upkeep::Slice) -> Result<crate::upkeep::Step, String> {
        if compile_wanted(Which::All, 1).is_empty() {
            prune(max_bytes());
            // The device layer's store (CUDA's cubins, OpenCL's binaries, the CPU's libraries; decision 0440 step B).
            rerail_device::store::prune(rerail_device::store::max_bytes());
            self.pruned = true;
            return Ok(crate::upkeep::Step::Done);
        }
        Ok(crate::upkeep::Step::More)
    }
}

/// Ships the fixed kernels of the store `from` (its manifest, `fixed.list`: what runs on this machine compiled
/// through [`pipeline_fixed`]) into `dir`: this device's archive of each kernel (`<device>/<pipeline>.archive`,
/// which skips the back end) and, with `air`, each library's AIR (`<library>.metallib`, any Apple GPU: skips the
/// front end), made by Xcode's offline compiler (its Metal Toolchain component). Run on another device into the same
/// `dir`, it adds that device's archives. A source that does not compile is an error.
pub fn ship(dir: &Path, from: &Path, air: bool) -> Result<Vec<String>, String> {
    let device = &super::shared::get().ok_or("no Metal device")?.device;
    let tag = device_tag(device);
    let manifest = std::fs::read_to_string(from.join("fixed.list")).map_err(|e| format!("{}: {e} (run the engine with this store first)", from.join("fixed.list").display()))?;
    let mut by_library: std::collections::BTreeMap<String, Vec<(String, Option<u32>)>> = Default::default();
    for line in manifest.lines() {
        let mut w = line.split(' ');
        let (Some(library), Some(name), Some(constant)) = (w.next(), w.next(), w.next()) else { continue };
        let kernels = by_library.entry(library.to_owned()).or_default();
        let k = (name.to_owned(), constant.parse().ok());
        if !kernels.contains(&k) {
            kernels.push(k);
        }
    }
    let mut lines = Vec::new();
    for (library, kernels) in &by_library {
        let source = std::fs::read_to_string(from.join("src").join(format!("{library}.metal"))).map_err(|e| format!("the source of {library}: {e}"))?;
        let source = msl(&source, library)?.into_owned();
        let lib = dir.join(format!("{library}.metallib"));
        let started = std::time::Instant::now();
        if air && !lib.is_file() {
            let src = dir.join("src").join(format!("{library}.metal"));
            write_whole(&src, source.as_bytes()).map_err(|e| format!("{}: {e}", src.display()))?;
            let air = lib.with_extension("air");
            let run = |args: &[&std::ffi::OsStr]| -> Result<(), String> {
                let out = std::process::Command::new("xcrun").args(args).output().map_err(|e| format!("xcrun: {e} (shipping kernels needs Xcode's Metal compiler)"))?;
                if out.status.success() {
                    Ok(())
                } else {
                    Err(format!("{library}: {} (the Metal Toolchain: `xcodebuild -downloadComponent MetalToolchain`)", String::from_utf8_lossy(&out.stderr).trim()))
                }
            };
            run(&["-sdk".as_ref(), "macosx".as_ref(), "metal".as_ref(), "-c".as_ref(), src.as_os_str(), "-o".as_ref(), air.as_os_str()])?;
            run(&["-sdk".as_ref(), "macosx".as_ref(), "metallib".as_ref(), air.as_os_str(), "-o".as_ref(), lib.as_os_str()])?;
            let _ = std::fs::remove_file(&air);
        }
        let l = if lib.is_file() {
            device.new_library_with_file(&lib).map_err(|e| format!("{}: {e}", lib.display()))?
        } else {
            objc::rc::autoreleasepool(|| device.new_library_with_source(&source, &metal::CompileOptions::new())).map_err(|e| format!("{library}: {e}"))?
        };
        let front = started.elapsed().as_secs_f64();
        let started = std::time::Instant::now();
        for (name, constant) in kernels {
            let key = Key::of(&source, name, *constant);
            let path = dir.join(&tag).join(format!("{}.archive", hex(&key.pipeline)));
            if path.is_file() {
                continue;
            }
            objc::rc::autoreleasepool(|| -> Result<(), String> {
                let constants = constant.map(|value| {
                    let c = metal::FunctionConstantValues::new();
                    c.set_constant_value_at_index((&value as *const u32).cast(), metal::MTLDataType::UInt, 0);
                    c
                });
                let function = l.get_function(name, constants).map_err(|e| format!("{library} {name}: {e}"))?;
                let descriptor = metal::ComputePipelineDescriptor::new();
                descriptor.set_label(name);
                descriptor.set_compute_function(Some(&function));
                let archive = device.new_binary_archive_with_descriptor(&metal::BinaryArchiveDescriptor::new())?;
                archive.add_compute_pipeline_functions_with_descriptor(&descriptor).map_err(|e| format!("{library} {name}: {e}"))?;
                std::fs::create_dir_all(path.parent().expect("a device's directory")).map_err(|e| e.to_string())?;
                archive.serialize_to_url(&url_of(&path)).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(())
            })?;
        }
        let names: Vec<&str> = kernels.iter().map(|(n, _)| n.as_str()).take(3).collect();
        lines.push(format!("{} {:>3} kernels  front end {:>7.2} s  back end {:>7.2} s  {}{}", &library[..16], kernels.len(), front, started.elapsed().as_secs_f64(), names.join(" "), if kernels.len() > 3 { " …" } else { "" }));
    }
    Ok(lines)
}

/// Starts this run's maintenance compile when it went without kernels (`Mode::Defer`): the engine, again, as
/// `kernels compile --last --idle` in a process of its own, detached, at the background priority. Off with
/// `RERAIL_METAL_MAINTENANCE=off`.
pub fn spawn_maintenance(engine: &Path) -> Option<u32> {
    if wanted_here() == 0 || std::env::var("RERAIL_METAL_MAINTENANCE").is_ok_and(|v| v == "off") || store_dir().is_none() {
        return None;
    }
    use std::os::unix::process::CommandExt;
    let mut c = std::process::Command::new(engine);
    c.args(["kernels", "compile", "--all", "--idle"]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // SAFETY: `setsid` only, between fork and exec: the child leaves the run's session and process group.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    c.spawn().ok().map(|child| child.id())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decision 0395: a pipeline compiled once is in the store, and made from its archive once this process's
    /// memory is released (as a process that never compiled it would); `ready` in `defer` says `None` for one not
    /// compiled and records it as wanted; once compiled (as maintenance does), `ready` finds it.
    #[test]
    fn a_pipeline_is_kept_in_the_store_and_wanted_ones_compiled_later() {
        if super::super::shared::get().is_none() || store_dir().is_none() {
            return;
        }
        let source = "#include <ukl/backend.h>\nDEFINE_KERNEL2(store, kernels_store_test, RWBUFFER, uint32_t, o, \"[n]\", GID0, uint32_t, i, ) { o[i] = i * 3u; }\n";
        let name = "kernels_store_test";
        let _ = function(source, name, None);
        assert!(archive_path(&Key::of(source, name, None)).is_some(), "its archive is in the store");
        release();
        // This thread's counts: other tests make pipelines beside this one, and `release` drops theirs too.
        let before = here();
        let _ = function(source, name, None);
        assert_eq!(here().0, before.0 + 1, "made again from its archive");
        assert_eq!(here().1, before.1, "not compiled again");

        let other = source.replace("i * 3u", "i * 5u");
        let key = Key::of(&other, name, None);
        if let Some(old) = archive_path(&key) {
            std::fs::remove_file(old).unwrap();
        }
        release();
        set_mode(rerail_context::shared_here().machine(), Mode::Defer);
        assert!(ready_function(&other, name, None).is_none(), "not compiled: the caller runs another form");
        assert!(made().0.lock().unwrap().wanted.contains(&key.pipeline), "recorded as wanted");
        assert!(store_dir().unwrap().join("src").join(format!("{}.metal", hex(&key.library))).is_file(), "its source in the store");
        let _ = make(key, &other, name, None, true);
        release();
        assert!(ready_function(&other, name, None).is_some(), "compiled, it is ready from its archive");
        set_mode(rerail_context::shared_here().machine(), Mode::Wait);
    }
}
