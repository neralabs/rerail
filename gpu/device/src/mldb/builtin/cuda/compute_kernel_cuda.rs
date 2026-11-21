//! The CUDA runtime of the grid kernels, beside compute_kernel_metal.rs and with its interface (Rerail, decision
//! 0440 steps 3 and 4): a library compiled at run time from UKL source by NVRTC, its functions with their arguments,
//! a queue of launches, copies and fills on a stream, its events, and buffers.
//!
//! Copyright (c) 2026 Jeremy Barnes. This file is part of MLDB's forest as carried in Rerail. Apache-2.0 (see the
//! LICENSE at `forest/src/mldb/`).
//!
//! - **No CUDA at build time.** The driver (`libcuda`, Windows' `nvcuda.dll`) and NVRTC are opened at run time; without them there is no
//!   device (`CudaDevice::system_default` is `None`), as on a Mac without Metal.
//! - **Buffers are pinned, mapped host memory** (`cuMemHostAlloc`): the host reads and writes them in place
//!   (`contents`), as Metal's shared storage, and kernels read them where they are. On a unified device (GB10) that
//!   is as fast as device memory (0440 §2: 0.97x); on a discrete card it reads over the bus, which is correct and
//!   slow: placing blocks in device memory there is the block directory's work (0440 §7.2, step 10).
//! - **Arguments without reflection.** UKL's CUDA shim emits beside each kernel the text of its rows
//!   (`Name__ukl_params`) and each row's type's size, alignment and kind (`Name__ukl_rows`), and for each `args`
//!   structure its fields' names and a kernel that measures its layout (`Type__ukl_layout`). They give the same
//!   `GridComputeFunctionArgument`s Metal's reflection gives, so the grid binds a kernel the same way on both.
//! - **Threadgroup memory** (`RWLOCAL` rows) is the launch's dynamic shared memory: each row takes its byte offset
//!   into it, as an argument, in the order the grid sets them, each aligned to 16.
//! - **The context** is the device's primary context, made current on each thread that uses it.

use crate::compute_kernel::*;
use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

type CUresult = i32;
type Ptr = *mut c_void;
type DevPtr = u64;

/// The entry points used, from `libcuda` and `libnvrtc`.
struct Api {
    cu_init: unsafe extern "C" fn(u32) -> CUresult,
    cu_device_get: unsafe extern "C" fn(*mut i32, i32) -> CUresult,
    cu_device_get_attribute: unsafe extern "C" fn(*mut i32, i32, i32) -> CUresult,
    cu_device_primary_ctx_retain: unsafe extern "C" fn(*mut Ptr, i32) -> CUresult,
    cu_ctx_set_current: unsafe extern "C" fn(Ptr) -> CUresult,
    cu_mem_host_alloc: unsafe extern "C" fn(*mut Ptr, usize, u32) -> CUresult,
    cu_mem_host_get_device_pointer: unsafe extern "C" fn(*mut DevPtr, Ptr, u32) -> CUresult,
    cu_mem_host_register: unsafe extern "C" fn(Ptr, usize, u32) -> CUresult,
    cu_mem_host_unregister: unsafe extern "C" fn(Ptr) -> CUresult,
    cu_module_load_data: unsafe extern "C" fn(*mut Ptr, *const c_void) -> CUresult,
    cu_module_get_function: unsafe extern "C" fn(*mut Ptr, Ptr, *const c_char) -> CUresult,
    cu_module_get_global: unsafe extern "C" fn(*mut DevPtr, *mut usize, Ptr, *const c_char) -> CUresult,
    cu_memcpy_dtoh: unsafe extern "C" fn(*mut c_void, DevPtr, usize) -> CUresult,
    cu_memcpy_async: unsafe extern "C" fn(DevPtr, DevPtr, usize, Ptr) -> CUresult,
    cu_memset_d8_async: unsafe extern "C" fn(DevPtr, u8, usize, Ptr) -> CUresult,
    cu_launch_kernel: unsafe extern "C" fn(Ptr, u32, u32, u32, u32, u32, u32, u32, Ptr, *mut Ptr, *mut Ptr) -> CUresult,
    cu_stream_create: unsafe extern "C" fn(*mut Ptr, u32) -> CUresult,
    cu_stream_synchronize: unsafe extern "C" fn(Ptr) -> CUresult,
    cu_stream_wait_value64: unsafe extern "C" fn(Ptr, DevPtr, u64, u32) -> CUresult,
    cu_event_create: unsafe extern "C" fn(*mut Ptr, u32) -> CUresult,
    cu_event_record: unsafe extern "C" fn(Ptr, Ptr) -> CUresult,
    cu_event_synchronize: unsafe extern "C" fn(Ptr) -> CUresult,
    cu_event_query: unsafe extern "C" fn(Ptr) -> CUresult,
    cu_event_elapsed_time: unsafe extern "C" fn(*mut f32, Ptr, Ptr) -> CUresult,
    cu_event_destroy: unsafe extern "C" fn(Ptr) -> CUresult,
    cu_func_set_attribute: unsafe extern "C" fn(Ptr, i32, i32) -> CUresult,
    cu_func_get_attribute: unsafe extern "C" fn(*mut i32, i32, Ptr) -> CUresult,
    cu_occupancy_max_active_blocks_per_multiprocessor: unsafe extern "C" fn(*mut i32, Ptr, i32, usize) -> CUresult,
    cu_get_error_string: unsafe extern "C" fn(CUresult, *mut *const c_char) -> CUresult,
    nvrtc_create_program: unsafe extern "C" fn(*mut Ptr, *const c_char, *const c_char, i32, *const *const c_char, *const *const c_char) -> i32,
    nvrtc_compile_program: unsafe extern "C" fn(Ptr, i32, *const *const c_char) -> i32,
    nvrtc_get_program_log_size: unsafe extern "C" fn(Ptr, *mut usize) -> i32,
    nvrtc_get_program_log: unsafe extern "C" fn(Ptr, *mut c_char) -> i32,
    nvrtc_get_cubin_size: unsafe extern "C" fn(Ptr, *mut usize) -> i32,
    nvrtc_get_cubin: unsafe extern "C" fn(Ptr, *mut c_char) -> i32,
    nvrtc_destroy_program: unsafe extern "C" fn(*mut Ptr) -> i32,
    nvrtc_version: unsafe extern "C" fn(*mut i32, *mut i32) -> i32,
    cu_driver_get_version: unsafe extern "C" fn(*mut i32) -> CUresult,
}

unsafe impl Send for Api {}
unsafe impl Sync for Api {}

