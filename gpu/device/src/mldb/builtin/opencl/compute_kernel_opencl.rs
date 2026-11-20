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
//!   over all platforms). A buffer is host memory the device reads in place (`CL_MEM_USE_HOST_PTR`). A driver
//!   that holds its own image of it (a discrete device's memory, or NVIDIA's on a GB10) is made current at each
//!   submission's edges by a map and unmap (`DeviceInner::synced`): the device's memory holding what it reads
//!   while it computes is not a copy (decision 0353).
//! - **Addresses:** with `RERAIL_OPENCL_USM=1`, where the device has Intel's unified shared memory, a buffer it
//!   allocates is a host allocation of it (`clHostMemAllocINTEL`), whose address kernels follow from a table
//!   (`ClBuffer::address`; the fused sorts and probes). It is bound and copied as itself
//!   (`clSetKernelArgMemPointerINTEL`, `clEnqueueMemcpyINTEL`): Intel's driver keeps an OpenCL buffer made over it
//!   apart from its pages. Memory wrapped in place has no address.
//! - **The SIMD group** is the device's sub-group where it has OpenCL 3.0's with 32 lanes (`native_subgroups`: Intel's
//!   GPUs), else emulated through threadgroup memory at 32 lanes (`ukl_opencl_shim.h`, Apple's OpenCL 1.2), or one
//!   lane (`RERAIL_OPENCL_SIMD_WIDTH=1`).
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
const CL_DEVICE_MAX_MEM_ALLOC_SIZE: ClUint = 0x1010;
const CL_DEVICE_EXTENSIONS: ClUint = 0x1030;
/// `cl_intel_required_subgroup_size`: the sub-group sizes a kernel may require (`size_t`s).
const CL_DEVICE_SUB_GROUP_SIZES_INTEL: ClUint = 0x4108;
/// `cl_intel_unified_shared_memory`: what host allocations support (a bit field; bit 0, access).
const CL_DEVICE_HOST_MEM_CAPABILITIES_INTEL: ClUint = 0x4190;
/// `cl_intel_unified_shared_memory`: the host allocations a launch reaches through addresses that buffers hold, not
/// its arguments (Metal's `useResource`).
const CL_KERNEL_EXEC_INFO_USM_PTRS_INTEL: ClUint = 0x4203;
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
const CL_MAP_READ: u64 = 1;
const CL_MAP_WRITE_INVALIDATE_REGION: u64 = 1 << 2;

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
    enqueue_read_buffer: unsafe extern "C" fn(Ptr, Ptr, ClUint, usize, usize, *mut c_void, ClUint, *const Ptr, *mut Ptr) -> ClInt,
    enqueue_copy_buffer: unsafe extern "C" fn(Ptr, Ptr, Ptr, usize, usize, usize, ClUint, *const Ptr, *mut Ptr) -> ClInt,
    enqueue_map_buffer: unsafe extern "C" fn(Ptr, Ptr, ClUint, u64, usize, usize, ClUint, *const Ptr, *mut Ptr, *mut ClInt) -> Ptr,
    enqueue_unmap_mem_object: unsafe extern "C" fn(Ptr, Ptr, Ptr, ClUint, *const Ptr, *mut Ptr) -> ClInt,
    enqueue_marker_with_wait_list: unsafe extern "C" fn(Ptr, ClUint, *const Ptr, *mut Ptr) -> ClInt,
    wait_for_events: unsafe extern "C" fn(ClUint, *const Ptr) -> ClInt,
    get_event_info: unsafe extern "C" fn(Ptr, ClUint, usize, Ptr, *mut usize) -> ClInt,
    get_event_profiling_info: unsafe extern "C" fn(Ptr, ClUint, usize, Ptr, *mut usize) -> ClInt,
    release_event: unsafe extern "C" fn(Ptr) -> ClInt,
    flush: unsafe extern "C" fn(Ptr) -> ClInt,
    finish: unsafe extern "C" fn(Ptr) -> ClInt,
    get_extension_function_address_for_platform: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    /// OpenCL 2.0's; absent from Apple's 1.2.
    set_kernel_exec_info: Option<unsafe extern "C" fn(Ptr, ClUint, usize, *const c_void) -> ClInt>,
}

/// `cl_intel_unified_shared_memory`'s entry points (the platform's extension functions): host allocations, which
/// have an address kernels follow.
#[allow(clippy::type_complexity)]
struct Usm {
    /// `clHostMemAllocINTEL(context, properties, size, alignment, errcode)`.
    host_mem_alloc: unsafe extern "C" fn(Ptr, *const u64, usize, ClUint, *mut ClInt) -> Ptr,
    /// `clMemBlockingFreeINTEL(context, ptr)`: frees once the device's work with it has completed.
    mem_blocking_free: unsafe extern "C" fn(Ptr, Ptr) -> ClInt,
    set_kernel_exec_info: unsafe extern "C" fn(Ptr, ClUint, usize, *const c_void) -> ClInt,
    /// `clSetKernelArgMemPointerINTEL(kernel, index, ptr)`: an argument that is a host allocation.
    set_kernel_arg_mem_pointer: unsafe extern "C" fn(Ptr, ClUint, *const c_void) -> ClInt,
    /// `clEnqueueMemcpyINTEL(queue, blocking, dst, src, size, waits, wait list, event)`.
    enqueue_memcpy: unsafe extern "C" fn(Ptr, ClUint, Ptr, *const c_void, usize, ClUint, *const Ptr, *mut Ptr) -> ClInt,
}

/// What a kernel's argument or a copy names of a buffer: an OpenCL buffer, or a host allocation's address (`Usm`),
/// as an integer, which a queue's operations carry to their enqueue.
#[derive(Clone, Copy)]
enum Mem {
    Cl(usize),
    Usm(usize),
}

