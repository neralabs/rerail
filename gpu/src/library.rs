//! The UKL library's device-wide operations, as calls a host plan makes (decision 0400 §5): the kernels are the
//! library's (`ukl/scan.h`, `ukl/compact.h`, `ukl/histogram.h`, `ukl/radix.h`, `ukl/hash.h`), compiled once a process; the plan
//! of launches is here. Each call
//! enqueues its work on the caller's queue and returns; the caller flushes and awaits.

use crate::device::{Buffer, BufferRef, ComputeQueue};
use crate::kernels::{alloc, buffer_of, launch, program, Program};
use crate::Pod;

/// The library's kernels, as one program.
const SOURCE: &str = "#include <ukl/scan.h>\n#include <ukl/compact.h>\n#include <ukl/histogram.h>\n#include <ukl/radix.h>\n#include <ukl/hash.h>\n";

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
    popcount: u32,
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
    scan(queue, input, output, n, false)
}

/// As `scan_exclusive_u32`, each value counted as its set bits: `output[i]` is the rank of word `i`'s first bit in
/// the bitset `input`, `output[n]` the bitset's count.
pub fn scan_exclusive_popcount_u32(queue: &ComputeQueue, input: &BufferRef, output: &BufferRef, n: u32) -> Result<(), String> {
    scan(queue, input, output, n, true)
}