// Driver API constants (cuda.h).
const CU_MEMHOSTALLOC_PORTABLE: u32 = 0x01;
const CU_MEMHOSTALLOC_DEVICEMAP: u32 = 0x02;
const CU_STREAM_NON_BLOCKING: u32 = 0x1;
const CU_STREAM_WAIT_VALUE_GEQ: u32 = 0x0;
const CU_ERROR_NOT_READY: CUresult = 600;
const CU_DEVICE_ATTRIBUTE_MULTIPROCESSOR_COUNT: i32 = 16;
const CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR: i32 = 75;
const CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR: i32 = 76;
const CU_DEVICE_ATTRIBUTE_MAX_SHARED_MEMORY_PER_BLOCK_OPTIN: i32 = 97;
const CU_DEVICE_ATTRIBUTE_PAGEABLE_MEMORY_ACCESS_USES_HOST_PAGE_TABLES: i32 = 100;
const CU_DEVICE_ATTRIBUTE_CAN_USE_HOST_POINTER_FOR_REGISTERED_MEM: i32 = 91;
const CUDA_ERROR_HOST_MEMORY_ALREADY_REGISTERED: CUresult = 712;
const CU_MEMHOSTREGISTER_PORTABLE: u32 = 0x01;
const CU_MEMHOSTREGISTER_DEVICEMAP: u32 = 0x02;
const CU_FUNC_ATTRIBUTE_SHARED_SIZE_BYTES: i32 = 1;
const CU_FUNC_ATTRIBUTE_MAX_DYNAMIC_SHARED_SIZE_BYTES: i32 = 8;

/// The driver's library, and NVRTC's in the system's search and in the toolkit's (`CUDA_HOME`, Windows' `CUDA_PATH`).
fn library_names() -> (&'static [&'static str], Vec<String>) {
    if cfg!(windows) {
        let nvrtc = ["nvrtc64_130_0.dll", "nvrtc64_120_0.dll", "nvrtc64_112_0.dll"];
        let mut names: Vec<String> = nvrtc.iter().map(|s| s.to_string()).collect();
        if let Ok(home) = std::env::var("CUDA_PATH") {
            names.extend(nvrtc.iter().map(|s| format!("{home}\\bin\\{s}")));
        }
        (&["nvcuda.dll"], names)
    } else {
        let home = std::env::var("CUDA_HOME").unwrap_or_else(|_| "/usr/local/cuda".into());
        let nvrtc = ["libnvrtc.so", "libnvrtc.so.13", "libnvrtc.so.12"];
        let mut names: Vec<String> = nvrtc.iter().map(|s| s.to_string()).collect();
        names.extend(nvrtc.iter().map(|s| format!("{home}/lib64/{s}")));
        (&["libcuda.so.1", "libcuda.so"], names)
    }
}

/// A symbol `dlsym` found, as the function pointer type `F` its field declares.
fn sym_as<F>(p: *mut c_void) -> F {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    // SAFETY: `F` is the symbol's C function pointer type, the size of a pointer (checked above).
    unsafe { std::mem::transmute_copy(&p) }
}

