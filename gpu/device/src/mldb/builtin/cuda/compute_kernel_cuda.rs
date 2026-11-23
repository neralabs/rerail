//! The CUDA runtime of the grid kernels, beside compute_kernel_metal.rs and with its interface (Rerail, decision
//! 0440 steps 3 and 4): a library compiled at run time from UKL source by NVRTC, its functions with their arguments,
//! a queue of launches, copies and fills on a stream, its events, and buffers.
//!
//! **HIP** (AMD's GPUs, decision 0440 "HIP"): the same runtime over HIP's driver calls (`amdhip64`) and hiprtc, which
//! are CUDA's driver API and NVRTC call for call, with the same signatures and, for every constant read here but the
//! device attributes', the same values. `Vendor` is which the process opened: CUDA's driver where there is one, else
//! HIP's (`RERAIL_GPU=hip` asks for HIP). The few calls whose shapes differ are their own entries (`Api`), and UKL
//! preprocesses the kernels for HIP's language (`rerail_ukl::Backend::Hip`).
//!
//! Copyright (c) 2026 Jeremy Barnes. This file is part of MLDB's forest as carried in Rerail. Apache-2.0 (see the
//! LICENSE at `forest/src/mldb/`).
//!
//! - **No CUDA at build time.** The driver (`libcuda`, Windows' `nvcuda.dll`) and NVRTC are opened at run time; without them there is no
//!   device (`CudaDevice::system_default` is `None`), as on a Mac without Metal.
//! - **Buffers are host memory, mapped**: the host reads and writes them in place (`contents`), as Metal's shared
//!   storage, and kernels read them where they are. On a device that reads pageable memory through the host's page
//!   tables (a GB10's ATS) a block is the host's own pages; elsewhere it is pinned (`cuMemHostAlloc`). On a GB10 the
//!   pinned path's atomics and fences are slow: a grid barrier of 32 groups 24-42 µs on it, 0.7-1.4 µs on the host's
//!   pages (decision 0580). On a discrete card the pinned memory reads over the bus, which is correct and slow:
//!   placing blocks in device memory there is the block directory's work (0440 §7.2, step 10).
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

/// Whose driver the process opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    /// NVIDIA's: `libcuda` and NVRTC.
    Cuda,
    /// AMD's: `amdhip64` and hiprtc.
    Hip,
}

impl Vendor {
    pub fn name(self) -> &'static str {
        match self {
            Vendor::Cuda => "CUDA",
            Vendor::Hip => "HIP",
        }
    }
}

/// The entry points used, from `libcuda` and `libnvrtc`, or HIP's of the same signatures under CUDA's names (HIP's
/// names beside each in `api`).
struct Api {
    vendor: Vendor,
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
    // The byte is CUDA's `unsigned char`, HIP's `int`: passed as a 32-bit register on both.
    cu_memset_d8_async: unsafe extern "C" fn(DevPtr, u32, usize, Ptr) -> CUresult,
    cu_launch_kernel: unsafe extern "C" fn(Ptr, u32, u32, u32, u32, u32, u32, u32, Ptr, *mut Ptr, *mut Ptr) -> CUresult,
    cu_stream_create: unsafe extern "C" fn(*mut Ptr, u32) -> CUresult,
    cu_stream_synchronize: unsafe extern "C" fn(Ptr) -> CUresult,
    cu_stream_wait_value64: Option<unsafe extern "C" fn(Ptr, DevPtr, u64, u32) -> CUresult>,
    /// HIP's: a pointer, and a mask of the word's bits compared.
    hip_stream_wait_value64: Option<unsafe extern "C" fn(Ptr, Ptr, u64, u32, u64) -> CUresult>,
    cu_event_create: unsafe extern "C" fn(*mut Ptr, u32) -> CUresult,
    cu_event_record: unsafe extern "C" fn(Ptr, Ptr) -> CUresult,
    cu_event_synchronize: unsafe extern "C" fn(Ptr) -> CUresult,
    cu_event_query: unsafe extern "C" fn(Ptr) -> CUresult,
    cu_event_elapsed_time: unsafe extern "C" fn(*mut f32, Ptr, Ptr) -> CUresult,
    cu_event_destroy: unsafe extern "C" fn(Ptr) -> CUresult,
    /// CUDA's opt-in to more dynamic shared memory than the default; HIP has none (a block may take all its LDS).
    cu_func_set_attribute: Option<unsafe extern "C" fn(Ptr, i32, i32) -> CUresult>,
    cu_func_get_attribute: unsafe extern "C" fn(*mut i32, i32, Ptr) -> CUresult,
    /// The blocks of a kernel a multiprocessor holds at once, at a block size and dynamic shared memory.
    cu_occupancy_max_active_blocks: unsafe extern "C" fn(*mut i32, Ptr, i32, usize) -> CUresult,
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
    /// HIP's device properties (`hipGetDevicePropertiesR0600`), for the architecture's name (`gcnArchName`).
    hip_get_device_properties: Option<unsafe extern "C" fn(*mut u8, i32) -> CUresult>,
}

unsafe impl Send for Api {}
unsafe impl Sync for Api {}

// Driver API constants (cuda.h); HIP's are the same (hip_runtime_api.h, measured on HIP 7.2), but its device
// attributes' (`Attributes`).
const CU_MEMHOSTALLOC_PORTABLE: u32 = 0x01;
const CU_MEMHOSTALLOC_DEVICEMAP: u32 = 0x02;
const CU_STREAM_NON_BLOCKING: u32 = 0x1;
const CU_STREAM_WAIT_VALUE_GEQ: u32 = 0x0;
const CU_ERROR_NOT_READY: CUresult = 600;

/// The device attributes read, by number: each vendor's enumeration's.
struct Attributes {
    capability_major: i32,
    capability_minor: i32,
    max_shared_per_block_optin: i32,
    pageable_uses_host_page_tables: i32,
    host_pointer_for_registered_mem: i32,
    warp_size: i32,
    multiprocessors: i32,
}

const CUDA_ATTRIBUTES: Attributes = Attributes {
    capability_major: 75,
    capability_minor: 76,
    max_shared_per_block_optin: 97,
    pageable_uses_host_page_tables: 100,
    host_pointer_for_registered_mem: 91,
    warp_size: 10,
    multiprocessors: 16,
};

