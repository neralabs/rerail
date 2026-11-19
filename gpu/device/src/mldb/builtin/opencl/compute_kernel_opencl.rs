//! The OpenCL runtime of the grid kernels, beside compute_kernel_metal.rs, compute_kernel_cuda.rs and
//! compute_kernel_cpu.rs and with their interface (Rerail, decision 0440 §3.5): for the GPUs that have OpenCL and no
//! other way in, integrated ones above all. A library compiled at run time from UKL source (OpenCL C 1.2, which the
//! library is written in) by the device's driver, its kernels with their arguments, queues of launches, copies and
//! fills, their events, and buffers.
//!
//! Copyright (c) 2026 Jeremy Barnes. This file is part of MLDB's forest as carried in Rerail. Apache-2.0 (see the
//! LICENSE at `forest/src/mldb/`).
//!
//! - **The driver** is the system's OpenCL library, loaded when the device is first asked for (macOS's framework,
//!   else `libOpenCL.so.1`): no link dependency, and without it no OpenCL device.
//! - **The device** is the first GPU of the first platform that has one (`RERAIL_OPENCL_DEVICE=k`: the k-th GPU
//!   over all platforms). It must share the host's memory (`CL_DEVICE_HOST_UNIFIED_MEMORY`): a buffer is host
//!   memory the device reads in place (`CL_MEM_USE_HOST_PTR`), never a copy (decision 0353). A discrete device
//!   would stage it, so it needs SVM, which is not written; it is refused.
//! - **The SIMD group**: no OpenCL device we can test has subgroups, so they are emulated through threadgroup
//!   memory at 32 lanes (`ukl_opencl_shim.h`), or a group is one lane (`RERAIL_OPENCL_SIMD_WIDTH=1`).
//! - **A queue** holds its operations until a flush, which enqueues them in order on its root's in-order command
//!   queue, then a marker: the event. A queue's work never interleaves with another's, as with Metal's command
//!   buffers. A wait for a host-signalled event holds the rest on the host, and the root's later flushes behind it,
//!   until the host signals (Apple's `clFlush` blocks on a barrier on a user event, so the device cannot hold it).
//! - **The device seconds** of a flush are its kernels' profiled times.
//! - **Arguments without reflection**, as on CUDA (`rows.rs`): each kernel's info kernel (`Name__ukl_info`) and
//!   each `ARGS` structure's layout kernel (`Type__ukl_layout`) are run once, when the kernel is first asked for.