/// The driver and NVRTC, opened once; `Err` says what is missing.
fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| {
        // The hardware queues the driver maps streams onto, round-robin in the order they are made: enough that no
        // two of the process's streams (a root's each, `CudaQueueRoot`, and any a caller makes) share one. Read by the driver when it loads; a value the
        // environment sets is kept.
        if std::env::var_os("CUDA_DEVICE_MAX_CONNECTIONS").is_none() {
            // SAFETY: once, before the driver loads, as the process's first use of the device.
            unsafe { std::env::set_var("CUDA_DEVICE_MAX_CONNECTIONS", MAX_CONNECTIONS.to_string()) };
        }
        use crate::os::{Lib, Scope};
        let (driver, nvrtc_names) = library_names();
        let cuda = Lib::open(driver, Scope::Global).ok_or_else(|| format!("no CUDA driver ({})", driver.join(", ")))?;
        let nvrtc = Lib::open(&nvrtc_names, Scope::Global).ok_or_else(|| format!("no NVRTC ({})", nvrtc_names.join(", ")))?;
        macro_rules! sym {
            ($lib:expr, $($name:literal)|+) => {{
                let mut p: *mut c_void = std::ptr::null_mut();
                $( if p.is_null() { p = $lib.sym($name); } )+
                if p.is_null() {
                    return Err(format!("{} not found", [$($name),+].join(" or ")));
                }
                sym_as(p)
            }};
        }
        Ok(Api {
            cu_init: sym!(cuda, "cuInit"),
            cu_device_get: sym!(cuda, "cuDeviceGet"),
            cu_device_get_attribute: sym!(cuda, "cuDeviceGetAttribute"),
            cu_device_primary_ctx_retain: sym!(cuda, "cuDevicePrimaryCtxRetain"),
            cu_ctx_set_current: sym!(cuda, "cuCtxSetCurrent"),
            cu_mem_host_alloc: sym!(cuda, "cuMemHostAlloc"),
            cu_mem_host_get_device_pointer: sym!(cuda, "cuMemHostGetDevicePointer_v2"),
            cu_mem_host_register: sym!(cuda, "cuMemHostRegister_v2" | "cuMemHostRegister"),
            cu_mem_host_unregister: sym!(cuda, "cuMemHostUnregister"),
            cu_module_load_data: sym!(cuda, "cuModuleLoadData"),
            cu_module_get_function: sym!(cuda, "cuModuleGetFunction"),
            cu_module_get_global: sym!(cuda, "cuModuleGetGlobal_v2"),
            cu_memcpy_dtoh: sym!(cuda, "cuMemcpyDtoH_v2"),
            cu_memcpy_async: sym!(cuda, "cuMemcpyAsync"),
            cu_memset_d8_async: sym!(cuda, "cuMemsetD8Async"),
            cu_launch_kernel: sym!(cuda, "cuLaunchKernel"),
            cu_stream_create: sym!(cuda, "cuStreamCreate"),
            cu_stream_synchronize: sym!(cuda, "cuStreamSynchronize"),
            cu_stream_wait_value64: sym!(cuda, "cuStreamWaitValue64_v2" | "cuStreamWaitValue64"),
            cu_event_create: sym!(cuda, "cuEventCreate"),
            cu_event_record: sym!(cuda, "cuEventRecord"),
            cu_event_synchronize: sym!(cuda, "cuEventSynchronize"),
            cu_event_query: sym!(cuda, "cuEventQuery"),
            cu_event_elapsed_time: sym!(cuda, "cuEventElapsedTime_v2" | "cuEventElapsedTime"),
            cu_event_destroy: sym!(cuda, "cuEventDestroy_v2"),
            cu_func_set_attribute: sym!(cuda, "cuFuncSetAttribute"),
            cu_func_get_attribute: sym!(cuda, "cuFuncGetAttribute"),
            cu_occupancy_max_active_blocks_per_multiprocessor: sym!(cuda, "cuOccupancyMaxActiveBlocksPerMultiprocessor"),
            cu_get_error_string: sym!(cuda, "cuGetErrorString"),
            nvrtc_create_program: sym!(nvrtc, "nvrtcCreateProgram"),
            nvrtc_compile_program: sym!(nvrtc, "nvrtcCompileProgram"),
            nvrtc_get_program_log_size: sym!(nvrtc, "nvrtcGetProgramLogSize"),
            nvrtc_get_program_log: sym!(nvrtc, "nvrtcGetProgramLog"),
            nvrtc_get_cubin_size: sym!(nvrtc, "nvrtcGetCUBINSize"),
            nvrtc_get_cubin: sym!(nvrtc, "nvrtcGetCUBIN"),
            nvrtc_destroy_program: sym!(nvrtc, "nvrtcDestroyProgram"),
            nvrtc_version: sym!(nvrtc, "nvrtcVersion"),
            cu_driver_get_version: sym!(cuda, "cuDriverGetVersion"),
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// A driver call's result: `Ok`, or an error naming the call and the driver's message.
fn ck(r: CUresult, what: &str) -> Result<(), String> {
    if r == 0 {
        return Ok(());
    }
    let mut s: *const c_char = std::ptr::null();
    let msg = match api() {
        Ok(a) if unsafe { (a.cu_get_error_string)(r, &mut s) } == 0 && !s.is_null() => unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned(),
        _ => String::new(),
    };
    Err(format!("{what}: CUDA error {r} {msg}"))
}

/// A driver call that cannot fail in a correct program (a release, a record on a live stream).
fn must(r: CUresult, what: &str) {
    if let Err(e) = ck(r, what) {
        panic!("{e}");
    }
}

/// The process's device: the first CUDA device, its primary context, and what the launches need of it.
#[derive(Clone, Copy, Debug)]
pub struct CudaDevice {
    ctx: Ptr,
    /// The compute capability as `major * 10 + minor` (121 for GB10's 12.1).
    pub sm: u32,
    /// The shared memory a block may opt in to (static and dynamic together).
    pub max_shared_per_block: u32,
    /// Whether the device reads the host's pageable memory through the host's page tables (a GB10's ATS): then
    /// memory wrapped in place (`wrap_host_memory`) is read at the host's address, nothing registered.
    pub pageable: bool,
    /// Whether registered host memory is the device's at the host's address (`CAN_USE_HOST_POINTER_FOR_REGISTERED_MEM`):
    /// then a wrap may span several registrations (`Registered`).
    host_pointer: bool,
    /// Its streaming multiprocessors: a persistent grid's resident blocks are these times a block's occupancy
    /// (`CudaComputeFunction::resident_groups`).
    pub multiprocessors: u32,
}

unsafe impl Send for CudaDevice {}
unsafe impl Sync for CudaDevice {}

/// The device a reference to it stands for (as `metal::DeviceRef` for `metal::Device`).
pub type CudaDeviceRef = CudaDevice;

thread_local! {
    static CURRENT: std::cell::Cell<Ptr> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

impl CudaDevice {
    /// The first device, or `None` without a driver, NVRTC or a device.
    pub fn system_default() -> Option<CudaDevice> {
        match Self::open() {
            Ok(d) => Some(d),
            Err(e) => {
                if std::env::var_os("RERAIL_CUDA_DEBUG").is_some() {
                    eprintln!("no CUDA device: {e}");
                }
                None
            }
        }
    }

    fn open() -> Result<CudaDevice, String> {
        let a = api()?;
        unsafe {
            ck((a.cu_init)(0), "cuInit")?;
            let mut dev = 0;
            ck((a.cu_device_get)(&mut dev, 0), "cuDeviceGet")?;
            let attr = |n: i32| -> Result<i32, String> {
                let mut v = 0;
                ck((a.cu_device_get_attribute)(&mut v, n, dev), "cuDeviceGetAttribute")?;
                Ok(v)
            };
            let sm = (attr(CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR)? * 10 + attr(CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR)?) as u32;
            let max_shared_per_block = attr(CU_DEVICE_ATTRIBUTE_MAX_SHARED_MEMORY_PER_BLOCK_OPTIN)? as u32;
            let pageable = attr(CU_DEVICE_ATTRIBUTE_PAGEABLE_MEMORY_ACCESS_USES_HOST_PAGE_TABLES).unwrap_or(0) != 0;
            let host_pointer = attr(CU_DEVICE_ATTRIBUTE_CAN_USE_HOST_POINTER_FOR_REGISTERED_MEM).unwrap_or(0) != 0;
            let multiprocessors = attr(CU_DEVICE_ATTRIBUTE_MULTIPROCESSOR_COUNT)?.max(1) as u32;
            let mut ctx = std::ptr::null_mut();
            ck((a.cu_device_primary_ctx_retain)(&mut ctx, dev), "cuDevicePrimaryCtxRetain")?;
            Ok(CudaDevice { ctx, sm, max_shared_per_block, pageable, host_pointer, multiprocessors })
        }
    }

    /// Makes the context current on this thread (once a thread).
    fn current(&self) -> &'static Api {
        let a = api().expect("a CUDA device without its API");
        CURRENT.with(|c| {
            if c.get() != self.ctx {
                must(unsafe { (a.cu_ctx_set_current)(self.ctx) }, "cuCtxSetCurrent");
                c.set(self.ctx);
            }
        });
        a
    }

    /// An event the host signals and a queue waits on (`metal::SharedEvent`): a word of pinned memory.
    pub fn new_shared_event(&self) -> CudaSharedEvent {
        let b = alloc_uninitialized_array_sync::<u64>(self, 1);
        unsafe { b.contents().cast::<u64>().write_volatile(0) };
        CudaSharedEvent { word: b }
    }
}

/// The device memory of a buffer: pinned host memory, mapped for the device.
struct Allocation {
    host: *mut u8,
    dev: DevPtr,
    /// The bytes asked for (at least 16, as on Metal), what `length` says.
    len: u64,
    /// The block's size class: the bytes it holds. 0: memory wrapped in place (`wrap_host_memory`), its owner's.
    class: u64,
    /// Wrapped memory's registration with the driver: none (read where it lies), the wrap's own (`cuMemHostRegister`
    /// of exactly its bytes), or its share of the page runs registered (`Registered`); released when the buffer goes.
    registered: Registration,
}

enum Registration {
    None,
    Own,
    Runs(Vec<u64>),
}

unsafe impl Send for Allocation {}
unsafe impl Sync for Allocation {}

impl Drop for Allocation {
    /// Back to its size class's free list, not to the driver: `cuMemFreeHost` waits for the whole device, and a
    /// stream held on a host-signalled event (an armed prediction) would then wait for the very host thread that is
    /// freeing. The pool keeps a process's peak of each class (0440 E1: the pool's slabs are `cuMemHostAlloc`'s).
    fn drop(&mut self) {
        if self.class == 0 {
            match std::mem::replace(&mut self.registered, Registration::None) {
                Registration::None => {}
                Registration::Own => {
                    if let Ok(a) = api() {
                        unsafe { (a.cu_mem_host_unregister)(self.host.cast()) };
                    }
                }
                Registration::Runs(runs) => release_runs(&runs),
            }
            return;
        }
        POOL.lock().unwrap().entry(self.class).or_default().push(Block { host: self.host, dev: self.dev });
    }
}

/// A pinned block of a size class, free.
struct Block {
    host: *mut u8,
    dev: DevPtr,
}

unsafe impl Send for Block {}

/// The free pinned blocks, by size (a power of two, at least 256 bytes).
static POOL: std::sync::Mutex<std::collections::BTreeMap<u64, Vec<Block>>> = std::sync::Mutex::new(std::collections::BTreeMap::new());

/// A buffer (`metal::Buffer`): shared by its clones, freed when the last goes (a queue keeps those it uses until
/// its work is awaited).
#[derive(Clone)]
pub struct CudaBuffer(Arc<Allocation>);

/// A reference to a buffer (`metal::BufferRef`); the same type.
pub type CudaBufferRef = CudaBuffer;

impl std::fmt::Debug for CudaBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CudaBuffer({:#x}, {} bytes)", self.0.dev, self.0.len)
    }
}

impl CudaBuffer {
    /// The host's pointer to the buffer's bytes.
    pub fn contents(&self) -> *mut c_void {
        self.0.host.cast()
    }

    pub fn length(&self) -> u64 {
        self.0.len
    }

    /// The device's address of the buffer's bytes.
    pub fn device_address(&self) -> u64 {
        self.0.dev
    }

    pub fn to_owned(&self) -> CudaBuffer {
        self.clone()
    }
}

/// A host-signalled event (`metal::SharedEvent`): a word of pinned memory a stream waits on (`cuStreamWaitValue64`)
/// until the host stores a value at least the one waited for. Its clones are the same event.
#[derive(Clone)]
pub struct CudaSharedEvent {
    word: CudaBuffer,
}

impl CudaSharedEvent {
    pub fn set_signaled_value(&self, v: u64) {
        std::sync::atomic::fence(Ordering::SeqCst);
        unsafe { self.word.contents().cast::<u64>().write_volatile(v) };
        std::sync::atomic::fence(Ordering::SeqCst);
    }

    pub fn signaled_value(&self) -> u64 {
        unsafe { self.word.contents().cast::<u64>().read_volatile() }
    }
}

/// `allocUninitializedArraySync`: memory for `length` elements (at least 16 bytes, as on Metal).
/// The block is of the bytes' size class (a power of two, at least 256), from the pool when one is free; it is
/// uninitialized (a reused block holds what it last held).
pub fn alloc_uninitialized_array_sync<T>(device: &CudaDevice, length: usize) -> CudaBuffer {
    let len = (length * std::mem::size_of::<T>()).max(16) as u64;
    let class = len.max(256).next_power_of_two();
    sweep_pending(device.current());
    let reused = POOL.lock().unwrap().get_mut(&class).and_then(Vec::pop);
    let Block { host, dev } = reused.unwrap_or_else(|| {
        let a = device.current();
        let mut host = std::ptr::null_mut();
        must(unsafe { (a.cu_mem_host_alloc)(&mut host, class as usize, CU_MEMHOSTALLOC_PORTABLE | CU_MEMHOSTALLOC_DEVICEMAP) }, "cuMemHostAlloc");
        let mut dev = 0;
        must(unsafe { (a.cu_mem_host_get_device_pointer)(&mut dev, host, 0) }, "cuMemHostGetDevicePointer");
        Block { host: host.cast(), dev }
    });
    CudaBuffer(Arc::new(Allocation { host, dev, len, class, registered: Registration::None }))
}

/// A buffer over `len` bytes of the host's memory at `ptr`, read in place (decision 0353): at the host's address
/// where the device reads pageable memory (`CudaDevice::pageable`); else its pages registered with the driver and
/// mapped. The driver registers whole pages and refuses a page registered already (error 712), and wraps share
/// pages (two small ones in a page, a view inside a block wrapped whole): where registered memory is the device's at
/// the host's address, the pages are registered once, in runs a wrap shares (`Registered`); elsewhere a wrap's own
/// registration.
///
/// # Safety
/// The memory outlives the buffer and its clones, and every use of them by queued work.
pub unsafe fn wrap_host_memory(device: &CudaDevice, ptr: *mut u8, len: usize) -> Result<CudaBuffer, String> {
    let wrap = |dev: DevPtr, registered: Registration| Ok(CudaBuffer(Arc::new(Allocation { host: ptr, dev, len: len as u64, class: 0, registered })));
    if device.pageable {
        return wrap(ptr as DevPtr, Registration::None);
    }
    let a = device.current();
    if device.host_pointer {
        let runs = register_runs(a, ptr as u64, len as u64)?;
        return wrap(ptr as DevPtr, Registration::Runs(runs));
    }
    unsafe {
        ck((a.cu_mem_host_register)(ptr.cast(), len, CU_MEMHOSTREGISTER_PORTABLE | CU_MEMHOSTREGISTER_DEVICEMAP), "cuMemHostRegister")?;
        let mut dev = 0;
        if let Err(e) = ck((a.cu_mem_host_get_device_pointer)(&mut dev, ptr.cast(), 0), "cuMemHostGetDevicePointer") {
            (a.cu_mem_host_unregister)(ptr.cast());
            return Err(e);
        }
        wrap(dev, Registration::Own)
    }
}

/// A run of pages registered with the driver for wraps (`wrap_host_memory`): its end, and how many wraps hold it.
struct Run {
    end: u64,
    holders: usize,
}

/// The runs registered, by their first byte; they never overlap. Pages pinned otherwise (the pool's blocks,
/// `cuMemHostAlloc`) are in none: they are mapped already, and their owner keeps them.
static REGISTERED: std::sync::Mutex<std::collections::BTreeMap<u64, Run>> = std::sync::Mutex::new(std::collections::BTreeMap::new());

fn page_bytes() -> u64 {
    static PAGE: OnceLock<u64> = OnceLock::new();
    *PAGE.get_or_init(|| crate::os::page_bytes() as u64)
}

/// Registers the pages of `[at, at + len)` that no run holds, as runs, and holds every run in it: the runs' starts.
fn register_runs(a: &Api, at: u64, len: u64) -> Result<Vec<u64>, String> {
    let page = page_bytes();
    let (lo, hi) = (at / page * page, (at + len.max(1)).div_ceil(page) * page);
    let mut runs = REGISTERED.lock().unwrap_or_else(|p| p.into_inner());
    // The gaps between the runs in [lo, hi).
    let mut held: Vec<(u64, u64)> = runs.range(..hi).rev().take_while(|(_, r)| r.end > lo).map(|(&s, r)| (s, r.end)).collect();
    held.reverse();
    let mut gaps = Vec::new();
    let mut from = lo;
    for &(s, e) in &held {
        if s > from {
            gaps.push((from, s));
        }
        from = from.max(e);
    }
    if from < hi {
        gaps.push((from, hi));
    }
    let mut made = Vec::new();
    for (s, e) in gaps {
        if let Err(err) = register_gap(a, s, e, page, &mut made) {
            for (s, _) in made {
                unsafe { (a.cu_mem_host_unregister)(s as Ptr) };
            }
            return Err(err);
        }
    }
    for (s, e) in made {
        runs.insert(s, Run { end: e, holders: 0 });
    }
    let starts: Vec<u64> = runs.range(..hi).rev().take_while(|(_, r)| r.end > lo).map(|(&s, _)| s).collect();
    for s in &starts {
        runs.get_mut(s).expect("a run").holders += 1;
    }
    Ok(starts)
}

/// Registers `[s, e)` (pages) as a run; where some of its pages are pinned otherwise (error 712), the pages that are
/// not, each stretch a run of its own.
fn register_gap(a: &Api, s: u64, e: u64, page: u64, made: &mut Vec<(u64, u64)>) -> Result<(), String> {
    let flags = CU_MEMHOSTREGISTER_PORTABLE | CU_MEMHOSTREGISTER_DEVICEMAP;
    let r = unsafe { (a.cu_mem_host_register)(s as Ptr, (e - s) as usize, flags) };
    if r == 0 {
        made.push((s, e));
        return Ok(());
    }
    if r != CUDA_ERROR_HOST_MEMORY_ALREADY_REGISTERED {
        return ck(r, "cuMemHostRegister");
    }
    let mapped = |p: u64| {
        let mut dev = 0;
        unsafe { (a.cu_mem_host_get_device_pointer)(&mut dev, p as Ptr, 0) == 0 }
    };
    let mut p = s;
    while p < e {
        if mapped(p) {
            p += page;
            continue;
        }
        let mut q = p + page;
        while q < e && !mapped(q) {
            q += page;
        }
        ck(unsafe { (a.cu_mem_host_register)(p as Ptr, (q - p) as usize, flags) }, "cuMemHostRegister")?;
        made.push((p, q));
        p = q;
    }
    Ok(())
}

/// Lets go of a wrap's runs: a run no wrap holds is unregistered.
fn release_runs(starts: &[u64]) {
    let mut runs = REGISTERED.lock().unwrap_or_else(|p| p.into_inner());
    for s in starts {
        let Some(r) = runs.get_mut(s) else { continue };
        r.holders -= 1;
        if r.holders == 0 {
            runs.remove(s);
            if let Ok(a) = api() {
                unsafe { (a.cu_mem_host_unregister)(*s as Ptr) };
            }
        }
    }
}

/// `transferToDeviceImmutable`: memory holding `data`.
pub fn transfer_to_device_immutable<T: crate::Pod>(device: &CudaDevice, data: &[T]) -> CudaBuffer {
    let b = alloc_uninitialized_array_sync::<T>(device, data.len());
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().cast::<u8>(), b.contents().cast::<u8>(), std::mem::size_of_val(data)) };
    b
}

/// `transferToHostSync`: the first `length` elements (the work writing them must have completed).
pub fn transfer_to_host_sync<T: crate::Pod + Clone>(buffer: &CudaBuffer, length: usize) -> Vec<T> {
    assert!(length * std::mem::size_of::<T>() <= buffer.length() as usize);
    unsafe { std::slice::from_raw_parts(buffer.contents().cast::<T>(), length).to_vec() }
}

static GPU_NANOS: AtomicU64 = AtomicU64::new(0);

/// The GPU seconds of every event awaited so far in this process (as Metal's `gpu_seconds_awaited`).
pub fn gpu_seconds_awaited() -> f64 {
    GPU_NANOS.load(Ordering::Relaxed) as f64 * 1e-9
}

/// A compiled module (`MetalComputeFunctionLibrary`).
pub struct CudaComputeFunctionLibrary {
    module: Ptr,
    device: CudaDevice,
}

unsafe impl Send for CudaComputeFunctionLibrary {}
unsafe impl Sync for CudaComputeFunctionLibrary {}

impl CudaComputeFunctionLibrary {
    /// Compiles CUDA C++ (UKL preprocessed for this device) with NVRTC for the device's architecture, and loads it.
    pub fn compile_from_source(device: &CudaDevice, preprocessed_source: &str, file_name_to_appear_in_error_messages: &str) -> Result<Self, String> {
        let a = device.current();
        let options = [format!("--gpu-architecture=sm_{}", device.sm), "-std=c++20".into(), "-default-device".into()];
        // The store's cubin (`crate::store`), keyed by all the compile reads: NVRTC's and the driver's builds too.
        let (mut major, mut minor, mut driver) = (0, 0, 0);
        unsafe { (a.nvrtc_version)(&mut major, &mut minor) };
        unsafe { (a.cu_driver_get_version)(&mut driver) };
        let versions = format!("nvrtc {major}.{minor} driver {driver}");
        let key = crate::store::key(&[b"cuda", versions.as_bytes(), options.join(" ").as_bytes(), preprocessed_source.as_bytes()]);
        if let Some(cubin) = crate::store::load("cuda", &key, "cubin") {
            let mut module = std::ptr::null_mut();
            if unsafe { (a.cu_module_load_data)(&mut module, cubin.as_ptr().cast()) } == 0 {
                crate::store::count(true);
                return Ok(CudaComputeFunctionLibrary { module, device: *device });
            }
        }
        crate::store::count(false);
        let src = CString::new(preprocessed_source).map_err(|e| e.to_string())?;
        let name = CString::new(file_name_to_appear_in_error_messages).map_err(|e| e.to_string())?;
        let opts: Vec<CString> = options.into_iter().map(|o| CString::new(o).unwrap()).collect();
        let opt_ptrs: Vec<*const c_char> = opts.iter().map(|o| o.as_ptr()).collect();
        let mut prog = std::ptr::null_mut();
        unsafe {
            if (a.nvrtc_create_program)(&mut prog, src.as_ptr(), name.as_ptr(), 0, std::ptr::null(), std::ptr::null()) != 0 {
                return Err("nvrtcCreateProgram failed".into());
            }
            let r = (a.nvrtc_compile_program)(prog, opt_ptrs.len() as i32, opt_ptrs.as_ptr());
            let mut n = 0;
            (a.nvrtc_get_program_log_size)(prog, &mut n);
            let mut log = vec![0u8; n.max(1)];
            (a.nvrtc_get_program_log)(prog, log.as_mut_ptr().cast());
            if r != 0 {
                (a.nvrtc_destroy_program)(&mut prog);
                let log = String::from_utf8_lossy(&log);
                let errors: Vec<&str> = log.lines().filter(|l| l.contains("error")).take(20).collect();
                return Err(format!("NVRTC compiling {file_name_to_appear_in_error_messages} for sm_{}: {}", device.sm, errors.join("\n")));
            }
            let mut n = 0;
            (a.nvrtc_get_cubin_size)(prog, &mut n);
            let mut cubin = vec![0u8; n];
            (a.nvrtc_get_cubin)(prog, cubin.as_mut_ptr().cast());
            (a.nvrtc_destroy_program)(&mut prog);
            let mut module = std::ptr::null_mut();
            ck((a.cu_module_load_data)(&mut module, cubin.as_ptr().cast()), "cuModuleLoadData")?;
            crate::store::save("cuda", &key, "cubin", &cubin);
            Ok(CudaComputeFunctionLibrary { module, device: *device })
        }
    }

    /// A `__device__` global's bytes, by name.
    fn global(&self, name: &str) -> Result<Vec<u8>, String> {
        let a = self.device.current();
        let c = CString::new(name).unwrap();
        let (mut p, mut n) = (0, 0);
        ck(unsafe { (a.cu_module_get_global)(&mut p, &mut n, self.module, c.as_ptr()) }, &format!("cuModuleGetGlobal {name}"))?;
        let mut v = vec![0u8; n];
        ck(unsafe { (a.cu_memcpy_dtoh)(v.as_mut_ptr().cast(), p, n) }, "cuMemcpyDtoH")?;
        Ok(v)
    }

    fn function_handle(&self, name: &str) -> Result<Ptr, String> {
        let a = self.device.current();
        let c = CString::new(name).unwrap();
        let mut f = std::ptr::null_mut();
        ck(unsafe { (a.cu_module_get_function)(&mut f, self.module, c.as_ptr()) }, &format!("no kernel {name}"))?;
        Ok(f)
    }

    /// An `args` structure's layout: its reflection kernel run once (`UKL_REFLECT_STRUCT`).
    fn struct_type(&self, ty: &str) -> Result<KernelBaseType, String> {
        let names = self.global(&format!("{ty}__ukl_fields")).map_err(|e| format!("{ty} is an ARGS row's type with no UKL_REFLECT_STRUCT: {e}"))?;
        let names = CStr::from_bytes_until_nul(&names).map_err(|e| e.to_string())?.to_str().map_err(|e| e.to_string())?.to_string();
        let n = crate::rows::layout_len(&names);
        let out = alloc_uninitialized_array_sync::<u64>(&self.device, n);
        let f = self.function_handle(&format!("{ty}__ukl_layout"))?;
        let a = self.device.current();
        let mut addr = out.device_address();
        let mut params = [(&mut addr as *mut u64).cast::<c_void>()];
        unsafe {
            ck((a.cu_launch_kernel)(f, 1, 1, 1, 1, 1, 1, 0, std::ptr::null_mut(), params.as_mut_ptr(), std::ptr::null_mut()), "launching a layout kernel")?;
            ck((a.cu_stream_synchronize)(std::ptr::null_mut()), "a layout kernel")?;
        }
        let l = transfer_to_host_sync::<u64>(&out, n);
        crate::rows::struct_type(ty, &names, &l)
    }

    /// The kernel `function_name`, with its arguments from its rows (`Name__ukl_params`, `Name__ukl_rows`).
    pub fn get_function(&self, _device: &CudaDevice, function_name: &str) -> Result<CudaComputeFunction, String> {
        let func = self.function_handle(function_name)?;
        let text = self.global(&format!("{function_name}__ukl_params"))?;
        let text = CStr::from_bytes_until_nul(&text).map_err(|e| e.to_string())?.to_str().map_err(|e| e.to_string())?.to_string();
        let rows = self.global(&format!("{function_name}__ukl_rows"))?;
        let rows: Vec<u64> = rows.as_chunks::<8>().0.iter().map(|&c| u64::from_le_bytes(c)).collect();
        let (arguments, param_sizes) = crate::rows::arguments(function_name, &text, &rows, |ty| self.struct_type(ty))?;
        // The shared memory a launch may take beyond the kernel's static storage.
        let a = self.device.current();
        let mut static_smem = 0;
        ck(unsafe { (a.cu_func_get_attribute)(&mut static_smem, CU_FUNC_ATTRIBUTE_SHARED_SIZE_BYTES, func) }, "cuFuncGetAttribute")?;
        let dynamic_limit = self.device.max_shared_per_block.saturating_sub(static_smem as u32);
        ck(unsafe { (a.cu_func_set_attribute)(func, CU_FUNC_ATTRIBUTE_MAX_DYNAMIC_SHARED_SIZE_BYTES, dynamic_limit as i32) }, "cuFuncSetAttribute")?;
        Ok(CudaComputeFunction { func, name: function_name.to_string(), arguments, param_sizes, dynamic_limit })
    }
}

/// A kernel of a library, with its arguments (`MetalComputeFunction`).
pub struct CudaComputeFunction {
    func: Ptr,
    pub name: String,
    arguments: Vec<GridComputeFunctionArgument>,
    /// Each launch parameter's size, in the kernel's order.
    param_sizes: Vec<usize>,
    /// The dynamic shared memory a launch may take.
    dynamic_limit: u32,
}

unsafe impl Send for CudaComputeFunction {}
unsafe impl Sync for CudaComputeFunction {}

impl CudaComputeFunction {
    pub fn get_argument_info(&self) -> &[GridComputeFunctionArgument] {
        &self.arguments
    }

    /// The blocks of `threads` threads, each with `dynamic` bytes of dynamic shared memory, that `device` holds at
    /// once: the driver's occupancy of this function (its registers, its static and dynamic shared memory) times the
    /// device's multiprocessors. A persistent grid's groups (decision 0568), the driver's answer, not a measure.
    pub fn resident_groups(&self, device: &CudaDevice, threads: u32, dynamic: u64) -> Result<u64, String> {
        let a = device.current();
        let mut per = 0;
        // SAFETY: the function's own handle, its context current.
        ck(unsafe { (a.cu_occupancy_max_active_blocks_per_multiprocessor)(&mut per, self.func, threads as i32, dynamic as usize) }, "cuOccupancyMaxActiveBlocksPerMultiprocessor")?;
        Ok(u64::from(device.multiprocessors) * per.max(0) as u64)
    }
}

/// The hardware queues asked of the driver (its maximum).
const MAX_CONNECTIONS: usize = 32;

/// What a queue's work goes to (`metal::CommandQueue`): one stream, made once. Its queues' work runs in the order it is
/// flushed, as a Metal command queue's command buffers run in the order they are committed: the engine's batches rely
/// on it (a batch reads what the one committed before it wrote, `Batch::commit_open`). A root of several streams, each
/// queue taking the next, ran two submissions at once with no order between them (0440 step E).
///
/// The driver runs streams on a few hardware queues; streams that share one are run in order, as one. A stream held
/// by a wait for the host (`CudaComputeQueue::wait_for_event`, an armed prediction) then holds back every stream on
/// its hardware queue, and the work the host awaits before it signals may be among them: a deadlock (seen on a
/// V100). Metal has the same rule within a command queue, which is why the forest keeps its waiting work on a queue
/// of its own (`shared.waiting`). Here each root's stream has a hardware queue of its own (`MAX_CONNECTIONS`), so the
/// waiting root's holds back nothing else.
pub struct CudaQueueRoot {
    device: CudaDevice,
    stream: Ptr,
}

/// Held while a submission's operations go onto a stream (`CudaComputeQueue::flush`): one submission's are
/// contiguous, as a command buffer's are, never interleaved with another thread's.
static ISSUE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// An operation of a queue, recorded when it is encoded and issued when the queue is flushed: nothing runs before
/// its submission is committed, as on Metal, so submissions run in the order they are committed (a launch issued as
/// it was encoded ran before an earlier-committed submission of another thread, and read what it had not written yet).
enum Op {
    Launch { func: Ptr, grid: [u32; 3], block: [u32; 3], shared: u32, params: Vec<Vec<u64>>, name: String },
    Memset { dst: u64, bytes: usize },
    Memcpy { dst: u64, src: u64, bytes: usize },
    WaitValue { at: u64, value: u64 },
}

unsafe impl Send for CudaQueueRoot {}
unsafe impl Sync for CudaQueueRoot {}

impl CudaQueueRoot {
    /// A root on `device`. Made at the process's start (`shared`), the waiting root first.
    pub fn new(device: CudaDevice) -> CudaQueueRoot {
        let a = device.current();
        let mut stream = std::ptr::null_mut();
        must(unsafe { (a.cu_stream_create)(&mut stream, CU_STREAM_NON_BLOCKING) }, "cuStreamCreate");
        CudaQueueRoot { device, stream }
    }
}

/// A queue of operations (`MetalComputeQueue`): launches, copies and fills in order on one of its root's streams. `flush`
/// records an event to await; `finish` flushes and waits. A queue dropped without a flush runs nothing (a Metal command
/// buffer never committed): its operations and the buffers they would use are dropped with it.
pub struct CudaComputeQueue {
    device: CudaDevice,
    stream: Ptr,
    /// The operations encoded since the last flush, in order (`Op`).
    ops: RefCell<Vec<Op>>,
    /// The staging memory for `enqueue_copy_from_host`, and how much is used (as on Metal).
    staging: RefCell<Option<(CudaBuffer, u64)>>,
    /// The buffers the work since the last flush uses that nothing else may hold (staging): kept to the event.
    keep: RefCell<Vec<CudaBuffer>>,
}

// SAFETY: the stream is its root's (which is `Send` and `Sync`), and the functions its launches name are the device's;
// the driver is called only in `flush`, on the thread that flushes, after `CudaDevice::current` makes the device's
// context current there. The queue moves between threads (a forest's scratch behind its mutex); it is not `Sync`:
// its cells are its owner's.
unsafe impl Send for CudaComputeQueue {}

/// The bytes a staging buffer holds (a larger copy gets its own).
const STAGING_BYTES: u64 = 64 << 10;

fn new_event(a: &Api) -> Ptr {
    let mut e = std::ptr::null_mut();
    must(unsafe { (a.cu_event_create)(&mut e, 0) }, "cuEventCreate");
    e
}

impl CudaComputeQueue {
    /// A queue on `root`'s stream.
    pub fn new(root: &CudaQueueRoot) -> Self {
        CudaComputeQueue { device: root.device, stream: root.stream, ops: Default::default(), staging: Default::default(), keep: Default::default() }
    }

    /// `newBindContext`: one launch of `function`, on the queue's stream.
    pub fn new_bind_context<'a>(&'a self, function: &'a CudaComputeFunction) -> CudaBindContext<'a> {
        // The rows' parameters, then the indirect launch's counts (`GEN_OUTER_PARAMETER_GROUPS`: none, a direct launch).
        let mut params = vec![Vec::new(); function.param_sizes.len() + 1];
        params[function.param_sizes.len()] = vec![0];
        CudaBindContext { function, params, shared: 0, keep: &self.keep, ops: &self.ops }
    }

    /// `enqueueCopyFromHost`: the bytes are staged now, and copied into `dst` at `dst_offset` in queue order.
    pub fn enqueue_copy_from_host(&self, dst: &CudaBuffer, dst_offset: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let n = bytes.len() as u64;
        let (staging, at) = {
            let mut st = self.staging.borrow_mut();
            let fits = st.as_ref().is_some_and(|(b, used)| used.next_multiple_of(16) + n <= b.length());
            if !fits {
                let b = alloc_uninitialized_array_sync::<u8>(&self.device, n.max(STAGING_BYTES) as usize);
                self.keep.borrow_mut().push(b.clone());
                *st = Some((b, 0));
            }
            let (b, used) = st.as_mut().unwrap();
            let at = used.next_multiple_of(16);
            *used = at + n;
            (b.clone(), at)
        };
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), staging.contents().cast::<u8>().add(at as usize), bytes.len()) };
        self.enqueue_copy_between_device_regions(&staging, dst, at, dst_offset, n);
    }

    /// `enqueueFillArray`: `count` copies of `value` into `dst` from element `offset` (all of it when `None`).
    pub fn enqueue_fill_array<T: crate::Pod>(&self, dst: &CudaBuffer, value: &T, offset: usize, count: Option<usize>) {
        let size = std::mem::size_of::<T>();
        let count = count.unwrap_or(dst.length() as usize / size - offset);
        let one = unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size) };
        if one.iter().all(|&b| b == 0) {
            self.keep.borrow_mut().push(dst.clone());
            self.ops.borrow_mut().push(Op::Memset { dst: dst.device_address() + (offset * size) as u64, bytes: count * size });
            return;
        }
        let bytes: Vec<u8> = one.iter().copied().cycle().take(count * size).collect();
        self.enqueue_copy_from_host(dst, (offset * size) as u64, &bytes);
    }

    /// `enqueueCopyBetweenDeviceRegions`.
    pub fn enqueue_copy_between_device_regions(&self, src: &CudaBuffer, dst: &CudaBuffer, src_offset: u64, dst_offset: u64, length: u64) {
        if length == 0 {
            return;
        }
        self.keep.borrow_mut().extend([src.clone(), dst.clone()]);
        self.ops.borrow_mut().push(Op::Memcpy { dst: dst.device_address() + dst_offset, src: src.device_address() + src_offset, bytes: length as usize });
    }

    /// Holds the queue's later work until the host signals `event` with `value` or more (Metal's
    /// `encodeWaitForEvent`).
    pub fn wait_for_event(&self, event: &CudaSharedEvent, value: u64) {
        self.keep.borrow_mut().push(event.word.clone());
        self.ops.borrow_mut().push(Op::WaitValue { at: event.word.device_address(), value });
    }

    /// The work so far, issued onto the root's stream between two events, as an event to await. A launch the driver
    /// refuses fails the event (`await_`); the operations after it are not issued.
    pub fn flush(&mut self) -> CudaComputeEvent {
        let a = self.device.current();
        let (start, end) = (new_event(a), new_event(a));
        let ops = std::mem::take(&mut *self.ops.borrow_mut());
        let mut failed = None;
        {
            let _issue = ISSUE.lock().unwrap_or_else(|p| p.into_inner());
            must(unsafe { (a.cu_event_record)(start, self.stream) }, "cuEventRecord");
            for op in ops {
                if let Err(e) = issue(a, self.stream, op) {
                    failed = Some(e);
                    break;
                }
            }
            must(unsafe { (a.cu_event_record)(end, self.stream) }, "cuEventRecord");
        }
        *self.staging.borrow_mut() = None;
        CudaComputeEvent { device: self.device, start, end, keep: std::mem::take(&mut *self.keep.borrow_mut()), failed }
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.flush().await_()
    }
}