const HIP_ATTRIBUTES: Attributes = Attributes {
    capability_major: 23,
    capability_minor: 61,
    max_shared_per_block_optin: 75,
    pageable_uses_host_page_tables: 66,
    host_pointer_for_registered_mem: 4,
    warp_size: 87,
    // A Radeon's are its work-group processors (6 on a 680M).
    multiprocessors: 63,
};

/// `hipDeviceProp_tR0600`: its size, and where its architecture's name lies (`gcnArchName`, 256 bytes), measured
/// against HIP 7.2's header. The buffer the call fills is larger, for a later revision's tail.
const HIP_PROPS_BYTES: usize = 4096;
const HIP_PROPS_ARCH_NAME: usize = 1160;
const CUDA_ERROR_HOST_MEMORY_ALREADY_REGISTERED: CUresult = 712;
const CU_MEMHOSTREGISTER_PORTABLE: u32 = 0x01;
const CU_MEMHOSTREGISTER_DEVICEMAP: u32 = 0x02;
const CU_FUNC_ATTRIBUTE_SHARED_SIZE_BYTES: i32 = 1;
const CU_FUNC_ATTRIBUTE_MAX_DYNAMIC_SHARED_SIZE_BYTES: i32 = 8;

/// The driver's library, and NVRTC's in the system's search and in the toolkit's (`CUDA_HOME`, Windows' `CUDA_PATH`).
fn cuda_library_names() -> (Vec<String>, Vec<String>) {
    let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    if cfg!(windows) {
        let nvrtc = ["nvrtc64_130_0.dll", "nvrtc64_120_0.dll", "nvrtc64_112_0.dll"];
        let mut names = s(&nvrtc);
        if let Ok(home) = std::env::var("CUDA_PATH") {
            names.extend(nvrtc.iter().map(|s| format!("{home}\\bin\\{s}")));
        }
        (s(&["nvcuda.dll"]), names)
    } else {
        let home = std::env::var("CUDA_HOME").unwrap_or_else(|_| "/usr/local/cuda".into());
        let nvrtc = ["libnvrtc.so", "libnvrtc.so.13", "libnvrtc.so.12"];
        let mut names = s(&nvrtc);
        names.extend(nvrtc.iter().map(|s| format!("{home}/lib64/{s}")));
        (s(&["libcuda.so.1", "libcuda.so"]), names)
    }
}

/// HIP's library and hiprtc's: in the system's search (the driver installs `amdhip64`), and in the SDK's (`HIP_PATH`,
/// else each version under ROCm's install, newest first; on Linux `ROCM_PATH`, else `/opt/rocm`). hiprtc's Windows
/// name carries its version (`hiprtc0702.dll`): each in the SDK's directory is a candidate.
fn hip_library_names() -> (Vec<String>, Vec<String>) {
    if cfg!(windows) {
        let mut dirs: Vec<String> = std::env::var("HIP_PATH").into_iter().map(|h| format!("{}\\bin", h.trim_end_matches('\\'))).collect();
        let root = "C:\\Program Files\\AMD\\ROCm";
        let mut versions: Vec<String> = std::fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        versions.sort_by(|a, b| b.cmp(a));
        dirs.extend(versions.iter().map(|v| format!("{root}\\{v}\\bin")));
        let mut hip: Vec<String> = ["amdhip64_7.dll", "amdhip64_6.dll"].iter().map(|s| s.to_string()).collect();
        let mut rtc = Vec::new();
        for d in &dirs {
            hip.extend(["amdhip64_7.dll", "amdhip64_6.dll"].iter().map(|s| format!("{d}\\{s}")));
            let mut found: Vec<String> = std::fs::read_dir(d)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("hiprtc0") && n.ends_with(".dll"))
                .collect();
            found.sort_by(|a, b| b.cmp(a));
            rtc.extend(found.into_iter().map(|n| format!("{d}\\{n}")));
        }
        (hip, rtc)
    } else {
        let home = std::env::var("ROCM_PATH").unwrap_or_else(|_| "/opt/rocm".into());
        let with = |names: &[&str]| -> Vec<String> {
            let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
            v.extend(names.iter().map(|s| format!("{home}/lib/{s}")));
            v
        };
        (with(&["libamdhip64.so", "libamdhip64.so.7", "libamdhip64.so.6"]), with(&["libhiprtc.so", "libhiprtc.so.7", "libhiprtc.so.6"]))
    }
}

/// A symbol `dlsym` found, as the function pointer type `F` its field declares.
fn sym_as<F>(p: *mut c_void) -> F {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    // SAFETY: `F` is the symbol's C function pointer type, the size of a pointer (checked above).
    unsafe { std::mem::transmute_copy(&p) }
}

/// The hardware queues HIP's runtime is asked for (`GPU_MAX_HW_QUEUES`, 4 by default): as CUDA's connections
/// (`MAX_CONNECTIONS`), enough that the waiting root's stream holds back no other's.
const HIP_HW_QUEUES: usize = 8;

static API: OnceLock<Result<Api, String>> = OnceLock::new();

/// Whose driver the process opened, once it has (without opening one).
pub fn opened_vendor() -> Option<Vendor> {
    API.get().and_then(|a| a.as_ref().ok()).map(|a| a.vendor)
}

/// The driver and its runtime compiler, opened once: CUDA's, else HIP's (HIP's alone where `RERAIL_GPU=hip` asks for
/// it); `Err` says what is missing.
fn api() -> Result<&'static Api, String> {
    API.get_or_init(|| {
        if std::env::var("RERAIL_GPU").is_ok_and(|v| v.eq_ignore_ascii_case("hip")) {
            return hip_api();
        }
        cuda_api().or_else(|c| hip_api().map_err(|h| format!("{c}; {h}")))
    })
    .as_ref()
    .map_err(Clone::clone)
}

macro_rules! sym {
    ($lib:expr, $($name:literal)|+) => {{
        let p = sym_opt!($lib, $($name)|+);
        if p.is_null() {
            return Err(format!("{} not found", [$($name),+].join(" or ")));
        }
        sym_as(p)
    }};
}

