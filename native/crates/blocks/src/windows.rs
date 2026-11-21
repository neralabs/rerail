//! Windows' calls for what blocks and scratch regions map, one for each `libc` call they make on Unix:
//! - an anonymous mapping (`mmap` of `MAP_ANON | MAP_PRIVATE`): `VirtualAlloc` of address space alone
//!   (`MEM_RESERVE`), its pages committed as they are first touched, as `mmap`'s are. A touch of a page not yet
//!   committed faults; this crate's vectored exception handler ([`on_fault`]) commits its granule
//!   ([`GRANULE`]) and the access runs again. Committed whole, a block's reservation (16 GiB, decision 0445) and a
//!   thread's scratch (64 GiB) counted against the commit limit from the start: a few processes ran the machine out
//!   of it (2026-11-20).
//! - a file's shared mapping (`mmap` of `MAP_SHARED`): `CreateFileMappingW` and `MapViewOfFile` over the file's
//!   handle (`as_raw_handle`); the mapping's handle is closed once the view holds it.
//! - `mprotect(PROT_READ)`: `VirtualProtect(PAGE_READONLY)` of the committed pages (it refuses others), the
//!   region marked frozen: a page committed after it is committed read-only.
//! - `munmap`: `VirtualFree(MEM_RELEASE)` for an anonymous region, `UnmapViewOfFile` for a file's view.
//! - `msync`: `FlushViewOfFile` (which does not wait for the disk; `File::sync_data` does).
//! - `madvise(MADV_FREE)`: `VirtualFree(MEM_DECOMMIT)`: the pages and their commit charge are given back, and a
//!   later touch commits zero pages again (a reader reads them again only after writing them).
//! - a device's wrap of host pages (a driver pins them, and takes no fault of ours): [`commit`] first.
use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Foundation::CloseHandle;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, FlushViewOfFile, MEM_COMMIT, MEM_DECOMMIT, MEM_RELEASE, MEM_RESERVE, MEMORY_BASIC_INFORMATION,
    MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE, UnmapViewOfFile, VirtualAlloc, VirtualFree, VirtualProtect, VirtualQuery,
};

/// The bytes a fault commits at once: a few pages, so a sequential writer faults once a granule, not once a page.
pub const GRANULE: usize = 64 << 10;

/// A reservation `anon` made: its range, and whether it is frozen (its pages read-only).
struct Region {
    start: usize,
    end: usize,
    frozen: AtomicBool,
}

/// Every live reservation `anon` made, by start: what the fault handler commits pages of.
static REGIONS: RwLock<Vec<Region>> = RwLock::new(Vec::new());
static HANDLER: std::sync::Once = std::sync::Once::new();

#[repr(C)]
struct ExceptionRecord {
    code: u32,
    flags: u32,
    record: *mut ExceptionRecord,
    address: *mut std::ffi::c_void,
    parameters: u32,
    information: [usize; 15],
}

#[repr(C)]
struct ExceptionPointers {
    record: *mut ExceptionRecord,
    context: *mut std::ffi::c_void,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn AddVectoredExceptionHandler(first: u32, handler: unsafe extern "system" fn(*mut ExceptionPointers) -> i32) -> *mut std::ffi::c_void;
}

const ACCESS_VIOLATION: u32 = 0xC000_0005;
const CONTINUE_EXECUTION: i32 = -1;
const CONTINUE_SEARCH: i32 = 0;

