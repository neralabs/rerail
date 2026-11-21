//! The CPU runtime of the grid kernels, beside compute_kernel_metal.rs and compute_kernel_cuda.rs and with their
//! interface (Rerail, decision 0440 §3.5): a library compiled at run time from UKL source by the host's C++
//! compiler and loaded, its functions with their arguments, a queue of launches, copies and fills run in order on a
//! thread of its root's, its events, and buffers. For debugging kernels, not for speed: a kernel runs where a
//! debugger and sanitizers reach it, the same every time, and a barrier no thread can pass is an error that says who
//! waits where, where a GPU would hang (`ukl_cpu_shim.h`).
//!
//! Copyright (c) 2026 Jeremy Barnes. This file is part of MLDB's forest as carried in Rerail. Apache-2.0 (see the
//! LICENSE at `forest/src/mldb/`).
//!
//! - **The compiler** is `RERAIL_CPU_CXX`, else `c++`, found when the device is first asked for; without one there
//!   is no CPU device. A library is compiled once per text and kept by its text's hash under the system's
//!   temporary directory (`rerail-ukl-cpu/`), so a process's second run of a kernel loads it.
//! - **Buffers are the host's memory**, 64-aligned and zeroed when made: kernels read them where they are.
//! - **A queue's root is a thread** (`CpuQueueRoot`): a flush hands it the work since the last, which it runs in
//!   order, as a GPU runs a queue's command buffers; an event is that work's completion. A wait for a host-signalled
//!   event holds the root's later work, as on Metal.
//! - **Each thread of a threadgroup has a stack of its own** (`STACK_BYTES`, a guard page below it), from a region the
//!   root's thread keeps across launches.
//! - **No thread-local in a kernel library**: the running scheduler is in one pthread key of the host's
//!   (`tls_key`), which each launch names; the scheduler's storage is the root thread's (`SCHED_BYTES`). macOS gives each
//!   loaded image's thread-locals a pthread key, and a process ran out of them after a few hundred libraries.
//! - **Arguments without reflection**, as on CUDA (`rows.rs`): the shim's rows and layouts, read from the library's
//!   symbols, and each `ARGS` structure's layout function called on the host.

use crate::compute_kernel::*;
use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, OnceLock};

/// The defines a CPU compile takes from the environment: `RERAIL_CPU_SIMD_WIDTH` (SIMD groups of another width, a
/// power of two up to 32: how a narrower device runs the kernel) and `RERAIL_CPU_OPENCL_RULE=1` (a SIMD-group
/// operation waits for the whole threadgroup, as OpenCL's emulated groups do: a kernel that breaks the rule fails).
pub fn defines() -> Vec<(&'static str, String)> {
    let mut v = Vec::new();
    if simd_width() != 32 {
        v.push(("UKL_SIMD_WIDTH", format!("{}u", simd_width())));
    }
    if std::env::var("RERAIL_CPU_OPENCL_RULE").is_ok_and(|v| v == "1") {
        v.push(("UKL_CPU_OPENCL_RULE", "1".to_string()));
    }
    if strict_simd() {
        v.push(("UKL_CPU_SIMD_STRICT", "1".to_string()));
    }
    v
}

/// `RERAIL_CPU_SIMD_STRICT=1`: a SIMD group's lanes meet only at the same operation reached by the same path
/// (`ukl_cpu_shim.h`'s `__ukl_cpu_site`), compiled without optimization so that none is duplicated: a kernel with a
/// SIMD-group operation some lanes skip fails, naming where each waits.
pub fn strict_simd() -> bool {
    std::env::var("RERAIL_CPU_SIMD_STRICT").is_ok_and(|v| v == "1")
}

/// The CPU's SIMD groups' lanes: `RERAIL_CPU_SIMD_WIDTH`, else 32.
pub fn simd_width() -> u32 {
    match std::env::var("RERAIL_CPU_SIMD_WIDTH") {
        Err(_) => 32,
        Ok(w) => match w.parse::<u32>() {
            Ok(n) if n.is_power_of_two() && n <= 64 => n,
            _ => panic!("RERAIL_CPU_SIMD_WIDTH={w}: a power of two from 1 to 64"),
        },
    }
}

/// Each thread's stack.
const STACK_BYTES: usize = 256 << 10;

/// The host's C++ compiler: `RERAIL_CPU_CXX`, else `c++`, if it runs.
fn compiler() -> Result<&'static str, String> {
    static CXX: OnceLock<Result<String, String>> = OnceLock::new();
    CXX.get_or_init(|| {
        let runs = |cxx: &str| std::process::Command::new(cxx).arg("--version").output().is_ok_and(|o| o.status.success());
        if let Ok(cxx) = std::env::var("RERAIL_CPU_CXX") {
            return if runs(&cxx) { Ok(cxx) } else { Err(format!("no C++ compiler ({cxx}, RERAIL_CPU_CXX)")) };
        }
        // Windows: clang (the shim's assembly and builtins are clang's), on the path or Visual Studio's own.
        let candidates: Vec<String> = if cfg!(windows) { std::iter::once("clang++".to_string()).chain(visual_studio_clang()).collect() } else { vec!["c++".to_string()] };
        candidates.iter().find(|c| runs(c)).cloned().ok_or_else(|| format!("no C++ compiler ({}; set RERAIL_CPU_CXX)", candidates.join(", ")))
    })
    .as_deref()
    .map_err(Clone::clone)
}