fn scan(queue: &ComputeQueue, input: &BufferRef, output: &BufferRef, n: u32, popcount: bool) -> Result<(), String> {
    let lib = library()?;
    let popcount = popcount as u32;
    let tiles = n.div_ceil(SCAN_TILE).max(1);
    let block = [SCAN_THREADS as u64];
    let tiles_kernel = lib.function("ukl_scan_u32_tiles")?;
    if tiles == 1 {
        let args = ScanArgs { n, has_offsets: 0, popcount };
        // No offsets: `input` stands in for the unread buffer.
        return launch(queue, tiles_kernel).value("args", &args).buffer("in", input).buffer("out", output).buffer("offsets", input).run(&[1], &block);
    }
    let sums = alloc::<u32>(tiles as usize)?;
    let offsets = alloc::<u32>(tiles as usize + 1)?;
    let args = ScanArgs { n, has_offsets: 1, popcount };
    launch(queue, lib.function("ukl_scan_u32_tile_sums")?).value("args", &args).buffer("in", input).buffer("sums", &sums).run(&[tiles as u64], &block)?;
    // The tiles' sums are counts already.
    scan(queue, &sums, &offsets, tiles, false)?;
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

#[repr(C)]
#[derive(Clone, Copy)]
struct HistogramArgs {
    width: u32,
}
unsafe impl Pod for HistogramArgs {}

/// A code type the histogram counts.
pub trait Code: Pod {
    const KERNEL: &'static str;
}
impl Code for u16 {
    const KERNEL: &'static str = "ukl_histogram_u16";
}
impl Code for u32 {
    const KERNEL: &'static str = "ukl_histogram_u32";
}

/// `H[p * width + c]` = how many of partition `p`'s codes (`codes[offsets[p]..offsets[p + 1]]`) are `c`, for each of
/// the `parts` partitions and `c` below `width`. A code at or above `width` sets `bad[0]` to 1 (the input is invalid).
pub fn histogram<C: Code>(queue: &ComputeQueue, codes: &BufferRef, offsets: &BufferRef, parts: u32, width: u32, h: &BufferRef, bad: &BufferRef) -> Result<(), String> {
    launch(queue, library()?.function(C::KERNEL)?)
        .value("args", &HistogramArgs { width })
        .buffer("codes", codes)
        .buffer("offsets", offsets)
        .buffer("H", h)
        .buffer("bad", bad)
        .run(&[parts as u64], &[256])
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RadixArgs {
    n: u32,
    shift: u32,
    tiles: u32,
}
unsafe impl Pod for RadixArgs {}

/// A key type the radix sort sorts.
pub trait RadixKey: Pod {
    const HIST: &'static str;
    const SCATTER: &'static str;
}
impl RadixKey for u32 {
    const HIST: &'static str = "ukl_radix_u32_hist";
    const SCATTER: &'static str = "ukl_radix_u32_scatter";
}
impl RadixKey for u64 {
    const HIST: &'static str = "ukl_radix_u64_hist";
    const SCATTER: &'static str = "ukl_radix_u64_scatter";
}

/// Rows a radix sort's threadgroup takes, a digit a pass.
pub const RADIX_TILE: u32 = 1024;

/// Where a sort left its result: a pass reads one pair of buffers and writes the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sorted {
    /// In `keys` and `values`.
    InPlace,
    /// In `tmp_keys` and `tmp_values`.
    InTemp,
}

/// `keys[..n]` and their `values` sorted by the keys' low `bits` bits, stably (equal keys keep their order): a pass
/// an 8-bit digit, low digit first, between the two pairs of buffers. Keys' bits above `bits` are not looked at.
pub fn radix_sort_pairs<K: RadixKey>(
    queue: &ComputeQueue,
    keys: &BufferRef,
    values: &BufferRef,
    tmp_keys: &BufferRef,
    tmp_values: &BufferRef,
    n: u32,
    bits: u32,
) -> Result<Sorted, String> {
    if crate::device::simd_width() < 8 {
        return Err(format!("the radix sort needs SIMD groups of 8 lanes or more; the device's have {}", crate::device::simd_width()));
    }
    let lib = library()?;
    let (hist_kernel, scatter_kernel) = (lib.function(K::HIST)?, lib.function(K::SCATTER)?);
    let tiles = n.div_ceil(RADIX_TILE).max(1);
    let hist = alloc::<u32>(256 * tiles as usize)?;
    let offsets = alloc::<u32>(256 * tiles as usize + 1)?;
    let mut from: (&BufferRef, &BufferRef) = (keys, values);
    let mut to: (&BufferRef, &BufferRef) = (tmp_keys, tmp_values);
    let passes = bits.div_ceil(8);
    for pass in 0..passes {
        let args = RadixArgs { n, shift: pass * 8, tiles };
        launch(queue, hist_kernel).value("args", &args).buffer("keys", from.0).buffer("hist", &hist).run(&[tiles as u64], &[256])?;
        scan_exclusive_u32(queue, &hist, &offsets, 256 * tiles)?;
        launch(queue, scatter_kernel)
            .value("args", &args)
            .buffer("keys", from.0)
            .buffer("values", from.1)
            .buffer("offsets", &offsets)
            .buffer("outKeys", to.0)
            .buffer("outValues", to.1)
            .run(&[tiles as u64], &[256])?;
        std::mem::swap(&mut from, &mut to);
    }
    Ok(if passes % 2 == 0 { Sorted::InPlace } else { Sorted::InTemp })
}

#[repr(C)]
#[derive(Clone, Copy)]
struct HashArgs {
    n: u32,
    mask: u32,
}
unsafe impl Pod for HashArgs {}

/// A hash table of 32-bit keys on the device (`ukl/hash.h`): per key, its first row and its rows' count.
pub struct HashTable {
    pub capacity: u32,
    pub keys: Buffer,
    pub first: Buffer,
    pub counts: Buffer,
    /// 1 when a build found the table full (its rows past that are not in it).
    pub full: Buffer,
}

impl HashTable {
    /// An empty table of `capacity` slots (a power of two), written in place.
    pub fn new(capacity: u32) -> Result<HashTable, String> {
        if !capacity.is_power_of_two() {
            return Err(format!("a hash table of {capacity} slots: not a power of two"));
        }
        let c = capacity as usize;
        Ok(HashTable {
            capacity,
            keys: buffer_of(&vec![u32::MAX; c])?,
            first: buffer_of(&vec![u32::MAX; c])?,
            counts: buffer_of(&vec![0u32; c])?,
            full: buffer_of(&[0u32])?,
        })
    }

    /// The rows' keys (`rows[..n]`, none `u32::MAX`) into the table.
    pub fn build(&self, queue: &ComputeQueue, rows: &BufferRef, n: u32) -> Result<(), String> {
        launch(queue, library()?.function("ukl_hash_build_u32")?)
            .value("args", &HashArgs { n, mask: self.capacity - 1 })
            .buffer("rows", rows)
            .buffer("keys", &self.keys)
            .buffer("first", &self.first)
            .buffer("counts", &self.counts)
            .buffer("full", &self.full)
            .run(&[n.div_ceil(256) as u64], &[256])
    }

    /// `out[i]` = `values`'s entry for `queries[i]`'s key (`&self.first`, `&self.counts`), or `u32::MAX` when absent.
    pub fn probe(&self, queue: &ComputeQueue, queries: &BufferRef, n: u32, values: &BufferRef, out: &BufferRef) -> Result<(), String> {
        launch(queue, library()?.function("ukl_hash_probe_u32")?)
            .value("args", &HashArgs { n, mask: self.capacity - 1 })
            .buffer("queries", queries)
            .buffer("keys", &self.keys)
            .buffer("values", values)
            .buffer("out", out)
            .run(&[n.div_ceil(256) as u64], &[256])
    }
}