/// The vectored exception handler: an access violation on a page of a reservation that is not committed commits its
/// granule (read-only in a frozen region) and runs the access again; any other fault is the next handler's (a write
/// to a frozen page faults on, as `mprotect`'s does).
unsafe extern "system" fn on_fault(info: *mut ExceptionPointers) -> i32 {
    // SAFETY: the system's record of the fault, valid for the call.
    let record = unsafe { &*(*info).record };
    if record.code != ACCESS_VIOLATION || record.parameters < 2 {
        return CONTINUE_SEARCH;
    }
    let (write, at) = (record.information[0] == 1, record.information[1]);
    let Ok(regions) = REGIONS.read() else { return CONTINUE_SEARCH };
    let k = regions.partition_point(|r| r.start <= at);
    let Some(region) = k.checked_sub(1).map(|k| &regions[k]).filter(|r| at < r.end) else { return CONTINUE_SEARCH };
    match committed(at) {
        // Another thread committed the granule between the fault and this look: the access runs again, unless it
        // is a write to a frozen page (a write `mprotect` refuses too).
        Some(protect) => return if write && protect == PAGE_READONLY { CONTINUE_SEARCH } else { CONTINUE_EXECUTION },
        None => {}
    }
    let from = (at & !(GRANULE - 1)).max(region.start);
    let to = (from + GRANULE).min(region.end);
    let protect = if region.frozen.load(Ordering::Acquire) { PAGE_READONLY } else { PAGE_READWRITE };
    // SAFETY: pages of the reservation, reserved and not yet committed (another thread's commit of them meanwhile is
    // one of the same pages: committing committed pages succeeds).
    let p = unsafe { VirtualAlloc(from as *const _, to - from, MEM_COMMIT, protect) };
    if p.is_null() { CONTINUE_SEARCH } else { CONTINUE_EXECUTION }
}

/// The protection of the page at `at` if it is committed.
fn committed(at: usize) -> Option<u32> {
    let mut info = std::mem::MaybeUninit::<MEMORY_BASIC_INFORMATION>::zeroed();
    // SAFETY: the call fills the structure it is given.
    let n = unsafe { VirtualQuery(at as *const _, info.as_mut_ptr(), size_of::<MEMORY_BASIC_INFORMATION>()) };
    let info = unsafe { info.assume_init() };
    (n != 0 && info.State == MEM_COMMIT).then_some(info.Protect)
}

/// Each committed run of `[base, base + len)` and its pages not committed: `f(start, len, committed)`.
fn runs(base: *mut u8, len: usize, mut f: impl FnMut(usize, usize, bool)) {
    let (mut at, end) = (base as usize, base as usize + len);
    while at < end {
        let mut info = std::mem::MaybeUninit::<MEMORY_BASIC_INFORMATION>::zeroed();
        // SAFETY: the call fills the structure it is given.
        let n = unsafe { VirtualQuery(at as *const _, info.as_mut_ptr(), size_of::<MEMORY_BASIC_INFORMATION>()) };
        if n == 0 {
            return;
        }
        let info = unsafe { info.assume_init() };
        let run_end = (info.BaseAddress as usize + info.RegionSize).min(end);
        f(at, run_end - at, info.State == MEM_COMMIT);
        at = run_end.max(at + 1);
    }
}

/// `len` bytes of address space reading zero, committed as they are touched (module doc).
pub fn anon(len: usize) -> io::Result<*mut u8> {
    HANDLER.call_once(|| {
        // SAFETY: a handler for the process's life, first in the chain: a reservation's fault is ours alone.
        let h = unsafe { AddVectoredExceptionHandler(1, on_fault) };
        assert!(!h.is_null(), "AddVectoredExceptionHandler");
    });
    // SAFETY: a new region the system chooses; nothing is passed in.
    let p = unsafe { VirtualAlloc(std::ptr::null(), len, MEM_RESERVE, PAGE_NOACCESS) };
    if p.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut regions = REGIONS.write().unwrap_or_else(|e| e.into_inner());
    let start = p as usize;
    let k = regions.partition_point(|r| r.start < start);
    regions.insert(k, Region { start, end: start + len, frozen: AtomicBool::new(false) });
    Ok(p.cast())
}

