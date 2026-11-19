//! Windows' calls for what blocks and scratch regions map, one for each `libc` call they make on Unix:
//! - an anonymous mapping (`mmap` of `MAP_ANON | MAP_PRIVATE`): `VirtualAlloc`, reserved and committed at once.
//!   Committed pages are demand-zero, as an anonymous mapping's are: none is resident until written. Unlike `mmap`,
//!   the whole length counts against the system's commit limit from the start.
//! - a file's shared mapping (`mmap` of `MAP_SHARED`): `CreateFileMappingW` and `MapViewOfFile` over the file's
//!   handle (`as_raw_handle`); the mapping's handle is closed once the view holds it.
//! - `mprotect(PROT_READ)`: `VirtualProtect(PAGE_READONLY)`.
//! - `munmap`: `VirtualFree(MEM_RELEASE)` for an anonymous region, `UnmapViewOfFile` for a file's view.
//! - `madvise(MADV_FREE)`: `VirtualAlloc(MEM_RESET)`: the pages stay committed and their contents may be dropped
//!   rather than written to the page file, as `MADV_FREE` lets the kernel drop them.
use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, MEM_RESET, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READONLY, PAGE_READWRITE, UnmapViewOfFile, VirtualAlloc, VirtualFree, VirtualProtect,
};

/// `len` bytes of fresh, zeroed memory.
pub fn anon(len: usize) -> io::Result<*mut u8> {
    // SAFETY: a new region the system chooses; nothing is passed in.
    let p = unsafe { VirtualAlloc(std::ptr::null(), len, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
    if p.is_null() { Err(io::Error::last_os_error()) } else { Ok(p.cast()) }
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

/// Makes `len` bytes from `base` (a region `anon` made) read-only.
pub fn read_only(base: *mut u8, len: usize) -> io::Result<()> {
    let mut old = 0;
    // SAFETY: the caller's own region.
    if unsafe { VirtualProtect(base.cast(), len, PAGE_READONLY, &mut old) } == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// Gives back a region `anon` made.
pub fn release(base: *mut u8) {
    // SAFETY: the caller's own region, which nothing reads after.
    unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
}

/// Unmaps a view `map_file` made.
pub fn unmap_view(base: *mut u8) {
    // SAFETY: the caller's own view, which nothing reads after.
    unsafe { UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: base.cast() }) };
}

/// Lets the system drop the contents of `len` bytes from `base` (whole pages of a region `anon` made): they stay
/// the caller's, their bytes whatever the system leaves.
pub fn reset(base: *mut u8, len: usize) {
    // SAFETY: the caller's own pages, which it reads again only after writing them.
    unsafe { VirtualAlloc(base.cast(), len, MEM_RESET, PAGE_READWRITE) };
}