/// One operation onto `stream`.
fn issue(a: &Api, stream: Ptr, op: Op) -> Result<(), String> {
    match op {
        Op::Launch { func, grid, block, shared, mut params, name } => {
            // The driver reads each parameter's bytes during the call.
            let mut ptrs: Vec<Ptr> = params.iter_mut().map(|p| p.as_mut_ptr().cast::<c_void>()).collect();
            ck(
                unsafe { (a.cu_launch_kernel)(func, grid[0], grid[1], grid[2], block[0], block[1], block[2], shared, stream, ptrs.as_mut_ptr(), std::ptr::null_mut()) },
                &format!("launching {name}"),
            )
        }
        Op::Memset { dst, bytes } => ck(unsafe { (a.cu_memset_d8_async)(dst, 0, bytes, stream) }, "cuMemsetD8Async"),
        Op::Memcpy { dst, src, bytes } => ck(unsafe { (a.cu_memcpy_async)(dst, src, bytes, stream) }, "cuMemcpyAsync"),
        Op::WaitValue { at, value } => ck(unsafe { (a.cu_stream_wait_value64)(stream, at, value, CU_STREAM_WAIT_VALUE_GEQ) }, "cuStreamWaitValue64"),
    }
}

/// Work dropped before it completed (an event not awaited), and the buffers it uses: Metal keeps a command buffer's
/// resources until it completes; here they go back to the pool once `end` has passed.
struct Pending {
    end: Ptr,
    keep: Vec<CudaBuffer>,
}

