# ANE microprobes: relational work as dense fp16 tensors

Machine: Apple M5 Max, macOS 26.4.1 (25E253), Xcode 26.6, Core ML ML programs (opset macOS15), built with
coremltools 9.0 and run from the Swift harness `anebench.swift` with `computeUnits = .cpuAndNeuralEngine`.
The compiled ANE bundles carry the `H17C` architecture tag.

## Intro: what was measured and how to read the numbers

The question: can the Neural Engine do relational work (joins, semi-joins, overlap counts, group-by counts,
boolean closure) as dense fp16 matrix work? And does its ability to read compressed constant operands
(int8, or palettes of 4, 2 or 1 bits expanded on chip) save the DRAM bandwidth that is expected to limit it?

How each number was taken:

- **Wall** is the time per `prediction(from:)` call on a monotonic clock: minimum and median over N calls,
  after warm-up calls. Wall time includes the dispatch cost (about 100 us, see A1).
- **Host CPU** is the process's user + sys time (getrusage) over the timed loop, divided by N. ANE execution
  itself is not CPU time.
- **Placement** comes from `MLComputePlan`: the preferred device of every non-const op, with the devices
  that support it in brackets, e.g. `matmul@ANE[CPU+ANE]`. The `constexpr_*` decompression ops show `none`,
  because they are folded into the consuming op at compile time. A row whose ops ran on the CPU is
  labelled **CPU** and is not counted as ANE.
- **IOSurface I/O** (`--iosurface`): the inputs and outputs are MLMultiArrays backed by CVPixelBuffers
  (`OneComponent16Half`), and the outputs are passed as `outputBackings`. Wall time is about the same with or
  without it, but host CPU per call drops (for 4096x4096 inputs, from 1.0 ms to 0.25 ms).
- **Load average** (1/5/15 min) is given with every table. During the session it fell from ~155 to ~10.
  Tables marked "low load" are reruns of cached models at load 10-40. They agree with the high-load runs to
  within about 10% on minimum wall time, so the ANE minimum times barely depend on CPU load. Medians and
  host CPU do depend on it.
- **Derived numbers**: TFLOPS = 2*M*K*N / wall_min. "Stored GB/s" = bytes of W as stored / wall_min.
  "fp16-eq GB/s" = 2*K*N / wall_min (the bandwidth that dense fp16 weights would have needed). The
  "slope" columns use the difference between two weight sizes (4096x4096 and 8192x8192), which removes
  the fixed per-call cost.

Raw result lines are in the scratchpad `ane-runs/out/*.log`. Every row below comes from one of them.

## A1. Dispatch floor, batch API, compile and load

The planner places small models on the **CPU** even when the ANE supports every op. That makes the true
empty-model ANE floor unobservable through the public API. Single-op models stayed on the CPU below roughly
0.5 GFLOP of work, unless the weights were large:

| model | placement |
|---|---|
| add 1, add 4096, add (1,C,1,W) up to 1024x1024 | CPU |
| matmul 1x2048x2048, 64x1024x1024, 512x512x512, 256x768x768 | CPU |
| matmul 1x4096x4096 (32 MB weights), 64x2048x2048, 256x1024x1024 | ANE |
| chain of 4 matmul+clip, 512x512 | ANE (1 step of the same: CPU) |

Per-call cost (us):

| model | placement | load | I/O | wall min | wall med | host CPU/call |
|---|---|---|---|---|---|---|
| add_1 | CPU | 156 | plain | 35.1 | 146.5 | 121 |
| add_1 | CPU | 10 | plain | 23.2 | 34.9 | 46 |
| add_1 | CPU | 10 | IOSurface | 14.4 | 25.1 | 35 |
| matmul 256x1024x1024 | ANE | 122 | plain | 165.9 | 373.9 | 220 |
| matmul 256x1024x1024 | ANE | 10 | plain | 140.7 | 185.6 | 127 |
| matmul 256x1024x1024 | ANE | 10 | IOSurface | 113.3 | 155.1 | 87 |
| matmul 64x2048x2048 | ANE | 10 | IOSurface | 157.3 | 193.3 | 90 |
| chain 512x512, k=4 | ANE | 23 | IOSurface | 154.9 | 177.7 | 95 |

