//! Columns on disk, mapped read-only (L6): a column file is the raw native-endian array; `map`
//! gives a slice over the mapping that every worker reads in place. Nothing is copied: the only
//! path that would copy a shared column (`Col::to_vec`) adds its bytes to `COPIED`, which the
//! harness reports and which must read 0.
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Bytes copied out of shared columns (L6's copy counter).
pub static COPIED: AtomicU64 = AtomicU64::new(0);
/// Bytes mapped (each column once, whatever the number of workers).
pub static MAPPED: AtomicU64 = AtomicU64::new(0);

/// Column element types: plain integers.
pub trait Elem: Copy + 'static {}
impl Elem for u32 {}
impl Elem for u64 {}

pub struct Col<T: Elem> {
    ptr: *const T,
    len: usize,
    map: usize,
}

// SAFETY: a read-only mapping, never written after `map`.
unsafe impl<T: Elem> Sync for Col<T> {}
unsafe impl<T: Elem> Send for Col<T> {}

impl<T: Elem> std::ops::Deref for Col<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: `map` mapped `len` elements at `ptr` (page-aligned, so aligned for T).
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl<T: Elem> Drop for Col<T> {
    fn drop(&mut self) {
        if self.map > 0 {
            unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.map) };
        }
    }
}

impl<T: Elem> Col<T> {
    /// An owned copy, counted.
    #[allow(dead_code)]
    pub fn to_vec(&self) -> Vec<T> {
        COPIED.fetch_add(std::mem::size_of_val::<[T]>(self) as u64, Relaxed);
        <[T]>::to_vec(self)
    }
}

/// Map `path` read-only.
pub fn map<T: Elem>(path: &Path) -> Col<T> {
    let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let bytes = f.metadata().unwrap().len() as usize;
    let sz = std::mem::size_of::<T>();
    assert_eq!(bytes % sz, 0, "{}: not a column of {sz}-byte elements", path.display());
    if bytes == 0 {
        return Col { ptr: std::ptr::null(), len: 0, map: 0 };
    }
    use std::os::fd::AsRawFd;
    // SAFETY: a fresh read-only shared mapping of the whole file.
    let p = unsafe { libc::mmap(std::ptr::null_mut(), bytes, libc::PROT_READ, libc::MAP_SHARED, f.as_raw_fd(), 0) };
    assert!(p != libc::MAP_FAILED, "{}: mmap failed", path.display());
    MAPPED.fetch_add(bytes as u64, Relaxed);
    Col { ptr: p as *const T, len: bytes / sz, map: bytes }
}

pub fn write<T: Elem>(path: &Path, v: &[T]) {
    // SAFETY: plain integers viewed as bytes.
    let b = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) };
    std::fs::write(path, b).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// The process's memory: (peak resident bytes, resident now, private footprint now, peak private
/// footprint). Resident counts mapped column pages once; the private footprint does not count
/// clean file-backed pages, so it is the memory the process owns.
pub fn memory() -> (u64, u64, u64, u64) {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, &mut info as *mut _ as *mut libc::rusage_info_t) };
    assert_eq!(r, 0, "proc_pid_rusage");
    (ru.ru_maxrss as u64, info.ri_resident_size, info.ri_phys_footprint, info.ri_lifetime_max_phys_footprint)
}

/// User + system CPU seconds of the process so far.
pub fn cpu_seconds() -> f64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 * 1e-6;
    t(ru.ru_utime) + t(ru.ru_stime)
}

/// The 1, 5 and 15-minute load averages.
pub fn load_average() -> [f64; 3] {
    let mut l = [0f64; 3];
    unsafe { libc::getloadavg(l.as_mut_ptr(), 3) };
    l
}