macro_rules! sym_opt {
    ($lib:expr, $($name:literal)|+) => {{
        let mut p: *mut c_void = std::ptr::null_mut();
        $( if p.is_null() { p = $lib.sym($name); } )+
        p
    }};
}

fn cuda_api() -> Result<Api, String> {
    // The hardware queues the driver maps streams onto, round-robin in the order they are made: enough that no two of
    // the process's streams (a root's each, `CudaQueueRoot`, and any a caller makes) share one. Read by the driver when
    // it loads; a value the environment sets is kept.
    crate::os::nvidia_hardware_queues(MAX_CONNECTIONS);
    use crate::os::{Lib, Scope};
    let (driver, nvrtc_names) = cuda_library_names();
    let cuda = Lib::open(&driver, Scope::Global).ok_or_else(|| format!("no CUDA driver ({})", driver.join(", ")))?;
    let nvrtc = Lib::open(&nvrtc_names, Scope::Global).ok_or_else(|| format!("no NVRTC ({})", nvrtc_names.join(", ")))?;
    Ok(Api {
        vendor: Vendor::Cuda,
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
        cu_stream_wait_value64: Some(sym!(cuda, "cuStreamWaitValue64_v2" | "cuStreamWaitValue64")),
        hip_stream_wait_value64: None,
        cu_event_create: sym!(cuda, "cuEventCreate"),
        cu_event_record: sym!(cuda, "cuEventRecord"),
        cu_event_synchronize: sym!(cuda, "cuEventSynchronize"),
        cu_event_query: sym!(cuda, "cuEventQuery"),
        cu_event_elapsed_time: sym!(cuda, "cuEventElapsedTime_v2" | "cuEventElapsedTime"),
        cu_event_destroy: sym!(cuda, "cuEventDestroy_v2"),
        cu_func_set_attribute: Some(sym!(cuda, "cuFuncSetAttribute")),
        cu_func_get_attribute: sym!(cuda, "cuFuncGetAttribute"),
        cu_occupancy_max_active_blocks: sym!(cuda, "cuOccupancyMaxActiveBlocksPerMultiprocessor"),
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
        hip_get_device_properties: None,
    })
}

/// HIP's calls under CUDA's names: each the same signature as the CUDA call its field is named for.
fn hip_api() -> Result<Api, String> {
    if std::env::var_os("GPU_MAX_HW_QUEUES").is_none() {
        // SAFETY: once, before HIP's runtime loads, as the process's first use of the device.
        unsafe { std::env::set_var("GPU_MAX_HW_QUEUES", HIP_HW_QUEUES.to_string()) };
    }
    use crate::os::{Lib, Scope};
    let (driver, rtc_names) = hip_library_names();
    let hip = Lib::open(&driver, Scope::Global).ok_or_else(|| format!("no HIP runtime ({})", driver.join(", ")))?;
    let rtc = Lib::open(&rtc_names, Scope::Global).ok_or_else(|| format!("no hiprtc ({})", rtc_names.join(", ")))?;
    Ok(Api {
        vendor: Vendor::Hip,
        cu_init: sym!(hip, "hipInit"),
        cu_device_get: sym!(hip, "hipDeviceGet"),
        cu_device_get_attribute: sym!(hip, "hipDeviceGetAttribute"),
        cu_device_primary_ctx_retain: sym!(hip, "hipDevicePrimaryCtxRetain"),
        cu_ctx_set_current: sym!(hip, "hipCtxSetCurrent"),
        cu_mem_host_alloc: sym!(hip, "hipHostMalloc" | "hipHostAlloc"),
        cu_mem_host_get_device_pointer: sym!(hip, "hipHostGetDevicePointer"),
        cu_mem_host_register: sym!(hip, "hipHostRegister"),
        cu_mem_host_unregister: sym!(hip, "hipHostUnregister"),
        cu_module_load_data: sym!(hip, "hipModuleLoadData"),
        cu_module_get_function: sym!(hip, "hipModuleGetFunction"),
        cu_module_get_global: sym!(hip, "hipModuleGetGlobal"),
        cu_memcpy_dtoh: sym!(hip, "hipMemcpyDtoH"),
        cu_memcpy_async: sym!(hip, "hipMemcpyDtoDAsync"),
        cu_memset_d8_async: sym!(hip, "hipMemsetD8Async"),
        cu_launch_kernel: sym!(hip, "hipModuleLaunchKernel"),
        cu_stream_create: sym!(hip, "hipStreamCreateWithFlags"),
        cu_stream_synchronize: sym!(hip, "hipStreamSynchronize"),
        cu_stream_wait_value64: None,
        hip_stream_wait_value64: Some(sym!(hip, "hipStreamWaitValue64")),
        cu_event_create: sym!(hip, "hipEventCreateWithFlags"),
        cu_event_record: sym!(hip, "hipEventRecord"),
        cu_event_synchronize: sym!(hip, "hipEventSynchronize"),
        cu_event_query: sym!(hip, "hipEventQuery"),
        cu_event_elapsed_time: sym!(hip, "hipEventElapsedTime"),
        cu_event_destroy: sym!(hip, "hipEventDestroy"),
        cu_func_set_attribute: None,
        cu_func_get_attribute: sym!(hip, "hipFuncGetAttribute"),
        cu_occupancy_max_active_blocks: sym!(hip, "hipModuleOccupancyMaxActiveBlocksPerMultiprocessor"),
        cu_get_error_string: sym!(hip, "hipDrvGetErrorString"),
        nvrtc_create_program: sym!(rtc, "hiprtcCreateProgram"),
        nvrtc_compile_program: sym!(rtc, "hiprtcCompileProgram"),
        nvrtc_get_program_log_size: sym!(rtc, "hiprtcGetProgramLogSize"),
        nvrtc_get_program_log: sym!(rtc, "hiprtcGetProgramLog"),
        nvrtc_get_cubin_size: sym!(rtc, "hiprtcGetCodeSize"),
        nvrtc_get_cubin: sym!(rtc, "hiprtcGetCode"),
        nvrtc_destroy_program: sym!(rtc, "hiprtcDestroyProgram"),
        nvrtc_version: sym!(rtc, "hiprtcVersion"),
        cu_driver_get_version: sym!(hip, "hipDriverGetVersion"),
        hip_get_device_properties: Some(sym!(hip, "hipGetDevicePropertiesR0600")),
    })
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
    let vendor = api().map_or("CUDA", |a| a.vendor.name());
    Err(format!("{what}: {vendor} error {r} {msg}"))
}