- **ANE fixed cost per call: about 100 us of wall time and 90-130 us of host CPU.** The wall figure is the
  intercept of the 4096x4096 to 8192x8192 matvec slope in A2: 368 us - 264 us = 104 us. So a single ANE
  call cannot pay for itself below about 1 GFLOP, or about 10 MB of weight streaming.
- **The batch API does not amortise the cost.** `predictions(from: MLBatchProvider)` with 64 items took
  72 us per item for add_1 (CPU) and 106 us min / 199 us median per item for matmul 256x1024x1024. Host CPU
  was 140-180 us per item, worse than single calls. With 128 items at high load it was 359 us per item.
- **Compile** (`MLModel.compileModel`, .mlpackage to .mlmodelc): 20-130 ms, independent of weight size.
- **First load** (the ANE compile, cached in `~/Library/Caches/<process>/com.apple.e5rt.e5bundlecache`)
  grows with weight size: 2 MB weights 0.1-0.2 s; 8 MB 0.6 s; 32 MB 1.1-4.3 s; 128 MB fp16 16-27 s;
  8192x8192 int8 9-17 s; p4 4.5-9 s; p2 3-4 s; p1 7-22 s. Loading from the cache takes 7-30 ms.
- **Building** a 128 MB-weight model in coremltools takes about 40 s. Building plus the first load does not
  fit in a 2-minute run, so each large model was built and loaded in a run of its own.

## A2. Matmul with a constant W: is the compressed weight faster when bandwidth-bound?

Setup: Y = X·W. X is an fp16 input (M x K). W is a random 0/1 constant (density 0.5) stored as fp16, int8
(linear symmetric), or palettized 4, 2 or 1 bit (uniform LUT; lossless for 0/1). The op is `matmul`
(`linear` and 1x1 `conv` are in A3).

**Placement:** every row is `matmul@ANE[CPU+ANE]`, with the decompression op `constexpr_*@none`.

**The compressed format reaches the hardware.** The ANE bundle's `weights.bin` for 8192x8192 is 128 MiB
(fp16), 64 MiB (int8), 32 MiB (uint4), 16 MiB (uint2) and 8 MiB (uint1), and the bundle MIL still contains
`constexpr_lut_to_dense` with uint1/2/4 indices. So the ANE reads the compressed bytes and expands them on
chip.

Low load (12/33/68), IOSurface I/O, 300 calls; wall min / median in us:

| shape M x K x N | W stored | W bytes | wall min | wall med | host CPU | TFLOPS | stored GB/s | fp16-eq GB/s |
|---|---|---|---|---|---|---|---|---|
| 1 x 8192 x 8192 | fp16 | 128 MiB | 1160 | 1265 | 172 | 0.12 | 116 | 116 |
| 1 x 8192 x 8192 | int8 | 64 MiB | 1023 | 1162 | 179 | 0.13 | 66 | 131 |
| 1 x 8192 x 8192 | p4 | 32 MiB | 881 | 966 | 171 | 0.15 | 38 | 152 |
| 1 x 8192 x 8192 | p2 | 16 MiB | 805 | 865 | 151 | 0.17 | 21 | 167 |
| 1 x 8192 x 8192 | p1 | 8 MiB | 660 | 717 | 146 | 0.20 | 12.7 | 203 |
| 64 x 8192 x 8192 | fp16 | 128 MiB | 1169 | 1229 | 163 | 7.3 | 115 | 115 |
| 64 x 8192 x 8192 | int8 | 64 MiB | 1026 | 1076 | 171 | 8.4 | 65 | 131 |
| 64 x 8192 x 8192 | p4 | 32 MiB | 823 | 914 | 173 | 10.4 | 41 | 163 |
| 64 x 8192 x 8192 | p2 | 16 MiB | 821 | 901 | 169 | 10.5 | 20 | 163 |
| 64 x 8192 x 8192 | p1 | 8 MiB | 698 | 767 | 162 | 12.3 | 12.0 | 192 |
| 1 x 16384 x 4096 | fp16 | 128 MiB | 1156 | 1209 | 161 | 0.12 | 116 | 116 |
| 1 x 16384 x 4096 | p1 | 8 MiB | 663 | 719 | 141 | 0.20 | 12.6 | 202 |
| 1 x 4096 x 4096 | fp16 | 32 MiB | 368 | 417 | 133 | 0.09 | 91 | 91 |
| 1 x 4096 x 4096 | int8 | 16 MiB | 343 | 401 | 134 | 0.10 | 49 | 98 |
| 1 x 4096 x 4096 | p4 | 8 MiB | 275 | 342 | 121 | 0.12 | 31 | 122 |
| 1 x 4096 x 4096 | p2 | 4 MiB | 277 | 326 | 120 | 0.12 | 15 | 121 |
| 1 x 4096 x 4096 | p1 | 2 MiB | 239 | 290 | 118 | 0.14 | 8.8 | 140 |
| 64 x 4096 x 4096 | fp16 | 32 MiB | 390 | 451 | 143 | 5.5 | 86 | 86 |
| 64 x 4096 x 4096 | p1 | 2 MiB | 251 | 322 | 127 | 8.6 | 8.4 | 134 |

