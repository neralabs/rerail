//! The UKL library's device-wide operations, as calls a host plan makes (decision 0440 §5): the kernels are the
//! library's (`ukl/scan.h`, `ukl/compact.h`, `ukl/histogram.h`, `ukl/radix.h`, `ukl/hash.h`), compiled once a process; the plan
//! of launches is here. Each call
//! enqueues its work on the caller's queue and returns; the caller flushes and awaits.

use crate::device::{Buffer, BufferRef, ComputeQueue};
use crate::compute_kernel_grid::GridBindContext;
use crate::kernels::{alloc, buffer_of, launch, program, Launch, Program};
use crate::Pod;

/// The library's kernels, as one program.
const SOURCE: &str = "#include <ukl/scan.h>\n#include <ukl/compact.h>\n#include <ukl/histogram.h>\n#include <ukl/radix.h>\n#include <ukl/hash.h>\n";

fn library() -> Result<&'static Program, String> {
    program("ukl_library", SOURCE)
}

/// Compiles the library's kernels for the process's device, ahead of a first call that should not wait for them (or
/// fail after its caller's other work was encoded).
pub fn ready() -> Result<(), String> {
    library().map(drop)
}

/// A scan's threads a threadgroup, and its values a threadgroup (`UKL_SCAN_ITEMS` a thread).
pub const SCAN_THREADS: u32 = 256;
pub const SCAN_TILE: u32 = SCAN_THREADS * 4;
/// A sized scan's levels at most, and its sizes' words (`UKL_SCAN_LEVELS`, `UKL_SCAN_SIZES`).
pub const SCAN_LEVELS: usize = 8;
pub const SCAN_SIZES: usize = 36;
/// The byte in a sized scan's sizes of the grid over its `n + 1` outputs (`ScanSpace::size`), for the launches around it.
pub const SCAN_OUTPUT_GRID: u64 = 4 * 32;

#[repr(C)]
#[derive(Clone, Copy)]
struct ScanArgs {
    n: u32,
    has_offsets: u32,
    popcount: u32,
    level: u32,
}
unsafe impl Pod for ScanArgs {}

#[repr(C)]
#[derive(Clone, Copy)]
struct ScanSizeArgs {
    slot: u32,
    cap: u32,
    levels: u32,
    threads: u32,
}
unsafe impl Pod for ScanSizeArgs {}

#[repr(C)]
#[derive(Clone, Copy)]
struct CompactArgs {
    n: u32,
}
unsafe impl Pod for CompactArgs {}

/// A scan's temporaries, for scans of up to `most` values: past one tile, a buffer a level of the tiles' sums
/// (`tiles + 1` words), each scanned in place into its tiles' offsets. Made once and kept, a scan allocates nothing.
pub struct ScanSpace {
    most: u32,
    levels: Vec<Buffer>,
}

impl ScanSpace {
    pub fn new(most: u32) -> Result<ScanSpace, String> {
        ScanSpace::new_in(most, |_, words| alloc::<u32>(words))
    }

    /// Each level's buffer from `make(level, words)`: a caller's scratch, at least `words` 32-bit words.
    pub fn new_in(most: u32, mut make: impl FnMut(usize, usize) -> Result<Buffer, String>) -> Result<ScanSpace, String> {
        let mut levels = Vec::new();
        let mut size = most;
        while size > SCAN_TILE {
            size = size.div_ceil(SCAN_TILE);
            levels.push(make(levels.len(), size as usize + 1)?);
        }
        Ok(ScanSpace { most, levels })
    }

    /// The levels a sized scan of up to `most` values takes: one, and one a level of tiles' sums.
    pub fn levels(&self) -> usize {
        self.levels.len() + 1
    }

