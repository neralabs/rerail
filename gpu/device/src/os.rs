//! The system's calls the backends make, on macOS and Linux and on Windows:
//!
//! - the libraries a backend loads when it is first asked for (OpenCL's, CUDA's driver and NVRTC, the CPU backend's
//!   kernels): `dlopen`, `LoadLibraryExW`. No backend has a link dependency, so a machine without a driver still runs,
//!   with no device of that kind (decision 0440);
//! - the size of a page;
//! - the CPU backend's thread stacks, a guard page below each (`Stacks`).

use std::ffi::c_void;

/// A library this process has open. It stays open for the process's life: the backends keep its functions.
#[derive(Clone, Copy)]
pub struct Lib(*mut c_void);

// SAFETY: a handle the loader gave, valid on every thread.
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

/// Where a library's symbols are seen once it is open. Windows keeps each module's apart either way.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// By every library opened after it (`RTLD_GLOBAL`): CUDA's NVRTC finds the driver's symbols so.
    Global,
    /// By itself alone (`RTLD_LOCAL`): kernel libraries that each define the same names, none bound to another's.
    Local,
}

impl Lib {
    /// The first of `names` the loader opens: a file name the system's search finds, or a path.
    pub fn open<S: AsRef<str>>(names: &[S], scope: Scope) -> Option<Lib> {
        names.iter().find_map(|n| sys::open(n.as_ref(), scope).map(Lib))
    }

    /// `name`'s address in the library, or null where it has none.
    pub fn sym(&self, name: &str) -> *mut c_void {
        sys::sym(self.0, name)
    }

    /// Why the last open on this thread failed, as the loader says (`dlerror`; Windows' last error).
    pub fn error() -> String {
        sys::error()
    }
}

/// Asks NVIDIA's driver for `n` hardware queues (`CUDA_DEVICE_MAX_CONNECTIONS`), before it loads: the CUDA driver and
/// NVIDIA's OpenCL, which is built on it, both read it. Each queue costs the context's creation about 18 ms, in every
/// process (a GB10: CUDA's primary context 0.63 s at 32, 0.19 s at 8, 0.09 s at 2; OpenCL's context 0.65 s at 32,
/// 0.17 s at its default, 0.09 s at 2). A value the environment sets is kept. Other drivers ignore it.
pub fn nvidia_hardware_queues(n: usize) {
    if std::env::var_os("CUDA_DEVICE_MAX_CONNECTIONS").is_none() {
        // SAFETY: called before the driver loads, as the process's first use of the device.
        unsafe { std::env::set_var("CUDA_DEVICE_MAX_CONNECTIONS", n.to_string()) };
    }
}

/// The size of the system's pages.
pub fn page_bytes() -> usize {
    sys::page_bytes().max(4096)
}

/// `len` bytes (whole pages) of fresh memory, zeroed by the system, its pages committed as they are touched where the
/// system does so (`mmap`; Windows commits them at once). Given back with [`unmap`].
pub fn map_zeroed(len: usize) -> Option<*mut u8> {
    sys::map(len)
}

/// `len` bytes (whole pages) of address space, zeroed by the system, committed only as [`commit`] commits a prefix of
/// it (Unix: at once as address space, its pages committed as they are touched). Given back with [`unmap`].
pub fn reserve(len: usize) -> Option<*mut u8> {
    sys::reserve(len)
}

/// `[at, at + len)` of a [`reserve`]d range committed (Windows; Unix commits pages as they are touched): false where
/// the system refused.
///
/// # Safety
/// The range lies in one range `reserve` gave.
pub unsafe fn commit(at: *mut u8, len: usize) -> bool {
    unsafe { sys::commit(at, len) }
}

/// Gives back memory [`map_zeroed`] gave.
///
/// # Safety
/// The whole of one mapping `map_zeroed` gave, `len` its length, read no more.
pub unsafe fn unmap(at: *mut u8, len: usize) {
    unsafe { sys::unmap(at, len) }
}

/// The process's user + system CPU seconds.
pub fn process_cpu() -> f64 {
    sys::process_cpu()
}

/// The calling thread's user + system CPU seconds.
pub fn thread_cpu() -> f64 {
    sys::thread_cpu()
}

/// The host's monotonic clock, in seconds from an unspecified start.
pub fn monotonic() -> f64 {
    sys::monotonic()
}