/// A driver call that cannot fail in a correct program (a release, a record on a live stream).
fn must(r: CUresult, what: &str) {
    if let Err(e) = ck(r, what) {
        panic!("{e}");
    }
}

/// The process's device: the first CUDA (or HIP) device, its primary context, and what the launches need of it.
#[derive(Clone, Copy, Debug)]
pub struct CudaDevice {
    ctx: Ptr,
    /// Whose driver it is.
    pub vendor: Vendor,
    /// The compute capability as `major * 10 + minor` (121 for GB10's 12.1); HIP's, which no compile reads.
    pub sm: u32,
    /// HIP's architecture (`gfx1035` for a Radeon 680M): what its kernels are compiled for. Empty on CUDA.
    pub arch: &'static str,
    /// The lanes of a SIMD group: a warp's 32; a wavefront's 32 or 64 on AMD's.
    pub simd_width: u32,
    /// The shared memory a block may opt in to (static and dynamic together).
    pub max_shared_per_block: u32,
    /// Whether the device reads the host's pageable memory through the host's page tables (a GB10's ATS): then
    /// memory wrapped in place (`wrap_host_memory`) is read at the host's address, nothing registered.
    pub pageable: bool,
    /// Its multiprocessors (NVIDIA's SMs, AMD's work-group processors): with a kernel's occupancy, the groups the
    /// device holds at once (`CudaComputeFunction::resident_groups`).
    pub multiprocessors: u32,
    /// Whether registered host memory is the device's at the host's address (`CAN_USE_HOST_POINTER_FOR_REGISTERED_MEM`,
    /// and a page registered at open to confirm it: HIP on Windows says so, and maps it elsewhere): then a wrap may
    /// span several registrations (`register_runs`); else each wrap lies in one (`register_mapped`).
    host_pointer: bool,
}

unsafe impl Send for CudaDevice {}
unsafe impl Sync for CudaDevice {}

/// The device a reference to it stands for (as `metal::DeviceRef` for `metal::Device`).
pub type CudaDeviceRef = CudaDevice;

thread_local! {
    static CURRENT: std::cell::Cell<Ptr> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Whether a page of the host's, registered, is the device's at the host's address.
fn registered_at_host_address(a: &Api) -> Result<bool, String> {
    let page = page_bytes() as usize;
    let layout = std::alloc::Layout::from_size_align(page, page).map_err(|e| e.to_string())?;
    // SAFETY: a page, freed below; registered and unregistered while nothing else holds it.
    unsafe {
        let p = std::alloc::alloc_zeroed(layout);
        if p.is_null() {
            return Err("no page to register".into());
        }
        let r = ck((a.cu_mem_host_register)(p.cast(), page, CU_MEMHOSTREGISTER_PORTABLE | CU_MEMHOSTREGISTER_DEVICEMAP), "registering a page");
        let mut dev = 0;
        let same = r.is_ok() && (a.cu_mem_host_get_device_pointer)(&mut dev, p.cast(), 0) == 0 && dev == p as u64;
        if r.is_ok() {
            (a.cu_mem_host_unregister)(p.cast());
        }
        std::alloc::dealloc(p, layout);
        r.map(|()| same)
    }
}

impl CudaDevice {
    /// The first device, or `None` without a driver, NVRTC or a device.
    pub fn system_default() -> Option<CudaDevice> {
        match Self::open() {
            Ok(d) => Some(d),
            Err(e) => {
                // A process that asked for CUDA (`RERAIL_GPU=cuda`) says why it has none, as one that asked for OpenCL
                // does: without it every unit after was refused as over capacity, with no word of the device.
                if std::env::var_os("RERAIL_CUDA_DEBUG").is_some() || crate::device::chosen() == "cuda" {
                    eprintln!("no CUDA or HIP device: {e}");
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
            let at = match a.vendor {
                Vendor::Cuda => &CUDA_ATTRIBUTES,
                Vendor::Hip => &HIP_ATTRIBUTES,
            };
            let sm = (attr(at.capability_major)? * 10 + attr(at.capability_minor)?) as u32;
            let max_shared_per_block = attr(at.max_shared_per_block_optin)? as u32;
            let pageable = attr(at.pageable_uses_host_page_tables).unwrap_or(0) != 0;
            let says_host_pointer = attr(at.host_pointer_for_registered_mem).unwrap_or(0) != 0;
            let simd_width = attr(at.warp_size)? as u32;
            let multiprocessors = attr(at.multiprocessors)? as u32;
            if simd_width != 32 && simd_width != 64 {
                return Err(format!("a SIMD group of {simd_width} lanes"));
            }
            let arch = match a.hip_get_device_properties {
                None => "",
                Some(props) => {
                    let mut p = vec![0u8; HIP_PROPS_BYTES];
                    ck(props(p.as_mut_ptr(), dev), "hipGetDevicePropertiesR0600")?;
                    let name = CStr::from_bytes_until_nul(&p[HIP_PROPS_ARCH_NAME..HIP_PROPS_ARCH_NAME + 256]).map_err(|e| e.to_string())?;
                    // The name and its features (`gfx90a:sramecc+:xnack-`): the compile takes the name.
                    let name = name.to_string_lossy();
                    let name = name.split(':').next().unwrap_or_default();
                    if !name.starts_with("gfx") {
                        return Err(format!("HIP's architecture is {name:?}, not a gfx name"));
                    }
                    Box::leak(name.to_owned().into_boxed_str())
                }
            };
            let mut ctx = std::ptr::null_mut();
            ck((a.cu_device_primary_ctx_retain)(&mut ctx, dev), "cuDevicePrimaryCtxRetain")?;
            ck((a.cu_ctx_set_current)(ctx), "cuCtxSetCurrent")?;
            // The attribute, measured: HIP on Windows says registered memory is the device's at the host's address,
            // and maps it elsewhere (0440, "The HIP backend": the device then read unmapped addresses, the IOMMU
            // faulted, and the host's hypervisor missed its DPC deadline). A page registered tells.
            let host_pointer = says_host_pointer && registered_at_host_address(a)?;
            Ok(CudaDevice { ctx, vendor: a.vendor, sm, arch, simd_width, multiprocessors, max_shared_per_block, pageable, host_pointer })
        }
    }

    /// Makes the context current on this thread (once a thread).
    fn current(&self) -> &'static Api {
        let a = api().expect("a CUDA or HIP device without its API");
        CURRENT.with(|c| {
            if c.get() != self.ctx {
                must(unsafe { (a.cu_ctx_set_current)(self.ctx) }, "cuCtxSetCurrent");
                c.set(self.ctx);
            }
        });
        a
    }

    /// The defines its kernels' preprocessing takes: on HIP, the dot-product instructions of the architectures they
    /// were measured on (RDNA2's `v_dot4_i32_i8` and `v_dot2_f32_f16`, a Radeon 680M's gfx1035: `ukl/dot.h`); RDNA3
    /// spells its byte form otherwise (`v_dot4_i32_iu8`), and takes the reference until it is measured.
    pub fn defines(&self) -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        if self.vendor == Vendor::Hip && self.arch.starts_with("gfx103") {
            v.push(("UKL_HIP_AMD_DOT", "1".to_string()));
        }
        v
    }

    /// An event the host signals and a queue waits on (`metal::SharedEvent`): a word of pinned memory.
    pub fn new_shared_event(&self) -> CudaSharedEvent {
        // Coherent: the device's wait reads the host's writes while it runs.
        let b = alloc(self, 8, true);
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
    /// Whether its pool's memory is kept coherent with the host while a kernel runs (`alloc`).
    coherent: bool,
    /// Wrapped memory's registration with the driver: none (read where it lies, or pinned by the driver itself), or
    /// its share of the page runs registered (`REGISTERED`); released when the buffer goes.
    registered: Registration,
}

enum Registration {
    None,
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
                Registration::Runs(runs) => release_runs(&runs),
            }
            return;
        }
        POOL.lock().unwrap().entry((self.class, self.coherent)).or_default().push(Block { host: self.host, dev: self.dev });
    }
}

