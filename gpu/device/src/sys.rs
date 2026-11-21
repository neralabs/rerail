//! What the backends ask of the operating system, once for each: a shared library loaded and its symbols found (the
//! system's OpenCL library, a CPU kernel library), the page size, and memory mapped with guard pages (the CPU
//! backend's thread stacks). POSIX on macOS and Linux; Win32 on Windows, where OpenCL is the GPU (`OpenCL.dll`, the
//! ICD loader each GPU driver installs).

use std::ffi::{c_void, CString};

/// A loaded library's handle: never unloaded (its functions are held for the process).
#[derive(Clone, Copy)]
pub struct Library(*mut c_void);

// SAFETY: a module handle is the process's, usable from any thread.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl Library {
    /// The library at `path` (a file name the system searches for, or a path), or why not.
    pub fn open(path: &str) -> Result<Library, String> {
        let c = CString::new(path).map_err(|_| format!("a library path with a NUL: {path:?}"))?;
        // SAFETY: a library whose initializers are the system's or a kernel compiler's (none of its own).
        let h = unsafe { imp::open(c.as_ptr()) };
        if h.is_null() { Err(format!("loading {path}: {}", imp::last_error())) } else { Ok(Library(h)) }
    }

    /// The address of its symbol `name`, if it has one.
    pub fn symbol(&self, name: &str) -> Option<*mut c_void> {
        let c = CString::new(name).ok()?;
        // SAFETY: a handle `open` returned, a NUL-terminated name.
        let p = unsafe { imp::symbol(self.0, c.as_ptr()) };
        (!p.is_null()).then_some(p)
    }
}

/// The system's page size.
pub fn page_size() -> usize {
    imp::page_size().max(4096)
}

/// The process's CPU seconds, user and system, every thread's.
pub fn process_cpu_seconds() -> f64 {
    imp::process_cpu_seconds()
}

/// The calling thread's CPU seconds, user and system.
pub fn thread_cpu_seconds() -> f64 {
    imp::thread_cpu_seconds()
}

/// A monotonic clock's seconds, from an arbitrary start: only differences mean anything.
pub fn monotonic_seconds() -> f64 {
    imp::monotonic_seconds()
}

/// `len` bytes of fresh, zeroed memory, readable and writable, with the page at each `stride` (from 0) made
/// inaccessible: a guard below each of the stacks it holds. `None` when the system has no memory for it.
pub fn map_guarded(len: usize, stride: usize) -> Option<*mut u8> {
    let page = page_size();
    // SAFETY: fresh memory of this process, its guards inside it.
    unsafe {
        let p = imp::map(len)?;
        for k in 0..len / stride {
            imp::protect_none(p.add(k * stride), page);
        }
        Some(p)
    }
}

/// Gives back memory `map_guarded` made.
///
/// # Safety
/// `p` and `len` are one `map_guarded` mapping's, used no more.
pub unsafe fn unmap(p: *mut u8, len: usize) {
    unsafe { imp::unmap(p, len) }
}

#[cfg(unix)]
mod imp {
    use std::ffi::{c_char, c_void, CStr};