use crate::compute_kernel::*;
use std::cell::RefCell;
use std::ffi::{c_char, c_void, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

type ClInt = i32;
type ClUint = u32;
type Ptr = *mut c_void;

const CL_SUCCESS: ClInt = 0;
const CL_DEVICE_TYPE_GPU: u64 = 1 << 2;
const CL_DEVICE_NAME: ClUint = 0x102B;
const CL_DEVICE_VERSION: ClUint = 0x102F;
const CL_DEVICE_MAX_WORK_GROUP_SIZE: ClUint = 0x1004;
const CL_DEVICE_HOST_UNIFIED_MEMORY: ClUint = 0x1035;
const CL_DEVICE_MEM_BASE_ADDR_ALIGN: ClUint = 0x1019;
const CL_BUFFER_CREATE_TYPE_REGION: ClUint = 0x1220;
const CL_MEM_READ_WRITE: u64 = 1;
const CL_MEM_USE_HOST_PTR: u64 = 1 << 3;
const CL_QUEUE_PROFILING_ENABLE: u64 = 1 << 1;
const CL_PROGRAM_BUILD_LOG: ClUint = 0x1183;
const CL_PROGRAM_BINARY_SIZES: ClUint = 0x1165;
const CL_PROGRAM_BINARIES: ClUint = 0x1166;
const CL_DRIVER_VERSION: ClUint = 0x102D;
const CL_PROFILING_COMMAND_START: ClUint = 0x1282;
const CL_PROFILING_COMMAND_END: ClUint = 0x1283;
const CL_EVENT_COMMAND_EXECUTION_STATUS: ClUint = 0x11D3;
const CL_COMPLETE: ClInt = 0;
const CL_FALSE: ClUint = 0;

/// The OpenCL entry points this runtime calls, from the system's library.
#[allow(clippy::type_complexity)]
struct Api {
    get_platform_ids: unsafe extern "C" fn(ClUint, *mut Ptr, *mut ClUint) -> ClInt,
    get_device_ids: unsafe extern "C" fn(Ptr, u64, ClUint, *mut Ptr, *mut ClUint) -> ClInt,
    get_device_info: unsafe extern "C" fn(Ptr, ClUint, usize, Ptr, *mut usize) -> ClInt,
    create_context: unsafe extern "C" fn(Ptr, ClUint, *const Ptr, Ptr, Ptr, *mut ClInt) -> Ptr,
    create_command_queue: unsafe extern "C" fn(Ptr, Ptr, u64, *mut ClInt) -> Ptr,
    release_command_queue: unsafe extern "C" fn(Ptr) -> ClInt,
    create_buffer: unsafe extern "C" fn(Ptr, u64, usize, Ptr, *mut ClInt) -> Ptr,
    release_mem_object: unsafe extern "C" fn(Ptr) -> ClInt,
    create_sub_buffer: unsafe extern "C" fn(Ptr, u64, ClUint, *const c_void, *mut ClInt) -> Ptr,
    create_program_with_source: unsafe extern "C" fn(Ptr, ClUint, *const *const c_char, *const usize, *mut ClInt) -> Ptr,
    create_program_with_binary: unsafe extern "C" fn(Ptr, ClUint, *const Ptr, *const usize, *const *const u8, *mut ClInt, *mut ClInt) -> Ptr,
    get_program_info: unsafe extern "C" fn(Ptr, ClUint, usize, Ptr, *mut usize) -> ClInt,
    build_program: unsafe extern "C" fn(Ptr, ClUint, *const Ptr, *const c_char, Ptr, Ptr) -> ClInt,
    get_program_build_info: unsafe extern "C" fn(Ptr, Ptr, ClUint, usize, Ptr, *mut usize) -> ClInt,
    release_program: unsafe extern "C" fn(Ptr) -> ClInt,
    create_kernel: unsafe extern "C" fn(Ptr, *const c_char, *mut ClInt) -> Ptr,
    release_kernel: unsafe extern "C" fn(Ptr) -> ClInt,
    set_kernel_arg: unsafe extern "C" fn(Ptr, ClUint, usize, *const c_void) -> ClInt,
    enqueue_nd_range_kernel: unsafe extern "C" fn(Ptr, Ptr, ClUint, *const usize, *const usize, *const usize, ClUint, *const Ptr, *mut Ptr) -> ClInt,
    enqueue_write_buffer: unsafe extern "C" fn(Ptr, Ptr, ClUint, usize, usize, *const c_void, ClUint, *const Ptr, *mut Ptr) -> ClInt,
    enqueue_copy_buffer: unsafe extern "C" fn(Ptr, Ptr, Ptr, usize, usize, usize, ClUint, *const Ptr, *mut Ptr) -> ClInt,
    enqueue_fill_buffer: unsafe extern "C" fn(Ptr, Ptr, *const c_void, usize, usize, usize, ClUint, *const Ptr, *mut Ptr) -> ClInt,
    enqueue_marker_with_wait_list: unsafe extern "C" fn(Ptr, ClUint, *const Ptr, *mut Ptr) -> ClInt,
    wait_for_events: unsafe extern "C" fn(ClUint, *const Ptr) -> ClInt,
    get_event_info: unsafe extern "C" fn(Ptr, ClUint, usize, Ptr, *mut usize) -> ClInt,
    get_event_profiling_info: unsafe extern "C" fn(Ptr, ClUint, usize, Ptr, *mut usize) -> ClInt,
    release_event: unsafe extern "C" fn(Ptr) -> ClInt,
    flush: unsafe extern "C" fn(Ptr) -> ClInt,
    finish: unsafe extern "C" fn(Ptr) -> ClInt,
}

unsafe impl Send for Api {}
unsafe impl Sync for Api {}

/// The system's OpenCL library, loaded once.
fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| {
        let paths: &[&str] = if cfg!(target_os = "macos") {
            &["/System/Library/Frameworks/OpenCL.framework/OpenCL"]
        } else {
            &["libOpenCL.so.1", "libOpenCL.so"]
        };
        let handle = paths
            .iter()
            .map(|p| {
                let c = CString::new(*p).unwrap();
                // SAFETY: the system's OpenCL loader.
                unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) }
            })
            .find(|h| !h.is_null())
            .ok_or_else(|| format!("no OpenCL library ({})", paths.join(", ")))?;
        let sym = |name: &str| -> Result<Ptr, String> {
            let c = CString::new(name).unwrap();
            let p = unsafe { libc::dlsym(handle, c.as_ptr()) };
            if p.is_null() { Err(format!("the OpenCL library has no {name}")) } else { Ok(p) }
        };
        // SAFETY: each symbol is OpenCL 1.2's function of that name, with the signature its field declares.
        macro_rules! f {
            ($n:literal) => {
                unsafe { std::mem::transmute::<Ptr, _>(sym($n)?) }
            };
        }
        Ok(Api {
            get_platform_ids: f!("clGetPlatformIDs"),
            get_device_ids: f!("clGetDeviceIDs"),
            get_device_info: f!("clGetDeviceInfo"),
            create_context: f!("clCreateContext"),
            create_command_queue: f!("clCreateCommandQueue"),
            release_command_queue: f!("clReleaseCommandQueue"),
            create_buffer: f!("clCreateBuffer"),
            release_mem_object: f!("clReleaseMemObject"),
            create_sub_buffer: f!("clCreateSubBuffer"),
            create_program_with_source: f!("clCreateProgramWithSource"),
            create_program_with_binary: f!("clCreateProgramWithBinary"),
            get_program_info: f!("clGetProgramInfo"),
            build_program: f!("clBuildProgram"),
            get_program_build_info: f!("clGetProgramBuildInfo"),
            release_program: f!("clReleaseProgram"),
            create_kernel: f!("clCreateKernel"),
            release_kernel: f!("clReleaseKernel"),
            set_kernel_arg: f!("clSetKernelArg"),
            enqueue_nd_range_kernel: f!("clEnqueueNDRangeKernel"),
            enqueue_write_buffer: f!("clEnqueueWriteBuffer"),
            enqueue_copy_buffer: f!("clEnqueueCopyBuffer"),
            enqueue_fill_buffer: f!("clEnqueueFillBuffer"),
            enqueue_marker_with_wait_list: f!("clEnqueueMarkerWithWaitList"),
            wait_for_events: f!("clWaitForEvents"),
            get_event_info: f!("clGetEventInfo"),
            get_event_profiling_info: f!("clGetEventProfilingInfo"),
            release_event: f!("clReleaseEvent"),
            flush: f!("clFlush"),
            finish: f!("clFinish"),
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// An OpenCL call's status as a result.
fn check(what: &str, e: ClInt) -> Result<(), String> {
    if e == CL_SUCCESS { Ok(()) } else { Err(format!("OpenCL: {what} failed ({e})")) }
}

fn device_info_bytes(a: &Api, d: Ptr, param: ClUint) -> Vec<u8> {
    let mut n = 0usize;
    unsafe { (a.get_device_info)(d, param, 0, std::ptr::null_mut(), &mut n) };
    let mut v = vec![0u8; n];
    unsafe { (a.get_device_info)(d, param, n, v.as_mut_ptr().cast(), std::ptr::null_mut()) };
    v
}

fn device_info_string(a: &Api, d: Ptr, param: ClUint) -> String {
    let v = device_info_bytes(a, d, param);
    String::from_utf8_lossy(&v).trim_end_matches('\0').trim().to_string()
}

/// The SIMD groups' lanes of the OpenCL device's kernels: `RERAIL_OPENCL_SIMD_WIDTH` (1 or 32), else 32, emulated.
pub fn simd_width() -> u32 {
    match std::env::var("RERAIL_OPENCL_SIMD_WIDTH") {
        Err(_) => 32,
        Ok(w) => match w.parse::<u32>() {
            Ok(n @ (1 | 32)) => n,
            _ => panic!("RERAIL_OPENCL_SIMD_WIDTH={w}: 1 or 32 (native subgroups are not written)"),
        },
    }
}

struct DeviceInner {
    device: Ptr,
    context: Ptr,
    name: String,
    max_group: usize,
    /// The alignment of a sub-buffer's origin, in bytes (`CL_DEVICE_MEM_BASE_ADDR_ALIGN`, given in bits).
    base_align: u64,
}

unsafe impl Send for DeviceInner {}
unsafe impl Sync for DeviceInner {}

/// The OpenCL device, with its context.
#[derive(Clone)]
pub struct ClDevice(Arc<DeviceInner>);

impl std::fmt::Debug for ClDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClDevice({})", self.0.name)
    }
}