/// The clang Visual Studio installs (its "C++ Clang tools" component), as its installer's `vswhere` finds it.
fn visual_studio_clang() -> Option<String> {
    let programs = std::env::var("ProgramFiles(x86)").ok()?;
    let vswhere = format!("{programs}\\Microsoft Visual Studio\\Installer\\vswhere.exe");
    let o = std::process::Command::new(vswhere)
        .args(["-latest", "-products", "*", "-find", "VC\\Tools\\Llvm\\x64\\bin\\clang++.exe"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&o.stdout).lines().next().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string)
}

/// The CPU device: the host, with its C++ compiler.
#[derive(Clone, Copy, Debug)]
pub struct CpuDevice {
    _private: (),
}

pub type CpuDeviceRef = CpuDevice;

impl CpuDevice {
    /// The CPU device, where the host has a C++ compiler.
    pub fn system_default() -> Option<CpuDevice> {
        compiler().ok().map(|_| CpuDevice { _private: () })
    }

    pub fn new_shared_event(&self) -> CpuSharedEvent {
        CpuSharedEvent(Arc::new((Mutex::new(0), Condvar::new())))
    }
}

/// A buffer's memory.
struct Allocation {
    ptr: *mut u8,
    len: u64,
    /// `None`: memory wrapped in place (`wrap_host_memory`), its owner's.
    layout: Option<std::alloc::Layout>,
}

unsafe impl Send for Allocation {}
unsafe impl Sync for Allocation {}

impl Drop for Allocation {
    fn drop(&mut self) {
        if let Some(layout) = self.layout {
            // SAFETY: allocated with this layout.
            unsafe { std::alloc::dealloc(self.ptr, layout) };
        }
    }
}

/// A buffer (`metal::Buffer`): host memory, shared by its clones, freed when the last goes (a queue's work keeps
/// those it uses until it has run).
#[derive(Clone)]
pub struct CpuBuffer(Arc<Allocation>);

pub type CpuBufferRef = CpuBuffer;

impl std::fmt::Debug for CpuBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CpuBuffer({:p}, {} bytes)", self.0.ptr, self.0.len)
    }
}

impl CpuBuffer {
    pub fn contents(&self) -> *mut c_void {
        self.0.ptr.cast()
    }

    pub fn length(&self) -> u64 {
        self.0.len
    }

    /// The kernels' address of the buffer: the host's.
    pub fn device_address(&self) -> u64 {
        self.0.ptr as u64
    }

    pub fn to_owned(&self) -> CpuBuffer {
        self.clone()
    }
}

/// A host-signalled event (`metal::SharedEvent`): a value a queue's work waits on until it reaches the one waited for.
#[derive(Clone)]
pub struct CpuSharedEvent(Arc<(Mutex<u64>, Condvar)>);

impl CpuSharedEvent {
    pub fn set_signaled_value(&self, v: u64) {
        *self.0 .0.lock().unwrap() = v;
        self.0 .1.notify_all();
    }

    pub fn signaled_value(&self) -> u64 {
        *self.0 .0.lock().unwrap()
    }

    fn wait_for(&self, v: u64) {
        let mut g = self.0 .0.lock().unwrap();
        while *g < v {
            g = self.0 .1.wait(g).unwrap();
        }
    }
}

/// `allocUninitializedArraySync`: memory for `length` elements (at least 16 bytes, as on Metal), zeroed.
pub fn alloc_uninitialized_array_sync<T>(_device: &CpuDevice, length: usize) -> CpuBuffer {
    let len = (length * std::mem::size_of::<T>()).max(16);
    let layout = std::alloc::Layout::from_size_align(len, 64).unwrap();
    // SAFETY: a non-zero size.
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    assert!(!ptr.is_null(), "out of memory for a {len}-byte buffer");
    CpuBuffer(Arc::new(Allocation { ptr, len: len as u64, layout: Some(layout) }))
}

/// A buffer over `len` bytes of the host's memory at `ptr`, read in place (decision 0353).
///
/// # Safety
/// The memory outlives the buffer and its clones, and every use of them by queued work.
pub unsafe fn wrap_host_memory(_device: &CpuDevice, ptr: *mut u8, len: usize) -> Result<CpuBuffer, String> {
    Ok(CpuBuffer(Arc::new(Allocation { ptr, len: len as u64, layout: None })))
}

/// `transferToDeviceImmutable`: memory holding `data`.
pub fn transfer_to_device_immutable<T: crate::Pod>(device: &CpuDevice, data: &[T]) -> CpuBuffer {
    let b = alloc_uninitialized_array_sync::<T>(device, data.len());
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().cast::<u8>(), b.contents().cast::<u8>(), std::mem::size_of_val(data)) };
    b
}