/// Sets argument `i` of kernel `k` to the buffer `m`.
///
/// # Safety
/// `k` is a kernel of the device whose host allocations `usm` names, and `m` is alive.
unsafe fn set_buffer_arg(a: &Api, usm: Option<&Usm>, k: Ptr, i: ClUint, m: Mem) -> ClInt {
    match (m, usm) {
        (Mem::Cl(mem), _) => unsafe { (a.set_kernel_arg)(k, i, std::mem::size_of::<Ptr>(), (&(mem as Ptr) as *const Ptr).cast()) },
        (Mem::Usm(p), Some(u)) => unsafe { (u.set_kernel_arg_mem_pointer)(k, i, p as *const c_void) },
        // CL_INVALID_VALUE: a host allocation on a device without them.
        (Mem::Usm(_), None) => -30,
    }
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
            enqueue_read_buffer: f!("clEnqueueReadBuffer"),
            enqueue_copy_buffer: f!("clEnqueueCopyBuffer"),
            enqueue_map_buffer: f!("clEnqueueMapBuffer"),
            enqueue_unmap_mem_object: f!("clEnqueueUnmapMemObject"),
            enqueue_marker_with_wait_list: f!("clEnqueueMarkerWithWaitList"),
            wait_for_events: f!("clWaitForEvents"),
            get_event_info: f!("clGetEventInfo"),
            get_event_profiling_info: f!("clGetEventProfilingInfo"),
            release_event: f!("clReleaseEvent"),
            flush: f!("clFlush"),
            finish: f!("clFinish"),
            get_extension_function_address_for_platform: f!("clGetExtensionFunctionAddressForPlatform"),
            // SAFETY: OpenCL 2.0's function of that name, where the library has it.
            set_kernel_exec_info: sym("clSetKernelExecInfo").ok().map(|p| unsafe { std::mem::transmute::<Ptr, _>(p) }),
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

/// The SIMD groups' lanes of the OpenCL device's kernels: `RERAIL_OPENCL_SIMD_WIDTH` (1 or 32), else 32 (the device's
/// sub-groups or emulated, `native_subgroups`).
pub fn simd_width() -> u32 {
    match std::env::var("RERAIL_OPENCL_SIMD_WIDTH") {
        Err(_) => 32,
        Ok(w) => match w.parse::<u32>() {
            Ok(n @ (1 | 32)) => n,
            _ => panic!("RERAIL_OPENCL_SIMD_WIDTH={w}: 1 or 32"),
        },
    }
}

/// Whether the device's sub-groups are the SIMD groups its kernels want (decision 0440, the Intel plan's step 1):
/// OpenCL 3.0's sub-groups with their shuffles, relative shuffles and ballot (`cl_khr_subgroups`,
/// `_subgroup_shuffle`, `_subgroup_shuffle_relative`, `_subgroup_ballot`), and a kernel may require 32 lanes
/// (`cl_intel_required_subgroup_size`, 32 among its sizes). Otherwise, or with `RERAIL_OPENCL_SUBGROUPS=0` (to
/// measure one against the other), the shim emulates them in threadgroup memory.
fn native_subgroups(a: &Api, device: Ptr) -> bool {
    if std::env::var("RERAIL_OPENCL_SUBGROUPS").is_ok_and(|v| v == "0") || simd_width() != 32 {
        return false;
    }
    let ext = device_info_string(a, device, CL_DEVICE_EXTENSIONS);
    let has = |e: &str| ext.split_ascii_whitespace().any(|x| x == e);
    let wanted = ["cl_khr_subgroups", "cl_khr_subgroup_shuffle", "cl_khr_subgroup_shuffle_relative", "cl_khr_subgroup_ballot", "cl_intel_required_subgroup_size"];
    if !wanted.iter().all(|e| has(e)) {
        return false;
    }
    let sizes = device_info_bytes(a, device, CL_DEVICE_SUB_GROUP_SIZES_INTEL);
    sizes.chunks_exact(8).any(|c| u64::from_ne_bytes(c.try_into().unwrap()) == 32)
}

/// The device's host allocations, with `RERAIL_OPENCL_USM=1`, where it has Intel's unified shared memory with host
/// allocations it can access (`cl_intel_unified_shared_memory`, `CL_DEVICE_HOST_MEM_CAPABILITIES_INTEL`). Checked
/// again by a kernel at the device's opening (`follows_addresses`). Not the default: on the Arc 140V under WSL2 the
/// radix sort's wall is 3-5 times the heap buffers' from 20,000 rows, its kernels' own time unchanged (decision
/// 0440, the Intel plan's step 2).
fn usm(a: &Api, platform: Ptr, device: Ptr) -> Option<Usm> {
    if !std::env::var("RERAIL_OPENCL_USM").is_ok_and(|v| v == "1") {
        return None;
    }
    let ext = device_info_string(a, device, CL_DEVICE_EXTENSIONS);
    if !ext.split_ascii_whitespace().any(|x| x == "cl_intel_unified_shared_memory") {
        return None;
    }
    let caps = device_info_bytes(a, device, CL_DEVICE_HOST_MEM_CAPABILITIES_INTEL);
    if caps.len() < 8 || u64::from_ne_bytes(caps[..8].try_into().unwrap()) & 1 == 0 {
        return None;
    }
    let f = |name: &str| {
        let c = CString::new(name).unwrap();
        let p = unsafe { (a.get_extension_function_address_for_platform)(platform, c.as_ptr()) };
        (!p.is_null()).then_some(p)
    };
    // SAFETY: each is the extension's function of that name, with the signature its field declares.
    Some(Usm {
        host_mem_alloc: unsafe { std::mem::transmute::<Ptr, _>(f("clHostMemAllocINTEL")?) },
        mem_blocking_free: unsafe { std::mem::transmute::<Ptr, _>(f("clMemBlockingFreeINTEL")?) },
        set_kernel_exec_info: a.set_kernel_exec_info?,
        set_kernel_arg_mem_pointer: unsafe { std::mem::transmute::<Ptr, _>(f("clSetKernelArgMemPointerINTEL")?) },
        enqueue_memcpy: unsafe { std::mem::transmute::<Ptr, _>(f("clEnqueueMemcpyINTEL")?) },
    })
}

struct DeviceInner {
    device: Ptr,
    context: Ptr,
    name: String,
    max_group: usize,
    /// Whether its kernels' SIMD groups are the device's sub-groups (`native_subgroups`), else emulated.
    subgroups: bool,
    /// The alignment of a sub-buffer's origin, in bytes (`CL_DEVICE_MEM_BASE_ADDR_ALIGN`, given in bits).
    base_align: u64,
    /// The largest buffer (`CL_DEVICE_MAX_MEM_ALLOC_SIZE`).
    max_alloc: u64,
    /// Where its buffers are host allocations with an address (`usm`).
    usm: Option<Usm>,
    /// Whether the driver holds its own image of a buffer over host pages (device memory, or NVIDIA's on a GB10,
    /// whose memory is the host's), which a map and unmap make current: then a submission's buffers are made the
    /// host's at its start and the device's back into the host's at its end (`ClComputeQueue::flush`).
    synced: bool,
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
            gpus.extend(ds.into_iter().map(|d| (p, d)));
        }
        let k: usize = std::env::var("RERAIL_OPENCL_DEVICE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let (platform, device) = *gpus.get(k).ok_or_else(|| format!("no OpenCL GPU {k} ({} found)", gpus.len()))?;
        let name = format!("{} ({})", device_info_string(a, device, CL_DEVICE_NAME), device_info_string(a, device, CL_DEVICE_VERSION));
        let unified = device_info_bytes(a, device, CL_DEVICE_HOST_UNIFIED_MEMORY);
        let unified = unified.len() >= 4 && u32::from_ne_bytes(unified[..4].try_into().unwrap()) != 0;
        let mg = device_info_bytes(a, device, CL_DEVICE_MAX_WORK_GROUP_SIZE);
        let max_group = usize::from_ne_bytes(mg[..8].try_into().unwrap());
        let mut e = 0;
        let context = unsafe { (a.create_context)(std::ptr::null_mut(), 1, &device, std::ptr::null_mut(), std::ptr::null_mut(), &mut e) };
        check("clCreateContext", e)?;
        let ba = device_info_bytes(a, device, CL_DEVICE_MEM_BASE_ADDR_ALIGN);
        let base_align = (u32::from_ne_bytes(ba[..4].try_into().unwrap()) as u64 / 8).max(1);
        let ma = device_info_bytes(a, device, CL_DEVICE_MAX_MEM_ALLOC_SIZE);
        let max_alloc = u64::from_ne_bytes(ma[..8].try_into().unwrap());
        let subgroups = native_subgroups(a, device);
        // Heap buffers first: memory wrapped in place (a block, a mapped tape) is read through one.
        let probe = ClDevice(Arc::new(DeviceInner { device, context, name: name.clone(), max_group, subgroups, base_align, max_alloc, usm: None, synced: false }));
        // A driver whose kernels' writes reach the host's pages only through a map is made current at each
        // submission's edges (`synced`); a device's own memory holding a buffer while it computes is not a copy (0353).
        // `RERAIL_OPENCL_SYNCED=1` takes this path on any device (its tests on a driver that needs no map).
        let synced = !unified || std::env::var_os("RERAIL_OPENCL_SYNCED").is_some() || probe.reads_host_pages_in_place().is_err();
        drop(probe);
        let d = ClDevice(Arc::new(DeviceInner { device, context, name: name.clone(), max_group, subgroups, base_align, max_alloc, usm: None, synced }));
        if synced {
            return Ok(d);
        }
        let Some(usm) = usm(a, platform, device) else { return Ok(d) };
        let with = ClDevice(Arc::new(DeviceInner { device, context, name, max_group, subgroups, base_align, max_alloc, usm: Some(usm), synced }));
        // Without the check's pass its buffers stay the heap's, with no address (one context; nothing releases it).
        if let Err(e) = with.follows_addresses() {
            eprintln!("rerail-device: {e}: OpenCL buffers without addresses");
            return Ok(d);
        }
        Ok(with)
    }

    /// Whether a kernel's write into a buffer over host pages is in those pages once it completes, with no map: a
    /// driver may say it shares the host's memory and still keep its own image of a `CL_MEM_USE_HOST_PTR` buffer,
    /// made current only by a map (NVIDIA's on a GB10, whose memory is the host's). Such a device is `synced`.
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
            check("clSetKernelArg", unsafe { set_buffer_arg(a, self.0.usm.as_ref(), k.0, 0, out.mem()) })?;
            let one = 1usize;
            check("the in-place probe", unsafe { (a.enqueue_nd_range_kernel)(q, k.0, 1, std::ptr::null(), &one, &one, 0, std::ptr::null(), std::ptr::null_mut()) })?;
            check("clFinish", unsafe { (a.finish)(q) })
        })();
        unsafe { (a.release_command_queue)(q) };
        r?;
        // SAFETY: the buffer's host pages, which no kernel uses now.
        let seen = unsafe { std::ptr::read_volatile(out.0.ptr.cast::<u32>()) };
        if seen != SEEN {
            return Err(format!("OpenCL device {}: a kernel's write is not in the host's pages without a map", self.0.name));
        }
        Ok(())
    }

    /// Whether a kernel follows a host allocation's address that a buffer holds, and its writes there are in the
    /// host's pages: a table in one allocation names a second, the kernel writes through it. A driver whose buffer
    /// over a host allocation is not that allocation (another address, or a copy) fails it.
    fn follows_addresses(&self) -> Result<(), String> {
        const SEEN: u32 = 0x0440_05E1;
        let lib = ClComputeFunctionLibrary::compile_from_source(
            self,
            "__kernel void ukl_follows(__global const ulong* t) { __global uint* p = (__global uint*)t[0]; p[3] = 0x044005E1u; }",
            "the address probe",
        )?;
        let a = api()?;
        let k = lib.kernel("ukl_follows")?;
        let u = self.0.usm.as_ref().ok_or("no host allocations")?;
        let table = alloc_uninitialized_array_sync::<u64>(self, 4);
        let out = alloc_uninitialized_array_sync::<u32>(self, 4);
        let at = out.address().ok_or("a host allocation without an address")?;
        check("clSetKernelExecInfo", unsafe { (u.set_kernel_exec_info)(k.0, CL_KERNEL_EXEC_INFO_USM_PTRS_INTEL, 8, (&at as *const u64).cast()) })?;
        // SAFETY: the table's host pages, which no kernel uses yet.
        unsafe { table.contents().cast::<u64>().write_volatile(at) };
        let mut e = 0;
        let q = unsafe { (a.create_command_queue)(self.0.context, self.0.device, 0, &mut e) };
        check("clCreateCommandQueue", e)?;
        let r = (|| {
            check("clSetKernelArg", unsafe { set_buffer_arg(a, self.0.usm.as_ref(), k.0, 0, table.mem()) })?;
            let one = 1usize;
            check("the address probe", unsafe { (a.enqueue_nd_range_kernel)(q, k.0, 1, std::ptr::null(), &one, &one, 0, std::ptr::null(), std::ptr::null_mut()) })?;
            check("clFinish", unsafe { (a.finish)(q) })
        })();
        unsafe { (a.release_command_queue)(q) };
        r?;
        // SAFETY: the buffer's host pages, which no kernel uses now.
        let seen = unsafe { std::ptr::read_volatile(out.contents().cast::<u32>().add(3)) };
        if seen != SEEN {
            return Err(format!("OpenCL device {}: a kernel's write through a host allocation's address is not in its pages", self.0.name));
        }
        Ok(())
    }

    /// Whether its buffers made here (not memory wrapped in place) have an address kernels follow: host
    /// allocations of Intel's unified shared memory (`usm`), checked by a kernel at the opening.
    pub fn allocations_have_addresses(&self) -> bool {
        self.0.usm.is_some()
    }

    pub fn name(&self) -> &str {
        &self.0.name
    }

    /// The largest buffer the device makes.
    pub fn max_alloc(&self) -> u64 {
        self.0.max_alloc
    }

    /// The largest threadgroup, and so the emulated SIMD group's scratch (`UKL_TG_MAX`).
    pub fn max_group(&self) -> usize {
        self.0.max_group.min(1024)
    }

    /// The defines its kernels' preprocessing takes: the SIMD width, the largest threadgroup, and whether the SIMD
    /// groups are the device's sub-groups (`UKL_OPENCL_SUBGROUPS`, `native_subgroups`).
    pub fn defines(&self) -> Vec<(&'static str, String)> {
        let mut d = vec![("UKL_SIMD_WIDTH", format!("{}u", simd_width())), ("UKL_TG_MAX", format!("{}u", self.max_group()))];
        if self.0.subgroups {
            d.push(("UKL_OPENCL_SUBGROUPS", "1".into()));
        }
        d
    }

    /// Whether its kernels' SIMD groups are the device's sub-groups (`native_subgroups`), not emulated.
    pub fn subgroups(&self) -> bool {
        self.0.subgroups
    }

    pub fn new_shared_event(&self) -> ClSharedEvent {
        ClSharedEvent(Arc::new(Mutex::new(Signal { value: 0, waiting: Vec::new() })))
    }
}

