//! The calling thread's instructions retired (its counter, not the process's: other tests' threads run beside it),
//! for the tests that keep a kernel's Rust from growing slower.

/// The calling thread's instructions retired so far: macOS's `thread_selfcounts` (its first kind: instructions, then
/// cycles); `None` where the thread's counter is not read.
pub fn instructions() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn thread_selfcounts(kind: i32, buf: *mut u64, bytes: usize) -> i32;
        }
        let mut c = [0u64; 2];
        // SAFETY: `c` holds the two counts the kind writes.
        (unsafe { thread_selfcounts(1, c.as_mut_ptr(), size_of_val(&c)) } == 0).then_some(c[0])
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// The fewest instructions of `runs` runs of `f`, if the thread's counter is read.
pub fn fewest(runs: usize, mut f: impl FnMut()) -> Option<u64> {
    (0..runs).map(|_| {
        let a = instructions()?;
        f();
        Some(instructions()? - a)
    }).min().flatten()
}