/// A block of a size class, free: the host's pages or pinned memory (`alloc_uninitialized_array_sync`).
struct Block {
    host: *mut u8,
    dev: DevPtr,
}

unsafe impl Send for Block {}

/// The free pinned blocks, by size (a power of two, at least 256 bytes).
static POOL: std::sync::Mutex<std::collections::BTreeMap<(u64, bool), Vec<Block>>> = std::sync::Mutex::new(std::collections::BTreeMap::new());

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
    alloc(device, (length * std::mem::size_of::<T>()) as u64, false)
}

/// `hipHostMalloc`'s kinds of host memory: kept coherent with the host while a kernel runs (fine-grained), or not
/// (coarse-grained: the device caches it, and the host sees the device's writes once the kernel ends, as it reads
/// them, after an event).
const HIP_HOST_MALLOC_COHERENT: u32 = 0x4000_0000;
const HIP_HOST_MALLOC_NON_COHERENT: u32 = 0x8000_0000;

/// Pinned host memory of at least `bytes`, from its size class's pool. On HIP it is coarse-grained but where
/// `coherent` asks (a word the host writes while the device waits on it, `new_shared_event`): AMD's device does its
/// atomics on fine-grained memory in the host's, past its caches. A Radeon 680M built a 1 Mi-key histogram in 150 ms
/// there, 0.76 ms in coarse-grained memory and 0.94 ms in its own (0440, "HIP's copies and fills"). Reads ran alike.
///
/// On HIP a block of a page or more is the host's own memory, page-aligned, registered with the driver: what
/// OpenCL's buffers are (`CL_MEM_USE_HOST_PTR`) and what a wrap of the engine's blocks is (`register_mapped`). Its
/// atomics ran as coarse-grained memory's (0.91 ms the histogram), and writes spread over many pages ran faster: a
/// radix pass's (a row to each of 256 buckets, keys and values) took 52.6 ms over 4 Mi rows there, 65.8 ms in
/// `hipHostMalloc`'s coarse-grained memory and 62.7 in the device's own. A smaller block stays `hipHostMalloc`'s,
/// as registering it would pin a page for it.
fn alloc(device: &CudaDevice, bytes: u64, coherent: bool) -> CudaBuffer {
    let len = bytes.max(16);
    let class = len.max(256).next_power_of_two();
    sweep_pending(device.current());
    let reused = POOL.lock().unwrap().get_mut(&(class, coherent)).and_then(Vec::pop);
    let Block { host, dev } = reused.unwrap_or_else(|| {
        // The host's pages where the device reads them through its page tables (decision 0580): a pinned block's
        // atomics and fences take a slow path there. `RERAIL_CUDA_PINNED` keeps pinned blocks, a measurement aid.
        if device.pageable && std::env::var_os("RERAIL_CUDA_PINNED").is_none() {
            let host = crate::os::map_zeroed(class as usize).expect("the host's pages for a CUDA block");
            return Block { host, dev: host as DevPtr };
        }
        let a = device.current();
        if a.vendor == Vendor::Hip && !coherent && class >= page_bytes() {
            return registered_block(a, class);
        }
        let mut host = std::ptr::null_mut();
        let kind = match a.vendor {
            Vendor::Cuda => 0,
            Vendor::Hip if coherent => HIP_HOST_MALLOC_COHERENT,
            Vendor::Hip => HIP_HOST_MALLOC_NON_COHERENT,
        };
        must(unsafe { (a.cu_mem_host_alloc)(&mut host, class as usize, CU_MEMHOSTALLOC_PORTABLE | CU_MEMHOSTALLOC_DEVICEMAP | kind) }, "cuMemHostAlloc");
        let mut dev = 0;
        must(unsafe { (a.cu_mem_host_get_device_pointer)(&mut dev, host, 0) }, "cuMemHostGetDevicePointer");
        Block { host: host.cast(), dev }
    });
    CudaBuffer(Arc::new(Allocation { host, dev, len, class, coherent, registered: Registration::None }))
}