    pub unsafe fn open(path: *const c_char) -> *mut c_void {
        unsafe { libc::dlopen(path, libc::RTLD_NOW | libc::RTLD_LOCAL) }
    }
    pub unsafe fn symbol(h: *mut c_void, name: *const c_char) -> *mut c_void {
        unsafe { libc::dlsym(h, name) }
    }
    pub fn last_error() -> String {
        // SAFETY: dlerror's message, or null.
        let e = unsafe { libc::dlerror() };
        if e.is_null() { "unknown".to_owned() } else { unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned() }
    }
    pub fn page_size() -> usize {
        unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(0) as usize
    }
    pub unsafe fn map(len: usize) -> Option<*mut u8> {
        let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0) };
        (p != libc::MAP_FAILED).then_some(p.cast())
    }
    pub unsafe fn protect_none(p: *mut u8, len: usize) {
        unsafe { libc::mprotect(p.cast(), len, libc::PROT_NONE) };
    }
    pub unsafe fn unmap(p: *mut u8, len: usize) {
        unsafe { libc::munmap(p.cast(), len) };
    }
    pub fn process_cpu_seconds() -> f64 {
        // SAFETY: fills the struct.
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
        let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 * 1e-6;
        s(u.ru_utime) + s(u.ru_stime)
    }
    pub fn thread_cpu_seconds() -> f64 {
        let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: fills the struct.
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
        t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
    }
    pub fn monotonic_seconds() -> f64 {
        let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: fills the struct.
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t) };
        t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::{c_char, c_void};

    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_RELEASE: u32 = 0x8000;
    const PAGE_READWRITE: u32 = 0x04;
    const PAGE_NOACCESS: u32 = 0x01;

    #[repr(C)]
    struct SystemInfo {
        processor_architecture: u16,
        reserved: u16,
        page_size: u32,
        minimum_application_address: *mut c_void,
        maximum_application_address: *mut c_void,
        active_processor_mask: usize,
        number_of_processors: u32,
        processor_type: u32,
        allocation_granularity: u32,
        processor_level: u16,
        processor_revision: u16,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryA(name: *const c_char) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn GetLastError() -> u32;
        fn GetSystemInfo(info: *mut SystemInfo);
        fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualProtect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
        fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
        fn GetCurrentThread() -> *mut c_void;
        fn GetThreadTimes(thread: *mut c_void, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn GetProcessTimes(process: *mut c_void, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn QueryPerformanceCounter(count: *mut i64) -> i32;
        fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
    }

    pub unsafe fn open(path: *const c_char) -> *mut c_void {
        unsafe { LoadLibraryA(path) }
    }
    pub unsafe fn symbol(h: *mut c_void, name: *const c_char) -> *mut c_void {
        unsafe { GetProcAddress(h, name) }
    }
    pub fn last_error() -> String {
        // SAFETY: the calling thread's last error code.
        format!("Windows error {}", unsafe { GetLastError() })
    }
    pub fn page_size() -> usize {
        // SAFETY: GetSystemInfo fills the structure.
        let mut s: SystemInfo = unsafe { std::mem::zeroed() };
        unsafe { GetSystemInfo(&mut s) };
        s.page_size as usize
    }
    pub unsafe fn map(len: usize) -> Option<*mut u8> {
        let p = unsafe { VirtualAlloc(std::ptr::null_mut(), len, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        (!p.is_null()).then_some(p.cast())
    }
    pub unsafe fn protect_none(p: *mut u8, len: usize) {
        let mut old = 0;
        unsafe { VirtualProtect(p.cast(), len, PAGE_NOACCESS, &mut old) };
    }
    pub unsafe fn unmap(p: *mut u8, _len: usize) {
        unsafe { VirtualFree(p.cast(), 0, MEM_RELEASE) };
    }
    pub fn process_cpu_seconds() -> f64 {
        // FILETIMEs: 100 ns units (two 32-bit halves, little-endian, read as one u64).
        let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
        // SAFETY: the process's own times, into four locals.
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
        (k + u) as f64 * 1e-7
    }
    pub fn thread_cpu_seconds() -> f64 {
        let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
        // SAFETY: the calling thread's own times, into four locals.
        unsafe { GetThreadTimes(GetCurrentThread(), &mut c, &mut e, &mut k, &mut u) };
        (k + u) as f64 * 1e-7
    }
    pub fn monotonic_seconds() -> f64 {
        let (mut n, mut f) = (0i64, 1i64);
        // SAFETY: two locals filled.
        unsafe { (QueryPerformanceCounter(&mut n), QueryPerformanceFrequency(&mut f)) };
        n as f64 / f.max(1) as f64
    }
}