impl ClDevice {
    /// The OpenCL device (the module's notes say which), or why there is none.
    pub fn open() -> Result<ClDevice, String> {
        let a = api()?;
        let mut np = 0u32;
        check("clGetPlatformIDs", unsafe { (a.get_platform_ids)(0, std::ptr::null_mut(), &mut np) })?;
        let mut platforms = vec![std::ptr::null_mut(); np as usize];
        check("clGetPlatformIDs", unsafe { (a.get_platform_ids)(np, platforms.as_mut_ptr(), std::ptr::null_mut()) })?;
        let mut gpus = Vec::new();
        for p in platforms {
            let mut nd = 0u32;
            if unsafe { (a.get_device_ids)(p, CL_DEVICE_TYPE_GPU, 0, std::ptr::null_mut(), &mut nd) } != CL_SUCCESS {
                continue;
            }
            let mut ds = vec![std::ptr::null_mut(); nd as usize];
            unsafe { (a.get_device_ids)(p, CL_DEVICE_TYPE_GPU, nd, ds.as_mut_ptr(), std::ptr::null_mut()) };
            gpus.extend(ds);
        }
        let k: usize = std::env::var("RERAIL_OPENCL_DEVICE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let device = *gpus.get(k).ok_or_else(|| format!("no OpenCL GPU {k} ({} found)", gpus.len()))?;
        let name = format!("{} ({})", device_info_string(a, device, CL_DEVICE_NAME), device_info_string(a, device, CL_DEVICE_VERSION));
        let unified = device_info_bytes(a, device, CL_DEVICE_HOST_UNIFIED_MEMORY);
        if unified.len() < 4 || u32::from_ne_bytes(unified[..4].try_into().unwrap()) == 0 {
            return Err(format!("OpenCL device {name} does not share the host's memory: its buffers would be copies (decision 0353)"));
        }
        let mg = device_info_bytes(a, device, CL_DEVICE_MAX_WORK_GROUP_SIZE);
        let max_group = usize::from_ne_bytes(mg[..8].try_into().unwrap());
        let mut e = 0;
        let context = unsafe { (a.create_context)(std::ptr::null_mut(), 1, &device, std::ptr::null_mut(), std::ptr::null_mut(), &mut e) };
        check("clCreateContext", e)?;
        let ba = device_info_bytes(a, device, CL_DEVICE_MEM_BASE_ADDR_ALIGN);
        let base_align = (u32::from_ne_bytes(ba[..4].try_into().unwrap()) as u64 / 8).max(1);
        let d = ClDevice(Arc::new(DeviceInner { device, context, name, max_group, base_align }));
        d.reads_host_pages_in_place()?;
        Ok(d)
    }

    /// Whether a kernel's write into a buffer over host pages is in those pages once it completes, with no map: a
    /// driver may say it shares the host's memory and still keep a copy of a `CL_MEM_USE_HOST_PTR` buffer, made
    /// visible only by a map (NVIDIA's on a GB10, whose memory is the host's: the reflection kernels' words read
    /// 0). Such a buffer is a copy (decision 0353), so the device is refused.
    fn reads_host_pages_in_place(&self) -> Result<(), String> {
        const SEEN: u32 = 0x5EED_0353;
        let lib = ClComputeFunctionLibrary::compile_from_source(self, "__kernel void ukl_in_place(__global uint* o) { o[0] = 0x5EED0353u; }", "the in-place probe")?;
        let a = api()?;
        let k = lib.kernel("ukl_in_place")?;
        let out = alloc_uninitialized_array_sync::<u32>(self, 4);
        let mut e = 0;
        let q = unsafe { (a.create_command_queue)(self.0.context, self.0.device, 0, &mut e) };
        check("clCreateCommandQueue", e)?;
        let r = (|| {
            check("clSetKernelArg", unsafe { (a.set_kernel_arg)(k.0, 0, std::mem::size_of::<Ptr>(), (&out.0.mem as *const Ptr).cast()) })?;
            let one = 1usize;
            check("the in-place probe", unsafe { (a.enqueue_nd_range_kernel)(q, k.0, 1, std::ptr::null(), &one, &one, 0, std::ptr::null(), std::ptr::null_mut()) })?;
            check("clFinish", unsafe { (a.finish)(q) })
        })();
        unsafe { (a.release_command_queue)(q) };
        r?;
        // SAFETY: the buffer's host pages, which no kernel uses now.
        let seen = unsafe { std::ptr::read_volatile(out.0.ptr.cast::<u32>()) };
        if seen != SEEN {
            return Err(format!(
                "OpenCL device {} keeps a copy of host memory (a kernel's write is not in the host's pages without a map): its buffers would be copies (decision 0353)",
                self.0.name
            ));
        }
        Ok(())
    }

    pub fn name(&self) -> &str {
        &self.0.name
    }

    /// The largest threadgroup, and so the emulated SIMD group's scratch (`UKL_TG_MAX`).
    pub fn max_group(&self) -> usize {
        self.0.max_group.min(1024)
    }

    /// The defines its kernels' preprocessing takes: the SIMD width and the largest threadgroup.
    pub fn defines(&self) -> Vec<(&'static str, String)> {
        vec![("UKL_SIMD_WIDTH", format!("{}u", simd_width())), ("UKL_TG_MAX", format!("{}u", self.max_group()))]
    }

    pub fn new_shared_event(&self) -> ClSharedEvent {
        ClSharedEvent(Arc::new(Mutex::new(Signal { value: 0, waiting: Vec::new() })))
    }
}

/// A buffer's memory: host pages, and the OpenCL buffer made over them.
struct Allocation {
    ptr: *mut u8,
    len: u64,
    /// `None`: memory wrapped in place (`wrap_host_memory`), its owner's.
    layout: Option<std::alloc::Layout>,
    mem: Ptr,
    /// A sub-buffer's buffer (`set_buffer_at`): kept while the region is.
    _parent: Option<ClBuffer>,
}

unsafe impl Send for Allocation {}
unsafe impl Sync for Allocation {}

impl Drop for Allocation {
    fn drop(&mut self) {
        // A queue's work keeps the buffers it uses until it has completed, so no kernel reads these pages now.
        if let Ok(a) = api() {
            unsafe { (a.release_mem_object)(self.mem) };
        }
        if let Some(layout) = self.layout {
            unsafe { std::alloc::dealloc(self.ptr, layout) };
        }
    }
}

/// A buffer: host memory the device reads in place, shared by its clones.
#[derive(Clone)]
pub struct ClBuffer(Arc<Allocation>);

impl std::fmt::Debug for ClBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClBuffer({:p}, {} bytes)", self.0.ptr, self.0.len)
    }
}

impl ClBuffer {
    pub fn contents(&self) -> *mut c_void {
        self.0.ptr.cast()
    }

    pub fn length(&self) -> u64 {
        self.0.len
    }
}

/// `allocUninitializedArraySync`: memory for `length` elements (at least 16 bytes), zeroed, page-aligned and a whole
/// number of cache lines: what drivers ask of host memory they use in place (Intel's: 4,096-aligned, a multiple of
/// 64 bytes). A host write after a buffer's first use is seen by the next launch (the armed prediction's test).
pub fn alloc_uninitialized_array_sync<T>(device: &ClDevice, length: usize) -> ClBuffer {
    let len = (length * std::mem::size_of::<T>()).max(16);
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as usize;
    let layout = std::alloc::Layout::from_size_align(len.next_multiple_of(64), page).unwrap();
    // SAFETY: a non-zero size.
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    assert!(!ptr.is_null(), "out of memory for a {len}-byte buffer");
    let a = api().expect("an OpenCL device without its library");
    let mut e = 0;
    let mem = unsafe { (a.create_buffer)(device.0.context, CL_MEM_READ_WRITE | CL_MEM_USE_HOST_PTR, layout.size(), ptr.cast(), &mut e) };
    if e != CL_SUCCESS {
        unsafe { std::alloc::dealloc(ptr, layout) };
        panic!("OpenCL: clCreateBuffer of {len} bytes failed ({e})");
    }
    ClBuffer(Arc::new(Allocation { ptr, len: len as u64, layout: Some(layout), mem, _parent: None }))
}