/// `transferToHostSync`: the first `length` elements (the work writing them must have completed).
pub fn transfer_to_host_sync<T: crate::Pod + Clone>(buffer: &CpuBuffer, length: usize) -> Vec<T> {
    assert!(length * std::mem::size_of::<T>() <= buffer.length() as usize);
    unsafe { std::slice::from_raw_parts(buffer.contents().cast::<T>(), length).to_vec() }
}

static CPU_NANOS: AtomicU64 = AtomicU64::new(0);

/// The seconds every awaited event's work took on its root's thread, so far in this process.
pub fn gpu_seconds_awaited() -> f64 {
    CPU_NANOS.load(Ordering::Relaxed) as f64 * 1e-9
}

/// A compiled library: the shared object the host's compiler made of the source, loaded.
pub struct CpuComputeFunctionLibrary {
    lib: crate::os::Lib,
    name: String,
}

unsafe impl Send for CpuComputeFunctionLibrary {}
unsafe impl Sync for CpuComputeFunctionLibrary {}

/// The compiler's options for a kernel library. On Windows a DLL has no position-independent option, and no stack
/// probes: Windows' probe walks down from the thread's own stack's limit, past which a coroutine's stack can lie.
#[cfg(not(windows))]
const CXX_FLAGS: [&str; 8] = ["-std=c++20", "-O1", "-g", "-fPIC", "-shared", "-fno-exceptions", "-w", "-o"];
#[cfg(windows)]
const CXX_FLAGS: [&str; 8] = ["-std=c++20", "-O1", "-g", "-mno-stack-arg-probe", "-shared", "-fno-exceptions", "-w", "-o"];

/// A kernel library's file extension: `so` (macOS's `dlopen` takes it too), Windows' `dll`.
const LIB_EXTENSION: &str = if cfg!(windows) { "dll" } else { "so" };

/// What names the compiler's build: its `--version` (a store's key holds it, so an upgrade compiles again).
fn compiler_version(cxx: &str) -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| std::process::Command::new(cxx).arg("--version").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default())
}

/// The kernel library at `lib`, loaded, its symbols its own (`Scope::Local`): every library defines the shim's names.
fn open_library(lib: &std::path::Path) -> Result<crate::os::Lib, String> {
    // A library a compiler made from UKL; it runs no initializers of its own.
    let path = lib.to_string_lossy();
    crate::os::Lib::open(&[path.as_ref()], crate::os::Scope::Local).ok_or_else(|| format!("loading {}: {}", lib.display(), crate::os::Lib::error()))
}

impl CpuComputeFunctionLibrary {
    /// Compiles C++ (UKL preprocessed for the CPU) with the host's compiler into a shared object, and loads it.
    pub fn compile_from_source(_device: &CpuDevice, preprocessed_source: &str, file_name_to_appear_in_error_messages: &str) -> Result<Self, String> {
        let cxx = compiler()?;
        // The store's (`crate::store`), else the temporary directory's: the CPU's form is a file `dlopen` loads.
        let k = crate::store::key(&[b"cpu", cxx.as_bytes(), compiler_version(cxx).as_bytes(), CXX_FLAGS.join(" ").as_bytes(), if strict_simd() { b"-O0" } else { b"" }, preprocessed_source.as_bytes()]);
        let lib = match crate::store::path("cpu", &k, LIB_EXTENSION) {
            Some(p) => p,
            None => {
                let dir = std::env::temp_dir().join("rerail-ukl-cpu");
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                dir.join(format!("{k}.{LIB_EXTENSION}"))
            }
        };
        let dir = lib.parent().expect("a library's directory").to_path_buf();
        let kept = lib.exists();
        if kept {
            match open_library(&lib) {
                Ok(opened) => {
                    crate::store::count(true);
                    crate::store::used(&lib);
                    return Ok(CpuComputeFunctionLibrary { lib: opened, name: file_name_to_appear_in_error_messages.to_string() });
                }
                // Damaged, or another build's: compiled again, and replaced.
                Err(_) => _ = std::fs::remove_file(&lib),
            }
        }
        crate::store::count(false);
        // The CPU's entry is written by the compiler, not `store::save`.
        crate::store::prune_once();
        {
            // A name of this compile's own: another thread or process may compile the same text meanwhile.
            let own = format!("{k}.{}.{}", std::process::id(), crate::store::unique());
            let src = dir.join(format!("{own}.cpp"));
            std::fs::write(&src, preprocessed_source).map_err(|e| format!("{}: {e}", src.display()))?;
            let tmp = dir.join(format!("{own}.{LIB_EXTENSION}"));
            // `-O0` last before `-o`: it overrides the flags' `-O1`.
            let (flags, out) = CXX_FLAGS.split_at(CXX_FLAGS.len() - 1);
            let o = std::process::Command::new(cxx)
                .args(flags)
                .args(strict_simd().then_some("-O0"))
                .args(out)
                .arg(&tmp)
                .arg(&src)
                .output()
                .map_err(|e| format!("{cxx}: {e}"))?;
            let _ = std::fs::remove_file(&src);
            // The linker's import library and exports file beside a Windows DLL: nothing links against it.
            for side in ["lib", "exp", "ilk"].iter().filter(|_| cfg!(windows)) {
                let _ = std::fs::remove_file(tmp.with_extension(side));
            }
            if !o.status.success() {
                let log = String::from_utf8_lossy(&o.stderr);
                let errors: Vec<&str> = log.lines().filter(|l| l.contains("error")).take(20).collect();
                return Err(format!("{cxx} compiling {file_name_to_appear_in_error_messages} for the CPU: {}", errors.join("\n")));
            }
            // Another process may have made it meanwhile: either is the same library. Its debug information (macOS's
            // `.dSYM`, Windows' `.pdb`, beside it) goes with it, where a debugger looks.
            let sym = |p: &std::path::Path| p.with_extension(if cfg!(windows) { "pdb" } else { "so.dSYM" });
            let _ = std::fs::rename(sym(&tmp), sym(&lib));
            if let Err(e) = std::fs::rename(&tmp, &lib) {
                // Windows replaces no library a process has loaded ("Access is denied"): another process made and
                // loaded this one meanwhile, the same library, which this process loads too.
                if !lib.exists() {
                    return Err(format!("{}: {e}", lib.display()));
                }
                let _ = std::fs::remove_file(&tmp);
                let _ = std::fs::remove_file(sym(&tmp));
            }
        }
        let opened = open_library(&lib)?;
        Ok(CpuComputeFunctionLibrary { lib: opened, name: file_name_to_appear_in_error_messages.to_string() })
    }

