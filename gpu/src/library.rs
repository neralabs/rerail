//! The UKL library's device-wide operations, as calls a host plan makes (decision 0400 §5): the kernels are the
//! library's (`ukl/scan.h`, `ukl/compact.h`), compiled once a process; the plan of launches is here. Each call
//! enqueues its work on the caller's queue and returns; the caller flushes and awaits.

use crate::device::{BufferRef, ComputeQueue};
use crate::kernels::{alloc, launch, program, Program};
use crate::Pod;

/// The library's kernels, as one program.
const SOURCE: &str = "#include <ukl/scan.h>\n#include <ukl/compact.h>\n";

fn library() -> Result<&'static Program, String> {
    program("ukl_library", SOURCE)
}

/// A scan's threads a threadgroup, and its values a threadgroup (`UKL_SCAN_ITEMS` a thread).
pub const SCAN_THREADS: u32 = 256;
pub const SCAN_TILE: u32 = SCAN_THREADS * 4;

#[repr(C)]
#[derive(Clone, Copy)]
struct ScanArgs {
    n: u32,
    has_offsets: u32,
}
unsafe impl Pod for ScanArgs {}

#[repr(C)]
#[derive(Clone, Copy)]
struct CompactArgs {
    n: u32,
}
unsafe impl Pod for CompactArgs {}

/// `output[i]` = the sum of `input[..i]` (wrapping), for `i` in `0..=n`: `output` holds `n + 1` values, the last the
/// total. `input` and `output` may not be the same buffer.
pub fn scan_exclusive_u32(queue: &ComputeQueue, input: &BufferRef, output: &BufferRef, n: u32) -> Result<(), String> {
    let lib = library()?;
    let tiles = n.div_ceil(SCAN_TILE).max(1);
    let block = [SCAN_THREADS as u64];
    let tiles_kernel = lib.function("ukl_scan_u32_tiles")?;
    if tiles == 1 {
        let args = ScanArgs { n, has_offsets: 0 };
        // No offsets: `input` stands in for the unread buffer.
        return launch(queue, tiles_kernel).value("args", &args).buffer("in", input).buffer("out", output).buffer("offsets", input).run(&[1], &block);
    }
    let sums = alloc::<u32>(tiles as usize)?;
    let offsets = alloc::<u32>(tiles as usize + 1)?;
    let args = ScanArgs { n, has_offsets: 1 };
    launch(queue, lib.function("ukl_scan_u32_tile_sums")?).value("args", &args).buffer("in", input).buffer("sums", &sums).run(&[tiles as u64], &block)?;
    scan_exclusive_u32(queue, &sums, &offsets, tiles)?;
    launch(queue, tiles_kernel).value("args", &args).buffer("in", input).buffer("out", output).buffer("offsets", &offsets).run(&[tiles as u64], &block)
}

/// The values whose flag is 1, in order, packed into `out` from 0; `offsets` (`n + 1` values) gets the flags'
/// exclusive scan, and `offsets[n]` is how many were kept. Flags are 0 or 1.
pub fn compact_u32(queue: &ComputeQueue, values: &BufferRef, flags: &BufferRef, out: &BufferRef, offsets: &BufferRef, n: u32) -> Result<(), String> {
    scan_exclusive_u32(queue, flags, offsets, n)?;
    let threads = 256u32;
    launch(queue, library()?.function("ukl_compact_u32")?)
        .value("args", &CompactArgs { n })
        .buffer("values", values)
        .buffer("offsets", offsets)
        .buffer("out", out)
        .run(&[n.div_ceil(threads) as u64], &[threads as u64])
}
