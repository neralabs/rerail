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

/// A column a worktree writes without copying it (L6, L10): the base's file mapped privately, so
/// every worktree reads the base's pages in place and the kernel copies a page (16 KB here) only
/// when this worktree first writes it. The mapping reserves `cap` elements; elements past the
/// file (ids local to the worktree) are anonymous pages, untouched until written. What the
/// worktree owns is measured from the kernel (`private_bytes`), not estimated.
pub struct CowCol<T: Elem> {
    ptr: *mut T,
    len: usize,
    cap: usize,
    reserved: usize,
}

// SAFETY: one owner writes it (a worktree's state); no other thread holds a reference.
unsafe impl<T: Elem> Send for CowCol<T> {}

pub fn page_size() -> usize {
    // SAFETY: plain query.
    (unsafe { libc::sysconf(libc::_SC_PAGESIZE) }) as usize
}

impl<T: Elem> CowCol<T> {
    /// The file at `path` (its first `len` elements are the column) mapped privately, with room
    /// for `cap` elements. The file is a whole number of pages (`write_padded`).
    pub fn map(path: &Path, len: usize, cap: usize) -> CowCol<T> {
        let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let bytes = f.metadata().unwrap().len() as usize;
        let sz = std::mem::size_of::<T>();
        let ps = page_size();
        assert!(bytes % ps == 0 && len * sz <= bytes, "{}: not a padded column", path.display());
        let reserved = (cap * sz).max(bytes).div_ceil(ps) * ps;
        use std::os::fd::AsRawFd;
        // SAFETY: a fresh anonymous reservation, then the file mapped privately over its start.
        unsafe {
            let r = libc::mmap(std::ptr::null_mut(), reserved, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0);
            assert!(r != libc::MAP_FAILED, "reserve");
            if bytes > 0 {
                let p = libc::mmap(r, bytes, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_FIXED, f.as_raw_fd(), 0);
                assert!(p == r, "{}: private mapping", path.display());
            }
            CowCol { ptr: r as *mut T, len, cap: reserved / sz, reserved }
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Extend to `len` elements, the new ones set to `fill`.
    pub fn grow(&mut self, len: usize, fill: T) {
        assert!(len <= self.cap, "CowCol: past its reservation ({len} > {})", self.cap);
        for i in self.len..len {
            // SAFETY: within the reservation.
            unsafe { *self.ptr.add(i) = fill };
        }
        self.len = len.max(self.len);
    }

    pub fn as_slice(&self) -> &[T] {
        // SAFETY: `len` ≤ the reservation, mapped readable.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: as above; the mapping is private and writable.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    /// Bytes this mapping owns: pages resident and anonymous (copied from the file on a write,
    /// or written past it), from `mincore` (Darwin: `MINCORE_INCORE` 0x1, `MINCORE_ANONYMOUS`
    /// 0x80).
    pub fn private_bytes(&self) -> usize {
        let ps = page_size();
        let pages = self.reserved / ps;
        let mut v = vec![0 as libc::c_char; pages];
        // SAFETY: the whole reservation, one byte per page.
        let r = unsafe { libc::mincore(self.ptr as *const libc::c_void, self.reserved, v.as_mut_ptr()) };
        assert_eq!(r, 0, "mincore");
        v.iter().filter(|&&b| (b as u8) & 0x81 == 0x81).count() * ps
    }
}

impl<T: Elem> Drop for CowCol<T> {
    fn drop(&mut self) {
        // SAFETY: the reservation this value mapped.
        unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.reserved) };
    }
}

/// `write`, padded with `pad` to a whole number of pages (what `CowCol::map` maps).
pub fn write_padded<T: Elem>(path: &Path, v: &[T], pad: T) {
    let sz = std::mem::size_of::<T>();
    let n = (v.len() * sz).div_ceil(page_size()) * page_size() / sz;
    let mut w = v.to_vec();
    w.resize(n.max(page_size() / sz), pad);
    write(path, &w);
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
