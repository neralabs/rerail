//! The engine as a long-lived WebAssembly instance (a WASI reactor: wasm/README.md, "A long-lived engine"). A page
//! instantiates it once and calls `rerail_command` for each command line, which runs as the `rerail-engine` binary
//! would (`tools::cli`), on the same in-memory filesystem: what one command leaves (a session's store and memo,
//! decision 0056; the rule programs read so far) the next one finds.
//!
//! The host writes the command line into memory it asks for with `rerail_alloc` (the arguments, each ended by a NUL
//! byte), calls `rerail_command`, and frees it with `rerail_free`. Output goes to stdout and stderr as the binary's
//! does, flushed before the call returns. A trap (a panic aborts) leaves the instance unusable: the host starts
//! another.

use std::io::Write as _;

/// `len` bytes the host may write a command line into.
#[unsafe(no_mangle)]
pub extern "C" fn rerail_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len.max(1));
    let at = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    at
}

/// Frees what `rerail_alloc(len)` gave.
///
/// # Safety
/// `at` is a pointer `rerail_alloc(len)` returned, not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rerail_free(at: *mut u8, len: usize) {
    // SAFETY: the caller's contract: `at` is `rerail_alloc(len)`'s, with capacity `len.max(1)`.
    drop(unsafe { Vec::from_raw_parts(at, 0, len.max(1)) });
}

/// Runs the command line at `at` (`len` bytes, each argument ended by NUL) and returns its exit code: 0 when it
/// succeeded, 1 when it reported an error, 2 when the line is not UTF-8.
///
/// # Safety
/// `at` holds `len` initialised bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rerail_command(at: *const u8, len: usize) -> i32 {
    // SAFETY: the caller's contract.
    let bytes = unsafe { std::slice::from_raw_parts(at, len) };
    let Ok(line) = std::str::from_utf8(bytes) else {
        eprintln!("rerail-engine: the command line is not UTF-8");
        return 2;
    };
    let args: Vec<String> = line.split_terminator('\0').map(str::to_owned).collect();
    let code = match crate::tools::cli(&args) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(error) => {
            eprintln!("rerail-engine: {error}");
            1
        }
    };
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    code
}