/// Whose a buffer's host pages are, and so how they are freed.
enum Owner {
    /// The process's heap.
    Heap(std::alloc::Layout),
    /// A host allocation of the device's unified shared memory (`usm`), whose address kernels follow.
    Usm(ClDevice),
    /// Memory wrapped in place (`wrap_host_memory`), its owner's; or a sub-buffer's, its buffer's.
    Borrowed,
}

/// A buffer's memory: host pages, and the OpenCL buffer made over them.
struct Allocation {
    ptr: *mut u8,
    len: u64,
    owner: Owner,
    /// Whether kernels follow `ptr` as an address: a host allocation's, or a region of one.
    addressed: bool,
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
            if !self.mem.is_null() {
                unsafe { (a.release_mem_object)(self.mem) };
            }
        }
        match &self.owner {
            Owner::Heap(layout) => unsafe { std::alloc::dealloc(self.ptr, *layout) },
            Owner::Usm(d) => {
                if let Some(u) = &d.0.usm {
                    unsafe { (u.mem_blocking_free)(d.0.context, self.ptr.cast()) };
                }
            }
            Owner::Borrowed => {}
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

    /// The address kernels follow to its first byte: a host allocation's (`ClDevice::allocations_have_addresses`),
    /// else none (memory wrapped in place, or a device without unified shared memory).
    pub fn address(&self) -> Option<u64> {
        self.0.addressed.then_some(self.0.ptr as u64)
    }

    /// What arguments and copies name of it: a host allocation (or a region of one) has no OpenCL buffer.
    fn mem(&self) -> Mem {
        if self.0.mem.is_null() { Mem::Usm(self.0.ptr as usize) } else { Mem::Cl(self.0.mem as usize) }
    }
}