    fn symbol(&self, name: &str) -> Result<*mut c_void, String> {
        let p = self.lib.sym(name);
        if p.is_null() {
            return Err(format!("{}: no {name}", self.name));
        }
        Ok(p)
    }

    fn string(&self, name: &str) -> Result<String, String> {
        let p = self.symbol(name)?;
        // SAFETY: the shim's strings are NUL-terminated `const char[]`s.
        Ok(unsafe { CStr::from_ptr(p.cast::<c_char>()) }.to_string_lossy().into_owned())
    }

    /// An `args` structure's layout: its layout function called (`UKL_REFLECT_STRUCT`).
    fn struct_type(&self, ty: &str) -> Result<KernelBaseType, String> {
        let names = self.string(&format!("{ty}__ukl_fields")).map_err(|e| format!("{ty} is an ARGS row's type with no UKL_REFLECT_STRUCT: {e}"))?;
        let f = self.symbol(&format!("{ty}__ukl_layout"))?;
        let mut l = vec![0u64; crate::rows::layout_len(&names)];
        // SAFETY: the shim's layout function writes that many numbers.
        let f: extern "C" fn(*mut u64) = unsafe { std::mem::transmute(f) };
        f(l.as_mut_ptr());
        crate::rows::struct_type(ty, &names, &l)
    }

    /// The kernel `function_name`, with its arguments from its rows (`Name__ukl_params`, `Name__ukl_rows`).
    pub fn get_function(&self, _device: &CpuDevice, function_name: &str) -> Result<CpuComputeFunction, String> {
        let entry = self.symbol(function_name)?;
        let text = self.string(&format!("{function_name}__ukl_params"))?;
        let n = crate::rows::split_top(&text).len() / 4 * 3;
        let rows = self.symbol(&format!("{function_name}__ukl_rows"))?;
        // SAFETY: the shim writes three numbers a row.
        let rows = unsafe { std::slice::from_raw_parts(rows.cast::<u64>(), n) }.to_vec();
        let (arguments, param_sizes) = crate::rows::arguments(function_name, &text, &rows, |ty| self.struct_type(ty))?;
        // SAFETY: the shim's entry has this signature.
        let entry: Entry = unsafe { std::mem::transmute(entry) };
        Ok(CpuComputeFunction { entry, name: function_name.to_string(), arguments, param_sizes })
    }
}

/// A launch, as the shim reads it (`__ukl_cpu_launch`).
#[repr(C)]
struct Launch {
    grid: [u32; 3],
    block: [u32; 3],
    shared: *mut u8,
    shared_bytes: u64,
    stacks: *mut u8,
    stack_bytes: u64,
    error: *mut c_char,
    error_bytes: u64,
    sched: *mut u8,
    sched_bytes: u64,
    tls_key: u64,
    resident: u32,
    pad: u32,
}

/// The scheduler's storage a root's thread gives each launch: more than the shim's (`__ukl_cpu_sched`, which checks).
const SCHED_BYTES: usize = 256 << 10;