/// A pool block of `class` bytes (a page or more) of the host's memory, registered and mapped (`alloc` on HIP). It is
/// the pool's for the process's life, as `cuMemHostAlloc`'s blocks are.
fn registered_block(a: &Api, class: u64) -> Block {
    let layout = std::alloc::Layout::from_size_align(class as usize, page_bytes() as usize).expect("a pool block's layout");
    // SAFETY: a non-zero size; the block is never freed (the pool keeps it).
    let host = unsafe { std::alloc::alloc(layout) };
    assert!(!host.is_null(), "out of memory for a {class}-byte block");
    must(unsafe { (a.cu_mem_host_register)(host.cast(), class as usize, CU_MEMHOSTREGISTER_PORTABLE | CU_MEMHOSTREGISTER_DEVICEMAP) }, "cuMemHostRegister");
    let mut dev = 0;
    must(unsafe { (a.cu_mem_host_get_device_pointer)(&mut dev, host.cast(), 0) }, "cuMemHostGetDevicePointer");
    Block { host, dev }
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
    let wrap = |dev: DevPtr, registered: Registration| Ok(CudaBuffer(Arc::new(Allocation { host: ptr, dev, len: len as u64, class: 0, coherent: true, registered })));
    if device.pageable {
        return wrap(ptr as DevPtr, Registration::None);
    }
    let a = device.current();
    if device.host_pointer {
        let runs = register_runs(a, ptr as u64, len as u64)?;
        return wrap(ptr as DevPtr, Registration::Runs(runs));
    }
    let (dev, registered) = register_mapped(a, ptr as u64, len as u64)?;
    wrap(dev, registered)
}

/// A wrap's pages registered where the device maps them elsewhere (`CudaDevice::host_pointer` false): its device
/// address, and its registration. A run that holds all its pages already gives it (its device base and the offset);
/// else its pages become a run of their own; pages the driver pinned itself (`cuMemHostAlloc`'s, error 712) are read
/// where it maps them, when that is one span. A wrap over pages of several registrations has no single device
/// address: an error, never an address the device does not map.
fn register_mapped(a: &Api, at: u64, len: u64) -> Result<(DevPtr, Registration), String> {
    let page = page_bytes();
    let (lo, hi) = (at / page * page, (at + len.max(1)).div_ceil(page) * page);
    let mut runs = REGISTERED.lock().unwrap_or_else(|p| p.into_inner());
    let over: Vec<(u64, u64, u64)> = runs.range(..hi).rev().take_while(|(_, r)| r.end > lo).map(|(&s, r)| (s, r.end, r.dev)).collect();
    match over.as_slice() {
        [(s, e, dev)] if *s <= lo && *e >= hi => {
            runs.get_mut(s).expect("a run").holders += 1;
            return Ok((dev + (at - s), Registration::Runs(vec![*s])));
        }
        [] => {}
        _ => return Err(format!("{len} bytes at {at:#x} span pages of {} registrations: no one device address", over.len())),
    }
    let flags = CU_MEMHOSTREGISTER_PORTABLE | CU_MEMHOSTREGISTER_DEVICEMAP;
    let r = unsafe { (a.cu_mem_host_register)(lo as Ptr, (hi - lo) as usize, flags) };
    let device_pointer = |p: u64| {
        let mut dev = 0;
        ck(unsafe { (a.cu_mem_host_get_device_pointer)(&mut dev, p as Ptr, 0) }, "cuMemHostGetDevicePointer").map(|()| dev)
    };
    if r == CUDA_ERROR_HOST_MEMORY_ALREADY_REGISTERED {
        let (first, last) = (device_pointer(at)?, device_pointer(at + len.max(1) - 1)?);
        if last != first + len.max(1) - 1 {
            return Err(format!("{len} bytes at {at:#x}: pages the driver pinned, mapped apart"));
        }
        return Ok((first, Registration::None));
    }
    ck(r, "cuMemHostRegister")?;
    match device_pointer(lo) {
        Ok(dev) => {
            runs.insert(lo, Run { end: hi, holders: 1, dev });
            Ok((dev + (at - lo), Registration::Runs(vec![lo])))
        }
        Err(e) => {
            unsafe { (a.cu_mem_host_unregister)(lo as Ptr) };
            Err(e)
        }
    }
}