unsafe impl Send for Pending {}

static PENDING: std::sync::Mutex<Vec<Pending>> = std::sync::Mutex::new(Vec::new());

/// Releases the pending work that has completed (its buffers to the pool).
fn sweep_pending(a: &Api) {
    let done: Vec<Pending> = {
        let mut p = PENDING.lock().unwrap();
        if p.is_empty() {
            return;
        }
        let (done, wait): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut *p).into_iter().partition(|w| unsafe { (a.cu_event_query)(w.end) } != CU_ERROR_NOT_READY);
        *p = wait;
        done
    };
    for w in done {
        unsafe { (a.cu_event_destroy)(w.end) };
        drop(w.keep);
    }
}

/// The work a queue flushed (`MetalComputeEvent`): the events at its start and end, and the memory it uses.
pub struct CudaComputeEvent {
    device: CudaDevice,
    start: Ptr,
    end: Ptr,
    keep: Vec<CudaBuffer>,
    /// The operation the driver refused when it was issued, if one was.
    failed: Option<String>,
}

unsafe impl Send for CudaComputeEvent {}
// SAFETY: its methods take `&self` and call the driver API, which is thread-safe, in the device's context made
// current on the calling thread (`current`); nothing in it is written after it is made.
unsafe impl Sync for CudaComputeEvent {}