/// The thread key whose value on a thread is its running launch's scheduler (`__ukl_cpu_sched`; its running thread is
/// the scheduler's): the one thread-local every kernel library uses, read and written by the shim itself
/// (`pthread_getspecific`, Windows' `TlsGetValue`). A library has none of its own: macOS gives each loaded image's
/// thread-locals a pthread key, and a process runs out of them after a few hundred libraries. A pthread key on Unix,
/// a TLS index on Windows.
fn tls_key() -> u64 {
    static KEY: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *KEY.get_or_init(|| {
        #[cfg(unix)]
        {
            let mut key: libc::pthread_key_t = 0;
            // SAFETY: a key made once for the process, with no destructor (its values are the launches', which
            // outlive no launch).
            let made = unsafe { libc::pthread_key_create(&mut key, None) };
            assert_eq!(made, 0, "the CPU backend's thread key: {}", std::io::Error::from_raw_os_error(made));
            key as u64
        }
        #[cfg(windows)]
        {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn TlsAlloc() -> u32;
            }
            const TLS_OUT_OF_INDEXES: u32 = u32::MAX;
            // SAFETY: an index made once for the process, never freed.
            let index = unsafe { TlsAlloc() };
            assert_ne!(index, TLS_OUT_OF_INDEXES, "the CPU backend's thread key: {}", std::io::Error::last_os_error());
            u64::from(index)
        }
    })
}

/// The threads the CPU runs at once across a launch's groups: its resident groups are these over a group's threads
/// (at most 64), so a persistent grid of up to that many groups meets its barriers (`ukl_cpu_shim.h`).
const RESIDENT_THREADS: usize = 8192;

/// The groups of `threads` threads the CPU runs at once.
pub fn resident_groups(threads: usize) -> usize {
    (RESIDENT_THREADS / threads.max(1)).clamp(1, 64)
}

/// A kernel's entry: a pointer to each argument's bytes, and the launch; 0, or 1 with the launch's error written.
type Entry = extern "C" fn(*const *const c_void, *const Launch) -> i32;

/// A kernel of a library, with its arguments (`MetalComputeFunction`).
pub struct CpuComputeFunction {
    entry: Entry,
    pub name: String,
    arguments: Vec<GridComputeFunctionArgument>,
    param_sizes: Vec<usize>,
}

impl CpuComputeFunction {
    pub fn get_argument_info(&self) -> &[GridComputeFunctionArgument] {
        &self.arguments
    }
}

/// An operation of a queue, run on its root's thread.
type Op = Box<dyn FnOnce(&mut Worker) -> Result<(), String> + Send>;

/// Work a flush handed over, and where its completion goes.
struct Batch {
    ops: Vec<Op>,
    done: Arc<Done>,
}

#[derive(Default)]
struct Done {
    result: Mutex<Option<(Result<(), String>, f64)>>,
    cv: Condvar,
}

/// The root's thread's own: the threads' stacks, kept across launches.
struct Worker {
    stacks: Option<crate::os::Stacks>,
    page: usize,
    sched: Vec<u64>,
}

impl Worker {
    /// Stacks for `n` threads, each `STACK_BYTES` with a guard page below it; their base and stride.
    fn stacks(&mut self, n: usize) -> (*mut u8, usize) {
        let stride = STACK_BYTES + self.page;
        if self.stacks.as_ref().is_none_or(|s| s.len() < n * stride) {
            // The old ones first: both at once could be more than the system has.
            self.stacks = None;
            self.stacks = Some(crate::os::Stacks::new(n.max(64), stride).expect("no memory for the CPU backend's thread stacks"));
        }
        (self.stacks.as_ref().expect("made above").base(), stride)
    }
}

/// What a queue's work goes to (`metal::CommandQueue`): a thread that runs the work its queues flush, in order.
#[derive(Clone)]
pub struct CpuQueueRoot(Arc<Mutex<mpsc::Sender<Batch>>>);

impl CpuQueueRoot {
    pub fn new(_device: CpuDevice) -> CpuQueueRoot {
        let (tx, rx) = mpsc::channel::<Batch>();
        std::thread::Builder::new()
            .name("rerail-cpu-queue".into())
            .stack_size(8 << 20)
            .spawn(move || {
                let mut w = Worker { stacks: None, page: crate::os::page_bytes(), sched: vec![0; SCHED_BYTES / 8] };
                for b in rx {
                    let t = std::time::Instant::now();
                    let mut r = Ok(());
                    for op in b.ops {
                        if r.is_ok() {
                            r = op(&mut w);
                        }
                    }
                    *b.done.result.lock().unwrap() = Some((r, t.elapsed().as_secs_f64()));
                    b.done.cv.notify_all();
                }
            })
            .expect("the CPU queue's thread");
        CpuQueueRoot(Arc::new(Mutex::new(tx)))
    }
}

/// A queue of operations (`MetalComputeQueue`): launches, copies and fills, run in order on its root's thread once
/// flushed. `finish` flushes and waits.
pub struct CpuComputeQueue {
    root: CpuQueueRoot,
    ops: RefCell<Vec<Op>>,
}

impl CpuComputeQueue {
    pub fn new(root: &CpuQueueRoot) -> Self {
        CpuComputeQueue { root: root.clone(), ops: Default::default() }
    }

    pub fn new_bind_context<'a>(&'a self, function: &'a CpuComputeFunction) -> CpuBindContext<'a> {
        CpuBindContext { queue: self, function, params: vec![Vec::new(); function.param_sizes.len()], shared: 0, keep: Vec::new() }
    }