The same runs at load 120-160, without IOSurface, gave the same minimums to within 5-8%. For example, the
1 x 8192 x 8192 fp16 run gave 1204 us.

Marginal rates, from the slope between 4096x4096 and 8192x8192 at M = 1 (no fixed cost):

| W stored | weight elements / s | stored bytes / s | speedup over fp16 | byte ratio |
|---|---|---|---|---|
| fp16 | 63 G | 127 GB/s | 1.0x | 1x |
| int8 | 74 G | 74 GB/s | 1.16x | 2x |
| p4 | 83 G | 42 GB/s | 1.31x | 4x |
| p2 | 95 G | 24 GB/s | 1.50x | 8x |
| p1 | 120 G | 15 GB/s | 1.88x | 16x |

What this shows:

- **fp16 weights stream at about 120 GB/s. That is the ceiling for ANE matvec and skinny matmul**, well
  below the M5 Max's DRAM bandwidth. Times are the same for M = 1 and M = 64, so the ANE gets up to 64 rows
  per weight read for free.
- **Compressed W is faster, but by much less than the byte ratio.** p1 moves 16x fewer bytes and runs
  1.9x faster. As the bits shrink, the bottleneck moves from bytes to an on-chip weight-ingest rate of about
  60-120 G weight elements per second. A 1-bit 0/1 relation therefore streams at about 120 G entries/s,
  which is 15 GB/s of DRAM. **The bandwidth saving is real (8-16x fewer DRAM bytes), but it does not become
  a proportional speedup.** It mainly frees DRAM bandwidth for other agents (see A8).

### Compute-bound shapes (W reused across many rows)

| shape | W | form | placement | I/O | load | wall min (us) | wall med | host CPU | TFLOPS |
|---|---|---|---|---|---|---|---|---|---|
| 2048^3 | fp16 | matmul | ANE | plain | 34 | 1503 | 1566 | 341 | 11.4 |
| 2048^3 | fp16 | matmul | ANE | IOSurface | 31 | 1362 | 1415 | 166 | 12.6 |
| 2048^3 | int8 | matmul | ANE | plain | 34 | 1472 | 1523 | 330 | 11.7 |
| 2048^3 | p4 | matmul | ANE | plain | 34 | 1304 | 1354 | 342 | 13.2 |
| 2048^3 | p2 | matmul | ANE | plain | 34 | 1233 | 1330 | 329 | 13.9 |
| 2048^3 | p1 | matmul | ANE | IOSurface | 10 | 1081 | 1132 | 147 | 15.9 |
| 4096^3 | fp16 | matmul | ANE | IOSurface | 42 | 17290 | 17334 | 269 | 7.9 |
| 4096^3 | int8 | matmul | ANE | IOSurface | 40 | 10579 | 10607 | 254 | 13.0 |
| 4096^3 | p4 | matmul | ANE | plain | 35 | 9647 | 9799 | 995 | 14.2 |
| 4096^3 | p2 | matmul | ANE | IOSurface | 40 | 7685 | 7746 | 253 | 17.9 |
| 4096^3 | p1 | matmul | ANE | IOSurface | 40 | 7928 | 7963 | 249 | 17.3 |

