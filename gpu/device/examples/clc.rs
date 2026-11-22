//! `clc`: the OpenCL driver alone, a measurement aid for decision 0395 §6's compile costs. It compiles a source the
//! engine dumped (`RERAIL_GPU_DUMP`), or variants of it, outside the engine, and times each step.
//!
//! ```text
//! clc info                                 the first GPU's name, driver, version and extensions
//! clc build <file.cl> [options]            clBuildProgram; keeps the binary as <file.cl>.bin
//! clc load <file.bin> [options]            clCreateProgramWithBinary and clBuildProgram
//! clc link <options> <file.cl>...          clCompileProgram each file (with -create-library none), then clLinkProgram
//! clc usm [mb]                             host USM (cl_intel_unified_shared_memory): a kernel reads through an
//!                                          address held in memory, as a block's directory is
//! ```
//! A source's first line `// options: ...` (the dump's) is prepended to the options given.

use std::ffi::{CString, c_char, c_void};
use std::time::Instant;

type Ptr = *mut c_void;

#[cfg(windows)]
mod dl {
    use super::*;
    unsafe extern "system" {
        fn LoadLibraryA(name: *const c_char) -> Ptr;
        fn GetProcAddress(lib: Ptr, name: *const c_char) -> Ptr;
    }
    pub fn open() -> Ptr {
        unsafe { LoadLibraryA(c"OpenCL.dll".as_ptr()) }
    }
    pub fn sym(lib: Ptr, name: &str) -> Ptr {
        let c = CString::new(name).unwrap();
        unsafe { GetProcAddress(lib, c.as_ptr()) }
    }
}
#[cfg(not(windows))]
mod dl {
    use super::*;
    pub fn open() -> Ptr {
        unsafe { libc::dlopen(c"libOpenCL.so.1".as_ptr(), libc::RTLD_NOW) }
    }
    pub fn sym(lib: Ptr, name: &str) -> Ptr {
        let c = CString::new(name).unwrap();
        unsafe { libc::dlsym(lib, c.as_ptr()) }
    }
}

macro_rules! f {
    ($lib:expr, $name:literal, $ty:ty) => {{
        let p = dl::sym($lib, $name);
        assert!(!p.is_null(), concat!("no ", $name));
        unsafe { std::mem::transmute::<Ptr, $ty>(p) }
    }};
}

const CL_DEVICE_TYPE_GPU: u64 = 4;
const CL_DEVICE_NAME: u32 = 0x102B;
const CL_DRIVER_VERSION: u32 = 0x102D;
const CL_DEVICE_VERSION: u32 = 0x102F;
const CL_DEVICE_EXTENSIONS: u32 = 0x1030;
const CL_PROGRAM_BUILD_LOG: u32 = 0x1183;
const CL_PROGRAM_BINARY_SIZES: u32 = 0x1165;
const CL_PROGRAM_BINARIES: u32 = 0x1166;
const CL_PROGRAM_KERNEL_NAMES: u32 = 0x1168;
const CL_DEVICE_HOST_MEM_CAPABILITIES_INTEL: u32 = 0x4190;
const CL_KERNEL_EXEC_INFO_INDIRECT_HOST_ACCESS_INTEL: u32 = 0x4200;