/// A buffer over `len` bytes of the host's memory at `ptr`, read in place (decision 0353; the device reads host
/// pages in place, `ClDevice::open`). Drivers use host memory in place only where it is page-aligned and a whole
/// number of cache lines (`alloc_uninitialized_array_sync`): other memory is refused.
///
/// # Safety
/// The memory outlives the buffer and its clones, and every use of them by queued work.
pub unsafe fn wrap_host_memory(device: &ClDevice, ptr: *mut u8, len: usize) -> Result<ClBuffer, String> {
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as usize;
    if (ptr as usize) % page != 0 || len % 64 != 0 || len == 0 {
        return Err(format!("OpenCL: host memory at {ptr:p} ({len} bytes) is not page-aligned and a whole number of cache lines: a driver would copy it"));
    }
    let a = api()?;
    let mut e = 0;
    let mem = unsafe { (a.create_buffer)(device.0.context, CL_MEM_READ_WRITE | CL_MEM_USE_HOST_PTR, len, ptr.cast(), &mut e) };
    check("clCreateBuffer", e)?;
    Ok(ClBuffer(Arc::new(Allocation { ptr, len: len as u64, layout: None, mem, _parent: None })))
}

/// `transferToDeviceImmutable`: memory holding `data`.
pub fn transfer_to_device_immutable<T: crate::Pod>(device: &ClDevice, data: &[T]) -> ClBuffer {
    let b = alloc_uninitialized_array_sync::<T>(device, data.len());
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().cast::<u8>(), b.contents().cast::<u8>(), std::mem::size_of_val(data)) };
    b
}

/// `transferToHostSync`: the first `length` elements (the work writing them must have completed).
pub fn transfer_to_host_sync<T: crate::Pod + Clone>(buffer: &ClBuffer, length: usize) -> Vec<T> {
    assert!(length * std::mem::size_of::<T>() <= buffer.length() as usize);
    unsafe { std::slice::from_raw_parts(buffer.contents().cast::<T>(), length).to_vec() }
}

/// A host-signalled value, and the roots whose work waits for it to reach a value.
struct Signal {
    value: u64,
    waiting: Vec<(u64, ClQueueRoot)>,
}

/// A host-signalled event (`metal::SharedEvent`). Queued work that waits for it is held on the host, behind the
/// wait, until the host signals: the signal enqueues it (a user event behind a barrier would do it on the device,
/// but Apple's `clFlush` blocks on such a barrier until the user event completes).
#[derive(Clone)]
pub struct ClSharedEvent(Arc<Mutex<Signal>>);

impl ClSharedEvent {
    pub fn set_signaled_value(&self, v: u64) {
        let ready: Vec<ClQueueRoot> = {
            let mut s = self.0.lock().unwrap();
            s.value = v;
            let (ready, wait): (Vec<_>, Vec<_>) = std::mem::take(&mut s.waiting).into_iter().partition(|(at, _)| *at <= v);
            s.waiting = wait;
            ready.into_iter().map(|(_, r)| r).collect()
        };
        for r in ready {
            r.drive();
        }
    }

    pub fn signaled_value(&self) -> u64 {
        self.0.lock().unwrap().value
    }
}

/// An OpenCL event, released when the last holder goes.
struct Event(Ptr);

unsafe impl Send for Event {}
unsafe impl Sync for Event {}

impl Drop for Event {
    fn drop(&mut self) {
        if let Ok(a) = api() {
            unsafe { (a.release_event)(self.0) };
        }
    }
}

static DEVICE_NANOS: AtomicU64 = AtomicU64::new(0);

/// The device seconds of every awaited event's kernels, so far in this process.
pub fn gpu_seconds_awaited() -> f64 {
    DEVICE_NANOS.load(Ordering::Relaxed) as f64 * 1e-9
}

struct Program(Ptr);

unsafe impl Send for Program {}
unsafe impl Sync for Program {}

impl Drop for Program {
    fn drop(&mut self) {
        if let Ok(a) = api() {
            unsafe { (a.release_program)(self.0) };
        }
    }
}

/// A compiled library: the program the driver built from the source.
pub struct ClComputeFunctionLibrary {
    program: Arc<Program>,
    name: String,
}

impl ClComputeFunctionLibrary {
    /// Builds OpenCL C (UKL preprocessed for OpenCL) with the device's driver.
    pub fn compile_from_source(device: &ClDevice, preprocessed_source: &str, file_name_to_appear_in_error_messages: &str) -> Result<Self, String> {
        let a = api()?;
        const OPTIONS: &str = "-cl-std=CL1.2";
        let options = CString::new(OPTIONS).unwrap();
        // The store's binary (`crate::store`), keyed by the device's name, version and driver beside the source.
        let driver = device_info_string(a, device.0.device, CL_DRIVER_VERSION);
        let key = crate::store::key(&[b"opencl", device.name().as_bytes(), driver.as_bytes(), OPTIONS.as_bytes(), preprocessed_source.as_bytes()]);
        if let Some(bin) = crate::store::load("opencl", &key, "bin") {
            let (n, p) = (bin.len(), bin.as_ptr());
            let (mut status, mut e) = (0, 0);
            let program = unsafe { (a.create_program_with_binary)(device.0.context, 1, &device.0.device, &n, &p, &mut status, &mut e) };
            if e == CL_SUCCESS && status == CL_SUCCESS {
                let program = Program(program);
                let built = unsafe { (a.build_program)(program.0, 1, &device.0.device, options.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut()) };
                if built == CL_SUCCESS {
                    crate::store::count(true);
                    return Ok(ClComputeFunctionLibrary { program: Arc::new(program), name: file_name_to_appear_in_error_messages.to_string() });
                }
            }
        }
        crate::store::count(false);
        let src = CString::new(preprocessed_source).map_err(|_| format!("{file_name_to_appear_in_error_messages}: a NUL in the source"))?;
        let p = src.as_ptr();
        let mut e = 0;
        let program = unsafe { (a.create_program_with_source)(device.0.context, 1, &p, std::ptr::null(), &mut e) };
        check("clCreateProgramWithSource", e)?;
        let program = Program(program);
        let e = unsafe { (a.build_program)(program.0, 1, &device.0.device, options.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut()) };
        if e != CL_SUCCESS {
            let mut n = 0usize;
            unsafe { (a.get_program_build_info)(program.0, device.0.device, CL_PROGRAM_BUILD_LOG, 0, std::ptr::null_mut(), &mut n) };
            let mut log = vec![0u8; n];
            unsafe { (a.get_program_build_info)(program.0, device.0.device, CL_PROGRAM_BUILD_LOG, n, log.as_mut_ptr().cast(), std::ptr::null_mut()) };
            let log = String::from_utf8_lossy(&log);
            let errors: Vec<&str> = log.lines().filter(|l| l.contains("error")).take(20).collect();
            return Err(format!("OpenCL building {file_name_to_appear_in_error_messages} for {} ({e}): {}", device.name(), errors.join("\n")));
        }
        // The one device's binary, kept for the next process.
        let mut size = 0usize;
        if unsafe { (a.get_program_info)(program.0, CL_PROGRAM_BINARY_SIZES, std::mem::size_of::<usize>(), (&raw mut size).cast(), std::ptr::null_mut()) } == CL_SUCCESS && size > 0 {
            let mut bin = vec![0u8; size];
            let mut ptr = bin.as_mut_ptr();
            if unsafe { (a.get_program_info)(program.0, CL_PROGRAM_BINARIES, std::mem::size_of::<*mut u8>(), (&raw mut ptr).cast(), std::ptr::null_mut()) } == CL_SUCCESS {
                crate::store::save("opencl", &key, "bin", &bin);
            }
        }
        Ok(ClComputeFunctionLibrary { program: Arc::new(program), name: file_name_to_appear_in_error_messages.to_string() })
    }