/// `count` stacks of `stride` bytes each (a whole number of pages), the first page of each inaccessible: a thread that
/// runs past its stack faults there, never writing the stack below. Released when dropped.
pub struct Stacks {
    base: *mut u8,
    len: usize,
}

// SAFETY: memory the process owns, used by one thread at a time (the CPU queue root's).
unsafe impl Send for Stacks {}

impl Stacks {
    /// The stacks, or `None` where the system has no memory for them. On Windows the memory is committed as it is
    /// mapped (charged to the system's commit limit, a page in memory only once a thread touches it): Windows grows
    /// only a thread's own stack, not one a coroutine runs on.
    pub fn new(count: usize, stride: usize) -> Option<Stacks> {
        let page = page_bytes();
        assert!(stride.is_multiple_of(page) && stride > page, "a stack is whole pages, more than its guard");
        let len = count * stride;
        let base = sys::map(len)?;
        for k in 0..count {
            // SAFETY: a page of the mapping just made.
            unsafe { sys::guard(base.add(k * stride), page) };
        }
        Some(Stacks { base, len })
    }

    pub fn base(&self) -> *mut u8 {
        self.base
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for Stacks {
    fn drop(&mut self) {
        // SAFETY: the mapping `new` made, no longer used.
        unsafe { sys::unmap(self.base, self.len) };
    }
}

#[cfg(unix)]
mod sys {
    use super::Scope;
    use std::ffi::{c_void, CStr, CString};

    pub fn open(name: &str, scope: Scope) -> Option<*mut c_void> {
        let c = CString::new(name).ok()?;
        let seen = if scope == Scope::Global { libc::RTLD_GLOBAL } else { libc::RTLD_LOCAL };
        // SAFETY: a library the system's loader opens.
        let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | seen) };
        (!h.is_null()).then_some(h)
    }

    pub fn sym(h: *mut c_void, name: &str) -> *mut c_void {
        let Ok(c) = CString::new(name) else { return std::ptr::null_mut() };
        // SAFETY: a handle `open` gave.
        unsafe { libc::dlsym(h, c.as_ptr()) }
    }

    pub fn error() -> String {
        // SAFETY: the loader's message for this thread's last failure, or null.
        let e = unsafe { libc::dlerror() };
        if e.is_null() { String::new() } else { unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned() }
    }

    pub fn page_bytes() -> usize {
        // SAFETY: a query with no arguments.
        (unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).max(0) as usize
    }

    pub fn process_cpu() -> f64 {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: the call fills the structure.
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
        let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 * 1e-6;
        s(u.ru_utime) + s(u.ru_stime)
    }

    pub fn thread_cpu() -> f64 {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: the call fills the structure.
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
        ts.tv_sec as f64 + ts.tv_nsec as f64 * 1e-9
    }

    pub fn monotonic() -> f64 {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: the call fills the structure.
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        ts.tv_sec as f64 + ts.tv_nsec as f64 * 1e-9
    }

