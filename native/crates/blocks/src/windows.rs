//! Windows' calls for what blocks and scratch regions map, one for each `libc` call they make on Unix:
//! - an anonymous reservation (`mmap` of `MAP_ANON | MAP_PRIVATE | MAP_NORESERVE`): `VirtualAlloc` of `MEM_RESERVE`,
//!   address space only. Windows commits no page on a first touch, as Linux and macOS do: a page is written only
//!   once committed (`commit`, `MEM_COMMIT`), and each block and scratch region commits what it allocates as its top
//!   passes it (`Block::alloc_filled`, `Scratch::take`). A committed page is demand-zero: none is resident until
//!   written. Committing the whole reservation at once, as this did, counted a block of relations' 16 GiB (decision
//!   0445) against the system's commit limit from the start ("the paging file is too small").
//! - a file's shared mapping (`mmap` of `MAP_SHARED`): `CreateFileMappingW` and `MapViewOfFile` over the file's
//!   handle (`as_raw_handle`); the mapping's handle is closed once the view holds it. A block's file is sparse
//!   (`sparse`), so its reservation takes no disk until written, as a Unix file's hole.
//! - `mprotect(PROT_READ)`: `VirtualProtect(PAGE_READONLY)`, of committed pages only.
//! - `munmap`: `VirtualFree(MEM_RELEASE)` for a whole reservation, `MEM_DECOMMIT` for a range inside one (Windows
//!   releases a reservation only whole, from its base); `UnmapViewOfFile` for a file's view.
//! - `msync` and `fdatasync`: `FlushViewOfFile`, then `FlushFileBuffers`.
//! - `madvise(MADV_FREE)`: `VirtualAlloc(MEM_RESET)`: the pages stay committed and their contents may be dropped
//!   rather than written to the page file, as `MADV_FREE` lets the kernel drop them.
use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, FlushViewOfFile, MEM_COMMIT, MEM_DECOMMIT, MEM_RELEASE, MEM_RESERVE, MEM_RESET,
    MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_READONLY, PAGE_READWRITE, UnmapViewOfFile, VirtualAlloc,
    VirtualFree, VirtualProtect, VirtualQuery,
};

/// `len` bytes of address space, reserved, nothing committed: `commit` makes a range of it memory.
pub fn reserve(len: usize) -> io::Result<*mut u8> {
    // SAFETY: a new region the system chooses; nothing is passed in.
    let p = unsafe { VirtualAlloc(std::ptr::null(), len, MEM_RESERVE, PAGE_READWRITE) };
    if p.is_null() { Err(io::Error::last_os_error()) } else { Ok(p.cast()) }
}

/// Commits `len` bytes from `p` (inside a reservation `reserve` made): fresh pages read zero, and pages committed
/// already are left as they are.
pub fn commit(p: *mut u8, len: usize) -> io::Result<()> {
    // SAFETY: the caller's own reservation.
    let q = unsafe { VirtualAlloc(p.cast(), len, MEM_COMMIT, PAGE_READWRITE) };
    if q.is_null() { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// Marks `file` sparse: what is never written takes no disk, as a Unix file's hole.
pub fn sparse(file: &File) -> io::Result<()> {
    let mut returned = 0u32;
    // SAFETY: the file's own handle; the control takes no input and gives no output.
    let ok = unsafe {
        DeviceIoControl(file.as_raw_handle(), FSCTL_SET_SPARSE, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut returned, std::ptr::null_mut())
    };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
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

/// Makes `len` bytes from `base` (committed pages, or a view) read-only.
pub fn read_only(base: *mut u8, len: usize) -> io::Result<()> {
    let mut old = 0;
    // SAFETY: the caller's own region.
    if unsafe { VirtualProtect(base.cast(), len, PAGE_READONLY, &mut old) } == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// Gives back `len` bytes at `p` of a reservation `reserve` made: the whole reservation when `p` is its base, else
/// the range's pages decommitted (Windows releases a reservation only whole; its address space then goes with the
/// reservation's).
pub fn release(p: *mut u8, len: usize) {
    // SAFETY: a query of the caller's own address.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let got = unsafe { VirtualQuery(p.cast(), &mut info, size_of::<MEMORY_BASIC_INFORMATION>()) };
    // SAFETY: the caller's own region, which nothing reads after.
    unsafe {
        if got != 0 && info.AllocationBase == p.cast() {
            VirtualFree(p.cast(), 0, MEM_RELEASE);
        } else {
            VirtualFree(p.cast(), len, MEM_DECOMMIT);
        }
    }
}

/// Unmaps a view `map_file` made.
pub fn unmap_view(base: *mut u8) {
    // SAFETY: the caller's own view, which nothing reads after.
    unsafe { UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: base.cast() }) };
}

/// Writes a view's first `len` bytes back to its file, and the file to the disk.
pub fn flush_view(base: *mut u8, len: usize, file: Option<&File>) -> io::Result<()> {
    // SAFETY: the caller's own view.
    if unsafe { FlushViewOfFile(base.cast(), len) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the file's own handle.
    if let Some(f) = file
        && unsafe { FlushFileBuffers(f.as_raw_handle()) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Lets the system drop the contents of `len` bytes from `base` (whole committed pages of a region `reserve` made):
/// they stay the caller's, their bytes whatever the system leaves.
pub fn reset(base: *mut u8, len: usize) {
    // SAFETY: the caller's own pages, which it reads again only after writing them.
    unsafe { VirtualAlloc(base.cast(), len, MEM_RESET, PAGE_READWRITE) };
}
