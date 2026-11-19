//! The memory a block or a scratch region reserves for itself, by platform. On Unix, an anonymous mapping: address
//! space only, nothing resident until written, read-only once frozen, pages given back to the OS. On Windows, the
//! same with Windows' calls (`windows`): reserved and committed address space, `VirtualProtect`ed once frozen, its
//! pages reset when given back. Elsewhere (WASI, where the engine also runs) there is no mapping: a zeroed,
//! page-aligned heap allocation of the whole reservation, never protected, its pages kept. Only the reservation's
//! cost differs; what a block holds and how it reads do not.

use crate::block::PAGE;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// The bytes `reserve` holds now (address space, not all of it resident): what a profile reads beside the heap, which
/// never sees these mappings.
static RESERVED: AtomicUsize = AtomicUsize::new(0);

/// The bytes reserved now and not yet released.
pub fn reserved() -> usize {
    RESERVED.load(Relaxed)
}

/// `len` bytes (a multiple of `PAGE`), page-aligned, reading zero.
pub fn reserve(len: usize) -> *mut u8 {
    debug_assert_eq!(len % PAGE, 0, "a reservation of whole pages");
    RESERVED.fetch_add(len, Relaxed);
    #[cfg(unix)]
    {
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_ANON | libc::MAP_PRIVATE | crate::RESERVE_ONLY, -1, 0)
        };
        assert!(p != libc::MAP_FAILED, "mmap of {len} bytes failed");
        p.cast()
    }
    #[cfg(windows)]
    {
        crate::windows::anon(len).unwrap_or_else(|e| panic!("VirtualAlloc of {len} bytes failed: {e}"))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let layout = std::alloc::Layout::from_size_align(len, PAGE).expect("a page-aligned reservation");
        let p = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!p.is_null(), "an allocation of {len} bytes failed");
        p
    }
}

/// Counts `len` bytes about to be `release`d that `reserve` did not take (a file's mapping).
pub fn uncounted(len: usize) {
    RESERVED.fetch_add(len, Relaxed);
}

/// Gives back what `reserve` (or, on Unix, any mapping of `len` bytes at `p`) took.
///
/// # Safety
/// `p` and `len` are a reservation's (a mapping's), and nothing reads it after.
pub unsafe fn release(p: *mut u8, len: usize) {
    RESERVED.fetch_sub(len, Relaxed);
    #[cfg(unix)]
    unsafe {
        libc::munmap(p.cast(), len);
    }
    #[cfg(windows)]
    {
        let _ = len;
        crate::windows::release(p);
    }
    #[cfg(not(any(unix, windows)))]
    unsafe {
        std::alloc::dealloc(p, std::alloc::Layout::from_size_align(len, PAGE).expect("a page-aligned reservation"));
    }
}

/// Makes a reservation read-only, so a write after freeze faults (Unix, Windows); elsewhere nothing.
///
/// # Safety
/// `p` and `len` are a reservation's.
pub unsafe fn protect_read(p: *mut u8, len: usize) {
    #[cfg(unix)]
    {
        let r = unsafe { libc::mprotect(p.cast(), len, libc::PROT_READ) };
        assert_eq!(r, 0, "mprotect");
    }
    #[cfg(windows)]
    crate::windows::read_only(p, len).expect("VirtualProtect");
    #[cfg(not(any(unix, windows)))]
    let _ = (p, len);
}

/// Gives back the whole pages inside `len` bytes at `p` of an anonymous reservation that nothing will read or write
/// again: on macOS as reusable (`MADV_FREE_REUSABLE`), which takes them out of the footprint at once (`MADV_FREE`
/// leaves them counted until the OS wants them); elsewhere `MADV_FREE`. Pages the range only partly covers stay.
///
/// # Safety
/// The range lies in a reservation of `reserve`'s, and nothing reads or writes it until `reuse_pages` marks it in
/// use again. Returns the bytes given back.
pub unsafe fn give_back(p: *mut u8, len: usize) -> usize {
    let (start, end) = ((p as usize).next_multiple_of(PAGE), (p as usize + len) / PAGE * PAGE);
    if end <= start {
        return 0;
    }
    #[cfg(target_vendor = "apple")]
    unsafe {
        libc::madvise(start as *mut libc::c_void, end - start, libc::MADV_FREE_REUSABLE);
    }
    #[cfg(all(unix, not(target_vendor = "apple")))]
    unsafe {
        libc::madvise(start as *mut libc::c_void, end - start, libc::MADV_FREE);
    }
    end - start
}

/// Marks pages `free_pages` gave back as in use again, before they are written: macOS counts them in the footprint
/// again (without it, its ledger of reusable pages would not); elsewhere nothing.
///
/// # Safety
/// `p` and `len` lie in a reservation.
pub unsafe fn reuse_pages(p: *mut u8, len: usize) {
    #[cfg(target_vendor = "apple")]
    unsafe {
        libc::madvise(p.cast(), len, libc::MADV_FREE_REUSE);
    }
    #[cfg(not(target_vendor = "apple"))]
    let _ = (p, len);
}

/// Tells the OS a reservation's pages need not be kept (their bytes may be lost); elsewhere nothing.
///
/// # Safety
/// `p` and `len` lie in a reservation, and nothing reads those bytes before writing them again.
pub unsafe fn free_pages(p: *mut u8, len: usize) {
    // On macOS `MADV_FREE` leaves the pages in the footprint until the OS wants them; reusable ones leave it at
    // once, and a later use marks them in use again (`reuse_pages`).
    #[cfg(target_vendor = "apple")]
    unsafe {
        libc::madvise(p.cast(), len, libc::MADV_FREE_REUSABLE);
    }
    #[cfg(all(unix, not(target_vendor = "apple")))]
    unsafe {
        libc::madvise(p.cast(), len, libc::MADV_FREE);
    }
    #[cfg(windows)]
    crate::windows::reset(p, len);
    #[cfg(not(any(unix, windows)))]
    let _ = (p, len);
}