/// Commits every page of `[base, base + len)` (of a reservation `anon` made) not yet committed, as the region's
/// pages are (read-only once frozen): before a device wraps them, whose driver pins pages and takes no fault.
pub fn commit(base: *mut u8, len: usize) -> io::Result<()> {
    let frozen = {
        let regions = REGIONS.read().unwrap_or_else(|e| e.into_inner());
        let at = base as usize;
        let k = regions.partition_point(|r| r.start <= at);
        match k.checked_sub(1).map(|k| &regions[k]).filter(|r| at < r.end) {
            Some(r) => r.frozen.load(Ordering::Acquire),
            // Not a reservation of ours: committed already (a file's view, a caller's memory).
            None => return Ok(()),
        }
    };
    let protect = if frozen { PAGE_READONLY } else { PAGE_READWRITE };
    let mut result = Ok(());
    runs(base, len, |start, n, done| {
        // SAFETY: reserved pages of the caller's reservation.
        if !done && result.is_ok() && unsafe { VirtualAlloc(start as *const _, n, MEM_COMMIT, protect) }.is_null() {
            result = Err(io::Error::last_os_error());
        }
    });
    result
}

/// The first `len` bytes of `file` (at least that long), mapped shared: written, when `writable`, through to it.
pub fn map_file(file: &File, len: usize, writable: bool) -> io::Result<*mut u8> {
    let (protect, access) = if writable { (PAGE_READWRITE, FILE_MAP_WRITE) } else { (PAGE_READONLY, FILE_MAP_READ) };
    // SAFETY: the file's own handle, open for as long as the call; a mapping of its current length.
    let mapping = unsafe { CreateFileMappingW(file.as_raw_handle(), std::ptr::null(), protect, 0, 0, std::ptr::null()) };
    if mapping.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the mapping just made; the view holds it once made, so its handle is closed either way.
    let view = unsafe { MapViewOfFile(mapping, access, 0, 0, len) };
    let error = io::Error::last_os_error();
    // SAFETY: the mapping's own handle, not used again.
    unsafe { CloseHandle(mapping) };
    if view.Value.is_null() { Err(error) } else { Ok(view.Value.cast()) }
}

/// Makes `file` sparse: a block's file is extended to its reservation (`Block::file`), and room not written then
/// takes no disk, as on Unix.
pub fn sparse(file: &File) -> io::Result<()> {
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
    let mut returned = 0u32;
    // SAFETY: the file's own handle, open for the call; the control takes no buffers.
    let ok = unsafe {
        DeviceIoControl(file.as_raw_handle(), FSCTL_SET_SPARSE, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut returned, std::ptr::null_mut())
    };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// Cuts `file` to `len` bytes, where Windows lets it: it refuses while a view maps the file
/// (`ERROR_USER_MAPPED_FILE`), and the file then stays longer, its tail sparse (`sparse`), the header saying the
/// block's length (`Block::open_file` reads that).
pub fn cut(file: &File, len: u64) -> io::Result<()> {
    const ERROR_USER_MAPPED_FILE: i32 = 1224;
    match file.set_len(len) {
        Err(e) if e.raw_os_error() == Some(ERROR_USER_MAPPED_FILE) => Ok(()),
        r => r,
    }
}

/// Makes `len` bytes from `base` (a region `anon` made) read-only: its committed pages now, those committed later as
/// they are (the region frozen).
pub fn read_only(base: *mut u8, len: usize) -> io::Result<()> {
    {
        let regions = REGIONS.read().unwrap_or_else(|e| e.into_inner());
        let at = base as usize;
        let k = regions.partition_point(|r| r.start <= at);
        if let Some(r) = k.checked_sub(1).map(|k| &regions[k]).filter(|r| at < r.end) {
            r.frozen.store(true, Ordering::Release);
        }
    }
    let mut result = Ok(());
    runs(base, len, |start, n, done| {
        let mut old = 0;
        // SAFETY: committed pages of the caller's own region.
        if done && result.is_ok() && unsafe { VirtualProtect(start as *const _, n, PAGE_READONLY, &mut old) } == 0 {
            result = Err(io::Error::last_os_error());
        }
    });
    result
}

/// Gives back a region `anon` made.
pub fn release(base: *mut u8) {
    {
        let mut regions = REGIONS.write().unwrap_or_else(|e| e.into_inner());
        if let Some(k) = regions.iter().position(|r| r.start == base as usize) {
            regions.remove(k);
        }
    }
    // SAFETY: the caller's own region, which nothing reads after.
    unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
}

/// Unmaps a view `map_file` made.
pub fn unmap_view(base: *mut u8) {
    // SAFETY: the caller's own view, which nothing reads after.
    unsafe { UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: base.cast() }) };
}