    fn kernel(&self, name: &str) -> Result<Kernel, String> {
        let a = api()?;
        let c = CString::new(name).unwrap();
        let mut e = 0;
        let k = unsafe { (a.create_kernel)(self.program.0, c.as_ptr(), &mut e) };
        if e != CL_SUCCESS {
            return Err(format!("{}: no kernel {name} ({e})", self.name));
        }
        Ok(Kernel(k))
    }

    /// One thread of the reflection kernel `name`, writing into words of which the result is the first `n`.
    fn reflect(&self, device: &ClDevice, name: &str) -> Result<Vec<u64>, String> {
        let a = api()?;
        let k = self.kernel(name)?;
        let out = alloc_uninitialized_array_sync::<u64>(device, 8192);
        let mut e = 0;
        let q = unsafe { (a.create_command_queue)(device.0.context, device.0.device, 0, &mut e) };
        check("clCreateCommandQueue", e)?;
        let r = (|| {
            check("clSetKernelArg", unsafe { (a.set_kernel_arg)(k.0, 0, std::mem::size_of::<Ptr>(), (&out.0.mem as *const Ptr).cast()) })?;
            let one = 1usize;
            check(name, unsafe { (a.enqueue_nd_range_kernel)(q, k.0, 1, std::ptr::null(), &one, &one, 0, std::ptr::null(), std::ptr::null_mut()) })?;
            check("clFinish", unsafe { (a.finish)(q) })
        })();
        unsafe { (a.release_command_queue)(q) };
        r?;
        Ok(transfer_to_host_sync::<u64>(&out, 8192))
    }