/// `allocUninitializedArraySync`: memory for `length` elements (at least 16 bytes), zeroed, page-aligned and a whole
/// number of cache lines: what drivers ask of host memory they use in place (Intel's: 4,096-aligned, a multiple of
/// 64 bytes). A host write after a buffer's first use is seen by the next launch (the armed prediction's test). On a
/// device with unified shared memory it is a host allocation of it, whose address kernels follow (`ClBuffer::address`);
/// the buffer over it is that allocation, not a copy (`ClDevice::follows_addresses`).
pub fn alloc_uninitialized_array_sync<T>(device: &ClDevice, length: usize) -> ClBuffer {
    let len = (length * std::mem::size_of::<T>()).max(16);
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as usize;
    let layout = std::alloc::Layout::from_size_align(len.next_multiple_of(64), page).unwrap();
    let (ptr, owner) = match &device.0.usm {
        Some(u) => {
            let mut e = 0;
            let p = unsafe { (u.host_mem_alloc)(device.0.context, std::ptr::null(), layout.size(), page as ClUint, &mut e) };
            assert!(e == CL_SUCCESS && !p.is_null(), "OpenCL: clHostMemAllocINTEL of {len} bytes failed ({e})");
            // SAFETY: the allocation's `layout.size()` bytes, no other user yet.
            unsafe { std::ptr::write_bytes(p.cast::<u8>(), 0, layout.size()) };
            (p.cast::<u8>(), Owner::Usm(device.clone()))
        }
        None => {
            // SAFETY: a non-zero size.
            let p = unsafe { std::alloc::alloc_zeroed(layout) };
            assert!(!p.is_null(), "out of memory for a {len}-byte buffer");
            (p, Owner::Heap(layout))
        }
    };
    let addressed = matches!(owner, Owner::Usm(_));
    // A host allocation is bound and copied as itself: an OpenCL buffer over it would be a copy (Intel's driver
    // keeps one apart from its pages).
    let mut held = Allocation { ptr, len: len as u64, owner, addressed, mem: std::ptr::null_mut(), _parent: None };
    if !addressed {
        let a = api().expect("an OpenCL device without its library");
        let mut e = 0;
        held.mem = unsafe { (a.create_buffer)(device.0.context, CL_MEM_READ_WRITE | CL_MEM_USE_HOST_PTR, layout.size(), ptr.cast(), &mut e) };
        if e != CL_SUCCESS {
            held.mem = std::ptr::null_mut();
            drop(held);
            panic!("OpenCL: clCreateBuffer of {len} bytes failed ({e})");
        }
    }
    ClBuffer(Arc::new(held))
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
    Ok(ClBuffer(Arc::new(Allocation { ptr, len: len as u64, owner: Owner::Borrowed, addressed: false, mem, _parent: None })))
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
        // OpenCL C 3.0 where the kernels use the device's sub-groups (its `sub_group_*` functions are 3.0's), else 1.2.
        let options_text = if device.subgroups() { "-cl-std=CL3.0" } else { "-cl-std=CL1.2" };
        let options = CString::new(options_text).unwrap();
        // The store's binary (`crate::store`), keyed by the device's name, version and driver beside the source.
        let driver = device_info_string(a, device.0.device, CL_DRIVER_VERSION);
        let key = crate::store::key(&[b"opencl", device.name().as_bytes(), driver.as_bytes(), options_text.as_bytes(), preprocessed_source.as_bytes()]);
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
            check("clSetKernelArg", unsafe { set_buffer_arg(a, device.0.usm.as_ref(), k.0, 0, out.mem()) })?;
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
    device: ClDevice,
    /// The fill kernel (`FILL`) and its library, compiled at the root's first fill.
    fill: Option<Result<(Arc<ClComputeFunctionLibrary>, Arc<Mutex<Kernel>>), String>>,
}