    fn push(&self, op: impl FnOnce(&mut Worker) -> Result<(), String> + Send + 'static) {
        self.ops.borrow_mut().push(Box::new(op));
    }

    /// `enqueueCopyFromHost`: the bytes are taken now, and copied into `dst` at `dst_offset` in queue order.
    pub fn enqueue_copy_from_host(&self, dst: &CpuBuffer, dst_offset: u64, bytes: &[u8]) {
        assert!(dst_offset + bytes.len() as u64 <= dst.length(), "a copy past its buffer's end");
        let (dst, bytes) = (dst.clone(), bytes.to_vec());
        self.push(move |_| {
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.contents().cast::<u8>().add(dst_offset as usize), bytes.len()) };
            Ok(())
        });
    }

    /// `enqueueFillArray`: `count` copies of `value` into `dst` from element `offset` (all of it when `None`).
    pub fn enqueue_fill_array<T: crate::Pod>(&self, dst: &CpuBuffer, value: &T, offset: usize, count: Option<usize>) {
        let size = std::mem::size_of::<T>();
        let count = count.unwrap_or(dst.length() as usize / size - offset);
        let one = unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size) };
        let bytes: Vec<u8> = one.iter().copied().cycle().take(count * size).collect();
        self.enqueue_copy_from_host(dst, (offset * size) as u64, &bytes);
    }

    /// `enqueueCopyBetweenDeviceRegions`.
    pub fn enqueue_copy_between_device_regions(&self, src: &CpuBuffer, dst: &CpuBuffer, src_offset: u64, dst_offset: u64, length: u64) {
        assert!(src_offset + length <= src.length() && dst_offset + length <= dst.length(), "a copy past its buffer's end");
        let (src, dst) = (src.clone(), dst.clone());
        self.push(move |_| {
            unsafe { std::ptr::copy(src.contents().cast::<u8>().add(src_offset as usize), dst.contents().cast::<u8>().add(dst_offset as usize), length as usize) };
            Ok(())
        });
    }

    /// Holds the root's later work until the host signals `event` with `value` or more (Metal's `encodeWaitForEvent`).
    pub fn wait_for_event(&self, event: &CpuSharedEvent, value: u64) {
        let event = event.clone();
        self.push(move |_| {
            event.wait_for(value);
            Ok(())
        });
    }

    /// The work so far, handed to the root's thread; its event.
    pub fn flush(&mut self) -> CpuComputeEvent {
        let done = Arc::new(Done::default());
        let ops = std::mem::take(&mut *self.ops.borrow_mut());
        self.root.0.lock().unwrap().send(Batch { ops, done: done.clone() }).expect("the CPU queue's thread is gone");
        CpuComputeEvent { done }
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.flush().await_()
    }
}

/// The work a queue flushed (`MetalComputeEvent`).
pub struct CpuComputeEvent {
    done: Arc<Done>,
}

impl CpuComputeEvent {
    /// Waits for the work to complete; its first error, if it had one.
    pub fn await_(&self) -> Result<(), String> {
        let mut g = self.done.result.lock().unwrap();
        while g.is_none() {
            g = self.done.cv.wait(g).unwrap();
        }
        let (r, s) = g.as_ref().unwrap();
        CPU_NANOS.fetch_add((s * 1e9) as u64, Ordering::Relaxed);
        r.clone()
    }

    pub fn is_complete(&self) -> bool {
        self.done.result.lock().unwrap().is_some()
    }

    /// The seconds the work took on the root's thread (the work must have completed).
    pub fn gpu_seconds(&self) -> f64 {
        self.done.result.lock().unwrap().as_ref().map_or(0.0, |(_, s)| *s)
    }
}

/// A launch's threadgroups: given, or three u32s at a host address read when it runs (an indirect launch).
#[derive(Clone, Copy)]
enum Grid {
    Fixed([u32; 3]),
    Indirect(usize),
}

/// The bind context of one launch: each parameter's bytes, the threadgroup memory its rows take, and the buffers it
/// binds (kept until it has run).
pub struct CpuBindContext<'a> {
    queue: &'a CpuComputeQueue,
    function: &'a CpuComputeFunction,
    params: Vec<Vec<u64>>,
    shared: u64,
    keep: Vec<CpuBuffer>,
}

fn words(bytes: &[u8]) -> Vec<u64> {
    let mut w = vec![0u64; bytes.len().div_ceil(8).max(1)];
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), w.as_mut_ptr().cast::<u8>(), bytes.len()) };
    w
}