    /// Writes into `sizes` (`SCAN_SIZES` words) a sized scan's counts and grids, its count `min(counts[slot], cap)`
    /// (`cap` below `most`): one launch. Then `scan_sized` scans on them, and the launches around it may take the grid
    /// at `SCAN_OUTPUT_GRID` of `threads`-thread groups over the `n + 1` outputs.
    pub fn size(&self, queue: &ComputeQueue, counts: &BufferRef, slot: u32, cap: u32, threads: u32, sizes: &BufferRef) -> Result<usize, String> {
        assert!(self.levels() <= SCAN_LEVELS && cap < self.most);
        launch(queue, library()?.function("ukl_scan_u32_size")?)
            .value("args", &ScanSizeArgs { slot, cap, levels: self.levels() as u32, threads })
            .buffer("counts", counts)
            .buffer("sizes", sizes)
            .run(&[1], &[1])?;
        Ok(1)
    }
}

/// `output[i]` = the sum of `input[..i]` (wrapping), for `i` in `0..=n`: `output` holds `n + 1` values, the last the
/// total. `output` may be `input` (a scan in place, its buffer `n + 1` values).
pub fn scan_exclusive_u32(queue: &ComputeQueue, input: &BufferRef, output: &BufferRef, n: u32) -> Result<(), String> {
    scan_exclusive_u32_in(queue, input, output, n, false, &ScanSpace::new(n)?).map(drop)
}

/// As `scan_exclusive_u32`, each value counted as its set bits: `output[i]` is the rank of word `i`'s first bit in
/// the bitset `input`, `output[n]` the bitset's count.
pub fn scan_exclusive_popcount_u32(queue: &ComputeQueue, input: &BufferRef, output: &BufferRef, n: u32) -> Result<(), String> {
    scan_exclusive_u32_in(queue, input, output, n, true, &ScanSpace::new(n)?).map(drop)
}

/// `scan_exclusive_u32` (or the popcount scan), in `space`'s temporaries: the launches it made.
pub fn scan_exclusive_u32_in(queue: &ComputeQueue, input: &BufferRef, output: &BufferRef, n: u32, popcount: bool, space: &ScanSpace) -> Result<usize, String> {
    assert!(n <= space.most, "a scan of {n} values in space for {}", space.most);
    scan(queue, input, output, n, popcount, &space.levels, None)
}

/// The scan of `ScanSpace::size`'s count, on the grids it wrote into `sizes`: the launches it made.
pub fn scan_sized(queue: &ComputeQueue, data: &BufferRef, sizes: &BufferRef, space: &ScanSpace) -> Result<usize, String> {
    scan(queue, data, data, space.most, false, &space.levels, Some((sizes, 0)))
}

/// A level: `n` values (a sized scan's most), its tiles' sums in `levels[0]`, the levels above in the rest. Sized,
/// each launch reads its count and its grid from `sizes` at its level.
fn scan(queue: &ComputeQueue, input: &BufferRef, output: &BufferRef, n: u32, popcount: bool, levels: &[Buffer], sized: Option<(&BufferRef, u32)>) -> Result<usize, String> {
    let lib = library()?;
    let tiles = n.div_ceil(SCAN_TILE).max(1);
    let (sizes, level) = sized.map_or((input, 0), |(s, k)| (s, k + 1));
    let tiles_kernel = lib.function("ukl_scan_u32_tiles")?;
    let popcount = popcount as u32;
    if tiles == 1 {
        let args = ScanArgs { n, has_offsets: 0, popcount, level };
        // No offsets: `input` stands in for the unread buffer.
        on_grid(launch(queue, tiles_kernel).value("args", &args).buffer("in", input).buffer("out", output).buffer("offsets", input).buffer("sizes", sizes), sized, tiles)?;
        return Ok(1);
    }
    let (sums, above) = levels.split_first().expect("a level for each tile's sums");
    let args = ScanArgs { n, has_offsets: 1, popcount, level };
    on_grid(launch(queue, lib.function("ukl_scan_u32_tile_sums")?).value("args", &args).buffer("in", input).buffer("sums", sums).buffer("sizes", sizes), sized, tiles)?;
    // The tiles' sums are counts already: scanned in place into their offsets.
    let made = scan(queue, sums, sums, tiles, false, above, sized.map(|(s, k)| (s, k + 1)))?;
    on_grid(launch(queue, tiles_kernel).value("args", &args).buffer("in", input).buffer("out", output).buffer("offsets", sums).buffer("sizes", sizes), sized, tiles)?;
    Ok(made + 2)
}