/// A fill's kernel (`ClComputeQueue::enqueue_fill_array`), OpenCL C (the device layer is below the UKL
/// preprocessor): word `first + i` of `out` is the pattern's word `i % period`, for `i < count`.
const FILL: &str = r#"
__kernel void ukl_fill_words(__global uint* out, uint first, uint count, uint period,
                             uint w0, uint w1, uint w2, uint w3, uint w4, uint w5, uint w6, uint w7) {
    uint i = get_global_id(0);
    if (i >= count)
        return;
    uint k = i % period;
    uint w = k == 0u ? w0 : k == 1u ? w1 : k == 2u ? w2 : k == 3u ? w3 : k == 4u ? w4 : k == 5u ? w5 : k == 6u ? w6 : w7;
    out[first + i] = w;
}
"#;

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
        Ok(ClQueueRoot(Arc::new(Mutex::new(RootState { q, backlog: Default::default(), device: device.clone(), fill: None }))))
    }

    /// The fill kernel (`FILL`), compiled at the first fill.
    fn fill(&self) -> Result<Arc<Mutex<Kernel>>, String> {
        let mut root = self.0.lock().unwrap();
        let device = root.device.clone();
        root.fill
            .get_or_insert_with(|| {
                let lib = ClComputeFunctionLibrary::compile_from_source(&device, FILL, "the fill kernel")?;
                let kernel = lib.kernel("ukl_fill_words")?;
                Ok((Arc::new(lib), Arc::new(Mutex::new(kernel))))
            })
            .clone()
            .map(|(_, kernel)| kernel)
    }

    /// Enqueues the backlog in order, up to a wait for a host-signalled event that has not reached its value: there
    /// it stops, and the event's signal drives the root again.
    fn drive(&self) {
        let a = api().expect("an OpenCL queue without its library");
        let mut root = self.0.lock().unwrap();
        let q = root.q;
        let device = root.device.clone();
        while let Some(p) = root.backlog.front_mut() {
            while let Some(op) = p.ops.pop_front() {
                match op {
                    Op::Run(f) => {
                        if p.error.is_none() {
                            let mut c = Enqueue { a, usm: device.0.usm.as_ref(), q, kernels: Vec::new() };
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
    /// The device's host allocations' entry points (`usm`).
    usm: Option<&'a Usm>,
    q: Ptr,
    kernels: Vec<Launched>,
}

/// A command of a submission (a kernel's launch, a copy, a fill): its event (its device time, and its status when
/// the work fails) and what it is.
struct Launched {
    event: Event,
    name: String,
}

/// An operation of a queue: work to enqueue, or a wait for a host-signalled event that holds what follows.
enum Op {
    Run(Box<dyn FnOnce(&mut Enqueue) -> Result<(), String> + Send>),
    Wait(ClSharedEvent, u64),
}

/// A flush's work not yet enqueued, with what its event will hold.
impl Enqueue<'_> {
    /// A command `enqueue` makes, its event kept as `what` (a failed submission names the command that failed).
    fn command(&mut self, what: String, call: &str, enqueue: impl FnOnce(&Api, Ptr, *mut Ptr) -> ClInt) -> Result<(), String> {
        let mut ev = std::ptr::null_mut();
        check(call, enqueue(self.a, self.q, &mut ev))?;
        if !ev.is_null() {
            self.kernels.push(Launched { event: Event(ev), name: what });
        }
        Ok(())
    }

    /// A map and unmap of each of `buffers` (an OpenCL buffer and its length), in the queue's order: with
    /// `CL_MAP_WRITE_INVALIDATE_REGION` the driver's image takes the host's pages, with `CL_MAP_READ` they take it.
    fn sync(&mut self, buffers: &[(usize, usize)], flags: u64) -> Result<(), String> {
        for &(m, n) in buffers {
            let mut e = 0;
            let p = unsafe { (self.a.enqueue_map_buffer)(self.q, m as Ptr, CL_FALSE, flags, 0, n, 0, std::ptr::null(), std::ptr::null_mut(), &mut e) };
            check("clEnqueueMapBuffer", e)?;
            self.command(format!("a sync of {n} bytes"), "clEnqueueUnmapMemObject", |a, q, ev| unsafe { (a.enqueue_unmap_mem_object)(q, m as Ptr, p, 0, std::ptr::null(), ev) })?;
        }
        Ok(())
    }

    /// A copy of `n` bytes at `src` to `dst`, one a host allocation, in the queue's order.
    fn memcpy(&self, dst: usize, src: usize, n: usize) -> Result<(), String> {
        let u = self.usm.ok_or("OpenCL: a host allocation's copy on a device without them")?;
        check("clEnqueueMemcpyINTEL", unsafe { (u.enqueue_memcpy)(self.q, CL_FALSE, dst as Ptr, src as *const c_void, n, 0, std::ptr::null(), std::ptr::null_mut()) })
    }
}

struct Pending {
    ops: std::collections::VecDeque<Op>,
    kernels: Vec<Launched>,
    error: Option<String>,
    done: Arc<Done>,
}

/// A flush's state: whether its work is enqueued yet (and then its marker, its kernels' events, its first error),
/// and the buffers and bytes it uses, kept until it has completed.
#[derive(Default)]
struct DoneState {
    enqueued: bool,
    marker: Option<Event>,
    kernels: Vec<Launched>,
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
        if let (Some(m), Ok(a)) = (&d.marker, api())
            && unsafe { (a.wait_for_events)(1, &m.0) } != CL_SUCCESS
        {
            // Work no one awaited failed: said here, as no caller will see it (the commands after it then fail).
            let what = failed_kernel(a, &d.kernels).unwrap_or_else(|| format!("none of its {} commands says it failed", d.kernels.len()));
            eprintln!("rerail: OpenCL work dropped before it was awaited failed: {what}");
        }
    }
}

/// A queue of operations (`MetalComputeQueue`): launches, copies and fills, enqueued in order on its root's command
/// queue at a flush. The buffers and bytes they use are kept until the flush's work has completed.
pub struct ClComputeQueue {
    root: ClQueueRoot,
    ops: RefCell<Vec<Op>>,
    keep: RefCell<Vec<Box<dyn std::any::Any + Send>>>,
    /// The OpenCL buffers its work names, with their lengths: what a `synced` device's map makes current.
    touched: RefCell<Vec<(usize, usize)>>,
}

impl ClComputeQueue {
    pub fn new(root: &ClQueueRoot) -> Self {
        ClComputeQueue { root: root.clone(), ops: Default::default(), keep: Default::default(), touched: Default::default() }
    }

    pub fn new_bind_context<'a>(&'a self, function: &'a ClComputeFunction) -> ClBindContext<'a> {
        // The rows' parameters, then the indirect launch's counts and their first word (`GEN_OUTER_PARAMETER_GROUPS`:
        // none, a direct launch).
        let mut params = vec![None; function.param_sizes.len() + 2];
        let n = function.param_sizes.len();
        (params[n], params[n + 1]) = (Some(Param::Null), Some(Param::Bytes(0u32.to_ne_bytes().to_vec())));
        ClBindContext { queue: self, function, params, used: Vec::new() }
    }

    fn push(&self, op: impl FnOnce(&mut Enqueue) -> Result<(), String> + Send + 'static) {
        self.ops.borrow_mut().push(Op::Run(Box::new(op)));
    }

    fn keep(&self, v: impl std::any::Any + Send) {
        self.keep.borrow_mut().push(Box::new(v));
    }

    /// `keep`s a buffer the work names, and notes it for a `synced` device's maps.
    fn keep_buffer(&self, b: &ClBuffer) {
        if let Mem::Cl(m) = b.mem() {
            self.touched.borrow_mut().push((m, b.length() as usize));
        }
        self.keep(b.clone());
    }

    /// `enqueueCopyFromHost`: the bytes are taken now, and copied into `dst` at `dst_offset` in queue order.
    pub fn enqueue_copy_from_host(&self, dst: &ClBuffer, dst_offset: u64, bytes: &[u8]) {
        assert!(dst_offset + bytes.len() as u64 <= dst.length(), "a copy past its buffer's end");
        let bytes = Arc::new(bytes.to_vec());
        let (d, b) = (dst.mem(), bytes.clone());
        self.keep_buffer(dst);
        self.keep(bytes);
        self.push(move |c| match d {
            Mem::Cl(d) => c.command(format!("a write of {} bytes from the host", b.len()), "clEnqueueWriteBuffer", |a, q, ev| unsafe {
                (a.enqueue_write_buffer)(q, d as Ptr, CL_FALSE, dst_offset as usize, b.len(), b.as_ptr().cast(), 0, std::ptr::null(), ev)
            }),
            Mem::Usm(d) => c.memcpy(d + dst_offset as usize, b.as_ptr() as usize, b.len()),
        });
    }

    /// `enqueueFillArray`: `count` copies of `value` into `dst` from element `offset` (all of it when `None`).
    ///
    /// A kernel writes them (`FILL`), never `clEnqueueFillBuffer`: Apple's leaves what was there, or garbage, in a
    /// buffer over host pages that a kernel then reads (`a_fill_then_a_kernel_are_in_the_host_pages`). A pattern
    /// of 1 or 2 bytes is widened to a word; one of no whole words, or of more than 8, is written from the host.
    pub fn enqueue_fill_array<T: crate::Pod>(&self, dst: &ClBuffer, value: &T, offset: usize, count: Option<usize>) {
        let size = std::mem::size_of::<T>();
        let count = count.unwrap_or(dst.length() as usize / size - offset);
        if count == 0 {
            return;
        }
        let one = unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size) }.to_vec();
        let words: Option<Vec<u32>> = match size {
            1 => Some(vec![u32::from_ne_bytes([one[0]; 4])]),
            2 => Some(vec![u32::from_ne_bytes([one[0], one[1], one[0], one[1]])]),
            _ if size % 4 == 0 && size <= 32 => Some(one.chunks(4).map(|w| u32::from_ne_bytes(w.try_into().unwrap())).collect()),
            _ => None,
        };
        let (start, bytes) = (offset * size, count * size);
        let whole = start % 4 == 0 && bytes % 4 == 0 && u32::try_from((start + bytes) / 4).is_ok();
        match words.filter(|_| whole) {
            Some(words) => {
                let kernel = self.root.fill().unwrap_or_else(|e| panic!("OpenCL: the fill kernel: {e}"));
                let mut w = [0u32; 8];
                w[..words.len()].copy_from_slice(&words);
                let mut args = vec![(start / 4) as u32, (bytes / 4) as u32, words.len() as u32];
                args.extend_from_slice(&w);
                let d = dst.mem();
                self.keep_buffer(dst);
                self.push(move |c| {
                    let k = kernel.lock().unwrap();
                    check("clSetKernelArg", unsafe { set_buffer_arg(c.a, c.usm, k.0, 0, d) })?;
                    for (i, v) in args.iter().enumerate() {
                        check("clSetKernelArg", unsafe { (c.a.set_kernel_arg)(k.0, 1 + i as ClUint, 4, (v as *const u32).cast()) })?;
                    }
                    let global = args[1] as usize;
                    c.command(format!("the fill kernel ({} words)", args[1]), "the fill kernel", |a, q, ev| unsafe {
                        (a.enqueue_nd_range_kernel)(q, k.0, 1, std::ptr::null(), &global, std::ptr::null(), 0, std::ptr::null(), ev)
                    })
                });
            }
            None => {
                let bytes: Vec<u8> = one.iter().copied().cycle().take(count * size).collect();
                self.enqueue_copy_from_host(dst, start as u64, &bytes);
            }
        }
    }

    /// `enqueueCopyBetweenDeviceRegions`.
    pub fn enqueue_copy_between_device_regions(&self, src: &ClBuffer, dst: &ClBuffer, src_offset: u64, dst_offset: u64, length: u64) {
        assert!(src_offset + length <= src.length() && dst_offset + length <= dst.length(), "a copy past its buffer's end");
        if length == 0 {
            return;
        }
        let (s, d) = (src.mem(), dst.mem());
        self.keep_buffer(src);
        self.keep_buffer(dst);
        let (so, dof, n) = (src_offset as usize, dst_offset as usize, length as usize);
        self.push(move |c| match (s, d) {
            (Mem::Cl(s), Mem::Cl(d)) => c.command(format!("a copy of {n} bytes"), "clEnqueueCopyBuffer", |a, q, ev| unsafe {
                (a.enqueue_copy_buffer)(q, s as Ptr, d as Ptr, so, dof, n, 0, std::ptr::null(), ev)
            }),
            (Mem::Usm(s), Mem::Usm(d)) => c.memcpy(d + dof, s + so, n),
            // A host allocation's pages are host memory: the buffer's own write or read of them, in the queue's order.
            (Mem::Usm(s), Mem::Cl(d)) => c.command(format!("a write of {n} bytes from a host allocation"), "clEnqueueWriteBuffer", |a, q, ev| unsafe {
                (a.enqueue_write_buffer)(q, d as Ptr, CL_FALSE, dof, n, (s + so) as *const c_void, 0, std::ptr::null(), ev)
            }),
            (Mem::Cl(s), Mem::Usm(d)) => c.command(format!("a read of {n} bytes into a host allocation"), "clEnqueueReadBuffer", |a, q, ev| unsafe {
                (a.enqueue_read_buffer)(q, s as Ptr, CL_FALSE, so, n, (d + dof) as *mut c_void, 0, std::ptr::null(), ev)
            }),
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
        let mut ops: std::collections::VecDeque<Op> = std::mem::take(&mut *self.ops.borrow_mut()).into();
        let mut touched = std::mem::take(&mut *self.touched.borrow_mut());
        if self.root.0.lock().unwrap().device.0.synced && !touched.is_empty() {
            touched.sort_unstable();
            touched.dedup();
            // The host's pages into the driver's image first (a host write since the last submission), and the
            // device's writes back into them last, as the work's last commands, before its marker.
            let first = touched.clone();
            ops.push_front(Op::Run(Box::new(move |c| c.sync(&first, CL_MAP_WRITE_INVALIDATE_REGION))));
            ops.push_back(Op::Run(Box::new(move |c| c.sync(&touched, CL_MAP_READ))));
        }
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

fn kernel_seconds(kernels: &[Launched]) -> f64 {
    let Ok(a) = api() else { return 0.0 };
    let at = |e: &Event, p| {
        let mut t = 0u64;
        unsafe { (a.get_event_profiling_info)(e.0, p, 8, (&mut t as *mut u64).cast(), std::ptr::null_mut()) };
        t
    };
    kernels.iter().map(|k| at(&k.event, CL_PROFILING_COMMAND_END).saturating_sub(at(&k.event, CL_PROFILING_COMMAND_START))).sum::<u64>() as f64 * 1e-9
}

/// The first of `kernels` whose command failed, with its status: the command a failed submission names.
fn failed_kernel(a: &Api, kernels: &[Launched]) -> Option<String> {
    kernels.iter().find_map(|k| {
        let mut status: ClInt = CL_COMPLETE;
        unsafe { (a.get_event_info)(k.event.0, CL_EVENT_COMMAND_EXECUTION_STATUS, 4, (&mut status as *mut ClInt).cast(), std::ptr::null_mut()) };
        (status < 0).then(|| format!("{} failed ({status})", k.name))
    })
}

impl ClComputeEvent {
    /// Waits for the work to complete; its first error, if it had one.
    pub fn await_(&self) -> Result<(), String> {
        let a = api()?;
        let mut d = self.done.state.lock().unwrap();
        while !d.enqueued {
            d = self.done.cv.wait(d).unwrap();
        }
        if let Some(m) = &d.marker
            && let Err(e) = check("clWaitForEvents", unsafe { (a.wait_for_events)(1, &m.0) })
        {
            // The command that failed, of the submission's: what the error names.
            return Err(match failed_kernel(a, &d.kernels) {
                Some(k) => format!("{e}: {k}"),
                None => format!("{e}: none of its {} commands says it failed", d.kernels.len()),
            });
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
    /// The host allocations the launch reaches through addresses buffers hold (`use_buffer`).
    used: Vec<u64>,
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
        let mem = if buffer.0.mem.is_null() {
            // A region of a host allocation is its address there.
            std::ptr::null_mut()
        } else {
            let a = api()?;
            let region = [offset as usize, (buffer.length() - offset) as usize];
            let mut e = 0;
            let mem = unsafe { (a.create_sub_buffer)(buffer.0.mem, CL_MEM_READ_WRITE, CL_BUFFER_CREATE_TYPE_REGION, region.as_ptr().cast(), &mut e) };
            check("clCreateSubBuffer", e)?;
            mem
        };
        // SAFETY: inside the buffer, which the region keeps.
        let ptr = unsafe { buffer.0.ptr.add(offset as usize) };
        let sub = ClBuffer(Arc::new(Allocation { ptr, len: buffer.length() - offset, owner: Owner::Borrowed, addressed: buffer.0.addressed, mem, _parent: Some(buffer.clone()) }));
        self.set_buffer(arg_num, &sub);
        Ok(())
    }

    pub fn set_thread_group_memory(&mut self, arg_num: u64, n_bytes: u64) {
        assert!(self.function.local[arg_num as usize], "{}: argument {arg_num} is no threadgroup memory", self.function.name);
        self.params[arg_num as usize] = Some(Param::Local((n_bytes as usize).max(16)));
    }

    /// A buffer the launch reaches through an address another holds: kept until the work completes, and a host
    /// allocation named to the driver (`CL_KERNEL_EXEC_INFO_USM_PTRS_INTEL`), which makes it the launch's.
    pub fn use_buffer(&mut self, buffer: &ClBuffer) {
        self.queue.keep_buffer(buffer);
        self.used.extend(buffer.address());
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
                self.queue.keep_buffer(b);
            }
        }
        let global = [grid[0] * block[0], grid[1] * block[1], grid[2] * block[2]];
        let (kernel, name) = (f.kernel.clone(), f.name.clone());
        let used = std::mem::take(&mut self.used);
        self.queue.push(move |c| {
            let k = kernel.lock().unwrap();
            // The list is the kernel's until the next is set: a kernel that follows addresses names them at each
            // launch (the fused sorts' and probes').
            if let (Some(u), false) = (c.usm, used.is_empty()) {
                check(&format!("{name}: its host allocations"), unsafe {
                    (u.set_kernel_exec_info)(k.0, CL_KERNEL_EXEC_INFO_USM_PTRS_INTEL, 8 * used.len(), used.as_ptr().cast())
                })?;
            }
            for (i, p) in params.iter().enumerate() {
                let e = match p {
                    Param::Bytes(b) => unsafe { (c.a.set_kernel_arg)(k.0, i as u32, b.len(), b.as_ptr().cast()) },
                    Param::Buffer(b) => unsafe { set_buffer_arg(c.a, c.usm, k.0, i as u32, b.mem()) },
                    Param::Local(n) => unsafe { (c.a.set_kernel_arg)(k.0, i as u32, *n, std::ptr::null()) },
                    Param::Null => unsafe { (c.a.set_kernel_arg)(k.0, i as u32, std::mem::size_of::<Ptr>(), (&std::ptr::null_mut::<c_void>() as *const Ptr).cast()) },
                };
                check(&format!("{name}: setting argument {i}"), e)?;
            }
            let mut ev = std::ptr::null_mut();
            check(&format!("{name}: launch"), unsafe {
                (c.a.enqueue_nd_range_kernel)(c.q, k.0, 3, std::ptr::null(), global.as_ptr(), block.as_ptr(), 0, std::ptr::null(), &mut ev)
            })?;
            c.kernels.push(Launched { event: Event(ev), name: name.clone() });
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

DEFINE_KERNEL2(t, t_count,
               ARGS,     Args,     args, ,
               RWBUFFER, uint32_t, out,  "[1]",
               GID0,     uint32_t, i,    )
{
    if (i < args.value)
        ukl_atomic_add(&out[0], 1u);
}
"#;

    fn write(q: &ClComputeQueue, f: &ClComputeFunction, out: &ClBuffer, value: u32) {
        let mut ctx = q.new_bind_context(f);
        ctx.set_primitive(0, &value.to_ne_bytes());
        ctx.set_buffer(1, out);
        ctx.launch(&[1], &[32]).unwrap();
    }

    /// A fill and the kernel after it, in one flush, are in the buffer's host pages once it completes, read with no
    /// map: the fill zeroes what the host wrote before, then the kernel's write lands on the zero.
    #[test]
    fn a_fill_then_a_kernel_are_in_the_host_pages() {
        let Some((device, lib)) = compile("fill.uglykl", KERNELS) else { return };
        let f = lib.get_function(&device, "t_write").unwrap();
        let root = ClQueueRoot::new(&device).unwrap();
        let out = alloc_uninitialized_array_sync::<u32>(&device, 1);
        // SAFETY: the buffer's host pages, which no work uses yet.
        unsafe { out.contents().cast::<u32>().write(7) };
        let mut q = ClComputeQueue::new(&root);
        q.enqueue_fill_array(&out, &0u32, 0, Some(1));
        write(&q, &f, &out, 5);
        q.flush().await_().unwrap();
        // SAFETY: the work completed.
        assert_eq!(unsafe { std::ptr::read_volatile(out.contents().cast::<u32>()) }, 5, "zeroed, then 0 * 10 + 5");
        // An atomic count after a fill, as a kernel appending its hits counts them.
        let count = lib.get_function(&device, "t_count").unwrap();
        let mut q = ClComputeQueue::new(&root);
        q.enqueue_fill_array(&out, &0u32, 0, Some(1));
        let mut ctx = q.new_bind_context(&count);
        ctx.set_primitive(0, &25u32.to_ne_bytes());
        ctx.set_buffer(1, &out);
        ctx.launch(&[32], &[32]).unwrap();
        q.flush().await_().unwrap();
        // SAFETY: the work completed.
        assert_eq!(unsafe { std::ptr::read_volatile(out.contents().cast::<u32>()) }, 25, "zeroed, then counted to 25");
        // A byte pattern over a range, widened to words: the bytes around it stay.
        let bytes = alloc_uninitialized_array_sync::<u8>(&device, 16);
        let mut q = ClComputeQueue::new(&root);
        q.enqueue_fill_array(&bytes, &0x11u8, 0, Some(16));
        q.enqueue_fill_array(&bytes, &0xABu8, 4, Some(8));
        q.flush().await_().unwrap();
        // SAFETY: the work completed.
        let seen = unsafe { std::slice::from_raw_parts(bytes.contents().cast::<u8>(), 16) };
        assert_eq!(seen, [[0x11; 4], [0xAB; 4], [0xAB; 4], [0x11; 4]].concat());
    }

    /// Work behind a wait for a host-signalled event stays held until the signal, though another queue flushes onto
    /// the same root meanwhile; that queue's work waits behind it (as a Metal command buffer behind a waiting one),
    /// and both run, in order, at the signal.
    #[test]
    fn a_wait_holds_the_roots_later_work_until_the_signal() {
        let Some((device, lib)) = compile("hold.uglykl", KERNELS) else { return };
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