/// A run of pages registered with the driver for wraps (`wrap_host_memory`): its end, and how many wraps hold it.
struct Run {
    end: u64,
    holders: usize,
    /// The device's address of its first byte: the host's where the device reads registered memory there.
    dev: u64,
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
        runs.insert(s, Run { end: e, holders: 0, dev: s });
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
    /// Compiles CUDA C++ (UKL preprocessed for this device) with NVRTC for the device's architecture, and loads it;
    /// on HIP, HIP's C++ with hiprtc for the device's `gfx` architecture, its code object.
    pub fn compile_from_source(device: &CudaDevice, preprocessed_source: &str, file_name_to_appear_in_error_messages: &str) -> Result<Self, String> {
        let a = device.current();
        let (mut options, kind, target): (Vec<String>, &str, String) = match device.vendor {
            Vendor::Cuda => (vec![format!("--gpu-architecture=sm_{}", device.sm), "-std=c++20".into(), "-default-device".into()], "cuda", format!("sm_{}", device.sm)),
            // hiprtc's functions are host functions unless marked; UKL's HIP shim marks every one a device's
            // (`force_cuda_host_device`), as NVRTC's `-default-device` does.
            Vendor::Hip => (vec![format!("--offload-arch={}", device.arch), "-std=c++20".into(), "-O3".into()], "hip", device.arch.to_string()),
        };
        // `RERAIL_CUDA_OPTIONS`: more of NVRTC's (or hiprtc's) options, a measurement aid (they are in the key).
        options.extend(std::env::var("RERAIL_CUDA_OPTIONS").unwrap_or_default().split_whitespace().map(str::to_owned));
        // The store's code (`crate::store`), keyed by all the compile reads: the compiler's and the driver's builds too.
        let (mut major, mut minor, mut driver) = (0, 0, 0);
        unsafe { (a.nvrtc_version)(&mut major, &mut minor) };
        unsafe { (a.cu_driver_get_version)(&mut driver) };
        let versions = format!("{kind} rtc {major}.{minor} driver {driver}");
        let key = crate::store::key(&[kind.as_bytes(), versions.as_bytes(), options.join(" ").as_bytes(), preprocessed_source.as_bytes()]);
        if let Some(cubin) = crate::store::load(kind, &key, "cubin") {
            let mut module = std::ptr::null_mut();
            if unsafe { (a.cu_module_load_data)(&mut module, cubin.as_ptr().cast()) } == 0 {
                crate::store::count(true);
                return Ok(CudaComputeFunctionLibrary { module, device: *device });
            }
        }
        crate::store::count(false);
        crate::store::dump(file_name_to_appear_in_error_messages, if kind == "hip" { "hip" } else { "cu" }, &options.join(" "), preprocessed_source);
        let src = CString::new(preprocessed_source).map_err(|e| e.to_string())?;
        let name = CString::new(file_name_to_appear_in_error_messages).map_err(|e| e.to_string())?;
        let opts: Vec<CString> = options.into_iter().map(|o| CString::new(o).unwrap()).collect();
        let opt_ptrs: Vec<*const c_char> = opts.iter().map(|o| o.as_ptr()).collect();
        let mut prog = std::ptr::null_mut();
        unsafe {
            if (a.nvrtc_create_program)(&mut prog, src.as_ptr(), name.as_ptr(), 0, std::ptr::null(), std::ptr::null()) != 0 {
                return Err(format!("{}'s program creation failed", a.vendor.name()));
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
                let compiler = if kind == "hip" { "hiprtc" } else { "NVRTC" };
                return Err(format!("{compiler} compiling {file_name_to_appear_in_error_messages} for {target}: {}", errors.join("\n")));
            }
            let mut n = 0;
            (a.nvrtc_get_cubin_size)(prog, &mut n);
            let mut cubin = vec![0u8; n];
            (a.nvrtc_get_cubin)(prog, cubin.as_mut_ptr().cast());
            (a.nvrtc_destroy_program)(&mut prog);
            let mut module = std::ptr::null_mut();
            ck((a.cu_module_load_data)(&mut module, cubin.as_ptr().cast()), "cuModuleLoadData")?;
            crate::store::save(kind, &key, "cubin", &cubin);
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
        if let Some(set) = a.cu_func_set_attribute {
            ck(unsafe { set(func, CU_FUNC_ATTRIBUTE_MAX_DYNAMIC_SHARED_SIZE_BYTES, dynamic_limit as i32) }, "cuFuncSetAttribute")?;
        }
        Ok(CudaComputeFunction { func, device: self.device, name: function_name.to_string(), arguments, param_sizes, dynamic_limit })
    }
}

/// A kernel of a library, with its arguments (`MetalComputeFunction`).
pub struct CudaComputeFunction {
    func: Ptr,
    device: CudaDevice,
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

    /// The groups of `threads` threads, each taking `dynamic` bytes of shared memory beyond the kernel's own, that
    /// the device holds at once: the driver's occupancy of this kernel (its registers, its shared memory) on each
    /// multiprocessor, times them. A persistent grid's bound (decision 0468 §4).
    pub fn resident_groups(&self, threads: u32, dynamic: u64) -> Option<u64> {
        let a = self.device.current();
        let mut n = 0;
        ck(unsafe { (a.cu_occupancy_max_active_blocks)(&mut n, self.func, threads as i32, dynamic as usize) }, "cuOccupancyMaxActiveBlocksPerMultiprocessor").ok()?;
        Some(n.max(0) as u64 * self.device.multiprocessors as u64)
    }
}

/// The streams the process makes: the waiting root's and the queue root's (`CudaQueueRoot`, `rerail_device`'s
/// `shared`), one each.
const STREAMS: usize = 2;

/// The hardware queues asked of the driver: one a stream. Not its maximum of 32: each one costs the context's creation
/// (`os::nvidia_hardware_queues`).
const MAX_CONNECTIONS: usize = STREAMS;

/// The streams made so far: past `STREAMS`, one would share a hardware queue with the waiting root's.
static STREAMS_MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

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
/// waiting root's holds back nothing else: the driver is given a hardware queue a stream the process makes, and a
/// stream past them is refused (`STREAMS`).
pub struct CudaQueueRoot {
    device: CudaDevice,
    stream: Ptr,
    /// HIP's copies and fills, as kernels (`Blit`); `None` on CUDA, whose driver's are fast.
    blit: Option<Arc<Blit>>,
}

/// Copies and fills as kernels, for HIP: its runtime's own (`hipMemcpyDtoDAsync`, `hipMemsetD8Async`) are slow on
/// host memory, which every buffer here is. Measured on a Radeon 680M, 1 MB: a copy 0.25 ms (a kernel's 0.1), a fill
/// 2.7 ms; 16 MB, a fill 31 ms (0.5 in device memory). OpenCL's driver copies and fills by kernels, and the grid's
/// sort and scan, which fill and copy between their passes, ran 4 to 15 times OpenCL's time (0440, "HIP's copies and
/// fills"). Each kernel moves elements of its width (16, 4 or 1 bytes): the widest that divides both addresses and the
/// length.
struct Blit {
    /// The functions by width: 16, 4 and 1 bytes.
    copy: [Ptr; 3],
    fill: [Ptr; 3],
    _library: CudaComputeFunctionLibrary,
}

unsafe impl Send for Blit {}
unsafe impl Sync for Blit {}

const BLIT_SOURCE: &str = r#"
typedef unsigned int u32;
typedef unsigned long long u64;
typedef u32 u32x4 __attribute__((ext_vector_type(4)));
#define BLIT(W, T) \
extern "C" __global__ void rerail_copy##W(T* d, const T* s, u64 n) { \
    for (u64 i = (u64)blockIdx.x * blockDim.x + threadIdx.x; i < n; i += (u64)gridDim.x * blockDim.x) d[i] = s[i]; } \
