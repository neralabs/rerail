//! The memory a block or a scratch region reserves for itself, by platform. On Unix, an anonymous mapping: address
//! space only, nothing resident until written, read-only once frozen, pages given back to the OS. Elsewhere (WASI,
//! where the engine also runs) there is no mapping: a zeroed, page-aligned heap allocation of the whole reservation,
//! never protected, its pages kept. Only the reservation's cost differs; what a block holds and how it reads do not.

use crate::block::PAGE;

/// `len` bytes (a multiple of `PAGE`), page-aligned, reading zero.
pub fn reserve(len: usize) -> *mut u8 {
    debug_assert_eq!(len % PAGE, 0, "a reservation of whole pages");
    #[cfg(unix)]
    {
        let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_ANON | libc::MAP_PRIVATE, -1, 0) };
        assert!(p != libc::MAP_FAILED, "mmap of {len} bytes failed");
        p.cast()
    }
    #[cfg(not(unix))]
    {
        let layout = std::alloc::Layout::from_size_align(len, PAGE).expect("a page-aligned reservation");
        let p = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!p.is_null(), "an allocation of {len} bytes failed");
        p
    }
}

/// Gives back what `reserve` (or, on Unix, any mapping of `len` bytes at `p`) took.
///
/// # Safety
/// `p` and `len` are a reservation's (a mapping's), and nothing reads it after.
pub unsafe fn release(p: *mut u8, len: usize) {
    #[cfg(unix)]
    unsafe {
        libc::munmap(p.cast(), len);
    }
    #[cfg(not(unix))]
    unsafe {
        std::alloc::dealloc(p, std::alloc::Layout::from_size_align(len, PAGE).expect("a page-aligned reservation"));
    }
}

/// Makes a reservation read-only, so a write after freeze faults (Unix); elsewhere nothing.
///
/// # Safety
/// `p` and `len` are a reservation's.
pub unsafe fn protect_read(p: *mut u8, len: usize) {
    #[cfg(unix)]
    {
        let r = unsafe { libc::mprotect(p.cast(), len, libc::PROT_READ) };
        assert_eq!(r, 0, "mprotect");
    }
    #[cfg(not(unix))]
    let _ = (p, len);
}

/// Tells the OS a reservation's pages need not be kept (their bytes may be lost); elsewhere nothing.
///
/// # Safety
/// `p` and `len` lie in a reservation, and nothing reads those bytes before writing them again.
pub unsafe fn free_pages(p: *mut u8, len: usize) {
    #[cfg(unix)]
    unsafe {
        libc::madvise(p.cast(), len, libc::MADV_FREE);
    }
    #[cfg(not(unix))]
    let _ = (p, len);
}