/// A scan's launch on `tiles` threadgroups, or, sized past one tile, on its level's grid in `sizes`.
fn on_grid<B: GridBindContext>(l: Launch<'_, B>, sized: Option<(&BufferRef, u32)>, tiles: u32) -> Result<(), String> {
    let block = [SCAN_THREADS as u64];
    match sized {
        Some((s, k)) if tiles > 1 => l.run_indirect(s, 4 * (8 + 3 * k as u64), &[tiles as u64], &block),
        _ => l.run(&[tiles as u64], &block),
    }
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

/// A radix sort's temporaries, for up to `most` rows: the tiles' digit histograms, scanned in place into their
/// offsets (`256 * tiles + 1` words), and that scan's.
pub struct RadixSpace {
    most: u32,
    hist: Buffer,
    scan: ScanSpace,
}

impl RadixSpace {
    pub fn new(most: u32) -> Result<RadixSpace, String> {
        RadixSpace::new_in(most, |_, words| alloc::<u32>(words))
    }

    /// Each buffer from `make(k, words)`: the histograms 0, the scan's levels 1 and up.
    pub fn new_in(most: u32, mut make: impl FnMut(usize, usize) -> Result<Buffer, String>) -> Result<RadixSpace, String> {
        let words = 256 * most.div_ceil(RADIX_TILE).max(1);
        let hist = make(0, words as usize + 1)?;
        Ok(RadixSpace { most, hist, scan: ScanSpace::new_in(words, |k, w| make(k + 1, w))? })
    }
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
    radix_sort_pairs_in::<K>(queue, [keys, values], [tmp_keys, tmp_values], n, bits, &RadixSpace::new(n)?).map(|(s, _)| s)
}

/// `radix_sort_pairs` of `[keys, values]`, `tmp` the other pair, in `space`'s temporaries: where the rows are, and the
/// launches it made.
pub fn radix_sort_pairs_in<K: RadixKey>(queue: &ComputeQueue, pair: [&BufferRef; 2], tmp: [&BufferRef; 2], n: u32, bits: u32, space: &RadixSpace) -> Result<(Sorted, usize), String> {
    assert!(n <= space.most, "a sort of {n} rows in space for {}", space.most);
    let lib = library()?;
    let (hist_kernel, scatter_kernel) = (lib.function(K::HIST)?, lib.function(K::SCATTER)?);
    let tiles = n.div_ceil(RADIX_TILE).max(1);
    let hist: &BufferRef = &space.hist;
    let (mut from, mut to) = (pair, tmp);
    let passes = bits.div_ceil(8);
    let mut made = 0;
    for pass in 0..passes {
        let args = RadixArgs { n, shift: pass * 8, tiles };
        launch(queue, hist_kernel).value("args", &args).buffer("keys", from[0]).buffer("hist", hist).run(&[tiles as u64], &[256])?;
        made += 2 + scan_exclusive_u32_in(queue, hist, hist, 256 * tiles, false, &space.scan)?;
        launch(queue, scatter_kernel)
            .value("args", &args)
            .buffer("keys", from[0])
            .buffer("values", from[1])
            .buffer("offsets", hist)
            .buffer("outKeys", to[0])
            .buffer("outValues", to[1])
            .run(&[tiles as u64], &[256])?;
        std::mem::swap(&mut from, &mut to);
    }
    Ok((if passes % 2 == 0 { Sorted::InPlace } else { Sorted::InTemp }, made))
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