- **Compression matters most at large square shapes.** At 4096^3 the fp16 W (32 MB) is evidently
  re-streamed once per tile of rows, so the "compute-bound" case is actually bandwidth-bound: fp16 reaches
  7.9 TFLOPS, and p2/p1 reach about 18 TFLOPS (2.25x). At 2048^3 (8 MB W, which fits on chip) the gain is
  only 1.1-1.25x.
- Plain (non-IOSurface) I/O costs about 1 ms of host CPU per call for 32 MB of input and output.

## A3. Dynamic matmul and 1x1 conv versus matmul

| model | placement | load | wall min (us) | wall med | host CPU | TFLOPS |
|---|---|---|---|---|---|---|
| dynamic matmul 2048^3 (both inputs) | ANE | 35 | 1680 | 1748 | 183 | 10.2 |
| dynamic matmul 4096^3 | ANE | 33 | 17271 | 17358 | 313 | 7.96 |
| conv 1x1, W fp16 const, 2048^3 | ANE | 33 | 1241 | 2930 | 220 | 13.8 |
| conv 1x1, W fp16 const, 4096^3 | ANE | 33 | 16698 | 16763 | 284 | 8.2 |
| conv 1x1, W p2 const, 4096^3 | ANE | 31 | 7116 | 7203 | 244 | 19.3 |
| linear, W fp16 const, 4096^3 | ANE | 31 | 17327 | 20123 | 330 | 7.9 |
| conv 1x1, weight as an input, 2048^3 | ANE (per plan) | 36 | 85230 | 87094 | 330 | 0.20 |

(All rows use IOSurface I/O.)

- **The reported 3x advantage of 1x1 conv over matmul does not hold here.** conv is 0-9% faster than
  matmul (`linear` is the same as matmul). Core ML evidently lowers both to the same ANE op.
- **A dynamic matmul (both operands are inputs) runs on the ANE at the same speed as fp16 const W at
  4096^3**, and 20% slower at 2048^3. This is the form a runtime relation (not baked into the model) would
  use. It cannot use the compressed formats.
- **A conv whose weight is an input** reports ANE placement but is 60x slower. It is unusable.

## A4. Exactness of counting

Setup: X and W are 0/1 inputs. Row i of X has a known number of ones, c_i, which sweeps the fp16
boundaries. W column 0 is all ones, so y[i,0] = c_i; the other columns are random. The outputs are y = X·W,
`cast(greater(y, 0.5))`, `clip(y, 0, 1)` and `reduce_sum(X, axis 1)`. Each output was compared with the
exact int64 result. Load was 57-89.

| K | matmul placement | y exact for counts | above 2048 | finite max | gt / clip | reduce_sum |
|---|---|---|---|---|---|---|
| 4096 | ANE | all <= 2048 | errors of 1-2 ulp (e.g. 2049 gives 2050, ties not to even); 213 of 65,536 outputs beyond one rounding | 4096 | exact | ANE; exact <= 2048, then +-1 ulp |
| 16384 | ANE | all <= 2048 | +-1-2 ulp | 16384 | exact | ANE; same |
| 16384, W x 4 | ANE | all <= 2048 | +-1 ulp up to 32764 | **32768, then inf** (exact 32772 gives inf) | n/a | n/a |
| 32768 | **CPU** (the ANE compile failed: `ANECCompile() FAILED`) | all <= 2048 | correctly rounded once | 32768 | exact (CPU) | ANE |
| 65536 | **CPU** (not ANE-supported) | <= 2048 | large errors; output **saturates at 32768** | 32768 | exact (CPU) | ANE; inf at >= 65520 |

- **Counts are exact up to 2048 on the ANE.** Above that they round to fp16 spacing, with slightly
  non-IEEE rounding (an error of 1 ulp beyond correct rounding).