struct Cl {
    platform: Ptr,
    device: Ptr,
    context: Ptr,
    get_device_info: unsafe extern "C" fn(Ptr, u32, usize, Ptr, *mut usize) -> i32,
    create_program_with_source: unsafe extern "C" fn(Ptr, u32, *const *const c_char, *const usize, *mut i32) -> Ptr,
    create_program_with_binary: unsafe extern "C" fn(Ptr, u32, *const Ptr, *const usize, *const *const u8, *mut i32, *mut i32) -> Ptr,
    build_program: unsafe extern "C" fn(Ptr, u32, *const Ptr, *const c_char, Ptr, Ptr) -> i32,
    compile_program: unsafe extern "C" fn(Ptr, u32, *const Ptr, *const c_char, u32, *const Ptr, *const *const c_char, Ptr, Ptr) -> i32,
    link_program: unsafe extern "C" fn(Ptr, u32, *const Ptr, *const c_char, u32, *const Ptr, Ptr, Ptr, *mut i32) -> Ptr,
    get_program_info: unsafe extern "C" fn(Ptr, u32, usize, Ptr, *mut usize) -> i32,
    get_program_build_info: unsafe extern "C" fn(Ptr, Ptr, u32, usize, Ptr, *mut usize) -> i32,
    create_kernel: unsafe extern "C" fn(Ptr, *const c_char, *mut i32) -> Ptr,
    create_command_queue: unsafe extern "C" fn(Ptr, Ptr, u64, *mut i32) -> Ptr,
    enqueue_nd_range_kernel: unsafe extern "C" fn(Ptr, Ptr, u32, *const usize, *const usize, *const usize, u32, *const Ptr, *mut Ptr) -> i32,
    finish: unsafe extern "C" fn(Ptr) -> i32,
    set_kernel_exec_info: unsafe extern "C" fn(Ptr, u32, usize, *const c_void) -> i32,
    ext: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
}

impl Cl {
    fn new() -> Cl {
        let lib = dl::open();
        assert!(!lib.is_null(), "no OpenCL library");
        let get_platform_ids = f!(lib, "clGetPlatformIDs", unsafe extern "C" fn(u32, *mut Ptr, *mut u32) -> i32);
        let get_device_ids = f!(lib, "clGetDeviceIDs", unsafe extern "C" fn(Ptr, u64, u32, *mut Ptr, *mut u32) -> i32);
        let create_context = f!(lib, "clCreateContext", unsafe extern "C" fn(*const isize, u32, *const Ptr, Ptr, Ptr, *mut i32) -> Ptr);
        let mut n = 0u32;
        unsafe { get_platform_ids(0, std::ptr::null_mut(), &mut n) };
        let mut platforms = vec![std::ptr::null_mut(); n as usize];
        unsafe { get_platform_ids(n, platforms.as_mut_ptr(), std::ptr::null_mut()) };
        for platform in platforms {
            let mut device = std::ptr::null_mut();
            if unsafe { get_device_ids(platform, CL_DEVICE_TYPE_GPU, 1, &mut device, std::ptr::null_mut()) } != 0 {
                continue;
            }
            let mut e = 0;
            let context = unsafe { create_context(std::ptr::null(), 1, &device, std::ptr::null_mut(), std::ptr::null_mut(), &mut e) };
            assert_eq!(e, 0, "clCreateContext");
            return Cl {
                platform,
                device,
                context,
                get_device_info: f!(lib, "clGetDeviceInfo", _),
                create_program_with_source: f!(lib, "clCreateProgramWithSource", _),
                create_program_with_binary: f!(lib, "clCreateProgramWithBinary", _),
                build_program: f!(lib, "clBuildProgram", _),
                compile_program: f!(lib, "clCompileProgram", _),
                link_program: f!(lib, "clLinkProgram", _),
                get_program_info: f!(lib, "clGetProgramInfo", _),
                get_program_build_info: f!(lib, "clGetProgramBuildInfo", _),
                create_kernel: f!(lib, "clCreateKernel", _),
                create_command_queue: f!(lib, "clCreateCommandQueue", _),
                enqueue_nd_range_kernel: f!(lib, "clEnqueueNDRangeKernel", _),
                finish: f!(lib, "clFinish", _),
                set_kernel_exec_info: f!(lib, "clSetKernelExecInfo", _),
                ext: f!(lib, "clGetExtensionFunctionAddressForPlatform", _),
            };
        }
        panic!("no OpenCL GPU");
    }

