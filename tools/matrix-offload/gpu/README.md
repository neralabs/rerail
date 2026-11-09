# GPU microprobes (G1..G6)

Metal compute probes for relational kernels restated as dense work, measured on the Apple GPU: plain
threads, and the matrix units (Metal 4 tensor ops via Metal Performance Primitives `matmul2d`, the older
`simdgroup_matrix` path, and MPS as Apple's own tuned reference). Results and conclusions are in
`RESULTS-gpu.md`.

## Layout

- `gpuprobe.swift`: host harness (one binary, one probe per run). Allocates the buffers, fills them, checks
  every kernel against a CPU reference before timing it, and prints a markdown table per probe.
- `shaders/*.metal`: kernel sources, compiled **at run time** with `MTLDevice.makeLibrary(source:options:)`
  at `MTLLanguageVersion.version4_0`. The offline Metal toolchain (`xcrun metal`) is not needed.
  - `g1_empty.metal`: empty kernel (dispatch floor).
  - `g2_stream.metal`: streaming read+reduce and copy (DRAM bandwidth).
  - `g3_bitmm.metal`: bit-matrix AND+popcount product, naive and threadgroup-tiled (macros `TM`, `KC`).
  - `g4_mpp.metal`: MPP `matmul2d` over inline tensors wrapping device buffers (macros `TA`, `TB`, `TC`,
    `TM`, `TN`, `NSG`, `SWZ`).
  - `g4_simdgroup.metal`: 8x8 `simdgroup_matrix` matmul, and a plain row gather (the one-hot baseline).
  - `g5_isect.metal`: sorted u16 intersection counts (thread per pair, SIMD-group binary search,
    SIMD-group chunked merge) and 8 KB bitmap AND+popcount.
  - `g6_hist.metal`: segmented histograms with threadgroup atomics, per-SIMD-group histograms and device
    atomics. `g6_hist_mm.metal`: the same histogram on the matrix units (one-hot hi x lo int8 tiles built in
    threadgroup memory, accumulated in an MPP cooperative tensor).

## Build

```sh
tools/matrix-offload/gpu/build.sh                       # -> tools/matrix-offload/gpu/bin/gpuprobe (gitignored)
OUT=/path/to/scratch/gpuprobe tools/matrix-offload/gpu/build.sh   # build elsewhere
```

`build.sh` runs `xcrun swiftc -O -framework Metal -framework MetalPerformanceShaders gpuprobe.swift`
(Swift 6.3, SDK 26.x, macOS 26).

## Run

```sh
B=tools/matrix-offload/gpu/bin/gpuprobe; S=tools/matrix-offload/gpu/shaders
$B $S g1            # dispatch floor
$B $S g2            # bandwidth (4 MB .. 512 MB read+reduce, 256 MB copy and blit)
$B $S g3            # bit-matrix popcount product
$B $S g4            # matrix units: MPP, simdgroup_matrix, MPS; one-hot shapes
$B $S g5            # intersections
$B $S g6            # segmented histogram
$B $S g4 quick      # smaller sizes / fewer repetitions
```

Each probe stays under about 30 s of wall time and 1 GB of buffers (the largest are G2's 512 MB read and
G4's n = 8192 matrices). Environment knobs:

- `GPUPROBE_WARM_MS=100`: before each timed series, keep the GPU busy for about 100 ms (streaming copy) so
  the clock has ramped. Without it the numbers are "cold" (bursty work at a low GPU clock). The tables in
  `RESULTS-gpu.md` say which mode each was taken in.
- `G4_SIZES=4096,8192`: square sizes for G4.
- `G4_REPEAT=k`: encode each G4 MPP matmul and each G2 reduce k times per command buffer (sustained-clock
  check).

## What is measured

Per command buffer: device time (`gpuEndTime - gpuStartTime`, the primary measure), host CPU time
(`getrusage(RUSAGE_SELF)` around encode+commit+wait, so it includes Metal's own threads) and wall time.
Tables report device time min and median, host CPU median, wall min / median, a throughput at the minimum
device time, and bytes/s at min / median. The load average is printed with every table.