extern "C" __global__ void rerail_fill##W(T* d, u64 n) { \
    for (u64 i = (u64)blockIdx.x * blockDim.x + threadIdx.x; i < n; i += (u64)gridDim.x * blockDim.x) d[i] = (T)0; }
BLIT(16, u32x4)
BLIT(4, u32)
BLIT(1, unsigned char)
"#;

/// The threads of a blit's group, and the most groups a blit launches (each thread strides past the grid).
const BLIT_THREADS: u32 = 256;
const BLIT_GROUPS: u64 = 4096;

impl Blit {
    fn new(device: &CudaDevice) -> Blit {
        let library = CudaComputeFunctionLibrary::compile_from_source(device, BLIT_SOURCE, "rerail_blit.hip").unwrap_or_else(|e| panic!("HIP's copy and fill kernels: {e}"));
        let f = |name: String| library.function_handle(&name).unwrap_or_else(|e| panic!("{e}"));
        let widths = [16, 4, 1];
        Blit { copy: widths.map(|w| f(format!("rerail_copy{w}"))), fill: widths.map(|w| f(format!("rerail_fill{w}"))), _library: library }
    }

    /// The widest of 16, 4 and 1 bytes that divides every one of `parts`: the width's index.
    fn width(parts: &[u64]) -> (usize, u64) {
        let bits = parts.iter().fold(0, |m, p| m | p);
        [16u64, 4, 1].iter().enumerate().find(|(_, w)| bits % **w == 0).map(|(i, w)| (i, *w)).expect("1 divides all")
    }

    fn launch(a: &Api, stream: Ptr, func: Ptr, args: &mut [u64], elements: u64, what: &str) -> Result<(), String> {
        if elements == 0 {
            return Ok(());
        }
        let groups = elements.div_ceil(BLIT_THREADS as u64).min(BLIT_GROUPS) as u32;
        let mut ptrs: Vec<Ptr> = args.iter_mut().map(|p| (p as *mut u64).cast::<c_void>()).collect();
        ck(unsafe { (a.cu_launch_kernel)(func, groups, 1, 1, BLIT_THREADS, 1, 1, 0, stream, ptrs.as_mut_ptr(), std::ptr::null_mut()) }, what)
    }

    fn copy(&self, a: &Api, stream: Ptr, dst: u64, src: u64, bytes: usize) -> Result<(), String> {
        let (i, w) = Blit::width(&[dst, src, bytes as u64]);
        Blit::launch(a, stream, self.copy[i], &mut [dst, src, bytes as u64 / w], bytes as u64 / w, "a copy kernel")
    }

    fn fill(&self, a: &Api, stream: Ptr, dst: u64, bytes: usize) -> Result<(), String> {
        let (i, w) = Blit::width(&[dst, bytes as u64]);
        Blit::launch(a, stream, self.fill[i], &mut [dst, bytes as u64 / w], bytes as u64 / w, "a fill kernel")
    }
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
        let made = STREAMS_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert!(made < STREAMS, "a CUDA stream past the process's {STREAMS}: it would share a hardware queue with the waiting root's (MAX_CONNECTIONS)");
        let a = device.current();
        let mut stream = std::ptr::null_mut();
        must(unsafe { (a.cu_stream_create)(&mut stream, CU_STREAM_NON_BLOCKING) }, "cuStreamCreate");
        let blit = (device.vendor == Vendor::Hip).then(|| Arc::new(Blit::new(&device)));
        CudaQueueRoot { device, stream, blit }
    }
}

/// A queue of operations (`MetalComputeQueue`): launches, copies and fills in order on one of its root's streams. `flush`
/// records an event to await; `finish` flushes and waits. A queue dropped without a flush runs nothing (a Metal command
/// buffer never committed): its operations and the buffers they would use are dropped with it.
pub struct CudaComputeQueue {
    device: CudaDevice,
    stream: Ptr,
    blit: Option<Arc<Blit>>,
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
        CudaComputeQueue { device: root.device, stream: root.stream, blit: root.blit.clone(), ops: Default::default(), staging: Default::default(), keep: Default::default() }
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
                if let Err(e) = issue(a, self.stream, self.blit.as_deref(), op) {
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

/// One operation onto `stream`: copies and fills by `blit`'s kernels where it is given (HIP's).
fn issue(a: &Api, stream: Ptr, blit: Option<&Blit>, op: Op) -> Result<(), String> {
    match op {
        Op::Launch { func, grid, block, shared, mut params, name } => {
            // The driver reads each parameter's bytes during the call.
            let mut ptrs: Vec<Ptr> = params.iter_mut().map(|p| p.as_mut_ptr().cast::<c_void>()).collect();
            ck(
                unsafe { (a.cu_launch_kernel)(func, grid[0], grid[1], grid[2], block[0], block[1], block[2], shared, stream, ptrs.as_mut_ptr(), std::ptr::null_mut()) },
                &format!("launching {name}"),
            )
        }
        Op::Memset { dst, bytes } => match blit {
            Some(b) => b.fill(a, stream, dst, bytes),
            None => ck(unsafe { (a.cu_memset_d8_async)(dst, 0, bytes, stream) }, "cuMemsetD8Async"),
        },
        Op::Memcpy { dst, src, bytes } => match blit {
            Some(b) => b.copy(a, stream, dst, src, bytes),
            None => ck(unsafe { (a.cu_memcpy_async)(dst, src, bytes, stream) }, "cuMemcpyAsync"),
        },
        Op::WaitValue { at, value } => match (a.cu_stream_wait_value64, a.hip_stream_wait_value64) {
            (Some(wait), _) => ck(unsafe { wait(stream, at, value, CU_STREAM_WAIT_VALUE_GEQ) }, "cuStreamWaitValue64"),
            // The word is pinned host memory, at the same address for the device (`hipHostMalloc`'s): all its bits.
            (None, Some(wait)) => ck(unsafe { wait(stream, at as Ptr, value, CU_STREAM_WAIT_VALUE_GEQ, u64::MAX) }, "hipStreamWaitValue64"),
            (None, None) => unreachable!("a driver without a wait on a value"),
        },
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