    fn info(&self, what: u32) -> String {
        let mut n = 0;
        unsafe { (self.get_device_info)(self.device, what, 0, std::ptr::null_mut(), &mut n) };
        let mut v = vec![0u8; n];
        unsafe { (self.get_device_info)(self.device, what, n, v.as_mut_ptr().cast(), std::ptr::null_mut()) };
        String::from_utf8_lossy(&v).trim_end_matches('\0').to_owned()
    }

    fn log(&self, p: Ptr) -> String {
        let mut n = 0;
        unsafe { (self.get_program_build_info)(p, self.device, CL_PROGRAM_BUILD_LOG, 0, std::ptr::null_mut(), &mut n) };
        let mut v = vec![0u8; n];
        unsafe { (self.get_program_build_info)(p, self.device, CL_PROGRAM_BUILD_LOG, n, v.as_mut_ptr().cast(), std::ptr::null_mut()) };
        let log = String::from_utf8_lossy(&v).into_owned();
        log.lines().filter(|l| l.contains("error") || l.contains("warning: ") && l.contains("inline")).take(20).collect::<Vec<_>>().join("\n")
    }

    fn source(&self, text: &str) -> Ptr {
        let c = CString::new(text).unwrap();
        let p = c.as_ptr();
        let mut e = 0;
        let prog = unsafe { (self.create_program_with_source)(self.context, 1, &p, std::ptr::null(), &mut e) };
        assert_eq!(e, 0, "clCreateProgramWithSource");
        prog
    }

    fn binary(&self, p: Ptr) -> Vec<u8> {
        let mut size = 0usize;
        unsafe { (self.get_program_info)(p, CL_PROGRAM_BINARY_SIZES, 8, (&raw mut size).cast(), std::ptr::null_mut()) };
        let mut bin = vec![0u8; size];
        let mut ptr = bin.as_mut_ptr();
        unsafe { (self.get_program_info)(p, CL_PROGRAM_BINARIES, 8, (&raw mut ptr).cast(), std::ptr::null_mut()) };
        bin
    }

    fn kernels(&self, p: Ptr) -> String {
        let mut n = 0;
        unsafe { (self.get_program_info)(p, CL_PROGRAM_KERNEL_NAMES, 0, std::ptr::null_mut(), &mut n) };
        let mut v = vec![0u8; n];
        unsafe { (self.get_program_info)(p, CL_PROGRAM_KERNEL_NAMES, n, v.as_mut_ptr().cast(), std::ptr::null_mut()) };
        String::from_utf8_lossy(&v).trim_end_matches('\0').to_owned()
    }
}

/// The process's CPU time (user and kernel, every thread), in seconds.
#[cfg(windows)]
fn cpu_seconds() -> f64 {
    unsafe extern "system" {
        fn GetCurrentProcess() -> Ptr;
        fn GetProcessTimes(p: Ptr, created: *mut u64, exited: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
    }
    let (mut a, mut b, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut k, &mut u) };
    (k + u) as f64 * 1e-7
}
#[cfg(not(windows))]
fn cpu_seconds() -> f64 {
    let mut r: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut r) };
    let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 * 1e-6;
    s(r.ru_utime) + s(r.ru_stime)
}