/// Writes the dirty pages of `len` bytes from `base` (a view `map_file` made) to the file.
pub fn flush_view(base: *mut u8, len: usize) -> io::Result<()> {
    // SAFETY: a range of the caller's own view.
    if unsafe { FlushViewOfFile(base.cast(), len) } == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// Gives back `len` bytes from `base` (whole pages of a region `anon` made) and their commit charge: they stay the
/// caller's address space, committed again, zeroed, when next touched.
pub fn reset(base: *mut u8, len: usize) {
    // SAFETY: the caller's own pages, which it reads again only after writing them.
    unsafe { VirtualFree(base.cast(), len, MEM_DECOMMIT) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reservations are address space alone: far more than the commit limit is reserved, and only what is touched
    /// is committed (whole, eight of a thread's scratch regions were 512 GiB of commit, more than any machine's).
    #[test]
    fn a_reservation_commits_only_what_is_touched() {
        let len = 64usize << 30;
        let regions: Vec<*mut u8> = (0..8).map(|_| anon(len).expect("reserved")).collect();
        for &p in &regions {
            // SAFETY: the reservation's first and last bytes, and one in the middle.
            unsafe {
                assert_eq!(*p.add(len / 2), 0, "an untouched page reads zero");
                *p = 1;
                *p.add(len - 1) = 2;
                assert_eq!((*p, *p.add(len - 1)), (1, 2));
            }
            let mut touched = 0;
            runs(p, len, |_, n, done| touched += if done { n } else { 0 });
            assert_eq!(touched, 3 * GRANULE, "three granules committed of {len} bytes");
        }
        for p in regions {
            release(p);
        }
    }

    /// Threads touching one granule at once all go on: the one that commits it, and those that find it committed.
    #[test]
    fn threads_touching_one_granule_at_once_all_go_on() {
        let len = 1usize << 30;
        for round in 0..20 {
            let p = anon(len).unwrap() as usize;
            std::thread::scope(|s| {
                for t in 0..8 {
                    s.spawn(move || {
                        // SAFETY: a byte of the reservation each, all in one granule.
                        unsafe { *((p + GRANULE * round + t * 64) as *mut u8) = t as u8 + 1 };
                    });
                }
            });
            for t in 0..8 {
                assert_eq!(unsafe { *((p + GRANULE * round + t * 64) as *const u8) }, t as u8 + 1);
            }
            release(p as *mut u8);
        }
    }

    /// A frozen region's pages are read-only, those committed after the freeze too; a decommitted page reads zero.
    #[test]
    fn a_frozen_regions_late_pages_are_read_only_and_reset_pages_read_zero() {
        let len = 16 * GRANULE;
        let p = anon(len).unwrap();
        // SAFETY: bytes of the reservation.
        unsafe {
            *p = 7;
            *p.add(4 * GRANULE) = 9;
        }
        reset(unsafe { p.add(4 * GRANULE) }, GRANULE);
        assert_eq!(unsafe { *p.add(4 * GRANULE) }, 0, "a reset page reads zero");
        read_only(p, len).unwrap();
        assert_eq!(committed(p as usize), Some(PAGE_READONLY));
        assert_eq!(unsafe { *p.add(9 * GRANULE) }, 0, "a page first read after the freeze reads zero");
        assert_eq!(committed(p as usize + 9 * GRANULE), Some(PAGE_READONLY), "and is committed read-only");
        commit(unsafe { p.add(12 * GRANULE) }, 2 * GRANULE).unwrap();
        assert_eq!(committed(p as usize + 13 * GRANULE), Some(PAGE_READONLY), "a device's commit keeps the freeze");
        assert_eq!(unsafe { *p }, 7);
        release(p);
    }
}