impl CudaComputeEvent {
    /// Waits for the work to complete; an error if it did not.
    pub fn await_(&self) -> Result<(), String> {
        let a = self.device.current();
        ck(unsafe { (a.cu_event_synchronize)(self.end) }, "awaiting the queue's work")?;
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        GPU_NANOS.fetch_add((self.gpu_seconds() * 1e9) as u64, Ordering::Relaxed);
        Ok(())
    }

    /// Whether the work has completed (or failed): Metal's command buffer status, polled.
    pub fn is_complete(&self) -> bool {
        let a = self.device.current();
        unsafe { (a.cu_event_query)(self.end) != CU_ERROR_NOT_READY }
    }

    /// The seconds from the work's start to its end, on the device's clock (the work must have completed).
    pub fn gpu_seconds(&self) -> f64 {
        let a = self.device.current();
        let mut ms = 0f32;
        if unsafe { (a.cu_event_elapsed_time)(&mut ms, self.start, self.end) } != 0 {
            return 0.0;
        }
        ms as f64 * 1e-3
    }
}

impl Drop for CudaComputeEvent {
    /// Work not complete keeps its buffers until it is (`PENDING`).
    fn drop(&mut self) {
        let a = self.device.current();
        unsafe { (a.cu_event_destroy)(self.start) };
        let keep = std::mem::take(&mut self.keep);
        if !keep.is_empty() && unsafe { (a.cu_event_query)(self.end) } == CU_ERROR_NOT_READY {
            PENDING.lock().unwrap().push(Pending { end: self.end, keep });
        } else {
            unsafe { (a.cu_event_destroy)(self.end) };
        }
    }
}