    pub fn map(len: usize) -> Option<*mut u8> {
        // SAFETY: anonymous memory.
        let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0) };
        (p != libc::MAP_FAILED).then_some(p.cast())
    }

    pub fn reserve(len: usize) -> Option<*mut u8> {
        map(len)
    }

    /// # Safety
    /// The caller's (`super::commit`): nothing to do, pages are committed as they are touched.
    pub unsafe fn commit(_at: *mut u8, _len: usize) -> bool {
        true
    }

    /// # Safety
    /// `[at, at + len)` is whole pages of a mapping `map` made.
    pub unsafe fn guard(at: *mut u8, len: usize) {
        unsafe { libc::mprotect(at.cast(), len, libc::PROT_NONE) };
    }

    /// # Safety
    /// The whole of a mapping `map` made, no longer used.
    pub unsafe fn unmap(at: *mut u8, len: usize) {
        unsafe { libc::munmap(at.cast(), len) };
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::{c_void, CString};

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
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const i8) -> *mut c_void;
        fn GetLastError() -> u32;
        fn GetSystemInfo(info: *mut SystemInfo);
        fn VirtualAlloc(at: *mut c_void, len: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualProtect(at: *mut c_void, len: usize, protect: u32, old: *mut u32) -> i32;
        fn VirtualFree(at: *mut c_void, len: usize, kind: u32) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
        fn GetCurrentThread() -> *mut c_void;
        fn GetProcessTimes(process: *mut c_void, created: *mut u64, exited: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn GetThreadTimes(thread: *mut c_void, created: *mut u64, exited: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn QueryPerformanceCounter(count: *mut i64) -> i32;
        fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
    }

    pub fn monotonic() -> f64 {
        let (mut count, mut frequency) = (0i64, 0i64);
        // SAFETY: each call fills its integer (both succeed on every Windows since XP).
        unsafe { QueryPerformanceCounter(&mut count) };
        unsafe { QueryPerformanceFrequency(&mut frequency) };
        count as f64 / frequency.max(1) as f64
    }

    /// Kernel + user time of a process or thread, in seconds (the calls count in 100 ns units).
    fn times(get: unsafe extern "system" fn(*mut c_void, *mut u64, *mut u64, *mut u64, *mut u64) -> i32, of: *mut c_void) -> f64 {
        let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
        // SAFETY: a pseudo-handle of this process or thread; the call fills the four times.
        if unsafe { get(of, &mut c, &mut e, &mut k, &mut u) } == 0 {
            return 0.0;
        }
        (k + u) as f64 * 1e-7
    }

    pub fn process_cpu() -> f64 {
        // SAFETY: a pseudo-handle, never closed.
        times(GetProcessTimes, unsafe { GetCurrentProcess() })
    }

    pub fn thread_cpu() -> f64 {
        // SAFETY: a pseudo-handle, never closed.
        times(GetThreadTimes, unsafe { GetCurrentThread() })
    }

    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_RELEASE: u32 = 0x8000;
    const PAGE_NOACCESS: u32 = 0x01;
    const PAGE_READWRITE: u32 = 0x04;

    pub fn open(name: &str, _: super::Scope) -> Option<*mut c_void> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // A path's library finds the libraries it needs beside it (a toolkit's: HIP's hiprtc needs its comgr), as
        // `dlopen` finds them through the library's own run path; a bare name, the system's search (System32 for a
        // driver's).
        const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x8;
        let flags = if name.contains(['\\', '/']) { LOAD_WITH_ALTERED_SEARCH_PATH } else { 0 };
        // SAFETY: a NUL-terminated name.
        let h = unsafe { LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), flags) };
        (!h.is_null()).then_some(h)
    }

    pub fn sym(h: *mut c_void, name: &str) -> *mut c_void {
        let Ok(c) = CString::new(name) else { return std::ptr::null_mut() };
        // SAFETY: a module `open` gave.
        unsafe { GetProcAddress(h, c.as_ptr()) }
    }

    pub fn error() -> String {
        // SAFETY: this thread's last error code.
        format!("Windows error {}", unsafe { GetLastError() })
    }

    pub fn page_bytes() -> usize {
        let mut info = std::mem::MaybeUninit::<SystemInfo>::zeroed();
        // SAFETY: the call fills the structure.
        unsafe { GetSystemInfo(info.as_mut_ptr()) };
        unsafe { info.assume_init() }.page_size as usize
    }

    pub fn map(len: usize) -> Option<*mut u8> {
        // SAFETY: fresh memory, zeroed by the system.
        let p = unsafe { VirtualAlloc(std::ptr::null_mut(), len, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
        (!p.is_null()).then_some(p.cast())
    }

    pub fn reserve(len: usize) -> Option<*mut u8> {
        // SAFETY: fresh address space, nothing committed.
        let p = unsafe { VirtualAlloc(std::ptr::null_mut(), len, MEM_RESERVE, PAGE_READWRITE) };
        (!p.is_null()).then_some(p.cast())
    }

    /// # Safety
    /// The caller's (`super::commit`).
    pub unsafe fn commit(at: *mut u8, len: usize) -> bool {
        // SAFETY: within a range `reserve` gave; committed pages are zeroed by the system.
        !unsafe { VirtualAlloc(at.cast(), len, MEM_COMMIT, PAGE_READWRITE) }.is_null()
    }

    /// # Safety
    /// `[at, at + len)` is whole pages of a mapping `map` made.
    pub unsafe fn guard(at: *mut u8, len: usize) {
        let mut old = 0;
        unsafe { VirtualProtect(at.cast(), len, PAGE_NOACCESS, &mut old) };
    }

    /// # Safety
    /// The whole of a mapping `map` made, no longer used.
    pub unsafe fn unmap(at: *mut u8, _len: usize) {
        unsafe { VirtualFree(at.cast(), 0, MEM_RELEASE) };
    }
}