/// A file's text and the options its dump recorded.
fn read(path: &str) -> (String, String) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let options = text.lines().next().and_then(|l| l.strip_prefix("// options: ")).unwrap_or("").to_owned();
    (text, options)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `CLC_SECONDS` (default 300): the process exits (124) past it, so a compile that does not end is bounded.
    let bound: f64 = std::env::var("CLC_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(300.0);
    let what = args.join(" ");
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs_f64(bound));
        println!("clc {what}: not done after {bound} s");
        std::process::exit(124);
    });
    let cl = Cl::new();
    match args.first().map(String::as_str) {
        Some("info") | None => {
            println!("device  {}", cl.info(CL_DEVICE_NAME));
            println!("driver  {}", cl.info(CL_DRIVER_VERSION));
            println!("version {}", cl.info(CL_DEVICE_VERSION));
            for e in cl.info(CL_DEVICE_EXTENSIONS).split_whitespace() {
                println!("ext     {e}");
            }
        }
        Some("builds") => {
            let fresh = std::env::var("CLC_FRESH").is_ok();
            let mut cl = cl;
            for path in &args[1..] {
                if fresh {
                    cl = Cl::new();
                }
                let (text, options) = read(path);
                let p = cl.source(&text);
                let o = CString::new(options.as_str()).unwrap();
                let (t, c) = (Instant::now(), cpu_seconds());
                let e = unsafe { (cl.build_program)(p, 1, &cl.device, o.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut()) };
                let (w, c) = (t.elapsed().as_secs_f64(), cpu_seconds() - c);
                let bin = cl.binary(p);
                let h = bin.iter().fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3));
                println!("build {path}: {e} in {w:.2} s, {c:.2} s of CPU, binary {} bytes {h:016x}", bin.len());
            }
        }
        Some("build") => {
            let (text, mut options) = read(&args[1]);
            if let Some(more) = args.get(2) {
                options = format!("{options} {more}");
            }
            let p = cl.source(&text);
            let o = CString::new(options.as_str()).unwrap();
            let t = Instant::now();
            let e = unsafe { (cl.build_program)(p, 1, &cl.device, o.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut()) };
            let s = t.elapsed().as_secs_f64();
            let bin = cl.binary(p);
            println!("build {} [{options}]: {e} in {s:.2} s, binary {} bytes, kernels {}", args[1], bin.len(), cl.kernels(p));
            if e != 0 {
                println!("{}", cl.log(p));
            } else {
                std::fs::write(format!("{}.bin", args[1]), &bin).unwrap();
            }
        }
        Some("load") => {
            let bin = std::fs::read(&args[1]).unwrap();
            let options = CString::new(args.get(2).cloned().unwrap_or_default()).unwrap();
            let t = Instant::now();
            let (n, b) = (bin.len(), bin.as_ptr());
            let (mut status, mut e) = (0, 0);
            let p = unsafe { (cl.create_program_with_binary)(cl.context, 1, &cl.device, &n, &b, &mut status, &mut e) };
            let e2 = unsafe { (cl.build_program)(p, 1, &cl.device, options.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut()) };
            println!("load {}: {e}/{status}/{e2} in {:.3} s, kernels {}", args[1], t.elapsed().as_secs_f64(), cl.kernels(p));
        }
        Some("link") => {
            let options = &args[1];
            let mut compiled = Vec::new();
            for path in &args[2..] {
                let (text, recorded) = read(path);
                let o = CString::new(format!("{recorded} {options}")).unwrap();
                let p = cl.source(&text);
                let t = Instant::now();
                let e = unsafe { (cl.compile_program)(p, 1, &cl.device, o.as_ptr(), 0, std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut()) };
                println!("compile {path}: {e} in {:.2} s", t.elapsed().as_secs_f64());
                if e != 0 {
                    println!("{}", cl.log(p));
                    return;
                }
                compiled.push(p);
            }
            let o = CString::new("").unwrap();
            let t = Instant::now();
            let mut e = 0;
            let p = unsafe { (cl.link_program)(cl.context, 1, &cl.device, o.as_ptr(), compiled.len() as u32, compiled.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), &mut e) };
            println!("link: {e} in {:.2} s, kernels {}", t.elapsed().as_secs_f64(), if p.is_null() { String::new() } else { cl.kernels(p) });
            if e != 0 && !p.is_null() {
                println!("{}", cl.log(p));
            }
        }
        Some("usm") => usm(&cl, args.get(1).and_then(|v| v.parse().ok()).unwrap_or(64)),
        Some(other) => panic!("clc {other}: info, build, load, link or usm"),
    }
}