    /// The numbers and the text a reflection kernel wrote: its count of numbers, the numbers, then the text's length
    /// and bytes.
    fn numbers_and_text(w: &[u64], what: &str) -> Result<(Vec<u64>, String), String> {
        let n = w[0] as usize;
        let len = *w.get(1 + n).ok_or_else(|| format!("{what}: {n} reflected numbers"))? as usize;
        if 2 + n + len.div_ceil(8) > w.len() {
            return Err(format!("{what}: reflected text of {len} bytes"));
        }
        let bytes: Vec<u8> = w[2 + n..2 + n + len.div_ceil(8)].iter().flat_map(|x| x.to_le_bytes()).take(len).collect();
        Ok((w[1..1 + n].to_vec(), String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// An `args` structure's layout: its layout kernel run (`UKL_REFLECT_STRUCT`).
    fn struct_type(&self, device: &ClDevice, ty: &str) -> Result<KernelBaseType, String> {
        let w = self.reflect(device, &format!("{ty}__ukl_layout")).map_err(|e| format!("{ty} is an ARGS row's type with no UKL_REFLECT_STRUCT: {e}"))?;
        let (layout, names) = Self::numbers_and_text(&w, ty)?;
        crate::rows::struct_type(ty, &names, &layout)
    }

    /// The kernel `function_name`, with its arguments from its rows (its info kernel's).
    pub fn get_function(&self, device: &ClDevice, function_name: &str) -> Result<ClComputeFunction, String> {
        let kernel = self.kernel(function_name)?;
        let w = self.reflect(device, &format!("{function_name}__ukl_info"))?;
        let (rows, text) = Self::numbers_and_text(&w, function_name)?;
        let (arguments, param_sizes) = crate::rows::arguments(function_name, &text, &rows, |ty| self.struct_type(device, ty))?;
        // An `RWLOCAL` row is a `__local` argument here, sized at the launch, not an offset.
        let local: Vec<bool> = {
            let fields = crate::rows::split_top(&text);
            fields.chunks(4).filter(|r| matches!(r[0], "ROBUFFER" | "CONSTBUFFER" | "RWBUFFER" | "WOBUFFER" | "ARGS" | "LITERAL" | "TUNEABLE" | "RWLOCAL")).map(|r| r[0] == "RWLOCAL").collect()
        };
        Ok(ClComputeFunction {
            kernel: Arc::new(Mutex::new(kernel)),
            device: device.clone(),
            name: function_name.to_string(),
            arguments,
            param_sizes,
            local,
        })
    }
}

struct Kernel(Ptr);

unsafe impl Send for Kernel {}

impl Drop for Kernel {
    fn drop(&mut self) {
        if let Ok(a) = api() {
            unsafe { (a.release_kernel)(self.0) };
        }
    }
}

/// A kernel of a library, with its arguments (`MetalComputeFunction`). Its OpenCL kernel's arguments are set and the
/// launch enqueued under one lock (an enqueue takes the arguments as they are then), so launches from several threads
/// do not mix their arguments.
pub struct ClComputeFunction {
    kernel: Arc<Mutex<Kernel>>,
    device: ClDevice,
    pub name: String,
    arguments: Vec<GridComputeFunctionArgument>,
    param_sizes: Vec<usize>,
    local: Vec<bool>,
}

impl ClComputeFunction {
    pub fn get_argument_info(&self) -> &[GridComputeFunctionArgument] {
        &self.arguments
    }
}

/// A root's command queue, and the flushed work not enqueued yet: held behind a wait for a host-signalled event,
/// with every flush after it.
struct RootState {
    q: Ptr,
    backlog: std::collections::VecDeque<Pending>,
}

unsafe impl Send for RootState {}

/// What a queue's work goes to (`metal::CommandQueue`): an in-order OpenCL command queue, with profiling. Work waiting
/// for a host-signalled event holds back the work flushed after it, as on Metal.
#[derive(Clone)]
pub struct ClQueueRoot(Arc<Mutex<RootState>>);

impl ClQueueRoot {
    pub fn new(device: &ClDevice) -> Result<ClQueueRoot, String> {
        let a = api()?;
        let mut e = 0;
        let q = unsafe { (a.create_command_queue)(device.0.context, device.0.device, CL_QUEUE_PROFILING_ENABLE, &mut e) };
        check("clCreateCommandQueue", e)?;
        Ok(ClQueueRoot(Arc::new(Mutex::new(RootState { q, backlog: Default::default() }))))
    }

    /// Enqueues the backlog in order, up to a wait for a host-signalled event that has not reached its value: there
    /// it stops, and the event's signal drives the root again.
    fn drive(&self) {
        let a = api().expect("an OpenCL queue without its library");
        let mut root = self.0.lock().unwrap();
        let q = root.q;
        while let Some(p) = root.backlog.front_mut() {
            while let Some(op) = p.ops.pop_front() {
                match op {
                    Op::Run(f) => {
                        if p.error.is_none() {
                            let mut c = Enqueue { a, q, kernels: Vec::new() };
                            p.error = f(&mut c).err();
                            p.kernels.extend(c.kernels);
                        }
                    }
                    Op::Wait(event, value) => {
                        let mut s = event.0.lock().unwrap();
                        if s.value < value {
                            // The wait stays first: a drive before the signal (another queue's flush) stops here too.
                            s.waiting.push((value, self.clone()));
                            drop(s);
                            p.ops.push_front(Op::Wait(event, value));
                            unsafe { (a.flush)(q) };
                            return;
                        }
                    }
                }
            }
            let p = root.backlog.pop_front().unwrap();
            let mut marker = std::ptr::null_mut();
            let e = unsafe { (a.enqueue_marker_with_wait_list)(q, 0, std::ptr::null(), &mut marker) };
            let mut d = p.done.state.lock().unwrap();
            d.error = p.error.or_else(|| (e != CL_SUCCESS).then(|| format!("OpenCL: clEnqueueMarkerWithWaitList failed ({e})")));
            d.marker = (e == CL_SUCCESS).then(|| Event(marker));
            d.kernels = p.kernels;
            d.enqueued = true;
            drop(d);
            p.done.cv.notify_all();
        }
        unsafe { (a.flush)(q) };
    }
}

/// What an operation enqueues into, and what it leaves for the event: its kernels' events (their device time).
struct Enqueue<'a> {
    a: &'a Api,
    q: Ptr,
    kernels: Vec<Event>,
}

/// An operation of a queue: work to enqueue, or a wait for a host-signalled event that holds what follows.
enum Op {
    Run(Box<dyn FnOnce(&mut Enqueue) -> Result<(), String> + Send>),
    Wait(ClSharedEvent, u64),
}

/// A flush's work not yet enqueued, with what its event will hold.
struct Pending {
    ops: std::collections::VecDeque<Op>,
    kernels: Vec<Event>,
    error: Option<String>,
    done: Arc<Done>,
}

/// A flush's state: whether its work is enqueued yet (and then its marker, its kernels' events, its first error),
/// and the buffers and bytes it uses, kept until it has completed.
#[derive(Default)]
struct DoneState {
    enqueued: bool,
    marker: Option<Event>,
    kernels: Vec<Event>,
    error: Option<String>,
    keep: Vec<Box<dyn std::any::Any + Send>>,
    seconds: Option<f64>,
}

#[derive(Default)]
struct Done {
    state: Mutex<DoneState>,
    cv: std::sync::Condvar,
}

impl Drop for Done {
    fn drop(&mut self) {
        // The kept buffers go with the last holder: not before the work that reads them has completed.
        let d = self.state.get_mut().unwrap();
        if let (Some(m), Ok(a)) = (&d.marker, api()) {
            unsafe { (a.wait_for_events)(1, &m.0) };
        }
    }
}

/// A queue of operations (`MetalComputeQueue`): launches, copies and fills, enqueued in order on its root's command
/// queue at a flush. The buffers and bytes they use are kept until the flush's work has completed.
pub struct ClComputeQueue {
    root: ClQueueRoot,
    ops: RefCell<Vec<Op>>,
    keep: RefCell<Vec<Box<dyn std::any::Any + Send>>>,
}

impl ClComputeQueue {
    pub fn new(root: &ClQueueRoot) -> Self {
        ClComputeQueue { root: root.clone(), ops: Default::default(), keep: Default::default() }
    }

    pub fn new_bind_context<'a>(&'a self, function: &'a ClComputeFunction) -> ClBindContext<'a> {
        // The rows' parameters, then the indirect launch's counts and their first word (`GEN_OUTER_PARAMETER_GROUPS`:
        // none, a direct launch).
        let mut params = vec![None; function.param_sizes.len() + 2];
        let n = function.param_sizes.len();
        (params[n], params[n + 1]) = (Some(Param::Null), Some(Param::Bytes(0u32.to_ne_bytes().to_vec())));
        ClBindContext { queue: self, function, params }
    }

    fn push(&self, op: impl FnOnce(&mut Enqueue) -> Result<(), String> + Send + 'static) {
        self.ops.borrow_mut().push(Op::Run(Box::new(op)));
    }

    fn keep(&self, v: impl std::any::Any + Send) {
        self.keep.borrow_mut().push(Box::new(v));
    }

    /// `enqueueCopyFromHost`: the bytes are taken now, and copied into `dst` at `dst_offset` in queue order.
    pub fn enqueue_copy_from_host(&self, dst: &ClBuffer, dst_offset: u64, bytes: &[u8]) {
        assert!(dst_offset + bytes.len() as u64 <= dst.length(), "a copy past its buffer's end");
        let bytes = Arc::new(bytes.to_vec());
        let (d, b) = (dst.0.mem as usize, bytes.clone());
        self.keep(dst.clone());
        self.keep(bytes);
        self.push(move |c| {
            check("clEnqueueWriteBuffer", unsafe {
                (c.a.enqueue_write_buffer)(c.q, d as Ptr, CL_FALSE, dst_offset as usize, b.len(), b.as_ptr().cast(), 0, std::ptr::null(), std::ptr::null_mut())
            })
        });
    }

    /// `enqueueFillArray`: `count` copies of `value` into `dst` from element `offset` (all of it when `None`).
    pub fn enqueue_fill_array<T: crate::Pod>(&self, dst: &ClBuffer, value: &T, offset: usize, count: Option<usize>) {
        let size = std::mem::size_of::<T>();
        let count = count.unwrap_or(dst.length() as usize / size - offset);
        if count == 0 {
            return;
        }
        let one = unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size) }.to_vec();
        if !size.is_power_of_two() || size > 128 {
            let bytes: Vec<u8> = one.iter().copied().cycle().take(count * size).collect();
            return self.enqueue_copy_from_host(dst, (offset * size) as u64, &bytes);
        }
        let d = dst.0.mem as usize;
        self.keep(dst.clone());
        self.push(move |c| {
            check("clEnqueueFillBuffer", unsafe {
                (c.a.enqueue_fill_buffer)(c.q, d as Ptr, one.as_ptr().cast(), size, offset * size, count * size, 0, std::ptr::null(), std::ptr::null_mut())
            })
        });
    }