/// The bind context of one launch (`MetalBindContext`): each parameter's bytes, and the dynamic shared memory the
/// threadgroup rows take.
pub struct CudaBindContext<'a> {
    function: &'a CudaComputeFunction,
    /// Each parameter's bytes, in 8-byte words (the driver reads each at its own size).
    params: Vec<Vec<u64>>,
    shared: u64,
    /// The queue's buffers in use: those bound here are kept until the launch completes.
    keep: &'a RefCell<Vec<CudaBuffer>>,
    /// The queue's operations, which the launch joins.
    ops: &'a RefCell<Vec<Op>>,
}

fn words(bytes: &[u8]) -> Vec<u64> {
    let mut w = vec![0u64; bytes.len().div_ceil(8).max(1)];
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), w.as_mut_ptr().cast::<u8>(), bytes.len()) };
    w
}

impl CudaBindContext<'_> {
    pub fn set_primitive(&mut self, arg_num: u64, bytes: &[u8]) {
        let want = self.function.param_sizes[arg_num as usize];
        assert_eq!(bytes.len(), want, "{}: argument {arg_num} is {want} bytes, given {}", self.function.name, bytes.len());
        self.params[arg_num as usize] = words(bytes);
    }

    pub fn set_buffer(&mut self, arg_num: u64, buffer: &CudaBuffer) {
        self.keep.borrow_mut().push(buffer.clone());
        self.params[arg_num as usize] = vec![buffer.device_address()];
    }

    pub fn set_buffer_at(&mut self, arg_num: u64, buffer: &CudaBuffer, offset: u64) -> Result<(), String> {
        self.keep.borrow_mut().push(buffer.clone());
        self.params[arg_num as usize] = vec![buffer.device_address() + offset];
        Ok(())
    }

    pub fn set_thread_group_memory(&mut self, arg_num: u64, n_bytes: u64) {
        let at = self.shared.next_multiple_of(16);
        self.shared = at + n_bytes;
        self.params[arg_num as usize] = vec![at];
    }

    pub fn launch(&mut self, grid: &[u64], block: &[u64]) -> Result<(), String> {
        let f = self.function;
        if let Some(i) = self.params.iter().position(|p| p.is_empty()) {
            let name = f.arguments.iter().find(|a| a.compute_function_arg_index == i as u64).map_or("?", |a| a.name.as_str());
            return Err(format!("{}: argument {name} was not bound", f.name));
        }
        if self.shared > f.dynamic_limit as u64 {
            return Err(format!("{}: {} bytes of threadgroup memory, more than the {} the device allows", f.name, self.shared, f.dynamic_limit));
        }
        let d = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(1) as u32;
        // An empty grid launches nothing (Metal dispatches nothing; CUDA calls it an invalid argument).
        if (0..3).any(|i| d(grid, i) == 0 || d(block, i) == 0) {
            return Ok(());
        }
        self.ops.borrow_mut().push(Op::Launch {
            func: f.func,
            grid: [d(grid, 0), d(grid, 1), d(grid, 2)],
            block: [d(block, 0), d(block, 1), d(block, 2)],
            shared: self.shared as u32,
            params: self.params.clone(),
            name: f.name.clone(),
        });
        Ok(())
    }

    pub fn use_buffer(&mut self, buffer: &CudaBuffer) {
        self.keep.borrow_mut().push(buffer.clone());
    }

    /// CUDA has no indirect launch: `max_groups` are launched, and those past the counts return first (the shim's
    /// `__ukl_groups`), so the kernel runs on the grid the counts give, with no host read of them.
    pub fn launch_indirect(&mut self, groups: &CudaBuffer, offset: u64, max_groups: &[u64], block: &[u64]) -> Result<(), String> {
        let hidden = self.function.param_sizes.len();
        self.keep.borrow_mut().push(groups.clone());
        self.params[hidden] = vec![groups.device_address() + offset];
        let r = self.launch(max_groups, block);
        self.params[hidden] = vec![0];
        r
    }
}