/// Host USM: the host's pages with a device address. A directory (itself host USM) holds the address of a block (host
/// USM); a kernel reads the block through it and writes into a third, as the engine's reads of a block do.
fn usm(cl: &Cl, mb: usize) {
    let mut caps = 0u64;
    unsafe { (cl.get_device_info)(cl.device, CL_DEVICE_HOST_MEM_CAPABILITIES_INTEL, 8, (&raw mut caps).cast(), std::ptr::null_mut()) };
    println!("host USM capabilities {caps:#x} (1 access, 2 atomic, 4 concurrent, 8 concurrent atomic)");
    let ext = |name: &str| {
        let c = CString::new(name).unwrap();
        let p = unsafe { (cl.ext)(cl.platform, c.as_ptr()) };
        assert!(!p.is_null(), "no {name}");
        p
    };
    let alloc: unsafe extern "C" fn(Ptr, *const u64, usize, u32, *mut i32) -> Ptr = unsafe { std::mem::transmute(ext("clHostMemAllocINTEL")) };
    let set_ptr: unsafe extern "C" fn(Ptr, u32, *const c_void) -> i32 = unsafe { std::mem::transmute(ext("clSetKernelArgMemPointerINTEL")) };
    let set_info = cl.set_kernel_exec_info;
    let n = mb << 17; // longs
    let bytes = n * 8;
    let mut e = 0;
    let t = Instant::now();
    let block = unsafe { alloc(cl.context, std::ptr::null(), bytes, 4096, &mut e) } as *mut i64;
    println!("clHostMemAllocINTEL {mb} MB: {e} in {:.3} s at {block:?}", t.elapsed().as_secs_f64());
    let out = unsafe { alloc(cl.context, std::ptr::null(), bytes, 4096, &mut e) } as *mut i64;
    let dir = unsafe { alloc(cl.context, std::ptr::null(), 64, 64, &mut e) } as *mut u64;
    let t = Instant::now();
    for i in 0..n {
        unsafe { *block.add(i) = i as i64 * 3 };
    }
    unsafe { *dir = block as u64 };
    unsafe { *dir.add(1) = n as u64 };
    println!("host wrote {mb} MB in {:.3} s", t.elapsed().as_secs_f64());
    let src = "typedef struct { global const long* p; ulong n; } Dir;\n\
               kernel void k(global const Dir* d, global long* out) { size_t i = get_global_id(0); if (i < d->n) out[i] = d->p[i] + 1; }\n";
    let p = cl.source(src);
    let o = CString::new("-cl-std=CL3.0").unwrap();
    assert_eq!(unsafe { (cl.build_program)(p, 1, &cl.device, o.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut()) }, 0, "{}", cl.log(p));
    let k = unsafe { (cl.create_kernel)(p, c"k".as_ptr(), &mut e) };
    assert_eq!(e, 0);
    let yes: u32 = 1;
    println!("indirect host access: {}", unsafe { set_info(k, CL_KERNEL_EXEC_INFO_INDIRECT_HOST_ACCESS_INTEL, 4, (&raw const yes).cast()) });
    println!("arg 0: {}, arg 1: {}", unsafe { set_ptr(k, 0, dir.cast()) }, unsafe { set_ptr(k, 1, out.cast()) });
    let q = unsafe { (cl.create_command_queue)(cl.context, cl.device, 0, &mut e) };
    for round in 0..2 {
        let t = Instant::now();
        let r = unsafe { (cl.enqueue_nd_range_kernel)(q, k, 1, std::ptr::null(), &n, std::ptr::null(), 0, std::ptr::null(), std::ptr::null_mut()) };
        let f = unsafe { (cl.finish)(q) };
        let s = t.elapsed().as_secs_f64();
        let wrong = (0..n).filter(|&i| unsafe { *out.add(i) } != i as i64 * 3 + 1).count();
        println!("round {round}: launch {r}, finish {f}, {s:.4} s ({:.1} GB/s read+write), {wrong} wrong of {n}", 2.0 * bytes as f64 / s / 1e9);
    }
}