impl CpuBindContext<'_> {
    pub fn set_primitive(&mut self, arg_num: u64, bytes: &[u8]) {
        let want = self.function.param_sizes[arg_num as usize];
        assert_eq!(bytes.len(), want, "{}: argument {arg_num} is {want} bytes, given {}", self.function.name, bytes.len());
        self.params[arg_num as usize] = words(bytes);
    }

    pub fn set_buffer(&mut self, arg_num: u64, buffer: &CpuBuffer) {
        self.keep.push(buffer.clone());
        self.params[arg_num as usize] = vec![buffer.device_address()];
    }

    pub fn set_buffer_at(&mut self, arg_num: u64, buffer: &CpuBuffer, offset: u64) -> Result<(), String> {
        self.keep.push(buffer.clone());
        self.params[arg_num as usize] = vec![buffer.device_address() + offset];
        Ok(())
    }

    pub fn set_thread_group_memory(&mut self, arg_num: u64, n_bytes: u64) {
        let at = self.shared.next_multiple_of(16);
        self.shared = at + n_bytes;
        self.params[arg_num as usize] = vec![at];
    }

    pub fn launch(&mut self, grid: &[u64], block: &[u64]) -> Result<(), String> {
        let d = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(1) as u32;
        self.launch_of(Grid::Fixed([d(grid, 0), d(grid, 1), d(grid, 2)]), block)
    }

    pub fn use_buffer(&mut self, buffer: &CpuBuffer) {
        self.keep.push(buffer.clone());
    }

    /// The counts are read when the launch runs, after the work before it (the root runs a queue's work in order).
    pub fn launch_indirect(&mut self, groups: &CpuBuffer, offset: u64, _max_groups: &[u64], block: &[u64]) -> Result<(), String> {
        self.keep.push(groups.clone());
        self.launch_of(Grid::Indirect(groups.contents() as usize + offset as usize), block)
    }

    fn launch_of(&mut self, grid: Grid, block: &[u64]) -> Result<(), String> {
        let f = self.function;
        if let Some(i) = self.params.iter().position(|p| p.is_empty()) {
            let name = f.arguments.iter().find(|a| a.compute_function_arg_index == i as u64).map_or("?", |a| a.name.as_str());
            return Err(format!("{}: argument {name} was not bound", f.name));
        }
        let d = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(1) as u32;
        let block = [d(block, 0), d(block, 1), d(block, 2)];
        if matches!(grid, Grid::Fixed(g) if g.contains(&0)) || block.contains(&0) {
            return Ok(());
        }
        let threads = block.iter().map(|&b| b as usize).product::<usize>();
        let (entry, name, shared) = (f.entry, f.name.clone(), self.shared);
        let params = std::mem::take(&mut self.params);
        self.params = vec![Vec::new(); params.len()];
        let keep = std::mem::take(&mut self.keep);
        self.queue.push(move |w| {
            let _keep = keep;
            let grid = match grid {
                Grid::Fixed(g) => g,
                // SAFETY: three u32s of a buffer this launch keeps, written (if at all) by the work before it.
                Grid::Indirect(at) => unsafe { std::ptr::read_volatile(at as *const [u32; 3]) },
            };
            if grid.contains(&0) {
                return Ok(());
            }
            let groups = grid.iter().map(|&g| g as usize).product::<usize>();
            let resident = resident_groups(threads).min(groups);
            let slot = (shared as usize).div_ceil(16) * 16;
            let mut local = vec![0u128; (slot * resident).div_ceil(16).max(1)];
            // A stack for each thread of each resident group, and, past one group, one for each group's own coroutine.
            let (stacks, stride) = w.stacks(threads * resident + if resident > 1 { resident } else { 0 });
            let mut error = vec![0 as c_char; 1024];
            let ptrs: Vec<*const c_void> = params.iter().map(|p| p.as_ptr().cast::<c_void>()).collect();
            let launch = Launch {
                grid,
                block,
                shared: local.as_mut_ptr().cast(),
                shared_bytes: slot as u64,
                stacks,
                stack_bytes: stride as u64,
                error: error.as_mut_ptr(),
                error_bytes: error.len() as u64,
                sched: w.sched.as_mut_ptr().cast(),
                sched_bytes: SCHED_BYTES as u64,
                tls_key: tls_key(),
                resident: resident as u32,
                pad: 0,
            };
            if entry(ptrs.as_ptr(), &launch) != 0 {
                let e = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy().into_owned();
                return Err(format!("{name} on the CPU: {e}"));
            }
            Ok(())
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kernel of `text` compiled for the CPU, apart from the process's device (these tests run whatever
    /// `RERAIL_GPU` is).
    fn compile(name: &str, text: &str) -> Option<(CpuDevice, CpuComputeFunctionLibrary)> {
        let Some(device) = CpuDevice::system_default() else {
            eprintln!("no C++ compiler: skipped");
            return None;
        };
        let path = format!("/<tests>/{name}");
        let source = rerail_ukl::Preprocess::new(rerail_ukl::Backend::Cpu).file(&path, text).run(&path).unwrap_or_else(|e| panic!("{e}"));
        Some((device, CpuComputeFunctionLibrary::compile_from_source(&device, &source, name).unwrap_or_else(|e| panic!("{e}"))))
    }

    const KERNELS: &str = r#"
#include <ukl/backend.h>
#include <ukl/simd.h>

struct Args {
    uint32_t n;
};
UKL_REFLECT_STRUCT(Args, n)

// Each thread's value through threadgroup memory and a barrier, then summed across its SIMD group by shuffles.
DEFINE_KERNEL2(t, t_group,
               ARGS,     Args,     args, ,
               RWBUFFER, uint32_t, out,  "[n]",
               RWLOCAL,  uint32_t, tmp,  "[64]",
               LID0,     uint32_t, lid,  ,
               LSZ0,     uint32_t, lsz,  ,
               TGID0,    uint32_t, g,    ,
               LANE,     uint32_t, lane, )
{
    tmp[lid] = g * 1000u + lid;
    ukl_tg_barrier();
    uint32_t v = tmp[lsz - 1u - lid];
    for (uint32_t d = 16u; d > 0u; d >>= 1u)
        v += ukl_sg_shuffle_xor(v, d);
    out[g * lsz + lid] = v;
}

// Lane 0 ends straight after a reduction: the CPU runs a group's lanes one after another, so the others read the
// reduction's values after lane 0 is done, and it is in the reduction still.
DEFINE_KERNEL2(t, t_ends_after,
               RWBUFFER, uint32_t, out, "[32]",
               LID0,     uint32_t, lid, )
{
    uint32_t m = ukl_sg_min(lid == 0u ? 7u : 100u + lid);
    if (lid == 0u)
        return;
    out[lid] = m;
}

// A SIMD group split: half its lanes at a threadgroup barrier, half at a shuffle that needs them all.
DEFINE_KERNEL2(t, t_divergent,
               RWBUFFER, uint32_t, out, "[1]",
               LID0,     uint32_t, lid, )
{
    if (lid < 16u)
        ukl_tg_barrier();
    else
        out[0] = ukl_sg_shuffle_xor(lid, 1u);
}
"#;

    fn run(device: &CpuDevice, f: &CpuComputeFunction, out: &CpuBuffer, args: Option<u32>, grid: u64, block: u64) -> Result<(), String> {
        let root = CpuQueueRoot::new(*device);
        let q = CpuComputeQueue::new(&root);
        let mut ctx = q.new_bind_context(f);
        for a in f.get_argument_info() {
            match a.name.as_str() {
                "args" => ctx.set_primitive(a.compute_function_arg_index, &args.unwrap().to_ne_bytes()),
                "out" => ctx.set_buffer(a.compute_function_arg_index, out),
                "tmp" => ctx.set_thread_group_memory(a.compute_function_arg_index, block * 4),
                n => panic!("{n}"),
            }
        }
        ctx.launch(&[grid], &[block])?;
        q.finish()
    }

    /// Threadgroup memory, a barrier and SIMD-group shuffles give each thread what a GPU gives it.
    #[test]
    fn a_threadgroup_runs_as_on_a_gpu() {
        let Some((device, lib)) = compile("t.uglykl", KERNELS) else { return };
        let f = lib.get_function(&device, "t_group").unwrap();
        let (grid, block) = (3u32, 64u32);
        let out = alloc_uninitialized_array_sync::<u32>(&device, (grid * block) as usize);
        run(&device, &f, &out, Some(grid * block), grid as u64, block as u64).unwrap();
        let got = transfer_to_host_sync::<u32>(&out, (grid * block) as usize);
        for g in 0..grid {
            for lid in 0..block {
                // The SIMD group's 32 values, each read from the mirrored thread.
                let first = lid / 32 * 32;
                let want: u32 = (first..first + 32).map(|l| g * 1000 + block - 1 - l).sum();
                assert_eq!(got[(g * block + lid) as usize], want, "group {g}, thread {lid}");
            }
        }
    }

    /// A lane that ends after a SIMD-group reduction is in it for the lanes that read it after: each lane's minimum
    /// is lane 0's 7, not the least of the lanes from it on.
    #[test]
    fn a_lane_done_after_a_reduction_is_in_it() {
        let Some((device, lib)) = compile("t.uglykl", KERNELS) else { return };
        let f = lib.get_function(&device, "t_ends_after").unwrap();
        let out = alloc_uninitialized_array_sync::<u32>(&device, 32);
        run(&device, &f, &out, None, 1, 32).unwrap();
        let got = transfer_to_host_sync::<u32>(&out, 32);
        assert_eq!(&got[1..], &[7u32; 31][..]);
    }

    /// A barrier no thread can pass is an error that says who waits where, not a hang.
    #[test]
    fn a_barrier_no_thread_can_pass_is_an_error() {
        let Some((device, lib)) = compile("t.uglykl", KERNELS) else { return };
        let f = lib.get_function(&device, "t_divergent").unwrap();
        let out = alloc_uninitialized_array_sync::<u32>(&device, 1);
        let e = run(&device, &f, &out, None, 1, 64).unwrap_err();
        assert!(e.contains("no barrier can be passed"), "{e}");
        assert!(e.contains("threads 0 to 15 at a threadgroup barrier;") && e.contains("threads 16 to 31 at a SIMD-group operation;"), "{e}");
        assert!(!e.contains("thread 32 ") && !e.contains("threads 32 "), "the second SIMD group's shuffle passed, and it returned: {e}");
    }
}