- **ANE matmul outputs overflow to inf above 32768, not at fp16's 65504.** So the accumulator or output
  path is effectively limited to about 2^15.
- **The ANE matmul accepts K up to 16384.** At K = 32768 the ANE compile fails and the model falls back to
  the CPU. The CPU fallback at K = 65536 also clamps at 32768.
- **`greater(y, 0.5)` and `clip(y, 0, 1)` were exact in every case**, including rows whose counts had
  rounded or overflowed to inf. **Boolean (semi-join / existence) results are exact regardless of K.**
- `reduce_sum` over a long axis runs on the ANE and is accurate to fp16 rounding (+-1 ulp) up to 65504.

## A5. Equality-join counts by broadcast compare

Setup: counts[i] = sum over j of (a[i] == b[j]). a is (N,1) and b is (1,M), fp16 integers up to 2047. For the
two-digit key (15-bit key split into an 11-bit high half and a low half),
eq = eq(a_hi, b_hi) * eq(a_lo, b_lo), using multiply instead of logical_and.

Placement:

- `equal` -> `cast` -> `reduce_sum`: all ANE.
- `sub` -> `abs` -> `minimum` -> `sub` (no compare op): all ANE.
- **`select(equal, 1, 0)`: CPU.** The whole model goes to the CPU. At 4096 the ANE compile failed and the
  run errored.

All ANE results were exact (0 wrong counts in every case). Load 25-36, IOSurface not used:

| N x M | key | form | wall min (us) | wall med | host CPU | G comparisons / s |
|---|---|---|---|---|---|---|
| 2048 x 2048 | 1 digit | equal+cast | 293 | 329 | 137 | 14.3 |
| 2048 x 2048 | 1 digit | sub/abs/min | 317 | 335 | 95 | 13.2 |
| 4096 x 4096 | 1 digit | equal+cast | 756 | 812 | 161 | 22.2 |
| 4096 x 4096 | 1 digit | sub/abs/min | 976 | 1016 | 153 | 17.2 |
| 8192 x 8192 | 1 digit | equal+cast | 2435 | 2492 | 198 | 27.6 |
| 16384 x 4096 | 1 digit | equal+cast | 5949 | 6187 | 300 | 11.3 |
| 2048 x 2048 | 2 digits | equal+cast, mul | 586 | 624 | 142 | 7.2 |
| 2048 x 2048 | 2 digits | sub form, mul | 520 | 548 | 140 | 8.1 |
| 4096 x 4096 | 2 digits | equal+cast, mul | 1767 | 1826 | 207 | 9.5 |
| 4096 x 4096 | 2 digits | sub form, mul | 1450 | 1672 | 202 | 11.6 |

Brute-force compare peaks at about 28 G comparisons/s, and about 10 G/s for two-digit keys. That is about
2-3 ops per comparison, so roughly 0.1 TFLOPS-equivalent: elementwise work on the ANE is about 100x slower
than its matmul path. A one-hot or bitmap matmul formulation (A2) is the way to do joins there.

## A6. Fusion on chip: k steps in one model

Setup: Y (n x n, fp16 input) is updated by `Y = clip(Y·A, 0, 1)` (or `acc`: `Y = min(Y + Y·A, 1)`),
k times within one model. A is a constant n x n 0/1 matrix, stored as fp16 or p1.

- "sparse" A has density 2/n, the shape of a graph adjacency.
- "dense" A has density 0.5.
- Per-step time is the wall minimum divided by k.
- TFLOPS counts the dense 2n^3 per step, so it is a dense-equivalent rate.
- Every row used IOSurface I/O, and every op was placed on the ANE, except the k = 1, n = 512 models
  (CPU).