    /// `enqueueCopyBetweenDeviceRegions`.
    pub fn enqueue_copy_between_device_regions(&self, src: &ClBuffer, dst: &ClBuffer, src_offset: u64, dst_offset: u64, length: u64) {
        assert!(src_offset + length <= src.length() && dst_offset + length <= dst.length(), "a copy past its buffer's end");
        if length == 0 {
            return;
        }
        let (s, d) = (src.0.mem as usize, dst.0.mem as usize);
        self.keep(src.clone());
        self.keep(dst.clone());
        self.push(move |c| {
            check("clEnqueueCopyBuffer", unsafe {
                (c.a.enqueue_copy_buffer)(c.q, s as Ptr, d as Ptr, src_offset as usize, dst_offset as usize, length as usize, 0, std::ptr::null(), std::ptr::null_mut())
            })
        });
    }

    /// Holds the root's later work until the host signals `event` with `value` or more (Metal's
    /// `encodeWaitForEvent`).
    pub fn wait_for_event(&self, event: &ClSharedEvent, value: u64) {
        self.ops.borrow_mut().push(Op::Wait(event.clone(), value));
    }

    /// The work so far, handed to the root, which enqueues it behind its backlog; its event.
    pub fn flush(&mut self) -> ClComputeEvent {
        let done = Arc::new(Done::default());
        done.state.lock().unwrap().keep = std::mem::take(&mut *self.keep.borrow_mut());
        let ops = std::mem::take(&mut *self.ops.borrow_mut()).into();
        self.root.0.lock().unwrap().backlog.push_back(Pending { ops, kernels: Vec::new(), error: None, done: done.clone() });
        self.root.drive();
        ClComputeEvent { done }
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.flush().await_()
    }
}

/// The work a queue flushed (`MetalComputeEvent`). The work's buffers stay until it has completed, awaited or not.
pub struct ClComputeEvent {
    done: Arc<Done>,
}

fn kernel_seconds(kernels: &[Event]) -> f64 {
    let Ok(a) = api() else { return 0.0 };
    let at = |e: &Event, p| {
        let mut t = 0u64;
        unsafe { (a.get_event_profiling_info)(e.0, p, 8, (&mut t as *mut u64).cast(), std::ptr::null_mut()) };
        t
    };
    kernels.iter().map(|k| at(k, CL_PROFILING_COMMAND_END).saturating_sub(at(k, CL_PROFILING_COMMAND_START))).sum::<u64>() as f64 * 1e-9
}

impl ClComputeEvent {
    /// Waits for the work to complete; its first error, if it had one.
    pub fn await_(&self) -> Result<(), String> {
        let a = api()?;
        let mut d = self.done.state.lock().unwrap();
        while !d.enqueued {
            d = self.done.cv.wait(d).unwrap();
        }
        if let Some(m) = &d.marker {
            check("clWaitForEvents", unsafe { (a.wait_for_events)(1, &m.0) })?;
        }
        d.keep.clear();
        if d.seconds.is_none() {
            let secs = kernel_seconds(&d.kernels);
            DEVICE_NANOS.fetch_add((secs * 1e9) as u64, Ordering::Relaxed);
            d.seconds = Some(secs);
        }
        match &d.error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    pub fn is_complete(&self) -> bool {
        let d = self.done.state.lock().unwrap();
        if !d.enqueued {
            return false;
        }
        let (Some(m), Ok(a)) = (&d.marker, api()) else { return true };
        let mut status: ClInt = 1;
        unsafe { (a.get_event_info)(m.0, CL_EVENT_COMMAND_EXECUTION_STATUS, 4, (&mut status as *mut ClInt).cast(), std::ptr::null_mut()) };
        status <= CL_COMPLETE
    }

    /// The seconds the work's kernels took on the device (the work must have completed).
    pub fn gpu_seconds(&self) -> f64 {
        let d = self.done.state.lock().unwrap();
        d.seconds.unwrap_or_else(|| kernel_seconds(&d.kernels))
    }
}

/// One launch's arguments: each parameter's bytes (a buffer's: its OpenCL buffer; threadgroup memory's: its size).
pub struct ClBindContext<'a> {
    queue: &'a ClComputeQueue,
    function: &'a ClComputeFunction,
    params: Vec<Option<Param>>,
}

#[derive(Clone)]
enum Param {
    Bytes(Vec<u8>),
    Buffer(ClBuffer),
    Local(usize),
    /// A null buffer: a direct launch's counts (`GEN_OUTER_PARAMETER_GROUPS`).
    Null,
}