| n | A | W stored | k = 1 (us) | k = 4 per step | k = 16 per step | TFLOPS k = 1 -> best | load |
|---|---|---|---|---|---|---|---|
| 512 | sparse | fp16 | 153 (**CPU**) | 38.7 | 18.4 | -> 14.6 | 23 |
| 512 | sparse | p1 | 147 (**CPU**) | 40.0 | 26.4 | -> 10.2 | 23 |
| 1024 | sparse | fp16 | 230 | 147 | 124 | 9.3 -> 17.3 | 22 |
| 1024 | sparse | p1 | 172 | 95 | 72 | 12.5 -> 29.9 | 22 |
| 1024 | dense | fp16 | 286 | - | 155 | 7.5 -> 13.9 | 18 |
| 1024 | dense | p1 | 255 | - | 121 | 8.4 -> 17.8 | 18 |
| 1024 | sparse, acc | fp16 / p1 | - | - | 183 / 139 | -> 11.7 / 15.4 | 18 |
| 2048 | sparse | fp16 | 1583 | 1402 | 1350 | 10.9 -> 12.7 | 15 |
| 2048 | dense | fp16 | 1601 | - | 1349 | 10.7 -> 12.7 | 15 |
| 2048 | sparse | p1 | 648 | 510 | 435 | 26.5 -> 39.5 | 14 |
| 2048 | dense | p1 | 836 | - | 656 | 20.6 -> 26.2 | 13 |
| 4096 | sparse | fp16 | 22754 | 18138 | - | 6.0 -> 7.6 | 13 |
| 4096 | sparse | p1 | 3869 | 3324 | 3996 | 35.5 -> 41.3 | 12 |
| 4096 | dense | p1 | 6468 | 5911 | - | 21.2 -> 23.2 | 10 |

- **Fusing steps pays where the fixed per-call cost dominates.** At n <= 1024, k = 16 in one model is 1.9x
  (fp16) to 2.4x (p1) faster per step than one call per step (the k = 1 time). At n >= 2048 the gain is
  15-30%, because each step is already about 1 ms or more.
- **fp16 constant weights cap a square step at 6-13 TFLOPS.** **1-bit A reaches 21-26 TFLOPS when dense.**
  When sparse it reaches **35-41 TFLOPS dense-equivalent**, which suggests the ANE also skips zero blocks
  of compressed weights. fp16 A shows no sparse/dense difference at n >= 2048. For boolean closure over a
  fixed relation, **the 1-bit palettized form is the one to use: 3-6x faster than fp16 at n = 2048-4096.**
- Above roughly 15 TFLOPS, the bitmap matmul is no longer limited by DRAM. The limit is the ANE's
  weight-expansion and MAC rate.

## A7. gather, gather_nd, topk, argsort

| model | placement (supported devices) | load | wall min (us) | wall med | host CPU |
|---|---|---|---|---|---|
| gather rows, T const 65536x64, 4096 indices | **CPU** (`gather[CPU]`) | 11 | 210 | 219 | 230 |
| gather_nd, same | **CPU** (`[CPU]`) | 11 | 325 | 365 | 378 |
| gather from an input table 16384x64 | **CPU** | 11 | 235 | 252 | 266 |
| gather, T 4096x256, 4096 indices | **CPU** | 11 | 594 | 777 | 790 |
| topk k=64 over 16384 | **CPU** (`topk[CPU+ANE]`; the planner chose CPU) | 11 | 52 | 65 | 87 |
| topk k=16 over 65536 | **CPU** (`[CPU]`) | 11 | 105 | 119 | 140 |
| argsort 4096 / 16384 | **CPU** (`[CPU]`) | 11 | 116 / 463 | 127 / 485 | 147 / 523 |

**On M5 / macOS 26.4, neither gather nor argsort is supported on the ANE at all**, and topk ran on the CPU.
Index lookups must be expressed as one-hot matmuls (A2 rates) or stay off the ANE.

## A8. ANE under a busy GPU

A Metal thread ran a memory-bound kernel continuously during the timed loop: 64 MB read plus 64 MB write
per dispatch, in 4-dispatch command buffers. Load was 10-26 and IOSurface I/O was used. The host CPU column
includes the GPU-feeding thread, so it is inflated.

| model | GPU idle: wall min / med (us) | GPU busy: wall min / med (us) | slowdown (min) |
|---|---|---|---|
| matmul 1x8192x8192 fp16 | 1127 / 1163 | 1204 / 1323 | 1.07x |
| matmul 1x8192x8192 p1 | 535 / 608 | 1413 / 2448 | 2.6x |
| matmul 2048^3 p1 | 1081 / 1132 | 2300 / 3038 | 2.1x |
| matmul 4096^3 p2 | 7674 / 7724 | 8253 / 8879 | 1.08x |
| matmul 4096^3 fp16 | 17282 / 17300 | 19156 / 31032 | 1.11x (median 1.8x) |

- **A busy GPU slows the ANE 1.1-2.6x.** Surprisingly, the models that move the fewest DRAM bytes (p1)
  slowed the most. That suggests the shared fabric, cache or power and clock budget, not DRAM bytes. ANE and
  GPU work are not free to overlap.
- Heavy CPU load alone (load average 155 versus 10) left the ANE minimum times within about 10%. It did
  raise medians (2-3x) and host CPU per call (1.5-2x).

## What this means for relational work

- **Shape of the work.** The ANE is a matmul engine with a fixed cost of about 100 us and 100 us of host
  CPU per call. Use it only for batched work of at least about 1 GFLOP or 10 MB per call. Many-small-call
  patterns (per-rule, per-file) lose to the CPU, and Core ML's planner sends them to the CPU anyway. Fusing
  k iterations into one model removes most of the per-call cost. Fixpoint loops must therefore be unrolled
  into a fixed k per call, with convergence checked between calls.
- **Bandwidth hypothesis: partly confirmed, partly refuted.**
  - Confirmed: compressed constants really are read compressed. A 1-bit relation moves 16x fewer DRAM bytes.
  - Confirmed: for repeated passes over a fixed relation (closure steps, wide matmuls), 1-bit weights give
    2-6x speedups and 20-40 TFLOPS dense-equivalent. Zero-block skipping helps further for sparse
    relations.
  - Refuted for streaming a relation once (matvec, semi-join of a few probe rows against a big relation):
    the cost is an on-chip ingest rate of about 60-120 G entries/s, not DRAM. 1-bit is only 1.9x faster
    than fp16, and fp16 itself streams at only about 120 GB/s.
  - In that regime the ANE gives about 0.1-0.2 TFLOPS. The CPU or GPU reading a packed bitmap at hundreds
    of GB/s would likely do better.
- **Compression applies only to constants.** A relation that changes per run must be an input
  (dynamic matmul): fp16 only, 8-13 TFLOPS, with 2 bytes per bit-entry of input traffic. A compiled model
  bakes the relation in, and the first load of a 128 MB-equivalent model takes 7-27 s of ANE compile
  (cached afterwards). So the compressed path suits relations that are stable across many runs, such as
  schema-level or rule-level matrices, not per-run facts.
- **Exactness.**
  - Boolean results (`greater` / `clip` after a matmul) are exact for any K up to the ANE limit of 16384.
    Semi-joins and closure are safe.
  - Counts are exact only up to 2048, rounded above that, and inf above 32768. Group-by and overlap counts
    need K-splitting so that partial counts stay at or below 2048, or must be used as thresholds only.
  - K above 16384 must be split across ops or calls (the ANE compile fails at 32768).
- **Joins by compare.** Broadcast equality runs on the ANE and is exact, but at only 10-28 G comparisons/s.
  Use one-hot or bitmap matmuls instead. `select` and `logical_and` kick models off the ANE; use `cast` and
  `mul`.
- **No gather, argsort or topk on the ANE.** Index-driven lookups and sorting stay on the CPU or GPU.
- **Coexistence.** ANE throughput drops up to 2.6x when the GPU is saturated, so an ANE + GPU split is not
  additive.

Honest limits:

- The machine was shared and loaded (load average 10-160). Only minimum times are robust.
- Core ML hides the true empty-call ANE floor, and gives no way to force placement or see the ANE's own
  device time.
- TFLOPS for sparse A are dense-equivalent, not useful work.
- The int8 path used per-tensor symmetric quantization only.
- 4-bit at 4096^3 was measured without IOSurface (host CPU about 1 ms).
- The 128 MB models were measured at M = 1 and 64 only.