impl ClBindContext<'_> {
    pub fn set_primitive(&mut self, arg_num: u64, bytes: &[u8]) {
        let want = self.function.param_sizes[arg_num as usize];
        assert_eq!(bytes.len(), want, "{}: argument {arg_num} is {want} bytes, given {}", self.function.name, bytes.len());
        self.params[arg_num as usize] = Some(Param::Bytes(bytes.to_vec()));
    }

    pub fn set_buffer(&mut self, arg_num: u64, buffer: &ClBuffer) {
        self.params[arg_num as usize] = Some(Param::Buffer(buffer.clone()));
    }

    /// The buffer from byte `offset` on: a sub-buffer of it (the same memory, `clCreateSubBuffer`), whose origin the
    /// device's base alignment must divide.
    pub fn set_buffer_at(&mut self, arg_num: u64, buffer: &ClBuffer, offset: u64) -> Result<(), String> {
        if offset == 0 {
            self.set_buffer(arg_num, buffer);
            return Ok(());
        }
        let align = self.function.device.0.base_align;
        if offset % align != 0 || offset >= buffer.length() {
            return Err(format!("{}: argument {arg_num} at byte {offset} of a {}-byte buffer: OpenCL binds a region at a multiple of {align} bytes", self.function.name, buffer.length()));
        }
        let a = api()?;
        let region = [offset as usize, (buffer.length() - offset) as usize];
        let mut e = 0;
        let mem = unsafe { (a.create_sub_buffer)(buffer.0.mem, CL_MEM_READ_WRITE, CL_BUFFER_CREATE_TYPE_REGION, region.as_ptr().cast(), &mut e) };
        check("clCreateSubBuffer", e)?;
        // SAFETY: inside the buffer, which the region keeps.
        let ptr = unsafe { buffer.0.ptr.add(offset as usize) };
        let sub = ClBuffer(Arc::new(Allocation { ptr, len: buffer.length() - offset, layout: None, mem, _parent: Some(buffer.clone()) }));
        self.set_buffer(arg_num, &sub);
        Ok(())
    }

    pub fn set_thread_group_memory(&mut self, arg_num: u64, n_bytes: u64) {
        assert!(self.function.local[arg_num as usize], "{}: argument {arg_num} is no threadgroup memory", self.function.name);
        self.params[arg_num as usize] = Some(Param::Local((n_bytes as usize).max(16)));
    }

    pub fn use_buffer(&mut self, buffer: &ClBuffer) {
        self.queue.keep(buffer.clone());
    }

    /// OpenCL 1.2 has no indirect launch: `max_groups` are launched, and those past the counts return first (the
    /// shim's `__ukl_groups`), so the kernel runs on the grid the counts give, with no host read of them.
    pub fn launch_indirect(&mut self, groups: &ClBuffer, offset: u64, max_groups: &[u64], block: &[u64]) -> Result<(), String> {
        let n = self.function.param_sizes.len();
        self.params[n] = Some(Param::Buffer(groups.clone()));
        self.params[n + 1] = Some(Param::Bytes(((offset / 4) as u32).to_ne_bytes().to_vec()));
        self.launch(max_groups, block)
    }

    pub fn launch(&mut self, grid: &[u64], block: &[u64]) -> Result<(), String> {
        let f = self.function;
        if let Some(i) = self.params.iter().position(|p| p.is_none()) {
            let name = f.arguments.iter().find(|a| a.compute_function_arg_index == i as u64).map_or("?", |a| a.name.as_str());
            return Err(format!("{}: argument {name} was not bound", f.name));
        }
        let d = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(1) as usize;
        let (grid, block) = ([d(grid, 0), d(grid, 1), d(grid, 2)], [d(block, 0), d(block, 1), d(block, 2)]);
        if grid.contains(&0) || block.contains(&0) {
            return Ok(());
        }
        let threads: usize = block.iter().product();
        if threads > f.device.max_group() {
            return Err(format!("{}: a threadgroup of {threads} threads; the OpenCL device takes {}", f.name, f.device.max_group()));
        }
        let params: Vec<Param> = self.params.iter_mut().map(|p| p.take().unwrap()).collect();
        for p in &params {
            if let Param::Buffer(b) = p {
                self.queue.keep(b.clone());
            }
        }
        let global = [grid[0] * block[0], grid[1] * block[1], grid[2] * block[2]];
        let (kernel, name) = (f.kernel.clone(), f.name.clone());
        self.queue.push(move |c| {
            let k = kernel.lock().unwrap();
            for (i, p) in params.iter().enumerate() {
                let e = match p {
                    Param::Bytes(b) => unsafe { (c.a.set_kernel_arg)(k.0, i as u32, b.len(), b.as_ptr().cast()) },
                    Param::Buffer(b) => unsafe { (c.a.set_kernel_arg)(k.0, i as u32, std::mem::size_of::<Ptr>(), (&b.0.mem as *const Ptr).cast()) },
                    Param::Local(n) => unsafe { (c.a.set_kernel_arg)(k.0, i as u32, *n, std::ptr::null()) },
                    Param::Null => unsafe { (c.a.set_kernel_arg)(k.0, i as u32, std::mem::size_of::<Ptr>(), (&std::ptr::null_mut::<c_void>() as *const Ptr).cast()) },
                };
                check(&format!("{name}: setting argument {i}"), e)?;
            }
            let mut ev = std::ptr::null_mut();
            check(&format!("{name}: launch"), unsafe {
                (c.a.enqueue_nd_range_kernel)(c.q, k.0, 3, std::ptr::null(), global.as_ptr(), block.as_ptr(), 0, std::ptr::null(), &mut ev)
            })?;
            c.kernels.push(Event(ev));
            Ok(())
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kernel of `text` compiled for the OpenCL device, apart from the process's device (these tests run whatever
    /// `RERAIL_GPU` is); none without an OpenCL GPU that shares the host's memory.
    fn compile(name: &str, text: &str) -> Option<(ClDevice, ClComputeFunctionLibrary)> {
        let device = match ClDevice::open() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{e}: skipped");
                return None;
            }
        };
        let path = format!("/<tests>/{name}");
        let mut p = rerail_ukl::Preprocess::new(rerail_ukl::Backend::OpenCl).file(&path, text);
        for (n, v) in device.defines() {
            p = p.define(n, Some(&v));
        }
        let source = p.run(&path).unwrap_or_else(|e| panic!("{e}"));
        let lib = ClComputeFunctionLibrary::compile_from_source(&device, &source, name).unwrap_or_else(|e| panic!("{e}"));
        Some((device, lib))
    }

    const KERNELS: &str = r#"
#include <ukl/backend.h>

typedef struct Args {
    uint32_t value;
} Args;
UKL_REFLECT_STRUCT(Args, value)

DEFINE_KERNEL2(t, t_write,
               ARGS,     Args,     args, ,
               RWBUFFER, uint32_t, out,  "[1]",
               GID0,     uint32_t, i,    )
{
    if (i == 0u)
        out[0] = out[0] * 10u + args.value;
}
"#;

    fn write(q: &ClComputeQueue, f: &ClComputeFunction, out: &ClBuffer, value: u32) {
        let mut ctx = q.new_bind_context(f);
        ctx.set_primitive(0, &value.to_ne_bytes());
        ctx.set_buffer(1, out);
        ctx.launch(&[1], &[32]).unwrap();
    }

    /// Work behind a wait for a host-signalled event stays held until the signal, though another queue flushes onto
    /// the same root meanwhile; that queue's work waits behind it (as a Metal command buffer behind a waiting one),
    /// and both run, in order, at the signal.
    #[test]
    fn a_wait_holds_the_roots_later_work_until_the_signal() {
        let Some((device, lib)) = compile("hold.ukl", KERNELS) else { return };
        let f = lib.get_function(&device, "t_write").unwrap();
        assert_eq!(f.get_argument_info().iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["args", "out"]);
        let root = ClQueueRoot::new(&device).unwrap();
        let out = alloc_uninitialized_array_sync::<u32>(&device, 1);
        let event = device.new_shared_event();

        let mut held = ClComputeQueue::new(&root);
        held.wait_for_event(&event, 1);
        write(&held, &f, &out, 1);
        let first = held.flush();

        let mut later = ClComputeQueue::new(&root);
        write(&later, &f, &out, 2);
        let second = later.flush();

        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!first.is_complete() && !second.is_complete(), "work ran before the signal");
        assert_eq!(transfer_to_host_sync::<u32>(&out, 1), [0]);

        event.set_signaled_value(1);
        first.await_().unwrap();
        second.await_().unwrap();
        assert_eq!(transfer_to_host_sync::<u32>(&out, 1), [12], "the held write, then the later one");

        // A wait already satisfied holds nothing.
        let mut q = ClComputeQueue::new(&root);
        q.wait_for_event(&event, 1);
        write(&q, &f, &out, 3);
        q.flush().await_().unwrap();
        assert_eq!(transfer_to_host_sync::<u32>(&out, 1), [123]);
    }
}
